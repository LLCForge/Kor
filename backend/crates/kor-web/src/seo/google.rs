//! Google Search Console + GA4, read-only (spec §29.6).
//!
//! OAuth uses the **user's own** Google Cloud OAuth client ("Desktop app" type): Kor ships no
//! client id and never borrows another vendor's. The installed-app loopback flow (RFC 8252) with
//! PKCE (RFC 7636): Kor listens on `127.0.0.1:<random port>`, the browser returns the code there.
//! Client id/secret and the refresh token live in the vault (OS keychain); access tokens only in
//! memory. Scopes are read-only (`webmasters.readonly`, `analytics.readonly`). Fetched rows are
//! cached in a local SQLite file for comparisons and the opportunity score.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use kor_secrets::{ExposeSecret as _, SecretStore, SecretString, vault};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const VAULT_CLIENT_ID: &str = "KOR_SEO_GOOGLE_CLIENT_ID";
pub const VAULT_CLIENT_SECRET: &str = "KOR_SEO_GOOGLE_CLIENT_SECRET";
pub const VAULT_REFRESH: &str = "KOR_SEO_GOOGLE_REFRESH_TOKEN";
pub const SCOPES: &str = "https://www.googleapis.com/auth/webmasters.readonly https://www.googleapis.com/auth/analytics.readonly";
/// How long the loopback listener waits for the browser.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// Rows per Search Console request (API maximum 25 000).
pub const MAX_ROWS: u32 = 25_000;

/// Google endpoints (overridable for tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub gsc: String,
    pub ga: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            gsc: "https://www.googleapis.com/webmasters/v3".into(),
            ga: "https://analyticsdata.googleapis.com/v1beta".into(),
        }
    }
}

/// The user's OAuth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub id: String,
    pub secret: String,
}

/// Google's downloaded `client_secret_*.json` (`installed` = Desktop app).
pub fn parse_client_json(text: &str) -> Result<OAuthClient, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let c = v.get("installed").ok_or(
        "not a Desktop-app OAuth client (the JSON has no \"installed\" section); create one of type \"Desktop app\"",
    )?;
    let id = c["client_id"].as_str().unwrap_or("").trim().to_owned();
    let secret = c["client_secret"].as_str().unwrap_or("").trim().to_owned();
    if !id.ends_with(".apps.googleusercontent.com") || secret.is_empty() {
        return Err("client_id / client_secret missing or not a Google OAuth client".into());
    }
    Ok(OAuthClient { id, secret })
}

pub fn save_client(store: &dyn SecretStore, c: &OAuthClient) -> Result<(), String> {
    vault::add(store, VAULT_CLIENT_ID, &SecretString::from(c.id.clone()))
        .map_err(|e| e.to_string())?;
    vault::add(
        store,
        VAULT_CLIENT_SECRET,
        &SecretString::from(c.secret.clone()),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The configured client, if the user set one up.
pub fn client(store: &dyn SecretStore) -> Option<OAuthClient> {
    let id = vault::get(store, VAULT_CLIENT_ID).ok()??;
    let secret = vault::get(store, VAULT_CLIENT_SECRET).ok()??;
    Some(OAuthClient {
        id: id.expose_secret().to_owned(),
        secret: secret.expose_secret().to_owned(),
    })
}

/// Setup state for the UI: client configured, signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Status {
    pub client: bool,
    pub signed_in: bool,
}

pub fn status(store: &dyn SecretStore) -> Status {
    Status {
        client: client(store).is_some(),
        signed_in: vault::get(store, VAULT_REFRESH).ok().flatten().is_some(),
    }
}

/// Forget the refresh token (the client stays).
pub fn logout(store: &dyn SecretStore) -> Result<bool, String> {
    vault::remove(store, VAULT_REFRESH).map_err(|e| e.to_string())
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b64url(&b))
}

/// PKCE verifier/challenge and the CSRF `state`.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

pub fn pkce() -> Result<Pkce, String> {
    use sha2::Digest as _;
    let verifier = random(32)?;
    let challenge = b64url(&sha2::Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
        state: random(16)?,
    })
}

