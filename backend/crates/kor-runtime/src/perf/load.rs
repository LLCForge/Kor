//! A small HTTP load generator for `kor perf <url>`: N GET requests with C at a time through
//! Kor's network guard (`kor_core::net::client_builder`). Only loopback hosts, or hosts the user
//! named with `--allow-host`, are accepted; privacy and offline modes still apply on top.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const MAX_REQUESTS: u32 = 1_000_000;
pub const MAX_CONCURRENCY: u32 = 1024;

/// Measured by the load generator.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadStats {
    pub requests: u32,
    /// 2xx/3xx responses.
    pub ok: u32,
    /// Other statuses and failed requests.
    pub errors: u32,
    pub wall_ms: f64,
    pub rps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    /// First failure, for the report.
    pub first_error: Option<String>,
}

/// `url` must be http(s) on a loopback host or one in `allow_hosts`, and pass the guard.
pub fn check_target(url: &str, allow_hosts: &[String]) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!("{url}: only http:// and https:// URLs can be load-tested"));
    }
    let host = kor_core::net::url_host(url).ok_or_else(|| format!("{url}: no host"))?;
    if !kor_core::net::is_loopback_host(&host) && !allow_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        return Err(format!(
            "{host} is not this computer; Kor only sends load to loopback hosts unless you pass --allow-host {host}"
        ));
    }
    kor_core::net::check_url(url).map_err(|b| b.message)
}

#[allow(clippy::cast_precision_loss)]
fn percentile(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)].as_secs_f64() * 1e3
}

/// Send `requests` GETs to `url`, `concurrency` at a time.
pub async fn run(url: &str, requests: u32, concurrency: u32, allow_hosts: &[String]) -> Result<LoadStats, String> {
    check_target(url, allow_hosts)?;
    let requests = requests.clamp(1, MAX_REQUESTS);
    let concurrency = concurrency.clamp(1, MAX_CONCURRENCY).min(requests);
    let client = kor_core::net::client_builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(usize::try_from(concurrency).unwrap_or(64))
        .build()
        .map_err(|e| e.to_string())?;
    let next = Arc::new(AtomicU32::new(0));
    let start = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..concurrency {
        let (client, next, url) = (client.clone(), next.clone(), url.to_owned());
        tasks.push(tokio::spawn(async move {
            let mut lat = Vec::new();
            let (mut ok, mut err, mut first) = (0u32, 0u32, None);
            while next.fetch_add(1, Ordering::Relaxed) < requests {
                let t = Instant::now();
                let r = client.get(&url).send().await;
                match r {
                    Ok(resp) => {
                        let status = resp.status();
                        let _ = resp.bytes().await;
                        if status.is_success() || status.is_redirection() {
                            ok += 1;
                        } else {
                            err += 1;
                            first.get_or_insert_with(|| format!("HTTP {status}"));
                        }
                    }
                    Err(e) => {
                        err += 1;
                        first.get_or_insert_with(|| kor_core::net::describe(&e));
                    }
                }
                lat.push(t.elapsed());
            }
            (lat, ok, err, first)
        }));
    }
    let mut all = Vec::with_capacity(usize::try_from(requests).unwrap_or(0));
    let mut stats = LoadStats::default();
    for t in tasks {
        let (lat, ok, err, first) = t.await.map_err(|e| e.to_string())?;
        all.extend(lat);
        stats.ok += ok;
        stats.errors += err;
        if stats.first_error.is_none() {
            stats.first_error = first;
        }
    }
    let wall = start.elapsed().as_secs_f64();
    all.sort_unstable();
    stats.requests = stats.ok + stats.errors;
    stats.wall_ms = wall * 1e3;
    stats.rps = if wall > 0.0 { f64::from(stats.requests) / wall } else { 0.0 };
    stats.p50_ms = percentile(&all, 0.50);
    stats.p95_ms = percentile(&all, 0.95);
    stats.max_ms = all.last().map_or(0.0, |d| d.as_secs_f64() * 1e3);
    Ok(stats)
}

/// Wait until `url` answers (any HTTP status) or `timeout` passes.
pub async fn wait_ready(url: &str, timeout: Duration) -> bool {
    let Ok(client) = kor_core::net::client_builder().timeout(Duration::from_secs(2)).build() else {
        return false;
    };
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if client.get(url).send().await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn only_loopback_or_named_hosts() {
        assert!(check_target("http://127.0.0.1:8080/x", &[]).is_ok());
        assert!(check_target("http://localhost/x", &[]).is_ok());
        assert!(check_target("http://[::1]:3000/", &[]).is_ok());
        let e = check_target("https://example.com/", &[]).unwrap_err();
        assert!(e.contains("--allow-host example.com"), "{e}");
        assert!(check_target("https://example.com/", &["Example.com".into()]).is_ok());
        assert!(check_target("ftp://127.0.0.1/", &[]).is_err());
    }

    #[tokio::test]
    async fn sends_n_requests_and_measures() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/hot")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).expect(40).mount(&server).await;
        Mock::given(method("GET")).and(path("/bad")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
        let url = format!("{}/hot", server.uri());
        assert!(wait_ready(&server.uri(), Duration::from_secs(5)).await);
        let s = run(&url, 40, 8, &[]).await.unwrap();
        assert_eq!((s.requests, s.ok, s.errors), (40, 40, 0));
        assert!(s.rps > 0.0 && s.p50_ms <= s.p95_ms && s.p95_ms <= s.max_ms, "{s:?}");
        let bad = run(&format!("{}/bad", server.uri()), 3, 2, &[]).await.unwrap();
        assert_eq!((bad.ok, bad.errors), (0, 3));
        assert_eq!(bad.first_error.as_deref(), Some("HTTP 503 Service Unavailable"));
        server.verify().await;
    }
}
