//! CLIProxyAPI `config.yaml`, the subset Kor serves (port of `internal/config`).
//!
//! Both layouts load: the v8 one (`server.port`, `access.api-keys`, `api-keys.<provider>` groups
//! with `keys`) and the legacy one (`port`, top-level `api-keys` list, `gemini-api-key`,
//! `claude-api-key`, `codex-api-key`, `xai-api-key`, `openai-compatibility` with
//! `api-key-entries`). When both spellings are present the v8 value wins, as upstream.
//! Keys Kor does not implement (OAuth, plugins, TLS, Redis, ...) are read and ignored;
//! [`Settings::ignored`] lists the ones that were set so `kor proxy serve` can say so.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_yaml::Value;

/// Upstream port and body limit when the file does not set them.
pub const DEFAULT_PORT: u16 = 8317;
/// Kor's default request body limit (CLIProxyAPI's default is "no limit").
pub const DEFAULT_MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Upstream API family of a credential group (`api-keys.<kind>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Gemini,
    Claude,
    Codex,
    Xai,
    Meta,
    OpenAiCompat,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Xai => "xai",
            Self::Meta => "meta",
            Self::OpenAiCompat => "openai-compatibility",
        }
    }
}

/// `routing.strategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strategy {
    #[default]
    RoundRobin,
    WeightedRoundRobin,
    FillFirst,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMap {
    /// Upstream model name.
    pub name: String,
    /// Client-visible name; empty means `name`.
    pub alias: String,
}

impl ModelMap {
    pub fn client_name(&self) -> &str {
        if self.alias.is_empty() {
            &self.name
        } else {
            &self.alias
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEntry {
    pub api_key: String,
    /// `weight` (default 1, `<= 0` excludes the key from weighted routing, max 1 000 000).
    pub weight: u32,
    pub priority: Option<i64>,
}

/// One `api-keys.<kind>[]` group (or a legacy entry): one endpoint, several keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub kind: Kind,
    pub name: String,
    pub base_url: Option<String>,
    pub prefix: Option<String>,
    pub priority: i64,
    pub models: Vec<ModelMap>,
    pub excluded: Vec<String>,
    pub keys: Vec<KeyEntry>,
    pub disable_cooling: Option<bool>,
}

/// Normalized settings the gateway runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // each bool mirrors one independent `config.yaml` switch
pub struct Settings {
    pub host: Option<String>,
    pub port: u16,
    pub max_body: usize,
    /// `access.api-keys`: client keys accepted besides Kor's keychain key.
    pub client_keys: Vec<String>,
    /// `management.secret-key` when given in plain text (hashed values are not accepted).
    pub management_key: Option<String>,
    pub allow_remote_management: bool,
    pub strategy: Strategy,
    pub force_model_prefix: bool,
    /// Extra credential rounds after the first (`routing.retry.request-retry`).
    pub request_retry: u32,
    /// Longest wait for a cooling credential between rounds (`max-retry-interval`, seconds).
    pub max_retry_interval: Duration,
    pub disable_cooling: bool,
    /// Cooldown after 408/5xx (`transient-error-cooldown-seconds`; 0 = 60 s, -1 = off).
    pub transient_cooldown: Option<Duration>,
    pub usage_statistics: bool,
    pub groups: Vec<Group>,
    /// Kor extension (`kor.aliases`): client model name → `provider/model` of Kor's registry.
    pub aliases: BTreeMap<String, String>,
    /// `oauth.auth-dir`. Empty means the default directory under Kor's config.
    pub auth_dir: Option<std::path::PathBuf>,
    /// Set keys Kor reads but does not implement (shown once at start).
    pub ignored: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: None,
            port: DEFAULT_PORT,
            max_body: DEFAULT_MAX_BODY,
            client_keys: Vec::new(),
            management_key: None,
            allow_remote_management: false,
            strategy: Strategy::RoundRobin,
            force_model_prefix: false,
            request_retry: 3,
            max_retry_interval: Duration::from_secs(30),
            disable_cooling: false,
            transient_cooldown: Some(Duration::from_secs(60)),
            usage_statistics: true,
            groups: Vec::new(),
            aliases: BTreeMap::new(),
            auth_dir: None,
            ignored: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Raw YAML shapes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct RawModel {
    name: String,
    alias: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct RawKey {
    api_key: String,
    weight: Option<i64>,
    priority: Option<i64>,
}

/// A v8 group, a legacy `*-api-key` entry or a legacy `openai-compatibility` entry.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct RawGroup {
    name: String,
    disabled: bool,
    priority: i64,
    prefix: String,
    base_url: String,
    models: Option<Vec<RawModel>>,
    excluded_models: Vec<String>,
    disable_cooling: Option<bool>,
    keys: Vec<RawKey>,
    /// Legacy `openai-compatibility[].api-key-entries`.
    api_key_entries: Vec<RawKey>,
    /// Legacy `*-api-key[]` entries carry their key inline.
    api_key: String,
    weight: Option<i64>,
}

fn s(v: &Value, path: &[&str]) -> Option<Value> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    (!cur.is_null()).then(|| cur.clone())
}

fn as_str(v: Option<Value>) -> Option<String> {
    v.and_then(|v| match v {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn as_i64(v: Option<Value>) -> Option<i64> {
    v.and_then(|v| v.as_i64())
}

fn as_bool(v: Option<Value>) -> Option<bool> {
    v.and_then(|v| v.as_bool())
}

fn strings(v: Option<Value>) -> Vec<String> {
    v.and_then(|v| v.as_sequence().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|x| x.as_str().map(|s| s.trim().to_owned()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// The v8 value when present, else the legacy one.
fn first(v: &Value, paths: &[&[&str]]) -> Option<Value> {
    paths.iter().find_map(|p| s(v, p))
}

fn weight(w: Option<i64>) -> u32 {
    match w {
        None => 1,
        Some(w) if w <= 0 => 0,
        Some(w) => u32::try_from(w.min(1_000_000)).unwrap_or(1_000_000),
    }
}

fn group(kind: Kind, raw: RawGroup, index: usize) -> Option<Group> {
    if raw.disabled {
        return None;
    }
    let mut keys: Vec<KeyEntry> = raw
        .keys
        .into_iter()
        .chain(raw.api_key_entries)
        .map(|k| KeyEntry {
            api_key: k.api_key.trim().to_owned(),
            weight: weight(k.weight),
            priority: k.priority,
        })
        .collect();
    if !raw.api_key.trim().is_empty() {
        keys.push(KeyEntry {
            api_key: raw.api_key.trim().to_owned(),
            weight: weight(raw.weight),
            priority: None,
        });
    }
    // An openai-compatibility entry without keys is a keyless endpoint (local servers).
    keys.retain(|k| !k.api_key.is_empty() && k.api_key.len() <= kor_secrets::MAX_SECRET_LEN);
    if keys.is_empty() && kind != Kind::OpenAiCompat {
        return None;
    }
    let trim = |s: String| {
        let s = s.trim().to_owned();
        (!s.is_empty()).then_some(s)
    };
    Some(Group {
        kind,
        name: trim(raw.name).unwrap_or_else(|| format!("{}-{}", kind.as_str(), index + 1)),
        base_url: trim(raw.base_url),
        prefix: trim(raw.prefix).map(|p| p.trim_matches('/').to_owned()),
        priority: raw.priority,
        models: raw
            .models
            .unwrap_or_default()
            .into_iter()
            .filter(|m| !m.name.trim().is_empty())
            .map(|m| ModelMap {
                name: m.name.trim().to_owned(),
                alias: m.alias.trim().to_owned(),
            })
            .collect(),
        excluded: raw.excluded_models,
        keys,
        disable_cooling: raw.disable_cooling,
    })
}

fn groups_of(kind: Kind, v: Option<Value>, out: &mut Vec<Group>) -> Result<(), String> {
    let Some(v) = v else {
        return Ok(());
    };
    let raw: Vec<RawGroup> =
        serde_yaml::from_value(v).map_err(|e| format!("{}: {e}", kind.as_str()))?;
    out.extend(
        raw.into_iter()
            .enumerate()
            .filter_map(|(i, g)| group(kind, g, i)),
    );
    Ok(())
}

fn expand_home(path: &str) -> std::path::PathBuf {
    let trimmed = path.trim();
    let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) else {
        return std::path::PathBuf::from(trimmed);
    };
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    home.map_or_else(|| std::path::PathBuf::from(trimmed), |home| std::path::PathBuf::from(home).join(rest))
}

/// Parse a CLIProxyAPI `config.yaml` (v8 or legacy layout).
#[allow(clippy::too_many_lines)] // one linear pass over the config keys, read top to bottom
pub fn parse(text: &str) -> Result<Settings, String> {
    let root: Value = if text.trim().is_empty() {
        Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(text).map_err(|e| e.to_string())?
    };
    if !root.is_mapping() {
        return Err("the top level must be a mapping".into());
    }
    let mut st = Settings {
        host: as_str(first(&root, &[&["server", "host"], &["host"]]))
            .filter(|h| !h.trim().is_empty()),
        ..Settings::default()
    };
    if let Some(p) = as_i64(first(&root, &[&["server", "port"], &["port"]])) {
        st.port = u16::try_from(p).map_err(|_| format!("port {p} is out of range"))?;
    }
    if let Some(b) = as_i64(first(
        &root,
        &[
            &["server", "max-request-body-bytes"],
            &["max-request-body-bytes"],
        ],
    )) && b > 0
    {
        st.max_body = usize::try_from(b)
            .unwrap_or(usize::MAX)
            .min(DEFAULT_MAX_BODY * 8);
    }
    // Client keys: v8 `access.api-keys`, legacy top-level `api-keys` as a list.
    st.client_keys = match s(&root, &["access", "api-keys"]) {
        Some(v) => strings(Some(v)),
        None => strings(s(&root, &["api-keys"]).filter(Value::is_sequence)),
    };
    st.management_key = as_str(first(
        &root,
        &[
            &["management", "secret-key"],
            &["remote-management", "secret-key"],
        ],
    ))
    .filter(|k| !k.is_empty() && !k.starts_with("$2"));
    st.allow_remote_management = as_bool(first(
        &root,
        &[
            &["management", "allow-remote"],
            &["remote-management", "allow-remote"],
        ],
    ))
    .unwrap_or(false);
    st.strategy = match as_str(s(&root, &["routing", "strategy"])).as_deref() {
        None | Some("" | "round-robin" | "roundrobin" | "rr") => Strategy::RoundRobin,
        Some("weighted-round-robin" | "weighted") => Strategy::WeightedRoundRobin,
        Some("fill-first" | "fillfirst" | "ff") => Strategy::FillFirst,
        Some(other) => return Err(format!("routing.strategy `{other}` is not supported")),
    };
    st.force_model_prefix = as_bool(first(
        &root,
        &[&["routing", "force-model-prefix"], &["force-model-prefix"]],
    ))
    .unwrap_or(false);
    if let Some(r) = as_i64(first(
        &root,
        &[&["routing", "retry", "request-retry"], &["request-retry"]],
    )) {
        st.request_retry = u32::try_from(r.clamp(0, 10)).unwrap_or(0);
    }
    if let Some(r) = as_i64(first(
        &root,
        &[
            &["routing", "retry", "max-retry-interval"],
            &["max-retry-interval"],
        ],
    )) {
        st.max_retry_interval = Duration::from_secs(u64::try_from(r.clamp(0, 600)).unwrap_or(0));
    }
    st.disable_cooling = as_bool(first(
        &root,
        &[
            &["routing", "cooldown", "disable-cooling"],
            &["disable-cooling"],
        ],
    ))
    .unwrap_or(false);
    st.transient_cooldown = match as_i64(first(
        &root,
        &[
            &["routing", "cooldown", "transient-error-cooldown-seconds"],
            &["transient-error-cooldown-seconds"],
        ],
    )) {
        Some(n) if n < 0 => None,
        Some(n) if n > 0 => Some(Duration::from_secs(u64::try_from(n).unwrap_or(60))),
        _ => Some(Duration::from_secs(60)),
    };
    if let Some(u) = as_bool(first(
        &root,
        &[
            &["observability", "usage", "usage-statistics-enabled"],
            &["usage-statistics-enabled"],
        ],
    )) {
        st.usage_statistics = u;
    }

    let groups_root = s(&root, &["api-keys"]).filter(Value::is_mapping);
    let pick = |v8: &str, legacy: &str| {
        groups_root
            .as_ref()
            .and_then(|g| g.get(v8).filter(|v| !v.is_null()).cloned())
            .or_else(|| s(&root, &[legacy]))
    };
    for (kind, v8, legacy) in [
        (Kind::Gemini, "gemini", "gemini-api-key"),
        (Kind::Claude, "claude", "claude-api-key"),
        (Kind::Codex, "codex", "codex-api-key"),
        (Kind::Xai, "xai", "xai-api-key"),
        (Kind::Meta, "meta", "meta-api-key"),
        (
            Kind::OpenAiCompat,
            "openai-compatibility",
            "openai-compatibility",
        ),
    ] {
        groups_of(kind, pick(v8, legacy), &mut st.groups)?;
    }

    if let Some(Value::Mapping(m)) = s(&root, &["kor", "aliases"]) {
        for (k, v) in m {
            if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                st.aliases.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
    }

    if let Some(dir) = as_str(s(&root, &["oauth", "auth-dir"])) {
        st.auth_dir = Some(expand_home(&dir));
    }

    for (label, path) in [
        ("auth-dir", &["auth-dir"][..]),
        ("api-keys.vertex", &["api-keys", "vertex"]),
        ("api-keys.interactions", &["api-keys", "interactions"]),
        ("vertex-api-key", &["vertex-api-key"]),
        ("server.tls", &["server", "tls", "enable"]),
        ("requests.proxy-url", &["requests", "proxy-url"]),
        ("proxy-url", &["proxy-url"]),
        ("requests.payload", &["requests", "payload"]),
        ("plugins", &["plugins", "enabled"]),
        ("routing.session-affinity", &["routing", "session-affinity"]),
        ("quota-exceeded", &["quota-exceeded"]),
    ] {
        let set = s(&root, path).is_some_and(|v| match &v {
            Value::Bool(b) => *b,
            Value::String(s) => !s.is_empty(),
            Value::Sequence(q) => !q.is_empty(),
            Value::Mapping(m) => !m.is_empty(),
            _ => true,
        });
        if set {
            st.ignored.push(label.to_owned());
        }
    }
    Ok(st)
}

/// Read and parse a config file; a missing file gives the defaults.
pub fn load(path: &Path) -> Result<Settings, String> {
    use std::io::Read;
    let mut text = String::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            f.take(MAX_FILE_BYTES)
                .read_to_string(&mut text)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Validate then write `text` atomically (temp file + rename), for `PUT /config.yaml`.
pub fn save(path: &Path, text: &str) -> Result<Settings, String> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err("config too large".into());
    }
    let st = parse(text)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v8_layout_with_groups_weights_and_aliases() {
        let st = parse(
            r#"
server: { host: "127.0.0.1", port: 9000, max-request-body-bytes: 1000 }
access: { api-keys: ["client-1"] }
routing:
  strategy: weighted-round-robin
  retry: { request-retry: 1, max-retry-interval: 5 }
api-keys:
  gemini:
    - name: g
      prefix: team
      models: [{ name: gemini-2.5-flash, alias: flash }]
      keys: [{ api-key: k1, weight: 5 }, { api-key: k2, weight: 0 }]
  openai-compatibility:
    - name: openrouter
      base-url: https://openrouter.ai/api/v1
      keys: [{ api-key: or-1 }]
      models: [{ name: "moonshotai/kimi-k2:free", alias: kimi-k2 }]
kor:
  aliases: { fast: "local/llama3" }
oauth: { auth-dir: "~/.cli-proxy-api" }
"#,
        )
        .unwrap();
        assert_eq!((st.port, st.max_body), (9000, 1000));
        assert_eq!(st.client_keys, ["client-1"]);
        assert_eq!(st.strategy, Strategy::WeightedRoundRobin);
        assert_eq!(st.request_retry, 1);
        assert_eq!(st.groups.len(), 2);
        let g = &st.groups[0];
        assert_eq!((g.kind, g.prefix.as_deref()), (Kind::Gemini, Some("team")));
        assert_eq!(g.keys.iter().map(|k| k.weight).collect::<Vec<_>>(), [5, 0]);
        assert_eq!(g.models[0].client_name(), "flash");
        assert_eq!(st.groups[1].kind, Kind::OpenAiCompat);
        assert_eq!(st.aliases["fast"], "local/llama3");
        assert!(st.auth_dir.as_deref().is_some_and(|p| p.ends_with(".cli-proxy-api")));
        assert!(st.ignored.is_empty());
    }

    #[test]
    fn legacy_layout_still_loads() {
        let st = parse(
            r#"
port: 8318
api-keys: ["a", "b"]
request-retry: 2
remote-management: { secret-key: "plain", allow-remote: false }
gemini-api-key:
  - api-key: AIza1
  - api-key: AIza2
    base-url: https://example.com
claude-api-key:
  - api-key: sk-ant-1
openai-compatibility:
  - name: local
    base-url: http://127.0.0.1:11434/v1
    models: [{ name: llama3, alias: llama }]
"#,
        )
        .unwrap();
        assert_eq!(st.port, 8318);
        assert_eq!(st.client_keys, ["a", "b"]);
        assert_eq!(st.request_retry, 2);
        assert_eq!(st.management_key.as_deref(), Some("plain"));
        let kinds: Vec<Kind> = st.groups.iter().map(|g| g.kind).collect();
        assert_eq!(
            kinds,
            [Kind::Gemini, Kind::Gemini, Kind::Claude, Kind::OpenAiCompat]
        );
        assert!(st.groups[3].keys.is_empty(), "keyless local endpoint");
    }

    #[test]
    fn bad_input_is_an_error_and_hashed_management_keys_are_ignored() {
        assert!(parse("port: 99999").is_err());
        assert!(parse("- a").is_err());
        assert!(parse("routing: { strategy: random }").is_err());
        let st = parse("management: { secret-key: \"$2a$10$abc\" }").unwrap();
        assert!(st.management_key.is_none());
        assert_eq!(parse("").unwrap(), Settings::default());
    }
}
