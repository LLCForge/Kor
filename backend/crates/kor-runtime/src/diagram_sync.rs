//! Live diagrams (Master Build Prompt §27.14, ADR 0063): marked Mermaid / PlantUML / D2 blocks in
//! the project's docs are regenerated from the project graph (§7), so `kor diagram check` can fail
//! CI when a diagram no longer matches the code.
//!
//! A block is a marked region; only its inside is Kor's:
//!
//! ````text
//! <!-- kor:diagram kind=architecture format=mermaid depth=2 hash=1a2b3c4d -->
//! ```mermaid
//! flowchart LR
//!   ...generated...
//!   %% kor:keep
//!   style src_auth fill:#fdd        <- hand-written, kept on every sync
//! ```
//! <!-- /kor:diagram -->
//! ````
//!
//! In `.mmd` files the markers are `%% kor:diagram …` / `%% /kor:diagram`, in `.puml` files
//! `' kor:diagram …` / `' /kor:diagram`, in `.d2` files `# kor:diagram …` / `# /kor:diagram`.
//! An `.excalidraw` drawing is one block when it has a text element `kor:diagram kind=…`: its
//! Kor-made shapes are regenerated, moved/recoloured ones keep their place and style, the user's
//! own shapes are untouched (`diagram_render`). `svg=<path>` on any block also writes an SVG of
//! the diagram (for READMEs that show a picture).
//! Everything outside the markers, and every line after a `kor:keep` line inside them, is the
//! user's and never rewritten. `hash` is the hash of the generated part as Kor last wrote it: when
//! the generated part was edited by hand, sync stops and asks (`--force` overwrites).
//!
//! Kinds: `architecture` (directories or packages and their dependencies), `dependency` (files),
//! `sequence` (call flow from `from=<symbol>`), `er` (database tables), `deploy` (servers and
//! services from `kor discover`). Parameters: `scope=<dir>`, `depth=<n>`, `max=<nodes>`,
//! `from=<symbol>`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_graph::{EdgeKind, Graph, NodeKind};
use serde::Serialize;

/// Doc files scanned for blocks.
const EXTS: &[&str] = &[
    "md",
    "markdown",
    "mmd",
    "mermaid",
    "puml",
    "plantuml",
    "pu",
    "d2",
    "excalidraw",
];
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 5_000;
const DEFAULT_MAX: usize = 40;
const KEEP: &str = "kor:keep";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mermaid,
    Plantuml,
    D2,
    /// An `.excalidraw` drawing (whole file).
    Excalidraw,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "mermaid" | "mmd" => Some(Self::Mermaid),
            "plantuml" | "puml" | "uml" => Some(Self::Plantuml),
            "d2" => Some(Self::D2),
            "excalidraw" => Some(Self::Excalidraw),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Mermaid => "mermaid",
            Self::Plantuml => "plantuml",
            Self::D2 => "d2",
            Self::Excalidraw => "excalidraw",
        }
    }

    fn comment(self) -> &'static str {
        match self {
            Self::Mermaid => "%%",
            Self::Plantuml => "'",
            Self::D2 | Self::Excalidraw => "#",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Architecture,
    Dependency,
    Sequence,
    Er,
    Deploy,
}

impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Architecture,
        Self::Dependency,
        Self::Sequence,
        Self::Er,
        Self::Deploy,
    ];

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "architecture" | "arch" | "modules" => Some(Self::Architecture),
            "dependency" | "dependencies" | "deps" => Some(Self::Dependency),
            "sequence" | "seq" => Some(Self::Sequence),
            "er" | "schema" => Some(Self::Er),
            "deploy" | "deployment" | "topology" => Some(Self::Deploy),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::Dependency => "dependency",
            Self::Sequence => "sequence",
            Self::Er => "er",
            Self::Deploy => "deploy",
        }
    }
}

/// How a block's file wraps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    /// Markdown: HTML comment markers and a fenced code block.
    Markdown,
    /// A diagram file: comment markers in the diagram's own syntax.
    Raw(Format),
    /// An Excalidraw drawing: the marker is a text element.
    Excalidraw,
}

fn host_of(path: &Path) -> Option<Host> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "md" | "markdown" => Host::Markdown,
        "mmd" | "mermaid" => Host::Raw(Format::Mermaid),
        "puml" | "plantuml" | "pu" => Host::Raw(Format::Plantuml),
        "d2" => Host::Raw(Format::D2),
        "excalidraw" => Host::Excalidraw,
        _ => return None,
    })
}

/// One marked block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Block {
    /// Project-relative, `/`-separated.
    pub file: String,
    /// 1-based line of the opening marker.
    pub line: usize,
    pub kind: Kind,
    pub format: Format,
    pub params: BTreeMap<String, String>,
    /// Hash in the marker (`None` for a block never generated).
    pub hash: Option<String>,
    /// Current generated part (without the fence and the kept lines).
    pub generated: String,
    /// Hand-written lines after `kor:keep`.
    pub kept: Vec<String>,
    #[serde(skip)]
    range: (usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    UpToDate,
    /// The code changed; sync rewrites the block.
    Stale,
    /// The generated part was edited by hand; sync asks first.
    Edited,
    /// The marker is invalid (unknown kind/format, missing end marker, symbol not found).
    Invalid,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub edges_added: usize,
    pub edges_removed: usize,
}

