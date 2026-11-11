//! `kor visualize` (spec §10): LLM-free views of the project graph (§7) — a module-level
//! architecture map, the call graph and the data flow (code → tables / environment variables) —
//! rendered as the interactive kor-graph HTML viewer, Mermaid or SVG.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_graph::{EdgeKind, Graph, GraphNode, NodeKind};
use serde::Serialize;
use serde_json::json;

/// Nodes kept in one call-graph view (highest degree first).
const CALL_CAP: usize = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewKind {
    Modules,
    Calls,
    DataFlow,
}

impl ViewKind {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Modules => "modules",
            Self::Calls => "calls",
            Self::DataFlow => "dataflow",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Modules => "Architecture map (modules)",
            Self::Calls => "Call graph",
            Self::DataFlow => "Data flow (tables and environment)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VNode {
    pub id: String,
    pub label: String,
    /// Directory, node kind or `table` / `env`; colours the node.
    pub group: String,
    pub file: String,
    pub line: Option<u32>,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VEdge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub weight: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub kind: ViewKind,
    pub nodes: Vec<VNode>,
    pub edges: Vec<VEdge>,
    /// Nodes left out by the cap.
    pub omitted: usize,
}

#[derive(Default)]
struct Builder {
    nodes: Vec<VNode>,
    index: HashMap<String, usize>,
    edges: BTreeMap<(usize, usize, String), u32>,
}

impl Builder {
    fn node(&mut self, n: VNode) -> usize {
        if let Some(&i) = self.index.get(&n.id) {
            self.nodes[i].weight += n.weight;
            return i;
        }
        self.index.insert(n.id.clone(), self.nodes.len());
        self.nodes.push(n);
        self.nodes.len() - 1
    }
    fn edge(&mut self, a: usize, b: usize, label: &str) {
        *self.edges.entry((a, b, label.to_owned())).or_default() += 1;
    }
    fn finish(self, kind: ViewKind, omitted: usize) -> View {
        let edges = self
            .edges
            .into_iter()
            .map(|((from, to, label), weight)| VEdge {
                from,
                to,
                label,
                weight,
            })
            .collect();
        View {
            kind,
            nodes: self.nodes,
            edges,
            omitted,
        }
    }
}

fn file_of(n: &GraphNode) -> Option<String> {
    n.file
        .as_ref()
        .map(|f| f.to_string_lossy().replace('\\', "/"))
}

fn dir_of(file: &str) -> String {
    file.rsplit_once('/')
        .map_or_else(|| ".".to_owned(), |(d, _)| d.to_owned())
}

fn in_scope(file: &str, scope: &str) -> bool {
    scope.is_empty() || file == scope || file.starts_with(&format!("{scope}/"))
}

/// The scope as a `/`-separated path relative to the graph root ("" = everything).
pub fn scope_of(root: &Path, path: Option<&Path>) -> String {
    let Some(p) = path else {
        return String::new();
    };
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    let abs = abs.canonicalize().unwrap_or(abs);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = kor_tools::without_verbatim(&abs);
    let base = kor_tools::without_verbatim(&root);
    rel.strip_prefix(&base)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_owned()
}

fn code_dep(k: EdgeKind) -> Option<&'static str> {
    match k {
        EdgeKind::Calls => Some("calls"),
        EdgeKind::Imports => Some("imports"),
        EdgeKind::Implements => Some("implements"),
        EdgeKind::Extends => Some("extends"),
        _ => None,
    }
}

