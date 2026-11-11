//! Minimal WordPress REST client for `kor seo content publish` and `kor seo meta apply`
//! (spec §29.7, §29.8): application-password Basic auth, https only (loopback excepted), every
//! call through Kor's network guard. Nothing here runs without the caller's explicit approval.

use base64::Engine as _;
use serde_json::{Value, json};
use url::Url;

/// SEO plugin meta keys written next to the core fields. Plugins must expose them in REST
/// (`register_post_meta(..., ['show_in_rest' => true])`); WordPress silently drops unknown meta,
/// so every write is read back and reported when it did not stick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaKeys {
    Yoast,
    RankMath,
    /// Core only: the excerpt stands in for the description.
    None,
}

impl MetaKeys {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "yoast" => Some(Self::Yoast),
            "rankmath" | "rank-math" => Some(Self::RankMath),
            "none" | "core" => Some(Self::None),
            _ => None,
        }
    }

    fn keys(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Yoast => Some(("_yoast_wpseo_title", "_yoast_wpseo_metadesc")),
            Self::RankMath => Some(("rank_math_title", "rank_math_description")),
            Self::None => None,
        }
    }

    /// REST body fields for a title + description.
    pub fn fields(self, title: &str, description: &str) -> Value {
        match self.keys() {
            Some((t, d)) => json!({ "meta": { t: title, d: description } }),
            None => json!({ "excerpt": description }),
        }
    }

    /// Whether a REST response shows the values applied.
    pub fn applied(self, resp: &Value, title: &str, description: &str) -> bool {
        match self.keys() {
            Some((t, d)) => resp["meta"][t] == title && resp["meta"][d] == description,
            None => resp["excerpt"]["raw"]
                .as_str()
                .or_else(|| resp["excerpt"]["rendered"].as_str())
                .is_some_and(|e| e.contains(description)),
        }
    }

    /// Current values in a REST object (`context=edit`).
    pub fn current(self, obj: &Value) -> (String, String) {
        let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
        match self.keys() {
            Some((t, d)) => (s(&obj["meta"][t]), s(&obj["meta"][d])),
            None => (String::new(), s(&obj["excerpt"]["raw"])),
        }
    }
}

/// A WordPress site.
#[derive(Debug, Clone)]
pub struct Wp {
    base: Url,
    auth: String,
    client: reqwest::Client,
}

/// A post or page found by its public URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `posts`, `pages` or `product` (WooCommerce).
    pub kind: String,
    pub id: u64,
    pub object: Value,
}

impl Wp {
    /// `base` = site URL (`https://example.com`), `user` + application password.
    pub fn new(base: &str, user: &str, app_password: &str) -> Result<Self, String> {
        let mut base = Url::parse(base.trim()).map_err(|e| format!("{base}: {e}"))?;
        let host = base.host_str().unwrap_or("").to_owned();
        if base.scheme() != "https"
            && !(base.scheme() == "http" && kor_core::net::is_loopback_host(&host))
        {
            // Application passwords travel in every request: never over plain http.
            return Err("WordPress URL must use https".into());
        }
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        let auth =
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{app_password}"));
        let client = kor_core::net::client_builder()
            .user_agent(super::fetch::default_user_agent())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base,
            auth: format!("Basic {auth}"),
            client,
        })
    }

    fn url(&self, path: &str) -> Result<Url, String> {
        self.base
            .join(&format!("wp-json/wp/v2/{path}"))
            .map_err(|e| e.to_string())
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, String> {
        let r = req
            .header("Authorization", &self.auth)
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
            // WordPress errors are `{code, message}`; nothing secret in them.
            Err(format!(
                "WordPress HTTP {}: {}",
                status.as_u16(),
                v["message"].as_str().unwrap_or("")
            ))
        }
    }

    /// Create a post (`status` = `draft` or `publish`).
    pub async fn create_post(
        &self,
        title: &str,
        html: &str,
        excerpt: &str,
        status: &str,
        extra: Option<Value>,
    ) -> Result<Value, String> {
        if !matches!(status, "draft" | "publish" | "pending") {
            return Err(format!("bad status {status}"));
        }
        let mut body =
            json!({ "title": title, "content": html, "excerpt": excerpt, "status": status });
        if let (Some(Value::Object(e)), Some(b)) = (extra, body.as_object_mut()) {
            b.extend(e);
        }
        self.send(
            self.client
                .post(self.url("posts")?)
                .header("content-type", "application/json")
                .body(body.to_string()),
        )
        .await
    }

    pub async fn update(&self, kind: &str, id: u64, fields: &Value) -> Result<Value, String> {
        if !matches!(kind, "posts" | "pages" | "media" | "product") {
            return Err(format!("bad kind {kind}"));
        }
        self.send(
            self.client
                .post(self.url(&format!("{kind}/{id}"))?)
                .header("content-type", "application/json")
                .body(fields.to_string()),
        )
        .await
    }

    /// Find the post or page whose `link` is `page_url` (by its last path segment as slug).
    pub async fn find_by_url(&self, page_url: &str) -> Result<Option<Item>, String> {
        let u = Url::parse(page_url).map_err(|e| e.to_string())?;
        let slug = u
            .path_segments()
            .and_then(|mut s| s.rfind(|x| !x.is_empty()))
            .unwrap_or("")
            .to_owned();
        if slug.is_empty() {
            // The front page has no slug; its meta lives in the SEO plugin's settings.
            return Ok(None);
        }
        let norm = |s: &str| s.trim_end_matches('/').to_owned();
        // WooCommerce registers products for REST as `/wp/v2/product`.
        for kind in ["posts", "pages", "product"] {
            let mut q = self.url(kind)?;
            q.query_pairs_mut()
                .append_pair("slug", &slug)
                .append_pair("context", "edit");
            let list = match self.send(self.client.get(q)).await {
                Ok(l) => l,
                // No WooCommerce: the route does not exist.
                Err(_) if kind == "product" => continue,
                Err(e) => return Err(e),
            };
            if let Some(obj) = list.as_array().into_iter().flatten().find(|o| {
                o["link"]
                    .as_str()
                    .is_some_and(|l| norm(l) == norm(page_url))
            }) {
                return Ok(Some(Item {
                    kind: kind.into(),
                    id: obj["id"].as_u64().unwrap_or(0),
                    object: obj.clone(),
                }));
            }
        }
        Ok(None)
    }

    /// Media item whose `source_url` is `src` (searched by file name).
    pub async fn find_media(&self, src: &str) -> Result<Option<Item>, String> {
        let name = src.rsplit('/').next().unwrap_or(src);
        let stem = name.split('.').next().unwrap_or(name);
        let mut q = self.url("media")?;
        q.query_pairs_mut()
            .append_pair("search", stem)
            .append_pair("context", "edit")
            .append_pair("per_page", "20");
        let list = self.send(self.client.get(q)).await?;
        Ok(list
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| {
                o["source_url"]
                    .as_str()
                    .is_some_and(|s| s == src || s.ends_with(&format!("/{name}")))
            })
            .map(|o| Item {
                kind: "media".into(),
                id: o["id"].as_u64().unwrap_or(0),
                object: o.clone(),
            }))
    }
}

