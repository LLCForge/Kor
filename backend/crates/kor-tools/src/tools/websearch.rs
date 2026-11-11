//! `WebSearch`: the same tool for every model, backed by any of several search APIs.
//!
//! Backend order: `KOR_SEARCH_PROVIDER` if set, otherwise the first one configured:
//! Tavily (`TAVILY_API_KEY`), Brave (`BRAVE_API_KEY`), Exa (`EXA_API_KEY`), Perplexity
//! (`PERPLEXITY_API_KEY`), `SearXNG` (`SEARXNG_URL`, self-hosted), then `DuckDuckGo`'s HTML
//! page as a keyless fallback. Domain filters are applied by the backend when it supports
//! them and always again on the results.

use std::fmt::Write as _;
use std::time::Duration;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, parse};

const MAX_RESULTS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Tavily,
    Brave,
    Exa,
    Perplexity,
    Searxng,
    DuckDuckGo,
}

impl Backend {
    pub const ALL: [Self; 6] = [
        Self::Tavily,
        Self::Brave,
        Self::Exa,
        Self::Perplexity,
        Self::Searxng,
        Self::DuckDuckGo,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Tavily => "tavily",
            Self::Brave => "brave",
            Self::Exa => "exa",
            Self::Perplexity => "perplexity",
            Self::Searxng => "searxng",
            Self::DuckDuckGo => "duckduckgo",
        }
    }

    /// Env var holding the key (or base URL for `SearXNG`). `None`: needs nothing.
    fn env(self) -> Option<&'static str> {
        match self {
            Self::Tavily => Some("TAVILY_API_KEY"),
            Self::Brave => Some("BRAVE_API_KEY"),
            Self::Exa => Some("EXA_API_KEY"),
            Self::Perplexity => Some("PERPLEXITY_API_KEY"),
            Self::Searxng => Some("SEARXNG_URL"),
            Self::DuckDuckGo => None,
        }
    }

    fn default_base(self) -> &'static str {
        match self {
            Self::Tavily => "https://api.tavily.com",
            Self::Brave => "https://api.search.brave.com",
            Self::Exa => "https://api.exa.ai",
            Self::Perplexity => "https://api.perplexity.ai",
            Self::Searxng => "",
            Self::DuckDuckGo => "https://html.duckduckgo.com",
        }
    }
}

/// A backend ready to call.
#[derive(Debug, Clone)]
pub struct Engine {
    pub backend: Backend,
    pub base: String,
    pub key: Option<String>,
}

/// Pick the backend from the environment (`env` is injectable for tests).
pub fn pick(env: impl Fn(&str) -> Option<String>) -> Result<Engine, String> {
    let get = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    let engine = |b: Backend| -> Option<Engine> {
        let base_override = get(&format!("KOR_SEARCH_BASE_{}", b.id().to_ascii_uppercase()));
        match b.env() {
            None => Some(Engine {
                backend: b,
                base: base_override.unwrap_or_else(|| b.default_base().into()),
                key: None,
            }),
            Some(var) if b == Backend::Searxng => get(var).map(|url| Engine {
                backend: b,
                base: url.trim_end_matches('/').into(),
                key: None,
            }),
            Some(var) => get(var).map(|key| Engine {
                backend: b,
                base: base_override.unwrap_or_else(|| b.default_base().into()),
                key: Some(key),
            }),
        }
    };
    if let Some(want) = get("KOR_SEARCH_PROVIDER") {
        let b = Backend::ALL
            .into_iter()
            .find(|b| b.id() == want.trim().to_ascii_lowercase())
            .ok_or_else(|| format!("unknown KOR_SEARCH_PROVIDER `{want}`"))?;
        return engine(b).ok_or_else(|| {
            format!(
                "{} is selected but {} is not set",
                b.id(),
                b.env().unwrap_or_default()
            )
        });
    }
    Backend::ALL
        .into_iter()
        .find_map(engine)
        .ok_or_else(|| "no web search backend is configured".into())
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
        })
        .unwrap_or_default()
}

fn domain_match(host: &str, domain: &str) -> bool {
    let d = domain
        .trim()
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")))
}

