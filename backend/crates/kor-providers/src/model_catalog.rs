//! Model intelligence data (Master Build Prompt §4.4, ADR-0003): intelligence and coding index,
//! speed, latency, price, context, tool calling, vision and licence per model.
//!
//! Order: the published `catalog.json` (built daily in CI by `catalog/build_catalog.py`, the only
//! place an Artificial Analysis key is used), then `OpenRouter`'s public `/api/v1/models` when that
//! file is unreachable. Kor never calls Artificial Analysis itself and needs no user AA key (ADR-0003).
//! The downloaded catalog is stored at `<cache>/kor/model-catalog.json` and refreshed after 24
//! hours; offline (and privacy mode) reads only that stored copy. Every value carries its source in
//! [`ModelData::source`]; a value no source gives stays `None`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{TTL_SECS, now};

pub const CATALOG_URL: &str = "https://raw.githubusercontent.com/LLCForge/Kor/catalog/catalog.json";
pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/models";
pub const SRC_OPENROUTER: &str = "openrouter";
/// Source tag of values the CI builder took from Artificial Analysis.
pub const SRC_AA: &str = "artificial-analysis";

const MAX_BYTES: usize = 32 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelData {
    pub model_id: String,
    pub provider: Option<String>,
    pub aa_intelligence_index: Option<f64>,
    pub aa_coding_index: Option<f64>,
    pub aa_math_index: Option<f64>,
    pub speed_tps: Option<f64>,
    pub ttft_ms: Option<f64>,
    pub input_price_per_m: Option<f64>,
    pub output_price_per_m: Option<f64>,
    pub context: Option<u64>,
    pub tool_calling: Option<bool>,
    pub vision: Option<bool>,
    pub license: Option<String>,
    pub updated_at: Option<String>,
    /// When CI last fetched this model's AA values (kept on carry-forward when AA was down).
    pub aa_fetched: Option<String>,
    /// Field name → source (`openrouter`, `huggingface`, `artificial-analysis`, ...).
    pub source: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogFile {
    pub schema: u32,
    pub generated_at: Option<String>,
    pub models: Vec<ModelData>,
}

/// Where the data came from on the last fetch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    #[default]
    Published,
    Openrouter,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cached {
    pub fetched_at: u64,
    pub origin: Origin,
    pub catalog: CatalogFile,
}

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub catalog: String,
    pub openrouter: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            catalog: CATALOG_URL.into(),
            openrouter: OPENROUTER_URL.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Cache only, no network (offline flag or privacy mode).
    pub offline: bool,
    /// Ignore a fresh cache.
    pub refresh: bool,
}

#[derive(Debug, Clone)]
pub struct Loaded {
    pub data: Cached,
    pub from_cache: bool,
    /// Non-fatal problems (a source failed, a stale cache was used).
    pub notices: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("no cached model catalog yet; run once without offline or privacy mode")]
    NoCache,
    #[error("model catalog unavailable: {0}")]
    Unavailable(String),
}

/// Read the cache; a missing or corrupt file is `None`.
pub fn read_cache(path: &Path) -> Option<Cached> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn write_cache(path: &Path, c: &Cached) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(c).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

