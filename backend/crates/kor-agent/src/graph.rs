//! Project graph for the agent (spec §7.5): `graph_query`, `graph_explain`, `graph_path` and
//! `graph_impact` answer from `kor_graph` so the model reads 4 files instead of 40.
//!
//! One [`Session`] per agent session: the graph is opened and brought up to date on a background
//! thread when the session starts ([`Session::warm`]; the first build of a large repository takes
//! about a second), then kept current by the file watcher (`[graph] watch`) and by the agent's own
//! writes ([`Tracked`] calls `update_files` after every successful edit). Strict mode
//! (`[graph] strict`) refuses the session's first raw `Read` once and points at the graph tools.
//! Front ends find the session of a project with [`find`] and subscribe with [`Session::listen`]
//! (graph updated, nodes the agent touched).

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_graph::{Confidence, Graph, Link, MemoryDecision, NodeRef, UpdateStats, WatchHandle};
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};

/// System prompt section added when the graph tools are on (spec §7.5).
pub const SYSTEM_RULE: &str = "\n# Project graph\nBefore reading a file, get its context from the graph; open only the relevant files. `graph_query` finds where something lives, `graph_explain` shows a symbol or file with its source, callers, callees and decisions, `graph_path` links two concepts, `graph_impact` lists what a change breaks.\n";

/// Returned instead of the first raw read in strict mode.
pub const STRICT_HINT: &str = "Strict graph mode: this read was not run. Get the context from the project graph first (`graph_query` with what you are looking for, `graph_explain` with a file or symbol), then read only the files you need. Reading again is allowed.";

/// The four tool names.
pub const TOOL_NAMES: [&str; 4] = ["graph_query", "graph_explain", "graph_path", "graph_impact"];

const MAX_QUERY_LEN: usize = 500;
const MAX_LIST: usize = 15;

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Refuse the first raw `Read` once (`[graph] strict`).
    pub strict: bool,
    /// Keep the graph updated with the file watcher (`[graph] watch`).
    pub watch: bool,
    /// Kor home, for project-memory decisions, `hotspots.md` and linked projects.
    pub home: Option<PathBuf>,
    /// `[auto] hotspots = false`: do not write `hotspots.md` (§28.14).
    pub skip_hotspots: bool,
}

/// What front ends hear from a session.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The graph changed (first build, watcher, the agent's own write).
    Updated(UpdateStats),
    /// Root-relative files the agent read or changed just now.
    Touched(Vec<String>),
}

pub type Listener = Arc<dyn Fn(&Event) + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn emit(l: &Mutex<Option<Listener>>, ev: &Event) {
    let f = lock(l).clone();
    if let Some(f) = f {
        f(ev);
    }
}

/// The project graph of one agent session.
pub struct Session {
    root: PathBuf,
    opts: Options,
    graph: Mutex<Option<Arc<Mutex<Graph>>>>,
    watch: Mutex<Option<WatchHandle>>,
    linked: Mutex<HashMap<PathBuf, Arc<Mutex<Graph>>>>,
    strict_pending: AtomicBool,
    active: AtomicBool,
    touched: Mutex<BTreeSet<String>>,
    listener: Arc<Mutex<Option<Listener>>>,
    /// `kor-media/` modification time the media layer was built from.
    media_seen: Mutex<Option<std::time::SystemTime>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("root", &self.root)
            .field("opts", &self.opts)
            .field("built", &lock(&self.graph).is_some())
            .finish_non_exhaustive()
    }
}

static SESSIONS: Mutex<Vec<Weak<Session>>> = Mutex::new(Vec::new());

/// The live graph session of the project at `root` (newest first), if an agent has one open.
pub fn find(root: &Path) -> Option<Arc<Session>> {
    let mut all = lock(&SESSIONS);
    all.retain(|w| w.strong_count() > 0);
    all.iter()
        .rev()
        .filter_map(Weak::upgrade)
        .find(|s| s.root == root)
}

