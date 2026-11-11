//! Polite HTTP for the SEO crawler: one client through Kor's network guard
//! (`kor_core::net::client_builder`), user agent `kor/<version>`, robots.txt obeyed (per-origin
//! cache, longest-match rules, `Crawl-delay`), a per-host minimum delay, manual redirects so
//! chains and loops are visible, and a body cap.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use url::Url;

/// Largest body read; the rest is dropped (pages over this are reported, not parsed whole).
pub const MAX_BODY: usize = 5 * 1024 * 1024;
/// Redirect hops followed before giving up.
pub const MAX_HOPS: usize = 10;
/// `Crawl-delay` values above this are capped (a hostile robots.txt cannot stall a run).
const MAX_CRAWL_DELAY: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(20);

/// `kor/<version>` of this crate; the CLI passes the product version instead.
pub fn default_user_agent() -> String {
    format!("kor/{}", env!("CARGO_PKG_VERSION"))
}

/// One HTTP response, headers lower-cased.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The same body as bytes (gzip sitemaps).
    pub bytes: Vec<u8>,
    /// Body was cut at [`MAX_BODY`].
    pub truncated: bool,
    pub elapsed_ms: u64,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn is_html(&self) -> bool {
        self.header("content-type")
            .is_none_or(|c| c.contains("html"))
    }
}

/// One redirect hop: `from` answered `status` with `Location: to`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hop {
    pub from: String,
    pub status: u16,
    pub to: String,
}

/// How a redirect chain ended.
#[derive(Debug, Clone)]
pub enum Chain {
    Done(Response),
    Loop,
    TooLong,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Group {
    agents: Vec<String>,
    rules: Vec<(bool, String)>,
    delay: Option<f64>,
}

/// A parsed robots.txt (RFC 9309 rules plus `Crawl-delay` and `Sitemap`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Robots {
    groups: Vec<Group>,
    pub sitemaps: Vec<String>,
}

impl Robots {
    pub fn parse(text: &str) -> Self {
        let mut r = Self::default();
        let mut cur: Option<Group> = None;
        let mut last_was_agent = false;
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "user-agent" => {
                    if !last_was_agent {
                        if let Some(g) = cur.take() {
                            r.groups.push(g);
                        }
                        cur = Some(Group::default());
                    }
                    if let Some(g) = cur.as_mut() {
                        g.agents.push(v.to_ascii_lowercase());
                    }
                    last_was_agent = true;
                }
                "allow" | "disallow" => {
                    last_was_agent = false;
                    if let Some(g) = cur.as_mut()
                        && !v.is_empty()
                    {
                        g.rules.push((k == "allow", v.to_owned()));
                    }
                }
                "crawl-delay" => {
                    last_was_agent = false;
                    if let Some(g) = cur.as_mut() {
                        g.delay = v.parse().ok().filter(|d: &f64| d.is_finite() && *d >= 0.0);
                    }
                }
                "sitemap" => r.sitemaps.push(v.to_owned()),
                _ => {}
            }
        }
        if let Some(g) = cur {
            r.groups.push(g);
        }
        r
    }

    /// The group for `agent` (product token, e.g. `kor`), else `*`.
    fn group(&self, agent: &str) -> Option<&Group> {
        let agent = agent.to_ascii_lowercase();
        let token = agent.split('/').next().unwrap_or(&agent);
        self.groups
            .iter()
            .find(|g| {
                g.agents
                    .iter()
                    .any(|a| a != "*" && token.contains(a.as_str()))
            })
            .or_else(|| {
                self.groups
                    .iter()
                    .find(|g| g.agents.iter().any(|a| a == "*"))
            })
    }

    /// Whether `path` (path + query) may be fetched by `agent`: the longest matching rule wins,
    /// `Allow` wins a tie, no rule = allowed.
    pub fn allowed(&self, agent: &str, path: &str) -> bool {
        let Some(g) = self.group(agent) else {
            return true;
        };
        let mut best: Option<(usize, bool)> = None;
        for (allow, pat) in &g.rules {
            if pattern_matches(pat, path) {
                let len = pat.len();
                if best.is_none_or(|(l, a)| len > l || (len == l && *allow && !a)) {
                    best = Some((len, *allow));
                }
            }
        }
        best.is_none_or(|(_, a)| a)
    }

    pub fn crawl_delay(&self, agent: &str) -> Option<Duration> {
        self.group(agent)
            .and_then(|g| g.delay)
            .map(|d| Duration::from_secs_f64(d).min(MAX_CRAWL_DELAY))
    }
}

/// robots.txt pattern: `*` any run, trailing `$` end anchor, else a prefix.
fn pattern_matches(pat: &str, path: &str) -> bool {
    let (pat, anchored) = match pat.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pat, false),
    };
    let parts: Vec<&str> = pat.split('*').collect();
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !path.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if let Some(at) = path[pos..].find(part) {
            pos += at + part.len();
        } else {
            return false;
        }
    }
    if !anchored {
        return true;
    }
    if parts.len() > 1 {
        // `*` before the anchor: the last literal must end the path.
        return path.ends_with(parts.last().copied().unwrap_or(""));
    }
    pos == path.len()
}

/// The crawler's HTTP side.
#[derive(Debug)]
pub struct Fetcher {
    client: reqwest::Client,
    pub user_agent: String,
    delay: Duration,
    obey_robots: bool,
    last: Mutex<HashMap<String, Instant>>,
    robots: tokio::sync::Mutex<HashMap<String, Robots>>,
}

fn origin(u: &Url) -> String {
    u.origin().ascii_serialization()
}

