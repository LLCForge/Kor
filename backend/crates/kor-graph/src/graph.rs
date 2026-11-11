//! The live graph: scanning, incremental update by content hash, and the in-memory petgraph index.

use crate::community;
use crate::git::{self, GitStats};
use crate::layers::{self, MemoryDecision};
use crate::workspace::{self, Package, Workspace};
use crate::lang::Lang;
use crate::model::{Community, Edge, GraphNode, NodeId};
use crate::parse::{self, FileFacts, FileKind, MAX_FILE_BYTES};
use crate::resolve::{self, EdgeKey};
use crate::store::{FileRow, Store};
use crate::{Error, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What one `update` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStats {
    /// Files looked at.
    pub scanned: usize,
    /// Files (re)parsed because their SHA-256 changed.
    pub parsed: usize,
    /// Files whose facts came from the AST cache.
    pub cached: usize,
    pub removed: usize,
    /// True when the graph was recomputed (anything changed).
    pub changed: bool,
    pub nodes: usize,
    pub edges: usize,
    pub communities: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
struct FileEntry {
    row: FileRow,
    facts: Option<FileFacts>,
}

/// A project's graph. Open with [`Graph::open`], refresh with [`Graph::update`] or
/// [`Graph::update_files`], read with the query methods (see `query.rs`).
#[derive(Debug)]
pub struct Graph {
    pub(crate) root: PathBuf,
    store: Store,
    files: BTreeMap<String, FileEntry>,
    git: GitStats,
    pub(crate) nodes: BTreeMap<NodeId, GraphNode>,
    pub(crate) edges: BTreeMap<EdgeKey, Edge>,
    pub(crate) communities: Vec<Community>,
    pub(crate) last_update: Option<u64>,
    ignore: Gitignore,
    /// petgraph index over `nodes`/`edges`; edge weight = index into `edge_list`.
    pub(crate) pg: DiGraph<NodeId, usize>,
    pub(crate) ix: HashMap<NodeId, NodeIndex>,
    pub(crate) edge_list: Vec<Edge>,
    /// Monorepo packages (`None` until first detected).
    workspace: Option<Workspace>,
    /// Decisions from Kor's project memory (set by the caller, not persisted as input).
    memory: Vec<MemoryDecision>,
    /// Generated media and what each came from (set by the caller, media layer).
    media: Vec<layers::MediaItem>,
    /// Force a recompute on the next apply (a layer input changed).
    dirty: bool,
    /// Live database snapshots (`.kor/graph/db/*.json`, database-schema layer).
    pub(crate) db: Vec<crate::db::DbSnapshot>,
    /// Linked hosts (`.kor/graph/infra/*.json`, infrastructure layer).
    pub(crate) infra: Vec<crate::infra::InfraSnapshot>,
    /// Linked documentation pages (`.kor/graph/docs/*.json`, web-docs layer, ADR 0058).
    pub(crate) docs: Vec<crate::webdocs::DocSnapshot>,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn mtime_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn tag_of(kind: FileKind, lang: Option<Lang>) -> String {
    match (kind, lang) {
        (FileKind::Code, Some(l)) => l.name().to_owned(),
        (FileKind::Env, _) => "env".into(),
        (FileKind::Adr, _) => "adr".into(),
        (FileKind::Sql, _) => "sql".into(),
        (FileKind::Prisma, _) => "prisma".into(),
        _ => "config".into(),
    }
}

/// Directories never scanned: VCS data and our own output.
fn is_internal(rel: &str) -> bool {
    rel == ".git" || rel == ".kor" || rel.starts_with(".git/") || rel.starts_with(".kor/")
}

pub(crate) struct Job {
    pub(crate) rel: String,
    pub(crate) kind: FileKind,
    pub(crate) lang: Option<Lang>,
    pub(crate) text: String,
    pub(crate) row: FileRow,
}

/// Parse jobs on all cores. Output order matches input order.
pub(crate) fn parse_parallel(jobs: Vec<Job>) -> Vec<(Job, FileFacts)> {
    if jobs.len() < 8 {
        return jobs
            .into_iter()
            .map(|j| {
                let f = parse::parse(j.kind, j.lang, &j.text);
                (j, f)
            })
            .collect();
    }
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(16);
    let chunk = jobs.len().div_ceil(threads);
    let mut chunks: Vec<Vec<Job>> = Vec::new();
    let mut it = jobs.into_iter().peekable();
    while it.peek().is_some() {
        chunks.push(it.by_ref().take(chunk).collect());
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|c| {
                s.spawn(move || {
                    c.into_iter()
                        .map(|j| {
                            let f = parse::parse(j.kind, j.lang, &j.text);
                            (j, f)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

impl Graph {
    /// Open (or create) the graph stored under `<root>/.kor/graph/`. Does not scan; call
    /// [`Graph::update`] to bring it up to date.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = std::path::absolute(root.as_ref())?;
        if !root.is_dir() {
            return Err(Error::NotADirectory(root));
        }
        let store = Store::open(&root)?;
        let mut files = BTreeMap::new();
        for row in store.files()? {
            let facts = store.cache_read(&row.sha, &row.tag);
            files.insert(row.path.clone(), FileEntry { row, facts });
        }
        let git = store
            .meta("git")?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let last_update = store.meta("last_update")?.and_then(|s| s.parse().ok());
        let mut b = GitignoreBuilder::new(&root);
        for name in [".gitignore", ".ignore"] {
            let p = root.join(name);
            if p.is_file() {
                let _ = b.add(p);
            }
        }
        let ignore = b.build().unwrap_or_else(|_| Gitignore::empty());
        let db = crate::db::load_snapshots(&root);
        let infra = crate::infra::load_snapshots(&root);
        let docs = crate::webdocs::load_snapshots(&root);
        let mut g = Self {
            nodes: store.nodes()?,
            edges: store.edges()?,
            communities: store.communities()?,
            root,
            store,
            files,
            git,
            last_update,
            ignore,
            pg: DiGraph::new(),
            ix: HashMap::new(),
            edge_list: Vec::new(),
            workspace: None,
            memory: Vec::new(),
            media: Vec::new(),
            dirty: false,
            db,
            infra,
            docs,
        };
        g.reindex();
        Ok(g)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.values()
    }

    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.values()
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.get(id)
    }

    pub fn communities(&self) -> &[Community] {
        &self.communities
    }

    /// Monorepo packages found at the last update (`None` before the first one).
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    /// The innermost workspace package containing `file` (root-relative or absolute).
    pub fn package_of(&self, file: &str) -> Option<&Package> {
        self.workspace.as_ref()?.package_of(Path::new(file))
    }

    /// Replace the project-memory decisions (`DecidedBy` layer); recomputes only when they changed.
    pub fn set_memory_decisions(&mut self, decisions: Vec<MemoryDecision>) -> Result<UpdateStats> {
        let mut stats = UpdateStats::default();
        if decisions != self.memory {
            self.memory = decisions;
            self.dirty = true;
            self.apply(Vec::new(), &[], Vec::new(), false, false, &mut stats)?;
        }
        Ok(stats)
    }

    /// Replace the generated-media items (media layer, §20.3); recomputes only when they changed.
    pub fn set_media_items(&mut self, items: Vec<layers::MediaItem>) -> Result<UpdateStats> {
        let mut stats = UpdateStats::default();
        if items != self.media {
            self.media = items;
            self.dirty = true;
            self.apply(Vec::new(), &[], Vec::new(), false, false, &mut stats)?;
        }
        Ok(stats)
    }

    /// Recompute nodes and edges from the cached facts (a layer input changed).
    pub(crate) fn recompute(&mut self) -> Result<UpdateStats> {
        let mut stats = UpdateStats::default();
        self.dirty = true;
        self.apply(Vec::new(), &[], Vec::new(), false, false, &mut stats)?;
        Ok(stats)
    }

    /// Package and project-memory layers on top of the parsed graph.
    fn add_layers(&self, derived: &mut resolve::Derived) {
        let packages = self.workspace.as_ref().filter(|w| w.is_monorepo());
        if let Some(ws) = packages {
            layers::packages(derived, ws);
        }
        if !self.memory.is_empty() {
            layers::memory(derived, &self.memory);
        }
        if !self.infra.is_empty() {
            let name = self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            crate::infra::layer(derived, &self.infra, self.workspace.as_ref(), &name);
        }
        if !self.docs.is_empty() {
            crate::webdocs::layer(derived, &self.docs);
        }
        if !self.media.is_empty() {
            layers::media(derived, &self.media);
        }
        // Layers add edges after `derive` counted degrees; recount only when they did.
        if packages.is_some()
            || !self.memory.is_empty()
            || !self.infra.is_empty()
            || !self.docs.is_empty()
            || !self.media.is_empty()
        {
            layers::recount_degree(derived);
        }
    }

    /// Re-read the monorepo manifests; marks the graph dirty when the packages changed.
    fn refresh_workspace(&mut self) {
        let ws = workspace::detect(&self.root);
        let same = if let Some(old) = &self.workspace {
            *old == ws
        } else {
            // Freshly opened: compare with the package layer loaded from SQLite.
            let pkg = |id: &str| id.strip_prefix(layers::PACKAGE_PREFIX).map(str::to_owned);
            let stored: HashSet<String> = self.nodes.keys().filter_map(|k| pkg(k)).collect();
            let stored_deps: HashSet<(String, String)> = self
                .edges
                .values()
                .filter(|e| e.kind == crate::model::EdgeKind::DependsOn)
                .filter_map(|e| Some((pkg(&e.source)?, pkg(&e.target)?)))
                .collect();
            let names: HashSet<String> = ws.packages.iter().map(|p| p.name.clone()).collect();
            let deps: HashSet<(String, String)> = ws
                .edges()
                .into_iter()
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
                .collect();
            stored == names && stored_deps == deps
        };
        self.dirty |= !same;
        self.workspace = Some(ws);
    }

    fn reindex(&mut self) {
        let mut pg = DiGraph::with_capacity(self.nodes.len(), self.edges.len());
        let mut ix = HashMap::with_capacity(self.nodes.len());
        for id in self.nodes.keys() {
            ix.insert(id.clone(), pg.add_node(id.clone()));
        }
        let edge_list: Vec<Edge> = self.edges.values().cloned().collect();
        for (i, e) in edge_list.iter().enumerate() {
            if let (Some(a), Some(b)) = (ix.get(&e.source), ix.get(&e.target)) {
                pg.add_edge(*a, *b, i);
            }
        }
        self.pg = pg;
        self.ix = ix;
        self.edge_list = edge_list;
    }

    /// Read a candidate file; `None` when it is not a graph input (unknown type, too big, binary).
    fn job(rel: &str, abs: &Path) -> Option<Job> {
        let (kind, lang) = parse::classify(rel)?;
        let meta = std::fs::metadata(abs).ok()?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            return None;
        }
        let row = FileRow {
            path: rel.to_owned(),
            sha: String::new(),
            size: meta.len(),
            mtime: mtime_of(&meta),
            tag: tag_of(kind, lang),
        };
        Some(Job {
            rel: rel.to_owned(),
            kind,
            lang,
            text: String::new(),
            row,
        })
    }

    /// Fill sha/text; returns false when the content is unchanged (only the row is refreshed).
    fn load(&self, job: &mut Job) -> bool {
        let Ok(bytes) = std::fs::read(self.root.join(&job.rel)) else {
            return false;
        };
        job.row.sha = sha256_hex(&bytes);
        let Ok(text) = String::from_utf8(bytes) else {
            return false;
        };
        // Minified bundles: very long lines make a useless graph.
        if text.len() > 20_000 && text.len() / text.lines().count().max(1) > 1000 {
            return false;
        }
        job.text = text;
        true
    }

    /// Scan the whole project (respecting `.gitignore`), reparse files whose SHA-256 changed,
    /// drop removed files, refresh the git layer when `HEAD` moved, and persist the difference.
    pub fn update(&mut self) -> Result<UpdateStats> {
        let start = Instant::now();
        let mut seen: HashSet<String> = HashSet::new();
        let mut jobs = Vec::new();
        let mut touched_rows = Vec::new();
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .require_git(false)
            .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | ".kor")))
            .build();
        let mut stats = UpdateStats::default();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let Some(mut job) = Self::job(&rel, entry.path()) else {
                continue;
            };
            stats.scanned += 1;
            seen.insert(rel.clone());
            // Fast path: same size and mtime as last time and facts cached.
            if let Some(old) = self.files.get(&rel)
                && old.facts.is_some()
                && old.row.size == job.row.size
                && old.row.mtime == job.row.mtime
            {
                continue;
            }
            let readable = self.load(&mut job);
            if let Some(old) = self.files.get(&rel)
                && old.facts.is_some()
                && old.row.sha == job.row.sha
            {
                touched_rows.push(job.row);
                continue;
            }
            if readable {
                jobs.push(job);
            }
        }
        let removed: Vec<String> = self
            .files
            .keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        let git_changed = self.refresh_git();
        self.refresh_workspace();
        let snaps = crate::db::load_snapshots(&self.root);
        if snaps != self.db {
            self.db = snaps;
            self.dirty = true;
        }
        let infra = crate::infra::load_snapshots(&self.root);
        if infra != self.infra {
            self.infra = infra;
            self.dirty = true;
        }
        let docs = crate::webdocs::load_snapshots(&self.root);
        if docs != self.docs {
            self.docs = docs;
            self.dirty = true;
        }
        self.apply(jobs, &removed, touched_rows, git_changed, true, &mut stats)?;
        stats.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(stats)
    }

    /// Incremental update for specific paths (absolute or root-relative), e.g. from the watcher.
    /// Deleted paths drop their nodes; unchanged content is a no-op.
    pub fn update_files(&mut self, paths: &[PathBuf]) -> Result<UpdateStats> {
        let start = Instant::now();
        let mut stats = UpdateStats::default();
        let mut jobs = Vec::new();
        let mut removed = Vec::new();
        let mut touched_rows = Vec::new();
        let mut done: HashSet<String> = HashSet::new();
        for p in paths {
            let rel = if p.is_absolute() {
                p.strip_prefix(&self.root).map(Path::to_path_buf).ok()
            } else {
                Some(p.clone())
            };
            let Some(rel) = rel else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.is_empty()
                || is_internal(&rel)
                || rel.split('/').any(|s| s == "..")
                || !done.insert(rel.clone())
            {
                continue;
            }
            let abs = self.root.join(&rel);
            if !abs.exists() {
                if self.files.contains_key(&rel) {
                    removed.push(rel);
                }
                continue;
            }
            let known = self.files.contains_key(&rel);
            if !known
                && self
                    .ignore
                    .matched_path_or_any_parents(&rel, false)
                    .is_ignore()
            {
                continue;
            }
            let Some(mut job) = Self::job(&rel, &abs) else {
                if known {
                    removed.push(rel);
                }
                continue;
            };
            stats.scanned += 1;
            let readable = self.load(&mut job);
            if let Some(old) = self.files.get(&rel)
                && old.facts.is_some()
                && old.row.sha == job.row.sha
            {
                touched_rows.push(job.row);
                continue;
            }
            if readable {
                jobs.push(job);
            }
        }
        if self.workspace.is_none() || done.iter().any(|p| layers::is_manifest(p)) {
            self.refresh_workspace();
        }
        self.apply(jobs, &removed, touched_rows, false, false, &mut stats)?;
        stats.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(stats)
    }

    /// Re-read git history when `HEAD` moved. Returns true if the git layer changed.
    fn refresh_git(&mut self) -> bool {
        let head = git::head(&self.root).unwrap_or_default();
        if head == self.git.head {
            return false;
        }
        self.git = if head.is_empty() {
            GitStats::default()
        } else {
            git::stats(&self.root, now_secs()).unwrap_or_default()
        };
        true
    }

    fn apply(
        &mut self,
        jobs: Vec<Job>,
        removed: &[String],
        mut rows: Vec<FileRow>,
        git_changed: bool,
        full: bool,
        stats: &mut UpdateStats,
    ) -> Result<()> {
        // Facts first from the content-addressed cache, then parse the rest in parallel.
        let mut to_parse = Vec::new();
        for job in jobs {
            if let Some(f) = self.store.cache_read(&job.row.sha, &job.row.tag) {
                stats.cached += 1;
                rows.push(job.row.clone());
                self.files.insert(
                    job.rel,
                    FileEntry {
                        row: job.row,
                        facts: Some(f),
                    },
                );
            } else {
                to_parse.push(job);
            }
        }
        stats.parsed = to_parse.len();
        for (job, facts) in parse_parallel(to_parse) {
            self.store.cache_write(&job.row.sha, &job.row.tag, &facts)?;
            rows.push(job.row.clone());
            self.files.insert(
                job.rel,
                FileEntry {
                    row: job.row,
                    facts: Some(facts),
                },
            );
        }
        for r in removed {
            self.files.remove(r);
        }
        stats.removed = removed.len();
        for r in &rows {
            if let Some(e) = self.files.get_mut(&r.path) {
                e.row = r.clone();
            }
        }
        let content_changed =
            stats.parsed + stats.cached + stats.removed > 0 || git_changed || self.dirty;
        self.dirty = false;
        let first = self.last_update.is_none();
        stats.changed = content_changed || first;
        if stats.changed {
            let facts: BTreeMap<String, FileFacts> = self
                .files
                .iter()
                .filter_map(|(p, e)| e.facts.clone().map(|f| (p.clone(), f)))
                .collect();
            let mut derived = resolve::derive(&facts, &self.git, &self.db);
            self.add_layers(&mut derived);
            let mut nodes = derived.nodes;
            let prev: HashMap<NodeId, u16> = self
                .nodes
                .values()
                .map(|n| (n.id.clone(), n.community))
                .collect();
            let communities = community::detect(&mut nodes, &derived.edges, &prev);
            let now = now_secs();
            let mut meta = vec![
                ("last_update", now.to_string()),
                ("version", crate::VERSION.to_owned()),
            ];
            if git_changed || first {
                meta.push(("git", serde_json::to_string(&self.git)?));
            }
            self.store.save(
                &rows,
                removed,
                &self.nodes,
                &nodes,
                &self.edges,
                &derived.edges,
                &communities,
                &meta,
            )?;
            self.nodes = nodes;
            self.edges = derived.edges;
            self.communities = communities;
            self.last_update = Some(now);
            self.reindex();
        } else if !rows.is_empty() {
            self.store.save_files(&rows)?;
        }
        if full && stats.changed {
            let live: HashSet<String> = self
                .files
                .values()
                .map(|e| Store::cache_name(&e.row.sha, &e.row.tag))
                .collect();
            self.store.cache_gc(&live)?;
        }
        stats.nodes = self.nodes.len();
        stats.edges = self.edges.len();
        stats.communities = self.communities.len();
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let w = |p: &str, s: &str| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, s).expect("write");
        };
        w(".gitignore", "build/\n");
        w("build/gen.rs", "fn generated() {}\n");
        w(
            "src/lib.rs",
            "mod auth;\nmod db;\nuse crate::auth::login;\npub fn main_entry() { login(\"u\"); }\n",
        );
        w(
            "src/auth.rs",
            "use crate::db::Db;\n// WHY: tokens expire fast\npub fn login(u: &str) -> bool { let db = Db::connect(); db.find(u) }\npub fn logout() {}\n",
        );
        w(
            "src/db.rs",
            "pub struct Db;\nimpl Db {\n    pub fn connect() -> Self { let _ = std::env::var(\"DATABASE_URL\"); Db }\n    pub fn find(&self, _u: &str) -> bool { true }\n}\n",
        );
        w(".env.example", "DATABASE_URL=postgres://localhost/x\n");
        w("Cargo.toml", "[package]\nname = \"x\"\n");
        w(
            "docs/decisions/0001-db.md",
            "# One database\nUse `Db` from `src/db.rs`.\n",
        );
        dir
    }

    #[test]
    fn build_incremental_and_persist() -> Result<()> {
        let dir = fixture();
        let mut g = Graph::open(dir.path())?;
        let s = g.update()?;
        assert!(s.changed);
        assert_eq!(s.parsed, 6, "{s:?}");
        assert!(
            !g.nodes.contains_key("build/gen.rs"),
            "gitignored file indexed"
        );
        assert!(g.nodes.contains_key("src/db.rs#Db.connect"));
        assert!(g.edges.keys().any(|k| k.0 == "src/auth.rs#login"
            && k.1 == "src/db.rs#Db.connect"
            && k.2 == "calls"));
        assert!(dir.path().join(".kor/graph/graph.sqlite").is_file());
        // No change: nothing parsed.
        let s = g.update()?;
        assert_eq!((s.parsed, s.changed), (0, false));
        // Edit one file through update_files.
        std::fs::write(
            dir.path().join("src/auth.rs"),
            "use crate::db::Db;\npub fn login(u: &str) -> bool { Db::connect().find(u) }\npub fn signup() {}\n",
        )?;
        let s = g.update_files(&[dir.path().join("src/auth.rs")])?;
        assert_eq!(s.parsed, 1);
        assert!(g.nodes.contains_key("src/auth.rs#signup"));
        assert!(!g.nodes.contains_key("src/auth.rs#logout"));
        assert!(!g.nodes.contains_key("src/auth.rs#note:1"));
        // Delete a file.
        std::fs::remove_file(dir.path().join("src/db.rs"))?;
        let s = g.update_files(&[PathBuf::from("src/db.rs")])?;
        assert_eq!(s.removed, 1);
        assert!(!g.nodes.keys().any(|k| k.starts_with("src/db.rs")));
        // Reopen from SQLite: same graph.
        let (nodes, edges) = (g.nodes.clone(), g.edges.clone());
        drop(g);
        let g2 = Graph::open(dir.path())?;
        assert_eq!(g2.nodes, nodes);
        assert_eq!(g2.edges, edges);
        assert_eq!(g2.files.len(), 5);
        assert!(g2.files.values().all(|f| f.facts.is_some()));
        Ok(())
    }

    #[test]
    fn ignored_and_internal_paths_are_skipped() -> Result<()> {
        let dir = fixture();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        std::fs::write(dir.path().join("build/new.rs"), "fn x() {}\n")?;
        let s = g.update_files(&[
            dir.path().join("build/new.rs"),
            dir.path().join(".kor/graph/graph.sqlite"),
            PathBuf::from("../escape.rs"),
        ])?;
        assert_eq!((s.scanned, s.changed), (0, false));
        Ok(())
    }
}