impl Session {
    pub fn new(root: &Path, opts: Options) -> Arc<Self> {
        let s = Arc::new(Self {
            root: root.to_path_buf(),
            strict_pending: AtomicBool::new(opts.strict),
            opts,
            graph: Mutex::new(None),
            media_seen: Mutex::new(None),
            watch: Mutex::new(None),
            linked: Mutex::new(HashMap::new()),
            active: AtomicBool::new(true),
            touched: Mutex::new(BTreeSet::new()),
            listener: Arc::new(Mutex::new(None)),
        });
        lock(&SESSIONS).push(Arc::downgrade(&s));
        s
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Build the graph on a background thread so the first tool call does not wait.
    pub fn warm(self: &Arc<Self>) {
        let s = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("kor-graph-open".into())
            .spawn(move || {
                let _ = s.graph();
            });
    }

    /// Subscribe a front end (TUI panel, desktop bridge); replaces the previous listener.
    pub fn listen(&self, f: Listener) {
        *lock(&self.listener) = Some(f);
    }

    /// Files the agent read or changed this session (root-relative).
    pub fn touched(&self) -> Vec<String> {
        lock(&self.touched).iter().cloned().collect()
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Turn the graph tools off (`kor bench --graph off`): wrapped tools pass through untouched
    /// and the graph tools refuse.
    pub fn set_active(&self, on: bool) {
        self.active.store(on, Ordering::Relaxed);
    }

    /// True once per session in strict mode: the caller blocks this read.
    pub fn take_strict(&self) -> bool {
        self.strict_pending.swap(false, Ordering::Relaxed)
    }

    /// The graph when it is already built; `None` while the first build runs (never blocks, for
    /// UI loops).
    pub fn ready(&self) -> Option<Arc<Mutex<Graph>>> {
        self.graph.try_lock().ok()?.clone()
    }

    /// Media layer (§20.3): the gallery's derivation chain, re-read when `kor-media/` changed.
    fn sync_media(&self, g: &mut Graph) {
        let dir = crate::gallery::dir(&self.root);
        let mtime = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
        let mut seen = lock(&self.media_seen);
        if mtime.is_none() || *seen == mtime {
            return;
        }
        *seen = mtime;
        let items = crate::gallery::list(&self.root)
            .into_iter()
            .map(|i| kor_graph::MediaItem {
                text: format!("{} {} {}", i.prompt, i.provider, i.model)
                    .trim()
                    .to_owned(),
                id: i.id,
                file: i.file,
                inputs: i.inputs,
            })
            .collect();
        let _ = g.set_media_items(items);
    }

    /// The up-to-date graph, opened and built on first use (later calls return it at once).
    pub fn graph(&self) -> Result<Arc<Mutex<Graph>>, String> {
        let mut slot = lock(&self.graph);
        if let Some(g) = &*slot {
            self.sync_media(&mut lock(g));
            return Ok(Arc::clone(g));
        }
        let mut g = Graph::open(&self.root).map_err(|e| e.to_string())?;
        let stats = g.update().map_err(|e| e.to_string())?;
        self.sync_media(&mut g);
        if let Some(home) = &self.opts.home {
            let decisions = kor_memory::decisions(home, &self.root)
                .into_iter()
                .map(|d| MemoryDecision {
                    title: d.title,
                    body: d.body,
                    refs: d.refs,
                })
                .collect();
            let _ = g.set_memory_decisions(decisions);
            if !self.opts.skip_hotspots {
                write_hotspots(home, &self.root, &g);
            }
        }
        let g = Arc::new(Mutex::new(g));
        if self.opts.watch {
            let listener = Arc::clone(&self.listener);
            // No watcher (inotify limit, network drive): the agent's own writes still update it.
            if let Ok(h) = kor_graph::watch_graph(Arc::clone(&g), move |r| {
                if let Ok(s) = r {
                    emit(&listener, &Event::Updated(s));
                }
            }) {
                *lock(&self.watch) = Some(h);
            }
        }
        emit(&self.listener, &Event::Updated(stats));
        *slot = Some(Arc::clone(&g));
        Ok(g)
    }

    /// The graph of a project linked with `kor context add` (`project`: its path or folder name).
    fn linked_graph(&self, project: &str) -> Result<Arc<Mutex<Graph>>, String> {
        let home = self
            .opts
            .home
            .as_ref()
            .ok_or("no linked projects: Kor home is unknown")?;
        let links = kor_memory::linked_projects(home, &self.root);
        let want = project.trim().replace('\\', "/");
        let path = links
            .iter()
            .find(|p| {
                p.to_string_lossy().replace('\\', "/") == want
                    || p.file_name()
                        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&want))
            })
            .ok_or_else(|| {
                let names: Vec<String> = links
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                format!(
                    "`{want}` is not a linked project (linked: {})",
                    if names.is_empty() {
                        "none; link one with `kor context add <path>`".to_owned()
                    } else {
                        names.join(", ")
                    }
                )
            })?;
        let mut cache = lock(&self.linked);
        if let Some(g) = cache.get(path) {
            return Ok(Arc::clone(g));
        }
        let mut g = Graph::open(path).map_err(|e| e.to_string())?;
        g.update().map_err(|e| e.to_string())?;
        let g = Arc::new(Mutex::new(g));
        cache.insert(path.clone(), Arc::clone(&g));
        Ok(g)
    }

    /// Bring the graph up to date for root-relative paths the agent just changed. Nothing to do
    /// before the first build (the build reads them anyway).
    pub fn after_write(&self, rel: &[String]) {
        let Some(g) = lock(&self.graph).clone() else {
            return;
        };
        let paths: Vec<PathBuf> = rel.iter().map(PathBuf::from).collect();
        let res = lock(&g).update_files(&paths);
        if let Ok(s) = res
            && s.changed
        {
            emit(&self.listener, &Event::Updated(s));
        }
    }

    fn touch(&self, rel: Vec<String>) {
        if rel.is_empty() {
            return;
        }
        lock(&self.touched).extend(rel.iter().cloned());
        emit(&self.listener, &Event::Touched(rel));
    }
}

/// `hotspots.md` of the project memory from the graph's file-level hotspots.
fn write_hotspots(home: &Path, root: &Path, g: &Graph) {
    let mut seen = BTreeSet::new();
    let rows: Vec<kor_memory::Hotspot> = g
        .hotspots(60)
        .into_iter()
        .filter_map(|h| {
            let file = h.node.file.clone()?;
            seen.insert(file.clone()).then(|| kor_memory::Hotspot {
                path: file,
                #[allow(clippy::cast_precision_loss)] // display only
                score: h.score as f64,
                reason: format!("degree {}, churn {} (30 d)", h.degree, h.churn_30d),
            })
        })
        .take(20)
        .collect();
    if !rows.is_empty() {
        let _ = kor_memory::ProjectMemory::new(home, root).write_hotspots(&rows);
    }
}

