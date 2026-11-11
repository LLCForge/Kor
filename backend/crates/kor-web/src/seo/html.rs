//! A small, forgiving HTML scanner for SEO facts. It keeps the 1-based line of every tag so
//! findings can say `url:line` and a selector (spec §29.16: evidence at line level), and never
//! executes or renders anything. Script/style bodies are raw text; comments are skipped.

use serde::Serialize;

/// One element of interest with its source line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct At<T> {
    pub value: T,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Img {
    pub src: String,
    pub alt: Option<String>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub href: String,
    pub rel: String,
    pub text: String,
    pub line: u32,
    /// Inline style hides it (`display:none`, `visibility:hidden`, off-screen indent).
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hreflang {
    pub lang: String,
    pub href: String,
    pub line: u32,
}

/// Sub-resource reference (`img src`, `script src`, `link href`, `iframe src`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resource {
    pub tag: String,
    pub url: String,
    pub line: u32,
}

/// What the audit needs from one HTML document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Page {
    pub titles: Vec<At<String>>,
    pub descriptions: Vec<At<String>>,
    pub robots: Vec<At<String>>,
    pub canonicals: Vec<At<String>>,
    pub hreflang: Vec<Hreflang>,
    /// `(level, text)`.
    pub headings: Vec<At<(u8, String)>>,
    pub images: Vec<Img>,
    pub links: Vec<Link>,
    pub jsonld: Vec<At<String>>,
    pub resources: Vec<Resource>,
    pub lang: Option<String>,
    pub viewport: bool,
    pub word_count: usize,
    /// Visible text (first 20 000 chars), for content checks and prompts.
    pub text: String,
}

const MAX_TEXT: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    closing: bool,
    line: u32,
}

impl Tag {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;`, `&apos;`, `&nbsp;`, `&#NN;`, `&#xHH;`.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let decoded = end.and_then(|e| {
            let ent = &rest[1..e];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => ent
                    .strip_prefix("#x")
                    .or_else(|| ent.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, e + 1))
        });
        if let Some((c, len)) = decoded {
            out.push(c);
            rest = &rest[len..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let name = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = decode_entities(&s[vs..i]);
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = decode_entities(&s[vs..i]);
            }
        }
        out.push((name, value));
    }
    out
}

/// What the scanner emits in document order.
#[derive(Debug)]
enum Token {
    Tag(Tag),
    Text(String),
    /// Raw body of `<script>` / `<style>` with the opening tag.
    Raw(Tag, String),
}

#[allow(clippy::naive_bytecount)] // short spans; no extra crate for this
fn tokens(html: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut line: u32 = 1;
    let mut pos = 0;
    let lower = html.to_ascii_lowercase();
    let count = |from: usize, to: usize| -> u32 {
        u32::try_from(
            html.as_bytes()[from..to]
                .iter()
                .filter(|&&c| c == b'\n')
                .count(),
        )
        .unwrap_or(u32::MAX)
    };
    while pos < html.len() {
        let Some(off) = html[pos..].find('<') else {
            out.push(Token::Text(html[pos..].to_owned()));
            break;
        };
        let lt = pos + off;
        if lt > pos {
            out.push(Token::Text(html[pos..lt].to_owned()));
            line = line.saturating_add(count(pos, lt));
        }
        if html[lt..].starts_with("<!--") {
            let end = html[lt..].find("-->").map_or(html.len(), |e| lt + e + 3);
            line = line.saturating_add(count(lt, end));
            pos = end;
            continue;
        }
        let Some(gt) = html[lt..].find('>').map(|g| lt + g) else {
            break;
        };
        let inner = &html[lt + 1..gt];
        let tag_line = line;
        line = line.saturating_add(count(lt, gt + 1));
        pos = gt + 1;
        if inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        let closing = inner.starts_with('/');
        let body = inner.trim_start_matches('/');
        let name_end = body
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(body.len());
        let name = body[..name_end].to_ascii_lowercase();
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            continue;
        }
        let tag = Tag {
            attrs: if closing {
                Vec::new()
            } else {
                parse_attrs(&body[name_end..])
            },
            name,
            closing,
            line: tag_line,
        };
        if !closing && matches!(tag.name.as_str(), "script" | "style") {
            let close = format!("</{}", tag.name);
            let end = lower[pos..].find(&close).map_or(html.len(), |e| pos + e);
            let raw = html[pos..end].to_owned();
            line = line.saturating_add(count(pos, end));
            pos = html[end..].find('>').map_or(html.len(), |g| end + g + 1);
            out.push(Token::Raw(tag, raw));
            continue;
        }
        out.push(Token::Tag(tag));
    }
    out
}