/// A site connected with `kor wp connect <name> --user … --secret …` (K5's profiles, ADR 0054):
/// URL and user from the profile, the application password from the vault entry it names.
pub fn from_profile(name: &str, store: &dyn kor_secrets::SecretStore) -> Result<Wp, String> {
    use kor_secrets::ExposeSecret as _;
    let site = crate::site::wp::load(&crate::site::base()?, name)?;
    let (Some(user), Some(secret)) = (site.user.as_deref(), site.secret.as_deref()) else {
        return Err(format!(
            "WordPress site `{name}` has no REST user/secret (kor wp connect {name} --user … --secret …)"
        ));
    };
    let pass = kor_secrets::vault::get(store, secret)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("vault entry `{secret}` is missing (kor vault add {secret})"))?;
    Wp::new(&site.url, user, pass.expose_secret())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn refuses_plain_http() {
        assert!(Wp::new("http://example.com", "u", "p").is_err());
        assert!(Wp::new("http://127.0.0.1:8080", "u", "p").is_ok());
    }

    #[tokio::test]
    async fn create_find_update() {
        let s = MockServer::start().await;
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("ed:pw")
        );
        Mock::given(method("POST"))
            .and(path("/wp-json/wp/v2/posts"))
            .and(header("authorization", auth.as_str()))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"id": 7, "status": "draft", "link": "x"})),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET")).and(path("/wp-json/wp/v2/posts")).and(query_param("slug", "hello"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": 3, "link": format!("{}/hello/", s.uri()), "meta": {"_yoast_wpseo_title": "Old"}}])))
            .mount(&s).await;
        Mock::given(method("POST"))
            .and(path("/wp-json/wp/v2/posts/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"id": 3, "meta": {"_yoast_wpseo_title": "T", "_yoast_wpseo_metadesc": "D"}}),
            ))
            .mount(&s)
            .await;
        let wp = Wp::new(&s.uri(), "ed", "pw").unwrap();
        let r = wp
            .create_post("T", "<p>x</p>", "e", "draft", None)
            .await
            .unwrap();
        assert_eq!(r["id"], 7);
        let it = wp
            .find_by_url(&format!("{}/hello", s.uri()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((it.kind.as_str(), it.id), ("posts", 3));
        assert_eq!(MetaKeys::Yoast.current(&it.object).0, "Old");
        let resp = wp
            .update("posts", 3, &MetaKeys::Yoast.fields("T", "D"))
            .await
            .unwrap();
        assert!(MetaKeys::Yoast.applied(&resp, "T", "D"));
        assert!(!MetaKeys::RankMath.applied(&resp, "T", "D"));
    }
}
