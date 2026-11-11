//! Web documentation layer (spec §27.11, ADR 0058): documentation pages the user read through the
//! Kor side panel or `kor docs use`. A page is a [`DocSnapshot`] (URL, title, library and the
//! version the page describes; never the page text, which stays in Kor's docs cache) kept in
//! `<root>/.kor/graph/docs/<id>.json`, read like the infrastructure snapshots of ADR 0044.
//!
//! Nodes: `webdoc:<id>` ([`NodeKind::Doc`]). Edge: root manifest → doc `DocumentedBy`
//! (`Inferred`) when the page names a library the project depends on; no node is invented for
//! code that is not in the graph.

use std::fmt::Write as _;

use crate::Result;
use crate::model::{Confidence, EdgeKind, NodeKind};
use crate::resolve::Derived;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DOC_PREFIX: &str = "webdoc:";

/// One documentation page linked to the project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocSnapshot {
    /// File-safe id (hex of the URL hash, see `kor_runtime::chrome_assist::docs`).
    pub id: String,
    pub url: String,
    pub title: String,
    /// Package name the page documents (`react`, `serde`), when known.
    pub library: String,
    /// Version the page describes, when the page or URL states one.
    pub version: String,
    /// Version the project uses, when found in its manifests.
    pub project_version: String,
    /// Unix seconds.
    pub fetched_at: u64,
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `<root>/.kor/graph/docs/`.
pub fn docs_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("graph").join("docs")
}

/// Write `<root>/.kor/graph/docs/<id>.json` atomically. Returns the path.
///
/// # Errors
/// If the id is not file-safe or the file cannot be written.
pub fn save(root: &Path, snap: &DocSnapshot) -> Result<PathBuf> {
    if !valid_id(&snap.id) {
        return Err(crate::Error::Host(snap.id.clone()));
    }
    let dir = docs_dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", snap.id));
    let tmp = dir.join(format!("{}.json.tmp", snap.id));
    std::fs::write(&tmp, serde_json::to_vec_pretty(snap)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every readable snapshot of the project at `root`, by id.
pub fn load_snapshots(root: &Path) -> Vec<DocSnapshot> {
    let Ok(rd) = std::fs::read_dir(docs_dir(root)) else {
        return Vec::new();
    };
    let mut out: Vec<DocSnapshot> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<DocSnapshot>(&b).ok())
        .filter(|s| valid_id(&s.id))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Doc nodes and manifest → doc edges.
pub(crate) fn layer(d: &mut Derived, snaps: &[DocSnapshot]) {
    let manifest = crate::infra::ROOT_MANIFESTS
        .iter()
        .find(|m| d.nodes.contains_key(**m))
        .map(|m| (*m).to_owned());
    for s in snaps {
        let id = format!("{DOC_PREFIX}{}", s.id);
        let mut text = s.url.clone();
        if !s.library.is_empty() {
            let _ = write!(text, "\nlibrary {} {}", s.library, s.version);
        }
        let name = if s.title.is_empty() { &s.url } else { &s.title };
        d.node(&id, NodeKind::Doc, name, None, None, &text);
        if let Some(m) = manifest.as_deref().filter(|_| !s.library.is_empty()) {
            d.edge(
                m,
                &id,
                EdgeKind::DocumentedBy,
                Confidence::Inferred { score: 0.7 },
            );
        }
    }
}

impl crate::Graph {
    /// Documentation pages linked to this project (`.kor/graph/docs/*.json`).
    pub fn web_docs(&self) -> &[DocSnapshot] {
        &self.docs
    }

    /// Re-read `.kor/graph/docs/*.json`; recomputes only when a snapshot changed.
    ///
    /// # Errors
    /// If the graph cannot be saved.
    pub fn refresh_docs(&mut self) -> Result<crate::UpdateStats> {
        let snaps = load_snapshots(&self.root);
        if snaps == self.docs {
            return Ok(crate::UpdateStats::default());
        }
        self.docs = snaps;
        self.recompute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    #[test]
    fn doc_pages_become_nodes_linked_to_the_manifest() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"app\"\n[dependencies]\nserde = \"1\"\n",
        )?;
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n")?;
        let snap = DocSnapshot {
            id: "0123456789abcdef".into(),
            url: "https://docs.rs/serde/1.0.0/serde/".into(),
            title: "serde".into(),
            library: "serde".into(),
            version: "1.0.0".into(),
            ..DocSnapshot::default()
        };
        save(root, &snap)?;
        let mut g = Graph::open(root)?;
        g.update()?;
        let node = g.node("webdoc:0123456789abcdef").map(|n| n.kind);
        assert_eq!(node, Some(NodeKind::Doc));
        assert!(g.edges().any(|e| e.source == "Cargo.toml"
            && e.target == "webdoc:0123456789abcdef"
            && e.kind == EdgeKind::DocumentedBy));
        assert_eq!(g.web_docs().len(), 1);
        // Unsafe ids are refused.
        let bad = DocSnapshot {
            id: "../x".into(),
            ..DocSnapshot::default()
        };
        assert!(save(root, &bad).is_err());
        Ok(())
    }
}
