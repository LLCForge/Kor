//! `graph.*`: canlı proje grafı (§7) için ince köprü (`kor graph status|explain|path|query|
//! hotspots|diff|impact|export|update`). İş mantığı `kor_graph` içinde; burası yalnızca JSON'a
//! çevirir. Açık projenin grafı ilk çağrıda kurulur ve izleyiciyle güncel tutulur; açık bir sohbet
//! oturumu aynı projede graf tutuyorsa onun grafı kullanılır (iki kopya olmaz).
//!
//! - `graph.status()` → `{ status, packages, touched }` (`touched`: ajanın bu oturumda dokunduğu dosyalar)
//! - `graph.snapshot(limit?, community?)` → `{ nodes, edges, total, truncated }`: çizim için en yüksek
//!   dereceli `limit` düğüm (varsayılan 400, en çok 5000) ve aralarındaki kenarlar `[s, t, kind, ratio]`
//! - `graph.reveal(path)` → `null`: proje içindeki bir dosyayı (ör. dışa aktarım) klasöründe gösterir
//! - `graph.explain(name)` → `Explanation | null`
//! - `graph.path(from, to)` → `PathStep[] | null`
//! - `graph.query(text, limit?)` → `QueryHit[]`
//! - `graph.hotspots(limit?)` → `Hotspot[]`
//! - `graph.diff(rev)` → `GraphDiff`
//! - `graph.impact(target, depth?)` → `Impact`
//! - `graph.export(format: "json"|"mermaid"|"obsidian"|"html"|"graphify", out?)` → `ExportReport`
//! - `graph.update()` → `UpdateStats` (tam yeniden tarama)
//! - `graph.watch()` → `{ root }`: izleyiciyi başlatır (diğer çağrılar da başlatır)
//!
//! Olaylar: `kor:graph/updated` `{ root, stats, status }` (her değişen güncellemede),
//! `kor:graph/touched` `{ root, files }` (ajan bu oturumda bir dosyayı okuyunca/değiştirince).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kor_graph::{Format, Graph, WatchHandle};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `graph::call`'a yönlendirir).
pub const NS: &str = "graph";

/// Köprünün kendi açtığı graf (sohbet oturumu yokken) ve izleyicisi.
struct Open {
    root: PathBuf,
    graph: Arc<Mutex<Graph>>,
    _watch: Option<WatchHandle>,
}

static OPEN: Mutex<Option<Open>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(
        name,
        "status"
            | "explain"
            | "path"
            | "query"
            | "hotspots"
            | "diff"
            | "impact"
            | "export"
            | "update"
            | "watch"
            | "snapshot"
            | "reveal"
    ) {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let (name, args, app) = (name.to_owned(), args.to_vec(), app.clone());
    // Graf kurulumu ve sorgular diski okur: arayüz iş parçacığını bekletmesin.
    Some(
        tauri::async_runtime::spawn_blocking(move || run(&name, &args, &app, &root))
            .await
            .unwrap_or_else(|e| Err(format!("Graf işlemi başarısız: {e}"))),
    )
}

