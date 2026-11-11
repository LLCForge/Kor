//! Read-only queries over the graph. Pure Rust, no LLM; every result is a plain serializable struct.

use crate::Result;
use crate::git;
use crate::graph::{Graph, Job, parse_parallel};
use crate::model::{Community, Confidence, EdgeKind, GraphNode, NodeKind, NodeRef};
use crate::parse::{self, MAX_FILE_BYTES};
use crate::resolve::{self, EdgeKey};
use crate::store::FileRow;
use petgraph::Direction;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;

const LIST_CAP: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub root: String,
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    /// Unix seconds of the last recompute.
    pub last_update: Option<u64>,
    /// Code files per language.
    pub languages: BTreeMap<String, usize>,
    pub nodes_by_kind: BTreeMap<String, usize>,
    pub edges_by_kind: BTreeMap<String, usize>,
}

/// A neighbour reached through one edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub node: NodeRef,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    pub node: GraphNode,
    /// Source lines of the node's span (capped).
    pub source: Option<String>,
    pub community: Option<Community>,
    /// Incoming edges (who calls / imports / implements / reads this).
    pub callers: Vec<Link>,
    /// Outgoing edges (what this calls / imports / reads).
    pub callees: Vec<Link>,
    /// Files that change together with this node's file.
    pub co_changes: Vec<Link>,
    /// Rationale: `WHY:`/`NOTE:` comments and ADRs.
    pub decisions: Vec<NodeRef>,
    /// Other nodes that matched the name.
    pub alternatives: Vec<NodeRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathStep {
    pub node: NodeRef,
    /// Edge from the previous step; `forward` is false when it points back to the previous node.
    pub edge: Option<EdgeKind>,
    pub forward: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryHit {
    pub node: NodeRef,
    pub score: f32,
    pub matched: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hotspot {
    pub node: NodeRef,
    pub degree: u32,
    pub churn_30d: u16,
    /// `degree × (churn_30d + 1)`; `+1` keeps the ranking useful without git history.
    pub score: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactEntry {
    pub node: NodeRef,
    pub depth: u32,
    pub via: EdgeKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Impact {
    pub roots: Vec<NodeRef>,
    pub affected: Vec<ImpactEntry>,
    /// Distinct files of the affected nodes.
    pub files: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeChange {
    pub source: String,
    pub target: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphDiff {
    pub rev: String,
    pub added_nodes: Vec<NodeRef>,
    pub removed_nodes: Vec<NodeRef>,
    pub added_edges: Vec<EdgeChange>,
    pub removed_edges: Vec<EdgeChange>,
}

const STOPWORDS: [&str; 30] = [
    "the", "an", "of", "in", "is", "to", "and", "or", "for", "on", "where", "how", "what", "which",
    "does", "do", "it", "be", "are", "ne", "nerede", "nasıl", "bir", "ve", "ile", "mi", "da", "de",
    "bu", "için",
];

/// Lowercase tokens; identifiers are split on `_`, `-`, `.`, `/`, `::` and camelCase.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let mut cur = String::new();
        let mut prev_lower = false;
        for c in word.chars() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur).to_lowercase());
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur.to_lowercase());
        }
    }
    out.retain(|t| t.chars().count() >= 2 && !STOPWORDS.contains(&t.as_str()));
    out
}

impl Graph {
    fn nref(&self, ix: NodeIndex) -> Option<NodeRef> {
        self.nodes.get(&self.pg[ix]).map(NodeRef::from)
    }

    pub fn status(&self) -> Status {
        let mut languages = BTreeMap::new();
        let mut nodes_by_kind = BTreeMap::new();
        let mut files = 0;
        for n in self.nodes.values() {
            *nodes_by_kind.entry(n.kind.as_str().to_owned()).or_insert(0) += 1;
            if n.kind == NodeKind::Module
                || (n.kind == NodeKind::Config
                    && n.file.as_ref().is_some_and(|f| f.to_string_lossy() == n.id))
            {
                files += 1;
                if !n.text.is_empty() {
                    *languages.entry(n.text.clone()).or_insert(0) += 1;
                }
            }
        }
        let mut edges_by_kind = BTreeMap::new();
        for e in self.edges.values() {
            *edges_by_kind.entry(e.kind.as_str().to_owned()).or_insert(0) += 1;
        }
        Status {
            root: self.root.to_string_lossy().into_owned(),
            files,
            nodes: self.nodes.len(),
            edges: self.edges.len(),
            communities: self.communities.len(),
            last_update: self.last_update,
            languages,
            nodes_by_kind,
            edges_by_kind,
        }
    }

