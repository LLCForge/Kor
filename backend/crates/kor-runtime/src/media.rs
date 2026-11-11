//! Media generation shared by the CLI, the TUI and the desktop (§20, ADR 0060): the configured
//! clients, `[media]` settings, the media tool set, and the capability catalog cached for a day.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kor_agent::media::MediaEnv;
use kor_i18n::Localizer;
use kor_providers::fal::Fal;
use kor_providers::huggingface::Hf;
use kor_providers::media::VeniceMedia;
use kor_providers::media_catalog::{self as catalog, Capability, CatalogModel};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The catalog is fetched again after this long (§20.1: "updated daily").
const MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
const CACHE_FILE: &str = "media-catalog.json";

/// The media providers with a key. In privacy mode none (§20.4: only local generation runs).
#[derive(Debug, Clone, Default)]
pub struct Clients {
    pub fal: Option<Fal>,
    pub venice: Option<VeniceMedia>,
    /// Only with a token: Hub inference needs one.
    pub hf: Option<Hf>,
}

impl Clients {
    pub fn load(l: &Localizer) -> Self {
        if crate::privacy_mode() {
            return Self::default();
        }
        Self {
            fal: crate::fal_client(l).map(|(f, _)| f),
            venice: crate::venice_client(l),
            hf: crate::hf_client(l, None)
                .ok()
                .and_then(|(h, k)| k.map(|_| h)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fal.is_none() && self.venice.is_none() && self.hf.is_none()
    }

    /// `fal`, `huggingface`, `venice`: which providers the catalog covers.
    pub fn providers(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.fal.is_some() {
            v.push("fal".to_owned());
        }
        if self.hf.is_some() {
            v.push("huggingface".to_owned());
        }
        if self.venice.is_some() {
            v.push("venice".to_owned());
        }
        v
    }

    fn sources(&self) -> catalog::Sources<'_> {
        catalog::Sources {
            fal: self.fal.as_ref(),
            venice: self.venice.as_ref(),
            hf: self.hf.as_ref(),
        }
    }

    /// The fal client a job of `provider` resumes with (`huggingface`: fal through the HF router).
    pub fn fal_for(&self, provider: &str) -> Option<Fal> {
        if provider == "huggingface" {
            self.hf.as_ref().and_then(|h| h.fal().ok())
        } else {
            self.fal.clone()
        }
    }
}

/// `[media]` settings and the usage ledger as the media tools take them.
pub fn env(config: Option<&kor_config::MediaConfig>) -> MediaEnv {
    let c = config.cloned().unwrap_or_default();
    MediaEnv {
        ledger: crate::usage::file(),
        defaults: c
            .defaults
            .iter()
            .filter_map(|(cap, t)| {
                let v = serde_json::to_value(t).ok()?;
                Some((cap.clone(), v.as_object()?.clone()))
            })
            .collect(),
        confirm_above: c.confirm_above.max(0.0),
        no_ask: !c.ask,
    }
}

/// The media tools for these clients (Venice, fal / Hugging Face, the gallery).
pub fn tools(c: &Clients, env: &MediaEnv) -> Vec<Arc<dyn kor_tools::Tool>> {
    let mut out = Vec::new();
    if let Some(v) = &c.venice {
        out.extend(kor_agent::media::tools(&kor_agent::media::Media::with_env(
            v.clone(),
            env.clone(),
        )));
    }
    if let Some(s) = kor_agent::media::Studio::with_env(c.fal.clone(), c.hf.clone(), env.clone()) {
        out.extend(kor_agent::media::studio_tools(&s));
    }
    if !out.is_empty() {
        out.push(kor_agent::media::gallery_tool());
    }
    out
}

/// The cached catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Catalog {
    /// Unix milliseconds.
    pub fetched: u64,
    pub providers: Vec<String>,
    pub models: Vec<CatalogModel>,
    /// Sources that could not be read the last time.
    pub errors: Vec<String>,
}

impl Catalog {
    /// Models with `cap` (all without one), priced first and cheapest first.
    pub fn with(&self, cap: Option<Capability>) -> Vec<&CatalogModel> {
        let mut v: Vec<&CatalogModel> = self
            .models
            .iter()
            .filter(|m| cap.is_none_or(|c| m.capabilities.contains(&c)))
            .collect();
        v.sort_by(|a, b| match (&a.price, &b.price) {
            (Some(x), Some(y)) => x.amount.total_cmp(&y.amount).then(a.id.cmp(&b.id)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)),
        });
        v
    }

    /// A model by `provider:id` or a bare id.
    pub fn find(&self, reference: &str) -> Option<&CatalogModel> {
        self.models
            .iter()
            .find(|m| m.reference() == reference)
            .or_else(|| self.models.iter().find(|m| m.id == reference))
    }
}

fn cache_path() -> Option<PathBuf> {
    kor_platform::cache_dir().map(|d| d.join(CACHE_FILE))
}