pub fn auth_url(
    ep: &Endpoints,
    c: &OAuthClient,
    redirect: &str,
    p: &Pkce,
) -> Result<String, String> {
    let mut u = url::Url::parse(&ep.auth).map_err(|e| e.to_string())?;
    u.query_pairs_mut()
        .append_pair("client_id", &c.id)
        .append_pair("redirect_uri", redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &p.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &p.state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    Ok(u.into())
}

fn http() -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn token(ep: &Endpoints, form: &[(&str, &str)]) -> Result<TokenResp, String> {
    kor_core::net::check_url(&ep.token).map_err(|e| e.to_string())?;
    // reqwest's `form` feature is off in this workspace; encode by hand.
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let r = http()?
        .post(&ep.token)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    let status = r.status();
    let v: Value = serde_json::from_slice(&r.bytes().await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "Google token endpoint: {} {}",
            v["error"].as_str().unwrap_or(""),
            v["error_description"].as_str().unwrap_or("")
        ));
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// Wait on `listener` for the browser's redirect; returns the code after checking `state`.
async fn wait_for_code(listener: tokio::net::TcpListener, state: &str) -> Result<String, String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    loop {
        let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        while n < buf.len() {
            let k = sock.read(&mut buf[n..]).await.map_err(|e| e.to_string())?;
            if k == 0 {
                break;
            }
            n += k;
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head.split_whitespace().nth(1).unwrap_or("/");
        let u = url::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|e| e.to_string())?;
        let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
        let (body, result) = match (q.get("state"), q.get("code"), q.get("error")) {
            (Some(s), Some(code), _) if s == state => (
                "Kor: sign-in complete. You can close this tab.",
                Some(Ok(code.clone())),
            ),
            (Some(s), _, Some(err)) if s == state => (
                "Kor: sign-in was cancelled.",
                Some(Err(format!("Google: {err}"))),
            ),
            // favicon or a stray request: keep waiting.
            _ => ("Kor", None),
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        if let Some(r) = result {
            return r;
        }
    }
}

/// Full installed-app flow: listen on loopback, hand the consent URL to `open` (browser or
/// printed), take the code, exchange it, store the refresh token in the vault.
pub async fn login(
    ep: &Endpoints,
    store: &dyn SecretStore,
    open: &(dyn Fn(&str) + Sync),
    timeout: Duration,
) -> Result<(), String> {
    let c = client(store).ok_or("no OAuth client configured")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let p = pkce()?;
    open(&auth_url(ep, &c, &redirect, &p)?);
    let code = tokio::time::timeout(timeout, wait_for_code(listener, &p.state))
        .await
        .map_err(|_| "timed out waiting for the browser".to_owned())??;
    let t = token(
        ep,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &c.id),
            ("client_secret", &c.secret),
            ("redirect_uri", &redirect),
            ("code_verifier", &p.verifier),
        ],
    )
    .await?;
    let refresh = t.refresh_token.ok_or("Google returned no refresh token")?;
    vault::add(store, VAULT_REFRESH, &SecretString::from(refresh)).map_err(|e| e.to_string())?;
    Ok(())
}

/// One Search Console row (`keys` follow the requested dimensions).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub keys: Vec<String>,
    pub clicks: f64,
    pub impressions: f64,
    pub ctr: f64,
    pub position: f64,
}

/// An authorized session.
#[derive(Debug, Clone)]
pub struct Google {
    ep: Endpoints,
    http: reqwest::Client,
    access: String,
}

