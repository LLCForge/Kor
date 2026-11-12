//! Credential pool, selection and cooldown (port of CLIProxyAPI `sdk/cliproxy/auth` selection,
//! `internal/credentialweight` and the conductor's cooldown rules), for API-key credentials.
//!
//! * Candidates are the keys of every group that serves the requested model (aliases, `prefix/`,
//!   `excluded-models` wildcards, thinking suffix `model(high)` stripped).
//! * Higher `priority` first; inside a tier `routing.strategy` orders them: round-robin,
//!   smooth weighted round-robin (`weight`, 0 = excluded) or fill-first.
//! * 429 cools a key down for its `retry-after`, else an exponential backoff 1 s · 2ⁿ capped at
//!   30 min (`quotaBackoffBase/Max`); 408/5xx/timeouts for `transient-error-cooldown-seconds`
//!   (60 s). Cooling keys go last. `request-retry` extra rounds wait at most
//!   `max-retry-interval` for the first key to cool off.
//!
//! Upstream calls go through Kor's own adapters (`kor_providers::build`), never a copy.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use kor_core::{ProviderError, ProviderErrorKind};
use kor_providers::custom::{AuthMethod, custom_manifest};
use kor_providers::{Provider, ProviderManifest, Registry};
use kor_secrets::SecretString;

use crate::config::{Group, Kind, Settings, Strategy};

const QUOTA_BACKOFF_BASE: Duration = Duration::from_secs(1);
const QUOTA_BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Default)]
struct State {
    cooling_until: Option<Instant>,
    quota_level: u32,
    /// Smooth weighted round-robin credit.
    credit: i64,
    success: u64,
    failed: u64,
}

pub struct Credential {
    /// `<group>` or `<group>#<n>` (n = key position, 1-based).
    pub label: String,
    pub group: usize,
    pub kind: Kind,
    pub provider: Arc<dyn Provider>,
    pub masked_key: String,
    pub base_url: String,
    pub weight: u32,
    pub priority: i64,
    pub local: bool,
    disable_cooling: bool,
    state: Mutex<State>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("label", &self.label)
            .field("kind", &self.kind)
            .field("key", &self.masked_key)
            .finish_non_exhaustive()
    }
}

/// Public view of a credential for usage/status output. Keys are masked.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CredentialView {
    pub id: String,
    pub provider: &'static str,
    pub key: String,
    pub base_url: String,
    pub weight: u32,
    pub priority: i64,
    pub cooling_for_secs: u64,
    pub success: u64,
    pub failed: u64,
}

