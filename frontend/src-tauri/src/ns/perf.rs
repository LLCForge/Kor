//! `perf.*`: `kor perf` için ince köprü (§15.8, ADR 0049). Profiler seçimi, çalıştırma, ayrıştırma,
//! flamegraph ve graf eşlemesi `kor_runtime::perf` içinde.
//!
//! - `perf.run({ target, profiler?, duration?, requests?, concurrency?, pid?, cmd?, allowHosts?, baseline? })`
//!   → `Report`. Çalıştırma korumalı alanın dışına çıkacaksa (ptrace/perf_event isteyen profiler'lar,
//!   uç sunucuları) önce yerel onay penceresi açılır; red → hata.
//! - `perf.list()` → `Summary[]` (yeniden eskiye)
//! - `perf.load(id)` → `{ report, flamegraph, memoryFlamegraph }` (SVG metinleri; betik içermez)
//! - `perf.report()` → dosya seçme penceresi, sonra `Report` (iptalde `null`)
//! - `perf.optimizePrompt(id)` → en sıcak proje fonksiyonu için ajan istemi ya da `null`

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `perf::call`'a yönlendirir).
pub const NS: &str = "perf";

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// Arayüzden gelen seçenekler; bilinmeyen alanlar yok sayılır, sayılar sınırlanır.
fn options(
    root: std::path::PathBuf,
    v: &Value,
    lang: kor_i18n::Lang,
) -> Result<kor_runtime::perf::Options, String> {
    let target = v
        .get("target")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or("`perf.run`: hedef gerekli")?;
    let text = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let num = |k: &str| v.get(k).and_then(Value::as_u64);
    let mut o = kor_runtime::perf::Options::new(root, target, lang);
    o.profiler = text("profiler");
    o.duration = num("duration");
    o.server_cmd = text("cmd");
    o.baseline = text("baseline");
    o.pid = num("pid").and_then(|p| u32::try_from(p).ok());
    if let Some(n) = num("requests") {
        o.requests = u32::try_from(n).unwrap_or(u32::MAX);
    }
    if let Some(n) = num("concurrency") {
        o.concurrency = u32::try_from(n).unwrap_or(u32::MAX);
    }
    o.allow_hosts = v
        .get("allowHosts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .take(16)
                .collect()
        })
        .unwrap_or_default();
    Ok(o)
}

async fn run(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let root = root(state)?;
    let loaded = crate::bridge::load_config(&root)?;
    let lang = loaded.config.ui.language.resolve();
    let opts = options(root, args.first().unwrap_or(&Value::Null), lang)?;
    let plan = kor_runtime::perf::plan(&opts, &loaded.config)?;
    // Onay arka uçta istenir: arayüz bu adımı atlayamaz.
    if let Some(q) = &plan.ask
        && !crate::api::confirm(app, "Korumalı alan dışında çalıştır", q, "Çalıştır").await
    {
        return Err(kor_i18n::Localizer::new(lang).t("perf-denied"));
    }
    to_json(&kor_runtime::perf::execute(plan).await?)
}

fn id_arg<'a>(args: &'a [Value], method: &str) -> Result<&'a str, String> {
    let id = s(args, 0, method)?;
    if kor_runtime::valid_session_id(id) {
        Ok(id)
    } else {
        Err("Geçersiz çalıştırma kimliği".into())
    }
}

fn load(args: &[Value], state: &AppState) -> Res {
    let root = root(state)?;
    let id = id_arg(args, "perf.load")?;
    let report = kor_runtime::perf::load_report(&root, id)?;
    let flame = kor_runtime::perf::run::flamegraph(&root, id, false).ok();
    let memory = report
        .memory_flamegraph
        .as_ref()
        .and_then(|_| kor_runtime::perf::run::flamegraph(&root, id, true).ok());
    Ok(json!({ "report": report, "flamegraph": flame, "memoryFlamegraph": memory }))
}

async fn report(app: &AppHandle, state: &AppState) -> Res {
    let root = root(state)?;
    let Some(path) = crate::api::open_dialog(app, Some(root.as_path())).await else {
        return Ok(Value::Null);
    };
    let lang = crate::bridge::load_config(&root)
        .map_or(kor_i18n::Lang::En, |l| l.config.ui.language.resolve());
    to_json(&kor_runtime::perf::report_file(&root, &path, lang)?)
}

fn optimize_prompt(args: &[Value], state: &AppState) -> Res {
    let root = root(state)?;
    let r = kor_runtime::perf::load_report(&root, id_arg(args, "perf.optimizePrompt")?)?;
    Ok(kor_runtime::perf::optimize_prompt(&r).map_or(Value::Null, Value::String))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "run" => run(args, app, state).await,
        "list" => root(state).and_then(|r| to_json(&kor_runtime::perf::list(&r))),
        "load" => load(args, state),
        "report" => report(app, state).await,
        "optimizePrompt" => optimize_prompt(args, state),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_checked() {
        let root = std::path::PathBuf::from(".");
        let o = options(
            root.clone(),
            &json!({ "target": " http://127.0.0.1:3000/x ", "requests": 50, "concurrency": 4, "cmd": "node s.js", "pid": 99999999999u64, "allowHosts": ["a", 3], "extra": true }),
            kor_i18n::Lang::Tr,
        )
        .unwrap();
        assert_eq!(o.target, "http://127.0.0.1:3000/x");
        assert_eq!((o.requests, o.concurrency, o.pid), (50, 4, None));
        assert_eq!(
            (o.server_cmd.as_deref(), o.allow_hosts.as_slice()),
            (Some("node s.js"), &["a".to_owned()][..])
        );
        assert!(options(root.clone(), &json!({ "target": "  " }), kor_i18n::Lang::Tr).is_err());
        assert!(options(root, &Value::Null, kor_i18n::Lang::Tr).is_err());
    }

    #[test]
    fn only_run_ids_are_accepted() {
        assert_eq!(
            id_arg(&[json!("20261110-101010-abcd")], "perf.load"),
            Ok("20261110-101010-abcd")
        );
        for bad in [json!("../x"), json!("C:\\x"), json!(1)] {
            assert!(id_arg(&[bad], "perf.load").is_err());
        }
    }
}