/// Files ranked by hotspot score from the stored graph, for the repository map; `None` when the
/// project has no graph yet. Never builds anything (session start must stay fast).
pub fn ranked_files(root: &Path) -> Option<Vec<String>> {
    kor_graph::ranked_files(root, 5000).filter(|v| !v.is_empty())
}

/// `kor commit` scope (spec §15.5) from the graph: the one workspace package or the one graph
/// community every changed file belongs to. `None` without a stored graph or when they differ.
pub fn commit_scope(root: &Path, files: &[String]) -> Option<String> {
    if files.is_empty()
        || !kor_graph::store::graph_dir(root)
            .join("graph.sqlite")
            .is_file()
    {
        return None;
    }
    let ws = kor_graph::workspace::detect(root);
    if ws.is_monorepo() {
        let pkgs: BTreeSet<&str> = files
            .iter()
            .map(|f| ws.package_of(Path::new(f)).map(|p| p.name.as_str()))
            .collect::<Option<_>>()?;
        if pkgs.len() == 1 {
            return pkgs.into_iter().next().and_then(short_scope);
        }
    }
    let g = Graph::open(root).ok()?;
    let ids: BTreeSet<u16> = files
        .iter()
        .map(|f| g.node(f).map(|n| n.community))
        .collect::<Option<_>>()?;
    if ids.len() != 1 {
        return None;
    }
    let id = ids.into_iter().next()?;
    let label = &g.communities().iter().find(|c| c.id == id)?.label;
    short_scope(label.split(':').next().unwrap_or(label))
}

/// §15.5: a public symbol a diff removes or re-declares, and the files outside the change that
/// still depend on it according to the project graph (`graph_impact`, one hop).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApiBreak {
    pub symbol: String,
    pub file: String,
    pub users: Vec<String>,
}

/// Public declarations (Rust `pub`, TS/JS `export`, top-level Python, exported Go) whose line a
/// unified diff removes without adding the same line back: removed, renamed or re-signed.
pub fn changed_public_symbols(diff: &str) -> Vec<(String, String)> {
    use std::sync::LazyLock;
    static DECL: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
        [
            r#"^\s*pub\s+(?:(?:async|const|unsafe)\s+|extern\s+"[^"]*"\s+)*(?:fn|struct|enum|trait|type|const|static|mod|union)\s+([A-Za-z_]\w*)"#,
            r"^\s*export\s+(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|class|const|let|var|interface|type|enum)\s+([A-Za-z_$][\w$]*)",
            r"^(?:async\s+)?(?:def|class)\s+([A-Za-z]\w*)",
            r"^func\s+(?:\([^)]*\)\s*)?([A-Z]\w*)",
            r"^type\s+([A-Z]\w*)\s",
        ]
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
    });
    let decl = |line: &str| {
        DECL.iter().find_map(|r| {
            r.captures(line)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_owned())
        })
    };
    let mut out = Vec::new();
    let mut flush =
        |file: &Option<String>, removed: &mut Vec<(String, String)>, added: &mut Vec<String>| {
            if let Some(f) = file {
                for (name, line) in removed.drain(..) {
                    if !added.contains(&line)
                        && !out
                            .iter()
                            .any(|(of, on): &(String, String)| of == f && *on == name)
                    {
                        out.push((f.clone(), name));
                    }
                }
            }
            removed.clear();
            added.clear();
        };
    let (mut file, mut removed, mut added) = (None, Vec::new(), Vec::new());
    for l in diff.lines() {
        if let Some(p) = l.strip_prefix("+++ ") {
            flush(&file, &mut removed, &mut added);
            file = p.strip_prefix("b/").map(str::to_owned);
        } else if l.starts_with("--- ") || l.starts_with("diff --git") {
        } else if let Some(t) = l.strip_prefix('-') {
            if let Some(n) = decl(t) {
                removed.push((n, t.trim().to_owned()));
            }
        } else if let Some(t) = l.strip_prefix('+') {
            added.push(t.trim().to_owned());
        }
    }
    flush(&file, &mut removed, &mut added);
    out
}

/// [`changed_public_symbols`] that the stored graph knows, with their users outside `changed`.
/// The graph is read as stored (not updated), so a symbol the diff deletes is still found.
pub fn api_breaks(root: &Path, changed: &[String], diff: &str) -> Vec<ApiBreak> {
    let decls = changed_public_symbols(diff);
    if decls.is_empty()
        || !kor_graph::store::graph_dir(root)
            .join("graph.sqlite")
            .is_file()
    {
        return Vec::new();
    }
    let Ok(g) = Graph::open(root) else {
        return Vec::new();
    };
    decls
        .into_iter()
        .filter_map(|(file, symbol)| {
            let node = g.find(&symbol).into_iter().find(|n| {
                n.name == symbol
                    && n.file
                        .as_ref()
                        .is_some_and(|f| f.to_string_lossy().replace('\\', "/") == file)
            })?;
            let users: Vec<String> = g
                .impact(&node.id, 1)
                .files
                .into_iter()
                .filter(|f| *f != file && !changed.contains(f))
                .collect();
            (!users.is_empty()).then_some(ApiBreak {
                symbol,
                file,
                users,
            })
        })
        .collect()
}

/// `@acme/web-ui` → `web-ui`, `src/auth` → `auth`; lowercase letters, digits, `-` and `_` only.
fn short_scope(s: &str) -> Option<String> {
    let last = s.trim().rsplit(['/', '\\', ':']).next().unwrap_or(s);
    let out: String = last
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(30)
        .collect();
    (!out.is_empty() && out != "src").then_some(out)
}

