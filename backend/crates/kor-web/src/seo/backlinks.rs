//! Backlink connectors for competitor watch (§29.10): Ahrefs API v3, Semrush Analytics API and
//! Moz Links API v2. Each needs the user's own key (vault names `AHREFS_API_KEY`,
//! `SEMRUSH_API_KEY`, `MOZ_ACCESS_ID` + `MOZ_SECRET_KEY`); Kor only reads summary numbers.
//!
//! - Ahrefs: `GET /v3/site-explorer/domain-rating` and `/backlinks-stats`, `Authorization: Bearer`.
//! - Semrush: `GET /analytics/v1/?type=backlinks_overview&key=…` (CSV, `;`-separated).
//! - Moz: `POST /v2/url_metrics`, HTTP Basic with access id and secret.

use base64::Engine as _;
use serde::Serialize;
use serde_json::{Value, json};

use crate::site::http::Http;

pub const AHREFS: &str = "https://api.ahrefs.com/v3";
pub const SEMRUSH: &str = "https://api.semrush.com";
pub const MOZ: &str = "https://lsapi.seomoz.com/v2";

/// One source's numbers for a domain. `authority` is 0–100 on the source's own scale (Ahrefs
/// DR, Semrush Authority Score, Moz DA), so sources are not compared with each other.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Backlinks {
    pub source: String,
    pub domain: String,
    pub authority: Option<f64>,
    pub backlinks: Option<u64>,
    pub ref_domains: Option<u64>,
}

/// A bare host such as `example.com` (no scheme or path reaches the query string).
pub fn domain_of(target: &str) -> Result<String, String> {
    let t = target.trim();
    let host = if t.contains("://") {
        url::Url::parse(t)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default()
    } else {
        t.split('/').next().unwrap_or_default().to_owned()
    };
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    if crate::site::tr::registrar::valid_domain(&host) {
        Ok(host)
    } else {
        Err(format!("not a domain: `{target}`"))
    }
}

fn q(pairs: &[(&str, &str)]) -> String {
    crate::site::http::form(pairs)
}

fn err(source: &str, r: &crate::site::http::Resp) -> String {
    let detail = r
        .json()
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| {
                    e.as_str()
                        .map(str::to_owned)
                        .or_else(|| e.get("message").and_then(Value::as_str).map(str::to_owned))
                })
                .or_else(|| v.get("message").and_then(Value::as_str).map(str::to_owned))
        })
        .unwrap_or_else(|| {
            r.body
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect()
        });
    format!("{source}: HTTP {} {detail}", r.status)
}

pub async fn ahrefs(
    http: &Http,
    base: &str,
    key: &str,
    domain: &str,
    date: &str,
) -> Result<Backlinks, String> {
    let d = domain_of(domain)?;
    let h = [
        ("authorization", format!("Bearer {key}")),
        ("accept", "application/json".into()),
    ];
    let get = |path: &str| {
        let url = format!(
            "{base}/site-explorer/{path}?{}",
            q(&[("target", &d), ("date", date), ("mode", "domain")])
        );
        let h = h.clone();
        async move { http.get_with(&url, &h).await }
    };
    let dr = get("domain-rating").await?;
    if !dr.ok() {
        return Err(err("Ahrefs", &dr));
    }
    let st = get("backlinks-stats").await?;
    if !st.ok() {
        return Err(err("Ahrefs", &st));
    }
    let (dr, st) = (dr.json()?, st.json()?);
    Ok(Backlinks {
        source: "ahrefs".into(),
        domain: d,
        authority: dr
            .pointer("/domain_rating/domain_rating")
            .and_then(Value::as_f64),
        backlinks: st.pointer("/metrics/live").and_then(Value::as_u64),
        ref_domains: st
            .pointer("/metrics/live_refdomains")
            .and_then(Value::as_u64),
    })
}

