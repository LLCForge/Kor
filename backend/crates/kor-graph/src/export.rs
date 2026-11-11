//! Exports: own JSON, graphify-compatible node-link `graph.json`, Mermaid, Obsidian vault, and a
//! single self-contained HTML viewer (inline JS, no CDN).

use crate::graph::Graph;
use crate::model::{Confidence, EdgeKind, GraphNode, NodeKind};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Kor's own schema: `{version, root, nodes, edges, communities}`.
    Json,
    /// graphify / networkx node-link: `{directed, multigraph, graph, nodes:[{id,..}], links:[{source,target,..}]}`.
    Graphify,
    /// Mermaid `flowchart` of the highest-degree nodes.
    Mermaid,
    /// A directory with one Markdown note per node, linked with `[[..]]`.
    Obsidian,
    /// One HTML file with an inline force-directed viewer.
    Html,
}

impl FromStr for Format {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(
            match s.trim_start_matches("--").to_ascii_lowercase().as_str() {
                "json" => Self::Json,
                "graphify" => Self::Graphify,
                "mermaid" | "mmd" => Self::Mermaid,
                "obsidian" => Self::Obsidian,
                "html" => Self::Html,
                other => return Err(Error::Format(other.to_owned())),
            },
        )
    }
}

impl Format {
    /// Default output path under `<root>/.kor/graph/export/`.
    pub fn default_path(self, root: &Path) -> PathBuf {
        let dir = crate::store::graph_dir(root).join("export");
        match self {
            Self::Json => dir.join("kor-graph.json"),
            Self::Graphify => dir.join("graph.json"),
            Self::Mermaid => dir.join("graph.mmd"),
            Self::Obsidian => dir.join("obsidian"),
            Self::Html => dir.join("graph.html"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub path: PathBuf,
    pub files_written: usize,
    /// Nodes left out (Mermaid/HTML cap to the highest-degree nodes).
    pub nodes_omitted: usize,
}

const MERMAID_CAP: usize = 200;
const HTML_CAP: usize = 3000;

fn file_str(n: &GraphNode) -> String {
    n.file
        .as_ref()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn write(path: &Path, content: &str) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

impl Graph {
    /// Write the graph in `format` to `out` (a file, or a directory for Obsidian).
    pub fn export(&self, format: Format, out: &Path) -> Result<ExportReport> {
        let (files_written, nodes_omitted) = match format {
            Format::Json => {
                let doc = json!({
                    "version": 1,
                    "generator": format!("kor-graph {}", crate::VERSION),
                    "root": self.root.to_string_lossy(),
                    "last_update": self.last_update,
                    "nodes": self.nodes.values().collect::<Vec<_>>(),
                    "edges": self.edges.values().collect::<Vec<_>>(),
                    "communities": self.communities,
                });
                write(out, &serde_json::to_string_pretty(&doc)?)?;
                (1, 0)
            }
            Format::Graphify => {
                write(out, &serde_json::to_string_pretty(&self.graphify())?)?;
                (1, 0)
            }
            Format::Mermaid => {
                let (text, omitted) = self.mermaid();
                write(out, &text)?;
                (1, omitted)
            }
            Format::Obsidian => (self.obsidian(out)?, 0),
            Format::Html => {
                let (text, omitted) = self.html()?;
                write(out, &text)?;
                (1, omitted)
            }
        };
        Ok(ExportReport {
            path: out.to_path_buf(),
            files_written,
            nodes_omitted,
        })
    }

    fn graphify(&self) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .nodes
            .values()
            .map(|n| {
                let file_type = match n.kind {
                    NodeKind::Decision => "rationale",
                    NodeKind::Doc | NodeKind::Media => "document",
                    NodeKind::EnvVar
                    | NodeKind::Config
                    | NodeKind::Package
                    | NodeKind::Table
                    | NodeKind::Server
                    | NodeKind::Service => "concept",
                    _ => "code",
                };
                json!({
                    "id": n.id,
                    "label": n.name,
                    "file_type": file_type,
                    "source_file": n.file.as_ref().map(|f| f.to_string_lossy()),
                    "source_location": n.span.map(|s| format!("L{}", s.0)),
                    "community": n.community,
                    "kind": n.kind.as_str(),
                    "degree": n.degree,
                    "churn_30d": n.churn_30d,
                })
            })
            .collect();
        let links: Vec<serde_json::Value> = self
            .edges
            .values()
            .map(|e| {
                let source_file = self
                    .nodes
                    .get(&e.source)
                    .and_then(|n| n.file.as_ref())
                    .map(|f| f.to_string_lossy());
                let weight = if let EdgeKind::CoChanges { ratio } = e.kind {
                    f64::from(ratio)
                } else {
                    1.0
                };
                json!({
                    "source": e.source,
                    "target": e.target,
                    "relation": e.kind.as_str(),
                    "confidence": e.confidence.as_str().to_uppercase(),
                    "confidence_score": e.confidence.score(),
                    "source_file": source_file,
                    "weight": weight,
                })
            })
            .collect();
        json!({ "directed": true, "multigraph": false, "graph": { "generator": "kor" }, "nodes": nodes, "links": links })
    }

    /// The `cap` highest-degree nodes (ties by id).
    fn top_nodes(&self, cap: usize) -> Vec<&GraphNode> {
        let mut v: Vec<&GraphNode> = self.nodes.values().collect();
        v.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.id.cmp(&b.id)));
        v.truncate(cap);
        v
    }

    fn mermaid(&self) -> (String, usize) {
        let top = self.top_nodes(MERMAID_CAP);
        let ids: HashMap<&str, usize> = top
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let mut s = String::from("flowchart LR\n");
        for (i, n) in top.iter().enumerate() {
            let label: String = n
                .name
                .chars()
                .take(60)
                .map(|c| {
                    if matches!(c, '"' | '[' | ']' | '<' | '>' | '{' | '}' | '|') {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            let _ = writeln!(s, "  n{i}[\"{label}\"]");
        }
        for e in self.edges.values() {
            if let (Some(a), Some(b)) = (ids.get(e.source.as_str()), ids.get(e.target.as_str())) {
                let arrow = if e.confidence == Confidence::Extracted {
                    "-->"
                } else {
                    "-.->"
                };
                let _ = writeln!(s, "  n{a} {arrow}|{}| n{b}", e.kind.as_str());
            }
        }
        (s, self.nodes.len() - top.len())
    }

    fn obsidian(&self, dir: &Path) -> Result<usize> {
        fn note_name(id: &str) -> String {
            id.chars()
                .map(|c| {
                    if matches!(
                        c,
                        '/' | '\\'
                            | ':'
                            | '#'
                            | '*'
                            | '?'
                            | '"'
                            | '<'
                            | '>'
                            | '|'
                            | '['
                            | ']'
                            | '^'
                    ) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect()
        }
        std::fs::create_dir_all(dir)?;
        let mut out_links: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        let mut in_links: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        for e in self.edges.values() {
            out_links
                .entry(e.source.as_str())
                .or_default()
                .push((e.kind.as_str(), e.target.as_str()));
            in_links
                .entry(e.target.as_str())
                .or_default()
                .push((e.kind.as_str(), e.source.as_str()));
        }
        let names: HashSet<String> = self.nodes.keys().map(|k| note_name(k)).collect();
        let link = |id: &str| {
            let label = self
                .nodes
                .get(id)
                .map_or(id, |n| n.name.as_str())
                .replace(['|', ']', '['], " ");
            format!("[[{}|{label}]]", note_name(id))
        };
        let mut written = 0;
        for n in self.nodes.values() {
            let mut s = String::new();
            let _ = writeln!(
                s,
                "---\nkind: {}\nfile: {:?}\nline: {}\ncommunity: {}\ndegree: {}\nchurn_30d: {}\n---\n",
                n.kind.as_str(),
                file_str(n),
                n.span.map_or(0, |x| x.0),
                n.community,
                n.degree,
                n.churn_30d
            );
            let _ = writeln!(s, "# {}\n", n.name);
            if !n.text.is_empty() && n.kind == NodeKind::Decision {
                let _ = writeln!(s, "> {}\n", n.text);
            }
            if let Some(c) = self.communities.iter().find(|c| c.id == n.community) {
                let _ = writeln!(s, "Community: {}\n", c.label);
            }
            for (title, list) in [
                ("Links", out_links.get(n.id.as_str())),
                ("Referenced by", in_links.get(n.id.as_str())),
            ] {
                if let Some(list) = list {
                    let _ = writeln!(s, "## {title}\n");
                    for (kind, other) in list {
                        let _ = writeln!(s, "- {kind} {}", link(other));
                    }
                    s.push('\n');
                }
            }
            std::fs::write(dir.join(format!("{}.md", note_name(&n.id))), s)?;
            written += 1;
        }
        let mut idx = String::from("# Project graph\n\n## Communities\n\n");
        for c in &self.communities {
            let _ = writeln!(idx, "- {} ({}): {}", c.id, c.size, c.label);
        }
        // Do not clobber a node that happens to be named `_index`.
        let index_name = if names.contains("_index") {
            "_kor_index.md"
        } else {
            "_index.md"
        };
        std::fs::write(dir.join(index_name), idx)?;
        Ok(written + 1)
    }

    fn html(&self) -> Result<(String, usize)> {
        let top = self.top_nodes(HTML_CAP);
        let pos: HashMap<&str, usize> = top
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let nodes: Vec<serde_json::Value> = top
            .iter()
            .map(|n| json!({"id": n.id, "n": n.name, "k": n.kind.as_str(), "f": file_str(n), "l": n.span.map(|s| s.0), "c": n.community, "d": n.degree}))
            .collect();
        let links: Vec<[usize; 2]> = self
            .edges
            .values()
            .filter_map(|e| Some([*pos.get(e.source.as_str())?, *pos.get(e.target.as_str())?]))
            .collect();
        let labels: BTreeMap<u16, &str> = self
            .communities
            .iter()
            .map(|c| (c.id, c.label.as_str()))
            .collect();
        let html = html_viewer(&json!({"nodes": nodes, "links": links, "communities": labels}))?;
        Ok((html, self.nodes.len() - top.len()))
    }
}

/// The self-contained HTML viewer for any `{nodes: [{id,n,k,f,l,c,d}], links: [[s,t]],
/// communities: {c: label}}` data (`kor visualize` reuses it for its views).
///
/// # Errors
/// JSON serialization.
pub fn html_viewer(data: &serde_json::Value) -> Result<String> {
    // `<` escaped so the data can never close the <script> element.
    let data = serde_json::to_string(data)?.replace('<', "\\u003c");
    Ok(HTML_TEMPLATE.replace("/*DATA*/null", &data))
}

const HTML_TEMPLATE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Kor graph</title>
<style>
:root{--bg:#f7f7f5;--fg:#1c1c1c;--muted:#6b6b6b;--panel:#ffffff;--line:#d9d9d4}
@media (prefers-color-scheme:dark){:root{--bg:#141414;--fg:#ececec;--muted:#9a9a9a;--panel:#1f1f1f;--line:#333}}
html,body{margin:0;height:100%;background:var(--bg);color:var(--fg);font:14px/1.5 system-ui,sans-serif}
#bar{position:fixed;top:12px;left:12px;right:12px;display:flex;gap:8px;z-index:2}
#q{flex:1;max-width:360px;padding:6px 10px;border:1px solid var(--line);border-radius:6px;background:var(--panel);color:var(--fg)}
#info{position:fixed;bottom:12px;left:12px;max-width:min(420px,calc(100% - 24px));background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:10px 12px;display:none;z-index:2;overflow-wrap:anywhere}
#info small{color:var(--muted)}canvas{display:block;width:100%;height:100%}
</style></head><body>
<div id="bar"><input id="q" placeholder="Search nodes" aria-label="Search nodes"><span id="count"></span></div>
<canvas id="c" aria-label="Project graph"></canvas><div id="info" role="status"></div>
<script>
const G=/*DATA*/null;
const cv=document.getElementById('c'),ctx=cv.getContext('2d'),info=document.getElementById('info');
const N=G.nodes,L=G.links;document.getElementById('count').textContent=N.length+' nodes, '+L.length+' edges';
N.forEach((n,i)=>{const a=i*2.399963;const r=10*Math.sqrt(i);n.x=Math.cos(a)*r;n.y=Math.sin(a)*r;n.vx=0;n.vy=0;});
const col=c=>'hsl('+((c*137.5)%360)+',55%,50%)';let scale=1,ox=0,oy=0,hit=null,match=null,alpha=1;
function resize(){cv.width=innerWidth*devicePixelRatio;cv.height=innerHeight*devicePixelRatio;}
addEventListener('resize',resize);resize();
function tick(){if(alpha<0.005)return;const k=30;
 for(let i=0;i<N.length;i++){const a=N[i];for(let j=i+1;j<Math.min(N.length,i+1+200);j++){const b=N[j];let dx=a.x-b.x,dy=a.y-b.y,d2=dx*dx+dy*dy+0.01;const f=k*k/d2*alpha;a.vx+=dx*f*0.05;a.vy+=dy*f*0.05;b.vx-=dx*f*0.05;b.vy-=dy*f*0.05;}}
 for(const [s,t] of L){const a=N[s],b=N[t];const dx=b.x-a.x,dy=b.y-a.y,d=Math.sqrt(dx*dx+dy*dy)+0.01;const f=(d-k)/d*0.05*alpha;a.vx+=dx*f;a.vy+=dy*f;b.vx-=dx*f;b.vy-=dy*f;}
 for(const n of N){n.vx-=n.x*0.002*alpha;n.vy-=n.y*0.002*alpha;n.x+=n.vx;n.y+=n.vy;n.vx*=0.6;n.vy*=0.6;}alpha*=0.99;}
function toScreen(n){return[(n.x*scale+ox)*devicePixelRatio+cv.width/2,(n.y*scale+oy)*devicePixelRatio+cv.height/2];}
function draw(){tick();ctx.clearRect(0,0,cv.width,cv.height);ctx.globalAlpha=0.25;ctx.strokeStyle=getComputedStyle(document.body).color;ctx.beginPath();
 for(const [s,t] of L){const a=toScreen(N[s]),b=toScreen(N[t]);ctx.moveTo(a[0],a[1]);ctx.lineTo(b[0],b[1]);}ctx.stroke();ctx.globalAlpha=1;
 for(const n of N){const p=toScreen(n);const r=(2+Math.sqrt(n.d))*devicePixelRatio;const dim=match&&!match.has(n);ctx.globalAlpha=dim?0.15:1;ctx.fillStyle=col(n.c);ctx.beginPath();ctx.arc(p[0],p[1],r,0,6.283);ctx.fill();
  if(n===hit||(match&&match.has(n)&&match.size<30)){ctx.fillStyle=getComputedStyle(document.body).color;ctx.fillText(n.n,p[0]+r+2,p[1]);}}
 ctx.globalAlpha=1;requestAnimationFrame(draw);}
ctx.font=(12*devicePixelRatio)+'px system-ui';requestAnimationFrame(draw);
function pick(e){let best=null,bd=1e9;for(const n of N){const p=toScreen(n);const d=Math.hypot(p[0]-e.clientX*devicePixelRatio,p[1]-e.clientY*devicePixelRatio);if(d<bd){bd=d;best=n;}}return bd<12*devicePixelRatio?best:null;}
const esc=s=>String(s).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
cv.addEventListener('click',e=>{hit=pick(e);if(!hit){info.style.display='none';return;}info.style.display='block';
 info.innerHTML='<b>'+esc(hit.n)+'</b> <small>'+esc(hit.k)+'</small><br><small>'+esc(hit.f)+(hit.l?':'+hit.l:'')+'</small><br><small>community: '+esc(G.communities[hit.c]||hit.c)+' · degree '+hit.d+'</small>';});
let drag=null;cv.addEventListener('mousedown',e=>drag=[e.clientX,e.clientY]);addEventListener('mouseup',()=>drag=null);
addEventListener('mousemove',e=>{if(drag){ox+=e.clientX-drag[0];oy+=e.clientY-drag[1];drag=[e.clientX,e.clientY];}});
cv.addEventListener('wheel',e=>{e.preventDefault();scale*=e.deltaY<0?1.1:0.9;},{passive:false});
document.getElementById('q').addEventListener('input',e=>{const q=e.target.value.trim().toLowerCase();match=q?new Set(N.filter(n=>n.n.toLowerCase().includes(q)||n.f.toLowerCase().includes(q))):null;});
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::fixture;

    #[test]
    fn all_formats() -> Result<()> {
        let dir = fixture();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        let out = tempfile::tempdir()?;
        for f in ["json", "--graphify", "mermaid", "obsidian", "html"] {
            let fmt: Format = f.parse()?;
            let path = out.path().join(format!("out-{f}"));
            let r = g.export(fmt, &path)?;
            assert!(r.files_written >= 1);
        }
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.path().join("out---graphify"))?)?;
        assert!(v["nodes"].as_array().is_some_and(|n| {
            n.iter()
                .any(|n| n["id"] == "src/auth.rs#login" && n["source_location"] == "L3")
        }));
        assert!(v["links"].as_array().is_some_and(|l| {
            l.iter()
                .any(|l| l["relation"] == "calls" && l["confidence"] == "EXTRACTED")
        }));
        let mmd = std::fs::read_to_string(out.path().join("out-mermaid"))?;
        assert!(mmd.starts_with("flowchart LR") && mmd.contains("|calls|"));
        let note =
            std::fs::read_to_string(out.path().join("out-obsidian").join("src_auth.rs_login.md"))?;
        assert!(note.contains("[[src_db.rs_Db.connect|connect]]"), "{note}");
        let html = std::fs::read_to_string(out.path().join("out-html"))?;
        assert!(
            html.contains("\"src/auth.rs#login\"")
                && !html.contains("/*DATA*/")
                && !html.contains("<script src")
        );
        let own: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.path().join("out-json"))?)?;
        assert_eq!(own["nodes"].as_array().map(Vec::len), Some(g.nodes.len()));
        assert!("xml".parse::<Format>().is_err());
        Ok(())
    }
}