/// Cached, published, or `OpenRouter` data.
///
/// # Errors
/// When nothing is cached and every source failed (or offline with no cache).
pub async fn load(
    cache: Option<&Path>,
    ep: &Endpoints,
    opts: Options,
) -> Result<Loaded, CatalogError> {
    let cached = cache.and_then(read_cache);
    let hit = |data: Cached, notices| Loaded {
        data,
        from_cache: true,
        notices,
    };
    if opts.offline {
        return cached
            .map(|c| hit(c, Vec::new()))
            .ok_or(CatalogError::NoCache);
    }
    if let Some(c) = &cached
        && !opts.refresh
        && now().saturating_sub(c.fetched_at) < TTL_SECS
    {
        return Ok(hit(c.clone(), Vec::new()));
    }

    let mut notices = Vec::new();
    let client = kor_core::net::client_builder()
        .timeout(TIMEOUT)
        .user_agent(format!("kor/{}", kor_core::VERSION))
        .build()
        .map_err(|e| CatalogError::Unavailable(e.to_string()))?;
    let fetched = match get_json(&client, &ep.catalog).await.and_then(|v| {
        serde_json::from_value::<CatalogFile>(v).map_err(|e| format!("{}: {e}", ep.catalog))
    }) {
        Ok(file) => Some((file, Origin::Published)),
        Err(e) => {
            notices.push(e);
            match get_json(&client, &ep.openrouter).await {
                Ok(v) => Some((
                    CatalogFile {
                        schema: 1,
                        generated_at: None,
                        models: from_openrouter(&v),
                    },
                    Origin::Openrouter,
                )),
                Err(e) => {
                    notices.push(e);
                    None
                }
            }
        }
    };
    let Some((catalog, origin)) = fetched else {
        return match cached {
            Some(c) => Ok(hit(c, notices)),
            None => Err(CatalogError::Unavailable(notices.join("; "))),
        };
    };
    let data = Cached {
        fetched_at: now(),
        origin,
        catalog,
    };
    if let Some(p) = cache
        && let Err(e) = write_cache(p, &data)
    {
        notices.push(format!("cache {}: {e}", p.display()));
    }
    Ok(Loaded {
        data,
        from_cache: false,
        notices,
    })
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{url}: {}", kor_core::net::describe(&e)))?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status().as_u16()));
    }
    let body = resp.bytes().await.map_err(|e| format!("{url}: {e}"))?;
    if body.len() > MAX_BYTES {
        return Err(format!("{url}: response too large"));
    }
    serde_json::from_slice(&body).map_err(|e| format!("{url}: {e}"))
}

fn put<T>(
    m: &mut ModelData,
    slot: fn(&mut ModelData) -> &mut Option<T>,
    field: &str,
    v: Option<T>,
    src: &str,
) {
    if let Some(v) = v {
        *slot(m) = Some(v);
        m.source.insert(field.to_owned(), src.to_owned());
    }
}

/// `OpenRouter` prices are USD per token as strings; negative means "varies".
fn per_million(v: &Value) -> Option<f64> {
    let p: f64 = v.as_str()?.parse().ok()?;
    (p >= 0.0).then(|| (p * 1e12).round() / 1e6)
}

/// `OpenRouter` `/api/v1/models` in catalog form (same mapping as `catalog/build_catalog.py`).
pub fn from_openrouter(v: &Value) -> Vec<ModelData> {
    let mut out = Vec::new();
    for r in v["data"].as_array().into_iter().flatten() {
        let Some(id) = r["id"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let mut m = ModelData {
            model_id: id.to_owned(),
            provider: id.split_once('/').map(|(p, _)| p.to_owned()),
            ..ModelData::default()
        };
        let s = SRC_OPENROUTER;
        put(
            &mut m,
            |m| &mut m.input_price_per_m,
            "input_price_per_m",
            per_million(&r["pricing"]["prompt"]),
            s,
        );
        put(
            &mut m,
            |m| &mut m.output_price_per_m,
            "output_price_per_m",
            per_million(&r["pricing"]["completion"]),
            s,
        );
        put(
            &mut m,
            |m| &mut m.context,
            "context",
            r["context_length"].as_u64().filter(|c| *c > 0),
            s,
        );
        let has = |a: &Value, x: &str| a.as_array().map(|a| a.iter().any(|v| v == x));
        put(
            &mut m,
            |m| &mut m.tool_calling,
            "tool_calling",
            has(&r["supported_parameters"], "tools"),
            s,
        );
        put(
            &mut m,
            |m| &mut m.vision,
            "vision",
            has(&r["architecture"]["input_modalities"], "image"),
            s,
        );
        out.push(m);
    }
    out
}

/// `anthropic/claude-3.5-sonnet:free` and `claude-35-sonnet` both become `claude35sonnet`.
fn norm(id: &str) -> String {
    let name = id.rsplit('/').next().unwrap_or(id);
    let name = name.split(':').next().unwrap_or(name);
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The entry for `model_id` (exact, else same normalized name, e.g. `venice/claude-3-5-sonnet`).
pub fn lookup<'a>(models: &'a [ModelData], model_id: &str) -> Option<&'a ModelData> {
    models.iter().find(|m| m.model_id == model_id).or_else(|| {
        let k = norm(model_id);
        models.iter().find(|m| norm(&m.model_id) == k)
    })
}