fn read(path: &Path) -> Option<Catalog> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The catalog of every configured provider: from the cache when it is younger than a day and
/// covers the same providers, else fetched (and cached) again.
pub async fn catalog(c: &Clients, refresh: bool) -> Catalog {
    catalog_at(c, refresh, cache_path().as_deref(), crate::cost::now_ms()).await
}

async fn catalog_at(c: &Clients, refresh: bool, cache: Option<&Path>, now: u64) -> Catalog {
    let providers = c.providers();
    if providers.is_empty() {
        return Catalog::default();
    }
    if !refresh
        && let Some(cached) = cache.and_then(read)
        && cached.providers == providers
        && now.saturating_sub(cached.fetched) < MAX_AGE_MS
    {
        return cached;
    }
    let (models, errors) = catalog::fetch(&c.sources(), &Capability::ALL).await;
    let out = Catalog {
        fetched: now,
        providers,
        models,
        errors,
    };
    // A failed fetch is not cached as the day's catalog.
    if let Some(p) = cache.filter(|_| !out.models.is_empty())
        && let Some(d) = p.parent()
        && std::fs::create_dir_all(d).is_ok()
        && let Ok(s) = serde_json::to_string(&out)
    {
        let _ = std::fs::write(p, s);
    }
    out
}

/// "Done" / "failed" in the notification center (desktop bell, OS toast, `kor notifications`),
/// for generations that ran in the background (§20.4). Muted categories are respected by the
/// readers of the store.
pub fn notify(ok: bool, title: &str, body: &str, cost: Option<&(f64, String, bool)>) {
    use crate::presence::notifications::{Category, Notification, Store};
    if let Some(s) = Store::standard() {
        let usd = cost.filter(|c| c.1 == "USD");
        let _ = s.push(Notification {
            category: if ok { Category::Done } else { Category::Error },
            title: title.to_owned(),
            body: body.to_owned(),
            cost_usd: usd.map(|c| c.0),
            cost_estimated: usd.is_some_and(|c| c.2),
            ..Default::default()
        });
    }
}

/// Saved defaults for `cap` from `[media.defaults.<cap>]`, without the `model` key.
pub fn defaults_for(env: &MediaEnv, cap: Capability) -> Map<String, Value> {
    let mut d = env.defaults_for(Some(cap));
    d.remove("model");
    d
}

/// The model saved as `[media.defaults.<cap>] model`, if any.
pub fn default_model(env: &MediaEnv, cap: Capability) -> Option<String> {
    env.defaults_for(Some(cap))
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn settings_become_the_tools_env() {
        let mut c = kor_config::MediaConfig::default();
        let t: toml::Table = toml::from_str("duration = \"5s\"\nmodel = \"fal:x\"").unwrap();
        c.defaults.insert("video".into(), t);
        c.ask = false;
        c.confirm_above = 0.5;
        let e = env(Some(&c));
        assert!(e.no_ask);
        assert!((e.confirm_above - 0.5).abs() < f64::EPSILON);
        assert_eq!(defaults_for(&e, Capability::Video)["duration"], "5s");
        assert!(!defaults_for(&e, Capability::Video).contains_key("model"));
        assert_eq!(
            default_model(&e, Capability::Video).as_deref(),
            Some("fal:x")
        );
    }

    #[tokio::test]
    async fn catalog_is_cached_for_a_day() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"endpoint_id": "fal-ai/flux/schnell", "metadata": {"display_name": "FLUX", "category": "text-to-image"}}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models/pricing"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"prices": []})))
            .mount(&server)
            .await;
        let manifest = kor_providers::ProviderManifest::parse(
            include_str!("../../../providers/fal.toml"),
            "fal.toml",
        )
        .unwrap();
        let fal = Fal::with_bases(
            &manifest,
            kor_secrets::SecretString::from("k".to_owned()),
            &server.uri(),
            Some(&server.uri()),
            "",
        )
        .unwrap();
        let c = Clients {
            fal: Some(fal),
            ..Clients::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(CACHE_FILE);
        let first = catalog_at(&c, false, Some(&cache), 1_000).await;
        // Every category is asked; only text-to-image answers with a model here.
        assert!(first.models.iter().all(|m| m.provider == "fal"));
        assert!(first.find("fal:fal-ai/flux/schnell").is_some());
        let hits = server.received_requests().await.unwrap().len();
        let again = catalog_at(&c, false, Some(&cache), 2_000).await;
        assert_eq!(again, first, "served from the cache");
        assert_eq!(server.received_requests().await.unwrap().len(), hits);
        let later = catalog_at(&c, false, Some(&cache), 1_000 + MAX_AGE_MS + 1).await;
        assert_eq!(
            later.fetched,
            1_000 + MAX_AGE_MS + 1,
            "a day later it is fetched again"
        );
        assert!(
            catalog_at(&Clients::default(), false, Some(&cache), 0)
                .await
                .models
                .is_empty()
        );
        assert_eq!(
            first.with(Some(Capability::Image)).len(),
            first.models.len()
        );
    }
}
