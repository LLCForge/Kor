//! Request and token counters per credential and model (port of CLIProxyAPI `internal/usage`,
//! in memory only). Never holds prompts or responses, only counts.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use kor_core::Usage;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counter {
    pub requests: u64,
    pub success: u64,
    pub failed: u64,
    pub rate_limited: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub last_used: u64,
}

#[derive(Debug, Default)]
pub struct Stats {
    /// (credential label, model) → counters.
    map: Mutex<BTreeMap<(String, String), Counter>>,
}

impl Stats {
    fn with(&self, cred: &str, model: &str, f: impl FnOnce(&mut Counter)) {
        let mut m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        let c = m.entry((cred.to_owned(), model.to_owned())).or_default();
        c.last_used = crate::translate::now_secs();
        f(c);
    }

    pub fn request(&self, cred: &str, model: &str, ok: bool, rate_limited: bool) {
        self.with(cred, model, |c| {
            c.requests += 1;
            if ok {
                c.success += 1;
            } else {
                c.failed += 1;
            }
            if rate_limited {
                c.rate_limited += 1;
            }
        });
    }

    pub fn tokens(&self, cred: &str, model: &str, u: &Usage) {
        self.with(cred, model, |c| {
            c.input_tokens = c.input_tokens.saturating_add(u.input);
            c.output_tokens = c.output_tokens.saturating_add(u.output);
        });
    }

    pub fn snapshot(&self) -> BTreeMap<(String, String), Counter> {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `{"usage": {total_requests, success_count, failure_count, total_tokens,
    /// apis: {<credential>: {total_requests, total_tokens, models: {<model>: {…}}}}}}`, the shape
    /// of CLIProxyAPI's usage statistics.
    pub fn to_json(&self) -> Value {
        let snap = self.snapshot();
        let mut apis: BTreeMap<String, Value> = BTreeMap::new();
        let (mut req, mut ok, mut bad, mut tok) = (0u64, 0u64, 0u64, 0u64);
        for ((cred, model), c) in &snap {
            let total = c.input_tokens.saturating_add(c.output_tokens);
            req += c.requests;
            ok += c.success;
            bad += c.failed;
            tok = tok.saturating_add(total);
            let api = apis.entry(cred.clone()).or_insert_with(
                || json!({"total_requests": 0u64, "total_tokens": 0u64, "models": {}}),
            );
            api["total_requests"] =
                (api["total_requests"].as_u64().unwrap_or(0) + c.requests).into();
            api["total_tokens"] = (api["total_tokens"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(total))
            .into();
            api["models"][model] = json!({
                "total_requests": c.requests, "success_count": c.success, "failure_count": c.failed,
                "rate_limited": c.rate_limited, "input_tokens": c.input_tokens,
                "output_tokens": c.output_tokens, "total_tokens": total, "last_used": c.last_used,
            });
        }
        json!({"usage": {"total_requests": req, "success_count": ok, "failure_count": bad,
                         "total_tokens": tok, "apis": apis}})
    }
}

/// One credential (account or key) of a running gateway, for `kor quota` and the cost panel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRow {
    pub account: String,
    pub requests: u64,
    pub failed: u64,
    pub rate_limited: u64,
    pub tokens: u64,
    /// Unix seconds of the last request.
    pub last_used: u64,
}

/// [`Stats::to_json`] (as served on `/v0/management/usage`, also by CLIProxyAPI) per account,
/// busiest first.
pub fn account_rows(usage: &Value) -> Vec<AccountRow> {
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let mut rows: Vec<AccountRow> = usage["usage"]["apis"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(account, api)| {
            let mut r = AccountRow {
                account: account.clone(),
                requests: n(&api["total_requests"]),
                tokens: n(&api["total_tokens"]),
                ..AccountRow::default()
            };
            for m in api["models"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, m)| m)
            {
                r.failed += n(&m["failure_count"]);
                r.rate_limited += n(&m["rate_limited"]);
                r.last_used = r.last_used.max(n(&m["last_used"]));
            }
            r
        })
        .collect();
    rows.sort_by(|a, b| {
        b.requests
            .cmp(&a.requests)
            .then_with(|| a.account.cmp(&b.account))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_from_the_usage_document() {
        let s = Stats::default();
        s.request("claude:a@x", "m1", true, false);
        s.request("claude:a@x", "m2", false, true);
        s.request("codex:b@y", "m1", true, false);
        s.tokens(
            "codex:b@y",
            "m1",
            &Usage {
                input: 5,
                output: 5,
                ..Usage::default()
            },
        );
        let rows = account_rows(&s.to_json());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].account, "claude:a@x");
        assert_eq!(
            (rows[0].requests, rows[0].failed, rows[0].rate_limited),
            (2, 1, 1)
        );
        assert_eq!(rows[1].tokens, 10);
        assert!(account_rows(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn counts_roll_up_per_credential() {
        let s = Stats::default();
        s.request("a", "m1", true, false);
        s.request("a", "m1", false, true);
        s.tokens(
            "a",
            "m1",
            &Usage {
                input: 3,
                output: 4,
                ..Usage::default()
            },
        );
        s.request("a", "m2", true, false);
        let v = s.to_json();
        assert_eq!(v["usage"]["total_requests"], 3);
        assert_eq!(v["usage"]["failure_count"], 1);
        assert_eq!(v["usage"]["total_tokens"], 7);
        assert_eq!(v["usage"]["apis"]["a"]["total_requests"], 3);
        assert_eq!(v["usage"]["apis"]["a"]["models"]["m1"]["rate_limited"], 1);
    }
}
