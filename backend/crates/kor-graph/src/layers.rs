//! Layers added on top of the parsed code graph: monorepo packages (spec §15.15, from
//! [`crate::workspace`]) and decisions from Kor's project memory (spec §7.3, `DecidedBy`).

use crate::model::{Confidence, EdgeKind, NodeKind};
use crate::resolve::Derived;
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Node id prefix of workspace packages (`pkg:<name>`).
pub const PACKAGE_PREFIX: &str = "pkg:";
/// Node id prefix of project-memory decisions (`memory:<n>`).
pub const MEMORY_PREFIX: &str = "memory:";

/// A decision from Kor's project memory (`decisions.md`), linked to the files or symbols it names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDecision {
    pub title: String,
    pub body: String,
    pub refs: Vec<String>,
}

/// Package nodes, `Contains` edges to the files of each package and `DependsOn` edges between
/// packages. Only file-level nodes (id == file) are contained; symbols hang off their file.
pub(crate) fn packages(d: &mut Derived, ws: &Workspace) {
    if !ws.is_monorepo() {
        return;
    }
    for p in &ws.packages {
        let id = format!("{PACKAGE_PREFIX}{}", p.name);
        let manifest = ws.rel(&p.manifest);
        d.node(
            &id,
            NodeKind::Package,
            &p.name,
            Some(&manifest),
            None,
            p.tool.key(),
        );
    }
    let files: Vec<String> = d
        .nodes
        .values()
        .filter(|n| n.kind != NodeKind::Package && n.file.as_deref() == Some(Path::new(&n.id)))
        .map(|n| n.id.clone())
        .collect();
    for f in files {
        if let Some(p) = ws.package_of(Path::new(&f)) {
            let id = format!("{PACKAGE_PREFIX}{}", p.name);
            d.edge(&id, &f, EdgeKind::Contains, Confidence::Extracted);
        }
    }
    for (from, to) in ws.edges() {
        d.edge(
            &format!("{PACKAGE_PREFIX}{from}"),
            &format!("{PACKAGE_PREFIX}{to}"),
            EdgeKind::DependsOn,
            Confidence::Extracted,
        );
    }
}

/// One `Decision` node per memory decision, with `DecidedBy` edges from every node a ref names:
/// an exact id or file path (`Extracted`), else up to three nodes with that name (`Inferred`).
pub(crate) fn memory(d: &mut Derived, decisions: &[MemoryDecision]) {
    let mut by_name: HashMap<&str, Vec<String>> = HashMap::new();
    for n in d.nodes.values() {
        by_name
            .entry(n.name.as_str())
            .or_default()
            .push(n.id.clone());
    }
    let mut links: Vec<(String, String, Confidence)> = Vec::new();
    for (i, dec) in decisions.iter().enumerate() {
        let id = format!("{MEMORY_PREFIX}{}", i + 1);
        for r in &dec.refs {
            let r = r.trim().trim_start_matches("./").replace('\\', "/");
            if d.nodes.contains_key(&r) {
                links.push((r, id.clone(), Confidence::Extracted));
                continue;
            }
            let last = r.rsplit([':', '.', '/', '#']).next().unwrap_or(&r);
            let hits = by_name.get(last).map(Vec::as_slice).unwrap_or_default();
            if (1..=3).contains(&hits.len()) {
                for h in hits {
                    links.push((h.clone(), id.clone(), Confidence::Inferred { score: 0.5 }));
                }
            }
        }
    }
    for (i, dec) in decisions.iter().enumerate() {
        let id = format!("{MEMORY_PREFIX}{}", i + 1);
        let name: String = dec.title.chars().take(60).collect();
        d.node(&id, NodeKind::Decision, &name, None, None, &dec.body);
    }
    for (src, dst, c) in links {
        d.edge(&src, &dst, EdgeKind::DecidedBy, c);
    }
}

/// Node id prefix of generated media (`media:<file name>`).
pub const MEDIA_PREFIX: &str = "media:";

/// A generated file in `kor-media/` and what it came from (the gallery sidecar, ADR 0060).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    /// Gallery id (file name in `kor-media/`).
    pub id: String,
    /// Project-relative path with `/`.
    pub file: String,
    /// Prompt and model, for search.
    pub text: String,
    /// Inputs: gallery ids, or project paths of files outside the gallery.
    pub inputs: Vec<String>,
}