/// Directories as nodes (weight = files), code dependencies between different directories as
/// edges (weight = number of underlying edges).
pub fn modules(g: &Graph, scope: &str) -> View {
    let mut b = Builder::default();
    for n in g.nodes() {
        if n.kind == NodeKind::Module
            && let Some(f) = file_of(n)
            && in_scope(&f, scope)
        {
            let d = dir_of(&f);
            b.node(VNode {
                id: d.clone(),
                label: d.clone(),
                group: d.split('/').next().unwrap_or(".").to_owned(),
                file: d,
                line: None,
                weight: 1,
            });
        }
    }
    for e in g.edges() {
        let (Some(label), Some(a), Some(t)) =
            (code_dep(e.kind), g.node(&e.source), g.node(&e.target))
        else {
            continue;
        };
        let (Some(fa), Some(ft)) = (file_of(a), file_of(t)) else {
            continue;
        };
        let (da, dt) = (dir_of(&fa), dir_of(&ft));
        if da != dt
            && let (Some(&ia), Some(&it)) = (b.index.get(&da), b.index.get(&dt))
        {
            b.edge(ia, it, label);
        }
    }
    b.finish(ViewKind::Modules, 0)
}

/// Functions and types joined by `calls`, the `CALL_CAP` best-connected ones.
pub fn calls(g: &Graph, scope: &str) -> View {
    let mut candidates: Vec<&GraphNode> = g
        .nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Struct | NodeKind::Trait
            )
        })
        .filter(|n| file_of(n).is_some_and(|f| in_scope(&f, scope)))
        .filter(|n| n.degree > 0)
        .collect();
    candidates.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.id.cmp(&b.id)));
    let omitted = candidates.len().saturating_sub(CALL_CAP);
    candidates.truncate(CALL_CAP);
    let mut b = Builder::default();
    for n in &candidates {
        let file = file_of(n).unwrap_or_default();
        b.node(VNode {
            id: n.id.clone(),
            label: n.name.clone(),
            group: dir_of(&file),
            file,
            line: n.span.map(|s| s.0),
            weight: n.degree,
        });
    }
    for e in g.edges() {
        if e.kind == EdgeKind::Calls
            && let (Some(&a), Some(&t)) = (b.index.get(&e.source), b.index.get(&e.target))
            && a != t
        {
            b.edge(a, t, "calls");
        }
    }
    b.finish(ViewKind::Calls, omitted)
}

/// Source files → tables (`reads_table` / `writes_table`) and env vars (`reads_env`).
pub fn data_flow(g: &Graph, scope: &str) -> View {
    let mut b = Builder::default();
    for e in g.edges() {
        let label = match e.kind {
            EdgeKind::ReadsTable => "reads",
            EdgeKind::WritesTable => "writes",
            EdgeKind::ReadsEnv => "reads env",
            _ => continue,
        };
        let (Some(src), Some(dst)) = (g.node(&e.source), g.node(&e.target)) else {
            continue;
        };
        let Some(file) = file_of(src).filter(|f| in_scope(f, scope)) else {
            continue;
        };
        let a = b.node(VNode {
            id: format!("file:{file}"),
            label: file.clone(),
            group: "code".into(),
            file,
            line: None,
            weight: 1,
        });
        let group = if dst.kind == NodeKind::EnvVar {
            "env"
        } else {
            "table"
        };
        let t = b.node(VNode {
            id: dst.id.clone(),
            label: dst.name.clone(),
            group: group.into(),
            file: file_of(dst).unwrap_or_default(),
            line: dst.span.map(|s| s.0),
            weight: 1,
        });
        b.edge(a, t, label);
    }
    b.finish(ViewKind::DataFlow, 0)
}

/// All three views.
pub fn views(g: &Graph, scope: &str) -> Vec<View> {
    vec![modules(g, scope), calls(g, scope), data_flow(g, scope)]
}

fn groups(v: &View) -> Vec<&str> {
    let mut gs: Vec<&str> = v.nodes.iter().map(|n| n.group.as_str()).collect();
    gs.sort_unstable();
    gs.dedup();
    gs
}