/// What sync would do with one block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub file: String,
    pub line: usize,
    pub kind: Option<Kind>,
    pub status: Status,
    /// Why a block is invalid.
    pub error: Option<String>,
    pub counts: Counts,
    /// The regenerated block content (without markers).
    pub text: String,
    /// One line per node and edge, for terminals (TUI ASCII summary).
    pub ascii: Vec<String>,
    /// The diagram the text was rendered from (Excalidraw and SVG outputs).
    #[serde(skip)]
    pub model: Option<Model>,
}

/// Short content hash (FNV-1a 32-bit, hex): edit detection, not security.
fn hash(s: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.trim().replace("\r\n", "\n").bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

fn parse_params(s: &str) -> BTreeMap<String, String> {
    s.split_whitespace()
        .filter_map(|w| w.split_once('='))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim_matches('"').to_owned()))
        .collect()
}

/// `(params, is_end)` of a marker line, `None` for other lines.
fn marker(line: &str, host: Host) -> Option<(String, bool)> {
    let t = line.trim();
    let inner = match host {
        Host::Markdown => t.strip_prefix("<!--")?.strip_suffix("-->")?.trim(),
        Host::Raw(f) => t.strip_prefix(f.comment())?.trim(),
        Host::Excalidraw => return None,
    };
    if let Some(rest) = inner.strip_prefix("/kor:diagram") {
        return rest.trim().is_empty().then(|| (String::new(), true));
    }
    let rest = inner.strip_prefix("kor:diagram")?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| (rest.trim().to_owned(), false))
}

/// Blocks of one file's text. Errors are `(line, message)`.
fn blocks_in(file: &str, text: &str, host: Host) -> (Vec<Block>, Vec<(usize, String)>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut errs = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((params, false)) = marker(lines[i], host) else {
            i += 1;
            continue;
        };
        let Some(end) = (i + 1..lines.len()).find(|&j| marker(lines[j], host).is_some()) else {
            errs.push((i + 1, "missing end marker".into()));
            break;
        };
        if marker(lines[end], host).is_some_and(|(_, e)| !e) {
            errs.push((i + 1, "nested kor:diagram marker".into()));
            i = end;
            continue;
        }
        let p = parse_params(&params);
        let kind = p.get("kind").and_then(|k| Kind::parse(k));
        let format = match host {
            Host::Raw(f) => Some(f),
            Host::Markdown => p
                .get("format")
                .map_or(Some(Format::Mermaid), |f| Format::parse(f))
                .filter(|f| *f != Format::Excalidraw),
            Host::Excalidraw => None,
        };
        let (Some(kind), Some(format)) = (kind, format) else {
            errs.push((i + 1, format!("unknown kind or format: {params}")));
            i = end + 1;
            continue;
        };
        let mut body: Vec<&str> = lines[i + 1..end].to_vec();
        if host == Host::Markdown {
            // Drop the fence lines.
            if body
                .first()
                .is_some_and(|l| l.trim_start().starts_with("```"))
            {
                body.remove(0);
            }
            if body
                .last()
                .is_some_and(|l| l.trim_start().starts_with("```"))
            {
                body.pop();
            }
        }
        let keep_at = body
            .iter()
            .position(|l| l.trim().ends_with(KEEP) && l.trim().starts_with(format.comment()));
        let (gen_part, kept) = match keep_at {
            Some(k) => (
                &body[..k],
                body[k + 1..].iter().map(|s| (*s).to_owned()).collect(),
            ),
            None => (&body[..], Vec::new()),
        };
        out.push(Block {
            file: file.to_owned(),
            line: i + 1,
            kind,
            format,
            hash: p.get("hash").cloned(),
            params: p
                .into_iter()
                .filter(|(k, _)| !matches!(k.as_str(), "kind" | "format" | "hash"))
                .collect(),
            generated: gen_part.join("\n"),
            kept,
            range: (i, end),
        });
        i = end + 1;
    }
    (out, errs)
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every marked block in the project's docs (gitignored files skipped). Invalid markers are
/// returned as `Change`s with [`Status::Invalid`].
pub fn scan(root: &Path) -> (Vec<Block>, Vec<Change>) {
    let mut blocks = Vec::new();
    let mut bad = Vec::new();
    let walk = ignore::WalkBuilder::new(root).hidden(true).build();
    for e in walk.flatten().take(200_000) {
        let p = e.path();
        if !e.file_type().is_some_and(|t| t.is_file())
            || p.components().any(|c| c.as_os_str() == ".kor")
        {
            continue;
        }
        let ext = p
            .extension()
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !EXTS.contains(&ext.as_str())
            || std::fs::metadata(p).map_or(true, |m| m.len() > MAX_FILE)
        {
            continue;
        }
        let Some(host) = host_of(p) else { continue };
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        if !text.contains("kor:diagram") {
            continue;
        }
        let file = rel(root, p);
        if host == Host::Excalidraw {
            match excalidraw_block(&file, &text) {
                Ok(Some(b)) => blocks.push(b),
                Ok(None) => {}
                Err(e) => bad.push(invalid(&file, 1, None, e)),
            }
            continue;
        }
        let (b, errs) = blocks_in(&file, &text, host);
        blocks.extend(b);
        bad.extend(
            errs.into_iter()
                .map(|(line, msg)| invalid(&file, line, None, msg)),
        );
        if blocks.len() > MAX_FILES {
            break;
        }
    }
    blocks.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    (blocks, bad)
}