fn hidden_style(style: &str) -> bool {
    let s: String = style
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    s.contains("display:none") || s.contains("visibility:hidden") || s.contains("text-indent:-9")
}

/// Scan a document.
#[allow(clippy::too_many_lines)] // one pass over the token stream
pub fn parse(html: &str) -> Page {
    let mut p = Page::default();
    // Open elements whose text we collect: (kind, line, text, hidden).
    let mut capture: Vec<(String, u32, String)> = Vec::new();
    let mut open_link: Option<(Link, String)> = None;
    let mut skip_text = 0usize; // inside noscript/template
    let mut text = String::new();
    for t in tokens(html) {
        match t {
            Token::Text(s) => {
                if skip_text > 0 {
                    continue;
                }
                let s = decode_entities(&s);
                for c in &mut capture {
                    c.2.push_str(&s);
                }
                if let Some((_, buf)) = open_link.as_mut() {
                    buf.push_str(&s);
                }
                if text.len() < MAX_TEXT {
                    text.push_str(&s);
                    text.push(' ');
                }
            }
            Token::Raw(tag, raw) => {
                if tag.name == "script" {
                    if let Some(src) = tag.attr("src") {
                        p.resources.push(Resource {
                            tag: "script".into(),
                            url: src.to_owned(),
                            line: tag.line,
                        });
                    }
                    if tag
                        .attr("type")
                        .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/ld+json"))
                    {
                        p.jsonld.push(At {
                            value: raw.trim().to_owned(),
                            line: tag.line,
                        });
                    }
                }
            }
            Token::Tag(tag) => on_tag(&mut p, &tag, &mut capture, &mut open_link, &mut skip_text),
        }
    }
    p.word_count = text.split_whitespace().count();
    p.text = squash(&text);
    p
}

