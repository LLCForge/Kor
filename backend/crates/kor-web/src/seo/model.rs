//! The model side of `kor seo` (content, meta, alt text, team summary): one small trait so the
//! pipelines run against a real provider or a scripted one in tests, plus Markdown → HTML for
//! publishing.

use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use kor_core::{ContentPart, ImageSource, Message, Role};
use kor_providers::{CancellationToken, ChatRequest, Provider};

/// Something that answers a prompt.
#[async_trait]
pub trait Ask: Send + Sync {
    async fn ask(&self, prompt: String, max_tokens: u32) -> Result<String, String>;
    /// With an image (`media_type`, base64). Models without vision return an error; callers fall
    /// back to OCR text.
    async fn ask_image(
        &self,
        prompt: String,
        media_type: &str,
        data: &str,
        max_tokens: u32,
    ) -> Result<String, String>;
    fn vision(&self) -> bool;
    /// `provider/model`, for cost estimates and reports.
    fn id(&self) -> String;
}

/// A configured provider and model (picked by the cost router's route for the job).
#[derive(Clone)]
pub struct ProviderAsk {
    pub provider: Arc<dyn Provider>,
    pub model: String,
}

impl std::fmt::Debug for ProviderAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderAsk")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl ProviderAsk {
    async fn run(&self, msg: Message, max_tokens: u32) -> Result<String, String> {
        let req = ChatRequest {
            model: self.model.clone(),
            system: Some(
                "You are an SEO specialist and editor. Follow the requested format exactly. \
                 Never invent facts, prices, statistics or sources."
                    .into(),
            ),
            messages: vec![msg],
            max_tokens: Some(max_tokens),
            ..ChatRequest::default()
        };
        let stream = self
            .provider
            .complete(req, CancellationToken::new())
            .await
            .map_err(|e| e.message)?;
        let text = kor_agent::subagent::collect_text(stream)
            .await
            .map_err(|e| e.message)?;
        if text.trim().is_empty() {
            Err("empty reply".into())
        } else {
            Ok(text)
        }
    }
}

#[async_trait]
impl Ask for ProviderAsk {
    async fn ask(&self, prompt: String, max_tokens: u32) -> Result<String, String> {
        self.run(Message::text(Role::User, prompt), max_tokens)
            .await
    }

    async fn ask_image(
        &self,
        prompt: String,
        media_type: &str,
        data: &str,
        max_tokens: u32,
    ) -> Result<String, String> {
        if !self.vision() {
            return Err("model has no vision".into());
        }
        let msg = Message {
            role: Role::User,
            parts: vec![
                ContentPart::Image {
                    source: ImageSource::Base64 {
                        media_type: media_type.to_owned(),
                        data: data.to_owned(),
                    },
                },
                ContentPart::Text { text: prompt },
            ],
        };
        self.run(msg, max_tokens).await
    }

    fn vision(&self) -> bool {
        self.provider.manifest().vision
    }

    fn id(&self) -> String {
        format!("{}/{}", self.provider.manifest().id, self.model)
    }
}

/// Which cost-router route a job uses (spec §5 routing; ADR 0055).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Briefs, outlines, drafts, the team summary: `[routing] docs`.
    Writing,
    /// Bulk titles/descriptions/alt text: `[routing] simple_edit` (the cheap route).
    Bulk,
}

/// The model for `job`: an explicit `--model`, else the routed model when routing is on and the
/// route is set, else `agent.model`. Provider rules (privacy mode, allow lists) apply as for
/// any one-shot request.
pub fn ask_for(
    l: &kor_i18n::Localizer,
    loaded: &kor_config::Loaded,
    job: Job,
    model: Option<&str>,
) -> Result<ProviderAsk, String> {
    let r = &loaded.config.routing;
    let routed = match job {
        Job::Writing => r.docs.trim(),
        Job::Bulk => r.simple_edit.trim(),
    };
    let pick = model
        .map(str::to_owned)
        .or_else(|| (r.enabled && !routed.is_empty()).then(|| routed.to_owned()));
    let (provider, model) = kor_runtime::oneshot_provider(l, loaded, pick.as_deref())?;
    Ok(ProviderAsk { provider, model })
}

