//! `kor lsp` (spec §12.3): a Language Server on stdin/stdout for VS Code, Neovim, Zed and
//! `JetBrains` IDEs. Diagnostics come from Kor's own checks (`kor audit --security` line rules and
//! secret scanner, run on the open buffer), hover from the project graph (`kor graph explain`),
//! code actions ask Kor to fix a diagnostic or explain a range, and `workspace/executeCommand`
//! runs an agent task. The agent's permission requests become `window/showMessageRequest`
//! questions in the editor. JSON-RPC framing is shared with ACP ([`crate::acp::rpc`]).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest};
use kor_config::Approval;
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::acp::rpc::{self, Framing, Handler, INVALID_PARAMS, METHOD_NOT_FOUND, Rpc, RpcError};
use crate::analysis::{Severity, rules, security, walk::Source};
use crate::triggers::runner::{self, AgentFactory};

pub const FIX: &str = "kor.fix";
pub const EXPLAIN: &str = "kor.explain";
pub const RUN_TASK: &str = "kor.runTask";
/// Largest buffer that is checked (bigger files get no diagnostics).
const MAX_DOC: usize = 1024 * 1024;

#[derive(Default)]
struct State {
    root: Option<PathBuf>,
    docs: HashMap<String, String>,
    rules: Vec<rules::Rule>,
}

/// One editor connection.
pub struct Server {
    l: Localizer,
    factory: Arc<dyn AgentFactory>,
    state: Mutex<State>,
    graph: Arc<Mutex<Option<kor_graph::Graph>>>,
    exit: tokio::sync::Notify,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

impl Server {
    pub fn new(l: Localizer, factory: Arc<dyn AgentFactory>) -> Self {
        Self {
            l,
            factory,
            state: Mutex::default(),
            graph: Arc::default(),
            exit: tokio::sync::Notify::new(),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn root(&self) -> Result<PathBuf, RpcError> {
        self.st()
            .root
            .clone()
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, self.l.t("lsp-no-root")))
    }

    fn rel(&self, path: &Path) -> PathBuf {
        let root = self.st().root.clone().unwrap_or_default();
        path.strip_prefix(&root)
            .map_or_else(|_| path.to_owned(), Path::to_path_buf)
    }

    /// Kor's checks on `text`, as LSP diagnostics.
    fn diagnostics(&self, uri: &str, text: &str) -> Vec<Value> {
        if text.len() > MAX_DOC {
            return Vec::new();
        }
        let Some(path) = uri_to_path(uri) else {
            return Vec::new();
        };
        let src = Source {
            rel: self.rel(&path),
            text: text.to_owned(),
        };
        let mut found = security::secrets(&src, &self.l);
        found.extend(security::line_rules(&src, &self.st().rules, &self.l));
        let lines: Vec<&str> = text.lines().collect();
        found
            .iter()
            .map(|f| {
                let line = f.line.saturating_sub(1);
                let end = lines
                    .get(line as usize)
                    .map_or(0, |l| l.encode_utf16().count());
                let start = f.column.map_or(0, |c| c.saturating_sub(1));
                let mut message = f.message.clone();
                if let Some(fix) = &f.fix {
                    message.push('\n');
                    message.push_str(fix);
                }
                json!({
                    "range": { "start": { "line": line, "character": start }, "end": { "line": line, "character": end.max(start as usize) } },
                    "severity": match f.severity {
                        Severity::Critical | Severity::High => 1,
                        Severity::Medium => 2,
                        Severity::Low => 3,
                        Severity::Info => 4,
                    },
                    "source": "kor",
                    "code": f.rule,
                    "message": message,
                })
            })
            .collect()
    }

    fn publish(&self, rpc: &Rpc, uri: &str, text: Option<&str>) {
        let diags = text.map_or_else(Vec::new, |t| self.diagnostics(uri, t));
        rpc.notify(
            "textDocument/publishDiagnostics",
            json!({ "uri": uri, "diagnostics": diags }),
        );
    }

    fn initialize(&self, p: &Value) -> Value {
        let uri = p["rootUri"]
            .as_str()
            .or_else(|| p.pointer("/workspaceFolders/0/uri").and_then(Value::as_str));
        let root = uri
            .and_then(uri_to_path)
            .or_else(|| p["rootPath"].as_str().map(PathBuf::from))
            .and_then(|r| crate::triggers::canon(&r).or(Some(r)));
        let mut st = self.st();
        if let Some(r) = &root {
            st.rules = rules::load(r).0;
        }
        st.root = root;
        json!({
            "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
                "hoverProvider": true,
                "codeActionProvider": { "codeActionKinds": ["quickfix", "refactor"] },
                "executeCommandProvider": { "commands": [FIX, EXPLAIN, RUN_TASK] },
            },
            "serverInfo": { "name": "kor", "version": env!("CARGO_PKG_VERSION") },
        })
    }

