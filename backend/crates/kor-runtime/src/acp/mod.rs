//! Agent Client Protocol (spec §28.8, ADR 0057), protocol version 1, JSON-RPC 2.0 over stdio:
//! Kor as an ACP **agent** for editors ([`agent`], `kor acp`) and as an ACP **client** that runs
//! other ACP agents as sub-agents ([`client`], `kor acp run`, the `AcpAgent` tool).

use std::sync::Arc;

pub mod agent;
pub mod client;
pub mod rpc;
#[cfg(test)]
mod tests;

/// `kor acp`: serve one editor on this process's stdin/stdout until it closes the pipe. Nothing
/// else may write to stdout meanwhile (notes go to the session as updates).
pub async fn serve_stdio(l: kor_i18n::Localizer, loaded: kor_config::Loaded) {
    let engine = Arc::new(agent::KorEngine {
        l: kor_i18n::Localizer::new(l.lang()),
        loaded,
    });
    let handler = Arc::new(agent::AcpAgent::new(l, engine));
    let (_rpc, run) = rpc::connect(tokio::io::stdin(), tokio::io::stdout(), handler);
    run.await;
}

/// The command an editor runs to start Kor as its agent, and Zed's settings block for it.
pub fn editor_setup(exe: &std::path::Path) -> serde_json::Value {
    let command = exe.display().to_string();
    serde_json::json!({
        "command": command,
        "args": ["acp"],
        "protocolVersion": agent::PROTOCOL_VERSION,
        "zed": { "agent_servers": { "Kor": { "command": command, "args": ["acp"], "env": {} } } },
    })
}