/// The block of an `.excalidraw` drawing with a `kor:diagram` text element.
fn excalidraw_block(file: &str, text: &str) -> Result<Option<Block>, String> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not an Excalidraw file: {e}"))?;
    let Some(params) = crate::diagram_render::excalidraw_marker(&doc) else {
        return Ok(None);
    };
    let p = parse_params(&params);
    let kind = p
        .get("kind")
        .and_then(|k| Kind::parse(k))
        .ok_or_else(|| format!("unknown kind: {params}"))?;
    Ok(Some(Block {
        file: file.to_owned(),
        line: 1,
        kind,
        format: Format::Excalidraw,
        hash: None,
        params: p.into_iter().filter(|(k, _)| k != "kind").collect(),
        generated: crate::diagram_render::excalidraw_structure(&doc),
        kept: Vec::new(),
        range: (0, 0),
    }))
}

fn invalid(file: &str, line: usize, kind: Option<Kind>, msg: String) -> Change {
    Change {
        file: file.to_owned(),
        line,
        kind,
        status: Status::Invalid,
        error: Some(msg),
        counts: Counts::default(),
        text: String::new(),
        ascii: Vec::new(),
        model: None,
    }
}

// ---------------------------------------------------------------------------------------------
// Models from the graph
// ---------------------------------------------------------------------------------------------

/// A diagram before it is rendered in a format.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    /// id → label (ids are already safe identifiers).
    pub nodes: BTreeMap<String, String>,
    /// (from, to, label).
    pub edges: BTreeSet<(String, String, String)>,
    /// ER only: table id → columns.
    pub columns: BTreeMap<String, Vec<String>>,
}

fn ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'n');
    }
    out
}

fn num(params: &BTreeMap<String, String>, key: &str, default: usize) -> usize {
    params
        .get(key)
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
        .min(500)
}

fn in_scope(file: &str, scope: Option<&String>) -> bool {
    scope.is_none_or(|s| {
        let s = s.trim_matches('/');
        s.is_empty() || file == s || file.starts_with(&format!("{s}/"))
    })
}

/// The directory of `file` cut to `depth` components below `scope`.
fn group(file: &str, scope: Option<&String>, depth: usize) -> String {
    let base = scope.map(|s| s.trim_matches('/')).filter(|s| !s.is_empty());
    let rest = base
        .and_then(|b| file.strip_prefix(b))
        .map_or(file, |r| r.trim_start_matches('/'));
    let parts: Vec<&str> = rest.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let take = dirs.len().min(depth);
    let g = if take == 0 {
        ".".to_owned()
    } else {
        dirs[..take].join("/")
    };
    match base {
        Some(b) if g == "." => b.to_owned(),
        Some(b) => format!("{b}/{g}"),
        None => g,
    }
}

fn dependency_edge(k: EdgeKind) -> bool {
    matches!(
        k,
        EdgeKind::Calls
            | EdgeKind::Imports
            | EdgeKind::DependsOn
            | EdgeKind::Implements
            | EdgeKind::Extends
    )
}

/// Keep the `max` busiest nodes and the edges between them.
fn trim(mut m: Model, max: usize) -> Model {
    if m.nodes.len() <= max {
        return m;
    }
    let mut degree: BTreeMap<&String, usize> = BTreeMap::new();
    for (a, b, _) in &m.edges {
        *degree.entry(a).or_default() += 1;
        *degree.entry(b).or_default() += 1;
    }
    let mut ids: Vec<(&String, usize)> = m
        .nodes
        .keys()
        .map(|k| (k, degree.get(k).copied().unwrap_or(0)))
        .collect();
    ids.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let keep: BTreeSet<String> = ids.into_iter().take(max).map(|(k, _)| k.clone()).collect();
    m.nodes.retain(|k, _| keep.contains(k));
    m.edges
        .retain(|(a, b, _)| keep.contains(a) && keep.contains(b));
    m
}

fn file_of(g: &Graph, id: &str) -> Option<String> {
    g.node(id)?
        .file
        .as_ref()
        .map(|f| f.to_string_lossy().replace('\\', "/"))
}