impl Credential {
    /// A subscription login (OAuth auth file) serving one group.
    #[allow(clippy::too_many_arguments)] // one field each of the credential row
    pub fn subscription(
        label: String,
        group: usize,
        kind: Kind,
        provider: Arc<dyn Provider>,
        masked_key: String,
        base_url: String,
        weight: u32,
        priority: i64,
    ) -> Self {
        Self {
            label,
            group,
            kind,
            provider,
            masked_key,
            base_url,
            weight: weight.max(1),
            priority,
            local: false,
            disable_cooling: false,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cooling(&self, now: Instant) -> Option<Instant> {
        self.state().cooling_until.filter(|t| *t > now)
    }

    pub fn view(&self) -> CredentialView {
        let st = self.state();
        let now = Instant::now();
        CredentialView {
            id: self.label.clone(),
            provider: self.kind.as_str(),
            key: self.masked_key.clone(),
            base_url: self.base_url.clone(),
            weight: self.weight,
            priority: self.priority,
            cooling_for_secs: st
                .cooling_until
                .filter(|t| *t > now)
                .map_or(0, |t| (t - now).as_secs()),
            success: st.success,
            failed: st.failed,
        }
    }
}

/// Built-in manifest for a group kind, pointed at the group's `base-url`.
fn manifest_for(g: &Group, builtin: &Registry) -> Result<ProviderManifest, String> {
    let id = format!(
        "proxy-{}",
        g.name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            })
            .take(40)
            .collect::<String>()
    );
    let from = |builtin_id: &str, base: Option<String>| -> Result<ProviderManifest, String> {
        let mut m = builtin
            .get(builtin_id)
            .cloned()
            .ok_or_else(|| format!("built-in provider `{builtin_id}` is missing"))?;
        m.id.clone_from(&id);
        m.name.clone_from(&g.name);
        if let Some(b) = base {
            m.base_url = b;
            m.base_url_env = None;
        }
        Ok(m)
    };
    let base = g
        .base_url
        .as_deref()
        .map(|b| b.trim_end_matches('/').to_owned());
    // CLIProxyAPI base URLs are host roots; Kor's adapters expect the versioned API base.
    let versioned = |b: String, v: &str| if b.ends_with(v) { b } else { format!("{b}{v}") };
    match g.kind {
        Kind::Gemini => from(
            "google",
            base.map(|b| {
                if b.ends_with("/v1beta") || b.ends_with("/v1") {
                    b
                } else {
                    format!("{b}/v1beta")
                }
            }),
        ),
        Kind::Claude => from("anthropic", base.map(|b| versioned(b, "/v1"))),
        Kind::Codex => from("openai", base),
        Kind::Xai => from("xai", base),
        Kind::Meta => {
            let b = base.unwrap_or_else(|| "https://api.meta.ai/v1".to_owned());
            custom_manifest(
                &id,
                &AuthMethod::CustomEndpoint {
                    base_url: b,
                    anthropic: false,
                    key: true,
                },
            )
            .map_err(|e| e.message)
        }
        Kind::OpenAiCompat => {
            let b = base.ok_or_else(|| format!("{}: base-url is required", g.name))?;
            custom_manifest(
                &id,
                &AuthMethod::CustomEndpoint {
                    base_url: b,
                    anthropic: false,
                    key: !g.keys.is_empty(),
                },
            )
            .map_err(|e| e.message)
        }
    }
}

/// Every credential of `settings`; groups that cannot be built are reported in the error list.
pub fn credentials(
    settings: &Settings,
    builtin: &Registry,
    is_local: &dyn Fn(&ProviderManifest) -> bool,
) -> (Vec<Credential>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (gi, g) in settings.groups.iter().enumerate() {
        let m = match manifest_for(g, builtin) {
            Ok(m) => m,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let keys: Vec<Option<(String, u32, Option<i64>)>> = if g.keys.is_empty() {
            vec![None]
        } else {
            g.keys
                .iter()
                .map(|k| Some((k.api_key.clone(), k.weight, k.priority)))
                .collect()
        };
        let n = keys.len();
        for (ki, k) in keys.into_iter().enumerate() {
            let (secret, weight, prio) = match k {
                Some((key, w, p)) => (Some(SecretString::from(key)), w, p),
                None => (None, 1, None),
            };
            let masked = secret.as_ref().map_or_else(String::new, |s| {
                kor_secrets::mask(kor_secrets::ExposeSecret::expose_secret(s))
            });
            match kor_providers::build(&m, secret) {
                Ok(provider) => out.push(Credential {
                    label: if n == 1 {
                        g.name.clone()
                    } else {
                        format!("{}#{}", g.name, ki + 1)
                    },
                    group: gi,
                    kind: g.kind,
                    provider,
                    masked_key: masked,
                    base_url: m.base_url.clone(),
                    weight,
                    priority: prio.unwrap_or(g.priority),
                    local: is_local(&m),
                    disable_cooling: g.disable_cooling.unwrap_or(settings.disable_cooling),
                    state: Mutex::new(State::default()),
                }),
                Err(e) => errors.push(format!("{}: {}", g.name, e.message)),
            }
        }
    }
    (out, errors)
}

/// `*` wildcards only (prefix, suffix, substring, exact), as `excluded-models` allows.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() || !name.ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

/// `gemini-2.5-pro(8192)` → `gemini-2.5-pro`: the thinking suffix is accepted and dropped
/// (Kor's request form has no thinking budget yet).
pub fn strip_suffix(model: &str) -> &str {
    match model
        .strip_suffix(')')
        .and_then(|m| m.rfind('(').map(|i| &model[..i]))
    {
        Some(base) if !base.is_empty() => base,
        _ => model,
    }
}

/// Model families an unlisted group serves (the original consults its model catalog).
fn family(kind: Kind, name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    match kind {
        Kind::Gemini => ["gemini", "gemma", "learnlm"]
            .iter()
            .any(|p| n.starts_with(p)),
        Kind::Claude => n.starts_with("claude"),
        Kind::Codex => ["gpt", "o1", "o3", "o4", "codex", "chatgpt"]
            .iter()
            .any(|p| n.starts_with(p)),
        Kind::Xai => n.starts_with("grok"),
        Kind::Meta => n.starts_with("muse") || n.starts_with("llama"),
        Kind::OpenAiCompat => false,
    }
}

/// Upstream model name when `g` serves the client's `model`.
pub fn serves(g: &Group, model: &str, force_prefix: bool) -> Option<String> {
    let name = match &g.prefix {
        Some(p) => match model
            .strip_prefix(p.as_str())
            .and_then(|r| r.strip_prefix('/'))
        {
            Some(rest) => rest,
            None if !force_prefix => model,
            None => return None,
        },
        None => model,
    };
    let name = strip_suffix(name);
    let upstream = if g.models.is_empty() {
        family(g.kind, name).then(|| name.to_owned())?
    } else {
        g.models
            .iter()
            .find(|m| m.client_name() == name || m.name == name)
            .map(|m| m.name.clone())?
    };
    (!g.excluded.iter().any(|p| wildcard(p, &upstream))).then_some(upstream)
}

/// The credential pool of one config generation.
#[derive(Debug)]
pub struct Pool {
    pub creds: Vec<Credential>,
    pub settings: Settings,
    rr: Mutex<BTreeMap<String, usize>>,
}

/// One attempt: a credential and the upstream model name to ask it for.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub cred: usize,
    pub model: String,
}

