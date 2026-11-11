//! Bing Webmaster Tools (spec §29.6), read-only: query and page statistics with the user's own
//! API key (Bing Webmaster → Settings → API access). The key is a vault entry; Bing's JSON API takes
//! it as the `apikey` query parameter (its documented method), over https only.

use kor_secrets::{ExposeSecret as _, SecretStore, SecretString, vault};
use serde::Serialize;
use serde_json::Value;

pub const VAULT_KEY: &str = "KOR_SEO_BING_KEY";
pub const BASE: &str = "https://ssl.bing.com/webmaster/api.svc/json";

pub fn save_key(store: &dyn SecretStore, key: &str) -> Result<(), String> {
    let k = key.trim();
    if k.len() < 16 || k.len() > 128 || !k.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Bing API key: 16-128 letters and digits".into());
    }
    vault::add(store, VAULT_KEY, &SecretString::from(k.to_owned()))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn has_key(store: &dyn SecretStore) -> bool {
    vault::get(store, VAULT_KEY).ok().flatten().is_some()
}

/// One row of `GetQueryStats` / `GetPageStats` (`key` = query or page URL), summed over the period.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stat {
    pub key: String,
    pub clicks: f64,
    pub impressions: f64,
    pub avg_click_position: f64,
    pub avg_impression_position: f64,
}

/// `kind` = `GetQueryStats` or `GetPageStats`. `base` is [`BASE`] (tests point it elsewhere).
pub async fn stats(
    base: &str,
    store: &dyn SecretStore,
    kind: &str,
    site: &str,
) -> Result<Vec<Stat>, String> {
    if !matches!(kind, "GetQueryStats" | "GetPageStats") {
        return Err(format!("bad Bing call {kind}"));
    }
    let key = vault::get(store, VAULT_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("no Bing API key (kor seo bing setup)")?;
    let mut u = url::Url::parse(&format!("{base}/{kind}")).map_err(|e| e.to_string())?;
    u.query_pairs_mut()
        .append_pair("siteUrl", site)
        .append_pair("apikey", key.expose_secret());
    kor_core::net::check_url(u.as_str()).map_err(|e| e.to_string())?;
    let client = kor_core::net::client_builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .get(u)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    let status = r.status();
    let v: Value = r
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    if !status.is_success() {
        // Never echo the URL: it carries the key.
        return Err(format!(
            "Bing Webmaster HTTP {}: {}",
            status.as_u16(),
            v["Message"].as_str().unwrap_or("")
        ));
    }
    let mut by: std::collections::BTreeMap<String, Stat> = std::collections::BTreeMap::new();
    let mut weight: std::collections::HashMap<String, (f64, f64)> =
        std::collections::HashMap::new();
    for row in v["d"].as_array().into_iter().flatten() {
        let k = row["Query"].as_str().unwrap_or("").to_owned();
        let n = |f: &str| row[f].as_f64().unwrap_or(0.0);
        let s = by.entry(k.clone()).or_insert_with(|| Stat {
            key: k.clone(),
            ..Stat::default()
        });
        s.clicks += n("Clicks");
        s.impressions += n("Impressions");
        let w = weight.entry(k).or_default();
        // Impression-weighted average positions over the period's daily rows.
        w.0 += n("AvgClickPosition") * n("Clicks");
        w.1 += n("AvgImpressionPosition") * n("Impressions");
    }
    let mut out: Vec<Stat> = by
        .into_values()
        .map(|mut s| {
            let (a, b) = weight[&s.key];
            s.avg_click_position = if s.clicks > 0.0 { a / s.clicks } else { 0.0 };
            s.avg_impression_position = if s.impressions > 0.0 {
                b / s.impressions
            } else {
                0.0
            };
            s
        })
        .collect();
    out.sort_by(|a, b| {
        b.clicks
            .total_cmp(&a.clicks)
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_secrets::MemoryStore;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn sums_daily_rows() {
        let s = MockServer::start().await;
        Mock::given(method("GET")).and(path("/GetQueryStats")).and(query_param("apikey", "ABCDEF0123456789")).and(query_param("siteUrl", "https://x.test/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"d": [
                {"Query": "kargo", "Clicks": 2, "Impressions": 10, "AvgClickPosition": 3, "AvgImpressionPosition": 4},
                {"Query": "kargo", "Clicks": 2, "Impressions": 30, "AvgClickPosition": 5, "AvgImpressionPosition": 8},
                {"Query": "gümrük", "Clicks": 1, "Impressions": 5, "AvgClickPosition": 9, "AvgImpressionPosition": 9}]})))
            .mount(&s).await;
        let store = MemoryStore::default();
        assert!(
            stats(&s.uri(), &store, "GetQueryStats", "https://x.test/")
                .await
                .is_err()
        );
        assert!(save_key(&store, "short").is_err());
        save_key(&store, "ABCDEF0123456789").unwrap();
        assert!(has_key(&store));
        let r = stats(&s.uri(), &store, "GetQueryStats", "https://x.test/")
            .await
            .unwrap();
        assert_eq!(r[0].key, "kargo");
        assert_eq!((r[0].clicks, r[0].impressions), (4.0, 40.0));
        assert!((r[0].avg_impression_position - 7.0).abs() < 1e-9);
        assert!((r[0].avg_click_position - 4.0).abs() < 1e-9);
    }
}
