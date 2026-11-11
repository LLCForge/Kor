//! Persistence: `<root>/.kor/graph/graph.sqlite` (nodes, edges, files, communities, meta) and
//! `<root>/.kor/graph/ast-cache/<sha256>-<kind>.json` (per-file facts keyed by content hash).
//! Updates are written as diffs, so an incremental update only touches changed rows.

use crate::Result;
use crate::model::{Community, Confidence, Edge, EdgeKind, GraphNode, NodeId, NodeKind};
use crate::parse::{FACTS_VERSION, FileFacts};
use crate::resolve::EdgeKey;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub sha: String,
    pub size: u64,
    pub mtime: u64,
    /// Cache tag: language name or file kind.
    pub tag: String,
}

#[derive(Debug)]
pub struct Store {
    conn: Connection,
    cache: PathBuf,
}

pub fn graph_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("graph")
}

fn i64_of(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u64_of(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let dir = graph_dir(root);
        let cache = dir.join("ast-cache");
        std::fs::create_dir_all(&cache)?;
        // Derived data: keep it out of the user's commits.
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(ignore, "*\n")?;
        }
        let conn = Connection::open(dir.join("graph.sqlite"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS files(path TEXT PRIMARY KEY, sha TEXT NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL, tag TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS nodes(id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, file TEXT,
                 span_start INTEGER, span_end INTEGER, community INTEGER NOT NULL, degree INTEGER NOT NULL, churn INTEGER NOT NULL, text TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS edges(source TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL, ratio REAL NOT NULL,
                 confidence TEXT NOT NULL, score REAL NOT NULL, PRIMARY KEY(source, target, kind));
             CREATE TABLE IF NOT EXISTS communities(id INTEGER PRIMARY KEY, label TEXT NOT NULL, size INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS nodes_name ON nodes(name);
             CREATE INDEX IF NOT EXISTS edges_target ON edges(target);",
        )?;
        Ok(Self { conn, cache })
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn files(&self) -> Result<Vec<FileRow>> {
        let mut st = self
            .conn
            .prepare("SELECT path, sha, size, mtime, tag FROM files")?;
        let rows = st.query_map([], |r| {
            Ok(FileRow {
                path: r.get(0)?,
                sha: r.get(1)?,
                size: u64_of(r.get(2)?),
                mtime: u64_of(r.get(3)?),
                tag: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn nodes(&self) -> Result<BTreeMap<NodeId, GraphNode>> {
        let mut st = self.conn.prepare("SELECT id, kind, name, file, span_start, span_end, community, degree, churn, text FROM nodes")?;
        let rows = st.query_map([], |r| {
            let kind: String = r.get(1)?;
            let file: Option<String> = r.get(3)?;
            let s: Option<u32> = r.get(4)?;
            let e: Option<u32> = r.get(5)?;
            Ok(GraphNode {
                id: r.get(0)?,
                kind: NodeKind::parse(&kind).unwrap_or(NodeKind::Module),
                name: r.get(2)?,
                file: file.map(PathBuf::from),
                span: s.zip(e),
                community: r.get(6)?,
                degree: r.get(7)?,
                churn_30d: r.get(8)?,
                text: r.get(9)?,
            })
        })?;
        let mut out = BTreeMap::new();
        for n in rows {
            let n = n?;
            out.insert(n.id.clone(), n);
        }
        Ok(out)
    }

    pub fn edges(&self) -> Result<BTreeMap<EdgeKey, Edge>> {
        let mut st = self
            .conn
            .prepare("SELECT source, target, kind, ratio, confidence, score FROM edges")?;
        let rows = st.query_map([], |r| {
            let kind: String = r.get(2)?;
            let ratio: f64 = r.get(3)?;
            let conf: String = r.get(4)?;
            let score: f64 = r.get(5)?;
            #[allow(clippy::cast_possible_truncation)]
            let (ratio, score) = (ratio as f32, score as f32);
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                EdgeKind::parse(&kind, ratio),
                Confidence::parse(&conf, score),
            ))
        })?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (source, target, kind, confidence) = row?;
            if let Some(kind) = kind {
                out.insert(
                    (source.clone(), target.clone(), kind.as_str()),
                    Edge {
                        source,
                        target,
                        kind,
                        confidence,
                    },
                );
            }
        }
        Ok(out)
    }

    pub fn communities(&self) -> Result<Vec<Community>> {
        let mut st = self
            .conn
            .prepare("SELECT id, label, size FROM communities ORDER BY size DESC, id")?;
        let rows = st.query_map([], |r| {
            Ok(Community {
                id: r.get(0)?,
                label: r.get(1)?,
                size: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Write the difference between the old and new state in one transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn save(
        &mut self,
        files_upsert: &[FileRow],
        files_removed: &[String],
        old_nodes: &BTreeMap<NodeId, GraphNode>,
        new_nodes: &BTreeMap<NodeId, GraphNode>,
        old_edges: &BTreeMap<EdgeKey, Edge>,
        new_edges: &BTreeMap<EdgeKey, Edge>,
        communities: &[Community],
        meta: &[(&str, String)],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut del = tx.prepare_cached("DELETE FROM files WHERE path = ?1")?;
            for p in files_removed {
                del.execute([p])?;
            }
            let mut up = tx.prepare_cached("INSERT OR REPLACE INTO files(path, sha, size, mtime, tag) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for f in files_upsert {
                up.execute(params![
                    f.path,
                    f.sha,
                    i64_of(f.size),
                    i64_of(f.mtime),
                    f.tag
                ])?;
            }
            let mut del = tx.prepare_cached("DELETE FROM nodes WHERE id = ?1")?;
            for id in old_nodes.keys().filter(|k| !new_nodes.contains_key(*k)) {
                del.execute([id])?;
            }
            let mut up = tx.prepare_cached(
                "INSERT OR REPLACE INTO nodes(id, kind, name, file, span_start, span_end, community, degree, churn, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for n in new_nodes
                .values()
                .filter(|n| old_nodes.get(&n.id) != Some(n))
            {
                let file = n.file.as_ref().map(|f| f.to_string_lossy().into_owned());
                up.execute(params![
                    n.id,
                    n.kind.as_str(),
                    n.name,
                    file,
                    n.span.map(|s| s.0),
                    n.span.map(|s| s.1),
                    n.community,
                    n.degree,
                    n.churn_30d,
                    n.text
                ])?;
            }
            let mut del = tx.prepare_cached(
                "DELETE FROM edges WHERE source = ?1 AND target = ?2 AND kind = ?3",
            )?;
            for k in old_edges.keys().filter(|k| !new_edges.contains_key(*k)) {
                del.execute(params![k.0, k.1, k.2])?;
            }
            let mut up = tx.prepare_cached("INSERT OR REPLACE INTO edges(source, target, kind, ratio, confidence, score) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for (k, e) in new_edges
                .iter()
                .filter(|(k, e)| old_edges.get(*k) != Some(*e))
            {
                let ratio = if let EdgeKind::CoChanges { ratio } = e.kind {
                    f64::from(ratio)
                } else {
                    0.0
                };
                up.execute(params![
                    k.0,
                    k.1,
                    k.2,
                    ratio,
                    e.confidence.as_str(),
                    f64::from(e.confidence.score())
                ])?;
            }
            tx.execute("DELETE FROM communities", [])?;
            let mut ins = tx.prepare_cached(
                "INSERT OR REPLACE INTO communities(id, label, size) VALUES (?1, ?2, ?3)",
            )?;
            for c in communities {
                ins.execute(params![c.id, c.label, c.size])?;
            }
            let mut m =
                tx.prepare_cached("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?;
            for (k, v) in meta {
                m.execute(params![k, v])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Refresh file rows only (mtime moved, content identical).
    pub fn save_files(&mut self, rows: &[FileRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut up = tx.prepare_cached("INSERT OR REPLACE INTO files(path, sha, size, mtime, tag) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for f in rows {
                up.execute(params![
                    f.path,
                    f.sha,
                    i64_of(f.size),
                    i64_of(f.mtime),
                    f.tag
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn cache_path(&self, sha: &str, tag: &str) -> PathBuf {
        self.cache.join(format!("{sha}-{tag}.json"))
    }

    pub fn cache_read(&self, sha: &str, tag: &str) -> Option<FileFacts> {
        let bytes = std::fs::read(self.cache_path(sha, tag)).ok()?;
        serde_json::from_slice::<FileFacts>(&bytes)
            .ok()
            .filter(|f| f.version == FACTS_VERSION)
    }

    pub fn cache_write(&self, sha: &str, tag: &str, facts: &FileFacts) -> Result<()> {
        let path = self.cache_path(sha, tag);
        if !path.exists() {
            std::fs::write(path, serde_json::to_vec(facts)?)?;
        }
        Ok(())
    }

    /// Delete cache entries no tracked file refers to.
    pub fn cache_gc(&self, live: &HashSet<String>) -> Result<usize> {
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.cache)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                && !live.contains(&name)
            {
                std::fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn cache_name(sha: &str, tag: &str) -> String {
        format!("{sha}-{tag}.json")
    }
}

/// Files of a stored graph ranked like hotspots (`Σ degree × (max churn + 1)`), best first,
/// without loading the graph. `None` when `<root>/.kor/graph/graph.sqlite` does not exist yet.
/// Used for cheap ranking on a hot path (the repository map at session start).
pub fn ranked_files(root: &Path, limit: usize) -> Option<Vec<String>> {
    let db = graph_dir(root).join("graph.sqlite");
    if !db.is_file() {
        return None;
    }
    let conn =
        Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut st = conn
        .prepare(
            "SELECT file FROM nodes WHERE file IS NOT NULL AND kind NOT IN ('package', 'decision')
             GROUP BY file ORDER BY SUM(degree) * (MAX(churn) + 1) DESC, file LIMIT ?1",
        )
        .ok()?;
    let rows = st
        .query_map([i64_of(u64::try_from(limit).unwrap_or(u64::MAX))], |r| r.get::<_, String>(0))
        .ok()?;
    Some(rows.filter_map(std::result::Result::ok).collect())
}
