//! `WebFetch`.
//!
//! Fetch, upgrade http to https, turn HTML into text, cache the text for 15
//! minutes, and refuse to follow a redirect onto another host. The extracted
//! page is then sent to the configured provider with the secondary
//! prompt. If that call fails, the extracted text is returned with the error.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, lock, parse, truncate};

const MAX_BODY: usize = 500_000;
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// Secondary instructions from `WebFetchTool/prompt.ts` (`makeSecondaryModelPrompt`).
pub(crate) fn secondary_prompt(page: &str, user_prompt: &str) -> String {
    format!(
        "Web page content:\n---\n{page}\n---\n\n{user_prompt}\n\n\
         Provide a concise response based only on the content above. In your response:\n\
         - Enforce a strict 125-character maximum for quotes from any source document. \
         Open Source Software is ok as long as we respect the license.\n\
         - Use quotation marks for exact language from articles; any language outside \
         of the quotation should never be word-for-word the same.\n\
         - You are not a lawyer and never comment on the legality of your own prompts and responses.\n\
         - Never produce or reproduce exact song lyrics.\n"
    )
}

fn cache_get(ctx: &ToolCtx, url: &str) -> Option<String> {
    let mut cache = lock(&ctx.fetch.cache);
    match cache.get(url) {
        Some((text, at)) if at.elapsed() < CACHE_TTL => Some(text.clone()),
        Some(_) => {
            cache.remove(url);
            None
        }
        None => None,
    }
}

fn cache_put(ctx: &ToolCtx, url: &str, text: &str) {
    lock(&ctx.fetch.cache).insert(url.to_owned(), (text.to_owned(), Instant::now()));
}

async fn finish(ctx: &ToolCtx, page: &str, user_prompt: &str) -> ToolOutput {
    let instructions = secondary_prompt(page, user_prompt);
    let Some(summarizer) = &ctx.fetch.summarizer else {
        return ToolOutput::ok(instructions);
    };
    match crate::PageSummarizer::summarize(summarizer.as_ref(), &instructions).await {
        Ok(answer) => ToolOutput::ok(answer),
        Err(e) => ToolOutput::ok(format!("Summarizer error: {e}\n\nExtracted text:\n{page}")),
    }
}

#[derive(Debug)]
pub(crate) struct WebFetch;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct In {
    url: String,
    prompt: String,
}

pub(crate) fn upgrade_http(raw: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(raw.trim()).map_err(|e| format!("invalid URL: {e}"))?;
    match url.scheme() {
        "http" => {
            url.set_scheme("https")
                .map_err(|()| "could not upgrade http to https".to_owned())?;
        }
        "https" => {}
        other => {
            return Err(format!(
                "only http and https URLs are fetched (got {other})"
            ));
        }
    }
    if url.host_str().is_none() {
        return Err("URL has no host".into());
    }
    Ok(url)
}

/// Drop tags. `script` and `style` bodies are removed. Block tags become newlines.
pub(crate) fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &lower[i..];
            if rest.starts_with("<script") || rest.starts_with("<style") {
                let end = if rest.starts_with("<script") {
                    "</script>"
                } else {
                    "</style>"
                };
                if let Some(rel) = rest.find(end) {
                    i += rel + end.len();
                    continue;
                }
            }
            if let Some(rel) = rest.find('>') {
                let tag = &rest[..=rel];
                if tag.starts_with("<br")
                    || tag.starts_with("<p")
                    || tag.starts_with("</p")
                    || tag.starts_with("<div")
                    || tag.starts_with("<h")
                    || tag.starts_with("<li")
                    || tag.starts_with("<tr")
                {
                    out.push('\n');
                }
                i += rel + 1;
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    let decoded = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    decoded
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &'static str {
        "WebFetch"
    }
    fn rule_name(&self) -> &'static str {
        "WebFetch"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Fetch a URL (http is upgraded to https). HTML is reduced to text and sent to a second model call with the user's prompt. A redirect to a different host is not followed; the new URL is returned so you can fetch it. Results are cached for 15 minutes. For GitHub, prefer the gh CLI via Bash.".into(),
            input_schema: json!({"type": "object", "properties": {
                "url": {"type": "string"},
                "prompt": {"type": "string", "description": "What to extract from the page"}
            }, "required": ["url", "prompt"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["url"].as_str().map(str::to_owned)
    }
    fn summary(&self, input: &Value) -> String {
        self.target(input).unwrap_or_default()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: In = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let url = match upgrade_http(&a.url) {
            Ok(u) => u,
            Err(e) => return ToolOutput::err(e),
        };
        let key = url.to_string();
        if let Some(text) = cache_get(ctx, &key) {
            return finish(ctx, &text, &a.prompt).await;
        }
        let host = url.host_str().unwrap_or_default().to_owned();
        let client = match kor_core::net::client_builder()
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.url().host_str() == Some(host.as_str()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let resp = match client.get(url.clone()).send().await {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(kor_core::net::describe(&e)),
        };
        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(missing Location)");
            return ToolOutput::ok(format!(
                "The URL redirected to a different host. Fetch this URL next: {loc}"
            ));
        }
        if !resp.status().is_success() {
            return ToolOutput::err(format!("HTTP {}", resp.status()));
        }
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let slice = if bytes.len() > MAX_BODY {
            &bytes[..MAX_BODY]
        } else {
            &bytes
        };
        let raw = String::from_utf8_lossy(slice);
        let text = truncate(
            &if raw.to_ascii_lowercase().contains("<html") {
                html_to_text(&raw)
            } else {
                raw.into_owned()
            },
            80_000,
        );
        cache_put(ctx, &key, &text);
        finish(ctx, &text, &a.prompt).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PageSummarizer;

    #[test]
    fn http_becomes_https_and_html_is_stripped() {
        let u = upgrade_http("http://example.com/a").unwrap();
        assert_eq!(u.scheme(), "https");
        assert_eq!(u.host_str(), Some("example.com"));
        assert!(upgrade_http("file:///etc/passwd").is_err());
        let text = html_to_text(
            "<html><style>x{}</style><p>Hello &amp; co</p><script>bad()</script></html>",
        );
        assert_eq!(text, "Hello & co");
        let prompt = secondary_prompt("page", "what is the title?");
        assert!(prompt.contains("125-character"));
        assert!(prompt.contains("song lyrics"));
    }

    struct Fixed;

    #[async_trait]
    impl PageSummarizer for Fixed {
        async fn summarize(&self, instructions: &str) -> Result<String, String> {
            if instructions.contains("fail") {
                Err("model down".into())
            } else {
                Ok("summarized".into())
            }
        }
    }

    #[tokio::test]
    async fn cache_uses_the_second_model_and_reports_its_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ToolCtx::new(dir.path()).unwrap();
        ctx.fetch.summarizer = Some(std::sync::Arc::new(Fixed));
        cache_put(&ctx, "https://example.com/a", "hello page");
        let ok = WebFetch
            .call(
                serde_json::json!({"url": "https://example.com/a", "prompt": "title"}),
                &ctx,
            )
            .await;
        assert_eq!(ok.content, "summarized");
        assert!(!ok.is_error);
        cache_put(&ctx, "https://example.com/b", "fail body");
        let bad = WebFetch
            .call(
                serde_json::json!({"url": "https://example.com/b", "prompt": "fail"}),
                &ctx,
            )
            .await;
        assert!(bad.content.contains("model down"), "{}", bad.content);
        assert!(bad.content.contains("fail body"), "{}", bad.content);
    }
}
