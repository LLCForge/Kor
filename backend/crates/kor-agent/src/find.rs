//! `kor find "<question>"` (spec §8.9, ADR 0042): natural-language codebase search.
//!
//! Three rankings are fused by reciprocal rank fusion (k = 60): the project graph's keyword query
//! over symbol names, paths and rationale (`kor_graph`), the local semantic index (`file_search`'s
//! cache, `embed-e5-small` on this computer) when the model is installed, and a plain keyword scan
//! of file text otherwise. The result is a deterministic list of `file:line` hits plus the graph
//! links between them; [`explain`] then asks a model for a plain-prose walk through the flow. No
//! model, no network: the list alone is still the answer (offline, `--no-model`).

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use kor_graph::{EdgeKind, Graph, NodeKind};
use kor_providers::Provider;
use kor_tools::ToolCtx;
use serde::Serialize;

use crate::gitflow::{self, FlowError};
use crate::semantic::{self, FileSearch};

const RRF_K: f32 = 60.0;
const PER_SOURCE: usize = 30;
const SNIPPET_LINES: usize = 14;
const PROMPT_BUDGET: usize = 24_000;
const MAX_LINKS: usize = 24;

/// Where a hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Graph,
    Semantic,
    Keyword,
}

/// One place in the code, `file:line` (1-based; `end` is the last line of the excerpt).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub file: String,
    pub line: usize,
    pub end: usize,
    /// Symbol name and kind when the graph knows the place.
    pub name: Option<String>,
    pub kind: Option<NodeKind>,
    /// Graph node id (for opening it in the graph panel).
    pub node: Option<String>,
    pub sources: Vec<Source>,
    pub score: f32,
}

/// A graph edge between two hits (or from a top hit to a neighbour): `from --kind--> to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowLink {
    pub from: String,
    pub from_at: Option<String>,
    pub kind: String,
    pub to: String,
    pub to_at: Option<String>,
}

/// How the text side was searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextIndex {
    /// The local embedding model.
    Semantic,
    /// No model installed (or it failed): keyword scan.
    Keyword,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Found {
    pub question: String,
    pub hits: Vec<Hit>,
    pub links: Vec<FlowLink>,
    pub text_index: TextIndex,
    /// Nodes in the graph; 0 when it could not be opened.
    pub graph_nodes: usize,
    /// Why the semantic index was not used, when it was installed but failed.
    pub notes: Vec<String>,
}

/// One ranked candidate before fusion.
struct Cand {
    file: String,
    line: usize,
    end: usize,
    name: Option<String>,
    kind: Option<NodeKind>,
    node: Option<String>,
}

fn hidden(ctx: &ToolCtx, rel: &str) -> bool {
    ctx.ai_ignore.as_ref().is_some_and(|g| {
        g.matched_path_or_any_parents(ctx.root.join(rel), false)
            .is_ignore()
    }) || semantic::walk_excluded(rel)
}

/// Ranked graph nodes for the question, and the graph itself for links.
fn graph_side(ctx: &ToolCtx, question: &str) -> (Vec<Cand>, Option<Graph>) {
    let Ok(mut g) = Graph::open(&ctx.root) else {
        return (Vec::new(), None);
    };
    if g.update().is_err() {
        return (Vec::new(), None);
    }
    let cands = g
        .query(question, PER_SOURCE * 2)
        .into_iter()
        .filter_map(|h| {
            let file = h.node.file.clone()?.replace('\\', "/");
            if hidden(ctx, &file)
                || matches!(
                    h.node.kind,
                    NodeKind::Package | NodeKind::Server | NodeKind::Service | NodeKind::Media
                )
            {
                return None;
            }
            let span = g.node(&h.node.id).and_then(|n| n.span);
            let line = span.map_or(1, |s| s.0 as usize);
            let end = span.map_or(line, |s| s.1 as usize).max(line);
            Some(Cand {
                file,
                line,
                end,
                name: Some(h.node.name.clone()),
                kind: Some(h.node.kind),
                node: Some(h.node.id.clone()),
            })
        })
        .take(PER_SOURCE)
        .collect();
    (cands, Some(g))
}

