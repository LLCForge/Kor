//! The Kor agent: a loop of model call → tool calls (with permission checks and hooks)
//! → results, until the model stops asking for tools.

mod agent;
pub mod ask;
pub mod assistant;
pub mod audit;
pub mod builtin;
// PATH tool catalog, typed tool definitions, `tools_search` (§26.2, ADR 0056).
pub mod clitools;
pub mod compact;
pub mod context;
pub mod cost;
pub mod find;
pub mod flags;
pub mod gallery;
pub mod gitflow;
pub mod graph;
pub mod hooks;
pub mod knowledge;
pub mod mcp;
pub mod mcp_config;
pub mod mcp_gen;
pub mod mcp_oauth;
pub mod mcp_tools;
pub mod media;
pub mod memory;
pub mod modes;
pub mod permissions;
pub mod plugins;
pub mod port;
pub mod prompt;
mod prompt_text;
pub mod recall;
mod record;
pub mod remote;
pub mod repair;
pub mod repomap;
pub mod review;
mod sandbox_report;
pub mod schema;
pub mod semantic;
pub mod session;
pub mod shadow;
pub mod shellint;
mod skill;
pub mod status;
pub mod subagent;
pub mod summarize;
pub mod team_client;
pub mod team_sync;
pub mod toolcall;
pub mod verify;

pub use agent::{
    Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, ScopedRule, StopReason,
};
pub use hooks::{HookOutcome, HookRunner};
pub use permissions::{Decision, Permissions, Rule, RuleError};
pub use review::{FileDecision, Proposal, Review};
pub use skill::SkillTool;
pub use subagent::AgentTool;

/// `KOR_<name>`, falling back to the Claude Code variable a user may already have set.
pub fn env_compat(kor: &str, claude: &str) -> Option<String> {
    std::env::var(kor)
        .ok()
        .or_else(|| std::env::var(claude).ok())
}