    /// Load the project graph in the background (building it when the project has none yet).
    fn load_graph(&self) {
        let Some(root) = self.st().root.clone() else {
            return;
        };
        let slot = self.graph.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(mut g) = kor_graph::Graph::open(&root) else {
                return;
            };
            if g.status().nodes == 0 {
                let _ = g.update();
            }
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(g);
        });
    }

    fn hover(&self, p: &Value) -> Value {
        let uri = p.pointer("/textDocument/uri").and_then(Value::as_str);
        let (Some(uri), Some(line), Some(ch)) = (
            uri,
            p.pointer("/position/line").and_then(Value::as_u64),
            p.pointer("/position/character").and_then(Value::as_u64),
        ) else {
            return Value::Null;
        };
        let text = self.st().docs.get(uri).cloned().unwrap_or_default();
        let Some(word) = word_at(&text, line, ch) else {
            return Value::Null;
        };
        let g = self.graph.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(ex) = g.as_ref().and_then(|g| g.explain(&word)) else {
            return Value::Null;
        };
        if ex.node.name != word {
            return Value::Null;
        }
        let names = |links: &[kor_graph::Link]| {
            let mut v: Vec<String> = links
                .iter()
                .take(8)
                .map(|x| format!("`{}`", x.node.name))
                .collect();
            if links.len() > 8 {
                v.push("…".into());
            }
            v.join(", ")
        };
        let mut md = format!("**{}** `{}`", ex.node.kind.as_str(), ex.node.name);
        if let Some(f) = &ex.node.file {
            let line = ex.node.span.map_or(1, |s| s.0);
            let _ = write!(
                md,
                "\n\n{}",
                self.l.t_args(
                    "lsp-hover-defined",
                    &args!("file" => f.to_string_lossy().replace('\\', "/"), "line" => line)
                )
            );
        }
        if !ex.callers.is_empty() {
            let _ = write!(
                md,
                "\n\n{} ({}): {}",
                self.l.t("lsp-hover-callers"),
                ex.callers.len(),
                names(&ex.callers)
            );
        }
        if !ex.callees.is_empty() {
            let _ = write!(
                md,
                "\n\n{} ({}): {}",
                self.l.t("lsp-hover-callees"),
                ex.callees.len(),
                names(&ex.callees)
            );
        }
        if let Some(c) = &ex.community {
            let _ = write!(md, "\n\n{}: {}", self.l.t("lsp-hover-area"), c.label);
        }
        json!({ "contents": { "kind": "markdown", "value": md } })
    }

    fn code_actions(&self, p: &Value) -> Value {
        let uri = p.pointer("/textDocument/uri").cloned().unwrap_or_default();
        let mut out: Vec<Value> = p
            .pointer("/context/diagnostics")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|d| d["source"] == "kor")
            .map(|d| {
                let message = d["message"].as_str().unwrap_or_default();
                let first = message.lines().next().unwrap_or_default();
                let title = self
                    .l
                    .t_args("lsp-action-fix", &args!("message" => first.to_owned()));
                json!({
                    "title": title,
                    "kind": "quickfix",
                    "diagnostics": [d],
                    "command": { "title": title, "command": FIX, "arguments": [{
                        "uri": uri, "line": d.pointer("/range/start/line"), "rule": d["code"], "message": message,
                    }] },
                })
            })
            .collect();
        let title = self.l.t("lsp-action-explain");
        out.push(json!({
            "title": title,
            "kind": "refactor",
            "command": { "title": title, "command": EXPLAIN, "arguments": [{ "uri": uri, "range": p["range"] }] },
        }));
        Value::Array(out)
    }

    /// The buffer's text, or the file on disk when the editor has not opened it.
    fn text_of(&self, uri: &str) -> Option<String> {
        if let Some(t) = self.st().docs.get(uri) {
            return Some(t.clone());
        }
        std::fs::read_to_string(uri_to_path(uri)?).ok()
    }

    #[allow(clippy::too_many_lines)] // one dispatch: arguments, prompt, run, result
    async fn execute(&self, rpc: Arc<Rpc>, p: &Value) -> Result<Value, RpcError> {
        let cmd = p["command"].as_str().unwrap_or_default();
        let arg = p.pointer("/arguments/0").cloned().unwrap_or(Value::Null);
        let bad = || RpcError::new(INVALID_PARAMS, format!("{cmd}: bad arguments"));
        let file = |uri: &str| {
            uri_to_path(uri)
                .map(|p| self.rel(&p).to_string_lossy().replace('\\', "/"))
                .ok_or_else(bad)
        };
        let (prompt, write) = match cmd {
            FIX => {
                let uri = arg["uri"].as_str().ok_or_else(bad)?;
                let line = arg["line"].as_u64().ok_or_else(bad)? + 1;
                let message = arg["message"].as_str().unwrap_or_default();
                (
                    format!(
                        "Fix this problem Kor's checks found in {}:{line} (rule `{}`):\n{message}\n\nChange only what the fix needs, keep the file's style, then say in one or two sentences what you changed.",
                        file(uri)?,
                        arg["rule"].as_str().unwrap_or_default()
                    ),
                    true,
                )
            }
            EXPLAIN => {
                let uri = arg["uri"].as_str().ok_or_else(bad)?;
                let text = self.text_of(uri).ok_or_else(bad)?;
                let start = arg
                    .pointer("/range/start/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let end = arg
                    .pointer("/range/end/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(start);
                let (start, end) = (
                    usize::try_from(start).map_err(|_| bad())?,
                    usize::try_from(end).map_err(|_| bad())?,
                );
                let code: Vec<&str> = text
                    .lines()
                    .skip(start)
                    .take(end.saturating_sub(start) + 1)
                    .take(400)
                    .collect();
                (
                    format!(
                        "Explain this code from {} (lines {}-{}) to a developer working on the project: what it does, how it fits the code around it, and anything risky. Do not change files.\n\n```\n{}\n```",
                        file(uri)?,
                        start + 1,
                        end + 1,
                        code.join("\n")
                    ),
                    false,
                )
            }
            RUN_TASK => {
                let task = arg
                    .as_str()
                    .or_else(|| arg["task"].as_str())
                    .filter(|t| !t.trim().is_empty() && t.len() <= crate::triggers::MAX_TASK)
                    .ok_or_else(bad)?;
                (task.to_owned(), true)
            }
            _ => {
                return Err(RpcError::new(
                    METHOD_NOT_FOUND,
                    format!("unknown command {cmd}"),
                ));
            }
        };
        let root = self.root()?;
        rpc.notify(
            "window/showMessage",
            json!({ "type": 3, "message": self.l.t_args("lsp-working", &args!("task" => clip(&prompt, 80))) }),
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let log = rpc.clone();
        let fwd = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let AgentEvent::ToolStart { name, summary, .. } = ev {
                    log.notify(
                        "window/logMessage",
                        json!({ "type": 4, "message": format!("kor: {name} {summary}") }),
                    );
                }
            }
        });
        let editor = EditorApprover {
            rpc: rpc.clone(),
            l: Localizer::new(self.l.lang()),
        };
        let approver: &dyn Approver = if write { &editor } else { &ReadOnly };
        let id = crate::triggers::notify::random_hex(6)
            .map_err(|e| RpcError::new(rpc::INTERNAL_ERROR, e))?;
        let res = runner::one_shot(
            &self.l,
            self.factory.as_ref(),
            &root,
            &runner::adhoc("lsp", "", "", Approval::Notify),
            &format!("lsp-{id}"),
            Vec::new(),
            "",
            prompt,
            approver,
            &tx,
            CancellationToken::new(),
        )
        .await;
        drop(tx);
        let _ = fwd.await;
        match res {
            Ok((text, _)) => {
                rpc.notify(
                    "window/showMessage",
                    json!({ "type": 3, "message": clip(&text, 2000) }),
                );
                Ok(json!({ "text": text }))
            }
            Err(e) => {
                rpc.notify("window/showMessage", json!({ "type": 1, "message": e }));
                Err(RpcError::new(rpc::INTERNAL_ERROR, e))
            }
        }
    }
}