    /// Nodes matching `name`, best first: id, exact name, file path, case-insensitive name, substring.
    pub fn find(&self, name: &str) -> Vec<&GraphNode> {
        let name = name.trim();
        if name.is_empty() {
            return Vec::new();
        }
        if let Some(n) = self.nodes.get(name) {
            return vec![n];
        }
        let lower = name.to_lowercase();
        let path_like = name.replace('\\', "/");
        let mut scored: Vec<(u8, &GraphNode)> = self
            .nodes
            .values()
            .filter_map(|n| {
                let rank = if n.name == name {
                    0
                } else if n.kind == NodeKind::Module
                    && (n.id == path_like || n.id.ends_with(&format!("/{path_like}")))
                {
                    1
                } else if n.name.to_lowercase() == lower {
                    2
                } else if n.id.ends_with(&format!(".{name}")) || n.id.ends_with(&format!("#{name}"))
                {
                    3
                } else if n.kind != NodeKind::Decision && n.name.to_lowercase().contains(&lower) {
                    4
                } else {
                    return None;
                };
                Some((rank, n))
            })
            .collect();
        let kind_rank = |k: NodeKind| match k {
            NodeKind::Struct | NodeKind::Trait => 0,
            NodeKind::Function => 1,
            NodeKind::Module => 2,
            _ => 3,
        };
        scored.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(kind_rank(a.1.kind).cmp(&kind_rank(b.1.kind)))
                .then(b.1.degree.cmp(&a.1.degree))
                .then(a.1.id.cmp(&b.1.id))
        });
        scored.into_iter().map(|s| s.1).collect()
    }

    fn links(&self, ix: NodeIndex, dir: Direction, keep: impl Fn(EdgeKind) -> bool) -> Vec<Link> {
        let mut out: Vec<Link> = self
            .pg
            .edges_directed(ix, dir)
            .filter_map(|e| {
                let edge = self.edge_list.get(*e.weight())?;
                let other = if dir == Direction::Outgoing {
                    e.target()
                } else {
                    e.source()
                };
                if !keep(edge.kind) {
                    return None;
                }
                Some(Link {
                    node: self.nref(other)?,
                    kind: edge.kind,
                    confidence: edge.confidence,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.confidence
                .score()
                .total_cmp(&a.confidence.score())
                .then(a.node.id.cmp(&b.node.id))
        });
        out.truncate(LIST_CAP);
        out
    }

    fn source_of(&self, n: &GraphNode) -> Option<String> {
        let (file, (start, end)) = (n.file.as_ref()?, n.span?);
        let text = std::fs::read_to_string(self.root.join(file)).ok()?;
        let end = end.min(start.saturating_add(59));
        let lines: Vec<&str> = text
            .lines()
            .skip(start.saturating_sub(1) as usize)
            .take((end + 1).saturating_sub(start) as usize)
            .collect();
        Some(lines.join("\n"))
    }

    /// Describe the best node matching `name`.
    pub fn explain(&self, name: &str) -> Option<Explanation> {
        let found = self.find(name);
        let node = (*found.first()?).clone();
        let ix = *self.ix.get(&node.id)?;
        let is_rationale = |k: EdgeKind| k == EdgeKind::DecidedBy || k == EdgeKind::DocumentedBy;
        let incoming = self.links(ix, Direction::Incoming, |k| {
            k != EdgeKind::Contains && !is_rationale(k) && !matches!(k, EdgeKind::CoChanges { .. })
        });
        let outgoing = self.links(ix, Direction::Outgoing, |k| {
            k != EdgeKind::Contains && !is_rationale(k) && !matches!(k, EdgeKind::CoChanges { .. })
        });
        let decisions = self
            .links(ix, Direction::Outgoing, is_rationale)
            .into_iter()
            .map(|l| l.node)
            .collect();
        let module = node
            .file
            .as_ref()
            .map(|f| f.to_string_lossy().replace('\\', "/"));
        let mut co_changes = Vec::new();
        if let Some(mix) = module.as_ref().and_then(|m| self.ix.get(m)) {
            let co = |k: EdgeKind| matches!(k, EdgeKind::CoChanges { .. });
            co_changes = self.links(*mix, Direction::Outgoing, co);
            co_changes.extend(self.links(*mix, Direction::Incoming, co));
            co_changes.sort_by(|a, b| b.confidence.score().total_cmp(&a.confidence.score()));
        }
        Some(Explanation {
            source: self.source_of(&node),
            community: self
                .communities
                .iter()
                .find(|c| c.id == node.community)
                .cloned(),
            callers: incoming,
            callees: outgoing,
            co_changes,
            decisions,
            alternatives: found
                .iter()
                .skip(1)
                .take(10)
                .map(|n| NodeRef::from(*n))
                .collect(),
            node,
        })
    }

    /// Shortest path between two concepts, following edges in either direction.
    pub fn path(&self, from: &str, to: &str) -> Option<Vec<PathStep>> {
        let sources: Vec<NodeIndex> = self
            .find(from)
            .iter()
            .take(5)
            .filter_map(|n| self.ix.get(&n.id).copied())
            .collect();
        let targets: HashSet<NodeIndex> = self
            .find(to)
            .iter()
            .take(5)
            .filter_map(|n| self.ix.get(&n.id).copied())
            .collect();
        if sources.is_empty() || targets.is_empty() {
            return None;
        }
        let mut prev: HashMap<NodeIndex, Option<(NodeIndex, usize, bool)>> =
            sources.iter().map(|s| (*s, None)).collect();
        let mut queue: VecDeque<NodeIndex> = sources.iter().copied().collect();
        let mut hit = sources.iter().copied().find(|s| targets.contains(s));
        while hit.is_none() {
            let Some(cur) = queue.pop_front() else { break };
            let outgoing = self
                .pg
                .edges_directed(cur, Direction::Outgoing)
                .map(|e| (e.target(), *e.weight(), true));
            let incoming = self
                .pg
                .edges_directed(cur, Direction::Incoming)
                .map(|e| (e.source(), *e.weight(), false));
            for (next, w, fwd) in outgoing.chain(incoming) {
                if prev.contains_key(&next) {
                    continue;
                }
                prev.insert(next, Some((cur, w, fwd)));
                if targets.contains(&next) {
                    hit = Some(next);
                    break;
                }
                queue.push_back(next);
            }
        }
        let mut steps = Vec::new();
        let mut cur = hit?;
        loop {
            let link = prev.get(&cur).copied().flatten();
            steps.push(PathStep {
                node: self.nref(cur)?,
                edge: link.and_then(|(_, w, _)| self.edge_list.get(w).map(|e| e.kind)),
                forward: link.is_none_or(|l| l.2),
            });
            match link {
                Some((p, _, _)) => cur = p,
                None => break,
            }
        }
        steps.reverse();
        Some(steps)
    }

    /// Keyword / identifier search over names, paths and rationale text.
    pub fn query(&self, text: &str, limit: usize) -> Vec<QueryHit> {
        let qt: BTreeSet<String> = tokens(text).into_iter().collect();
        if qt.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<QueryHit> = self
            .nodes
            .values()
            .filter_map(|n| {
                let name_tokens: HashSet<String> = tokens(&n.name).into_iter().collect();
                let name_l = n.name.to_lowercase();
                let path_l = n
                    .file
                    .as_ref()
                    .map(|f| f.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let text_l = n.text.to_lowercase();
                let mut score = 0.0f32;
                let mut matched = Vec::new();
                for t in &qt {
                    let s = if name_tokens.contains(t) {
                        3.0
                    } else if name_l.contains(t.as_str()) {
                        2.0
                    } else if !text_l.is_empty() && text_l.contains(t.as_str()) {
                        1.5
                    } else if path_l.contains(t.as_str()) {
                        1.0
                    } else {
                        0.0
                    };
                    if s > 0.0 {
                        score += s;
                        matched.push(t.clone());
                    }
                }
                #[allow(clippy::cast_precision_loss)]
                let boost = 1.0 + (1.0 + n.degree as f32).ln() * 0.1;
                (score > 0.0).then(|| QueryHit {
                    node: NodeRef::from(n),
                    score: score * boost,
                    matched,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.node.id.cmp(&b.node.id)));
        hits.truncate(limit);
        hits
    }

    /// Highest `degree × (churn + 1)` code nodes.
    pub fn hotspots(&self, limit: usize) -> Vec<Hotspot> {
        let mut out: Vec<Hotspot> = self
            .nodes
            .values()
            .filter(|n| {
                matches!(
                    n.kind,
                    NodeKind::Function | NodeKind::Struct | NodeKind::Trait | NodeKind::Module
                ) && n.degree > 0
            })
            .map(|n| Hotspot {
                node: NodeRef::from(n),
                degree: n.degree,
                churn_30d: n.churn_30d,
                score: u64::from(n.degree) * (u64::from(n.churn_30d) + 1),
            })
            .collect();
        out.sort_by(|a, b| b.score.cmp(&a.score).then(a.node.id.cmp(&b.node.id)));
        out.truncate(limit);
        out
    }

    /// Everything that (transitively) depends on `target` — a symbol name or a file path — up to
    /// `max_depth` hops over calls/imports/implements/extends/reads edges.
    pub fn impact(&self, target: &str, max_depth: u32) -> Impact {
        const CAP: usize = 500;
        let found = self.find(target);
        let roots: Vec<&GraphNode> = match found.first() {
            Some(m) if m.kind == NodeKind::Module => {
                let file = m.id.clone();
                let mut r: Vec<&GraphNode> = vec![m];
                r.extend(self.nodes.values().filter(|n| {
                    n.id != file && n.file.as_ref().is_some_and(|f| f.to_string_lossy() == file)
                }));
                r
            }
            Some(n) => vec![n],
            None => Vec::new(),
        };
        let mut seen: HashSet<NodeIndex> = roots
            .iter()
            .filter_map(|n| self.ix.get(&n.id).copied())
            .collect();
        let mut queue: VecDeque<(NodeIndex, u32)> = seen.iter().map(|i| (*i, 0)).collect();
        let mut affected = Vec::new();
        let mut truncated = false;
        while let Some((cur, d)) = queue.pop_front() {
            if d >= max_depth {
                continue;
            }
            for e in self.pg.edges_directed(cur, Direction::Incoming) {
                let Some(edge) = self.edge_list.get(*e.weight()) else {
                    continue;
                };
                if !edge.kind.is_dependency() || !seen.insert(e.source()) {
                    continue;
                }
                if affected.len() >= CAP {
                    truncated = true;
                    break;
                }
                if let Some(node) = self.nref(e.source()) {
                    affected.push(ImpactEntry {
                        node,
                        depth: d + 1,
                        via: edge.kind,
                    });
                }
                queue.push_back((e.source(), d + 1));
            }
        }
        let files: BTreeSet<String> = affected
            .iter()
            .filter_map(|a| a.node.file.clone())
            .collect();
        Impact {
            roots: roots.into_iter().map(NodeRef::from).collect(),
            affected,
            files: files.into_iter().collect(),
            truncated,
        }
    }
}

/// Graph at git revision `rev` versus the working tree now (co-change edges excluded on both sides).
pub fn diff(root: impl AsRef<Path>, rev: &str) -> Result<GraphDiff> {
    let root = root.as_ref();
    git::validate_rev(rev)?;
    let mut now = Graph::open(root)?;
    now.update()?;
    let tree: Vec<(String, u64, String)> = git::tree(root, rev)?
        .into_iter()
        .filter(|(p, size, _)| *size <= MAX_FILE_BYTES && parse::classify(p).is_some())
        .collect();
    let ids: Vec<String> = tree.iter().map(|t| t.2.clone()).collect();
    let contents = git::blobs(root, &ids)?;
    let jobs: Vec<Job> = tree
        .iter()
        .zip(contents)
        .filter_map(|((path, _, _), bytes)| {
            let (kind, lang) = parse::classify(path)?;
            let text = String::from_utf8(bytes).ok()?;
            let row = FileRow {
                path: path.clone(),
                sha: String::new(),
                size: 0,
                mtime: 0,
                tag: String::new(),
            };
            Some(Job {
                rel: path.clone(),
                kind,
                lang,
                text,
                row,
            })
        })
        .collect();
    let facts: BTreeMap<String, parse::FileFacts> = parse_parallel(jobs)
        .into_iter()
        .map(|(j, f)| (j.rel, f))
        .collect();
    // Live database snapshots are not versioned: the same ones on both sides.
    let old = resolve::derive(&facts, &git::GitStats::default(), now.db_snapshots());
    let not_co = |k: &&EdgeKey| k.2 != "co_changes";
    let change = |k: &EdgeKey| EdgeChange {
        source: k.0.clone(),
        target: k.1.clone(),
        kind: k.2.to_owned(),
    };
    Ok(GraphDiff {
        rev: rev.to_owned(),
        added_nodes: now
            .nodes
            .values()
            .filter(|n| !old.nodes.contains_key(&n.id))
            .map(NodeRef::from)
            .collect(),
        removed_nodes: old
            .nodes
            .values()
            .filter(|n| !now.nodes.contains_key(&n.id))
            .map(NodeRef::from)
            .collect(),
        added_edges: now
            .edges
            .keys()
            .filter(not_co)
            .filter(|k| !old.edges.contains_key(*k))
            .map(change)
            .collect(),
        removed_edges: old
            .edges
            .keys()
            .filter(not_co)
            .filter(|k| !now.edges.contains_key(*k))
            .map(change)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::fixture;

    #[test]
    fn tokenizer() {
        assert_eq!(
            tokens("where is parseHTTPRequest logged?"),
            vec!["parse", "httprequest", "logged"]
        );
        assert_eq!(tokens("db_connect::Find"), vec!["db", "connect", "find"]);
    }

    #[test]
    fn queries_on_fixture() -> Result<()> {
        let dir = fixture();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        let st = g.status();
        assert_eq!(st.files, 5);
        assert_eq!(st.languages.get("rust"), Some(&3));
        assert!(st.communities >= 1);

        let ex = g.explain("login").expect("login");
        assert_eq!(ex.node.id, "src/auth.rs#login");
        assert!(
            ex.callers
                .iter()
                .any(|l| l.node.id == "src/lib.rs#main_entry" && l.kind == EdgeKind::Calls)
        );
        assert!(
            ex.callees
                .iter()
                .any(|l| l.node.id == "src/db.rs#Db.connect")
        );
        assert!(ex.decisions.is_empty());
        assert!(
            ex.source
                .as_deref()
                .is_some_and(|s| s.contains("pub fn login"))
        );
        assert!(ex.community.is_some());
        let ex = g.explain("src/auth.rs").expect("module");
        assert_eq!(ex.decisions.len(), 1, "{:?}", ex.decisions);

        let p = g.path("main_entry", "DATABASE_URL").expect("path");
        let names: Vec<&str> = p.iter().map(|s| s.node.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["main_entry", "login", "connect", "DATABASE_URL"]
        );
        assert_eq!(p[1].edge, Some(EdgeKind::Calls));
        assert!(g.path("main_entry", "nonexistent_zz").is_none());

        let hits = g.query("database connect", 5);
        assert!(
            hits.iter()
                .take(3)
                .any(|h| h.node.id == "src/db.rs#Db.connect"),
            "{hits:?}"
        );
        assert!(hits.iter().all(|h| !h.matched.is_empty()));
        assert!(
            g.query("tokens expire", 5)
                .iter()
                .any(|h| h.node.kind == NodeKind::Decision)
        );

        let hot = g.hotspots(3);
        assert_eq!(hot.len(), 3);
        assert!(hot[0].score >= hot[2].score);

        let imp = g.impact("connect", 5);
        let ids: Vec<&str> = imp.affected.iter().map(|a| a.node.id.as_str()).collect();
        assert!(
            ids.contains(&"src/auth.rs#login") && ids.contains(&"src/lib.rs#main_entry"),
            "{ids:?}"
        );
        assert_eq!(
            imp.affected
                .iter()
                .find(|a| a.node.id == "src/lib.rs#main_entry")
                .map(|a| a.depth),
            Some(2)
        );
        let imp = g.impact("src/db.rs", 1);
        assert!(imp.roots.len() > 1);
        assert!(imp.files.contains(&"src/auth.rs".to_owned()));
        Ok(())
    }

    #[test]
    fn diff_against_git_revision() -> Result<()> {
        let dir = fixture();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !run(&["init", "-q"]) {
            return Ok(()); // git not installed: nothing to test
        }
        assert!(run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "add",
            "-A"
        ]));
        assert!(run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "init"
        ]));
        std::fs::write(
            dir.path().join("src/auth.rs"),
            "use crate::db::Db;\npub fn login(u: &str) -> bool { Db::connect().find(u) }\npub fn signup() {}\n",
        )?;
        let d = diff(dir.path(), "HEAD")?;
        assert!(
            d.added_nodes.iter().any(|n| n.id == "src/auth.rs#signup"),
            "{d:?}"
        );
        assert!(d.removed_nodes.iter().any(|n| n.id == "src/auth.rs#logout"));
        assert!(
            d.removed_edges
                .iter()
                .any(|e| e.target == "src/auth.rs#note:1")
        );
        assert!(!d.added_nodes.iter().any(|n| n.id == "src/db.rs#Db"));
        assert!(diff(dir.path(), "--help").is_err());
        Ok(())
    }
}