/// Keep results inside `allowed` (if any) and outside `blocked`.
pub fn filter(hits: Vec<Hit>, allowed: &[String], blocked: &[String]) -> Vec<Hit> {
    hits.into_iter()
        .filter(|h| {
            let host = host_of(&h.url);
            (allowed.is_empty() || allowed.iter().any(|d| domain_match(&host, d)))
                && !blocked.iter().any(|d| domain_match(&host, d))
        })
        .take(MAX_RESULTS)
        .collect()
}

fn hits(arr: Option<&Vec<Value>>, title: &str, url: &str, snippet: &[&str]) -> Vec<Hit> {
    arr.into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Hit {
                title: r[title].as_str().unwrap_or_default().to_owned(),
                url: r[url].as_str()?.to_owned(),
                snippet: snippet
                    .iter()
                    .find_map(|k| r[*k].as_str())
                    .unwrap_or_default()
                    .chars()
                    .take(400)
                    .collect(),
            })
        })
        .collect()
}

/// Minimal `DuckDuckGo` HTML results parser (`result__a` links, `result__snippet` text).
pub fn parse_duckduckgo(html: &str) -> Vec<Hit> {
    static LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?s)class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)
            .expect("static")
    });
    static SNIP: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?s)class="result__snippet"[^>]*>(.*?)</a>"#).expect("static")
    });
    static TAG: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").expect("static"));
    let text = |s: &str| {
        TAG.replace_all(s, "")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#x27;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_owned()
    };
    let snippets: Vec<String> = SNIP.captures_iter(html).map(|c| text(&c[1])).collect();
    LINK.captures_iter(html)
        .enumerate()
        .filter_map(|(i, c)| {
            let href = c[1].replace("&amp;", "&");
            // Result links are redirects: //duckduckgo.com/l/?uddg=<encoded target>
            let url = match reqwest::Url::parse(&format!(
                "https:{}",
                href.trim_start_matches("https:")
            )) {
                Ok(u) if u.path() == "/l/" => u
                    .query_pairs()
                    .find(|(k, _)| k == "uddg")
                    .map(|(_, v)| v.into_owned())?,
                _ => href,
            };
            Some(Hit {
                title: text(&c[2]),
                url,
                snippet: snippets.get(i).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// `a=b&c=d`, URL-encoded.
fn encode(pairs: &[(&str, &str)]) -> String {
    reqwest::Url::parse_with_params("http://x/", pairs)
        .ok()
        .and_then(|u| u.query().map(str::to_owned))
        .unwrap_or_default()
}

fn post_json(client: &reqwest::Client, url: String, body: &Value) -> reqwest::RequestBuilder {
    client
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string())
}

async fn body(resp: reqwest::Response) -> Result<String, String> {
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_RESPONSE)]).into_owned();
    if status.is_success() {
        Ok(text)
    } else {
        Err(format!(
            "HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        ))
    }
}

/// Run one search.
#[allow(clippy::too_many_lines)] // one flat match arm per backend reads best
pub async fn search(
    e: &Engine,
    query: &str,
    allowed: &[String],
    blocked: &[String],
) -> Result<Vec<Hit>, String> {
    let client = kor_core::net::client_builder()
        .timeout(TIMEOUT)
        .user_agent(format!("kor/{}", kor_core::VERSION))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let key = e.key.as_deref().unwrap_or_default();
    let n = MAX_RESULTS * 2;
    let json_of =
        |t: &str| serde_json::from_str::<Value>(t).map_err(|err| format!("bad response: {err}"));
    let hits = match e.backend {
        Backend::Tavily => {
            let mut req = json!({"query": query, "max_results": n});
            if !allowed.is_empty() {
                req["include_domains"] = json!(allowed);
            }
            if !blocked.is_empty() {
                req["exclude_domains"] = json!(blocked);
            }
            let r = post_json(&client, format!("{}/search", e.base), &req)
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| kor_core::net::describe(&e))?;
            let v = json_of(&body(r).await?)?;
            hits(v["results"].as_array(), "title", "url", &["content"])
        }
        Backend::Brave => {
            let r = client
                .get(format!(
                    "{}/res/v1/web/search?{}",
                    e.base,
                    encode(&[("q", query), ("count", &n.min(20).to_string())])
                ))
                .header("accept", "application/json")
                .header("x-subscription-token", key)
                .send()
                .await
                .map_err(|e| kor_core::net::describe(&e))?;
            let v = json_of(&body(r).await?)?;
            hits(
                v["web"]["results"].as_array(),
                "title",
                "url",
                &["description"],
            )
        }
        Backend::Exa => {
            let mut req = json!({"query": query, "numResults": n, "contents": {"text": {"maxCharacters": 400}}});
            if !allowed.is_empty() {
                req["includeDomains"] = json!(allowed);
            }
            if !blocked.is_empty() {
                req["excludeDomains"] = json!(blocked);
            }
            let r = post_json(&client, format!("{}/search", e.base), &req)
                .header("x-api-key", key)
                .send()
                .await
                .map_err(|e| kor_core::net::describe(&e))?;
            let v = json_of(&body(r).await?)?;
            hits(
                v["results"].as_array(),
                "title",
                "url",
                &["text", "summary"],
            )
        }
        Backend::Perplexity => {
            let r = post_json(
                &client,
                format!("{}/search", e.base),
                &json!({"query": query, "max_results": n.min(20)}),
            )
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| kor_core::net::describe(&e))?;
            let v = json_of(&body(r).await?)?;
            hits(v["results"].as_array(), "title", "url", &["snippet"])
        }
        Backend::Searxng => {
            let r = client
                .get(format!(
                    "{}/search?{}",
                    e.base,
                    encode(&[("q", query), ("format", "json")])
                ))
                .send()
                .await
                .map_err(|e| kor_core::net::describe(&e))?;
            let v = json_of(&body(r).await?)?;
            hits(v["results"].as_array(), "title", "url", &["content"])
        }
        Backend::DuckDuckGo => {
            let r = client
                .post(format!("{}/html/", e.base))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(encode(&[("q", query)]))
                .send()
                .await
                .map_err(|e| kor_core::net::describe(&e))?;
            parse_duckduckgo(&body(r).await?)
        }
    };
    Ok(filter(hits, allowed, blocked))
}