/// One `Media` node per generated file; `DerivedFrom` edges to the gallery items it was made from
/// and to project files used as references (when they are in the graph).
pub(crate) fn media(d: &mut Derived, items: &[MediaItem]) {
    let ids: std::collections::HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    for i in items {
        let id = format!("{MEDIA_PREFIX}{}", i.id);
        d.node(&id, NodeKind::Media, &i.id, Some(&i.file), None, &i.text);
    }
    for i in items {
        let id = format!("{MEDIA_PREFIX}{}", i.id);
        for input in &i.inputs {
            let target = if ids.contains(input.as_str()) {
                format!("{MEDIA_PREFIX}{input}")
            } else if d.nodes.contains_key(input) {
                input.clone()
            } else {
                continue;
            };
            d.edge(&id, &target, EdgeKind::DerivedFrom, Confidence::Extracted);
        }
    }
}

/// Degree (non-`Contains` edges) of every node, after layers were added.
pub(crate) fn recount_degree(d: &mut Derived) {
    let mut degree: HashMap<String, u32> = HashMap::new();
    for e in d.edges.values() {
        if e.kind != EdgeKind::Contains {
            *degree.entry(e.source.clone()).or_default() += 1;
            *degree.entry(e.target.clone()).or_default() += 1;
        }
    }
    for n in d.nodes.values_mut() {
        n.degree = degree.get(&n.id).copied().unwrap_or(0);
    }
}

/// Is `rel` (root-relative, `/`-separated) a file that can change the package layer?
pub(crate) fn is_manifest(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    matches!(
        name,
        "Cargo.toml"
            | "package.json"
            | "pnpm-workspace.yaml"
            | "lerna.json"
            | "nx.json"
            | "project.json"
            | "turbo.json"
            | "go.mod"
            | "go.work"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "build.gradle"
            | "build.gradle.kts"
            | "BUILD"
            | "BUILD.bazel"
            | "BUCK"
            | "WORKSPACE"
            | "MODULE.bazel"
    ) || Path::new(name)
        .extension()
        .is_some_and(|e| e == "sln" || e == "csproj" || e == "fsproj" || e == "vbproj")
}

#[cfg(test)]
mod tests {
    use crate::graph::Graph;
    use crate::model::{EdgeKind, NodeKind};
    use crate::{MediaItem, MemoryDecision, Result};