fn run(name: &str, args: &[Value], app: &AppHandle, root: &Path) -> Res {
    if name == "diff" {
        let rev = s(args, 0, "graph.diff")?;
        let d = kor_graph::diff(root, rev).map_err(|e| format!("Graf farkı alınamadı: {e}"))?;
        return to_json(&d);
    }
    if name == "reveal" {
        let path = safe_out(root, s(args, 0, "graph.reveal")?)?;
        if !path.exists() {
            return Err("Dosya bulunamadı".into());
        }
        return app
            .opener()
            .reveal_item_in_dir(&path)
            .map(|()| Value::Null)
            .map_err(|e| e.to_string());
    }
    let g = graph_for(app, root)?;
    let mut g = lock(&g);
    match name {
        "status" => Ok(json!({
            "status": g.status(),
            "packages": g.workspace().map(|w| w.packages.len()).unwrap_or(0),
            "touched": kor_agent::graph::find(root).map(|s| s.touched()).unwrap_or_default(),
        })),
        "snapshot" => {
            let limit = args
                .first()
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(400)
                .clamp(1, MAX_SNAPSHOT_NODES);
            let community = args
                .get(1)
                .and_then(Value::as_u64)
                .and_then(|c| u16::try_from(c).ok());
            Ok(snapshot(&g, limit, community))
        }
        "explain" => to_json(&g.explain(s(args, 0, "graph.explain")?)),
        "path" => to_json(&g.path(s(args, 0, "graph.path")?, s(args, 1, "graph.path")?)),
        "query" => to_json(&g.query(s(args, 0, "graph.query")?, limit(args, 1, 20))),
        "hotspots" => to_json(&g.hotspots(limit(args, 0, 20))),
        "impact" => {
            let depth = args
                .get(1)
                .and_then(Value::as_u64)
                .and_then(|d| u32::try_from(d).ok())
                .unwrap_or(3)
                .clamp(1, 10);
            to_json(&g.impact(s(args, 0, "graph.impact")?, depth))
        }
        "export" => {
            let format = Format::from_str(s(args, 0, "graph.export")?)
                .map_err(|e| format!("Bilinmeyen biçim: {e}"))?;
            let out = match opt_s(args, 1) {
                Some(p) => safe_out(root, p)?,
                None => format.default_path(root),
            };
            to_json(&g.export(format, &out).map_err(|e| e.to_string())?)
        }
        "update" => {
            let stats = g
                .update()
                .map_err(|e| format!("Graf güncellenemedi: {e}"))?;
            let _ = app.emit(
                "kor:graph/updated",
                json!({ "root": root, "stats": stats, "status": g.status() }),
            );
            to_json(&stats)
        }
        _ => Ok(json!({ "root": root })),
    }
}

const MAX_SNAPSHOT_NODES: usize = 5000;
const MAX_SNAPSHOT_EDGES: usize = 40_000;