#[async_trait]
impl Handler for Server {
    async fn request(&self, rpc: Arc<Rpc>, method: &str, p: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(self.initialize(&p)),
            "shutdown" => Ok(Value::Null),
            "textDocument/hover" => Ok(self.hover(&p)),
            "textDocument/codeAction" => Ok(self.code_actions(&p)),
            "workspace/executeCommand" => self.execute(rpc, &p).await,
            _ => Err(RpcError::new(METHOD_NOT_FOUND, method.to_owned())),
        }
    }

    async fn notification(&self, rpc: Arc<Rpc>, method: &str, p: Value) {
        let uri = p
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match method {
            "initialized" => self.load_graph(),
            "textDocument/didOpen" => {
                let text = p
                    .pointer("/textDocument/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.publish(&rpc, &uri, Some(&text));
                self.st().docs.insert(uri, text);
            }
            "textDocument/didChange" => {
                // Full sync: the last change holds the whole text.
                let Some(text) = p
                    .pointer("/contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|c| c.last())
                    .and_then(|c| c["text"].as_str())
                    .map(str::to_owned)
                else {
                    return;
                };
                self.publish(&rpc, &uri, Some(&text));
                self.st().docs.insert(uri, text);
            }
            "textDocument/didSave" => {
                let text = self.text_of(&uri);
                self.publish(&rpc, &uri, text.as_deref());
            }
            "textDocument/didClose" => {
                self.st().docs.remove(&uri);
                self.publish(&rpc, &uri, None);
            }
            "exit" => self.exit.notify_one(),
            _ => {}
        }
    }
}

/// Asks each permission request in the editor (`window/showMessageRequest`); no answer = no.
struct EditorApprover {
    rpc: Arc<Rpc>,
    l: Localizer,
}

#[async_trait]
impl Approver for EditorApprover {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        let (allow, deny) = (self.l.t("lsp-allow"), self.l.t("lsp-deny"));
        let mut message = self.l.t_args(
            "lsp-ask",
            &args!("action" => clip(&format!("{}: {}", req.tool, req.summary), 300)),
        );
        if let Some(why) = &req.destructive {
            let _ = write!(message, " ({why})");
        }
        let answer = self
            .rpc
            .request(
                "window/showMessageRequest",
                json!({ "type": if req.destructive.is_some() { 2 } else { 3 }, "message": message, "actions": [{ "title": allow }, { "title": deny }] }),
            )
            .await;
        if matches!(answer, Ok(v) if v["title"].as_str() == Some(allow.as_str())) {
            PermissionReply::Allow
        } else {
            PermissionReply::Deny
        }
    }

    fn surface(&self) -> &'static str {
        "editor"
    }
}

