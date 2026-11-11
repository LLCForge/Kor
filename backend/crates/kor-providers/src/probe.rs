//! Provider checks shared by `kor provider test|status|discover` and the desktop
//! (`g9.testKey`, `g9.health`, `provider.*`): one implementation, two thin clients.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use kor_core::{Message, ProviderError, ProviderErrorKind, Role, StreamEvent};
use tokio_util::sync::CancellationToken;

use crate::{Category, ChatRequest, Provider, ProviderManifest, Registry, health};

pub const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DISCOVER_TIMEOUT: Duration = Duration::from_millis(1500);

fn ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn timeout_err(provider: &str) -> ProviderError {
    ProviderError::new(provider, ProviderErrorKind::Timeout, "timed out")
}

/// Model list reachability: the cheap check behind `status` and the desktop key test.
#[derive(Debug, Clone)]
pub struct Check {
    pub provider: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub models: usize,
    pub error: Option<ProviderError>,
}

pub async fn check(p: &dyn Provider) -> Check {
    let started = Instant::now();
    let r = tokio::time::timeout(CHECK_TIMEOUT, p.list_models())
        .await
        .unwrap_or_else(|_| Err(timeout_err(p.id())));
    let latency_ms = ms(started);
    match &r {
        Ok(_) => health::record(p.id(), Ok(latency_ms)),
        Err(e) => health::record(p.id(), Err(&e.message)),
    }
    Check {
        provider: p.id().to_owned(),
        ok: r.is_ok(),
        latency_ms,
        models: r.as_ref().map_or(0, Vec::len),
        error: r.err(),
    }
}

/// `kor provider test`: the model list, then one tiny streaming call.
#[derive(Debug, Clone)]
pub struct TestReport {
    pub check: Check,
    /// Model used for the streaming call (`None` when no model was known).
    pub model: Option<String>,
    pub stream: Option<StreamReport>,
}

#[derive(Debug, Clone)]
pub struct StreamReport {
    pub ok: bool,
    pub first_token_ms: Option<u64>,
    pub total_ms: u64,
    pub text: String,
    pub error: Option<ProviderError>,
}

pub async fn test(p: &dyn Provider, model: Option<&str>) -> TestReport {
    let started = Instant::now();
    let listed = tokio::time::timeout(CHECK_TIMEOUT, p.list_models())
        .await
        .unwrap_or_else(|_| Err(timeout_err(p.id())));
    let check = Check {
        provider: p.id().to_owned(),
        ok: listed.is_ok(),
        latency_ms: ms(started),
        models: listed.as_ref().map_or(0, Vec::len),
        error: listed.as_ref().err().cloned(),
    };
    let model = model
        .map(str::to_owned)
        .or_else(|| listed.ok()?.into_iter().next().map(|m| m.id));
    let stream = match &model {
        Some(m) => Some(stream_once(p, m).await),
        None => None,
    };
    TestReport {
        check,
        model,
        stream,
    }
}

async fn stream_once(p: &dyn Provider, model: &str) -> StreamReport {
    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::text(Role::User, "Reply with the single word: ok")],
        max_tokens: Some(16),
        ..ChatRequest::default()
    };
    let started = Instant::now();
    let mut first = None;
    let mut text = String::new();
    let run = async {
        let mut s = p.complete(req, CancellationToken::new()).await?;
        while let Some(ev) = s.next().await {
            match ev? {
                StreamEvent::TextDelta { text: t } | StreamEvent::ThinkingDelta { text: t } => {
                    first.get_or_insert_with(|| ms(started));
                    if text.len() < 200 {
                        text.push_str(&t);
                    }
                }
                StreamEvent::Finish { .. } => break,
                _ => {}
            }
        }
        Ok::<(), ProviderError>(())
    };
    let r = tokio::time::timeout(CHECK_TIMEOUT, run)
        .await
        .unwrap_or_else(|_| Err(timeout_err(p.id())));
    let total_ms = ms(started);
    match &r {
        Ok(()) => health::record(p.id(), Ok(first.unwrap_or(total_ms))),
        Err(e) => health::record(p.id(), Err(&e.message)),
    }
    StreamReport {
        ok: r.is_ok(),
        first_token_ms: first,
        total_ms,
        text,
        error: r.err(),
    }
}

/// A local server found by [`discover`].
#[derive(Debug, Clone)]
pub struct Found {
    pub provider: String,
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    /// The URL differs from the manifest default (alternate port).
    pub moved: bool,
}

/// Other default ports a server has used (llama.cpp moved from 8080 to 9931).
fn alternates(m: &ProviderManifest) -> Vec<String> {
    match m.id.as_str() {
        "local-llamacpp" => vec![m.base_url.replace(":9931", ":8080")],
        _ => Vec::new(),
    }
}

/// Probe every local manifest on its default port(s), concurrently. Loopback only, so it is
/// allowed in privacy mode.
pub async fn discover(reg: &Registry) -> Vec<Found> {
    let jobs = reg
        .iter()
        .filter(|m| m.category == Category::Local)
        .flat_map(|m| {
            std::iter::once(m.base_url.clone())
                .chain(alternates(m))
                .map(move |url| (m, url))
        })
        .map(|(m, url)| async move {
            let mut mm = m.clone();
            mm.base_url.clone_from(&url);
            let p: Arc<dyn Provider> = crate::build(&mm, None).ok()?;
            let models = tokio::time::timeout(DISCOVER_TIMEOUT, p.list_models())
                .await
                .ok()?
                .ok()?;
            Some(Found {
                provider: m.id.clone(),
                name: m.name.clone(),
                moved: url != m.base_url,
                base_url: url,
                models: models.into_iter().map(|x| x.id).collect(),
            })
        });
    futures::future::join_all(jobs)
        .await
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn compat(id: &str, base: &str) -> ProviderManifest {
        let text = format!(
            "id = \"{id}\"\nname = \"P\"\ncategory = \"local\"\nadapter = \"openai_compat\"\n\
             base_url = \"{base}\"\nauth = {{ kind = \"none\" }}\nmodels_endpoint = \"/models\"\n\
             [privacy]\ntier = \"local\"\nretention = \"x\"\n"
        );
        ProviderManifest::parse(&text, "t").unwrap()
    }

    #[tokio::test]
    async fn test_lists_models_then_streams() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"id": "tiny"}]})),
            )
            .mount(&s)
            .await;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .insert_header("x-ratelimit-remaining-requests", "99")
                    .set_body_string(sse),
            )
            .mount(&s)
            .await;
        let p = crate::build(&compat("probe-ok", &format!("{}/v1", s.uri())), None).unwrap();
        let r = test(p.as_ref(), None).await;
        assert!(r.check.ok && r.check.models == 1);
        assert_eq!(r.model.as_deref(), Some("tiny"));
        let st = r.stream.unwrap();
        assert!(st.ok, "{:?}", st.error);
        assert_eq!(st.text, "ok");
        assert!(st.first_token_ms.is_some());
        let h = health::get("probe-ok").unwrap();
        assert_eq!(h.quota.remaining_requests, Some(99));
        assert!(h.error_rate() < 1e-9);
    }

    #[tokio::test]
    async fn check_reports_auth_errors() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"bad key\"}"))
            .mount(&s)
            .await;
        let p = crate::build(&compat("probe-auth", &format!("{}/v1", s.uri())), None).unwrap();
        let c = check(p.as_ref()).await;
        assert!(!c.ok);
        assert_eq!(c.error.unwrap().kind, ProviderErrorKind::AuthFailed);
    }
}