impl Fetcher {
    /// `delay` is the minimum gap between two requests to one host.
    pub fn new(user_agent: &str, delay: Duration, obey_robots: bool) -> Result<Self, String> {
        let client = kor_core::net::client_builder()
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            user_agent: user_agent.to_owned(),
            delay,
            obey_robots,
            last: Mutex::default(),
            robots: tokio::sync::Mutex::default(),
        })
    }

    async fn wait_turn(&self, host: &str, extra: Option<Duration>) {
        let gap = extra.map_or(self.delay, |d| d.max(self.delay));
        loop {
            let wait = {
                let mut last = self
                    .last
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let now = Instant::now();
                match last.get(host) {
                    Some(t) if now.duration_since(*t) < gap => {
                        gap.saturating_sub(now.duration_since(*t))
                    }
                    _ => {
                        last.insert(host.to_owned(), now);
                        return;
                    }
                }
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// robots.txt of `u`'s origin (cached; missing or unreadable = allow all).
    pub async fn robots(&self, u: &Url) -> Robots {
        let key = origin(u);
        if let Some(r) = self.robots.lock().await.get(&key) {
            return r.clone();
        }
        let r = match Url::parse(&format!("{key}/robots.txt")) {
            Ok(ru) => match self.raw(&ru, None).await {
                Ok(resp) if resp.status == 200 => Robots::parse(&resp.body),
                _ => Robots::default(),
            },
            Err(_) => Robots::default(),
        };
        self.robots.lock().await.insert(key, r.clone());
        r
    }

    /// Whether robots.txt lets Kor fetch `u` (always true when robots are not obeyed).
    pub async fn allowed(&self, u: &Url) -> bool {
        if !self.obey_robots {
            return true;
        }
        let path = match u.query() {
            Some(q) => format!("{}?{q}", u.path()),
            None => u.path().to_owned(),
        };
        self.robots(u).await.allowed(&self.user_agent, &path)
    }

    async fn raw(&self, u: &Url, delay: Option<Duration>) -> Result<Response, String> {
        if !matches!(u.scheme(), "http" | "https") {
            return Err(format!("unsupported scheme: {}", u.scheme()));
        }
        kor_core::net::check_url(u.as_str()).map_err(|e| e.to_string())?;
        self.wait_turn(u.host_str().unwrap_or(""), delay).await;
        let start = Instant::now();
        let mut resp = self
            .client
            .get(u.clone())
            .send()
            .await
            .map_err(|e| kor_core::net::describe(&e))?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            if body.len() + chunk.len() > MAX_BODY {
                body.extend_from_slice(&chunk[..MAX_BODY - body.len()]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response {
            url: u.to_string(),
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
            bytes: body,
            truncated,
            elapsed_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    /// One request (no redirect following), after robots' crawl delay and the host gap.
    pub async fn get(&self, u: &Url) -> Result<Response, String> {
        let delay = if self.obey_robots {
            self.robots(u).await.crawl_delay(&self.user_agent)
        } else {
            None
        };
        self.raw(u, delay).await
    }

    /// Follow redirects by hand, recording every hop.
    pub async fn follow(&self, start: &Url) -> Result<(Vec<Hop>, Chain), String> {
        let mut hops = Vec::new();
        let mut seen = vec![start.to_string()];
        let mut cur = start.clone();
        loop {
            let r = self.get(&cur).await?;
            let loc = r.header("location").map(str::to_owned);
            match (r.status, loc) {
                (301 | 302 | 303 | 307 | 308, Some(loc)) => {
                    let next = cur.join(&loc).map_err(|e| e.to_string())?;
                    hops.push(Hop {
                        from: cur.to_string(),
                        status: r.status,
                        to: next.to_string(),
                    });
                    if seen.contains(&next.to_string()) {
                        return Ok((hops, Chain::Loop));
                    }
                    if hops.len() >= MAX_HOPS {
                        return Ok((hops, Chain::TooLong));
                    }
                    seen.push(next.to_string());
                    cur = next;
                }
                _ => return Ok((hops, Chain::Done(r))),
            }
        }
    }

    /// POST/other requests for API clients share the guard but not the crawl rules.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_longest_match_and_groups() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /private\nAllow: /private/ok\nCrawl-delay: 2\n\n\
             User-agent: kor\nDisallow: /nokor$\nDisallow: /*.pdf$\n\nSitemap: https://x.test/sm.xml\n",
        );
        assert_eq!(r.sitemaps, ["https://x.test/sm.xml"]);
        // kor has its own group: the `*` rules do not apply.
        assert!(r.allowed("kor/0.8.0", "/private"));
        assert!(!r.allowed("kor/0.8.0", "/nokor"));
        assert!(r.allowed("kor/0.8.0", "/nokor/x"));
        assert!(!r.allowed("kor/0.8.0", "/files/a.pdf"));
        assert!(r.allowed("kor/0.8.0", "/files/a.pdf?x=1"));
        assert!(!r.allowed("other", "/private/x"));
        assert!(r.allowed("other", "/private/ok/1"));
        assert_eq!(r.crawl_delay("other"), Some(Duration::from_secs(2)));
        assert_eq!(r.crawl_delay("kor"), None);
    }

    #[test]
    fn shared_agent_lines_form_one_group() {
        let r = Robots::parse("User-agent: a\nUser-agent: kor\nDisallow: /\n");
        assert!(!r.allowed("kor", "/x"));
        assert!(Robots::parse("").allowed("kor", "/x"));
        let big = Robots::parse("User-agent: *\nCrawl-delay: 99999\n");
        assert_eq!(big.crawl_delay("kor"), Some(MAX_CRAWL_DELAY));
    }
}