/// Add the graph tools to `tools`, wrap the file tools for strict mode and write tracking, append
/// the system rule, and start building the graph in the background.
pub fn attach(
    tools: &mut Vec<Arc<dyn Tool>>,
    system: &mut String,
    root: &Path,
    opts: Options,
) -> Arc<Session> {
    let session = Session::new(root, opts);
    let wrapped: Vec<Arc<dyn Tool>> = tools
        .drain(..)
        .map(|t| -> Arc<dyn Tool> {
            if t.name() == "Read" || t.kind() == ToolKind::Edit {
                Arc::new(Tracked {
                    inner: t,
                    session: Arc::clone(&session),
                })
            } else {
                t
            }
        })
        .collect();
    *tools = wrapped;
    tools.extend(graph_tools(&session));
    system.push_str(SYSTEM_RULE);
    session.warm();
    session
}

/// The four graph tools over `session`.
pub fn graph_tools(session: &Arc<Session>) -> Vec<Arc<dyn Tool>> {
    [Which::Query, Which::Explain, Which::Path, Which::Impact]
        .into_iter()
        .map(|which| -> Arc<dyn Tool> {
            Arc::new(GraphTool {
                which,
                session: Arc::clone(session),
            })
        })
        .collect()
}

/// `kor bench --graph off`: no graph tools, no rule, wrapped tools pass through.
pub fn strip(agent: &mut crate::Agent) {
    if let Some(s) = find(&agent.ctx.root) {
        s.set_active(false);
    }
    agent.tools.retain(|t| !TOOL_NAMES.contains(&t.name()));
    agent.system = agent.system.replace(SYSTEM_RULE, "");
}

/// `kor bench --graph on`: graph tools even when the config turned them off.
pub fn ensure(agent: &mut crate::Agent, opts: Options) {
    if agent.tools.iter().any(|t| t.name() == "graph_query") {
        return;
    }
    let root = agent.ctx.root.clone();
    attach(&mut agent.tools, &mut agent.system, &root, opts);
}

// ---------------------------------------------------------------------------------------------
// File tools wrapper
// ---------------------------------------------------------------------------------------------

/// A file tool that blocks the first raw read in strict mode, records touched files, and updates
/// the graph after a successful write.
pub struct Tracked {
    inner: Arc<dyn Tool>,
    session: Arc<Session>,
}

impl std::fmt::Debug for Tracked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracked")
            .field("inner", &self.inner.name())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Tool for Tracked {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn rule_name(&self) -> &'static str {
        self.inner.rule_name()
    }
    fn kind(&self) -> ToolKind {
        self.inner.kind()
    }
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn target(&self, input: &Value) -> Option<String> {
        self.inner.target(input)
    }
    fn summary(&self, input: &Value) -> String {
        self.inner.summary(input)
    }
    fn touches(&self, input: &Value) -> Vec<String> {
        self.inner.touches(input)
    }
    // The per-file review (§12.3) and the parameter card (§20.2) come from the wrapped tool.
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<kor_tools::Proposal>> {
        self.inner.propose(input, ctx).await
    }
    async fn param_card(&self, input: &Value, ctx: &ToolCtx) -> Option<kor_tools::ParamCard> {
        self.inner.param_card(input, ctx).await
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let s = &self.session;
        if !s.is_active() {
            return self.inner.call(input, ctx).await;
        }
        if self.inner.name() == "Read" && s.take_strict() {
            return ToolOutput::err(STRICT_HINT);
        }
        let mut paths: Vec<String> = self.inner.target(&input).into_iter().collect();
        paths.extend(self.inner.touches(&input));
        let out = self.inner.call(input, ctx).await;
        if out.is_error {
            return out;
        }
        let mut rel: Vec<String> = paths
            .iter()
            .filter_map(|p| {
                let abs = ctx.resolve(p).ok()?;
                let r = abs.strip_prefix(&ctx.root).ok()?;
                Some(r.to_string_lossy().replace('\\', "/"))
            })
            .filter(|r| !r.is_empty())
            .collect();
        rel.sort();
        rel.dedup();
        if self.inner.kind() == ToolKind::Edit && !rel.is_empty() {
            let (s2, r2) = (Arc::clone(s), rel.clone());
            let _ = tokio::task::spawn_blocking(move || s2.after_write(&r2)).await;
        }
        s.touch(rel);
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Graph tools
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Query,
    Explain,
    Path,
    Impact,
}

#[derive(Debug)]
pub struct GraphTool {
    which: Which,
    session: Arc<Session>,
}

fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    let v = input[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("`{key}` is required"))?;
    if v.chars().count() > MAX_QUERY_LEN {
        return Err(format!("`{key}` is limited to {MAX_QUERY_LEN} characters"));
    }
    Ok(v)
}

fn project_schema() -> Value {
    json!({"type": "string", "maxLength": 300, "description": "A linked project's path or folder name (`kor context add`); default: this project"})
}

