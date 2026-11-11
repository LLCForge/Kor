//! `tools_search` (§26.2 "tools.search"): the model asks which tools exist for a task, or reads
//! one program's help summary before using it. Keyword match over the PATH catalog and the typed
//! definitions, plus local-embedding similarity when the `embed-e5-small` component is installed
//! (ADR 0023; nothing leaves the machine).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};

use super::defs::{self, Dirs, Safety};
use super::{Catalog, Entry, help};
use crate::semantic::Embedder;

const MAX_HITS: usize = 15;
/// Catalog entries embedded for semantic search (described ones only).
const MAX_EMBEDDED: usize = 400;

/// One search result line.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub name: String,
    pub line: String,
    pub score: f32,
}

fn entry_text(e: &Entry) -> String {
    format!("{} {} {}", e.name, e.description, e.examples.join(" "))
}

fn entry_line(e: &Entry, privacy: bool) -> String {
    let mut l = e.name.clone();
    if let Some(v) = &e.version {
        let _ = write!(l, " {v}");
    }
    if !e.description.is_empty() {
        let _ = write!(l, " — {}", e.description);
    }
    if !e.examples.is_empty() {
        let _ = write!(l, " (e.g. `{}`)", e.examples.join("`, `"));
    }
    if privacy && e.network {
        l.push_str(" [needs the network: off in privacy mode]");
    }
    l
}

/// Keyword score: share of query words found in the text (name hits count double).
fn keyword(query: &[String], name: &str, text: &str) -> f32 {
    if query.is_empty() {
        return 0.0;
    }
    let text = text.to_lowercase();
    let mut s = 0.0;
    for w in query {
        if name == w {
            s += 2.0;
        } else if name.contains(w.as_str()) {
            s += 1.5;
        } else if text.contains(w.as_str()) {
            s += 1.0;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let n = query.len() as f32;
    s / n
}

/// Catalog and definitions matching `query`, best first.
pub fn keyword_hits(
    cat: &Catalog,
    defined: &[defs::Loaded],
    query: &str,
    privacy: bool,
) -> Vec<Hit> {
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| w.len() >= 2)
        .map(str::to_owned)
        .collect();
    let mut hits: Vec<Hit> = cat
        .tools
        .iter()
        .map(|e| Hit {
            name: e.name.clone(),
            line: entry_line(e, privacy),
            score: keyword(&words, &e.name, &entry_text(e)) + if e.known { 0.05 } else { 0.0 },
        })
        .chain(defined.iter().map(|l| Hit {
            name: l.def.name.clone(),
            line: format!(
                "{} — typed tool: {} (`cli_tool` or ToolSearch; runs `{}`)",
                l.def.name,
                l.def.description,
                l.def.command.join(" ")
            ),
            score: keyword(
                &words,
                &l.def.name,
                &format!("{} {}", l.def.description, l.def.command.join(" ")),
            ) + 0.1,
        }))
        .filter(|h| h.score > 0.1)
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
    });
    hits.truncate(MAX_HITS);
    hits
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let (mut dot, mut nl, mut nr) = (0.0f32, 0.0f32, 0.0f32);
    for (p, q) in left.iter().zip(right) {
        dot += p * q;
        nl += p * p;
        nr += q * q;
    }
    if nl == 0.0 || nr == 0.0 {
        0.0
    } else {
        dot / (nl.sqrt() * nr.sqrt())
    }
}

/// Catalog `generated_at`, entry names and their vectors.
type SemanticIndex = (u64, Vec<String>, Vec<Vec<f32>>);

pub struct ToolsSearch {
    /// `<kor home>/tools`
    home: PathBuf,
    dirs: Dirs,
    privacy: bool,
    permits: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    embedder: Option<Arc<dyn Embedder>>,
    /// The last semantic index.
    index: Mutex<Option<SemanticIndex>>,
}

impl std::fmt::Debug for ToolsSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolsSearch")
            .field("home", &self.home)
            .field("privacy", &self.privacy)
            .finish_non_exhaustive()
    }
}

