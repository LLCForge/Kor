//! Provider quota dashboard (Master Build Prompt §15.10, `kor quota`): what is left on each key
//! and when it resets (from the providers' rate-limit headers), the last 24 hours from the usage
//! ledger, `OpenRouter` credits, and the failover order. Nothing is estimated here.

use std::collections::BTreeMap;

use kor_i18n::{Localizer, args};
use kor_providers::quota::KeyQuota;
use kor_secrets::{ExposeSecret, KeychainStore};
use serde::Serialize;

use crate::cost::{fmt_time, now_ms};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KeyRow {
    pub provider: String,
    pub key: String,
    pub remaining_requests: Option<u64>,
    pub limit_requests: Option<u64>,
    pub remaining_tokens: Option<u64>,
    pub limit_tokens: Option<u64>,
    /// Local time, or the provider's own text when it is not a duration.
    pub reset: Option<String>,
    pub used_percent: Option<f64>,
    /// Nearly exhausted: warn and fail over soon.
    pub low: bool,
    /// When the headers were seen (local time).
    pub seen: String,
}

impl From<&KeyQuota> for KeyRow {
    fn from(k: &KeyQuota) -> Self {
        Self {
            provider: k.provider.clone(),
            key: k.key.clone(),
            remaining_requests: k.quota.remaining_requests,
            limit_requests: k.quota.limit_requests,
            remaining_tokens: k.quota.remaining_tokens,
            limit_tokens: k.quota.limit_tokens,
            reset: k
                .reset_at()
                .map(|s| fmt_time(s.saturating_mul(1000)))
                .or_else(|| k.quota.reset.clone()),
            used_percent: k.used_share().map(|s| (s * 1000.0).round() / 10.0),
            low: k.low(),
            seen: fmt_time(k.at.saturating_mul(1000)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DayRow {
    pub provider: String,
    pub requests: u64,
    pub prompt: u64,
    pub completion: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    pub keys: Vec<KeyRow>,
    pub last_24h: Vec<DayRow>,
    /// `provider/model` in the order they are tried, each with its keys in pool order.
    pub failover: Vec<String>,
    /// `OpenRouter` (total credits, used), USD.
    pub openrouter: Option<(f64, f64)>,
    pub notes: Vec<String>,
}

/// Requests and tokens per provider since `since` (ms), from the shared ledger.
pub fn last_day(entries: &[crate::usage::Entry], since: u64) -> Vec<DayRow> {
    let mut m: BTreeMap<&str, DayRow> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.at >= since) {
        let p = e.model.split('/').next().unwrap_or_default();
        let r = m.entry(p).or_default();
        r.requests += 1;
        r.prompt += e.prompt;
        r.completion += e.completion;
    }
    m.into_iter()
        .map(|(p, mut r)| {
            p.clone_into(&mut r.provider);
            r
        })
        .collect()
}

fn mask(k: &kor_secrets::SecretString) -> String {
    let c: Vec<char> = k.expose_secret().chars().collect();
    if c.len() < 12 {
        return "••••".into();
    }
    format!("…{}", c[c.len() - 4..].iter().collect::<String>())
}

/// Everything `kor quota` shows. Network only for `OpenRouter` credits, never in privacy mode.
pub async fn report(l: &Localizer, config: &kor_config::Config) -> Report {
    let privacy = config.privacy.mode || crate::privacy_mode();
    if let Some(dir) = kor_platform::cache_dir() {
        kor_providers::quota::set_store(dir.join("provider-quota.json"));
    }
    let mut r = Report {
        keys: kor_providers::quota::snapshot()
            .iter()
            .map(KeyRow::from)
            .collect(),
        ..Report::default()
    };
    let entries = crate::usage::file()
        .map(|p| crate::usage::read(&p))
        .unwrap_or_default();
    r.last_24h = last_day(&entries, now_ms().saturating_sub(86_400_000));

    let reg = crate::registry(l, &mut r.notes).ok();
    let chain = std::iter::once(config.agent.model.trim())
        .chain(config.routing.fallback.iter().map(|s| s.trim()))
        .filter(|s| !s.is_empty());
    for m in chain {
        let provider = m.split('/').next().unwrap_or_default();
        let keys = reg
            .as_ref()
            .and_then(|reg| reg.get(provider))
            .and_then(|man| kor_providers::pool::keys_for(man, &KeychainStore).ok())
            .unwrap_or_default();
        let keys: Vec<String> = keys.iter().map(mask).collect();
        r.failover.push(if keys.is_empty() {
            m.to_owned()
        } else {
            format!("{m} [{}]", keys.join(" → "))
        });
    }

    if !privacy
        && let Some(man) = reg.as_ref().and_then(|reg| reg.get("openrouter"))
        && let Ok(Some((key, _))) = crate::key_for(man)
    {
        match kor_providers::openrouter::credits(man, key).await {
            Ok(c) => r.openrouter = Some(c),
            Err(e) => r.notes.push(e.message),
        }
    }
    r
}

fn num(n: Option<u64>) -> String {
    n.map_or_else(|| "-".into(), |n| n.to_string())
}

/// The `kor quota` text.
pub fn lines(l: &Localizer, r: &Report) -> Vec<String> {
    let mut out = vec![l.t("quota-keys-head")];
    if r.keys.is_empty() {
        out.push(format!("  {}", l.t("quota-none")));
    }
    for k in &r.keys {
        let reset = k.reset.clone().unwrap_or_else(|| "-".into());
        out.push(format!(
            "  {:<24} {:<8} {}  {}  {}",
            k.provider,
            k.key,
            l.t_args(
                "quota-requests",
                &args!("left" => num(k.remaining_requests), "limit" => num(k.limit_requests))
            ),
            l.t_args(
                "quota-tokens",
                &args!("left" => num(k.remaining_tokens), "limit" => num(k.limit_tokens))
            ),
            l.t_args(
                "quota-reset",
                &args!("reset" => reset, "seen" => k.seen.clone())
            ),
        ));
        if k.low {
            out.push(format!(
                "    {}",
                l.t_args(
                    "quota-low",
                    &args!("percent" => format!("{:.0}", k.used_percent.unwrap_or(100.0)))
                )
            ));
        }
    }
    out.push(l.t("quota-day-head"));
    if r.last_24h.is_empty() {
        out.push(format!("  {}", l.t("cost-none")));
    }
    for d in &r.last_24h {
        out.push(format!(
            "  {:<24} {}  {}↑ {}↓",
            d.provider,
            l.t_args("cost-requests", &args!("count" => d.requests)),
            d.prompt,
            d.completion
        ));
    }
    if let Some((total, used)) = r.openrouter {
        out.push(l.t_args(
            "quota-openrouter",
            &args!("left" => format!("{:.2}", total - used), "total" => format!("{total:.2}")),
        ));
    }
    if !r.failover.is_empty() {
        out.push(l.t_args(
            "quota-failover",
            &args!("order" => r.failover.join("  →  ")),
        ));
    }
    out.extend(r.notes.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Entry;

    #[test]
    fn last_day_groups_by_provider() {
        let e = |at, model: &str| Entry {
            at,
            model: model.into(),
            prompt: 10,
            completion: 1,
            cost_usd: None,
            path: None,
            role: None,
            media: false,
            estimated: false,
            cache_read: 0,
            cache_write: 0,
        };
        let rows = last_day(
            &[e(5, "openrouter/a/b"), e(6, "openrouter/c"), e(1, "groq/x")],
            5,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].provider.as_str(), rows[0].requests, rows[0].prompt),
            ("openrouter", 2, 20)
        );
    }

    #[test]
    fn key_rows_and_lines_warn_when_low() {
        let k = KeyQuota {
            provider: "groq".into(),
            key: "…abcd".into(),
            quota: kor_providers::health::Quota {
                remaining_requests: Some(1),
                limit_requests: Some(100),
                reset: Some("30s".into()),
                ..Default::default()
            },
            at: 1_700_000_000,
        };
        let row = KeyRow::from(&k);
        assert!(row.low);
        assert_eq!(row.used_percent, Some(99.0));
        assert!(row.reset.is_some());
        let r = Report {
            keys: vec![row],
            failover: vec!["groq/llama".into(), "openrouter/auto".into()],
            ..Report::default()
        };
        let l = Localizer::new(kor_i18n::Lang::En);
        let text = lines(&l, &r).join("\n");
        assert!(text.contains("…abcd"));
        assert!(text.contains("99"));
        assert!(text.contains("groq/llama  →  openrouter/auto"));
    }
}
