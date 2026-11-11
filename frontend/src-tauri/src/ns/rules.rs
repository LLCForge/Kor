//! `rules.*`: kural uyum denetçisi (§28.3, ADR 0063). İş mantığı `kor_runtime::compliance` ve
//! `kor_tools::rules_check`. Ajanın kendi denetimi `confirm` olayındaki `rules` alanında gelir.
//!
//! - `rules.check(staged, judge)` → `{ findings, files, judged, error }` (judge: yapılandırılmış
//!   modelle düz metin kurallarını değerlendir; maliyeti kullanım defterine ayrı yazılır)
//! - `rules.list()` → `[{ source, line, rule, checkable }]`
//! - `rules.report(days)` → `[{ source, rule, model, count }]`

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root};
use crate::state::AppState;

pub const NS: &str = "rules";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(name, "check" | "list" | "report") {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "list" => {
            let compat = kor_runtime::load_compat(&root);
            let set = kor_runtime::compliance::rule_set(&compat);
            let display = kor_runtime::compliance::display(&root);
            Ok(json!(set
                .rules
                .iter()
                .map(|r| json!({ "source": display(&r.source), "line": r.line, "rule": r.text, "checkable": r.checkable() }))
                .collect::<Vec<_>>()))
        }
        "report" => {
            let days = args
                .first()
                .and_then(Value::as_u64)
                .unwrap_or(7)
                .clamp(1, 365);
            let since = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
                .saturating_sub(days * 86_400);
            let rows = kor_runtime::compliance::log_path()
                .map(|p| kor_tools::rules_check::report(&p, since))
                .unwrap_or_default();
            Ok(json!(
                rows.iter()
                    .map(|(s, r, m, n)| json!({ "source": s, "rule": r, "model": m, "count": n }))
                    .collect::<Vec<_>>()
            ))
        }
        _ => check(&root, b(args, 0), b(args, 1)).await,
    })
}

async fn check(root: &std::path::Path, staged: bool, judge: bool) -> Res {
    let compat = kor_runtime::load_compat(root);
    let set = kor_runtime::compliance::rule_set(&compat);
    let display = kor_runtime::compliance::display(root);
    let changes = kor_runtime::compliance::working_changes(root, None, staged)?;
    let mut findings = set.check(&changes, &display);
    let mut judged = false;
    let mut error = None;
    if judge && !changes.is_empty() {
        let loaded = crate::bridge::load_config(root)?;
        let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
        match kor_runtime::oneshot_provider(&l, &loaded, None) {
            Ok((provider, model)) => {
                match kor_runtime::compliance::judge(&provider, &model, findings.clone(), &changes)
                    .await
                {
                    Ok(f) => {
                        findings = f;
                        judged = true;
                    }
                    Err(e) => error = Some(e),
                }
            }
            Err(e) => error = Some(e),
        }
    }
    Ok(json!({ "findings": findings, "files": changes.len(), "judged": judged, "error": error }))
}