impl Google {
    /// Refresh an access token from the vault's refresh token.
    pub async fn connect(ep: Endpoints, store: &dyn SecretStore) -> Result<Self, String> {
        let c = client(store).ok_or("no OAuth client configured")?;
        let refresh = vault::get(store, VAULT_REFRESH)
            .map_err(|e| e.to_string())?
            .ok_or("not signed in")?;
        let t = token(
            &ep,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.expose_secret()),
                ("client_id", &c.id),
                ("client_secret", &c.secret),
            ],
        )
        .await?;
        Ok(Self {
            ep,
            http: http()?,
            access: t.access_token,
        })
    }

    async fn call(&self, req: reqwest::RequestBuilder) -> Result<Value, String> {
        let r = req
            .bearer_auth(&self.access)
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
        if status.is_success() {
            Ok(v)
        } else {
            Err(format!(
                "Google API HTTP {}: {}",
                status.as_u16(),
                v["error"]["message"].as_str().unwrap_or("")
            ))
        }
    }

    /// Verified Search Console properties.
    pub async fn sites(&self) -> Result<Vec<String>, String> {
        let v = self
            .call(self.http.get(format!("{}/sites", self.ep.gsc)))
            .await?;
        Ok(v["siteEntry"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["siteUrl"].as_str().map(str::to_owned))
            .collect())
    }

    /// `searchAnalytics.query`: dates `YYYY-MM-DD`, dimensions `query`, `page`, `country`, `device`, `date`.
    pub async fn search_analytics(
        &self,
        site: &str,
        start: &str,
        end: &str,
        dims: &[&str],
        limit: u32,
    ) -> Result<Vec<Row>, String> {
        let enc = percent_encoding::utf8_percent_encode(site, percent_encoding::NON_ALPHANUMERIC);
        let body = json!({ "startDate": start, "endDate": end, "dimensions": dims, "rowLimit": limit.clamp(1, MAX_ROWS) });
        let v = self
            .call(
                self.http
                    .post(format!("{}/sites/{enc}/searchAnalytics/query", self.ep.gsc))
                    .header("content-type", "application/json")
                    .body(body.to_string()),
            )
            .await?;
        serde_json::from_value(v.get("rows").cloned().unwrap_or(json!([])))
            .map_err(|e| e.to_string())
    }

    /// GA4 `runReport` (property id digits): rows of `{dimension…, metric…}`.
    pub async fn ga_report(
        &self,
        property: &str,
        start: &str,
        end: &str,
        dims: &[&str],
        metrics: &[&str],
    ) -> Result<Vec<Value>, String> {
        if property.is_empty() || !property.bytes().all(|b| b.is_ascii_digit()) {
            return Err("GA4 property id must be digits".into());
        }
        let body = json!({
            "dateRanges": [{ "startDate": start, "endDate": end }],
            "dimensions": dims.iter().map(|d| json!({ "name": d })).collect::<Vec<_>>(),
            "metrics": metrics.iter().map(|m| json!({ "name": m })).collect::<Vec<_>>(),
            "limit": 10_000,
        });
        let v = self
            .call(
                self.http
                    .post(format!("{}/properties/{property}:runReport", self.ep.ga))
                    .header("content-type", "application/json")
                    .body(body.to_string()),
            )
            .await?;
        let names: Vec<String> = v["dimensionHeaders"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(v["metricHeaders"].as_array().into_iter().flatten())
            .filter_map(|h| h["name"].as_str().map(str::to_owned))
            .collect();
        Ok(v["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                let vals = r["dimensionValues"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(r["metricValues"].as_array().into_iter().flatten());
                let mut o = serde_json::Map::new();
                for (n, x) in names.iter().zip(vals) {
                    let s = x["value"].as_str().unwrap_or("");
                    o.insert(
                        n.clone(),
                        s.parse::<f64>().map_or_else(|_| json!(s), |f| json!(f)),
                    );
                }
                Value::Object(o)
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------------------------
// Local cache (SQLite)
// ---------------------------------------------------------------------------------------------

fn open(path: &Path) -> Result<Connection, String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let c = Connection::open(path).map_err(|e| e.to_string())?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS gsc (site TEXT NOT NULL, start TEXT NOT NULL, end TEXT NOT NULL, dims TEXT NOT NULL,
           keys TEXT NOT NULL, clicks REAL, impressions REAL, ctr REAL, position REAL, fetched INTEGER,
           PRIMARY KEY (site, start, end, dims, keys));
         CREATE TABLE IF NOT EXISTS ga (property TEXT NOT NULL, start TEXT NOT NULL, end TEXT NOT NULL, spec TEXT NOT NULL,
           rows TEXT NOT NULL, fetched INTEGER, PRIMARY KEY (property, start, end, spec));",
    )
    .map_err(|e| e.to_string())?;
    Ok(c)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn store_rows(
    path: &Path,
    site: &str,
    start: &str,
    end: &str,
    dims: &[&str],
    rows: &[Row],
) -> Result<(), String> {
    let mut c = open(path)?;
    let tx = c.transaction().map_err(|e| e.to_string())?;
    let d = dims.join(",");
    tx.execute(
        "DELETE FROM gsc WHERE site=?1 AND start=?2 AND end=?3 AND dims=?4",
        params![site, start, end, d],
    )
    .map_err(|e| e.to_string())?;
    for r in rows {
        tx.execute(
            "INSERT OR REPLACE INTO gsc VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                site,
                start,
                end,
                d,
                serde_json::to_string(&r.keys).unwrap_or_default(),
                r.clicks,
                r.impressions,
                r.ctr,
                r.position,
                now_ms()
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn load_rows(
    path: &Path,
    site: &str,
    start: &str,
    end: &str,
    dims: &[&str],
) -> Result<Vec<Row>, String> {
    let c = open(path)?;
    let mut st = c
        .prepare("SELECT keys, clicks, impressions, ctr, position FROM gsc WHERE site=?1 AND start=?2 AND end=?3 AND dims=?4 ORDER BY clicks DESC")
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map(params![site, start, end, dims.join(",")], |r| {
            let keys: String = r.get(0)?;
            Ok(Row {
                keys: serde_json::from_str(&keys).unwrap_or_default(),
                clicks: r.get(1)?,
                impressions: r.get(2)?,
                ctr: r.get(3)?,
                position: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub fn store_ga(
    path: &Path,
    property: &str,
    start: &str,
    end: &str,
    spec: &str,
    rows: &[Value],
) -> Result<(), String> {
    let c = open(path)?;
    c.execute(
        "INSERT OR REPLACE INTO ga VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            property,
            start,
            end,
            spec,
            serde_json::to_string(rows).unwrap_or_default(),
            now_ms()
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// "Which pages lost clicks, and why?"
// ---------------------------------------------------------------------------------------------

/// A page that lost clicks between two periods, with deterministic cause hints.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Drop {
    pub page: String,
    pub clicks_before: f64,
    pub clicks_now: f64,
    pub impressions_before: f64,
    pub impressions_now: f64,
    pub position_before: f64,
    pub position_now: f64,
    pub ctr_before: f64,
    pub ctr_now: f64,
    /// Hint ids: `position`, `demand`, `snippet`, `gone`; plus audit rules found on the page.
    pub hints: Vec<String>,
}

/// Pages (rows keyed by page) sorted by click loss, worst first. Audit findings for the page
/// (from the latest `kor seo audit`) are added as hints.
pub fn drops(
    before: &[Row],
    now: &[Row],
    audit: Option<&kor_runtime::analysis::Report>,
    n: usize,
) -> Vec<Drop> {
    let key = |r: &Row| r.keys.first().cloned().unwrap_or_default();
    let cur: std::collections::HashMap<String, &Row> = now.iter().map(|r| (key(r), r)).collect();
    let mut out: Vec<Drop> = before
        .iter()
        .filter_map(|b| {
            let page = key(b);
            let zero = Row::default();
            let c = cur.get(&page).copied().unwrap_or(&zero);
            if c.clicks >= b.clicks {
                return None;
            }
            let mut hints = Vec::new();
            if c.impressions == 0.0 {
                hints.push("gone".to_owned());
            } else {
                if c.position > b.position + 1.0 {
                    hints.push("position".into());
                }
                if c.impressions < b.impressions * 0.8 && c.position <= b.position + 1.0 {
                    hints.push("demand".into());
                }
                if c.ctr < b.ctr * 0.8 && (c.position - b.position).abs() <= 1.0 {
                    hints.push("snippet".into());
                }
            }
            if let Some(r) = audit {
                for f in r
                    .findings
                    .iter()
                    .filter(|f| f.file.to_string_lossy() == page.as_str())
                {
                    if !hints.contains(&f.rule) {
                        hints.push(f.rule.clone());
                    }
                }
            }
            Some(Drop {
                page,
                clicks_before: b.clicks,
                clicks_now: c.clicks,
                impressions_before: b.impressions,
                impressions_now: c.impressions,
                position_before: b.position,
                position_now: c.position,
                ctr_before: b.ctr,
                ctr_now: c.ctr,
                hints,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (b.clicks_before - b.clicks_now).total_cmp(&(a.clicks_before - a.clicks_now))
    });
    out.truncate(n);
    out
}

/// Prompt for "why did these pages drop, and what to do" (spec §29.6): the model sees only the
/// deterministic figures and hints and must not invent other causes as facts.
pub fn drops_prompt(d: &[Drop], lang: &str) -> String {
    use std::fmt::Write as _;
    let mut s = format!(
        "You are an SEO analyst. For each page below, give the most likely cause (a hypothesis, labelled as such) \
         and one concrete action, in language `{lang}`. Use only these figures and hints (hint ids: position = ranking fell, \
         demand = fewer impressions at the same rank, snippet = CTR fell at the same rank, gone = no longer shown; \
         seo.* = technical audit findings on that page). Numbered list, one line each.\n\n"
    );
    for (i, x) in d.iter().enumerate() {
        let _ = writeln!(
            s,
            "{}. {} clicks {:.0}→{:.0}, impressions {:.0}→{:.0}, CTR {:.3}→{:.3}, position {:.1}→{:.1}; hints: {}",
            i + 1,
            x.page,
            x.clicks_before,
            x.clicks_now,
            x.impressions_before,
            x.impressions_now,
            x.ctr_before,
            x.ctr_now,
            x.position_before,
            x.position_now,
            x.hints.join(", ")
        );
    }
    s
}

/// Routine that checks for traffic drops on a schedule and notifies (`--alert`).
pub fn drops_routine(
    root: &Path,
    site: &str,
    threshold: f64,
    every: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let (minutes, days) = match every {
        "daily" => (24 * 60, 7),
        "weekly" => (7 * 24 * 60, 7),
        _ => return Err("schedule: daily or weekly".into()),
    };
    if site.is_empty()
        || site.contains(['`', '\n', ';', '&', '|', ' '])
        || !threshold.is_finite()
        || threshold <= 0.0
    {
        return Err("bad site or threshold".into());
    }
    let mut m = serde_json::Map::new();
    m.insert(
        "name".into(),
        format!("Search Console drop watch: {site} ({every})").into(),
    );
    m.insert("prompt".into(), format!("Run the shell command `kor seo gsc drops {site} --days {days} --alert {threshold}` in this folder and summarize which pages lost clicks and the hints shown. Do not change any files.").into());
    m.insert("intervalMinutes".into(), minutes.into());
    m.insert("root".into(), root.to_string_lossy().into_owned().into());
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_secrets::MemoryStore;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ep(s: &MockServer) -> Endpoints {
        Endpoints {
            auth: format!("{}/auth", s.uri()),
            token: format!("{}/token", s.uri()),
            gsc: format!("{}/gsc", s.uri()),
            ga: format!("{}/ga", s.uri()),
        }
    }

    #[test]
    fn client_json_must_be_desktop_app() {
        let ok =
            r#"{"installed":{"client_id":"1-abc.apps.googleusercontent.com","client_secret":"s"}}"#;
        assert_eq!(
            parse_client_json(ok).unwrap().id,
            "1-abc.apps.googleusercontent.com"
        );
        assert!(parse_client_json(r#"{"web":{"client_id":"x","client_secret":"s"}}"#).is_err());
        assert!(
            parse_client_json(r#"{"installed":{"client_id":"x","client_secret":"s"}}"#).is_err()
        );
    }

    #[tokio::test]
    async fn loopback_login_then_queries() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=THECODE"))
            .and(body_string_contains("code_verifier="))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token": "A1", "refresh_token": "R1", "expires_in": 3600}),
            ))
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=R1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "A2", "expires_in": 3600})),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/gsc/sites"))
            .and(header("authorization", "Bearer A2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"siteEntry": [{"siteUrl": "sc-domain:x.test"}]})),
            )
            .mount(&s)
            .await;
        Mock::given(method("POST")).and(path("/gsc/sites/sc%2Ddomain%3Ax%2Etest/searchAnalytics/query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"rows": [{"keys": ["https://x.test/a"], "clicks": 5.0, "impressions": 100.0, "ctr": 0.05, "position": 7.2}]})))
            .mount(&s).await;
        Mock::given(method("POST")).and(path("/ga/properties/123:runReport"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dimensionHeaders": [{"name": "landingPage"}], "metricHeaders": [{"name": "sessions"}, {"name": "keyEvents"}],
                "rows": [{"dimensionValues": [{"value": "/a"}], "metricValues": [{"value": "42"}, {"value": "3"}]}]})))
            .mount(&s).await;

        let store = MemoryStore::default();
        assert!(!status(&store).client);
        save_client(
            &store,
            &OAuthClient {
                id: "1-abc.apps.googleusercontent.com".into(),
                secret: "sec".into(),
            },
        )
        .unwrap();
        // The "browser": follow the consent URL's redirect_uri with the code and the same state.
        let open = |u: &str| {
            let u = url::Url::parse(u).unwrap();
            let q: std::collections::HashMap<String, String> =
                u.query_pairs().into_owned().collect();
            assert_eq!(q["code_challenge_method"], "S256");
            assert_eq!(q["client_id"], "1-abc.apps.googleusercontent.com");
            let back = format!("{}/?state={}&code=THECODE", q["redirect_uri"], q["state"]);
            tokio::spawn(async move {
                let c = reqwest::Client::new();
                let _ = c
                    .get(format!("{}/favicon.ico", back.split("/?").next().unwrap()))
                    .send()
                    .await;
                let _ = c.get(back).send().await;
            });
        };
        login(&ep(&s), &store, &open, Duration::from_secs(10))
            .await
            .unwrap();
        assert!(status(&store).signed_in);

        let g = Google::connect(ep(&s), &store).await.unwrap();
        assert_eq!(g.sites().await.unwrap(), ["sc-domain:x.test"]);
        let rows = g
            .search_analytics(
                "sc-domain:x.test",
                "2026-10-01",
                "2026-10-28",
                &["page"],
                100,
            )
            .await
            .unwrap();
        assert_eq!(rows[0].keys[0], "https://x.test/a");
        let ga = g
            .ga_report(
                "123",
                "2026-10-01",
                "2026-10-28",
                &["landingPage"],
                &["sessions", "keyEvents"],
            )
            .await
            .unwrap();
        assert_eq!(ga[0]["sessions"], 42.0);

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("g.sqlite");
        store_rows(&db, "sc-domain:x.test", "a", "b", &["page"], &rows).unwrap();
        assert_eq!(
            load_rows(&db, "sc-domain:x.test", "a", "b", &["page"]).unwrap(),
            rows
        );
        store_ga(&db, "123", "a", "b", "x", &ga).unwrap();
        assert!(logout(&store).unwrap());
    }

    #[test]
    fn drop_prompt_and_routine() {
        let d = Drop {
            page: "/a".into(),
            clicks_before: 10.0,
            clicks_now: 2.0,
            impressions_before: 100.0,
            impressions_now: 90.0,
            position_before: 3.0,
            position_now: 9.0,
            ctr_before: 0.1,
            ctr_now: 0.02,
            hints: vec!["position".into()],
        };
        let p = drops_prompt(&[d], "tr");
        assert!(
            p.contains("/a clicks 10→2") && p.contains("hints: position") && p.contains("`tr`")
        );
        assert_eq!(
            drops_routine(Path::new("."), "sc-domain:x.test", 20.0, "daily").unwrap()["intervalMinutes"],
            1440
        );
        assert!(drops_routine(Path::new("."), "x; rm", 20.0, "daily").is_err());
    }

    #[test]
    fn drops_rank_and_hint() {
        let r = |p: &str, c: f64, i: f64, ctr: f64, pos: f64| Row {
            keys: vec![p.into()],
            clicks: c,
            impressions: i,
            ctr,
            position: pos,
        };
        let before = [
            r("/a", 100.0, 1000.0, 0.1, 3.0),
            r("/b", 50.0, 1000.0, 0.05, 5.0),
            r("/c", 10.0, 100.0, 0.1, 4.0),
            r("/d", 5.0, 50.0, 0.1, 2.0),
        ];
        let now = [
            r("/a", 20.0, 900.0, 0.02, 9.0),
            r("/b", 20.0, 1000.0, 0.02, 5.2),
            r("/c", 12.0, 100.0, 0.12, 4.0),
        ];
        let d = drops(&before, &now, None, 10);
        assert_eq!(
            d.iter().map(|x| x.page.as_str()).collect::<Vec<_>>(),
            ["/a", "/b", "/d"]
        );
        assert_eq!(d[0].hints, ["position"]);
        assert_eq!(d[1].hints, ["snippet"]);
        assert_eq!(d[2].hints, ["gone"]);
    }
}
