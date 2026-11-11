//! `metrics.*`: kod metrikleri (§9 `kor explain|quality|debt|style|stats|impact`, §15.14
//! `kor dead-code`, ADR 0047) için ince köprü. Ölçüm `kor_runtime::analysis::metrics` içinde;
//! burası yalnızca JSON'a çevirir. Graf, grafik panelinin paylaştığı kopyadır (`graph_for`).
//!
//! - `metrics.explain(file, level?: "junior"|"normal"|"expert", offline?: bool)` →
//!   `{ facts, explanation: string|null, explanationError: string|null }`
//! - `metrics.comment(file, lang, missing?, onlyMissing?, write?)` → `{ diff, translated, added, kept, written }`
//!   (`kor comment`, §12.2; dosya proje kökünün dışına çıkamaz)
//! - `metrics.quality()` → `QualityReport` (anlık görüntü `.kor/quality/<commit>.json`'a yazılır)
//! - `metrics.qualityHistory()` → `{ snapshots }`
//! - `metrics.debt(thresholdHours?: number)` → `Debt`
//! - `metrics.style()` → `{ profile, saved: bool }` (proje belleğine `patterns.md` / `style.json`)
//! - `metrics.styleCheck(staged?: bool)` → `{ report }`
//! - `metrics.stats(weeks?: number)` → `Stats` (takım sıralaması yalnızca `[stats] team = true`)
//! - `metrics.impact(target?: string, mode?: "name"|"working"|"staged", depth?: number)` → `ImpactReport`
//! - `metrics.deadCode()` → `DeadReport`
//! - `metrics.deadCodePrune(includeMedium?: bool, checkCmd?: string, testCmd?: string)` → `PrunePlan`
//!   (hiçbir şey gönderilmez; doğrulanan dal yerelde kalır)
//! - `metrics.deadCodePublish(branch)` → `{ url, number, forge }`: yalnızca bu süreçte hazırlanmış
//!   ve doğrulanmış dal için; arayüz onay penceresinden sonra çağırır.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use kor_runtime::analysis::metrics::{
    self, deadcode, debt, explain, impact, quality, stats, style,
};
use kor_runtime::review::DiffSource;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `metrics::call`'a yönlendirir).
pub const NS: &str = "metrics";

