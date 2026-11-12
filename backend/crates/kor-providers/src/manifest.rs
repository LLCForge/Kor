//! Declarative provider manifests (`providers/<id>.toml`) and the registry that holds them.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use kor_core::PrivacyTier;
use reqwest::Url;
use reqwest::header::HeaderName;
use serde::{Deserialize, Serialize};

const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Standard,
    Uncensored,
    Aggregator,
    Cloud,
    Local,
    Media,
    /// Subscription proxies such as `CLIProxyAPI`: advanced, at the user's own risk, never default.
    Proxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Anthropic,
    Openai,
    OpenaiCompat,
    Gemini,
    Vertex,
    Bedrock,
    Azure,
    Cohere,
    Oci,
    Ollama,
    Fal,
    /// `SageMaker` runtime (`SigV4`); declared, not implemented yet.
    Sagemaker,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::OpenaiCompat => "openai_compat",
            Self::Gemini => "gemini",
            Self::Vertex => "vertex",
            Self::Bedrock => "bedrock",
            Self::Azure => "azure",
            Self::Cohere => "cohere",
            Self::Oci => "oci",
            Self::Ollama => "ollama",
            Self::Fal => "fal",
            Self::Sagemaker => "sagemaker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// `Authorization: Bearer <key>` (or `header` if set).
    Bearer,
    /// Raw key in `header`.
    Header,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub kind: AuthKind,
    pub env: Option<String>,
    pub header: Option<String>,
    /// Scheme word before the key, e.g. `Key` for fal.ai (`Authorization: Key <key>`).
    /// Defaults to `Bearer` for `kind = "bearer"`.
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    pub tier: PrivacyTier,
    pub retention: String,
    pub training_on_data: Option<bool>,
    pub jurisdiction: Option<String>,
    pub policy_url: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Quirks {
    /// Deep-merged into every request body.
    pub extra_body: Option<serde_json::Value>,
    pub extra_headers: BTreeMap<String, String>,
    /// Model id → id sent on the wire (Azure deployment names).
    pub model_map: BTreeMap<String, String>,
    /// llama.cpp server: a structured reply is constrained with a GBNF `grammar` built from the
    /// schema (§15.13), which also works on builds without JSON-schema support.
    pub grammar: bool,
}

fn yes() -> bool {
    true
}

// Capability flags mirror the TOML schema one-to-one.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderManifest {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub adapter: AdapterKind,
    pub base_url: String,
    /// Env var that replaces `base_url` (resource-, project- or region-specific endpoints).
    /// A `base_url` on the reserved `.invalid` domain means the variable is required.
    pub base_url_env: Option<String>,
    pub auth: Auth,
    pub models_endpoint: Option<String>,
    /// Model ids to offer when the provider has no list endpoint (`models_endpoint` unset).
    #[serde(default)]
    pub static_models: Vec<String>,
    /// Free-form labels shown on the provider card: `low-latency`, `fixed-price`, `advanced`...
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "yes")]
    pub streaming: bool,
    #[serde(default)]
    pub tool_calling: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub json_mode: bool,
    #[serde(default)]
    pub thinking: bool,
    pub max_context_default: Option<u64>,
    /// Endpoint and behaviour were checked against the real API.
    #[serde(default)]
    pub verified: bool,
    pub docs: Option<String>,
    pub privacy: Privacy,
    #[serde(default)]
    pub quirks: Quirks,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{origin}: {message}")]
pub struct ManifestError {
    pub origin: String,
    pub message: String,
}

impl ProviderManifest {
    pub fn parse(text: &str, origin: &str) -> Result<Self, ManifestError> {
        let err = |message: String| ManifestError {
            origin: origin.to_owned(),
            message,
        };
        let m: Self = toml::from_str(text).map_err(|e| err(e.to_string()))?;
        m.validate().map_err(err)?;
        Ok(m)
    }

    /// Env var holding the key, from the manifest or the `<ID>_API_KEY` convention.
    pub fn key_env(&self) -> String {
        self.auth
            .env
            .clone()
            .unwrap_or_else(|| kor_secrets::env_var_for(&self.id))
    }

    pub fn needs_key(&self) -> bool {
        self.auth.kind != AuthKind::None
    }