fn mermaid_label(s: &str) -> String {
    s.chars()
        .take(60)
        .map(|c| {
            if matches!(c, '"' | '[' | ']' | '<' | '>' | '{' | '}' | '|' | '(' | ')') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Mermaid `flowchart LR`.
pub fn mermaid(v: &View) -> String {
    let mut s = String::from("flowchart LR\n");
    for (i, n) in v.nodes.iter().enumerate() {
        let shape = match n.group.as_str() {
            "table" => ("[(", ")]"),
            "env" => ("{{", "}}"),
            _ => ("[", "]"),
        };
        let _ = writeln!(
            s,
            "  n{i}{}\"{}\"{}",
            shape.0,
            mermaid_label(&n.label),
            shape.1
        );
    }
    for e in &v.edges {
        let _ = writeln!(s, "  n{} -->|{} ×{}| n{}", e.from, e.label, e.weight, e.to);
    }
    s
}

/// The interactive kor-graph viewer (search, pan, zoom, click for `file:line`) with this view's data.
///
/// # Errors
/// JSON serialization (never in practice).
pub fn html(v: &View) -> Result<String, String> {
    let gs = groups(v);
    let gi: HashMap<&str, usize> = gs.iter().enumerate().map(|(i, g)| (*g, i)).collect();
    let nodes: Vec<serde_json::Value> = v
        .nodes
        .iter()
        .map(|n| json!({"id": n.id, "n": n.label, "k": n.group, "f": n.file, "l": n.line, "c": gi.get(n.group.as_str()).copied().unwrap_or(0), "d": n.weight}))
        .collect();
    let links: Vec<[usize; 2]> = v.edges.iter().map(|e| [e.from, e.to]).collect();
    let labels: BTreeMap<usize, &str> = gs.iter().enumerate().map(|(i, g)| (i, *g)).collect();
    let page = kor_graph::export::html_viewer(
        &json!({"nodes": nodes, "links": links, "communities": labels}),
    )
    .map_err(|e| e.to_string())?;
    Ok(page.replacen(
        "<title>Kor graph</title>",
        &format!("<title>Kor · {}</title>", v.kind.title()),
        1,
    ))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Longest-path layers (sources left); cycles are cut after `n` passes.
fn layers(n: usize, edges: &[VEdge]) -> Vec<usize> {
    let mut depth = vec![0usize; n];
    let cap = n.min(30);
    for _ in 0..cap {
        let mut changed = false;
        for e in edges {
            if e.from != e.to && depth[e.to] <= depth[e.from] && depth[e.from] < cap {
                depth[e.to] = depth[e.from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    depth
}

/// A static layered SVG (columns by dependency depth), light and dark aware.
pub fn svg(v: &View) -> String {
    const W: usize = 220;
    const H: usize = 26;
    const GAP_X: usize = 90;
    const GAP_Y: usize = 14;
    let depth = layers(v.nodes.len(), &v.edges);
    let mut rows = vec![0usize; depth.iter().max().map_or(1, |m| m + 1)];
    let mut pos = vec![(0usize, 0usize); v.nodes.len()];
    for (i, d) in depth.iter().enumerate() {
        pos[i] = (20 + d * (W + GAP_X), 50 + rows[*d] * (H + GAP_Y));
        rows[*d] += 1;
    }
    let width = 40 + rows.len() * (W + GAP_X);
    let height = 70 + rows.iter().max().copied().unwrap_or(0) * (H + GAP_Y);
    let gs = groups(v);
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" font-family=\"system-ui,sans-serif\" font-size=\"12\">\n\
<style>text{{fill:#1c1c1c}}.e{{stroke:#8a8a8a;fill:none}}.bg{{fill:#f7f7f5}}@media (prefers-color-scheme:dark){{text{{fill:#ececec}}.bg{{fill:#141414}}}}</style>\n\
<defs><marker id=\"a\" viewBox=\"0 0 8 8\" refX=\"8\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M0,0 L8,4 L0,8 z\" fill=\"#8a8a8a\"/></marker></defs>\n\
<rect class=\"bg\" width=\"100%\" height=\"100%\"/>\n<text x=\"20\" y=\"28\" font-size=\"15\" font-weight=\"600\">{}</text>\n",
        xml(v.kind.title())
    );
    for e in &v.edges {
        let (a, b) = (pos[e.from], pos[e.to]);
        let _ = writeln!(
            s,
            "<path class=\"e\" marker-end=\"url(#a)\" d=\"M{} {} L{} {}\"><title>{} ×{}</title></path>",
            a.0 + W,
            a.1 + H / 2,
            b.0,
            b.1 + H / 2,
            xml(&e.label),
            e.weight
        );
    }
    for (i, n) in v.nodes.iter().enumerate() {
        let (x, y) = pos[i];
        let hue = gs.iter().position(|g| *g == n.group).unwrap_or(0) * 137 % 360;
        let label: String = n.label.chars().take(32).collect();
        let tip = if n.file.is_empty() {
            n.label.clone()
        } else {
            n.line
                .map_or_else(|| n.file.clone(), |l| format!("{}:{l}", n.file))
        };
        let _ = writeln!(
            s,
            "<g><title>{}</title><rect x=\"{x}\" y=\"{y}\" width=\"{W}\" height=\"{H}\" rx=\"5\" fill=\"hsl({hue},55%,50%)\" fill-opacity=\"0.18\" stroke=\"hsl({hue},55%,45%)\"/><text x=\"{}\" y=\"{}\">{}</text></g>",
            xml(&tip),
            x + 8,
            y + 17,
            xml(&label)
        );
    }
    s.push_str("</svg>\n");
    s
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Html,
    Mermaid,
    Svg,
}

impl std::str::FromStr for Format {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "html" => Self::Html,
            "mermaid" | "mmd" => Self::Mermaid,
            "svg" => Self::Svg,
            other => return Err(format!("unknown format `{other}` (html, mermaid, svg)")),
        })
    }
}

/// What was written.
#[derive(Debug, Clone, Serialize)]
pub struct Written {
    /// The file to open: `index.html`, `visualize.md` or the architecture SVG.
    pub main: PathBuf,
    pub files: Vec<PathBuf>,
    pub counts: Vec<(ViewKind, usize, usize, usize)>,
}

/// Default output folder: `<root>/.kor/graph/export/visualize`.
pub fn default_dir(root: &Path) -> PathBuf {
    kor_graph::Format::Html
        .default_path(root)
        .with_file_name("visualize")
}

fn put(path: &Path, text: &str, out: &mut Vec<PathBuf>) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    out.push(path.to_path_buf());
    Ok(())
}

/// Write the views in `format` into `dir`.
///
/// # Errors
/// I/O errors.
pub fn write(views: &[View], format: Format, dir: &Path) -> Result<Written, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    let counts = views
        .iter()
        .map(|v| (v.kind, v.nodes.len(), v.edges.len(), v.omitted))
        .collect();
    let main = match format {
        Format::Html => {
            let mut items = String::new();
            for v in views {
                let name = format!("{}.html", v.kind.slug());
                put(&dir.join(&name), &html(v)?, &mut files)?;
                let _ = write!(
                    items,
                    "<li><a href=\"{name}\">{}</a> <small>{} nodes, {} edges{}</small></li>",
                    xml(v.kind.title()),
                    v.nodes.len(),
                    v.edges.len(),
                    if v.omitted > 0 {
                        format!(", {} omitted", v.omitted)
                    } else {
                        String::new()
                    }
                );
            }
            let index = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Kor visualize</title>\
<style>:root{{--bg:#f7f7f5;--fg:#1c1c1c;--muted:#6b6b6b}}@media (prefers-color-scheme:dark){{:root{{--bg:#141414;--fg:#ececec;--muted:#9a9a9a}}}}\
body{{margin:0;padding:24px 16px;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,sans-serif}}a{{color:inherit}}small{{color:var(--muted)}}li{{margin:8px 0}}</style></head>\
<body><main><h1>Kor visualize</h1><p><small>Generated from the project graph without a model.</small></p><ul>{items}</ul></main></body></html>\n"
            );
            let p = dir.join("index.html");
            put(&p, &index, &mut files)?;
            p
        }
        Format::Mermaid => {
            let mut md = String::from("# Kor visualize\n");
            for v in views {
                let _ = write!(
                    md,
                    "\n## {}\n\n```mermaid\n{}```\n",
                    v.kind.title(),
                    mermaid(v)
                );
            }
            let p = dir.join("visualize.md");
            put(&p, &md, &mut files)?;
            p
        }
        Format::Svg => {
            for v in views {
                put(
                    &dir.join(format!("{}.svg", v.kind.slug())),
                    &svg(v),
                    &mut files,
                )?;
            }
            files[0].clone()
        }
    };
    Ok(Written {
        main,
        files,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let w = |p: &str, s: &str| {
            let f = d.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, s).unwrap();
        };
        w(
            "src/main.rs",
            "mod api;\nmod db;\nfn main() { api::serve(); }\n",
        );
        w(
            "src/api/mod.rs",
            "use crate::db::save;\npub fn serve() { handle(); }\nfn handle() { save(); }\n",
        );
        w(
            "src/db/mod.rs",
            "pub fn save() { let _ = std::env::var(\"DATABASE_URL\"); }\n",
        );
        d
    }

    #[test]
    fn three_views_render_in_every_format() {
        let dir = project();
        let mut graph = Graph::open(dir.path()).unwrap();
        graph.update().unwrap();
        let vs = views(&graph, "");
        let mods = &vs[0];
        let names: Vec<&str> = mods.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            names.contains(&"src/api") && names.contains(&"src/db"),
            "{names:?}"
        );
        assert!(
            mods.edges.iter().any(
                |e| mods.nodes[e.from].label == "src/api" && mods.nodes[e.to].label == "src/db"
            ),
            "{:?}",
            mods.edges
        );
        let call = &vs[1];
        assert!(
            call.edges
                .iter()
                .any(|e| call.nodes[e.from].label == "serve" && call.nodes[e.to].label == "handle"),
            "{call:?}"
        );
        let flow = &vs[2];
        assert!(
            flow.edges
                .iter()
                .any(|e| flow.nodes[e.from].label == "src/db/mod.rs"
                    && flow.nodes[e.to].group == "env"
                    && e.label == "reads env"),
            "{flow:?}"
        );
        assert!(
            mermaid(mods).starts_with("flowchart LR")
                && mermaid(flow).contains("{{\"DATABASE_URL\"}}")
        );
        let svg_text = svg(call);
        assert!(svg_text.starts_with("<svg") && svg_text.contains(">serve</text>"));
        let page = html(flow).unwrap();
        assert!(
            page.contains("Data flow")
                && page.contains("DATABASE_URL")
                && !page.contains("/*DATA*/")
        );
        // Scope: only src/db.
        let scoped = modules(&graph, "src/db");
        assert_eq!(scoped.nodes.len(), 1);
        let out = tempfile::tempdir().unwrap();
        for (fmt, main) in [
            (Format::Html, "index.html"),
            (Format::Mermaid, "visualize.md"),
            (Format::Svg, "modules.svg"),
        ] {
            let w = write(&vs, fmt, &out.path().join(format!("{fmt:?}"))).unwrap();
            assert!(w.main.ends_with(main) && w.main.is_file());
        }
        assert_eq!("MMD".parse::<Format>(), Ok(Format::Mermaid));
        assert!("png".parse::<Format>().is_err());
    }

    #[test]
    fn layers_survive_cycles_and_scope_is_relative() {
        let e = |from, to| VEdge {
            from,
            to,
            label: String::new(),
            weight: 1,
        };
        assert_eq!(layers(3, &[e(0, 1), e(1, 2)]), [0, 1, 2]);
        let cyc = layers(2, &[e(0, 1), e(1, 0)]);
        assert!(cyc.iter().all(|d| *d <= 2));
        let d = project();
        assert_eq!(scope_of(d.path(), Some(Path::new("src/db"))), "src/db");
        assert_eq!(scope_of(d.path(), None), "");
        assert!(in_scope("src/db/mod.rs", "src/db") && !in_scope("src/dbx/a.rs", "src/db"));
    }
}