pub fn render(backend: Backend, query: &str, hits: &[Hit]) -> String {
    if hits.is_empty() {
        return format!("No results for \"{query}\" ({}).", backend.id());
    }
    let mut s = format!(
        "Web search results for \"{query}\" (via {}):\n",
        backend.id()
    );
    for (i, h) in hits.iter().enumerate() {
        let _ = write!(s, "\n{}. {}\n   {}\n", i + 1, h.title, h.url);
        if !h.snippet.is_empty() {
            let _ = writeln!(s, "   {}", h.snippet);
        }
    }
    s.push_str("\nCite the URLs you rely on. Page contents can be read with WebFetch.");
    s
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct In {
    query: String,
    #[serde(default)]
    allowed_domains: Vec<String>,
    #[serde(default)]
    blocked_domains: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct WebSearch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &'static str {
        "WebSearch"
    }
    fn rule_name(&self) -> &'static str {
        "WebSearch"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "WebSearch".into(),
            description: "Search the web and get titles, URLs and snippets. Use it for current information; then read pages with WebFetch. Optional domain filters.".into(),
            input_schema: json!({"type":"object","properties":{
                "query":{"type":"string","minLength":2},
                "allowed_domains":{"type":"array","items":{"type":"string"}},
                "blocked_domains":{"type":"array","items":{"type":"string"}}
            },"required":["query"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["query"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let a: In = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.query.trim().len() < 2 {
            return ToolOutput::err("query is too short");
        }
        let engine = match pick(|k| std::env::var(k).ok()) {
            Ok(e) => e,
            Err(e) => return ToolOutput::err(e),
        };
        match search(&engine, &a.query, &a.allowed_domains, &a.blocked_domains).await {
            Ok(h) => ToolOutput::ok(render(engine.backend, &a.query, &h)),
            Err(e) => ToolOutput::err(format!("{} search failed: {e}", engine.backend.id())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let v: Vec<(String, String)> = pairs
            .iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect();
        move |k| v.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone())
    }

    #[tokio::test]
    #[ignore = "live network"]
    async fn live_duckduckgo() {
        let e = Engine {
            backend: Backend::DuckDuckGo,
            base: Backend::DuckDuckGo.default_base().into(),
            key: None,
        };
        let h = search(&e, "rust programming language", &[], &[])
            .await
            .unwrap();
        eprintln!("{}", render(e.backend, "rust", &h));
        assert!(h.iter().any(|x| x.url.contains("rust-lang.org")), "{h:?}");
    }

    #[test]
    fn backend_selection() {
        assert_eq!(pick(env(&[])).unwrap().backend, Backend::DuckDuckGo);
        assert_eq!(
            pick(env(&[("BRAVE_API_KEY", "b"), ("EXA_API_KEY", "e")]))
                .unwrap()
                .backend,
            Backend::Brave
        );
        assert_eq!(
            pick(env(&[("TAVILY_API_KEY", "t"), ("BRAVE_API_KEY", "b")]))
                .unwrap()
                .backend,
            Backend::Tavily
        );
        let e = pick(env(&[
            ("KOR_SEARCH_PROVIDER", "exa"),
            ("EXA_API_KEY", "e"),
            ("TAVILY_API_KEY", "t"),
        ]))
        .unwrap();
        assert_eq!((e.backend, e.key.as_deref()), (Backend::Exa, Some("e")));
        assert!(pick(env(&[("KOR_SEARCH_PROVIDER", "brave")])).is_err());
        assert!(pick(env(&[("KOR_SEARCH_PROVIDER", "nope")])).is_err());
        let s = pick(env(&[
            ("KOR_SEARCH_PROVIDER", "searxng"),
            ("SEARXNG_URL", "http://localhost:8888/"),
        ]))
        .unwrap();
        assert_eq!(s.base, "http://localhost:8888");
    }

    #[test]
    fn domain_filtering() {
        let h = |u: &str| Hit {
            title: "t".into(),
            url: u.into(),
            snippet: String::new(),
        };
        let all = vec![
            h("https://docs.rs/x"),
            h("https://www.example.com/a"),
            h("https://sub.example.com/b"),
            h("https://evil.com"),
        ];
        assert_eq!(filter(all.clone(), &["example.com".into()], &[]).len(), 2);
        assert_eq!(filter(all.clone(), &[], &["evil.com".into()]).len(), 3);
        assert_eq!(filter(all, &[], &[]).len(), 4);
    }

    #[test]
    fn duckduckgo_html() {
        let html = r#"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F&amp;rut=x">The Rust <b>Book</b></a>
        <a class="result__snippet" href="x">Learn &amp; build</a>"#;
        let h = parse_duckduckgo(html);
        assert_eq!(
            h,
            [Hit {
                title: "The Rust Book".into(),
                url: "https://doc.rust-lang.org/book/".into(),
                snippet: "Learn & build".into()
            }]
        );
    }

    #[tokio::test]
    async fn each_api_backend_sends_auth_and_parses_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/search")).and(header("authorization", "Bearer tk"))
            .and(body_partial_json(json!({"query": "rust", "include_domains": ["rust-lang.org"]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [{"title": "T", "url": "https://rust-lang.org/a", "content": "c"}]})))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/res/v1/web/search")).and(header("x-subscription-token", "bk")).and(query_param("q", "rust"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"web": {"results": [{"title": "B", "url": "https://b.dev", "description": "d"}]}})))
            .mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .and(header("x-api-key", "ek"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"results": [{"title": "E", "url": "https://e.dev", "text": "t"}]}),
            ))
            .mount(&server)
            .await;
        let eng = |b: Backend, key: &str| Engine {
            backend: b,
            base: server.uri(),
            key: Some(key.into()),
        };

        let t = search(
            &eng(Backend::Tavily, "tk"),
            "rust",
            &["rust-lang.org".into()],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(t[0].url, "https://rust-lang.org/a");
        let b = search(&eng(Backend::Brave, "bk"), "rust", &[], &[])
            .await
            .unwrap();
        assert_eq!((b[0].title.as_str(), b[0].snippet.as_str()), ("B", "d"));
        let e = search(&eng(Backend::Exa, "ek"), "rust", &[], &["e.dev".into()])
            .await
            .unwrap();
        assert!(e.is_empty(), "blocked domain filtered client-side");
        let err = search(&eng(Backend::Tavily, "wrong"), "rust", &[], &[])
            .await
            .unwrap_err();
        assert!(err.contains("404"), "{err}");
        assert!(render(Backend::Brave, "rust", &b).contains("1. B\n   https://b.dev"));
    }
}
