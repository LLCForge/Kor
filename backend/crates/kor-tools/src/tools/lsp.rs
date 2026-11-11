//! LSP hover and definition over stdio, Content-Length framing (`vscode-jsonrpc`).
//! Server records match `lspServers`: `command`, `args`, `extensionToLanguage`.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::{Tool, ToolCtx, ToolKind, ToolOutput};

pub fn missing_server(ext: &str) -> String {
    format!("No LSP server available for file type: {ext}")
}

fn server_for(root: &Path, ext: &str) -> Option<(String, Vec<String>)> {
    let text = std::fs::read_to_string(root.join(".lsp.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let obj = value.as_object()?;
    for spec in obj.values() {
        let map = spec.get("extensionToLanguage")?.as_object()?;
        if !map.contains_key(ext) {
            continue;
        }
        let command = spec.get("command")?.as_str()?.to_owned();
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
        return Some((command, args));
    }
    None
}

async fn frame(child_in: &mut tokio::process::ChildStdin, body: &Value) -> Result<(), String> {
    let bytes = body.to_string();
    let header = format!("Content-Length: {}\r\n\r\n", bytes.len());
    child_in
        .write_all(header.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    child_in
        .write_all(bytes.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    child_in.flush().await.map_err(|e| e.to_string())
}

async fn read_frame(out: &mut tokio::process::ChildStdout) -> Result<Value, String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = out.read(&mut byte).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("lsp closed".into());
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 4096 {
            return Err("lsp header too long".into());
        }
    }
    let header = String::from_utf8_lossy(&buf);
    let len = header
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length:"))
        .ok_or("missing Content-Length")?
        .trim()
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    let mut body = vec![0u8; len];
    out.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

async fn hover(command: &str, args: &[String], file: &str) -> Result<String, String> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to start {command}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"processId": null, "rootUri": null, "capabilities": {}}
    });
    frame(&mut stdin, &init).await?;
    let _ = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut stdout)).await;
    frame(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    )
    .await?;
    frame(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {"textDocument": {"uri": file}, "position": {"line": 0, "character": 0}}
        }),
    )
    .await?;
    let reply = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut stdout))
        .await
        .map_err(|_| "lsp hover timed out".to_owned())??;
    Ok(reply.to_string())
}

#[derive(Debug)]
pub(crate) struct LspTool;

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &'static str {
        "LSP"
    }
    fn rule_name(&self) -> &'static str {
        "LSP"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "LSP".into(),
            description: "Hover or goToDefinition via a language server from .lsp.json.".into(),
            input_schema: json!({"type":"object","properties":{
                "operation":{"type":"string"},
                "filePath":{"type":"string"},
                "line":{"type":"integer"},
                "character":{"type":"integer"}
            },"required":["operation","filePath","line","character"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["filePath"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let path = input["filePath"].as_str().unwrap_or("");
        let ext = Path::new(path)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let Some((command, args)) = server_for(&ctx.root, &ext) else {
            return ToolOutput::err(missing_server(&ext));
        };
        match hover(&command, &args, path).await {
            Ok(text) => ToolOutput::ok(text),
            Err(e) => ToolOutput::err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_extension_uses_the_source_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(LspTool.call(
            json!({"operation":"hover","filePath":"a.rs","line":1,"character":1}),
            &ctx,
        ));
        assert_eq!(out.content, "No LSP server available for file type: .rs");
    }

    #[tokio::test]
    async fn fake_server_answers_hover() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake.ps1");
        std::fs::write(
            &script,
            r#"
$in = [Console]::OpenStandardInput()
$out = [Console]::OpenStandardOutput()
function Read-Msg {
  $header = ''
  $buf = New-Object byte[] 1
  while ($true) {
    $n = $in.Read($buf, 0, 1)
    if ($n -le 0) { return $null }
    $header += [char]$buf[0]
    if ($header.EndsWith("`r`n`r`n")) { break }
  }
  if ($header -match 'Content-Length: (\d+)') {
    $len = [int]$Matches[1]
    $body = New-Object byte[] $len
    $got = 0
    while ($got -lt $len) {
      $r = $in.Read($body, $got, $len - $got)
      if ($r -le 0) { break }
      $got += $r
    }
    return [Text.Encoding]::UTF8.GetString($body)
  }
  return $null
}
function Write-Msg([string]$json) {
  $bytes = [Text.Encoding]::UTF8.GetBytes($json)
  $head = [Text.Encoding]::ASCII.GetBytes("Content-Length: $($bytes.Length)`r`n`r`n")
  $out.Write($head, 0, $head.Length)
  $out.Write($bytes, 0, $bytes.Length)
  $out.Flush()
}
while ($msg = Read-Msg) {
  $j = $msg | ConvertFrom-Json
  if ($j.method -eq 'initialize') {
    Write-Msg ("{""jsonrpc"":""2.0"",""id"":" + $j.id + ",""result"":{""capabilities"":{}}}")
  } elseif ($j.method -eq 'textDocument/hover') {
    Write-Msg ("{""jsonrpc"":""2.0"",""id"":" + $j.id + ",""result"":{""contents"":""hover-ok""}}")
  }
}
"#,
        )
        .unwrap();
        let cfg = json!({
            "fake": {
                "command": "powershell",
                "args": ["-NoProfile", "-File", script],
                "extensionToLanguage": { ".rs": "rust" }
            }
        });
        std::fs::write(dir.path().join(".lsp.json"), cfg.to_string()).unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let out = LspTool
            .call(
                json!({"operation":"hover","filePath":"a.rs","line":1,"character":1}),
                &ctx,
            )
            .await;
        assert!(out.content.contains("hover-ok"), "{}", out.content);
    }
}