/// `kor.explain` never changes files.
struct ReadOnly;

#[async_trait]
impl Approver for ReadOnly {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }

    fn surface(&self) -> &'static str {
        "editor"
    }
}

fn clip(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// `file:///C:/a%20b/x.rs` → `C:/a b/x.rs` (`/home/x.rs` elsewhere); `None` for other schemes.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://host/share/x` (UNC) keeps the host; `file:///x` has an empty one.
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    let drive = |p: &str| p.len() >= 3 && p.as_bytes()[2] == b':' && p.starts_with('/');
    let s = if drive(&s) {
        s[1..].to_owned()
    } else if !s.starts_with('/') {
        format!("//{s}")
    } else {
        s
    };
    Some(PathBuf::from(s))
}

/// The identifier under a 0-based line / UTF-16 character position.
fn word_at(text: &str, line: u64, ch: u64) -> Option<String> {
    let l = text.lines().nth(usize::try_from(line).ok()?)?;
    let chars: Vec<char> = l.chars().collect();
    let mut units = 0u64;
    let mut at = chars.len();
    for (i, c) in chars.iter().enumerate() {
        if units + c.len_utf16() as u64 > ch {
            at = i;
            break;
        }
        units += c.len_utf16() as u64;
    }
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    if at >= chars.len() || !word(&chars[at]) {
        return None;
    }
    let start = chars[..at]
        .iter()
        .rposition(|c| !word(c))
        .map_or(0, |p| p + 1);
    let end = chars[at..]
        .iter()
        .position(|c| !word(c))
        .map_or(chars.len(), |p| at + p);
    Some(chars[start..end].iter().collect())
}

/// Serve one editor over `reader`/`writer` until it sends `exit` or closes the stream.
pub async fn serve<R, W>(server: Arc<Server>, reader: R, writer: W)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (_rpc, run) = rpc::connect_framed(reader, writer, server.clone(), Framing::Headers);
    tokio::select! {
        () = run => {}
        () = server.exit.notified() => {}
    }
}

/// `kor lsp`: serve the editor on this process's stdin/stdout with the normal agent runtime.
pub async fn serve_stdio(l: Localizer) {
    let factory = Arc::new(runner::Prepare { lang: l.lang() });
    serve(
        Arc::new(Server::new(l, factory)),
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_become_paths() {
        assert_eq!(
            uri_to_path("file:///c%3A/a%20b/x.rs").unwrap(),
            PathBuf::from("c:/a b/x.rs")
        );
        assert_eq!(
            uri_to_path("file:///home/me/x.rs").unwrap(),
            PathBuf::from("/home/me/x.rs")
        );
        assert!(uri_to_path("untitled:Untitled-1").is_none());
    }

    #[test]
    fn word_under_cursor() {
        let t = "fn main() {\n    let ğx = login(user);\n}";
        assert_eq!(word_at(t, 1, 14).as_deref(), Some("login"));
        assert_eq!(word_at(t, 1, 9).as_deref(), Some("ğx"));
        assert_eq!(word_at(t, 1, 18), None);
        assert_eq!(word_at(t, 9, 0), None);
    }
}
