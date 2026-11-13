//! `kor lsp` (§12.3) spoken over in-memory pipes with Content-Length framing, as an editor would:
//! initialize, diagnostics from Kor's checks, code actions, hover from the project graph, the
//! fix command (with its permission request answered through `window/showMessageRequest`), the
//! explain command, shutdown and exit.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kor_runtime::lsp::{self, Server};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;

struct Editor {
    w: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    rx: mpsc::UnboundedReceiver<Value>,
    next: u64,
    notes: Vec<Value>,
    asked: Vec<Value>,
}

impl Editor {
    async fn send(&mut self, v: &Value) {
        let body = v.to_string();
        let msg = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        self.w.write_all(msg.as_bytes()).await.unwrap();
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    /// Next message from the server; answers its permission questions with "Allow".
    async fn recv(&mut self) -> Value {
        let v = tokio::time::timeout(Duration::from_secs(30), self.rx.recv())
            .await
            .expect("server answered in time")
            .expect("server open");
        if v["method"] == "window/showMessageRequest" {
            self.asked.push(v.clone());
            let allow = v["params"]["actions"][0].clone();
            self.send(&json!({ "jsonrpc": "2.0", "id": v["id"], "result": allow }))
                .await;
        } else if v.get("method").is_some() {
            self.notes.push(v.clone());
        }
        v
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let v = self.recv().await;
            if v["id"] == id && v.get("method").is_none() {
                assert!(v.get("error").is_none(), "{method}: {v}");
                return v["result"].clone();
            }
        }
    }

    async fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        loop {
            let v = self.recv().await;
            if v["method"] == "textDocument/publishDiagnostics" && v["params"]["uri"] == uri {
                return v["params"]["diagnostics"].as_array().unwrap().clone();
            }
        }
    }
}

/// Read Content-Length framed messages into a channel.
fn reader(r: tokio::io::ReadHalf<tokio::io::DuplexStream>) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut r = BufReader::new(r);
        loop {
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some(n) = line.strip_prefix("Content-Length: ") {
                    len = n.parse().unwrap();
                }
            }
            let mut body = vec![0u8; len];
            r.read_exact(&mut body).await.unwrap();
            if tx.send(serde_json::from_slice(&body).unwrap()).is_err() {
                return;
            }
        }
    });
    rx
}