/// Chunks scored by how many distinct question words they contain (then by occurrences).
fn keyword_side(ctx: &ToolCtx, question: &str) -> Vec<Cand> {
    let words: BTreeSet<String> = kor_graph::query::tokens(question).into_iter().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, usize, Cand)> = Vec::new();
    for (rel, abs, ..) in semantic::walk(&ctx.root, ctx.ai_ignore.as_deref()) {
        let Some(text) = semantic::read_text(&abs) else {
            continue;
        };
        for (start, end, body) in semantic::chunk_text(&text) {
            let lower = body.to_lowercase();
            let distinct = words.iter().filter(|w| lower.contains(w.as_str())).count();
            if distinct == 0 || !kor_tools::scan_secrets(&body).is_empty() {
                continue;
            }
            let hits: usize = words
                .iter()
                .map(|w| lower.matches(w.as_str()).count())
                .sum();
            scored.push((
                distinct,
                hits,
                Cand {
                    file: rel.clone(),
                    line: start,
                    end,
                    name: None,
                    kind: None,
                    node: None,
                },
            ));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    scored
        .into_iter()
        .take(PER_SOURCE)
        .map(|(.., c)| c)
        .collect()
}

/// Reciprocal rank fusion. A candidate whose lines fall inside an earlier hit's range of the same
/// file is the same place; the graph's precise line and name win.
fn fuse(rankings: Vec<(Source, Vec<Cand>)>, limit: usize) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    for (source, list) in rankings {
        for (rank, c) in list.into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let add = 1.0 / (RRF_K + rank as f32 + 1.0);
            let same = hits.iter_mut().find(|h| {
                h.file == c.file
                    && (h.line <= c.end && c.line <= h.end)
                    && (h.node.is_none() || c.node.is_none() || h.node == c.node)
            });
            match same {
                Some(h) => {
                    if !h.sources.contains(&source) {
                        h.sources.push(source);
                        h.score += add;
                    }
                    if h.node.is_none() && c.node.is_some() {
                        (h.line, h.end, h.name, h.kind, h.node) =
                            (c.line, c.end, c.name, c.kind, c.node);
                    }
                }
                None => hits.push(Hit {
                    file: c.file,
                    line: c.line,
                    end: c.end,
                    name: c.name,
                    kind: c.kind,
                    node: c.node,
                    sources: vec![source],
                    score: add,
                }),
            }
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });
    hits.truncate(limit);
    hits
}

fn at(n: &kor_graph::GraphNode) -> Option<String> {
    let f = n.file.as_ref()?.to_string_lossy().replace('\\', "/");
    Some(format!("{f}:{}", n.span.map_or(1, |s| s.0)))
}

/// Dependency edges between hit nodes, then the direct callees and callers of the top three.
fn links(g: &Graph, hits: &[Hit]) -> Vec<FlowLink> {
    let ids: HashSet<&str> = hits.iter().filter_map(|h| h.node.as_deref()).collect();
    let top: Vec<&str> = hits
        .iter()
        .filter_map(|h| h.node.as_deref())
        .take(3)
        .collect();
    let mut inner = Vec::new();
    let mut outer = Vec::new();
    let mut seen = HashSet::new();
    for edge in g.edges() {
        if !edge.kind.is_dependency() || edge.kind == EdgeKind::Contains {
            continue;
        }
        let (s, t) = (edge.source.as_str(), edge.target.as_str());
        let bucket = if ids.contains(s) && ids.contains(t) {
            &mut inner
        } else if top.contains(&s) || top.contains(&t) {
            &mut outer
        } else {
            continue;
        };
        let (Some(from), Some(to)) = (g.node(s), g.node(t)) else {
            continue;
        };
        if !seen.insert((s.to_owned(), t.to_owned(), edge.kind.as_str())) {
            continue;
        }
        bucket.push(FlowLink {
            from: from.name.clone(),
            from_at: at(from),
            kind: edge.kind.as_str().to_owned(),
            to: to.name.clone(),
            to_at: at(to),
        });
    }
    let sort = |v: &mut Vec<FlowLink>| v.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    sort(&mut inner);
    sort(&mut outer);
    inner.extend(outer);
    inner.truncate(MAX_LINKS);
    inner
}

