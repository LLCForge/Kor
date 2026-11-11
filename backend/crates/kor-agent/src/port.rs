//! Helpers whose behavior does not need the network.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Escape U+2028 and U+2029 so a line split cannot cut JSON.
pub fn ndjson_line(value: &Value) -> String {
    value
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Proxy URL from the environment. Lowercase wins. Does not connect.
pub fn proxy_url(env: &[(&str, Option<&str>)]) -> Option<String> {
    for key in ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"] {
        if let Some((_, Some(v))) = env.iter().find(|(k, _)| *k == key)
            && !v.is_empty()
        {
            return Some((*v).to_owned());
        }
    }
    None
}

/// `remoteManagedSettings` endpoint is `{BASE_API_URL}/api/claude_code/settings`.
/// The client stays off unless `enabled` is set.
pub fn remote_settings_payload(enabled: bool, body: &Value) -> Option<Value> {
    if !enabled {
        return None;
    }
    Some(json!({
        "endpoint": "https://api.anthropic.com/api/claude_code/settings",
        "body": body
    }))
}

/// `PaneBackendExecutor` teammate argv when the leader is not inside tmux.
pub fn teammate_argv(
    bin: &str,
    agent_id: &str,
    name: &str,
    team: &str,
    color: &str,
    parent: &str,
) -> Vec<String> {
    vec![
        bin.into(),
        "--agent-id".into(),
        agent_id.into(),
        "--agent-name".into(),
        name.into(),
        "--team-name".into(),
        team.into(),
        "--agent-color".into(),
        color.into(),
        "--parent-session-id".into(),
        parent.into(),
        "--teammate-mode".into(),
        "in-process".into(),
    ]
}

/// URL elicitation. `decline` and `cancel` block the tool call.
pub fn elicitation_allows(action: &str) -> bool {
    action == "accept"
}

pub fn mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// One reconnect after the first connect fails.
pub async fn connect_with_retry<F, Fut>(mut connect: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if let Err(first) = connect().await {
        connect().await.map_err(|e| format!("{first}; retry: {e}"))
    } else {
        Ok(())
    }
}

/// `getAgentTranscriptPath`: `{session}/subagents/agent-{id}.jsonl`.
pub fn agent_transcript_path(session_dir: &Path, session_id: &str, agent_id: &str) -> PathBuf {
    session_dir
        .join(session_id)
        .join("subagents")
        .join(format!("agent-{agent_id}.jsonl"))
}

pub fn resume_transcript(agent_id: &str, text: Option<&str>) -> Result<String, String> {
    match text.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Ok(s.to_owned()),
        None => Err(format!("No transcript found for agent ID: {agent_id}")),
    }
}

/// `.kor/agent-memory-local/<type>/` with `:` replaced by `-`.
pub fn agent_memory_dir(root: &Path, agent_type: &str) -> PathBuf {
    root.join(".kor")
        .join("agent-memory-local")
        .join(agent_type.replace(':', "-"))
}

pub fn clear_old_tool_results(contents: &mut [String], keep_recent: usize) {
    let clearable = contents.len().saturating_sub(keep_recent);
    for item in contents.iter_mut().take(clearable) {
        *item = "[Old tool result content cleared]".into();
    }
}

/// Parse a keystroke such as `ctrl+k`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keystroke {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub super_key: bool,
}

pub fn parse_keystroke(input: &str) -> Keystroke {
    let mut key = Keystroke {
        key: String::new(),
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
        super_key: false,
    };
    for part in input.split('+') {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => key.ctrl = true,
            "alt" | "opt" | "option" => key.alt = true,
            "shift" => key.shift = true,
            "meta" => key.meta = true,
            "cmd" | "command" | "super" | "win" => key.super_key = true,
            "esc" => key.key = "escape".into(),
            "return" => key.key = "enter".into(),
            other => other.clone_into(&mut key.key),
        }
    }
    key
}

/// Truncate so the file ends at the previous user message.
pub fn rewind_to_prior_user(roles_user: &[bool]) -> usize {
    let users: Vec<usize> = roles_user
        .iter()
        .enumerate()
        .filter_map(|(i, u)| u.then_some(i))
        .collect();
    if users.len() < 2 {
        return roles_user.len();
    }
    users[users.len() - 2] + 1
}

pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let mut out = format!("--- {path}\n+++ {path}\n");
    for line in old.lines() {
        if !new.lines().any(|n| n == line) {
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
    }
    for line in new.lines() {
        if !old.lines().any(|n| n == line) {
            out.push_str("+ ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct Tips {
    shown: bool,
}

impl Tips {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn once(&mut self) -> Option<&'static str> {
        if self.shown {
            return None;
        }
        self.shown = true;
        Some("Use /help to list commands.")
    }
}

pub fn empty_prompt_hint(last_assistant: &str) -> String {
    if last_assistant.trim().is_empty() {
        "Ask about the project.".into()
    } else {
        format!(
            "Continue from: {}",
            last_assistant.chars().take(80).collect::<String>()
        )
    }
}

/// Local half of slash commands that name a URL. Commands without one return nothing.
pub fn command_url(name: &str) -> Option<&'static str> {
    match name {
        "chrome" => Some("https://claude.ai/chrome"),
        "mobile-ios" => Some("https://apps.apple.com/app/claude-by-anthropic/id6473753684"),
        "mobile-android" => {
            Some("https://play.google.com/store/apps/details?id=com.anthropic.claude")
        }
        "install-github-app" => Some("https://github.com/apps/claude"),
        "review" => Some("https://code.claude.com/docs/en/claude-code-on-the-web"),
        _ => None,
    }
}

/// MCP `initialize` reports the server name `claude/tengu`.
pub fn mcp_serve_response(line: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .ok_or("missing method")?;
    let result = match method {
        "initialize" => json!({
            "protocolVersion": value["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),
            "serverInfo": {"name": "kor", "version": kor_core::VERSION},
            "capabilities": {"tools": {}}
        }),
        "tools/list" => json!({
            "tools": [{"name": "Read", "description": "Read a file", "inputSchema": {"type": "object"}}]
        }),
        _ => {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "method not found"}
            })
            .to_string());
        }
    };
    Ok(json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndjson_escapes_line_separators() {
        let line = ndjson_line(&json!({"t": "a\u{2028}b"}));
        assert!(line.contains("\\u2028"));
        assert!(!line.contains('\u{2028}'));
    }

    #[test]
    fn proxy_prefers_lowercase_and_does_not_connect() {
        let url = proxy_url(&[
            ("HTTPS_PROXY", Some("http://upper")),
            ("https_proxy", Some("http://lower")),
        ]);
        assert_eq!(url.as_deref(), Some("http://lower"));
        assert!(proxy_url(&[]).is_none());
    }

    #[test]
    fn remote_settings_stay_off_by_default() {
        assert!(remote_settings_payload(false, &json!({"theme": "dark"})).is_none());
        let on = remote_settings_payload(true, &json!({"theme": "dark"})).unwrap();
        assert_eq!(
            on["endpoint"],
            "https://api.anthropic.com/api/claude_code/settings"
        );
    }

    #[test]
    fn teammate_argv_uses_the_source_flags() {
        let argv = teammate_argv("kor", "a1", "worker", "team", "blue", "sess");
        assert_eq!(argv[0], "kor");
        assert!(argv.windows(2).any(|w| w == ["--agent-name", "worker"]));
        assert!(
            argv.windows(2)
                .any(|w| w == ["--teammate-mode", "in-process"])
        );
    }

    #[test]
    fn decline_blocks_elicitation() {
        assert!(elicitation_allows("accept"));
        assert!(!elicitation_allows("decline"));
        assert!(!elicitation_allows("cancel"));
    }

    #[tokio::test]
    async fn reconnects_once() {
        let n = std::sync::atomic::AtomicUsize::new(0);
        let result = connect_with_retry(|| {
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move { if i == 0 { Err("down".into()) } else { Ok(()) } }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(n.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn resume_returns_the_stored_snippet() {
        let dir = tempfile::tempdir().unwrap();
        let path = agent_transcript_path(dir.path(), "sess", "ag");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "fixed the parser\n").unwrap();
        let text = std::fs::read_to_string(&path).ok();
        assert_eq!(
            resume_transcript("ag", text.as_deref()).unwrap(),
            "fixed the parser"
        );
        assert!(
            resume_transcript("missing", None)
                .unwrap_err()
                .contains("missing")
        );
        let mem = agent_memory_dir(dir.path(), "plugin:agent");
        assert!(mem.ends_with("plugin-agent"));
    }

    #[test]
    fn microcompact_clears_old_results() {
        let mut items = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        clear_old_tool_results(&mut items, 2);
        assert_eq!(items[0], "[Old tool result content cleared]");
        assert_eq!(items[2], "c");
    }

    #[test]
    fn keystroke_ctrl_k() {
        let k = parse_keystroke("ctrl+k");
        assert!(k.ctrl);
        assert_eq!(k.key, "k");
    }

    #[test]
    fn rewind_ends_at_the_prior_user_message() {
        let roles = [true, false, true, false];
        assert_eq!(rewind_to_prior_user(&roles), 1);
    }

    #[test]
    fn diff_shows_removed_and_added_lines() {
        let d = unified_diff("a.txt", "old\nkeep\n", "keep\nnew\n");
        assert!(d.contains("- old"));
        assert!(d.contains("+ new"));
    }

    #[test]
    fn tip_does_not_repeat() {
        let mut tips = Tips::new();
        assert!(tips.once().is_some());
        assert!(tips.once().is_none());
        assert!(empty_prompt_hint("ship the parser").contains("ship the parser"));
    }

    #[test]
    fn command_urls_match_the_source() {
        assert_eq!(command_url("chrome"), Some("https://claude.ai/chrome"));
        assert_eq!(
            command_url("install-github-app"),
            Some("https://github.com/apps/claude")
        );
        assert!(command_url("bridge").is_none());
        assert!(command_url("usage").is_none());
        assert!(command_url("desktop").is_none());
        assert!(command_url("teleport").is_none());
    }

    #[test]
    fn mcp_serve_initialize_and_tools_list() {
        let init = mcp_serve_response(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
        )
        .unwrap();
        assert!(init.contains("\"name\":\"kor\""), "{init}");
        let list = mcp_serve_response(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert!(list.contains("Read"), "{list}");
    }
}