#[async_trait]
impl Tool for GraphTool {
    fn name(&self) -> &'static str {
        match self.which {
            Which::Query => "graph_query",
            Which::Explain => "graph_explain",
            Which::Path => "graph_path",
            Which::Impact => "graph_impact",
        }
    }
    fn rule_name(&self) -> &'static str {
        "graph"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        let (description, input_schema) = match self.which {
            Which::Query => (
                "Search the project graph (functions, types, files, config keys, env vars, packages, decisions) by words or identifiers. Returns the best matches with file:line and package, so you know which few files to open. Use before Read/Grep. Database tables are graph nodes too (`table:<name>`, with columns, foreign keys and the code reading/writing them).",
                json!({"type": "object", "properties": {
                    "query": {"type": "string", "maxLength": MAX_QUERY_LEN, "description": "What you are looking for, e.g. `payment error logging`"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50},
                    "package": {"type": "string", "maxLength": 200, "description": "Only results inside this workspace package"},
                    "project": project_schema()
                }, "required": ["query"], "additionalProperties": false}),
            ),
            Which::Explain => (
                "Explain one symbol or file from the project graph: kind, location, source lines, community, callers, callees, files that change with it, and recorded decisions. Usually enough to edit without reading the whole file. For a database table (`users` or `table:users`): columns, foreign keys, readers and writers.",
                json!({"type": "object", "properties": {
                    "name": {"type": "string", "maxLength": MAX_QUERY_LEN, "description": "Symbol name (`login`, `Db.connect`), file path or node id"},
                    "project": project_schema()
                }, "required": ["name"], "additionalProperties": false}),
            ),
            Which::Path => (
                "Shortest connection between two concepts in the project graph (calls, imports, config, packages), in either direction.",
                json!({"type": "object", "properties": {
                    "from": {"type": "string", "maxLength": MAX_QUERY_LEN},
                    "to": {"type": "string", "maxLength": MAX_QUERY_LEN},
                    "project": project_schema()
                }, "required": ["from", "to"], "additionalProperties": false}),
            ),
            Which::Impact => (
                "What depends on a symbol, file, package or database table (`table:<name>`: the functions that read or write it), transitively: the code a change can break and the files to check or test.",
                json!({"type": "object", "properties": {
                    "target": {"type": "string", "maxLength": MAX_QUERY_LEN, "description": "Symbol, file path, `pkg:<name>` or `table:<name>`"},
                    "depth": {"type": "integer", "minimum": 1, "maximum": 6},
                    "project": project_schema()
                }, "required": ["target"], "additionalProperties": false}),
            ),
        };
        ToolSpec {
            name: self.name().into(),
            description: description.into(),
            input_schema,
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        let key = match self.which {
            Which::Query => "query",
            Which::Explain => "name",
            Which::Path => "from",
            Which::Impact => "target",
        };
        input[key].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        if !self.session.is_active() {
            return ToolOutput::err("the project graph is off for this session");
        }
        let session = Arc::clone(&self.session);
        let which = self.which;
        // Visibility check needs the context; collect what the ignore file hides up front.
        let ignore = ctx.ai_ignore.clone();
        let root = ctx.root.clone();
        let res = tokio::task::spawn_blocking(move || {
            let project = input["project"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let (g, linked) = match project {
                Some(p) => (session.linked_graph(p)?, true),
                None => (session.graph()?, false),
            };
            let g = lock(&g);
            let hidden = |file: Option<&str>| {
                !linked
                    && file.is_some_and(|f| {
                        ignore.as_ref().is_some_and(|i| {
                            let abs = root.join(f);
                            i.matched_path_or_any_parents(&abs, false).is_ignore()
                        })
                    })
            };
            run(which, &g, &input, linked, &hidden)
        })
        .await;
        match res {
            Ok(Ok(text)) => ToolOutput::ok(text),
            Ok(Err(e)) => ToolOutput::err(e),
            Err(e) => ToolOutput::err(format!("graph tool failed: {e}")),
        }
    }
}

fn loc(n: &NodeRef) -> String {
    match (&n.file, n.line) {
        (Some(f), Some(l)) => format!(" {f}:{l}"),
        (Some(f), None) => format!(" {f}"),
        _ => String::new(),
    }
}

fn node_line(n: &NodeRef) -> String {
    if n.file.as_deref() == Some(n.name.as_str()) {
        return format!("{} {}", n.kind.as_str(), n.name);
    }
    format!("{} {}{}", n.kind.as_str(), n.name, loc(n))
}

fn link_line(l: &Link) -> String {
    let conf = match l.confidence {
        Confidence::Extracted => String::new(),
        Confidence::Inferred { score } => format!(", inferred {score:.1}"),
        Confidence::Ambiguous => ", ambiguous".into(),
    };
    format!("{} [{}{conf}]", node_line(&l.node), l.kind.as_str())
}

type Hidden<'a> = dyn Fn(Option<&str>) -> bool + 'a;

fn run(
    which: Which,
    g: &Graph,
    input: &Value,
    linked: bool,
    hidden: &Hidden<'_>,
) -> Result<String, String> {
    if g.status().nodes == 0 {
        return Ok("The project graph is empty (no supported source files found).".into());
    }
    match which {
        Which::Query => query(g, input, hidden),
        Which::Explain => explain(g, str_arg(input, "name")?, linked, hidden),
        Which::Path => path(g, str_arg(input, "from")?, str_arg(input, "to")?, hidden),
        Which::Impact => impact(g, input, hidden),
    }
}