fn uri(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    format!("file:///{}", s.trim_start_matches('/'))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)] // one editor session
async fn editor_session_over_pipes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = kor_tools::without_verbatim(&std::fs::canonicalize(tmp.path()).unwrap());
    let app = root.join("app.js");
    let code = "function run(code) {\n  return eval(code);\n}\n\nfunction main() {\n  return run(\"1\");\n}\n";
    std::fs::write(&app, code).unwrap();

    let factory = common::Factory::default();
    let server = Arc::new(Server::new(
        kor_i18n::Localizer::new(kor_i18n::Lang::En),
        Arc::new(factory.clone()),
    ));
    let (client, server_end) = tokio::io::duplex(1 << 20);
    let (server_in, server_out) = tokio::io::split(server_end);
    let session = tokio::spawn(lsp::serve(server, server_in, server_out));
    let (cr, cw) = tokio::io::split(client);
    let mut ed = Editor {
        w: cw,
        rx: reader(cr),
        next: 0,
        notes: Vec::new(),
        asked: Vec::new(),
    };

    let init = ed
        .request(
            "initialize",
            json!({ "processId": null, "rootUri": uri(&root), "capabilities": {} }),
        )
        .await;
    let caps = &init["capabilities"];
    assert_eq!(caps["hoverProvider"], true);
    assert_eq!(caps["textDocumentSync"]["change"], 1);
    assert_eq!(
        caps["executeCommandProvider"]["commands"],
        json!(["kor.fix", "kor.explain", "kor.runTask"])
    );
    assert_eq!(init["serverInfo"]["name"], "kor");
    ed.notify("initialized", json!({})).await;

    // Diagnostics from Kor's checks on the open buffer.
    let doc = uri(&app);
    ed.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": doc, "languageId": "javascript", "version": 1, "text": code } }),
    )
    .await;
    let diags = ed.diagnostics(&doc).await;
    let eval = diags
        .iter()
        .find(|d| d["code"] == "kor.eval")
        .unwrap_or_else(|| panic!("{diags:?}"))
        .clone();
    assert_eq!(eval["range"]["start"]["line"], 1);
    assert_eq!(eval["severity"], 2);
    assert_eq!(eval["source"], "kor");

    // A change that removes the problem clears it.
    ed.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": doc, "version": 2 }, "contentChanges": [{ "text": "const x = 1;\n" }] }),
    )
    .await;
    assert!(ed.diagnostics(&doc).await.is_empty());
    ed.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": doc, "version": 3 }, "contentChanges": [{ "text": code }] }),
    )
    .await;
    assert_eq!(ed.diagnostics(&doc).await.len(), diags.len());

    // Code actions: fix the diagnostic, explain the range.
    let actions = ed
        .request(
            "textDocument/codeAction",
            json!({ "textDocument": { "uri": doc }, "range": eval["range"], "context": { "diagnostics": [eval] } }),
        )
        .await;
    let fix = actions
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["command"]["command"] == "kor.fix")
        .unwrap()
        .clone();
    assert_eq!(fix["kind"], "quickfix");
    assert!(fix["title"].as_str().unwrap().starts_with("Kor: fix"));
    assert!(
        actions
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["command"]["command"] == "kor.explain")
    );

    // Hover from the project graph (built in the background after `initialized`).
    let mut hover = Value::Null;
    for _ in 0..100 {
        hover = ed
            .request(
                "textDocument/hover",
                json!({ "textDocument": { "uri": doc }, "position": { "line": 0, "character": 10 } }),
            )
            .await;
        if !hover.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let md = hover["contents"]["value"].as_str().unwrap_or_default();
    assert!(md.contains("`run`") && md.contains("app.js"), "{hover}");

    // The fix command runs the agent; its write is asked in the editor and allowed.
    factory.turns.lock().unwrap().extend([
        common::call(
            "c1",
            "Write",
            &json!({ "file_path": "safe.js", "content": "export const parse = JSON.parse;\n" }),
        ),
        common::text("Replaced eval with JSON.parse."),
    ]);
    let out = ed
        .request(
            "workspace/executeCommand",
            json!({ "command": "kor.fix", "arguments": fix["command"]["arguments"] }),
        )
        .await;
    assert_eq!(out["text"], "Replaced eval with JSON.parse.");
    assert_eq!(ed.asked.len(), 1, "one permission question");
    assert!(
        ed.asked[0]["params"]["message"]
            .as_str()
            .unwrap()
            .contains("Write")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("safe.js")).unwrap(),
        "export const parse = JSON.parse;\n"
    );
    let prompt = format!("{:?}", factory.seen.lock().unwrap()[0]);
    assert!(
        prompt.contains("app.js:2") && prompt.contains("kor.eval"),
        "{prompt}"
    );

    // Explain: read-only, the selected code goes to the model.
    let out = ed
        .request(
            "workspace/executeCommand",
            json!({ "command": "kor.explain", "arguments": [{ "uri": doc, "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 6, "character": 1 } } }] }),
        )
        .await;
    assert_eq!(out["text"], common::REPLY);
    let last = format!("{:?}", factory.seen.lock().unwrap().last().unwrap());
    assert!(last.contains("function main()"), "{last}");
    assert!(
        ed.notes
            .iter()
            .any(|n| n["method"] == "window/showMessage" && n["params"]["message"] == common::REPLY)
    );

    // Unknown commands and methods are errors, not crashes.
    ed.next += 1;
    let id = ed.next;
    ed.send(&json!({ "jsonrpc": "2.0", "id": id, "method": "workspace/executeCommand", "params": { "command": "kor.nope" } }))
        .await;
    loop {
        let v = ed.recv().await;
        if v["id"] == id {
            assert_eq!(v["error"]["code"], -32601);
            break;
        }
    }

    assert!(ed.request("shutdown", Value::Null).await.is_null());
    ed.notify("exit", Value::Null).await;
    tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .expect("exit ends the server")
        .unwrap();
}