/// The model of `block` from `g`.
///
/// # Errors
/// A message when the block cannot be generated (unknown `from` symbol).
pub fn model(g: &Graph, kind: Kind, params: &BTreeMap<String, String>) -> Result<Model, String> {
    let scope = params.get("scope");
    let max = num(params, "max", DEFAULT_MAX);
    let mut m = Model::default();
    match kind {
        Kind::Architecture => {
            // Packages when the repository is a monorepo, else directories.
            let packages: Vec<_> = g.nodes().filter(|n| n.kind == NodeKind::Package).collect();
            if !packages.is_empty() && scope.is_none() {
                for p in &packages {
                    m.nodes.insert(ident(&p.id), p.name.clone());
                }
                for e in g.edges().filter(|e| e.kind == EdgeKind::DependsOn) {
                    m.edges
                        .insert((ident(&e.source), ident(&e.target), String::new()));
                }
            } else {
                let depth = num(params, "depth", 1);
                for n in g.nodes().filter(|n| n.kind == NodeKind::Module) {
                    if let Some(f) = n
                        .file
                        .as_ref()
                        .map(|f| f.to_string_lossy().replace('\\', "/"))
                        && in_scope(&f, scope)
                    {
                        let d = group(&f, scope, depth);
                        m.nodes.insert(ident(&d), d);
                    }
                }
                for e in g.edges().filter(|e| dependency_edge(e.kind)) {
                    let (Some(a), Some(b)) = (file_of(g, &e.source), file_of(g, &e.target)) else {
                        continue;
                    };
                    if !in_scope(&a, scope) || !in_scope(&b, scope) {
                        continue;
                    }
                    let depth = num(params, "depth", 1);
                    let (ga, gb) = (group(&a, scope, depth), group(&b, scope, depth));
                    if ga != gb {
                        m.edges.insert((ident(&ga), ident(&gb), String::new()));
                    }
                }
            }
        }
        Kind::Dependency => {
            for n in g.nodes().filter(|n| n.kind == NodeKind::Module) {
                if let Some(f) = n
                    .file
                    .as_ref()
                    .map(|f| f.to_string_lossy().replace('\\', "/"))
                    && in_scope(&f, scope)
                {
                    m.nodes.insert(ident(&f), f);
                }
            }
            for e in g.edges().filter(|e| dependency_edge(e.kind)) {
                let (Some(a), Some(b)) = (file_of(g, &e.source), file_of(g, &e.target)) else {
                    continue;
                };
                if a != b && in_scope(&a, scope) && in_scope(&b, scope) {
                    m.edges.insert((ident(&a), ident(&b), String::new()));
                }
            }
        }
        Kind::Sequence => {
            let from = params.get("from").ok_or("sequence needs from=<symbol>")?;
            let start = g
                .nodes()
                .filter(|n| {
                    matches!(
                        n.kind,
                        NodeKind::Function | NodeKind::Struct | NodeKind::Trait
                    )
                })
                .find(|n| n.name == *from || n.id.ends_with(&format!("#{from}")))
                .ok_or_else(|| format!("symbol `{from}` not found in the graph"))?;
            let depth = num(params, "depth", 3);
            let calls: BTreeMap<&str, Vec<&str>> = g
                .edges()
                .filter(|e| e.kind == EdgeKind::Calls)
                .fold(BTreeMap::new(), |mut acc, e| {
                    acc.entry(e.source.as_str())
                        .or_default()
                        .push(e.target.as_str());
                    acc
                });
            let participant = |id: &str| file_of(g, id).unwrap_or_else(|| id.to_owned());
            let mut seen = BTreeSet::new();
            let mut frontier = vec![start.id.as_str()];
            let mut order = 0usize;
            for _ in 0..depth {
                let mut next = Vec::new();
                for s in frontier {
                    if !seen.insert(s) {
                        continue;
                    }
                    let mut targets = calls.get(s).cloned().unwrap_or_default();
                    targets.sort_unstable();
                    for t in targets {
                        let (pa, pb) = (participant(s), participant(t));
                        m.nodes.insert(ident(&pa), pa.clone());
                        m.nodes.insert(ident(&pb), pb.clone());
                        let name = g.node(t).map_or(t, |n| n.name.as_str());
                        // Order prefix keeps the call order in a sorted set.
                        m.edges
                            .insert((ident(&pa), ident(&pb), format!("{order:04}|{name}")));
                        order += 1;
                        next.push(t);
                    }
                }
                frontier = next;
            }
            if m.nodes.is_empty() {
                let p = participant(&start.id);
                m.nodes.insert(ident(&p), p);
            }
        }
        Kind::Er => {
            for t in g.tables() {
                let id = ident(&t.node.name);
                m.nodes.insert(id.clone(), t.node.name.clone());
                m.columns.insert(id.clone(), t.columns.clone());
                for r in &t.references {
                    m.edges
                        .insert((id.clone(), ident(&r.name), "references".into()));
                }
            }
        }
        Kind::Deploy => {
            for n in g
                .nodes()
                .filter(|n| matches!(n.kind, NodeKind::Server | NodeKind::Service))
            {
                m.nodes.insert(ident(&n.id), n.name.clone());
            }
            for e in g.edges().filter(|e| e.kind == EdgeKind::RunsOn) {
                let both = [&e.source, &e.target].iter().all(|id| {
                    g.node(id)
                        .is_some_and(|n| matches!(n.kind, NodeKind::Server | NodeKind::Service))
                });
                if both {
                    m.edges
                        .insert((ident(&e.source), ident(&e.target), "runs on".into()));
                }
            }
        }
    }
    Ok(trim(m, max))
}

fn label(l: &str) -> String {
    l.replace('"', "'")
}