/// Entries matching `query`: the exact/normalized match alone, else a case-insensitive substring.
pub fn search<'a>(models: &'a [ModelData], query: &str) -> Vec<&'a ModelData> {
    if let Some(m) = lookup(models, query) {
        return vec![m];
    }
    let q = query.to_lowercase();
    models
        .iter()
        .filter(|m| m.model_id.to_lowercase().contains(&q))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn openrouter_doc() -> Value {
        json!({"data": [
            {"id": "anthropic/claude-3.5-sonnet", "context_length": 200_000,
             "pricing": {"prompt": "0.000003", "completion": "0.000015"},
             "supported_parameters": ["tools"], "architecture": {"input_modalities": ["text", "image"]}},
            {"id": "openrouter/auto", "pricing": {"prompt": "-1", "completion": "-1"}}
        ]})
    }

    fn eps(s: &MockServer) -> Endpoints {
        Endpoints {
            catalog: format!("{}/catalog.json", s.uri()),
            openrouter: format!("{}/api/v1/models", s.uri()),
        }
    }

    #[test]
    fn openrouter_mapping_keeps_unknowns_null() {
        let ms = from_openrouter(&openrouter_doc());
        assert_eq!(ms[0].input_price_per_m, Some(3.0));
        assert_eq!(ms[0].output_price_per_m, Some(15.0));
        assert_eq!((ms[0].tool_calling, ms[0].vision), (Some(true), Some(true)));
        assert_eq!(ms[0].source["context"], "openrouter");
        assert_eq!(ms[1].input_price_per_m, None);
        assert_eq!(ms[1].tool_calling, None);
        assert!(ms[1].source.is_empty());
        assert_eq!(ms[0].aa_intelligence_index, None);
    }

    #[test]
    fn lookup_by_normalized_name() {
        let ms = from_openrouter(&openrouter_doc());
        assert_eq!(
            lookup(&ms, "venice/claude-3-5-sonnet").map(|m| m.model_id.as_str()),
            Some("anthropic/claude-3.5-sonnet")
        );
        assert_eq!(search(&ms, "CLAUDE").len(), 1);
        assert_eq!(search(&ms, "o").len(), 2);
    }

    #[tokio::test]
    async fn published_catalog_is_cached_for_a_day() {
        let s = MockServer::start().await;
        let file = json!({"schema": 1, "models": [{"model_id": "a/b", "aa_intelligence_index": 50.0,
            "source": {"aa_intelligence_index": "artificial-analysis"}}]});
        Mock::given(method("GET"))
            .and(path("/catalog.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file))
            .expect(1)
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("c").join("model-catalog.json");
        let l = load(Some(&cache), &eps(&s), Options::default())
            .await
            .unwrap();
        assert!(!l.from_cache && l.notices.is_empty());
        assert_eq!(l.data.origin, Origin::Published);
        assert_eq!(l.data.catalog.models[0].aa_intelligence_index, Some(50.0));
        let again = load(Some(&cache), &eps(&s), Options::default())
            .await
            .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.data.catalog, l.data.catalog);
    }

    #[tokio::test]
    async fn falls_back_to_openrouter_then_stale_cache() {
        let s = MockServer::start().await;
        Mock::given(path("/catalog.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        Mock::given(path("/api/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(openrouter_doc()))
            .up_to_n_times(1)
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("model-catalog.json");
        let l = load(Some(&cache), &eps(&s), Options::default())
            .await
            .unwrap();
        assert_eq!(l.data.origin, Origin::Openrouter);
        assert_eq!(l.data.catalog.models.len(), 2);
        assert_eq!(l.notices.len(), 1);

        // Both sources down now: the stale cache still answers, with the reasons.
        let opts = Options {
            refresh: true,
            ..Options::default()
        };
        let l = load(Some(&cache), &eps(&s), opts).await.unwrap();
        assert!(l.from_cache);
        assert_eq!(l.notices.len(), 2);
    }

    #[tokio::test]
    async fn offline_reads_only_the_cache() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("model-catalog.json");
        let offline = Options {
            offline: true,
            ..Options::default()
        };
        assert!(matches!(
            load(Some(&cache), &eps(&s), offline).await,
            Err(CatalogError::NoCache)
        ));
        let old = Cached {
            fetched_at: 1,
            ..Cached::default()
        };
        write_cache(&cache, &old).unwrap();
        let l = load(Some(&cache), &eps(&s), offline).await.unwrap();
        assert!(l.from_cache);
        assert_eq!(l.data.fetched_at, 1);
    }

    /// AA scores come only from the stored central catalog: once downloaded, they are read
    /// offline with no request to any host (and never to Artificial Analysis).
    #[tokio::test]
    async fn stored_catalog_serves_aa_scores_offline() {
        let online = MockServer::start().await;
        let file = json!({"schema": 1, "generated_at": "2026-11-10T04:17:00+00:00", "models": [
            {"model_id": "anthropic/claude-3.5-sonnet", "aa_intelligence_index": 44.1,
             "aa_coding_index": 38.0, "aa_math_index": 51.5, "speed_tps": 80.5, "ttft_ms": 900.0,
             "aa_fetched": "2026-11-09T04:17:00+00:00",
             "source": {"aa_intelligence_index": SRC_AA, "aa_coding_index": SRC_AA,
                        "aa_math_index": SRC_AA, "speed_tps": SRC_AA, "ttft_ms": SRC_AA}},
            {"model_id": "openrouter/auto", "source": {}}]});
        Mock::given(path("/catalog.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file))
            .expect(1)
            .mount(&online)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("kor").join("model-catalog.json");
        load(Some(&cache), &eps(&online), Options::default())
            .await
            .unwrap();

        let offline_net = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&offline_net)
            .await;
        let opts = Options {
            offline: true,
            ..Options::default()
        };
        let l = load(Some(&cache), &eps(&offline_net), opts).await.unwrap();
        assert!(l.from_cache && l.notices.is_empty());
        let m = lookup(&l.data.catalog.models, "venice/claude-3-5-sonnet").unwrap();
        assert_eq!(
            (m.aa_intelligence_index, m.aa_coding_index, m.aa_math_index),
            (Some(44.1), Some(38.0), Some(51.5))
        );
        assert_eq!(m.source["aa_math_index"], SRC_AA);
        assert_eq!(m.aa_fetched.as_deref(), Some("2026-11-09T04:17:00+00:00"));
        // A model without AA data has no scores (shown as "-"), not an error.
        let bare = lookup(&l.data.catalog.models, "openrouter/auto").unwrap();
        assert_eq!(bare.aa_intelligence_index, None);
        // The direct read path used by cost/context lookups sees the same stored copy.
        assert_eq!(read_cache(&cache).unwrap().catalog, l.data.catalog);
    }

    #[test]
    fn old_cache_with_live_flag_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("model-catalog.json");
        std::fs::write(
            &p,
            r#"{"fetched_at":5,"origin":"published","aa_live":true,"catalog":{"schema":1,"models":[]}}"#,
        )
        .unwrap();
        assert_eq!(read_cache(&p).unwrap().fetched_at, 5);
    }
}
