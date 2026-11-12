//! §4.2: provider APIs beyond chat. Cohere embeddings (`POST /v2/embed`) and rerank
//! (`POST /v2/rerank`), and Mistral Codestral fill-in-the-middle (`POST /fim/completions`).
//! Same HTTP client as chat: the network guard, no redirects, the key only to its own host.

use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::SecretString;
use serde_json::{Value, json};

use crate::ProviderManifest;
use crate::http::Http;

/// Cohere's current models for each task.
pub const COHERE_EMBED_MODEL: &str = "embed-v4.0";
pub const COHERE_RERANK_MODEL: &str = "rerank-v3.5";
pub const CODESTRAL_MODEL: &str = "codestral-latest";

/// What an embedding is for (Cohere embeds queries and documents differently).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    Document,
    Query,
}

#[derive(Debug)]
pub struct Cohere {
    http: Http,
}

impl Cohere {
    /// `manifest` is the `cohere` provider's.
    ///
    /// # Errors
    /// When the HTTP client cannot be built.
    pub fn new(
        manifest: &ProviderManifest,
        key: Option<SecretString>,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            http: Http::new(manifest, key)?,
        })
    }

    /// Vectors for `texts`, in order.
    ///
    /// # Errors
    /// A provider error, or an answer without one vector per text.
    pub async fn embed(
        &self,
        model: &str,
        texts: &[String],
        kind: EmbedKind,
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        let body = embed_body(model, texts, kind);
        let resp = self.http.post_plain_json("/v2/embed", &body).await?;
        let v = self.http.read_json(resp).await?;
        parse_embeddings(&v, texts.len()).ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "malformed embeddings answer")
        })
    }

    /// `(index into documents, relevance 0–1)`, most relevant first, at most `top`.
    ///
    /// # Errors
    /// A provider error or a malformed answer.
    pub async fn rerank(
        &self,
        model: &str,
        query: &str,
        documents: &[String],
        top: usize,
    ) -> Result<Vec<(usize, f64)>, ProviderError> {
        let body = json!({"model": model, "query": query, "documents": documents, "top_n": top.min(documents.len())});
        let resp = self.http.post_plain_json("/v2/rerank", &body).await?;
        let v = self.http.read_json(resp).await?;
        parse_rerank(&v, documents.len()).ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "malformed rerank answer")
        })
    }
}

fn embed_body(model: &str, texts: &[String], kind: EmbedKind) -> Value {
    let input_type = match kind {
        EmbedKind::Document => "search_document",
        EmbedKind::Query => "search_query",
    };
    json!({"model": model, "texts": texts, "input_type": input_type, "embedding_types": ["float"]})
}

#[allow(clippy::cast_possible_truncation)] // embeddings are f32 on every Kor path
fn parse_embeddings(v: &Value, n: usize) -> Option<Vec<Vec<f32>>> {
    let rows = v["embeddings"]["float"].as_array()?;
    let out: Vec<Vec<f32>> = rows
        .iter()
        .map(|r| {
            r.as_array().map(|a| {
                a.iter()
                    .filter_map(Value::as_f64)
                    .map(|x| x as f32)
                    .collect()
            })
        })
        .collect::<Option<_>>()?;
    (out.len() == n).then_some(out)
}

fn parse_rerank(v: &Value, docs: usize) -> Option<Vec<(usize, f64)>> {
    let mut out: Vec<(usize, f64)> = v["results"]
        .as_array()?
        .iter()
        .map(|r| {
            let i = usize::try_from(r["index"].as_u64()?)
                .ok()
                .filter(|i| *i < docs)?;
            Some((i, r["relevance_score"].as_f64()?))
        })
        .collect::<Option<_>>()?;
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    Some(out)
}

#[derive(Debug)]
pub struct Codestral {
    http: Http,
}

impl Codestral {
    /// `manifest` is the `codestral` provider's (base `https://codestral.mistral.ai/v1`).
    ///
    /// # Errors
    /// When the HTTP client cannot be built.
    pub fn new(
        manifest: &ProviderManifest,
        key: Option<SecretString>,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            http: Http::new(manifest, key)?,
        })
    }

    /// The code that goes between `prefix` and `suffix`.
    ///
    /// # Errors
    /// A provider error, or an answer without a completion.
    pub async fn fim(
        &self,
        model: &str,
        prefix: &str,
        suffix: &str,
        max_tokens: u32,
    ) -> Result<String, ProviderError> {
        let body = json!({
            "model": model,
            "prompt": prefix,
            "suffix": suffix,
            "max_tokens": max_tokens,
            "temperature": 0,
        });
        let resp = self.http.post_plain_json("/fim/completions", &body).await?;
        let v = self.http.read_json(resp).await?;
        parse_fim(&v).ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "malformed completion answer")
        })
    }
}

fn parse_fim(v: &Value) -> Option<String> {
    let c = &v["choices"][0];
    c["message"]["content"]
        .as_str()
        .or_else(|| c["text"].as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn manifest(id: &str, base: &str) -> ProviderManifest {
        let mut m = crate::Registry::builtin().get(id).expect("builtin").clone();
        m.base_url = base.to_owned();
        m
    }

    #[tokio::test]
    #[allow(clippy::many_single_char_names)] // short names of the math or the protocol
    async fn cohere_embeds_and_reranks_and_codestral_fills_the_middle() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/embed"))
            .and(body_partial_json(
                json!({"input_type": "search_query", "embedding_types": ["float"]}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"embeddings": {"float": [[0.5, 0.25]]}})),
            )
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/v2/rerank"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [
                {"index": 1, "relevance_score": 0.2}, {"index": 0, "relevance_score": 0.9}
            ]})))
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/fim/completions"))
            .and(body_partial_json(
                json!({"prompt": "fn add(a: i32, b: i32) -> i32 {", "suffix": "}"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"choices": [{"message": {"content": " a + b "}}]})),
            )
            .mount(&s)
            .await;
        let key = Some(SecretString::from("k".to_owned()));
        let c = Cohere::new(&manifest("cohere", &s.uri()), key.clone()).unwrap();
        let v = c
            .embed(COHERE_EMBED_MODEL, &["q".into()], EmbedKind::Query)
            .await
            .unwrap();
        assert_eq!(v, vec![vec![0.5, 0.25]]);
        let r = c
            .rerank(COHERE_RERANK_MODEL, "q", &["a".into(), "b".into()], 5)
            .await
            .unwrap();
        assert_eq!(r, vec![(0, 0.9), (1, 0.2)]);
        let f = Codestral::new(&manifest("codestral", &s.uri()), key).unwrap();
        assert_eq!(
            f.fim(CODESTRAL_MODEL, "fn add(a: i32, b: i32) -> i32 {", "}", 64)
                .await
                .unwrap(),
            " a + b "
        );
        // A wrong count or index is a protocol error, not silently wrong vectors.
        assert!(parse_embeddings(&json!({"embeddings": {"float": [[1.0]]}}), 2).is_none());
        assert!(
            parse_rerank(
                &json!({"results": [{"index": 9, "relevance_score": 1.0}]}),
                2
            )
            .is_none()
        );
    }
}