/// Son doğrulanmış `--prune` planı; `deadCodePublish` yalnızca onu gönderir.
static PLAN: Mutex<Option<(PathBuf, deadcode::PrunePlan)>> = Mutex::new(None);

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let dir = match root(state) {
        Ok(d) => d,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "explain" => explain_call(args, &dir).await,
        "comment" => comment_call(args, &dir).await,
        "quality" => {
            let l = localizer(&dir);
            blocking(move || to_json(&quality::run(&dir, &l))).await
        }
        "qualityHistory" => Ok(json!({ "snapshots": quality::history(&dir) })),
        "debt" => {
            let th = threshold(args, &dir);
            blocking(move || {
                let p = metrics::Project::gather(&dir);
                to_json(&debt::debt(&dir, &p, th))
            })
            .await
        }
        "style" => blocking(move || style_call(&dir)).await,
        "styleCheck" => {
            let src = if b(args, 0) {
                DiffSource::Staged
            } else {
                DiffSource::Working
            };
            let l = localizer(&dir);
            blocking(move || {
                let home = kor_runtime::kor_home().ok_or("Kor ana dizini bulunamadı")?;
                style::check(&home, &dir, &src, &l).map(|r| json!({ "report": r }))
            })
            .await
        }
        "stats" => {
            let weeks = num(args, 0).unwrap_or(4).clamp(1, 52);
            let team = crate::bridge::load_config(&dir).is_ok_and(|l| l.config.stats.team);
            blocking(move || {
                let o = stats::Options {
                    author: None,
                    weeks,
                    team,
                    audit: kor_runtime::secure::audit_path(),
                };
                stats::stats(&dir, &o).and_then(|s| to_json(&s))
            })
            .await
        }
        "impact" => impact_call(args, app, &dir).await,
        "deadCode" => match super::graph::graph_for(app, &dir) {
            Ok(g) => {
                blocking(move || {
                    let g = g.lock().unwrap_or_else(PoisonError::into_inner);
                    to_json(&deadcode::find(&dir, &g))
                })
                .await
            }
            Err(e) => Err(e),
        },
        "deadCodePrune" => prune(args, app, &dir).await,
        "deadCodePublish" => match s(args, 0, "metrics.deadCodePublish") {
            Ok(branch) => publish(branch, &dir).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

fn localizer(dir: &Path) -> kor_i18n::Localizer {
    let lang = crate::bridge::load_config(dir)
        .map_or(kor_i18n::Lang::Tr, |l| l.config.ui.language.resolve());
    kor_i18n::Localizer::new(lang)
}

async fn blocking(f: impl FnOnce() -> Res + Send + 'static) -> Res {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Ölçüm çalıştırılamadı: {e}"))?
}

fn to_json(v: &impl serde::Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn num(args: &[Value], i: usize) -> Option<u32> {
    args.get(i)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn threshold(args: &[Value], dir: &Path) -> f64 {
    let cfg = crate::bridge::load_config(dir).map_or(0, |l| l.config.stats.debt_threshold_hours);
    match num(args, 0)
        .filter(|n| (1..=1_000_000).contains(n))
        .unwrap_or(cfg)
    {
        0 => debt::DEFAULT_THRESHOLD_HOURS,
        h => f64::from(h),
    }
}

fn level(args: &[Value]) -> Result<explain::Level, String> {
    match opt_s(args, 1) {
        None => Ok(explain::Level::Normal),
        Some(x) => explain::Level::parse(x)
            .ok_or_else(|| "Düzey \"junior\", \"normal\" ya da \"expert\" olmalı".into()),
    }
}

async fn explain_call(args: &[Value], dir: &Path) -> Res {
    let file = s(args, 0, "metrics.explain")?.to_owned();
    let lv = level(args)?;
    let offline = b(args, 2);
    let d = dir.to_path_buf();
    let facts =
        blocking(move || explain::archaeology(&d, &file, None).and_then(|a| to_json(&a))).await?;
    if offline {
        return Ok(json!({ "facts": facts, "explanation": null, "explanationError": null }));
    }
    let arch: explain::Archaeology =
        serde_json::from_value(facts.clone()).map_err(|e| e.to_string())?;
    let l = localizer(dir);
    let language = match l.lang() {
        kor_i18n::Lang::Tr => "Turkish",
        kor_i18n::Lang::Fr => "French",
        kor_i18n::Lang::En => "English",
    };
    let loaded = crate::bridge::load_config(dir)?;
    // Gizlilik kipinde yerel olmayan sağlayıcıyı `oneshot_provider` reddeder.
    let (text, err) = match kor_runtime::oneshot_provider(&l, &loaded, None) {
        Ok((provider, model)) => {
            let source = std::fs::read_to_string(dir.join(&arch.file)).unwrap_or_default();
            let prompt = explain::prompt(&arch, lv, language, &source);
            match kor_agent::gitflow::ask(&provider, &model, prompt, 1500).await {
                Ok(t) => (Some(t), None),
                Err(e) => (None, Some(e.to_string())),
            }
        }
        Err(e) => (None, Some(e)),
    };
    Ok(json!({ "facts": facts, "explanation": text, "explanationError": err }))
}

async fn comment_call(args: &[Value], dir: &Path) -> Res {
    let rel = s(args, 0, "metrics.comment")?;
    let target = s(args, 1, "metrics.comment")?.trim().to_owned();
    if target.is_empty() || target.len() > 40 {
        return Err("Hedef dil geçersiz".into());
    }
    let (missing, only, write) = (b(args, 2), b(args, 3), b(args, 4));
    // SECURITY FIX: CWE-22 — yalnızca proje kökü içindeki dosyalar okunur/yazılır.
    let base = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let file = std::fs::canonicalize(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    if !file.starts_with(&base) || !file.is_file() {
        return Err(format!("{rel}: proje klasöründe bir dosya değil"));
    }
    let lang = file
        .extension()
        .and_then(|e| e.to_str())
        .and_then(kor_graph::lang::Lang::from_ext)
        .ok_or_else(|| format!("{rel}: bu dil desteklenmiyor"))?;
    let src = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let l = localizer(dir);
    let loaded = crate::bridge::load_config(dir)?;
    let (provider, model) = kor_runtime::oneshot_provider(&l, &loaded, None)?;
    let out = kor_runtime::comment::run(&provider, &model, lang, &src, &target, !only, missing || only).await?;
    let changed = out.text != src;
    if write && changed {
        std::fs::write(&file, &out.text).map_err(|e| e.to_string())?;
    }
    Ok(json!({
        "diff": if changed { kor_agent::port::unified_diff(rel, &src, &out.text) } else { String::new() },
        "translated": out.translated,
        "added": out.added,
        "kept": out.kept,
        "written": write && changed,
    }))
}

fn style_call(dir: &Path) -> Res {
    let home = kor_runtime::kor_home().ok_or("Kor ana dizini bulunamadı")?;
    let profile = style::profile(dir, None)?;
    let saved = style::save(&home, dir, &profile).map_err(|e| e.to_string())?;
    Ok(json!({ "profile": profile, "saved": saved }))
}

fn impact_target(args: &[Value]) -> Result<impact::Target, String> {
    match opt_s(args, 1).unwrap_or("name") {
        "working" => Ok(impact::Target::Diff(DiffSource::Working)),
        "staged" => Ok(impact::Target::Diff(DiffSource::Staged)),
        "name" => opt_s(args, 0)
            .map(str::trim)
            .filter(|t| !t.is_empty() && t.len() <= 500)
            .map(|t| impact::Target::Name(t.to_owned()))
            .ok_or_else(|| "Bir dosya ya da sembol adı girin".into()),
        _ => Err("Kip \"name\", \"working\" ya da \"staged\" olmalı".into()),
    }
}

async fn impact_call(args: &[Value], app: &AppHandle, dir: &Path) -> Res {
    let target = impact_target(args)?;
    let depth = num(args, 2).unwrap_or(impact::DEFAULT_DEPTH).clamp(1, 10);
    let g = super::graph::graph_for(app, dir)?;
    let d = dir.to_path_buf();
    blocking(move || {
        let g = g.lock().unwrap_or_else(PoisonError::into_inner);
        impact::impact(&d, &g, &target, depth).and_then(|r| to_json(&r))
    })
    .await
}

async fn prune(args: &[Value], app: &AppHandle, dir: &Path) -> Res {
    let g = super::graph::graph_for(app, dir)?;
    let d = dir.to_path_buf();
    let report = blocking(move || {
        let g = g.lock().unwrap_or_else(PoisonError::into_inner);
        to_json(&deadcode::find(&d, &g))
    })
    .await?;
    let report: deadcode::DeadReport = serde_json::from_value(report).map_err(|e| e.to_string())?;
    let opts = deadcode::PruneOptions {
        include_medium: b(args, 0),
        check: opt_s(args, 1)
            .map(str::to_owned)
            .filter(|c| !c.trim().is_empty()),
        test: opt_s(args, 2)
            .map(str::to_owned)
            .filter(|c| !c.trim().is_empty()),
    };
    let loaded = crate::bridge::load_config(dir)?;
    let l = localizer(dir);
    let plan = deadcode::prune(dir, &report, &opts, &loaded.config, &l).await?;
    let out = to_json(&plan)?;
    *PLAN.lock().unwrap_or_else(PoisonError::into_inner) =
        plan.ready.then(|| (dir.to_path_buf(), plan));
    Ok(out)
}

async fn publish(branch: &str, dir: &Path) -> Res {
    let plan = PLAN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .filter(|(d, p)| d == dir && p.branch == branch)
        .map(|(_, p)| p.clone())
        .ok_or("Bu dal için doğrulanmış bir silme planı yok; önce \"Doğrula\"yı çalıştırın")?;
    let pr = deadcode::publish(dir, &plan).await.map_err(|e| {
        let l = localizer(dir);
        kor_runtime::forge::error_text(&l, &e)
    })?;
    *PLAN.lock().unwrap_or_else(PoisonError::into_inner) = None;
    to_json(&pr)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{impact_target, level, num};
    use kor_runtime::analysis::metrics::{explain, impact};

    #[test]
    fn arguments_are_checked() {
        assert_eq!(
            level(&[json!("a.rs"), json!("junior")]).unwrap(),
            explain::Level::Junior
        );
        assert!(level(&[json!("a.rs"), json!("guru")]).is_err());
        assert_eq!(level(&[json!("a.rs")]).unwrap(), explain::Level::Normal);
        assert!(matches!(
            impact_target(&[json!("src/db.rs")]),
            Ok(impact::Target::Name(_))
        ));
        assert!(matches!(
            impact_target(&[json!(null), json!("staged")]),
            Ok(impact::Target::Diff(_))
        ));
        assert!(impact_target(&[json!("  ")]).is_err());
        assert!(impact_target(&[json!("x"), json!("bogus")]).is_err());
        assert_eq!(num(&[json!(3)], 0), Some(3));
        assert_eq!(num(&[json!("3")], 0), None);
    }
}
