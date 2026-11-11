//! What the side panels show, from the same core state the CLI commands and the desktop read:
//! the session's cost control (`kor cost`), `kor quota`, provider health (`kor provider status`)
//! and the tool context's background jobs. Plain files, no locks held.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use kor_i18n::{Localizer, args};

/// Background jobs: (running, total).
pub fn jobs(jobs: &Arc<Mutex<HashMap<String, kor_tools::Job>>>) -> (usize, usize) {
    let g = jobs.lock().unwrap_or_else(PoisonError::into_inner);
    (g.values().filter(|j| !j.done).count(), g.len())
}

/// This session's spend per model and the month line, as at the end of a session.
pub fn cost_lines(l: &Localizer, cost: &kor_runtime::cost::Control) -> Vec<String> {
    let lines = cost.summary_lines(l);
    if lines.is_empty() {
        vec![l.t("tui-cost-none")]
    } else {
        lines.into_iter().map(|s| s.trim_end().to_owned()).collect()
    }
}

/// `kor quota` text. Network only for `OpenRouter` credits, never in privacy mode.
pub async fn quota_lines(l: &Localizer, config: &kor_config::Config) -> Vec<String> {
    let r = kor_runtime::quota::report(l, config).await;
    kor_runtime::quota::lines(l, &r)
}

/// Recorded provider health (latency, error rate, last error); passive, nothing is probed.
pub fn health_lines(l: &Localizer) -> Vec<String> {
    if let Some(d) = dirs::cache_dir() {
        // Same file as `kor provider status`.
        kor_providers::health::set_store(d.join("kor").join("provider-health.json"));
    }
    let snap = kor_providers::health::snapshot();
    if snap.is_empty() {
        return vec![l.t("tui-health-none")];
    }
    snap.iter()
        .map(|(id, s)| {
            let mut line = l.t_args(
                "tui-health-line",
                &args!(
                    "id" => id.clone(),
                    "latency" => s.latency_ms().map_or_else(|| "-".into(), |ms| format!("{ms} ms")),
                    "errors" => format!("{:.0}", s.error_rate() * 100.0)
                ),
            );
            if let Some(e) = &s.last_error {
                line.push_str(" · ");
                line.extend(e.chars().take(60));
            }
            line
        })
        .collect()
}
