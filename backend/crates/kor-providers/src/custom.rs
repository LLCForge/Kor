//! How Kor authenticates to a provider (Master Build Prompt §4.1) and user-added providers
//! (`kor provider add`, desktop `provider.add`), stored as `<config>/providers/<id>.toml` and
//! picked up by [`crate::Registry::load_dir`]. Keys never go into these files; they stay in the
//! env var or the OS keychain.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{Category, ManifestError, ProviderManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// BYOK, the default: a built-in provider with its key in env or keychain.
    ApiKey { provider: String },
    /// Any `OpenAI`- or Anthropic-compatible endpoint; the key is optional.
    CustomEndpoint {
        base_url: String,
        anthropic: bool,
        key: bool,
    },
    /// A server on this machine (Ollama, vLLM, llama.cpp, LM Studio...).
    Local { base_url: String },
    /// External subscription proxy such as `CLIProxyAPI`. Advanced, at the user's own risk.
    SubscriptionProxy { endpoint: String },
}

impl AuthMethod {
    pub fn of(m: &ProviderManifest) -> Self {
        match m.category {
            Category::Local => Self::Local {
                base_url: m.base_url.clone(),
            },
            Category::Proxy => Self::SubscriptionProxy {
                endpoint: m.base_url.trim_end_matches("/v1").to_owned(),
            },
            _ if m.tags.iter().any(|t| t == "custom") => Self::CustomEndpoint {
                base_url: m.base_url.clone(),
                anthropic: m.adapter == crate::AdapterKind::Anthropic,
                key: m.needs_key(),
            },
            _ => Self::ApiKey {
                provider: m.id.clone(),
            },
        }
    }
}

/// `CLIProxyAPI` web management panel (<https://help.router-for.me/management/webui.html>).
pub fn proxy_panel(endpoint: &str) -> String {
    format!("{}/management.html", endpoint.trim_end_matches('/'))
}