impl Pool {
    pub fn new(settings: Settings, creds: Vec<Credential>) -> Self {
        Self {
            creds,
            settings,
            rr: Mutex::new(BTreeMap::new()),
        }
    }

    /// Credentials for `model` in the order to try them. `local_only` is privacy mode.
    pub fn order(&self, model: &str, local_only: bool) -> Vec<Attempt> {
        let s = &self.settings;
        let mut cands: Vec<Attempt> = self
            .creds
            .iter()
            .enumerate()
            .filter(|(_, c)| !local_only || c.local)
            .filter(|(_, c)| s.strategy != Strategy::WeightedRoundRobin || c.weight > 0)
            .filter_map(|(i, c)| {
                serves(&s.groups[c.group], model, s.force_model_prefix)
                    .map(|m| Attempt { cred: i, model: m })
            })
            .collect();
        if cands.is_empty() {
            return cands;
        }
        // Stable: priority tiers, high first; config order inside a tier.
        cands.sort_by_key(|a| std::cmp::Reverse(self.creds[a.cred].priority));
        let mut out = Vec::with_capacity(cands.len());
        let mut start = 0;
        while start < cands.len() {
            let p = self.creds[cands[start].cred].priority;
            let end = cands[start..]
                .iter()
                .position(|a| self.creds[a.cred].priority != p)
                .map_or(cands.len(), |n| start + n);
            let mut tier = cands[start..end].to_vec();
            self.arrange(model, p, &mut tier);
            out.extend(tier);
            start = end;
        }
        // Cooling credentials last, soonest available first.
        let now = Instant::now();
        out.sort_by_key(|a| self.creds[a.cred].cooling(now));
        out
    }

