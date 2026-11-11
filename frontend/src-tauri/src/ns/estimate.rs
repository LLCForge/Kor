//! `estimate.task(text, sessionId?, model?)`: composer altındaki süre/maliyet tahmini (§27.16,
//! ADR 0063) → `{ estimate, line, method, warning }`. Her değer ≈; iş mantığı
//! `kor_runtime::estimate`. Görev başlayınca ajan aynı tahmini `estimate` olayıyla da gönderir.

use kor_i18n::Localizer;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "estimate";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if name != "task" {
        return None;
    }
    Some(task(args, state))
}

fn task(args: &[Value], state: &AppState) -> Res {
    let text = s(args, 0, "estimate.task")?;
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    let loaded = root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok());
    let l = Localizer::new(
        loaded
            .as_ref()
            .map_or(kor_i18n::Lang::En, |c| c.config.ui.language.resolve()),
    );
    let model = opt_s(args, 2)
        .filter(|m| !m.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            loaded
                .as_ref()
                .map(|c| c.config.agent.model.trim().to_owned())
                .filter(|m| !m.is_empty())
        })
        .ok_or("Model seçilmedi")?;
    let context = opt_s(args, 1)
        .filter(|id| kor_runtime::session_tree::valid_id(id))
        .and_then(|id| kor_runtime::sessions_dir().map(|d| d.join(format!("{id}.jsonl"))))
        .filter(|p| p.is_file())
        .map_or(kor_runtime::estimate::BASE_CONTEXT, |p| {
            kor_runtime::estimate::session_context(&p)
        });
    let runs = kor_runtime::estimate::file()
        .map(|p| kor_runtime::estimate::read(&p))
        .unwrap_or_default();
    let e = kor_runtime::estimate::estimate(
        text,
        &model,
        context,
        &kor_runtime::cost::Prices::load(),
        &runs,
    );
    let warning = loaded.as_ref().and_then(|c| {
        let entries = kor_runtime::usage::file()
            .map(|p| kor_runtime::usage::read(&p))
            .unwrap_or_default();
        let r = kor_runtime::cost::report(
            &entries,
            &kor_runtime::cost::Prices::load(),
            kor_runtime::cost::now_ms(),
            30,
            &c.config.budget,
        );
        kor_runtime::estimate::budget_warning(&l, &e, &c.config.budget, r.today.usd, r.month.usd)
    });
    Ok(json!({
        "estimate": e,
        "line": kor_runtime::estimate::line(&l, &e),
        "method": kor_runtime::estimate::method_line(&l, &e),
        "warning": warning,
    }))
}