pub async fn semrush(
    http: &Http,
    base: &str,
    key: &str,
    domain: &str,
) -> Result<Backlinks, String> {
    let d = domain_of(domain)?;
    let url = format!(
        "{base}/analytics/v1/?{}",
        q(&[
            ("key", key),
            ("type", "backlinks_overview"),
            ("target", &d),
            ("target_type", "root_domain"),
            ("export_columns", "ascore,total,domains_num"),
        ])
    );
    let r = http.get(&url).await?;
    // Semrush answers errors as `ERROR 50 :: NOTHING FOUND` with HTTP 200.
    if !r.ok() || r.body.starts_with("ERROR") {
        return Err(format!(
            "Semrush: {}",
            r.body
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    let mut lines = r.body.lines();
    let head: Vec<&str> = lines
        .next()
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .collect();
    let row: Vec<&str> = lines
        .next()
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .collect();
    let col = |n: &str| {
        head.iter()
            .position(|h| *h == n)
            .and_then(|i| row.get(i).copied())
    };
    Ok(Backlinks {
        source: "semrush".into(),
        domain: d,
        authority: col("ascore").and_then(|v| v.parse().ok()),
        backlinks: col("total").and_then(|v| v.parse().ok()),
        ref_domains: col("domains_num").and_then(|v| v.parse().ok()),
    })
}

pub async fn moz(
    http: &Http,
    base: &str,
    access_id: &str,
    secret: &str,
    domain: &str,
) -> Result<Backlinks, String> {
    let d = domain_of(domain)?;
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{access_id}:{secret}"))
    );
    let r = http
        .post_json(
            &format!("{base}/url_metrics"),
            &[("authorization", auth)],
            &json!({ "targets": [d] }),
        )
        .await?;
    if !r.ok() {
        return Err(err("Moz", &r));
    }
    let v = r.json()?;
    let m = v.pointer("/results/0").cloned().unwrap_or_default();
    Ok(Backlinks {
        source: "moz".into(),
        domain: d,
        authority: m.get("domain_authority").and_then(Value::as_f64),
        backlinks: m
            .get("external_pages_to_root_domain")
            .and_then(Value::as_u64),
        ref_domains: m.get("root_domains_to_root_domain").and_then(Value::as_u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn three_sources_against_mocks() {
        let s = MockServer::start().await;
        Mock::given(path("/site-explorer/domain-rating"))
            .and(header("authorization", "Bearer ak"))
            .and(query_param("target", "rakip.com"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"domain_rating": {"domain_rating": 41.0, "ahrefs_rank": 9}}),
                ),
            )
            .mount(&s)
            .await;
        Mock::given(path("/site-explorer/backlinks-stats"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"metrics": {"live": 1200, "live_refdomains": 80}})),
            )
            .mount(&s)
            .await;
        Mock::given(path("/analytics/v1/"))
            .and(query_param("type", "backlinks_overview"))
            .and(query_param("target", "rakip.com"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("ascore;total;domains_num\n35;900;70\n"),
            )
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/url_metrics"))
            .and(header("authorization", "Basic aWQ6c2Vj"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [
                {"domain_authority": 38, "external_pages_to_root_domain": 1000, "root_domains_to_root_domain": 75}]})))
            .mount(&s)
            .await;
        let http = Http::new().unwrap();
        let a = ahrefs(
            &http,
            &s.uri(),
            "ak",
            "https://www.rakip.com/x",
            "2026-11-01",
        )
        .await
        .unwrap();
        assert_eq!(
            (a.authority, a.backlinks, a.ref_domains),
            (Some(41.0), Some(1200), Some(80))
        );
        let m = semrush(&http, &s.uri(), "sk", "rakip.com").await.unwrap();
        assert_eq!((m.authority, m.ref_domains), (Some(35.0), Some(70)));
        let z = moz(&http, &s.uri(), "id", "sec", "rakip.com")
            .await
            .unwrap();
        assert_eq!((z.authority, z.backlinks), (Some(38.0), Some(1000)));

        let bad = MockServer::start().await;
        Mock::given(path("/analytics/v1/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("ERROR 120 :: WRONG KEY - ID PAIR"),
            )
            .mount(&bad)
            .await;
        Mock::given(path("/site-explorer/domain-rating"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(json!({"error": "invalid token"})),
            )
            .mount(&bad)
            .await;
        let e = semrush(&http, &bad.uri(), "secret-key", "rakip.com")
            .await
            .unwrap_err();
        assert!(e.contains("WRONG KEY") && !e.contains("secret-key"));
        assert!(
            ahrefs(&http, &bad.uri(), "k", "rakip.com", "2026-11-01")
                .await
                .unwrap_err()
                .contains("invalid token")
        );
        assert!(domain_of("x y").is_err());
    }
}