impl ToolsSearch {
    pub fn new(
        home: PathBuf,
        dirs: Dirs,
        privacy: bool,
        permits: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Self {
        Self {
            home,
            dirs,
            privacy,
            permits,
            embedder,
            index: Mutex::new(None),
        }
    }

    async fn catalog(&self) -> Catalog {
        let mut c = Catalog::load(&self.home, false).await;
        let p = self.permits.clone();
        c.filter(move |n| p(n));
        c
    }

    async fn semantic(&self, cat: &Catalog, query: &str) -> Vec<(String, f32)> {
        let Some(e) = &self.embedder else {
            return Vec::new();
        };
        let described: Vec<&Entry> = cat
            .tools
            .iter()
            .filter(|t| !t.description.is_empty())
            .take(MAX_EMBEDDED)
            .collect();
        let cached = self.index.lock().ok().and_then(|g| {
            g.as_ref()
                .filter(|(at, _, _)| *at == cat.generated_at)
                .map(|(_, n, v)| (n.clone(), v.clone()))
        });
        let (names, vecs) = if let Some(x) = cached {
            x
        } else {
            let texts: Vec<String> = described.iter().map(|t| entry_text(t)).collect();
            let Ok(vecs) = e.embed(e.default_model(), &texts).await else {
                return Vec::new();
            };
            let names: Vec<String> = described.iter().map(|t| t.name.clone()).collect();
            if let Ok(mut g) = self.index.lock() {
                *g = Some((cat.generated_at, names.clone(), vecs.clone()));
            }
            (names, vecs)
        };
        let Ok(mut q) = e.embed_query(e.default_model(), &[query.to_owned()]).await else {
            return Vec::new();
        };
        let Some(q) = q.pop() else { return Vec::new() };
        let mut out: Vec<(String, f32)> = names
            .into_iter()
            .zip(vecs.iter().map(|v| cosine(&q, v)))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.truncate(MAX_HITS);
        out
    }
}

#[async_trait]
impl Tool for ToolsSearch {
    fn name(&self) -> &'static str {
        "tools_search"
    }
    fn rule_name(&self) -> &'static str {
        "tools_search"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "tools_search".into(),
            description: "Find command-line programs installed on this computer and typed tool definitions for a task (`query`, e.g. \"convert video\", \"kubernetes logs\"), or read one program's help summary (`describe`: its name) before using it.".into(),
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"},"describe":{"type":"string"}},"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input
            .get("describe")
            .or_else(|| input.get("query"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        if let Some(name) = input.get("describe").and_then(Value::as_str) {
            if !(self.permits)(name) {
                return ToolOutput::err(format!(
                    "`{name}` is not allowed by [tools] in the configuration"
                ));
            }
            let (defined, _) = defs::load(&self.dirs);
            if let Some(l) = defined.iter().find(|l| l.def.name == name) {
                return ToolOutput::ok(format!(
                    "{} (typed tool, {:?}): {}\ncommand: {}\nschema: {}",
                    l.def.name,
                    l.def.safety,
                    l.def.description,
                    l.def.command.join(" "),
                    l.def.input_schema()
                ));
            }
            return match help::describe(&self.home.join("help"), name, false, ctx).await {
                Ok(h) => ToolOutput::ok(help::render(&h)),
                Err(e) => ToolOutput::err(e),
            };
        }
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return ToolOutput::err("give `query` or `describe`");
        }
        let cat = self.catalog().await;
        let (mut defined, _) = defs::load(&self.dirs);
        defined.retain(|l| {
            (self.permits)(l.def.program()) && !(self.privacy && l.def.safety == Safety::Network)
        });
        let mut hits = keyword_hits(&cat, &defined, query, self.privacy);
        for (name, cos) in self.semantic(&cat, query).await {
            if cos < 0.8 || hits.iter().any(|h| h.name == name) {
                continue;
            }
            if let Some(e) = cat.get(&name) {
                hits.push(Hit {
                    name,
                    line: entry_line(e, self.privacy),
                    score: cos,
                });
            }
        }
        if hits.is_empty() {
            return ToolOutput::ok(format!(
                "no installed tool matches `{query}` ({} programs on PATH)",
                cat.tools.len()
            ));
        }
        let mut out = String::new();
        for h in hits.iter().take(MAX_HITS) {
            let _ = writeln!(out, "- {}", h.line);
        }
        ToolOutput::ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[tokio::test]
    async fn finds_tools_by_purpose_and_typed_definitions() {
        let dir = tempfile::tempdir().unwrap();
        let found: BTreeMap<String, PathBuf> = ["ffmpeg", "git", "curl", "kubectl"]
            .iter()
            .map(|n| ((*n).to_owned(), dir.path().join(n)))
            .collect();
        let cat = Catalog::from_found(found, None, false).await;
        let d: defs::ToolDef = toml::from_str(
            "name = \"thumbs\"\ndescription = \"video thumbnails\"\ncommand = \"ffmpeg -version\"",
        )
        .unwrap();
        let loaded = vec![defs::Loaded {
            def: d,
            origin: defs::Origin::User,
            file: dir.path().join("t.toml"),
        }];
        let hits = keyword_hits(&cat, &loaded, "convert video", false);
        assert_eq!(
            hits.first().map(|h| h.name.as_str()),
            Some("ffmpeg"),
            "{hits:?}"
        );
        assert!(hits.iter().any(|h| h.name == "thumbs"));
        let hits = keyword_hits(&cat, &[], "kubectl", true);
        assert_eq!(hits[0].name, "kubectl");
        assert!(hits[0].line.contains("privacy mode"));
        assert!(keyword_hits(&cat, &[], "zzzz", false).is_empty());
    }

    #[tokio::test]
    async fn describe_and_query_through_the_tool() {
        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(root.path()).unwrap();
        std::fs::write(home.path().join("gv.toml"), "name = \"git_version\"\ndescription = \"print git version\"\ncommand = \"git --version\"").unwrap();
        let t = ToolsSearch::new(
            home.path().into(),
            Dirs {
                home: Some(home.path().into()),
                project: None,
            },
            false,
            Arc::new(|n| n != "rm"),
            None,
        );
        let out = t.call(json!({"describe": "git_version"}), &ctx).await;
        assert!(
            out.content.contains("typed tool") && out.content.contains("git --version"),
            "{}",
            out.content
        );
        assert!(t.call(json!({"describe": "rm"}), &ctx).await.is_error);
        assert!(t.call(json!({}), &ctx).await.is_error);
        let out = t.call(json!({"query": "git version"}), &ctx).await;
        assert!(out.content.contains("git_version"), "{}", out.content);
        // The scan wrote the cache it reads next time.
        assert!(Catalog::cached(home.path()).is_some());
    }
}