    fn arrange(&self, model: &str, prio: i64, tier: &mut [Attempt]) {
        if tier.len() < 2 {
            return;
        }
        match self.settings.strategy {
            Strategy::FillFirst => {}
            Strategy::RoundRobin => {
                let mut rr = self.rr.lock().unwrap_or_else(PoisonError::into_inner);
                let n = rr.entry(format!("{prio}\u{0}{model}")).or_insert(0);
                tier.rotate_left(*n % tier.len());
                *n = n.wrapping_add(1);
            }
            Strategy::WeightedRoundRobin => {
                // Smooth weighted round-robin (nginx): add weights, pick the largest credit,
                // subtract the total from it. The others follow in config order.
                let total: i64 = tier
                    .iter()
                    .map(|a| i64::from(self.creds[a.cred].weight))
                    .sum();
                let mut best = 0;
                let mut best_credit = i64::MIN;
                for (i, a) in tier.iter().enumerate() {
                    let mut st = self.creds[a.cred].state();
                    st.credit += i64::from(self.creds[a.cred].weight);
                    if st.credit > best_credit {
                        best_credit = st.credit;
                        best = i;
                    }
                }
                self.creds[tier[best].cred].state().credit -= total;
                tier[..=best].rotate_right(1);
            }
        }
    }

    /// Book a failure: cool the credential down when the error says so.
    pub fn failed(&self, cred: usize, e: &ProviderError) {
        let c = &self.creds[cred];
        let mut st = c.state();
        st.failed += 1;
        if c.disable_cooling {
            return;
        }
        let wait = match e.kind {
            ProviderErrorKind::RateLimited => {
                let backoff = QUOTA_BACKOFF_BASE
                    .saturating_mul(1u32.checked_shl(st.quota_level).unwrap_or(u32::MAX))
                    .min(QUOTA_BACKOFF_MAX);
                st.quota_level = (st.quota_level + 1).min(20);
                Some(e.retry_after.unwrap_or(backoff).min(QUOTA_BACKOFF_MAX))
            }
            ProviderErrorKind::Unavailable
            | ProviderErrorKind::Timeout
            | ProviderErrorKind::AuthFailed => self.settings.transient_cooldown,
            _ => None,
        };
        if let Some(w) = wait {
            st.cooling_until = Some(Instant::now() + w);
        }
    }

    pub fn succeeded(&self, cred: usize) {
        let mut st = self.creds[cred].state();
        st.success += 1;
        st.quota_level = 0;
        st.cooling_until = None;
    }

    /// Time until the first of `attempts` stops cooling (zero when one is ready).
    pub fn next_ready(&self, attempts: &[Attempt]) -> Duration {
        let now = Instant::now();
        attempts
            .iter()
            .map(|a| {
                self.creds[a.cred]
                    .cooling(now)
                    .map_or(Duration::ZERO, |t| t - now)
            })
            .min()
            .unwrap_or(Duration::ZERO)
    }

