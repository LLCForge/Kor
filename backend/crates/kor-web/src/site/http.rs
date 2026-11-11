//! One HTTP client for the site module, built on Kor's network guard (`kor_core::net::client_builder`):
//! privacy and offline modes refuse hosts before any lookup. Redirects are never followed
//! automatically (redirect maps must see every hop), bodies are capped, and error messages never
//! carry request headers (where the credentials are).

use std::time::{Duration, Instant};

use super::Result;

/// Largest body read; the rest is dropped.
pub const MAX_BODY: usize = 5 * 1024 * 1024;
/// Redirect hops followed by [`Http::follow`].
pub const MAX_HOPS: usize = 10;

#[derive(Debug, Clone, Default)]
pub struct Resp {
    pub url: String,
    pub status: u16,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub ms: u64,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.body).map_err(|e| {
            format!(
                "{} answered HTTP {} with something that is not JSON ({e})",
                host_of(&self.url),
                self.status
            )
        })
    }
}

/// One hop of a redirect chain.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hop {
    pub url: String,
    pub status: u16,
}

#[derive(Debug, Clone)]
pub struct Http {
    client: reqwest::Client,
}

pub(crate) fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "server".into())
}

/// `a=b&c=d` with percent-encoding.
pub fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

impl Http {
    pub fn new() -> Result<Self> {
        Self::with_timeout(Duration::from_secs(30))
    }

    pub fn with_timeout(timeout: Duration) -> Result<Self> {
        let client = kor_core::net::client_builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("kor/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }

    /// Send one request. `headers` may hold credentials; they never appear in errors.
    pub async fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        headers: &[(&str, String)],
        body: Option<(&str, Vec<u8>)>,
    ) -> Result<Resp> {
        let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL {url}: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("only http and https URLs: {url}"));
        }
        kor_core::net::check_url(url).map_err(|e| e.message)?;
        let mut req = self.client.request(method, parsed);
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        if let Some((ct, b)) = body {
            req = req.header("content-type", ct).body(b);
        }
        let start = Instant::now();
        let mut r = req
            .send()
            .await
            .map_err(|e| format!("{}: {}", host_of(url), kor_core::net::describe(&e)))?;
        let status = r.status().as_u16();
        let headers = r
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_ascii_lowercase(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut buf = Vec::new();
        while let Some(chunk) = r
            .chunk()
            .await
            .map_err(|e| format!("{}: {}", host_of(url), kor_core::net::describe(&e)))?
        {
            let room = MAX_BODY.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if buf.len() >= MAX_BODY {
                break;
            }
        }
        Ok(Resp {
            url: url.to_owned(),
            status,
            headers,
            body: String::from_utf8_lossy(&buf).into_owned(),
            ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    pub async fn get(&self, url: &str) -> Result<Resp> {
        self.send(reqwest::Method::GET, url, &[], None).await
    }

    pub async fn get_with(&self, url: &str, headers: &[(&str, String)]) -> Result<Resp> {
        self.send(reqwest::Method::GET, url, headers, None).await
    }

    pub async fn post_form(
        &self,
        url: &str,
        headers: &[(&str, String)],
        pairs: &[(&str, &str)],
    ) -> Result<Resp> {
        self.send(
            reqwest::Method::POST,
            url,
            headers,
            Some((
                "application/x-www-form-urlencoded",
                form(pairs).into_bytes(),
            )),
        )
        .await
    }

    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: &serde_json::Value,
    ) -> Result<Resp> {
        self.send(
            reqwest::Method::POST,
            url,
            headers,
            Some(("application/json", body.to_string().into_bytes())),
        )
        .await
    }

    /// GET and follow redirects by hand (at most [`MAX_HOPS`]); returns the hops before the final
    /// answer. A loop or too many hops ends with an error-free result whose last hop repeats.
    pub async fn follow(&self, url: &str) -> Result<(Vec<Hop>, Resp)> {
        let mut hops = Vec::new();
        let mut cur = url.to_owned();
        loop {
            let r = self.get(&cur).await?;
            if !(300..400).contains(&r.status) {
                return Ok((hops, r));
            }
            let Some(loc) = r.header("location") else {
                return Ok((hops, r));
            };
            let next = url::Url::parse(&cur)
                .and_then(|b| b.join(loc))
                .map_err(|e| format!("bad redirect from {cur}: {e}"))?
                .to_string();
            hops.push(Hop {
                url: cur.clone(),
                status: r.status,
            });
            if hops.len() >= MAX_HOPS || hops.iter().any(|h| h.url == next) {
                return Ok((hops, Resp { url: next, ..r }));
            }
            cur = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn follows_redirects_by_hand() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "/b"))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/b"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&s)
            .await;
        let h = Http::new().unwrap();
        let (hops, r) = h.follow(&format!("{}/a", s.uri())).await.unwrap();
        assert_eq!(hops.len(), 1);
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "ok");
        assert!(h.get("ftp://x/").await.is_err());
    }

    #[test]
    fn form_encoding() {
        assert_eq!(form(&[("a", "b c"), ("d", "é&")]), "a=b+c&d=%C3%A9%26");
    }
}