/// Strip a ```` ```lang ```` fence a model wrapped its answer in.
pub fn unfence(s: &str) -> &str {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let body = rest.split_once('\n').map_or("", |(_, b)| b);
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

fn esc(s: &str) -> String {
    super::audit::escape(s)
}

/// Inline Markdown: `**bold**`, `*em*`, `` `code` ``, `[text](url)` (http/https/relative only).
fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("**")
            && let Some(end) = r.find("**")
        {
            let _ = write!(out, "<strong>{}</strong>", inline(&r[..end]));
            rest = &r[end + 2..];
        } else if let Some(r) = rest.strip_prefix('`')
            && let Some(end) = r.find('`')
        {
            let _ = write!(out, "<code>{}</code>", esc(&r[..end]));
            rest = &r[end + 1..];
        } else if let Some(r) = rest.strip_prefix('[')
            && let Some(mid) = r.find("](")
            && let Some(end) = r[mid..].find(')')
        {
            let (text, href) = (&r[..mid], &r[mid + 2..mid + end]);
            let safe = href.starts_with("http://")
                || href.starts_with("https://")
                || href.starts_with('/');
            if safe {
                let _ = write!(out, "<a href=\"{}\">{}</a>", esc(href), inline(text));
            } else {
                out.push_str(&inline(text));
            }
            rest = &r[mid + end + 1..];
        } else if let Some(r) = rest.strip_prefix('*')
            && let Some(end) = r.find('*')
            && end > 0
        {
            let _ = write!(out, "<em>{}</em>", inline(&r[..end]));
            rest = &r[end + 1..];
        } else {
            let ch = rest.chars().next().unwrap_or(' ');
            out.push_str(&esc(&ch.to_string()));
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

/// Block Markdown → HTML: headings, paragraphs, `-`/`*`/`1.` lists, `>` quotes, fenced code.
/// Raw HTML in the source is escaped (model output is untrusted).
pub fn markdown_to_html(md: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<&str> = Vec::new();
    let mut list: Option<&str> = None;
    let mut code: Option<String> = None;
    let flush = |para: &mut Vec<&str>, out: &mut String| {
        if !para.is_empty() {
            let _ = writeln!(out, "<p>{}</p>", inline(&para.join(" ")));
            para.clear();
        }
    };
    for line in md.lines() {
        if let Some(buf) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                let _ = writeln!(out, "<pre><code>{}</code></pre>", esc(buf));
                code = None;
            } else {
                buf.push_str(line);
                buf.push('\n');
            }
            continue;
        }
        let t = line.trim();
        let item = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .map(|r| ("ul", r))
            .or_else(|| {
                let d = t.find(". ")?;
                t[..d]
                    .chars()
                    .all(|c| c.is_ascii_digit())
                    .then(|| ("ol", &t[d + 2..]))
            })
            .filter(|_| !t.is_empty());
        if item.is_none()
            && let Some(kind) = list.take()
        {
            let _ = writeln!(out, "</{kind}>");
        }
        if t.starts_with("```") {
            flush(&mut para, &mut out);
            code = Some(String::new());
        } else if t.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some((kind, text)) = item {
            flush(&mut para, &mut out);
            if list != Some(kind) {
                if let Some(k) = list {
                    let _ = writeln!(out, "</{k}>");
                }
                let _ = writeln!(out, "<{kind}>");
                list = Some(kind);
            }
            let _ = writeln!(out, "<li>{}</li>", inline(text));
        } else if t.starts_with('#') {
            flush(&mut para, &mut out);
            let level = t.chars().take_while(|c| *c == '#').count().min(6);
            let _ = writeln!(out, "<h{level}>{}</h{level}>", inline(t[level..].trim()));
        } else if let Some(q) = t.strip_prefix('>') {
            flush(&mut para, &mut out);
            let _ = writeln!(out, "<blockquote>{}</blockquote>", inline(q.trim()));
        } else {
            para.push(t);
        }
    }
    flush(&mut para, &mut out);
    if let Some(k) = list {
        let _ = writeln!(out, "</{k}>");
    }
    if let Some(buf) = code {
        let _ = writeln!(out, "<pre><code>{}</code></pre>", esc(&buf));
    }
    out
}

#[cfg(test)]
pub mod testing {
    //! A scripted [`Ask`] for tests: answers in order, records prompts.
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    pub struct Scripted {
        pub replies: Mutex<Vec<String>>,
        pub prompts: Mutex<Vec<String>>,
        pub vision: bool,
    }

    impl Scripted {
        pub fn new(replies: &[&str]) -> Self {
            Self {
                replies: Mutex::new(replies.iter().rev().map(|s| (*s).to_owned()).collect()),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl Ask for Scripted {
        async fn ask(&self, prompt: String, _: u32) -> Result<String, String> {
            self.prompts.lock().unwrap().push(prompt);
            self.replies
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| "no scripted reply".into())
        }
        async fn ask_image(
            &self,
            prompt: String,
            _: &str,
            _: &str,
            m: u32,
        ) -> Result<String, String> {
            if !self.vision {
                return Err("model has no vision".into());
            }
            self.ask(format!("[image] {prompt}"), m).await
        }
        fn vision(&self) -> bool {
            self.vision
        }
        fn id(&self) -> String {
            "mock/scripted".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_blocks_and_escaping() {
        let h = markdown_to_html(
            "# Başlık\n\nBir **kalın** ve [link](https://x.test) <script>\n- a\n- b\n\n1. c\n```\n<x>\n```\n",
        );
        assert!(h.contains("<h1>Başlık</h1>"));
        assert!(h.contains("<strong>kalın</strong>"));
        assert!(h.contains("<a href=\"https://x.test\">link</a> &lt;script&gt;"));
        assert!(h.contains("<ul>\n<li>a</li>\n<li>b</li>\n</ul>"));
        assert!(h.contains("<ol>\n<li>c</li>"));
        assert!(h.contains("<pre><code>&lt;x&gt;\n</code></pre>"));
        assert!(!markdown_to_html("[x](javascript:alert(1))").contains("href"));
        assert_eq!(unfence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
    }
}