/// Çizim için düğüm/kenar kümesi: en yüksek dereceli `limit` düğüm (isteğe bağlı tek topluluk) ve
/// yalnızca ikisi de seçilmiş kenarlar. Kenarlar düğüm dizinleriyle gelir (JSON küçük kalsın).
fn snapshot(g: &Graph, limit: usize, community: Option<u16>) -> Value {
    let mut nodes: Vec<&kor_graph::GraphNode> = g
        .nodes()
        .filter(|n| community.is_none_or(|c| n.community == c))
        .collect();
    let total = nodes.len();
    nodes.sort_by(|a, b| b.degree.cmp(&a.degree).then_with(|| a.id.cmp(&b.id)));
    nodes.truncate(limit);
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut edges = Vec::new();
    let mut edges_cut = false;
    for e in g.edges() {
        if let (Some(&s), Some(&t)) = (index.get(e.source.as_str()), index.get(e.target.as_str()))
            && s != t
        {
            if edges.len() == MAX_SNAPSHOT_EDGES {
                edges_cut = true;
                break;
            }
            let ratio = match e.kind {
                kor_graph::EdgeKind::CoChanges { ratio } => json!(ratio),
                _ => Value::Null,
            };
            edges.push(json!([s, t, e.kind.as_str(), ratio]));
        }
    }
    let nodes: Vec<Value> = nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.id,
                "kind": n.kind.as_str(),
                "name": n.name,
                "file": n.file.as_ref().map(|f| f.to_string_lossy().replace('\\', "/")),
                "line": n.span.map(|s| s.0),
                "community": n.community,
                "degree": n.degree,
                "churn": n.churn_30d,
            })
        })
        .collect();
    json!({
        "nodes": nodes,
        "edges": edges,
        "total": total,
        "truncated": total > limit || edges_cut,
        "communities": g.communities(),
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn limit(args: &[Value], i: usize, default: usize) -> usize {
    args.get(i)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(default)
        .clamp(1, 200)
}

/// Dışa aktarma yolu: proje kökü içinde kalmalı (göreli yollar köke göre).
fn safe_out(root: &Path, p: &str) -> Result<PathBuf, String> {
    let path = Path::new(p);
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Dışa aktarma yolu `..` içeremez".into());
    }
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if !abs.starts_with(root) {
        return Err("Dışa aktarma yolu proje klasörünün içinde olmalı".into());
    }
    Ok(abs)
}

/// Açık projenin güncel grafı: sohbet oturumununki varsa o, yoksa köprünün izlenen kopyası.
pub(crate) fn graph_for(app: &AppHandle, root: &Path) -> Result<Arc<Mutex<Graph>>, String> {
    if let Some(session) = kor_agent::graph::find(root)
        && let Some(g) = session.ready()
    {
        forward(app, &session);
        return Ok(g);
    }
    let mut slot = lock(&OPEN);
    if let Some(o) = slot.as_ref()
        && o.root == root
    {
        return Ok(Arc::clone(&o.graph));
    }
    let mut g = Graph::open(root).map_err(|e| format!("Graf açılamadı: {e}"))?;
    let stats = g.update().map_err(|e| format!("Graf kurulamadı: {e}"))?;
    let _ = app.emit(
        "kor:graph/updated",
        json!({ "root": root, "stats": stats, "status": g.status() }),
    );
    let graph = Arc::new(Mutex::new(g));
    let (ev_app, ev_root, ev_graph) = (app.clone(), root.to_path_buf(), Arc::downgrade(&graph));
    let watch = kor_graph::watch_graph(Arc::clone(&graph), move |r| {
        if let Ok(stats) = r {
            let status = ev_graph.upgrade().map(|g| lock(&g).status());
            let _ = ev_app.emit(
                "kor:graph/updated",
                json!({ "root": ev_root, "stats": stats, "status": status }),
            );
        }
    })
    .ok();
    *slot = Some(Open {
        root: root.to_path_buf(),
        graph: Arc::clone(&graph),
        _watch: watch,
    });
    Ok(graph)
}

/// Sohbet oturumunun graf olaylarını arayüze aktarır (güncellendi, ajanın dokunduğu dosyalar).
fn forward(app: &AppHandle, session: &Arc<kor_agent::graph::Session>) {
    let (app, root) = (app.clone(), session.root().to_path_buf());
    session.listen(Arc::new(move |ev| match ev {
        kor_agent::graph::Event::Updated(stats) => {
            let _ = app.emit("kor:graph/updated", json!({ "root": root, "stats": stats }));
        }
        kor_agent::graph::Event::Touched(files) => {
            let _ = app.emit("kor:graph/touched", json!({ "root": root, "files": files }));
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_path_stays_in_project() {
        let root = Path::new("/p");
        assert!(safe_out(root, "../x.json").is_err());
        assert!(safe_out(root, "out/g.json").is_ok_and(|p| p.starts_with(root)));
        assert_eq!(limit(&[json!(500)], 0, 20), 200);
        assert_eq!(limit(&[], 0, 20), 20);
    }

    #[test]
    fn snapshot_keeps_top_degree_nodes_and_their_edges() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn a() { b(); c(); }\npub fn b() { c(); }\npub fn c() {}\n",
        )
        .expect("write");
        let mut g = Graph::open(dir.path()).expect("open");
        g.update().expect("update");
        let all = snapshot(&g, 5000, None);
        let n = all["nodes"].as_array().expect("nodes").len();
        assert!(n >= 3, "{all}");
        assert_eq!(all["truncated"], false);
        let small = snapshot(&g, 2, None);
        assert_eq!(small["nodes"].as_array().expect("nodes").len(), 2);
        assert_eq!(small["truncated"], true);
        for e in small["edges"].as_array().expect("edges") {
            assert!(e[0].as_u64().expect("s") < 2 && e[1].as_u64().expect("t") < 2);
        }
        let none = snapshot(&g, 10, Some(u16::MAX));
        assert_eq!(none["total"], 0);
    }
}