/// Render `m` as the structural part of a block.
pub fn render(m: &Model, kind: Kind, format: Format) -> String {
    let mut o = String::new();
    let seq_label = |l: &str| l.split_once('|').map_or(l, |(_, n)| n).to_owned();
    match (format, kind) {
        (Format::Mermaid, Kind::Sequence) => {
            o.push_str("sequenceDiagram\n");
            for (id, l) in &m.nodes {
                let _ = writeln!(o, "  participant {id} as {}", label(l));
            }
            for (a, b, l) in &m.edges {
                let _ = writeln!(o, "  {a}->>{b}: {}", label(&seq_label(l)));
            }
        }
        (Format::Mermaid, Kind::Er) => {
            o.push_str("erDiagram\n");
            for (id, cols) in &m.columns {
                let _ = writeln!(o, "  {id} {{");
                for c in cols {
                    let _ = writeln!(o, "    string {}", ident(c));
                }
                o.push_str("  }\n");
            }
            for (a, b, _) in &m.edges {
                let _ = writeln!(o, "  {a} }}o--|| {b} : references");
            }
        }
        (Format::Mermaid, _) => {
            o.push_str("flowchart LR\n");
            for (id, l) in &m.nodes {
                let _ = writeln!(o, "  {id}[\"{}\"]", label(l));
            }
            for (a, b, l) in &m.edges {
                if l.is_empty() {
                    let _ = writeln!(o, "  {a} --> {b}");
                } else {
                    let _ = writeln!(o, "  {a} -->|{}| {b}", label(l));
                }
            }
        }
        (Format::Plantuml, k) => {
            o.push_str("@startuml\n");
            match k {
                Kind::Sequence => {
                    for (id, l) in &m.nodes {
                        let _ = writeln!(o, "participant \"{}\" as {id}", label(l));
                    }
                    for (a, b, l) in &m.edges {
                        let _ = writeln!(o, "{a} -> {b} : {}", label(&seq_label(l)));
                    }
                }
                Kind::Er => {
                    for (id, cols) in &m.columns {
                        let _ = writeln!(o, "entity {id} {{");
                        for c in cols {
                            let _ = writeln!(o, "  {}", ident(c));
                        }
                        o.push_str("}\n");
                    }
                    for (a, b, _) in &m.edges {
                        let _ = writeln!(o, "{a} }}o--|| {b}");
                    }
                }
                Kind::Deploy => {
                    for (id, l) in &m.nodes {
                        let _ = writeln!(o, "node \"{}\" as {id}", label(l));
                    }
                    for (a, b, l) in &m.edges {
                        let _ = writeln!(o, "{a} --> {b} : {}", label(l));
                    }
                }
                Kind::Architecture | Kind::Dependency => {
                    for (id, l) in &m.nodes {
                        let _ = writeln!(o, "component \"{}\" as {id}", label(l));
                    }
                    for (a, b, _) in &m.edges {
                        let _ = writeln!(o, "{a} --> {b}");
                    }
                }
            }
            o.push_str("@enduml\n");
        }
        (Format::Excalidraw, _) => return crate::diagram_render::model_structure(m),
        (Format::D2, k) => {
            if k == Kind::Sequence {
                o.push_str("shape: sequence_diagram\n");
            }
            for (id, l) in &m.nodes {
                if k == Kind::Er {
                    let _ = writeln!(o, "{id}: {{\n  shape: sql_table");
                    for c in m.columns.get(id).into_iter().flatten() {
                        let _ = writeln!(o, "  {}: string", ident(c));
                    }
                    o.push_str("}\n");
                } else {
                    let _ = writeln!(o, "{id}: \"{}\"", label(l));
                }
            }
            for (a, b, l) in &m.edges {
                let l = seq_label(l);
                if l.is_empty() {
                    let _ = writeln!(o, "{a} -> {b}");
                } else {
                    let _ = writeln!(o, "{a} -> {b}: \"{}\"", label(&l));
                }
            }
        }
    }
    o.trim_end().to_owned()
}

/// Lines with an arrow are edges, the others nodes (for the "2 nodes added" summary).
fn split_lines(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut nodes = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if ["-->", "->>", "->", "--|", "}o--", "|o--"]
            .iter()
            .any(|a| l.contains(a))
        {
            edges.insert(l.to_owned());
        } else if !matches!(
            l,
            "@startuml"
                | "@enduml"
                | "}"
                | "flowchart LR"
                | "sequenceDiagram"
                | "erDiagram"
                | "shape: sequence_diagram"
                | "shape: sql_table"
        ) {
            nodes.insert(l.to_owned());
        }
    }
    (nodes, edges)
}

fn counts(old: &str, new: &str) -> Counts {
    let (on, oe) = split_lines(old);
    let (nn, ne) = split_lines(new);
    Counts {
        nodes_added: nn.difference(&on).count(),
        nodes_removed: on.difference(&nn).count(),
        edges_added: ne.difference(&oe).count(),
        edges_removed: oe.difference(&ne).count(),
    }
}

fn ascii(m: &Model, kind: Kind) -> Vec<String> {
    let name = |id: &String| m.nodes.get(id).cloned().unwrap_or_else(|| id.clone());
    let mut out: Vec<String> = Vec::new();
    for (a, b, l) in &m.edges {
        let l = l.split_once('|').map_or(l.as_str(), |(_, n)| n);
        out.push(if l.is_empty() || kind == Kind::Er {
            format!("{} -> {}", name(a), name(b))
        } else {
            format!("{} -> {} : {l}", name(a), name(b))
        });
    }
    let linked: BTreeSet<&String> = m.edges.iter().flat_map(|(a, b, _)| [a, b]).collect();
    out.extend(
        m.nodes
            .iter()
            .filter(|(id, _)| !linked.contains(id))
            .map(|(_, l)| l.clone()),
    );
    out
}

/// `svg=<path>` of a block, when it is a safe project-relative path.
fn svg_path(b: &Block) -> Option<&str> {
    let p = b.params.get("svg")?;
    let path = Path::new(p);
    (!path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        && p.to_ascii_lowercase().ends_with(".svg"))
    .then_some(p.as_str())
}

/// Plan sync for every block: regenerate each from `g` and compare. `root` is checked for a
/// missing SVG output (`svg=`).
pub fn plan_in(root: &Path, g: &Graph, blocks: &[Block]) -> Vec<Change> {
    let mut out = plan(g, blocks);
    for (c, b) in out.iter_mut().zip(blocks) {
        if c.status == Status::UpToDate && svg_path(b).is_some_and(|p| !root.join(p).is_file()) {
            c.status = Status::Stale;
        }
    }
    out
}

/// Plan sync for every block: regenerate each from `g` and compare.
pub fn plan(g: &Graph, blocks: &[Block]) -> Vec<Change> {
    blocks
        .iter()
        .map(|b| match model(g, b.kind, &b.params) {
            Err(e) => invalid(&b.file, b.line, Some(b.kind), e),
            Ok(m) => {
                let text = render(&m, b.kind, b.format);
                let current = b.generated.trim();
                let edited = b.hash.as_ref().is_some_and(|h| *h != hash(current));
                let status = if edited {
                    Status::Edited
                } else if current.replace("\r\n", "\n") == text {
                    Status::UpToDate
                } else {
                    Status::Stale
                };
                Change {
                    file: b.file.clone(),
                    line: b.line,
                    kind: Some(b.kind),
                    status,
                    error: None,
                    counts: counts(current, &text),
                    ascii: ascii(&m, b.kind),
                    text,
                    model: Some(m),
                }
            }
        })
        .collect()
}

