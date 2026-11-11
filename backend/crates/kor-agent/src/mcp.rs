//! MCP stdio client: newline-delimited JSON-RPC for local servers.
//!
//! `url` servers stay in the list. Call [`crate::mcp_oauth::discover`] with that URL.

use std::path::Path;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Set for `url` servers. Those are kept so OAuth can run; they are not skipped.
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

/// Read `.mcp.json` `mcpServers`. Stdio servers are returned. URL servers are skipped.
pub fn load_config(root: &Path) -> (Vec<McpServer>, Vec<Skipped>) {
    let path = root.join(".mcp.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return (
            Vec::new(),
            vec![Skipped {
                name: ".mcp.json".into(),
                reason: "invalid JSON".into(),
            }],
        );
    };
    let Some(servers) = value.get("mcpServers").and_then(Value::as_object) else {
        return (Vec::new(), Vec::new());
    };
    let mut ok = Vec::new();
    let mut skipped = Vec::new();
    for (name, spec) in servers {
        if let Some(url) = spec.get("url").and_then(Value::as_str) {
            ok.push(McpServer {
                name: name.clone(),
                command: String::new(),
                args: Vec::new(),
                url: Some(url.to_owned()),
            });
            continue;
        }
        let Some(command) = spec.get("command").and_then(Value::as_str) else {
            skipped.push(Skipped {
                name: name.clone(),
                reason: "missing command".into(),
            });
            continue;
        };
        let args = spec
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        ok.push(McpServer {
            name: name.clone(),
            command: command.to_owned(),
            args,
            url: None,
        });
    }
    (ok, skipped)
}

/// One JSON-RPC request and its result object.
pub async fn request<R, W>(
    read: &mut R,
    write: &mut W,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    write
        .write_all(format!("{body}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    write.flush().await.map_err(|e| e.to_string())?;
    let mut line = String::new();
    let n = read.read_line(&mut line).await.map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("server closed".into());
    }
    let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if let Some(err) = value.get("error") {
        return Err(err.to_string());
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "response has no result".into())
}

/// `initialize` then `tools/list`.
pub async fn list_tools<R, W>(read: &mut R, write: &mut W) -> Result<Vec<String>, String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    request(
        read,
        write,
        1,
        "initialize",
        json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "kor", "version": "0"}}),
    )
    .await?;
    let listed = request(read, write, 2, "tools/list", json!({})).await?;
    Ok(listed
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// `tools/call`.
pub async fn call_tool<R, W>(
    read: &mut R,
    write: &mut W,
    name: &str,
    arguments: Value,
) -> Result<Value, String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    request(
        read,
        write,
        3,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, BufReader, duplex};

    async fn fake_server(io: tokio::io::DuplexStream) {
        let (read_half, mut write_half) = tokio::io::split(io);
        let mut read = BufReader::new(read_half);
        let mut line = String::new();
        while read.read_line(&mut line).await.unwrap_or(0) > 0 {
            let req: Value = serde_json::from_str(&line).unwrap_or(json!({}));
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let method = req.get("method").and_then(Value::as_str).unwrap_or("");
            let result = match method {
                "initialize" => json!({"protocolVersion": "2024-11-05", "capabilities": {}}),
                "tools/list" => json!({"tools": [{"name": "echo"}]}),
                "tools/call" => json!({"content": [{"type": "text", "text": "pong"}]}),
                _ => json!({}),
            };
            let body = json!({"jsonrpc": "2.0", "id": id, "result": result});
            let _ = write_half.write_all(format!("{body}\n").as_bytes()).await;
            let _ = write_half.flush().await;
            line.clear();
        }
    }

    #[tokio::test]
    async fn lists_and_calls_over_a_duplex() {
        let (client, server) = duplex(4096);
        tokio::spawn(fake_server(server));
        let (read_half, write_half) = tokio::io::split(client);
        let mut read = BufReader::new(read_half);
        let mut write = write_half;
        let names = list_tools(&mut read, &mut write).await.unwrap();
        assert_eq!(names, vec!["echo".to_owned()]);
        let called = call_tool(&mut read, &mut write, "echo", json!({"text": "hi"}))
            .await
            .unwrap();
        assert!(called.to_string().contains("pong"), "{called}");
    }

    #[test]
    fn config_keeps_stdio_and_skips_urls() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"local":{"command":"tool","args":["--stdio"]},"remote":{"url":"https://example.com"}}}"#,
        )
        .unwrap();
        let (ok, skipped) = load_config(dir.path());
        let local = ok.iter().find(|s| s.name == "local").unwrap();
        let remote = ok.iter().find(|s| s.name == "remote").unwrap();
        assert_eq!(local.command, "tool");
        assert_eq!(remote.url.as_deref(), Some("https://example.com"));
        assert!(skipped.is_empty());
    }
}
