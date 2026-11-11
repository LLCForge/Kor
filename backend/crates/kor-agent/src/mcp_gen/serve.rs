//! A stdio MCP server loop (newline-delimited JSON-RPC 2.0), shared by `kor mcp run <id>`
//! (generated servers) and `kor mcp serve` (Kor's own graph, memory and cost tools).

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt, BufReader,
};

const PROTOCOL_VERSION: &str = "2025-03-26";
/// A longer request line is refused (and skipped) instead of being buffered.
const MAX_LINE: u64 = 4 * 1024 * 1024;

/// What a server offers.
#[async_trait]
pub trait Backend: Send + Sync {
    /// `serverInfo.name`.
    fn name(&self) -> String;
    /// `tools/list` entries (`name`, `description`, `inputSchema`, `annotations`).
    fn tools(&self) -> Vec<Value>;
    /// Run a tool. `Err` becomes an `isError` result, not a protocol error.
    async fn call(&self, name: &str, args: Value) -> Result<String, String>;
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The answer to one message; `None` for notifications and stray responses.
pub async fn handle(b: &dyn Backend, line: &str) -> Option<Value> {
    let Ok(msg) = serde_json::from_str::<Value>(line) else {
        return Some(error(&Value::Null, -32700, "parse error"));
    };
    let id = msg.get("id").cloned()?;
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return None; // a response to something we never send
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "serverInfo": {"name": b.name(), "version": kor_core::VERSION},
            "capabilities": {"tools": {"listChanged": false}},
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": b.tools()}),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(&id, -32602, "missing tool name"));
            };
            if !b.tools().iter().any(|t| t["name"] == name) {
                return Some(error(&id, -32602, &format!("unknown tool `{name}`")));
            }
            let args = params
                .get("arguments")
                .cloned()
                .filter(|a| !a.is_null())
                .unwrap_or_else(|| json!({}));
            match b.call(name, args).await {
                Ok(text) => json!({"content": [{"type": "text", "text": super::cap(text)}]}),
                Err(e) => {
                    json!({"content": [{"type": "text", "text": super::cap(e)}], "isError": true})
                }
            }
        }
        _ => return Some(error(&id, -32601, "method not found")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Answer requests from `read` on `write` until the input ends.
///
/// # Errors
/// When writing fails (the client went away).
pub async fn serve<R, W>(b: &dyn Backend, read: R, mut write: W) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut read = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = (&mut read)
            .take(MAX_LINE)
            .read_until(b'\n', &mut buf)
            .await?;
        if n == 0 {
            return Ok(());
        }
        let reply = if buf.last() != Some(&b'\n') && n as u64 == MAX_LINE {
            // Skip the rest of the oversized line.
            let mut rest = Vec::new();
            read.read_until(b'\n', &mut rest).await?;
            Some(error(&Value::Null, -32600, "request too large"))
        } else {
            let line = String::from_utf8_lossy(&buf);
            if line.trim().is_empty() {
                continue;
            }
            handle(b, line.trim()).await
        };
        if let Some(v) = reply {
            write
                .write_all(format!("{}\n", crate::port::ndjson_line(&v)).as_bytes())
                .await?;
            write.flush().await?;
        }
    }
}