/// The full marker line for `b` after writing `generated`.
fn open_marker(b: &Block, host: Host, generated: &str) -> String {
    let mut p = format!("kind={}", b.kind.id());
    if host == Host::Markdown {
        let _ = write!(p, " format={}", b.format.id());
    }
    for (k, v) in &b.params {
        let _ = write!(p, " {k}={v}");
    }
    let _ = write!(p, " hash={}", hash(generated));
    match host {
        Host::Markdown => format!("<!-- kor:diagram {p} -->"),
        Host::Raw(f) => format!("{} kor:diagram {p}", f.comment()),
        Host::Excalidraw => format!("kor:diagram {p}"),
    }
}

fn close_marker(host: Host) -> String {
    match host {
        Host::Markdown => "<!-- /kor:diagram -->".into(),
        Host::Raw(f) => format!("{} /kor:diagram", f.comment()),
        Host::Excalidraw => String::new(),
    }
}

/// Rewrite the blocks of `changes` (those `Stale`, and `Edited` ones when `force`) in their
/// files. Returns the files written. Lines outside the markers and kept lines are untouched; the
/// file's line ending style is kept.
///
/// # Errors
/// I/O errors; a file whose blocks moved since the scan is skipped with an error.
pub fn apply(
    root: &Path,
    blocks: &[Block],
    changes: &[Change],
    force: bool,
) -> Result<Vec<String>, String> {
    let mut by_file: BTreeMap<&str, Vec<(&Block, &Change)>> = BTreeMap::new();
    for c in changes {
        let write = c.status == Status::Stale || (force && c.status == Status::Edited);
        if !write {
            continue;
        }
        if let Some(b) = blocks.iter().find(|b| b.file == c.file && b.line == c.line) {
            by_file.entry(b.file.as_str()).or_default().push((b, c));
        }
    }
    let mut written = Vec::new();
    // `svg=` pictures of every block being written.
    for (b, c) in by_file.values().flatten() {
        if let (Some(p), Some(m)) = (svg_path(b), &c.model) {
            let dst = root.join(p);
            if let Some(d) = dst.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("{p}: {e}"))?;
            }
            std::fs::write(&dst, crate::diagram_render::svg(m)).map_err(|e| format!("{p}: {e}"))?;
            written.push(p.to_owned());
        }
    }
    for (file, mut items) in by_file {
        let path: PathBuf = root.join(file);
        let host = host_of(&path).ok_or_else(|| format!("{file}: not a diagram host"))?;
        if host == Host::Excalidraw {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
            let mut doc: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| format!("{file}: {e}"))?;
            if let Some(m) = items.first().and_then(|(_, c)| c.model.as_ref()) {
                crate::diagram_render::excalidraw_apply(&mut doc, m);
                let out = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
                std::fs::write(&path, out + "\n").map_err(|e| format!("{file}: {e}"))?;
                written.push(file.to_owned());
            }
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
        let crlf = text.contains("\r\n");
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        // Bottom-up so earlier ranges stay valid.
        items.sort_by(|a, b| b.0.range.0.cmp(&a.0.range.0));
        for (b, c) in items {
            let (start, end) = b.range;
            if end >= lines.len() || marker(&lines[start], host).is_none_or(|(_, e)| e) {
                return Err(format!(
                    "{file}:{}: the file changed since it was read",
                    b.line
                ));
            }
            let mut body = vec![open_marker(b, host, &c.text)];
            if host == Host::Markdown {
                body.push(format!("```{}", b.format.id()));
            }
            body.extend(c.text.lines().map(str::to_owned));
            if !b.kept.is_empty() {
                body.push(format!("{} {KEEP}", b.format.comment()));
                body.extend(b.kept.iter().cloned());
            }
            if host == Host::Markdown {
                body.push("```".into());
            }
            body.push(close_marker(host));
            lines.splice(start..=end, body);
        }
        let nl = if crlf { "\r\n" } else { "\n" };
        let mut out = lines.join(nl);
        out.push_str(nl);
        let tmp = path.with_extension("kor-diagram.tmp");
        std::fs::write(&tmp, out).map_err(|e| format!("{file}: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{file}: {e}"))?;
        written.push(file.to_owned());
    }
    Ok(written)
}