    fn validate(&self) -> Result<(), String> {
        if !kor_core::is_valid_provider_id(&self.id) {
            return Err(format!("invalid id `{}`", self.id));
        }
        let url = Url::parse(&self.base_url).map_err(|e| format!("base_url: {e}"))?;
        let loopback = matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        // Never send a key over plaintext to a remote host.
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            s => {
                return Err(format!(
                    "base_url: scheme `{s}` not allowed for a remote host"
                ));
            }
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("base_url must not embed credentials".into());
        }
        for env in self.auth.env.iter().chain(&self.base_url_env) {
            if !(1..=128).contains(&env.len())
                || !env
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(format!("env var `{env}` must be UPPER_SNAKE_CASE"));
            }
        }
        if url.host_str().is_some_and(|h| h.ends_with(".invalid")) && self.base_url_env.is_none() {
            return Err("a `.invalid` base_url needs base_url_env".into());
        }
        if let Some(p) = &self.auth.prefix
            && (!(1..=16).contains(&p.len()) || !p.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err(format!("auth.prefix `{p}` must be a single word"));
        }
        if self.auth.kind == AuthKind::Header && self.auth.header.is_none() {
            return Err("auth.kind = \"header\" needs auth.header".into());
        }
        let auth_header = self
            .auth
            .header
            .as_deref()
            .unwrap_or("authorization")
            .to_ascii_lowercase();
        for name in self
            .auth
            .header
            .iter()
            .chain(self.quirks.extra_headers.keys())
        {
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| format!("invalid header name `{name}`"))?;
        }
        if self
            .quirks
            .extra_headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case(&auth_header))
        {
            return Err("quirks.extra_headers must not set the auth header".into());
        }
        if let Some(b) = &self.quirks.extra_body
            && !b.is_object()
        {
            return Err("quirks.extra_body must be a table".into());
        }
        Ok(())
    }

    /// The manifest with `base_url_env` applied: the env value replaces `base_url` (validated
    /// like any base URL). Errors when the variable is required (`.invalid` placeholder) but unset.
    pub fn resolved(
        &self,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<std::borrow::Cow<'_, Self>, ManifestError> {
        let err = |message: String| ManifestError {
            origin: self.id.clone(),
            message,
        };
        let Some(var) = &self.base_url_env else {
            return Ok(std::borrow::Cow::Borrowed(self));
        };
        match env(var)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        {
            Some(url) => {
                let mut m = self.clone();
                m.base_url = url;
                m.validate().map_err(err)?;
                Ok(std::borrow::Cow::Owned(m))
            }
            None if self.base_url.contains(".invalid") => {
                Err(err(format!("set {var} to this provider's endpoint URL")))
            }
            None => Ok(std::borrow::Cow::Borrowed(self)),
        }
    }
}

macro_rules! builtin {
    ($($id:literal),* $(,)?) => {
        &[$(($id, include_str!(concat!("../../../providers/", $id, ".toml")))),*]
    };
}

/// Built-in manifests shipped in the binary; `providers.toml` must list the same ids
/// (test `registry_matches_manifests_and_adapters`).
pub(crate) const BUILTIN: &[(&str, &str)] = builtin![
    "abacus",
    "abliteration",
    "anthropic",
    "arliai",
    "azure",
    "bedrock",
    "cerebras",
    "codestral",
    "cohere",
    "dashscope",
    "dashscope-cn",
    "deepseek",
    "fal",
    "fireworks",
    "github-models",
    "google",
    "groq",
    "huggingface",
    "local",
    "local-jan",
    "local-koboldcpp",
    "local-llamacpp",
    "local-lmstudio",
    "local-localai",
    "local-tgwebui",
    "local-vllm",
    "mancer",
    "microsoft-foundry",
    "mistral",
    "moonshot",
    "nvidia",
    "oci",
    "openai",
    "openrouter",
    "perplexity",
    "refuseless",
    "sagemaker",
    "sambanova",
    "together",
    "uncensoredllm",
    "venice",
    "vertex",
    "xai",
    "zai",
    "zai-coding",
    "zhipu",
];

#[derive(Debug, Clone, Default)]
pub struct Registry {
    manifests: BTreeMap<String, ProviderManifest>,
}

impl Registry {
    /// # Panics
    /// If a built-in manifest is invalid; the `builtins_are_valid` test guards this.
    pub fn builtin() -> Self {
        let mut r = Self::default();
        for (name, text) in BUILTIN {
            let m = ProviderManifest::parse(text, &format!("builtin:{name}"))
                .unwrap_or_else(|e| panic!("{e}"));
            r.manifests.insert(m.id.clone(), m);
        }
        r
    }