#[allow(clippy::too_many_lines)] // one arm per element of interest
fn on_tag(
    p: &mut Page,
    tag: &Tag,
    capture: &mut Vec<(String, u32, String)>,
    open_link: &mut Option<(Link, String)>,
    skip_text: &mut usize,
) {
    let n = tag.name.as_str();
    if tag.closing {
        if matches!(n, "noscript" | "template") {
            *skip_text = skip_text.saturating_sub(1);
        }
        if n == "a"
            && let Some((mut l, buf)) = open_link.take()
        {
            l.text = squash(&buf);
            p.links.push(l);
        }
        if let Some(i) = capture.iter().rposition(|c| c.0 == n) {
            let (kind, line, buf) = capture.remove(i);
            let value = squash(&buf);
            if kind == "title" {
                p.titles.push(At { value, line });
            } else if let Some(level) = kind.strip_prefix('h').and_then(|d| d.parse().ok()) {
                p.headings.push(At {
                    value: (level, value),
                    line,
                });
            }
        }
        return;
    }
    let line = tag.line;
    let a = |k: &str| tag.attr(k).map(str::to_owned);
    match n {
        "html" => p.lang = a("lang"),
        "title" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            capture.push((n.to_owned(), line, String::new()));
        }
        "noscript" | "template" => *skip_text += 1,
        "meta" => {
            let name = a("name")
                .or_else(|| a("property"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let content = a("content").unwrap_or_default();
            match name.as_str() {
                "description" => p.descriptions.push(At {
                    value: squash(&content),
                    line,
                }),
                "robots" | "googlebot" => p.robots.push(At {
                    value: content.to_ascii_lowercase(),
                    line,
                }),
                "viewport" => p.viewport = true,
                _ => {}
            }
        }
        "link" => {
            let rel = a("rel").unwrap_or_default().to_ascii_lowercase();
            let href = a("href").unwrap_or_default();
            if rel.split_whitespace().any(|r| r == "canonical") {
                p.canonicals.push(At { value: href, line });
            } else if rel.split_whitespace().any(|r| r == "alternate")
                && let Some(lang) = a("hreflang")
            {
                p.hreflang.push(Hreflang { lang, href, line });
            } else if rel.contains("stylesheet") || rel.contains("preload") || rel.contains("icon")
            {
                p.resources.push(Resource {
                    tag: "link".into(),
                    url: href,
                    line,
                });
            }
        }
        "a" => {
            if let Some((mut l, buf)) = open_link.take() {
                l.text = squash(&buf);
                p.links.push(l);
            }
            if let Some(href) = a("href") {
                *open_link = Some((
                    Link {
                        href,
                        rel: a("rel").unwrap_or_default().to_ascii_lowercase(),
                        text: String::new(),
                        line,
                        hidden: tag.attr("style").is_some_and(hidden_style)
                            || tag.attr("hidden").is_some(),
                    },
                    String::new(),
                ));
            }
        }
        "img" => {
            let src = a("src").or_else(|| a("data-src")).unwrap_or_default();
            p.resources.push(Resource {
                tag: "img".into(),
                url: src.clone(),
                line,
            });
            p.images.push(Img {
                src,
                alt: a("alt"),
                width: a("width"),
                height: a("height"),
                line,
            });
        }
        "iframe" | "source" | "video" | "audio" | "embed" => {
            if let Some(src) = a("src") {
                p.resources.push(Resource {
                    tag: n.to_owned(),
                    url: src,
                    line,
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_facts_with_lines() {
        let html = "<!doctype html>\n<html lang=\"tr\">\n<head>\n<title>Ana &amp; Sayfa</title>\n\
            <meta name=\"description\" content=\"Açıklama\">\n<link rel=\"canonical\" href=\"https://x.test/\">\n\
            <link rel=alternate hreflang=en href=https://x.test/en>\n<!-- <h1>not</h1> -->\n\
            <script type=\"application/ld+json\">{\"@type\":\"Organization\"}</script>\n\
            <meta name=viewport content=\"width=device-width\"></head>\n<body><h1>Başlık <b>bir</b></h1>\n\
            <img src=\"http://cdn.test/a.png\" width=10><a href=\"/x\" style=\"display: none\">gizli</a>\n\
            <noscript>skip me</noscript><h3>alt</h3></body></html>";
        let p = parse(html);
        assert_eq!(p.titles[0].value, "Ana & Sayfa");
        assert_eq!(p.titles[0].line, 4);
        assert_eq!(p.descriptions[0].value, "Açıklama");
        assert_eq!(p.canonicals[0].line, 6);
        assert_eq!(p.hreflang[0].lang, "en");
        assert_eq!(p.jsonld.len(), 1);
        assert_eq!(p.jsonld[0].line, 9);
        assert!(p.viewport);
        assert_eq!(p.lang.as_deref(), Some("tr"));
        assert_eq!(p.headings[0].value, (1, "Başlık bir".to_owned()));
        assert_eq!(p.headings[0].line, 11);
        assert_eq!(p.headings[1].value.0, 3);
        assert_eq!(p.images[0].alt, None);
        assert_eq!(p.links[0].text, "gizli");
        assert!(p.links[0].hidden);
        assert!(!p.text.contains("skip me"));
        assert!(!p.text.contains("not"));
    }

    #[test]
    fn entities_decode() {
        assert_eq!(
            decode_entities("a &lt;b&gt; &#252; &#x15F; &bogus"),
            "a <b> ü ş &bogus"
        );
    }
}