/// Append an empty marked block to `file` (created when missing); the next sync fills it.
///
/// # Errors
/// Not a diagram file type, a path outside the project, or I/O errors.
pub fn add(
    root: &Path,
    file: &str,
    kind: Kind,
    format: Format,
    params: &BTreeMap<String, String>,
) -> Result<(), String> {
    let p = Path::new(file);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("{file}: must be a path inside the project"));
    }
    let path = root.join(p);
    let host = host_of(&path)
        .ok_or_else(|| format!("{file}: use .md, .mmd, .puml, .d2 or .excalidraw"))?;
    if host == Host::Excalidraw {
        // A drawing with only the marker text; the next sync draws the shapes.
        let mut p = format!("kor:diagram kind={}", kind.id());
        for (k, v) in params {
            let _ = write!(p, " {k}={v}");
        }
        let mut doc = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .unwrap_or_else(|| serde_json::json!({ "type": "excalidraw", "version": 2, "source": "kor", "elements": [], "appState": { "viewBackgroundColor": "#ffffff" }, "files": {} }));
        if !doc["elements"].is_array() {
            doc["elements"] = serde_json::json!([]);
        }
        let marker = serde_json::json!({
            "id": "kor-marker", "type": "text", "x": 20, "y": -40, "width": 400, "height": 20, "angle": 0,
            "strokeColor": "#868e96", "backgroundColor": "transparent", "fillStyle": "solid", "strokeWidth": 1,
            "strokeStyle": "solid", "roughness": 0, "opacity": 100, "groupIds": [], "frameId": null,
            "roundness": null, "seed": 1, "version": 1, "versionNonce": 1, "isDeleted": false,
            "boundElements": null, "updated": 1, "link": null, "locked": false, "text": p,
            "originalText": p, "fontSize": 12, "fontFamily": 3, "textAlign": "left", "verticalAlign": "top",
            "containerId": null, "lineHeight": 1.25, "autoResize": true
        });
        doc["elements"].as_array_mut().expect("array").push(marker);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let out = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        return std::fs::write(&path, out + "\n").map_err(|e| format!("{file}: {e}"));
    }
    let format = match host {
        Host::Raw(f) => f,
        Host::Markdown => format,
        Host::Excalidraw => unreachable!("handled above"),
    };
    let b = Block {
        file: file.to_owned(),
        line: 0,
        kind,
        format,
        params: params.clone(),
        hash: None,
        generated: String::new(),
        kept: Vec::new(),
        range: (0, 0),
    };
    let mut open = open_marker(&b, host, "");
    // A fresh block has no hash: the first sync must not see it as hand-edited.
    if let Some(i) = open.find(" hash=") {
        let end = open[i + 1..].find(' ').map_or(open.len(), |j| i + 1 + j);
        open.replace_range(i..end, "");
    }
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&open);
    text.push('\n');
    if host == Host::Markdown {
        let _ = writeln!(text, "```{}\n```", format.id());
    }
    text.push_str(&close_marker(host));
    text.push('\n');
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{file}: {e}"))
}