    fn monorepo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let w = |p: &str, s: &str| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, s).expect("write");
        };
        w("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
        w("crates/core/Cargo.toml", "[package]\nname = \"core\"\n");
        w("crates/core/src/lib.rs", "pub fn shared() -> u32 { 1 }\n");
        w(
            "crates/app/Cargo.toml",
            "[package]\nname = \"app\"\n[dependencies]\ncore = { path = \"../core\" }\n",
        );
        w(
            "crates/app/src/main.rs",
            "fn main() { let _ = core::shared(); }\n",
        );
        dir
    }

    #[test]
    fn package_layer() -> Result<()> {
        let dir = monorepo();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        let app = g.node("pkg:app").expect("app package node");
        assert_eq!(app.kind, NodeKind::Package);
        assert_eq!(app.text, "cargo");
        assert!(g.node("pkg:core").is_some());
        let has = |s: &str, t: &str, k: &str| {
            g.edges()
                .any(|e| e.source == s && e.target == t && e.kind.as_str() == k)
        };
        assert!(has("pkg:app", "pkg:core", "depends_on"));
        assert!(has("pkg:app", "crates/app/src/main.rs", "contains"));
        assert!(has("pkg:core", "crates/core/src/lib.rs", "contains"));
        assert!(!has("pkg:app", "crates/core/src/lib.rs", "contains"));
        assert_eq!(
            g.package_of("crates/core/src/lib.rs")
                .map(|p| p.name.as_str()),
            Some("core")
        );
        // Dependents of `core` include `app` through the package layer.
        let imp = g.impact("pkg:core", 2);
        assert!(imp.affected.iter().any(|a| a.node.id == "pkg:app"));
        // Persisted and reloaded.
        drop(g);
        let g = Graph::open(dir.path())?;
        assert!(g.node("pkg:app").is_some());
        // Cheap ranking straight from SQLite (repository map); no package or decision rows.
        let ranked = crate::ranked_files(dir.path(), 10).expect("stored graph");
        assert!(
            ranked.iter().any(|f| f == "crates/core/src/lib.rs"),
            "{ranked:?}"
        );
        assert!(ranked.iter().all(|f| !f.starts_with("pkg:")));
        assert!(crate::ranked_files(&dir.path().join("crates"), 10).is_none());
        Ok(())
    }

    #[test]
    fn manifest_change_redetects_packages() -> Result<()> {
        let dir = monorepo();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        std::fs::create_dir_all(dir.path().join("crates/extra/src"))?;
        std::fs::write(
            dir.path().join("crates/extra/Cargo.toml"),
            "[package]\nname = \"extra\"\n",
        )?;
        std::fs::write(
            dir.path().join("crates/extra/src/lib.rs"),
            "pub fn x() {}\n",
        )?;
        g.update_files(&[
            dir.path().join("crates/extra/Cargo.toml"),
            dir.path().join("crates/extra/src/lib.rs"),
        ])?;
        assert!(g.node("pkg:extra").is_some());
        assert!(g.edges().any(|e| e.source == "pkg:extra"
            && e.target == "crates/extra/src/lib.rs"
            && e.kind == EdgeKind::Contains));
        Ok(())
    }

    #[test]
    fn memory_decisions_link() -> Result<()> {
        let dir = monorepo();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        let s = g.set_memory_decisions(vec![MemoryDecision {
            title: "Keep core tiny".into(),
            body: "Only shared helpers.".into(),
            refs: vec!["crates/core/src/lib.rs".into(), "shared".into()],
        }])?;
        assert!(s.changed);
        let dec = g.node("memory:1").expect("decision node");
        assert_eq!(dec.kind, NodeKind::Decision);
        assert!(g.edges().any(|e| e.source == "crates/core/src/lib.rs"
            && e.target == "memory:1"
            && e.kind == EdgeKind::DecidedBy));
        let ex = g.explain("shared").expect("explain");
        assert!(ex.decisions.iter().any(|d| d.id == "memory:1"));
        // Same list again: nothing to recompute.
        let s = g.set_memory_decisions(vec![MemoryDecision {
            title: "Keep core tiny".into(),
            body: "Only shared helpers.".into(),
            refs: vec!["crates/core/src/lib.rs".into(), "shared".into()],
        }])?;
        assert!(!s.changed);
        Ok(())
    }

    #[test]
    fn media_derivation_chain() -> Result<()> {
        let dir = monorepo();
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        let item = |id: &str, inputs: &[&str]| MediaItem {
            id: id.into(),
            file: format!("kor-media/{id}"),
            text: "a cat".into(),
            inputs: inputs.iter().map(|s| (*s).to_owned()).collect(),
        };
        let s = g.set_media_items(vec![
            item("cat.png", &["crates/core/src/lib.rs"]),
            item("cat-big.png", &["cat.png"]),
        ])?;
        assert!(s.changed);
        assert_eq!(
            g.node("media:cat.png").map(|n| n.kind),
            Some(NodeKind::Media)
        );
        let has = |s: &str, t: &str| {
            g.edges()
                .any(|e| e.source == s && e.target == t && e.kind == EdgeKind::DerivedFrom)
        };
        assert!(has("media:cat-big.png", "media:cat.png"));
        assert!(
            has("media:cat.png", "crates/core/src/lib.rs"),
            "a project file used as reference"
        );
        assert!(
            !g.set_media_items(vec![
                item("cat.png", &["crates/core/src/lib.rs"]),
                item("cat-big.png", &["cat.png"])
            ])?
            .changed
        );
        Ok(())
    }

    #[test]
    fn manifests() {
        assert!(super::is_manifest("crates/x/Cargo.toml"));
        assert!(super::is_manifest("src/App.csproj"));
        assert!(!super::is_manifest("src/main.rs"));
    }
}