fn query(g: &Graph, input: &Value, hidden: &Hidden<'_>) -> Result<String, String> {
    let text = str_arg(input, "query")?;
    let limit = input["limit"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(MAX_LIST)
        .clamp(1, 50);
    let package = input["package"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(p) = package
        && g.workspace().is_none_or(|w| w.get(p).is_none())
    {
        return Err(format!("no workspace package named `{p}`"));
    }
    let pkg_of = |f: Option<&str>| f.and_then(|f| g.package_of(f)).map(|p| p.name.as_str());
    let hits: Vec<_> = g
        .query(text, limit.saturating_mul(4))
        .into_iter()
        .filter(|h| !hidden(h.node.file.as_deref()))
        .filter(|h| package.is_none() || pkg_of(h.node.file.as_deref()) == package)
        .take(limit)
        .collect();
    if hits.is_empty() {
        return Ok(format!(
            "No graph match for `{text}`. Try other words or identifiers, or `Grep`."
        ));
    }
    let mut out = String::new();
    let pkgs: BTreeSet<&str> = hits
        .iter()
        .filter_map(|h| pkg_of(h.node.file.as_deref()))
        .collect();
    if !pkgs.is_empty() {
        let _ = writeln!(
            out,
            "packages: {}",
            pkgs.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    for h in &hits {
        let pkg = pkg_of(h.node.file.as_deref())
            .map(|p| format!(" [pkg {p}]"))
            .unwrap_or_default();
        let _ = writeln!(out, "- {}{pkg}", node_line(&h.node));
    }
    let files: BTreeSet<&str> = hits.iter().filter_map(|h| h.node.file.as_deref()).collect();
    let _ = write!(
        out,
        "{} files: {}",
        files.len(),
        files.into_iter().collect::<Vec<_>>().join(", ")
    );
    Ok(out)
}

#[allow(clippy::many_single_char_names, clippy::too_many_lines)] // g = graph, n = node, as everywhere in this module
fn explain(g: &Graph, name: &str, linked: bool, hidden: &Hidden<'_>) -> Result<String, String> {
    let Some(ex) = g.explain(name) else {
        return Ok(format!(
            "Nothing named `{name}` in the graph. Use `graph_query` to search."
        ));
    };
    let n = &ex.node;
    let file = n
        .file
        .as_ref()
        .map(|f| f.to_string_lossy().replace('\\', "/"));
    if hidden(file.as_deref()) {
        return Err(format!("`{name}` is excluded by the project's ignore file"));
    }
    let mut out = String::new();
    let span = match (&file, n.span) {
        (Some(f), Some((a, b))) => format!(" {f}:{a}-{b}"),
        (Some(f), None) => format!(" {f}"),
        _ => String::new(),
    };
    let _ = writeln!(out, "{} {}{span} (id `{}`)", n.kind.as_str(), n.name, n.id);
    let community = ex.community.as_ref().map_or_else(
        || n.community.to_string(),
        |c| format!("{} \"{}\"", c.id, c.label),
    );
    let pkg = file
        .as_deref()
        .and_then(|f| g.package_of(f))
        .map(|p| format!(", package {}", p.name))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "community {community}, degree {}, churn {} commits/30 d{pkg}",
        n.degree, n.churn_30d
    );
    if !n.text.is_empty() && n.kind != kor_graph::NodeKind::Module {
        let _ = writeln!(
            out,
            "note: {}",
            n.text.chars().take(300).collect::<String>()
        );
    }
    if n.kind == kor_graph::NodeKind::Table
        && let Some(t) = g.table(&n.id)
    {
        if !t.indexes.is_empty() {
            let _ = writeln!(out, "indexes: {}", t.indexes.join("; "));
        }
        for q in t.slow_queries.iter().take(5) {
            let _ = writeln!(
                out,
                "slow query ({} µs avg, {} calls): {}",
                q.mean_us,
                q.calls,
                q.query.chars().take(300).collect::<String>()
            );
        }
    }
    if !linked && let Some(src) = &ex.source {
        let _ = writeln!(out, "source:\n{src}");
    }
    let section = |out: &mut String, title: &str, links: &[Link]| {
        let shown: Vec<&Link> = links
            .iter()
            .filter(|l| !hidden(l.node.file.as_deref()))
            .collect();
        if shown.is_empty() {
            return;
        }
        let _ = writeln!(out, "{title} ({}):", shown.len());
        for l in shown.iter().take(MAX_LIST) {
            let _ = writeln!(out, "  - {}", link_line(l));
        }
        if shown.len() > MAX_LIST {
            let _ = writeln!(out, "  … {} more", shown.len() - MAX_LIST);
        }
    };
    section(&mut out, "callers / used by", &ex.callers);
    section(&mut out, "calls / uses", &ex.callees);
    section(&mut out, "changes together with", &ex.co_changes);
    if !ex.decisions.is_empty() {
        let _ = writeln!(out, "decisions:");
        for d in ex.decisions.iter().take(MAX_LIST) {
            let text = g
                .node(&d.id)
                .map(|n| n.text.chars().take(200).collect::<String>())
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  - {}{}",
                node_line(d),
                if text.is_empty() {
                    String::new()
                } else {
                    format!(": {text}")
                }
            );
        }
    }
    if !ex.alternatives.is_empty() {
        let alts: Vec<String> = ex
            .alternatives
            .iter()
            .filter(|a| !hidden(a.file.as_deref()))
            .take(5)
            .map(|a| format!("`{}`", a.id))
            .collect();
        if !alts.is_empty() {
            let _ = writeln!(out, "other matches: {}", alts.join(", "));
        }
    }
    Ok(out.trim_end().to_owned())
}

fn path(g: &Graph, from: &str, to: &str, hidden: &Hidden<'_>) -> Result<String, String> {
    let Some(steps) = g.path(from, to) else {
        return Ok(format!(
            "No connection between `{from}` and `{to}` in the graph."
        ));
    };
    if steps.iter().any(|s| hidden(s.node.file.as_deref())) {
        return Err("the path passes through files excluded by the project's ignore file".into());
    }
    let mut out = String::new();
    for (i, s) in steps.iter().enumerate() {
        if let Some(e) = s.edge {
            let arrow = if s.forward {
                format!("  -{}->", e.as_str())
            } else {
                format!("  <-{}-", e.as_str())
            };
            let _ = writeln!(out, "{arrow}");
        }
        let _ = writeln!(out, "{}. {}", i + 1, node_line(&s.node));
    }
    Ok(out.trim_end().to_owned())
}

fn impact(g: &Graph, input: &Value, hidden: &Hidden<'_>) -> Result<String, String> {
    let target = str_arg(input, "target")?;
    let depth = input["depth"]
        .as_u64()
        .and_then(|d| u32::try_from(d).ok())
        .unwrap_or(3)
        .clamp(1, 6);
    let imp = g.impact(target, depth);
    if imp.roots.is_empty() {
        return Ok(format!(
            "Nothing named `{target}` in the graph. Use `graph_query` to search."
        ));
    }
    let mut out = String::new();
    let roots: Vec<String> = imp.roots.iter().map(node_line).collect();
    let _ = writeln!(out, "impact of {}:", roots.join("; "));
    let affected: Vec<_> = imp
        .affected
        .iter()
        .filter(|a| !hidden(a.node.file.as_deref()))
        .collect();
    if affected.is_empty() {
        let _ = writeln!(out, "nothing depends on it");
    }
    for a in affected.iter().take(40) {
        let _ = writeln!(
            out,
            "  depth {} [{}] {}",
            a.depth,
            a.via.as_str(),
            node_line(&a.node)
        );
    }
    if affected.len() > 40 || imp.truncated {
        let _ = writeln!(out, "  … more (truncated)");
    }
    let files: Vec<&str> = imp
        .files
        .iter()
        .map(String::as_str)
        .filter(|f| !hidden(Some(f)))
        .collect();
    if !files.is_empty() {
        let _ = writeln!(out, "files to check: {}", files.join(", "));
    }
    if let Some(ws) = g.workspace()
        && ws.is_monorepo()
    {
        let pkgs = ws.affected(&files.iter().map(PathBuf::from).collect::<Vec<_>>());
        if !pkgs.is_empty() {
            let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
            let _ = writeln!(out, "affected packages: {}", names.join(", "));
            for c in ws.narrowed_commands(&pkgs) {
                let _ = writeln!(out, "  test: {c}");
            }
        }
    }
    Ok(out.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let w = |p: &str, s: &str| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, s).expect("write");
        };
        w(
            "src/lib.rs",
            "mod auth;\nmod db;\nuse crate::auth::login;\npub fn main_entry() { login(\"u\"); }\n",
        );
        w(
            "src/auth.rs",
            "use crate::db::Db;\n// WHY: tokens expire fast\npub fn login(u: &str) -> bool { let db = Db::connect(); db.find(u) }\n",
        );
        w(
            "src/db.rs",
            "pub struct Db;\nimpl Db {\n    pub fn connect() -> Self { Db }\n    pub fn find(&self, _u: &str) -> bool { true }\n}\n",
        );
        w("secret/keys.rs", "pub fn secret_login_key() -> u32 { 7 }\n");
        w(".korignore", "secret/\n");
        dir
    }

    fn ctx(root: &Path) -> ToolCtx {
        let mut c = ToolCtx::new(root).expect("ctx");
        let _ = c.load_ai_ignore();
        c
    }

    fn tool(s: &Arc<Session>, name: &str) -> Arc<dyn Tool> {
        graph_tools(s)
            .into_iter()
            .find(|t| t.name() == name)
            .expect("tool")
    }

    #[tokio::test]
    #[allow(clippy::many_single_char_names)] // one letter per tool result
    async fn tools_answer_from_the_graph() {
        let dir = fixture();
        let c = ctx(dir.path());
        let s = Session::new(&c.root, Options::default());
        let q = tool(&s, "graph_query")
            .call(json!({"query": "login"}), &c)
            .await;
        assert!(!q.is_error, "{}", q.content);
        assert!(
            q.content.contains("function login src/auth.rs:3"),
            "{}",
            q.content
        );
        assert!(
            !q.content.contains("secret"),
            "ignored file leaked: {}",
            q.content
        );

        let e = tool(&s, "graph_explain")
            .call(json!({"name": "login"}), &c)
            .await;
        assert!(e.content.contains("source:"), "{}", e.content);
        assert!(e.content.contains("calls / uses"), "{}", e.content);
        assert!(
            e.content.contains("Db.connect") || e.content.contains("connect"),
            "{}",
            e.content
        );
        let f = tool(&s, "graph_explain")
            .call(json!({"name": "src/auth.rs"}), &c)
            .await;
        assert!(f.content.contains("tokens expire fast"), "{}", f.content);

        let hidden = tool(&s, "graph_explain")
            .call(json!({"name": "secret/keys.rs"}), &c)
            .await;
        assert!(
            hidden.is_error || !hidden.content.contains("7 }"),
            "{}",
            hidden.content
        );

        let p = tool(&s, "graph_path")
            .call(json!({"from": "main_entry", "to": "Db"}), &c)
            .await;
        assert!(
            p.content.contains("main_entry") && p.content.contains("src/db.rs"),
            "{}",
            p.content
        );

        let i = tool(&s, "graph_impact")
            .call(json!({"target": "src/db.rs"}), &c)
            .await;
        assert!(i.content.contains("src/auth.rs"), "{}", i.content);

        let bad = tool(&s, "graph_query").call(json!({}), &c).await;
        assert!(bad.is_error);
        let linked = tool(&s, "graph_query")
            .call(json!({"query": "x", "project": "elsewhere"}), &c)
            .await;
        assert!(linked.is_error, "{}", linked.content);
        assert_eq!(tool(&s, "graph_query").kind(), ToolKind::Read);
    }

    #[tokio::test]
    async fn strict_mode_blocks_first_read_once_and_writes_update_the_graph() {
        let dir = fixture();
        let c = ctx(dir.path());
        let mut tools = kor_tools::default_tools();
        let mut system = String::new();
        let s = attach(
            &mut tools,
            &mut system,
            &c.root,
            Options {
                strict: true,
                ..Options::default()
            },
        );
        assert!(system.contains("Before reading a file"));
        assert!(
            TOOL_NAMES
                .iter()
                .all(|n| tools.iter().any(|t| t.name() == *n))
        );
        let read = tools
            .iter()
            .find(|t| t.name() == "Read")
            .expect("Read")
            .clone();
        let first = read.call(json!({"file_path": "src/db.rs"}), &c).await;
        assert!(
            first.is_error && first.content.contains("Strict graph mode"),
            "{}",
            first.content
        );
        let second = read.call(json!({"file_path": "src/db.rs"}), &c).await;
        assert!(!second.is_error, "{}", second.content);
        assert_eq!(s.touched(), vec!["src/db.rs".to_owned()]);
        assert!(find(&c.root).is_some_and(|f| Arc::ptr_eq(&f, &s)));

        // Build, then let the agent write a new function: the graph has it without the watcher.
        s.graph().expect("graph");
        let write = tools
            .iter()
            .find(|t| t.name() == "Write")
            .expect("Write")
            .clone();
        let out = write
            .call(
                json!({"file_path": "src/extra.rs", "content": "pub fn added_by_agent() {}\n"}),
                &c,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        let g = s.graph().expect("graph");
        assert!(lock(&g).node("src/extra.rs#added_by_agent").is_some());

        // Bench "off" arm: inactive session passes reads through, graph tools refuse.
        s.set_active(false);
        let q = tool(&s, "graph_query")
            .call(json!({"query": "login"}), &c)
            .await;
        assert!(q.is_error);
    }

    #[test]
    fn commit_scope_from_package_or_community() {
        assert_eq!(short_scope("@acme/web-ui").as_deref(), Some("web-ui"));
        assert_eq!(short_scope("src/Auth").as_deref(), Some("auth"));
        assert_eq!(short_scope("src"), None);
        let dir = tempfile::tempdir().expect("tempdir");
        let w = |p: &str, s: &str| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, s).expect("write");
        };
        w(
            "Cargo.toml",
            "[workspace]
members = [\"crates/*\"]
",
        );
        w(
            "crates/core/Cargo.toml",
            "[package]
name = \"core\"
",
        );
        w(
            "crates/core/src/lib.rs",
            "pub fn shared() {}
",
        );
        w(
            "crates/app/Cargo.toml",
            "[package]
name = \"app\"
",
        );
        w(
            "crates/app/src/main.rs",
            "fn main() {}
",
        );
        let files = vec!["crates/core/src/lib.rs".to_owned()];
        // No stored graph yet: the directory rule decides.
        assert_eq!(commit_scope(dir.path(), &files), None);
        Graph::open(dir.path())
            .and_then(|mut g| g.update())
            .expect("build");
        assert_eq!(commit_scope(dir.path(), &files).as_deref(), Some("core"));
        let both = vec![files[0].clone(), "crates/app/src/main.rs".to_owned()];
        assert_ne!(commit_scope(dir.path(), &both).as_deref(), Some("core"));
    }

    #[test]
    fn removed_public_symbols_are_checked_against_their_users() {
        let diff = "diff --git a/src/auth.rs b/src/auth.rs\n--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -3 +3 @@\n-pub fn login(u: &str) -> bool { true }\n+pub fn sign_in(u: &str) -> bool { true }\n-pub struct Kept;\n+pub struct Kept;\n-fn private() {}\n";
        assert_eq!(
            changed_public_symbols(diff),
            [("src/auth.rs".to_owned(), "login".to_owned())],
            "moved lines and private items are not API changes"
        );
        let ts = "+++ b/web/api.ts\n-export async function fetchUser(id: string) {\n+export async function fetchUser(id: number) {\n";
        assert_eq!(
            changed_public_symbols(ts),
            [("web/api.ts".to_owned(), "fetchUser".to_owned())]
        );

        let dir = fixture();
        // No stored graph: nothing to verify against.
        assert!(api_breaks(dir.path(), &["src/auth.rs".into()], diff).is_empty());
        Graph::open(dir.path())
            .and_then(|mut g| g.update())
            .expect("build");
        let found = api_breaks(dir.path(), &["src/auth.rs".into()], diff);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].symbol, "login");
        assert_eq!(found[0].users, ["src/lib.rs"]);
        // Callers updated in the same change: not breaking for this repository.
        assert!(
            api_breaks(
                dir.path(),
                &["src/auth.rs".into(), "src/lib.rs".into()],
                diff
            )
            .is_empty()
        );
    }
}