/// Summary of a sync, e.g. for a PR description: "docs/arch.md:3 architecture: +2 nodes, +1 edge".
pub fn summary_line(c: &Change) -> String {
    let k = c.kind.map_or("?", Kind::id);
    let n = &c.counts;
    format!(
        "{}:{} {k}: +{} −{} nodes, +{} −{} edges",
        c.file, c.line, n.nodes_added, n.nodes_removed, n.edges_added, n.edges_removed
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let w = |p: &str, t: &str| {
            let p = d.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "src/main.rs",
            "mod auth;\nmod db;\nuse crate::auth::login;\npub fn main_entry() { login(\"u\"); }\n",
        );
        w(
            "src/auth/mod.rs",
            "use crate::db::Db;\npub fn login(u: &str) -> bool { Db::connect().find(u) }\n",
        );
        w(
            "src/db/mod.rs",
            "pub struct Db;\nimpl Db {\n    pub fn connect() -> Self { Db }\n    pub fn find(&self, _u: &str) -> bool { true }\n}\n",
        );
        w(
            "migrations/1.sql",
            "CREATE TABLE users (id INT PRIMARY KEY, name TEXT);\nCREATE TABLE posts (id INT, user_id INT REFERENCES users(id));\n",
        );
        d
    }

    #[test]
    fn markers_in_every_host() {
        let md = "# A\n<!-- kor:diagram kind=architecture depth=2 hash=0000 -->\n```mermaid\nflowchart LR\n  a --> b\n%% kor:keep\nstyle a fill:#f00\n```\n<!-- /kor:diagram -->\ntail\n";
        let (b, e) = blocks_in("a.md", md, Host::Markdown);
        assert!(e.is_empty());
        assert_eq!(b[0].kind, Kind::Architecture);
        assert_eq!(b[0].format, Format::Mermaid);
        assert_eq!(b[0].params.get("depth").map(String::as_str), Some("2"));
        assert_eq!(b[0].generated, "flowchart LR\n  a --> b");
        assert_eq!(b[0].kept, vec!["style a fill:#f00".to_owned()]);
        let puml = "' kor:diagram kind=er\n@startuml\n@enduml\n' /kor:diagram\n";
        let (b, _) = blocks_in("x.puml", puml, Host::Raw(Format::Plantuml));
        assert_eq!((b[0].kind, b[0].format), (Kind::Er, Format::Plantuml));
        let (_, e) = blocks_in(
            "x.md",
            "<!-- kor:diagram kind=nope -->\n<!-- /kor:diagram -->\n",
            Host::Markdown,
        );
        assert_eq!(e.len(), 1);
        let (_, e) = blocks_in("x.md", "<!-- kor:diagram kind=er -->\n", Host::Markdown);
        assert_eq!(e[0].1, "missing end marker");
    }

    #[test]
    fn sync_check_keep_and_edit_detection() {
        let d = fixture();
        let root = d.path();
        let mut g = Graph::open(root).unwrap();
        g.update().unwrap();
        add(
            root,
            "docs/arch.md",
            Kind::Architecture,
            Format::Mermaid,
            &BTreeMap::from([("depth".into(), "2".into())]),
        )
        .unwrap();
        add(
            root,
            "docs/calls.mmd",
            Kind::Sequence,
            Format::Mermaid,
            &BTreeMap::from([("from".into(), "main_entry".into())]),
        )
        .unwrap();
        add(
            root,
            "docs/schema.puml",
            Kind::Er,
            Format::Plantuml,
            &BTreeMap::new(),
        )
        .unwrap();
        add(
            root,
            "docs/deps.d2",
            Kind::Dependency,
            Format::D2,
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(add(root, "../x.md", Kind::Er, Format::Mermaid, &BTreeMap::new()).is_err());
        let (blocks, bad) = scan(root);
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(blocks.len(), 4);
        let changes = plan(&g, &blocks);
        assert!(
            changes.iter().all(|c| c.status == Status::Stale),
            "{changes:?}"
        );
        let arch = changes.iter().find(|c| c.file == "docs/arch.md").unwrap();
        assert!(
            arch.text.contains("src/auth") && arch.text.contains("-->"),
            "{}",
            arch.text
        );
        assert!(arch.counts.nodes_added >= 2 && arch.counts.edges_added >= 1);
        let seq = changes.iter().find(|c| c.file == "docs/calls.mmd").unwrap();
        assert!(
            seq.text.starts_with("sequenceDiagram") && seq.text.contains(": login"),
            "{}",
            seq.text
        );
        let er = changes
            .iter()
            .find(|c| c.file == "docs/schema.puml")
            .unwrap();
        assert!(
            er.text.contains("entity users") && er.text.contains("posts }o--|| users"),
            "{}",
            er.text
        );
        let written = apply(root, &blocks, &changes, false).unwrap();
        assert_eq!(written.len(), 4);

        // Now in sync: check passes.
        let (blocks, _) = scan(root);
        assert!(
            plan(&g, &blocks)
                .iter()
                .all(|c| c.status == Status::UpToDate)
        );

        // A kept line survives; a hand edit of the generated part is detected.
        let arch_path = root.join("docs/arch.md");
        let t = std::fs::read_to_string(&arch_path).unwrap();
        let t = t.replace(
            "```\n<!-- /kor:diagram -->",
            "%% kor:keep\nstyle src_auth fill:#fdd\n```\n<!-- /kor:diagram -->",
        );
        std::fs::write(&arch_path, &t).unwrap();
        let (blocks, _) = scan(root);
        assert!(
            plan(&g, &blocks)
                .iter()
                .all(|c| c.status == Status::UpToDate)
        );
        std::fs::write(&arch_path, t.replace("flowchart LR", "flowchart TB")).unwrap();
        let (blocks, _) = scan(root);
        let changes = plan(&g, &blocks);
        let c = changes.iter().find(|c| c.file == "docs/arch.md").unwrap();
        assert_eq!(c.status, Status::Edited);
        assert!(apply(root, &blocks, &changes, false).unwrap().is_empty());
        apply(root, &blocks, &changes, true).unwrap();
        let after = std::fs::read_to_string(&arch_path).unwrap();
        assert!(after.contains("flowchart LR") && after.contains("style src_auth fill:#fdd"));

        // Code change: a new module makes the dependency diagram stale.
        std::fs::write(
            root.join("src/db/cache.rs"),
            "use crate::auth::login;\npub fn warm() { login(\"x\"); }\n",
        )
        .unwrap();
        g.update().unwrap();
        let (blocks, _) = scan(root);
        let changes = plan(&g, &blocks);
        let deps = changes.iter().find(|c| c.file == "docs/deps.d2").unwrap();
        assert_eq!(deps.status, Status::Stale);
        assert!(deps.counts.nodes_added >= 1, "{}", summary_line(deps));
        assert!(!deps.ascii.is_empty());
    }

    #[test]
    fn excalidraw_and_svg_outputs() {
        let d = fixture();
        let root = d.path();
        let mut g = Graph::open(root).unwrap();
        g.update().unwrap();
        add(
            root,
            "docs/arch.excalidraw",
            Kind::Architecture,
            Format::Mermaid,
            &BTreeMap::from([("depth".into(), "2".into())]),
        )
        .unwrap();
        add(
            root,
            "README.md",
            Kind::Dependency,
            Format::Mermaid,
            &BTreeMap::from([("svg".into(), "docs/deps.svg".into())]),
        )
        .unwrap();
        let (blocks, bad) = scan(root);
        assert!(bad.is_empty(), "{bad:?}");
        let changes = plan_in(root, &g, &blocks);
        assert!(changes.iter().all(|c| c.status == Status::Stale));
        apply(root, &blocks, &changes, false).unwrap();
        assert!(
            std::fs::read_to_string(root.join("docs/deps.svg"))
                .unwrap()
                .starts_with("<svg")
        );
        let drawing = std::fs::read_to_string(root.join("docs/arch.excalidraw")).unwrap();
        assert!(drawing.contains("kor-n-") && drawing.contains("kor:diagram kind=architecture"));
        let (blocks, _) = scan(root);
        assert!(
            plan_in(root, &g, &blocks)
                .iter()
                .all(|c| c.status == Status::UpToDate)
        );
        std::fs::remove_file(root.join("docs/deps.svg")).unwrap();
        let (blocks, _) = scan(root);
        assert!(
            plan_in(root, &g, &blocks)
                .iter()
                .any(|c| c.status == Status::Stale),
            "a missing SVG is stale"
        );
    }

    #[test]
    fn unknown_sequence_symbol_is_invalid() {
        let d = fixture();
        let mut g = Graph::open(d.path()).unwrap();
        g.update().unwrap();
        assert!(
            model(
                &g,
                Kind::Sequence,
                &BTreeMap::from([("from".into(), "nope".into())])
            )
            .is_err()
        );
        assert!(model(&g, Kind::Sequence, &BTreeMap::new()).is_err());
        assert!(
            model(&g, Kind::Deploy, &BTreeMap::new())
                .unwrap()
                .nodes
                .is_empty()
        );
    }
}