/// Search the project at `ctx.root`. `semantic` is the local-model index (`None` = keyword scan).
pub async fn search(
    ctx: &ToolCtx,
    question: &str,
    limit: usize,
    semantic: Option<&FileSearch>,
) -> Result<Found, String> {
    let question = question.trim();
    if question.chars().count() < 2 || question.len() > 1000 {
        return Err("the question must be 2 to 1000 characters".into());
    }
    let limit = limit.clamp(1, 50);
    let mut notes = Vec::new();
    let mut text_index = TextIndex::Keyword;
    let mut text: Option<(Source, Vec<Cand>)> = None;
    if let Some(fs) = semantic {
        match fs.search(ctx, question, PER_SOURCE).await {
            Ok(found) => {
                text_index = TextIndex::Semantic;
                let cands = found
                    .hits
                    .into_iter()
                    .filter(|h| !hidden(ctx, &h.file))
                    .map(|h| Cand {
                        file: h.file,
                        line: h.start,
                        end: h.end,
                        name: None,
                        kind: None,
                        node: None,
                    })
                    .collect();
                text = Some((Source::Semantic, cands));
            }
            Err(e) => notes.push(e),
        }
    }
    let (ctx_root, q) = (ctx.root.clone(), question.to_owned());
    let ai = ctx.ai_ignore.clone();
    let (graph_cands, graph, keyword) = tokio::task::spawn_blocking(move || {
        let mut c = ToolCtx::new(&ctx_root).map_err(|e| e.to_string())?;
        c.ai_ignore = ai;
        let (gc, g) = graph_side(&c, &q);
        let kw = if text.is_none() {
            Some(keyword_side(&c, &q))
        } else {
            None
        };
        Ok::<_, String>((gc, g, kw.map(|k| (Source::Keyword, k)).or(text)))
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut rankings = vec![(Source::Graph, graph_cands)];
    rankings.extend(keyword);
    let hits = fuse(rankings, limit);
    let links = graph.as_ref().map(|g| links(g, &hits)).unwrap_or_default();
    Ok(Found {
        question: question.to_owned(),
        graph_nodes: graph.as_ref().map_or(0, |g| g.nodes().count()),
        hits,
        links,
        text_index,
        notes,
    })
}

/// Lines `line..=end` of `file` (at most [`SNIPPET_LINES`]), numbered.
fn excerpt(root: &Path, h: &Hit) -> Option<String> {
    let text = semantic::read_text(&root.join(&h.file))?;
    let last = h.end.min(h.line + SNIPPET_LINES - 1);
    let mut out = String::new();
    for (i, l) in text.lines().enumerate().skip(h.line.saturating_sub(1)) {
        if i + 1 > last {
            break;
        }
        let l: String = l.chars().take(200).collect();
        let _ = writeln!(out, "{:>5} {l}", i + 1);
    }
    (!out.is_empty() && kor_tools::scan_secrets(&out).is_empty()).then_some(out)
}

/// One-line label: `src/auth.rs:3 login (function)`.
pub fn label(h: &Hit) -> String {
    let mut s = format!("{}:{}", h.file, h.line);
    if let Some(n) = &h.name {
        let _ = write!(s, " {n}");
        if let Some(k) = h.kind {
            let _ = write!(s, " ({})", k.as_str());
        }
    }
    s
}

/// One-line link: `main_entry (src/main.rs:4) calls login (src/auth.rs:3)`.
pub fn link_line(l: &FlowLink) -> String {
    let side = |n: &str, a: &Option<String>| match a {
        Some(a) => format!("{n} ({a})"),
        None => n.to_owned(),
    };
    format!(
        "{} {} {}",
        side(&l.from, &l.from_at),
        l.kind.replace('_', " "),
        side(&l.to, &l.to_at)
    )
}

/// The request for the model: the question, numbered excerpts and graph links.
pub fn prompt(root: &Path, found: &Found) -> String {
    let mut p = format!(
        "Question about this codebase: {}\n\n\
         Answer in the language of the question, in plain prose (no headings, no lists, at most about 250 words). \
         Walk through what happens step by step, following the flow between the places below. \
         Cite every claim with a path:line reference exactly as given (for example src/auth.rs:12). \
         Use only the excerpts and links below; if they do not answer the question, say which part is missing.\n\n\
         Relevant code:\n",
        found.question
    );
    for h in &found.hits {
        let Some(ex) = excerpt(root, h) else {
            continue;
        };
        let block = format!("\n### {}\n```\n{ex}```\n", label(h));
        if p.len() + block.len() > PROMPT_BUDGET {
            break;
        }
        p.push_str(&block);
    }
    if !found.links.is_empty() {
        p.push_str("\nLinks from the project graph:\n");
        for l in &found.links {
            let _ = writeln!(p, "- {}", link_line(l));
        }
    }
    p
}

/// The model's plain-prose explanation of the flow.
pub async fn explain(
    provider: &Arc<dyn Provider>,
    model: &str,
    root: &Path,
    found: &Found,
) -> Result<String, FlowError> {
    gitflow::ask(provider, model, prompt(root, found), 1200)
        .await
        .map(|t| t.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use kor_core::{ProviderError, StreamEvent};
    use kor_providers::{CancellationToken, ChatRequest, EventStream, ModelInfo, ProviderManifest};
    use std::sync::Mutex;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let w = |p: &str, t: &str| {
            let f = d.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        };
        w(
            "src/main.rs",
            "mod auth;\nmod session;\nuse crate::auth::login;\npub fn main_entry() { login(\"u\", \"p\"); }\n",
        );
        w(
            "src/auth.rs",
            "use crate::session::create_session;\n// WHY: passwords are checked before a session exists\npub fn login(user: &str, password: &str) -> bool {\n    if password.is_empty() { return false; }\n    create_session(user);\n    true\n}\n",
        );
        w(
            "src/session.rs",
            "pub fn create_session(user: &str) -> String {\n    format!(\"session for {user}\")\n}\n",
        );
        w(
            "README.md",
            "# Demo\nUsers log in with a password; a session is created.\n",
        );
        w(".env", "SECRET_LOGIN_TOKEN=abc\n");
        d
    }

    #[tokio::test]
    async fn graph_and_keywords_find_the_login_flow() {
        let d = project();
        let ctx = ToolCtx::new(d.path()).unwrap();
        let f = search(
            &ctx,
            "what happens when a user does login and gets a session",
            10,
            None,
        )
        .await
        .unwrap();
        assert_eq!(f.text_index, TextIndex::Keyword);
        assert!(f.graph_nodes > 0);
        let top: Vec<String> = f.hits.iter().take(4).map(label).collect();
        assert!(
            top.iter().any(|l| l.starts_with("src/auth.rs:3 login")),
            "{top:?}"
        );
        assert!(f.hits.iter().any(|h| h.file == "src/session.rs"), "{top:?}");
        assert!(!f.hits.iter().any(|h| h.file == ".env"));
        // The login hit is found by both rankings and merged into one entry.
        let login = f
            .hits
            .iter()
            .find(|h| h.name.as_deref() == Some("login"))
            .unwrap();
        assert!(
            login.sources.contains(&Source::Graph) && login.sources.contains(&Source::Keyword),
            "{login:?}"
        );
        let lines: Vec<String> = f.links.iter().map(link_line).collect();
        assert!(
            lines.iter().any(
                |l| l.contains("login (src/auth.rs:3) calls create_session (src/session.rs:1)")
            ),
            "{lines:?}"
        );
        let p = prompt(d.path(), &f);
        assert!(
            p.contains("### src/auth.rs:3 login (function)")
                && p.contains("    5     create_session(user);"),
            "{p}"
        );
        assert!(!p.contains("SECRET_LOGIN_TOKEN"));
        assert!(search(&ctx, " ", 5, None).await.is_err());
    }

    #[test]
    fn fusion_merges_overlapping_places_and_ranks_agreement_first() {
        let c = |file: &str, line, end, node: Option<&str>| Cand {
            file: file.into(),
            line,
            end,
            name: node.map(str::to_owned),
            kind: node.map(|_| NodeKind::Function),
            node: node.map(str::to_owned),
        };
        let hits = fuse(
            vec![
                (
                    Source::Graph,
                    vec![c("a.rs", 10, 12, Some("a")), c("b.rs", 1, 3, Some("b"))],
                ),
                (
                    Source::Keyword,
                    vec![c("b.rs", 1, 60, None), c("z.rs", 1, 9, None)],
                ),
            ],
            10,
        );
        assert_eq!(hits.len(), 3);
        assert_eq!(
            (hits[0].file.as_str(), hits[0].line, hits[0].end),
            ("b.rs", 1, 3)
        );
        assert_eq!(hits[0].sources, [Source::Graph, Source::Keyword]);
        assert_eq!(hits[0].name.as_deref(), Some("b"));
        assert_eq!(
            fuse(vec![(Source::Keyword, vec![c("x", 1, 1, None)])], 0).len(),
            0
        );
    }

    struct Mock {
        manifest: ProviderManifest,
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Provider for Mock {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            req: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, ProviderError> {
            self.seen.lock().unwrap().push(req.messages[0].plain_text());
            Ok(Box::pin(futures::stream::iter(vec![Ok(StreamEvent::TextDelta {
                text: " login (src/auth.rs:3) checks the password, then create_session (src/session.rs:1) runs. ".into(),
            })])))
        }
    }

    #[tokio::test]
    async fn the_model_gets_excerpts_and_returns_prose() {
        let d = project();
        let ctx = ToolCtx::new(d.path()).unwrap();
        let f = search(&ctx, "login session", 5, None).await.unwrap();
        let mock = Arc::new(Mock {
            manifest: kor_providers::Registry::builtin()
                .get("local")
                .unwrap()
                .clone(),
            seen: Mutex::new(Vec::new()),
        });
        let p: Arc<dyn Provider> = mock.clone();
        let text = explain(&p, "m", d.path(), &f).await.unwrap();
        assert!(text.starts_with("login (src/auth.rs:3)"));
        let sent = mock.seen.lock().unwrap();
        assert!(sent[0].contains("Question about this codebase: login session"));
        assert!(sent[0].contains("src/auth.rs:3"));
    }
}