    /// Client model names this pool lists (`/v1/models`): configured aliases and names.
    pub fn listed_models(&self, local_only: bool) -> Vec<(String, &'static str)> {
        let mut out: Vec<(String, &'static str)> = Vec::new();
        for c in self.creds.iter().filter(|c| !local_only || c.local) {
            let g = &self.settings.groups[c.group];
            for m in &g.models {
                let name = match &g.prefix {
                    Some(p) => format!("{p}/{}", m.client_name()),
                    None => m.client_name().to_owned(),
                };
                if !g.excluded.iter().any(|p| wildcard(p, &m.name))
                    && !out.iter().any(|(n, _)| *n == name)
                {
                    out.push((name, g.kind.as_str()));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyEntry, ModelMap, parse};

    fn group(kind: Kind) -> Group {
        Group {
            kind,
            name: "g".into(),
            base_url: None,
            prefix: None,
            priority: 0,
            models: Vec::new(),
            excluded: Vec::new(),
            keys: vec![KeyEntry {
                api_key: "k".into(),
                weight: 1,
                priority: None,
            }],
            disable_cooling: None,
        }
    }

    #[test]
    fn wildcards_and_suffixes() {
        assert!(wildcard("gemini-2.5-*", "gemini-2.5-pro"));
        assert!(wildcard("*-preview", "gemini-3-pro-preview"));
        assert!(wildcard("*flash*", "gemini-2.5-flash-lite"));
        assert!(!wildcard("*flash*", "gemini-pro"));
        assert!(wildcard("a*b*c", "a-x-b-y-c") && !wildcard("a*b*c", "a-c-b"));
        assert_eq!(strip_suffix("gemini-2.5-pro(8192)"), "gemini-2.5-pro");
        assert_eq!(strip_suffix("(x)"), "(x)");
    }

    #[test]
    fn serves_aliases_prefixes_families_and_exclusions() {
        let mut g = group(Kind::Gemini);
        assert_eq!(
            serves(&g, "gemini-2.5-flash", false).as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(serves(&g, "claude-x", false), None);
        g.excluded = vec!["*-flash".into()];
        assert_eq!(serves(&g, "gemini-2.5-flash", false), None);
        g.excluded.clear();
        g.models = vec![ModelMap {
            name: "gemini-2.5-flash".into(),
            alias: "fast".into(),
        }];
        assert_eq!(
            serves(&g, "fast(high)", false).as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(serves(&g, "gemini-2.5-pro", false), None);
        g.prefix = Some("team".into());
        assert_eq!(
            serves(&g, "team/fast", true).as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(serves(&g, "fast", true), None);
        assert_eq!(
            serves(&g, "fast", false).as_deref(),
            Some("gemini-2.5-flash")
        );
    }

    fn pool(yaml: &str) -> Pool {
        let st = parse(yaml).unwrap();
        let (creds, errs) = credentials(&st, &Registry::builtin(), &|_| false);
        assert!(errs.is_empty(), "{errs:?}");
        Pool::new(st, creds)
    }

    #[test]
    fn round_robin_rotates_and_cooling_keys_go_last() {
        let p = pool(
            "api-keys: { claude: [ { name: c, keys: [ {api-key: a}, {api-key: b}, {api-key: c} ] } ] }",
        );
        let first = |p: &Pool| p.order("claude-x", false)[0].cred;
        assert_eq!([first(&p), first(&p), first(&p), first(&p)], [0, 1, 2, 0]);
        let mut e = ProviderError::new("x", ProviderErrorKind::RateLimited, "slow");
        e.retry_after = Some(Duration::from_secs(5));
        p.failed(1, &e);
        let o: Vec<usize> = p.order("claude-x", false).iter().map(|a| a.cred).collect();
        assert_eq!(*o.last().unwrap(), 1);
        assert!(p.next_ready(&p.order("claude-x", false)[2..]) > Duration::from_secs(3));
        p.succeeded(1);
        assert_eq!(p.next_ready(&p.order("claude-x", false)), Duration::ZERO);
        assert_eq!(p.creds[1].view().failed, 1);
        assert!(p.creds[1].view().key.contains('•') || p.creds[1].view().key.len() <= 8);
    }

    #[test]
    fn weighted_round_robin_follows_weights_and_priority_tiers() {
        let p = pool(
            "routing: { strategy: weighted-round-robin }\napi-keys: { claude: [ { name: c, keys: [ \
             {api-key: a, weight: 3}, {api-key: b, weight: 1}, {api-key: z, weight: 0} ] }, \
             { name: low, priority: -1, keys: [ {api-key: l} ] } ] }",
        );
        let mut counts = [0; 4];
        for _ in 0..8 {
            let o = p.order("claude-x", false);
            assert_eq!(o.len(), 3, "weight 0 excluded");
            assert_eq!(o.last().unwrap().cred, 3, "lower priority tier last");
            counts[o[0].cred] += 1;
        }
        assert_eq!(counts[..2], [6, 2]);
    }

    #[test]
    fn quota_backoff_doubles_without_retry_after() {
        let p = pool("api-keys: { claude: [ { name: c, keys: [ {api-key: a} ] } ] }");
        let e = ProviderError::new("x", ProviderErrorKind::RateLimited, "q");
        p.failed(0, &e);
        let a = p.next_ready(&p.order("claude-x", false));
        p.failed(0, &e);
        let b = p.next_ready(&p.order("claude-x", false));
        assert!(a <= Duration::from_secs(1) && b > Duration::from_millis(1500));
        assert!(p.listed_models(false).is_empty());
        let bad = ProviderError::new("x", ProviderErrorKind::InvalidRequest, "no");
        p.succeeded(0);
        p.failed(0, &bad);
        assert_eq!(p.next_ready(&p.order("claude-x", false)), Duration::ZERO);
    }
}
