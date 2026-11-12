//! `cost.*`: maliyet panosu, yönlendirme kararları ve kota (§5.1, §5.2, §15.10) için ince köprü
//! (`kor cost`, `kor cost why`, `kor cost --export`, `kor quota`). İş mantığı `kor_runtime::cost`
//! ve `kor_runtime::quota` içinde; burası yalnızca JSON'a çevirir. `lines` alanı, kullanıcının
//! dilinde CLI ile aynı metindir.
//!
//! - `cost.summary(days?: number)` → `{ report, lines }`
//! - `cost.export(format: "csv" | "json", days?: number)` → `{ text }`
//! - `cost.why(last?: number)` → `{ decisions }` (eskiden yeniye)
//! - `cost.quota()` → `{ report, lines }`
//! - `cost.settings()` → `{ budget, routing }` (etkin `[budget]`/`[routing]`; yazma `core.configSet`)
//! - `cost.exportFile(format, days?)` → `{ path }` (kaydetme penceresi; iptalde `path: null`)
//! - `cost.consensus(prompt, models?)` → `Consensus` (§5.1; 2–3 model, varsayılan `[routing] consensus`)
//! - `cost.shadow()` → `[{ main, shadow, agreement, samples }]` (gölge modunun uyum geçmişi)

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `cost::call`'a yönlendirir).
pub const NS: &str = "cost";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "summary" => summary(args, state),
        "export" => export(args, state),
        "exportFile" => export_file(args, app, state).await,
        "settings" => {
            let cfg = config(state);
            Ok(json!({ "budget": cfg.budget, "routing": cfg.routing }))
        }
        "why" => why(args),
        "consensus" => consensus(args, state).await,
        "shadow" => Ok(shadow()),
        "quota" => quota(state).await,
        _ => return None,
    })
}

async fn consensus(args: &[Value], state: &AppState) -> Res {
    let prompt = s(args, 0, "cost.consensus")?.trim().to_owned();
    if prompt.is_empty() || prompt.len() > 20_000 {
        return Err("Soru boş ya da çok uzun".into());
    }
    let dir = root(state)?;
    let loaded = crate::bridge::load_config(&dir)?;
    let mut models: Vec<String> = args
        .get(1)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if models.is_empty() {
        models.clone_from(&loaded.config.routing.consensus);
    }
    if !(2..=3).contains(&models.len()) {
        return Err(
            "Consensus için 2–3 model gerekir (Maliyet → Ayarlar → Consensus modelleri).".into(),
        );
    }
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let mut targets = Vec::new();
    for m in &models {
        targets.push(
            kor_runtime::oneshot_provider(&l, &loaded, Some(m)).map_err(|e| format!("{m}: {e}"))?,
        );
    }
    let c = kor_runtime::consensus::run(&targets, &prompt).await;
    serde_json::to_value(&c).map_err(|e| e.to_string())
}

fn shadow() -> Value {
    let log = kor_runtime::kor_home()
        .map(|h| kor_runtime::consensus::read_log(&h.join("shadow.jsonl")))
        .unwrap_or_default();
    let mut pairs: Vec<(String, String)> = log
        .iter()
        .map(|e| (e.main.clone(), e.shadow.clone()))
        .collect();
    pairs.sort();
    pairs.dedup();
    Value::Array(
        pairs
            .into_iter()
            .filter_map(|(main, sh)| {
                kor_runtime::consensus::average(&log, &main, &sh)
                    .map(|(agreement, samples)| json!({ "main": main, "shadow": sh, "agreement": agreement, "samples": samples }))
            })
            .collect(),
    )
}

fn config(state: &AppState) -> kor_config::Config {
    root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map(|l| l.config)
        .unwrap_or_default()
}

fn days(args: &[Value], i: usize) -> Result<u32, String> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(30),
        Some(v) => v
            .as_u64()
            .filter(|d| (1..=365).contains(d))
            .and_then(|d| u32::try_from(d).ok())
            .ok_or_else(|| "Gün sayısı 1-365 olmalı".into()),
    }
}

fn report(cfg: &kor_config::Config, days: u32) -> kor_runtime::cost::Report {
    let entries = kor_runtime::usage::file()
        .map(|p| kor_runtime::usage::read(&p))
        .unwrap_or_default();
    kor_runtime::cost::report(
        &entries,
        &kor_runtime::cost::Prices::load(),
        kor_runtime::cost::now_ms(),
        days,
        &cfg.budget,
    )
}

fn summary(args: &[Value], state: &AppState) -> Res {
    let cfg = config(state);
    let r = report(&cfg, days(args, 0)?);
    let l = kor_i18n::Localizer::new(cfg.ui.language.resolve());
    Ok(json!({ "report": r, "lines": kor_runtime::cost::report_lines(&l, &r) }))
}

fn export(args: &[Value], state: &AppState) -> Res {
    let format = s(args, 0, "cost.export")?;
    let r = report(&config(state), days(args, 1)?);
    let text = match format {
        "csv" => kor_runtime::cost::to_csv(&r),
        "json" => serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?,
        _ => return Err("Biçim csv ya da json olmalı".into()),
    };
    Ok(json!({ "text": text }))
}

/// `export` + kaydetme penceresi; dosyayı kullanıcının seçtiği yola yazar.
async fn export_file(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let text = export(args, state)?["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let ext = s(args, 0, "cost.exportFile")?;
    let name = format!("kor-cost.{ext}");
    let Some(path) = crate::api::save_dialog(app, None, &name).await else {
        return Ok(json!({ "path": null }));
    };
    std::fs::write(&path, text).map_err(|e| format!("Dosya yazılamadı: {e}"))?;
    Ok(json!({ "path": path.to_string_lossy() }))
}

fn why(args: &[Value]) -> Res {
    let last = match args.first() {
        None | Some(Value::Null) => 20,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=1000).contains(n))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("Karar sayısı 1-1000 olmalı")?,
    };
    let all = kor_runtime::cost::decisions_file()
        .map(|p| kor_runtime::cost::decisions(&p))
        .unwrap_or_default();
    Ok(json!({ "decisions": &all[all.len().saturating_sub(last)..] }))
}

async fn quota(state: &AppState) -> Res {
    let cfg = config(state);
    let l = kor_i18n::Localizer::new(cfg.ui.language.resolve());
    let r = kor_runtime::quota::report(&l, &cfg).await;
    // §4.3, §15.10: the subscription accounts and keys of a running gateway.
    let proxy = crate::ns::proxy::usage()
        .await
        .map(|(url, v)| json!({ "url": url, "accounts": kor_proxy::usage::account_rows(&v) }));
    Ok(json!({ "lines": kor_runtime::quota::lines(&l, &r), "report": r, "proxy": proxy }))
}
