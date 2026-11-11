//! Graph data model (spec §7.4).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stable node identifier: `<file>` for a file/module node, `<file>#<Qualified.name>` for a symbol,
/// `env:<KEY>` for an environment variable. Ids never contain line numbers so they survive edits.
pub type NodeId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Function,
    /// Struct, class, enum, type alias.
    Struct,
    /// Trait or interface.
    Trait,
    /// A source file.
    Module,
    Table,
    EnvVar,
    /// A config file (`*.toml`, `*.yaml`, `.env*`).
    Config,
    Doc,
    /// An ADR file or a `WHY:`/`NOTE:` comment.
    Decision,
    /// A workspace package (monorepo layer, `workspace.rs`).
    Package,
    /// A machine found by `kor discover` (infrastructure layer, `infra.rs`).
    Server,
    /// A systemd unit, container, Compose service or VM/LXC on a server (infrastructure layer).
    Service,
    /// A generated image/video/audio in `kor-media/` (media layer, §20.3, ADR 0060).
    Media,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Trait => "trait",
            Self::Module => "module",
            Self::Table => "table",
            Self::EnvVar => "env_var",
            Self::Config => "config",
            Self::Doc => "doc",
            Self::Decision => "decision",
            Self::Package => "package",
            Self::Server => "server",
            Self::Service => "service",
            Self::Media => "media",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "function" => Self::Function,
            "struct" => Self::Struct,
            "trait" => Self::Trait,
            "module" => Self::Module,
            "table" => Self::Table,
            "env_var" => Self::EnvVar,
            "config" => Self::Config,
            "doc" => Self::Doc,
            "decision" => Self::Decision,
            "package" => Self::Package,
            "server" => Self::Server,
            "service" => Self::Service,
            "media" => Self::Media,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EdgeKind {
    Calls,
    Imports,
    Implements,
    Extends,
    Contains,
    ReadsTable,
    WritesTable,
    ReadsEnv,
    CoChanges {
        ratio: f32,
    },
    DocumentedBy,
    DecidedBy,
    /// Package → package dependency (monorepo layer).
    DependsOn,
    /// Table → table foreign key (database-schema layer).
    References,
    /// Code → service and service → server (infrastructure layer, `kor discover`).
    RunsOn,
    /// Generated media → the input it was made from (media layer, §20.3).
    DerivedFrom,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Implements => "implements",
            Self::Extends => "extends",
            Self::Contains => "contains",
            Self::ReadsTable => "reads_table",
            Self::WritesTable => "writes_table",
            Self::ReadsEnv => "reads_env",
            Self::CoChanges { .. } => "co_changes",
            Self::DocumentedBy => "documented_by",
            Self::DecidedBy => "decided_by",
            Self::DependsOn => "depends_on",
            Self::References => "references",
            Self::RunsOn => "runs_on",
            Self::DerivedFrom => "derived_from",
        }
    }

    /// Inverse of [`EdgeKind::as_str`]; `ratio` is only used for `co_changes`.
    pub fn parse(s: &str, ratio: f32) -> Option<Self> {
        Some(match s {
            "calls" => Self::Calls,
            "imports" => Self::Imports,
            "implements" => Self::Implements,
            "extends" => Self::Extends,
            "contains" => Self::Contains,
            "reads_table" => Self::ReadsTable,
            "writes_table" => Self::WritesTable,
            "reads_env" => Self::ReadsEnv,
            "co_changes" => Self::CoChanges { ratio },
            "documented_by" => Self::DocumentedBy,
            "decided_by" => Self::DecidedBy,
            "depends_on" => Self::DependsOn,
            "references" => Self::References,
            "runs_on" => Self::RunsOn,
            "derived_from" => Self::DerivedFrom,
            _ => return None,
        })
    }

    /// True for edges meaning "source depends on target" (used by impact analysis).
    pub fn is_dependency(self) -> bool {
        matches!(
            self,
            Self::Calls
                | Self::Imports
                | Self::Implements
                | Self::Extends
                | Self::ReadsEnv
                | Self::ReadsTable
                | Self::WritesTable
                | Self::DependsOn
                | Self::References
                | Self::RunsOn
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "level")]
pub enum Confidence {
    /// Read directly from the source (same file, resolved import).
    Extracted,
    /// Resolved by a heuristic; `score` in 0..=1.
    Inferred { score: f32 },
    /// Several candidates matched; the edge may be wrong.
    Ambiguous,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "extracted",
            Self::Inferred { .. } => "inferred",
            Self::Ambiguous => "ambiguous",
        }
    }

    pub fn score(self) -> f32 {
        match self {
            Self::Extracted => 1.0,
            Self::Inferred { score } => score,
            Self::Ambiguous => 0.3,
        }
    }

    pub fn parse(s: &str, score: f32) -> Self {
        match s {
            "extracted" => Self::Extracted,
            "inferred" => Self::Inferred { score },
            _ => Self::Ambiguous,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    /// Path relative to the project root, `/`-separated.
    pub file: Option<PathBuf>,
    /// 1-based first and last line.
    pub span: Option<(u32, u32)>,
    pub community: u16,
    /// Number of non-`Contains` edges touching the node.
    pub degree: u32,
    /// Commits touching the node's file in the last 30 days.
    pub churn_30d: u16,
    /// Extra text: comment text for rationale nodes, title for ADRs, language for modules.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

impl Edge {
    pub fn key(&self) -> (NodeId, NodeId, &'static str) {
        (self.source.clone(), self.target.clone(), self.kind.as_str())
    }
}

/// Compact node reference used in query results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

impl From<&GraphNode> for NodeRef {
    fn from(n: &GraphNode) -> Self {
        Self {
            id: n.id.clone(),
            kind: n.kind,
            name: n.name.clone(),
            file: n.file.as_ref().map(|f| f.to_string_lossy().into_owned()),
            line: n.span.map(|s| s.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Community {
    pub id: u16,
    pub label: String,
    pub size: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for k in [
            EdgeKind::Calls,
            EdgeKind::CoChanges { ratio: 0.5 },
            EdgeKind::DependsOn,
            EdgeKind::DecidedBy,
            EdgeKind::References,
            EdgeKind::ReadsTable,
            EdgeKind::RunsOn,
        ] {
            assert_eq!(EdgeKind::parse(k.as_str(), 0.5), Some(k));
        }
        for k in [
            NodeKind::Function,
            NodeKind::Package,
            NodeKind::EnvVar,
            NodeKind::Server,
            NodeKind::Service,
        ] {
            assert_eq!(NodeKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(
            Confidence::parse("inferred", 0.6),
            Confidence::Inferred { score: 0.6 }
        );
    }
}
