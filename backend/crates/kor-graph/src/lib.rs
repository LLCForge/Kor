//! Kor's live project graph (spec §7, §15.15): code structure, call graph, config/env, git
//! co-change and rationale, built without an LLM and kept up to date incrementally.
//!
//! # API
//!
//! ```no_run
//! # fn main() -> kor_graph::Result<()> {
//! use kor_graph::{Format, Graph};
//! let mut g = Graph::open(".")?;               // loads <root>/.kor/graph/graph.sqlite (creates it)
//! let stats = g.update()?;                      // full scan; reparses only files whose SHA-256 changed
//! g.update_files(&["src/main.rs".into()])?;     // incremental update for given paths (watcher path)
//! let status = g.status();                      // nodes / edges / communities / last update
//! let ex = g.explain("Auth");                   // Option<Explanation>: source, community, callers, callees, churn, co-change
//! let p = g.path("Login", "Db");                // Option<Vec<PathStep>>: shortest path, either edge direction
//! let hits = g.query("payment error logged", 20); // keyword / identifier ranking
//! let hot = g.hotspots(20);                     // degree × (churn_30d + 1)
//! let imp = g.impact("src/db.rs", 3);           // transitive dependents of a symbol or file
//! let d = kor_graph::diff(".", "HEAD~5")?;      // graph at a git revision vs now
//! g.export(Format::Graphify, &Format::Graphify.default_path(g.root()))?;
//! let w = kor_graph::watch(".", |r| { let _ = r; })?; // debounced watcher; w.stop() or drop to stop
//! # let _ = (stats, status, ex, p, hits, hot, imp, d); w.stop(); Ok(()) }
//! ```
//!
//! Layers on top of the code graph (`layers.rs`): monorepo packages from [`workspace`] become
//! `pkg:<name>` [`NodeKind::Package`] nodes with `Contains` edges to their files and `DependsOn`
//! edges between packages ([`Graph::workspace`], [`Graph::package_of`]); project-memory
//! decisions given with [`Graph::set_memory_decisions`] become `memory:<n>` decision nodes with
//! `DecidedBy` edges. The database-schema layer (`db.rs`) adds `table:<name>` [`NodeKind::Table`]
//! nodes from schema files and live snapshots (`.kor/graph/db/<connection>.json`, written by
//! `kor db connect`), `References` foreign-key edges and `ReadsTable` / `WritesTable` edges from
//! code ([`Graph::tables`], [`Graph::table`], [`Graph::refresh_db`]). The infrastructure layer
//! (`infra.rs`, `kor discover`) adds `server:<host>` / `service:<host>/<kind>/<name>` nodes and
//! `RunsOn` edges code → service → server from `.kor/graph/infra/<host>.json`
//! ([`Graph::refresh_infra`]).
//!
//! Result types ([`Status`], [`Explanation`], [`PathStep`], [`QueryHit`], [`Hotspot`], [`Impact`],
//! [`GraphDiff`], [`UpdateStats`], [`ExportReport`]) are plain `Serialize` structs.
//!
//! Storage: `<root>/.kor/graph/graph.sqlite` and `<root>/.kor/graph/ast-cache/<sha256>-<kind>.json`.
//! `.env*` files contribute variable names only; values are never read into the graph.

pub mod comments;
pub mod community;
pub mod db;
pub mod export;
pub mod git;
pub mod graph;
pub mod infra;
pub mod lang;
pub mod layers;
pub mod model;
pub mod parse;
pub mod query;
pub mod resolve;
pub mod store;
pub mod symbols;
pub mod watch;
// Web documentation pages read through the side panel (§27.11, ADR 0058).
pub mod webdocs;
pub mod workspace;

pub use db::{DbSnapshot, LiveTable, SlowQuery, TableInfo};
pub use export::{ExportReport, Format};
pub use graph::{Graph, UpdateStats};
pub use infra::InfraSnapshot;
pub use layers::{MediaItem, MemoryDecision};
pub use model::{Community, Confidence, Edge, EdgeKind, GraphNode, NodeId, NodeKind, NodeRef};
pub use query::{
    EdgeChange, Explanation, GraphDiff, Hotspot, Impact, ImpactEntry, Link, PathStep, QueryHit,
    Status, diff,
};
pub use store::ranked_files;
pub use watch::{WatchHandle, watch, watch_graph};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a directory: {0}")]
    NotADirectory(std::path::PathBuf),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("graph database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("git: {0}")]
    Git(String),
    #[error("file watcher: {0}")]
    Watch(#[from] notify::Error),
    #[error("unknown export format: {0}")]
    Format(String),
    #[error("graph lock poisoned")]
    Poisoned,
    #[error("invalid database connection name: {0}")]
    Connection(String),
    #[error("invalid host name: {0}")]
    Host(String),
}

pub type Result<T> = std::result::Result<T, Error>;