    /// Add or override manifests from `dir/*.toml`. A missing directory is not an error.
    pub fn load_dir(&mut self, dir: &Path) -> Result<(), ManifestError> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Ok(());
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            let origin = path.display().to_string();
            let err = |message: String| ManifestError {
                origin: origin.clone(),
                message,
            };
            let mut text = String::new();
            fs::File::open(&path)
                .and_then(|f| f.take(MAX_MANIFEST_BYTES + 1).read_to_string(&mut text))
                .map_err(|e| err(e.to_string()))?;
            if text.len() as u64 > MAX_MANIFEST_BYTES {
                return Err(err("file too large".into()));
            }
            let m = ProviderManifest::parse(&text, &origin)?;
            self.manifests.insert(m.id.clone(), m);
        }
        Ok(())
    }

    /// Add a manifest unless one with the same id exists (built-ins and user files win).
    pub fn add_if_absent(&mut self, m: ProviderManifest) -> bool {
        if self.manifests.contains_key(&m.id) {
            return false;
        }
        self.manifests.insert(m.id.clone(), m);
        true
    }

    pub fn get(&self, id: &str) -> Option<&ProviderManifest> {
        self.manifests.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProviderManifest> {
        self.manifests.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> String {
        r#"
id = "demo"
name = "Demo"
category = "standard"
adapter = "openai_compat"
base_url = "https://example.com/v1"
auth = { kind = "bearer", env = "DEMO_KEY" }
[privacy]
tier = "standard"
retention = "unknown"
"#
        .to_owned()
    }

    #[test]
    fn builtins_are_valid() {
        let r = Registry::builtin();
        assert_eq!(r.iter().count(), BUILTIN.len());
        for m in r.iter() {
            // Every remote built-in sends keys over TLS only.
            if m.category != Category::Local {
                assert!(m.base_url.starts_with("https://"), "{}", m.id);
            }
        }
        assert_eq!(r.get("local").unwrap().privacy.tier, PrivacyTier::Local);
        assert!(!r.get("local").unwrap().needs_key());
        assert_eq!(r.get("anthropic").unwrap().key_env(), "ANTHROPIC_API_KEY");
    }

    #[test]
    fn rejects_unsafe_manifests() {
        let cases = [
            ("https://example.com/v1", "http://example.com/v1"),
            ("https://example.com/v1", "https://user:pw@example.com/v1"),
            ("DEMO_KEY", "demo-key"),
            ("id = \"demo\"", "id = \"De mo\""),
            (
                "[privacy]",
                "[quirks]\nextra_headers = { Authorization = \"x\" }\n[privacy]",
            ),
            ("[privacy]", "[quirks]\nextra_body = 3\n[privacy]"),
            ("[privacy]", "surprise = true\n[privacy]"),
        ];
        assert!(ProviderManifest::parse(&base(), "t").is_ok());
        for (from, to) in cases {
            let text = base().replacen(from, to, 1);
            assert!(ProviderManifest::parse(&text, "t").is_err(), "{to}");
        }
        let local = base().replace("https://example.com/v1", "http://127.0.0.1:8080/v1");
        assert!(ProviderManifest::parse(&local, "t").is_ok());
    }

    #[test]
    fn user_dir_overrides_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let text = base().replace("\"demo\"", "\"venice\"");
        fs::write(dir.path().join("venice.toml"), text).unwrap();
        fs::write(dir.path().join("ignored.txt"), "x").unwrap();
        let mut r = Registry::builtin();
        r.load_dir(dir.path()).unwrap();
        assert_eq!(r.get("venice").unwrap().name, "Demo");
        r.load_dir(&dir.path().join("missing")).unwrap();
    }

    /// `providers.toml` ↔ `providers/*.toml` ↔ `BUILTIN` ↔ `src/adapters/*.rs`, one to one.
    #[test]
    fn registry_matches_manifests_and_adapters() {
        #[derive(Deserialize)]
        struct Entry {
            id: String,
            adapter: AdapterKind,
            status: Option<String>,
        }
        #[derive(Deserialize)]
        struct File {
            provider: Vec<Entry>,
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let reg: File =
            toml::from_str(&fs::read_to_string(root.join("providers.toml")).unwrap()).unwrap();
        let mut listed: Vec<&str> = reg.provider.iter().map(|e| e.id.as_str()).collect();
        listed.sort_unstable();
        let mut files: Vec<String> = fs::read_dir(root.join("../../providers"))
            .unwrap()
            .filter_map(|e| {
                let p = e.ok()?.path();
                (p.extension()? == "toml").then(|| p.file_stem()?.to_str().map(str::to_owned))?
            })
            .collect();
        files.sort();
        let mut builtin: Vec<&str> = BUILTIN.iter().map(|(id, _)| *id).collect();
        builtin.sort_unstable();
        files.dedup();
        assert_eq!(listed, files, "providers.toml vs providers/*.toml");
        assert_eq!(listed, builtin, "providers.toml vs BUILTIN");
        let builtins = Registry::builtin();
        for e in &reg.provider {
            let m = builtins.get(&e.id).unwrap();
            assert_eq!(m.adapter, e.adapter, "{}", e.id);
            let src = root.join(format!("src/adapters/{}.rs", e.adapter.as_str()));
            match e.status.as_deref() {
                None => assert!(src.is_file(), "{}: no adapter {}", e.id, src.display()),
                Some("not_yet" | "retired") => {
                    assert!(
                        crate::build(m, None).is_err(),
                        "{} must refuse to build",
                        e.id
                    );
                }
                Some(s) => panic!("{}: unknown status {s}", e.id),
            }
        }
    }

    #[test]
    fn base_url_env_overrides_and_placeholder_requires_it() {
        let text = base()
            .replace("https://example.com/v1", "https://your-resource.invalid/v1")
            .replace("auth =", "base_url_env = \"DEMO_URL\"\nauth =");
        let m = ProviderManifest::parse(&text, "t").unwrap();
        assert!(m.resolved(|_| None).is_err());
        let r = m
            .resolved(|_| Some("https://real.example.org/openai/v1".into()))
            .unwrap();
        assert_eq!(r.base_url, "https://real.example.org/openai/v1");
        assert!(
            m.resolved(|_| Some("http://evil.example.org".into()))
                .is_err()
        );
        // `.invalid` without base_url_env is rejected at parse time.
        let bad = base().replace("https://example.com/v1", "https://x.invalid/v1");
        assert!(ProviderManifest::parse(&bad, "t").is_err());
    }
}