fn quoted(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// Manifest for a user-added provider. `base_url` is validated like any manifest (https, or
/// http only on loopback; no embedded credentials).
pub fn custom_manifest(id: &str, method: &AuthMethod) -> Result<ProviderManifest, ManifestError> {
    let env = kor_secrets::env_var_for(id);
    let (category, adapter, base, auth, tags, tier, docs) = match method {
        AuthMethod::ApiKey { .. } => {
            return Err(ManifestError {
                origin: id.to_owned(),
                message: "built-in providers need no manifest; add a key with `kor auth login`"
                    .into(),
            });
        }
        AuthMethod::CustomEndpoint {
            base_url,
            anthropic,
            key,
        } => {
            let auth = match (key, anthropic) {
                (false, _) => "{ kind = \"none\" }".to_owned(),
                (true, true) => format!(
                    "{{ kind = \"header\", env = {}, header = \"x-api-key\" }}",
                    quoted(&env)
                ),
                (true, false) => format!("{{ kind = \"bearer\", env = {} }}", quoted(&env)),
            };
            let adapter = if *anthropic {
                "anthropic"
            } else {
                "openai_compat"
            };
            (
                "standard",
                adapter,
                base_url.clone(),
                auth,
                "[\"custom\"]",
                "unknown",
                None,
            )
        }
        AuthMethod::Local { base_url } => (
            "local",
            "openai_compat",
            base_url.clone(),
            "{ kind = \"none\" }".to_owned(),
            "[\"custom\"]",
            "local",
            None,
        ),
        AuthMethod::SubscriptionProxy { endpoint } => {
            let endpoint = endpoint.trim_end_matches('/');
            (
                "proxy",
                "openai_compat",
                format!("{endpoint}/v1"),
                format!("{{ kind = \"bearer\", env = {} }}", quoted(&env)),
                "[\"advanced\"]",
                "unknown",
                Some(proxy_panel(endpoint)),
            )
        }
    };
    let docs = docs.map_or(String::new(), |d| format!("docs = {}\n", quoted(&d)));
    let text = format!(
        "id = {}\nname = {}\ncategory = \"{category}\"\nadapter = \"{adapter}\"\nbase_url = {}\n\
         auth = {auth}\nmodels_endpoint = \"/models\"\ntool_calling = true\ntags = {tags}\n{docs}\
         [privacy]\ntier = \"{tier}\"\nretention = \"User-added endpoint; see its operator's policy.\"\n",
        quoted(id),
        quoted(id),
        quoted(&base),
    );
    ProviderManifest::parse(&text, id)
}

fn path_for(dir: &Path, id: &str) -> Result<PathBuf, ManifestError> {
    if kor_core::is_valid_provider_id(id) {
        Ok(dir.join(format!("{id}.toml")))
    } else {
        Err(ManifestError {
            origin: id.to_owned(),
            message: format!("invalid id `{id}`"),
        })
    }
}

/// Write `<dir>/<id>.toml` atomically.
pub fn save_user_manifest(dir: &Path, m: &ProviderManifest) -> Result<PathBuf, ManifestError> {
    let path = path_for(dir, &m.id)?;
    let err = |e: &dyn std::fmt::Display| ManifestError {
        origin: path.display().to_string(),
        message: e.to_string(),
    };
    let text = toml::to_string(m).map_err(|e| err(&e))?;
    fs::create_dir_all(dir).map_err(|e| err(&e))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(|e| err(&e))?;
    fs::rename(&tmp, &path).map_err(|e| err(&e))?;
    Ok(path)
}

/// Remove a user-added provider; `false` when there was none.
pub fn remove_user_manifest(dir: &Path, id: &str) -> Result<bool, ManifestError> {
    let path = path_for(dir, id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ManifestError {
            origin: path.display().to_string(),
            message: e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdapterKind, Registry};

    #[test]
    fn custom_endpoints_round_trip_through_the_user_dir() {
        let dir = tempfile::tempdir().unwrap();
        let m = custom_manifest(
            "my-gw",
            &AuthMethod::CustomEndpoint {
                base_url: "https://gw.example.com/v1".into(),
                anthropic: true,
                key: true,
            },
        )
        .unwrap();
        assert_eq!(m.adapter, AdapterKind::Anthropic);
        assert_eq!(m.auth.header.as_deref(), Some("x-api-key"));
        assert_eq!(m.key_env(), "MY_GW_API_KEY");
        assert!(matches!(
            AuthMethod::of(&m),
            AuthMethod::CustomEndpoint {
                anthropic: true,
                ..
            }
        ));
        save_user_manifest(dir.path(), &m).unwrap();
        let mut r = Registry::builtin();
        r.load_dir(dir.path()).unwrap();
        assert_eq!(r.get("my-gw").unwrap(), &m);
        assert!(remove_user_manifest(dir.path(), "my-gw").unwrap());
        assert!(!remove_user_manifest(dir.path(), "my-gw").unwrap());
        assert!(remove_user_manifest(dir.path(), "../x").is_err());
    }

    #[test]
    fn proxy_is_advanced_and_unsafe_urls_are_rejected() {
        let m = custom_manifest(
            "cliproxy",
            &AuthMethod::SubscriptionProxy {
                endpoint: "http://localhost:8317/".into(),
            },
        )
        .unwrap();
        assert_eq!(m.base_url, "http://localhost:8317/v1");
        assert_eq!(m.category, Category::Proxy);
        assert!(m.tags.contains(&"advanced".to_owned()));
        assert_eq!(
            m.docs.as_deref(),
            Some("http://localhost:8317/management.html")
        );
        assert!(matches!(
            AuthMethod::of(&m),
            AuthMethod::SubscriptionProxy { .. }
        ));
        for bad in [
            "http://remote.example.com/v1",
            "https://u:p@x.example.com",
            "ftp://x",
        ] {
            let r = custom_manifest(
                "x",
                &AuthMethod::CustomEndpoint {
                    base_url: bad.into(),
                    anthropic: false,
                    key: true,
                },
            );
            assert!(r.is_err(), "{bad}");
        }
        // A quote in the URL cannot break out of the TOML string.
        let m = custom_manifest(
            "x",
            &AuthMethod::Local {
                base_url: "http://localhost:1/\"\nid = \"evil".into(),
            },
        );
        assert!(m.is_err() || m.unwrap().id == "x");
    }
}
