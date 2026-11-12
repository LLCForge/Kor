//! End-to-end scenarios (spec §16.4, §17.2, ADR 0061): a deterministic mock provider, an isolated
//! Kor home for one run, and the checker every surface (CLI, TUI, desktop) is held to.
//!
//! A scenario (`tests/e2e/scenarios/*.yaml`) names the project files, the provider's scripted replies,
//! the user's steps and the expected outcome. Each surface drives the real core against [`MockServer`]
//! with `KOR_IPC_TRACE` on; [`check`] then compares the trace's core event stream
//! ([`kor_core::ipc_trace::normalize`]), the files and the cost ledger with the expectation, so the
//! three surfaces are compared with one implementation.

use kor_platform::NoWindow as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The model id the mock serves (`mock/kor-mock`).
pub const MODEL: &str = "kor-mock";
/// Provider id of the mock manifest.
pub const PROVIDER: &str = "mock";

/// One scenario file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub title: String,
    /// Surfaces it runs on (`cli`, `tui`, `desktop`); all three when empty.
    pub surfaces: Vec<String>,
    pub project: Project,
    /// Scripted provider replies, consumed in order (or by `match`).
    pub mock: Vec<Reply>,
    pub steps: Vec<Step>,
    pub expect: Expect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Project {
    /// Project-relative path → content.
    pub files: BTreeMap<String, String>,
    /// Trust the folder (project config may then grant permissions, MCP servers start).
    pub trust: bool,
    /// Extra `config.toml` lines for this scenario (`[budget]`, `[permissions]`…).
    pub config: String,
    /// `git init` and one commit of the files (multi-agent worktrees, undo).
    pub git: bool,
}

/// `${KOR}` in project files and command arguments: the `kor` binary under test.
pub const KOR_PLACEHOLDER: &str = "${KOR}";

/// One provider reply.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reply {
    /// Taken only when the request body contains this text; otherwise replies go in order.
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolCall>,
    pub usage: Tokens,
    /// An HTTP error instead of a completion (401 invalid key, 429 quota, 500…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Error body text for `status`.
    pub error: String,
    /// Close the connection without answering (network failure).
    pub disconnect: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tokens {
    pub prompt: u64,
    pub completion: u64,
}

/// One user step; exactly one field is set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Step {
    /// A message to the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Answer to the next approval card: `yes`, `no`, `always`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approve: Option<String>,
    /// A `parity.toml` feature name; each surface runs its own command for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments after the CLI command (`kor <command> <args…>`, TUI `/<command> <args…>`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Arguments of the desktop bridge method the parity manifest maps the command to.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub desktop_args: Vec<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Expect {
    /// The normalized core event stream: a type (`"confirm"`) or an object whose fields must match.
    pub events: Vec<Value>,
    /// Project-relative path → content after the run; `~` (null) = the file must not exist.
    pub files: BTreeMap<String, Option<String>>,
    /// Lines in the cost ledger (`usage.jsonl`) after the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_requests: Option<usize>,
    /// The run ends with an error the surface shows (provider down, invalid key, quota…).
    pub error: bool,
    /// Text the surface's own output must contain (`--json` lines, panel text…).
    pub output: Vec<String>,
    /// Models the provider was asked for, in order (provider / model switching).
    pub models: Vec<String>,
}

/// Errors loading or checking scenarios.
#[derive(Debug, thiserror::Error)]
pub enum E2eError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Parse(PathBuf, String),
    #[error("{0}: {1}")]
    Invalid(String, String),
}

impl Scenario {
    /// Parse and validate one scenario file.
    ///
    /// # Errors
    /// Unreadable file, invalid YAML, or a step that sets no field or several.
    pub fn load(path: &Path) -> Result<Self, E2eError> {
        let text = std::fs::read_to_string(path).map_err(|e| E2eError::Io(path.to_owned(), e))?;
        Self::parse(&text).map_err(|e| match e {
            E2eError::Parse(_, m) => E2eError::Parse(path.to_owned(), m),
            other => other,
        })
    }

    /// Parse and validate scenario YAML.
    ///
    /// # Errors
    /// Invalid YAML or an invalid scenario.
    pub fn parse(text: &str) -> Result<Self, E2eError> {
        let s: Self = serde_yaml::from_str(text)
            .map_err(|e| E2eError::Parse(PathBuf::new(), e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    /// Every `*.yaml` in `dir`, by file name.
    ///
    /// # Errors
    /// The first scenario that does not load.
    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, E2eError> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| E2eError::Io(dir.to_owned(), e))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
            .collect();
        files.sort();
        files.iter().map(|p| Self::load(p)).collect()
    }

    fn validate(&self) -> Result<(), E2eError> {
        let bad = |m: String| Err(E2eError::Invalid(self.name.clone(), m));
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return bad("name must be [a-z0-9-]".into());
        }
        for (i, st) in self.steps.iter().enumerate() {
            let set = [
                st.prompt.is_some(),
                st.approve.is_some(),
                st.command.is_some(),
            ]
            .iter()
            .filter(|b| **b)
            .count();
            if set != 1 {
                return bad(format!(
                    "step {i}: exactly one of prompt / approve / command"
                ));
            }
            if let Some(a) = &st.approve
                && !matches!(a.as_str(), "yes" | "no" | "always")
            {
                return bad(format!("step {i}: approve is yes, no or always"));
            }
        }
        for s in &self.surfaces {
            if !matches!(s.as_str(), "cli" | "tui" | "desktop") {
                return bad(format!("unknown surface {s}"));
            }
        }
        for p in self.project.files.keys().chain(self.expect.files.keys()) {
            if Path::new(p).is_absolute() || p.split(['/', '\\']).any(|c| c == "..") {
                return bad(format!("{p}: project-relative paths only"));
            }
        }
        Ok(())
    }

    /// Runs on `surface` (`cli`, `tui`, `desktop`).
    pub fn on(&self, surface: &str) -> bool {
        self.surfaces.is_empty() || self.surfaces.iter().any(|s| s == surface)
    }
}

// ---------------------------------------------------------------------------------------------
// Mock provider
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Script {
    replies: Vec<Reply>,
    used: Vec<bool>,
    requests: Vec<Value>,
}

/// An `OpenAI`-compatible chat server on loopback that answers from a scenario's script.
#[derive(Debug, Clone)]
pub struct MockServer {
    /// `http://127.0.0.1:<port>/v1`
    pub url: String,
    script: Arc<Mutex<Script>>,
}

impl MockServer {
    /// Bind a free loopback port and serve on the current tokio runtime.
    ///
    /// # Errors
    /// When no port can be bound.
    pub async fn start(replies: Vec<Reply>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/v1", listener.local_addr()?);
        let used = vec![false; replies.len()];
        let script = Arc::new(Mutex::new(Script {
            replies,
            used,
            requests: Vec::new(),
        }));
        let s = Arc::clone(&script);
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let s = Arc::clone(&s);
                tokio::spawn(async move {
                    // A broken client connection only ends that connection.
                    let _ = serve(sock, &s).await;
                });
            }
        });
        Ok(Self { url, script })
    }

    /// Chat request bodies received so far.
    pub fn requests(&self) -> Vec<Value> {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }

    /// Script entries never served (a scenario that ends early).
    pub fn unused(&self) -> usize {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .used
            .iter()
            .filter(|u| !**u)
            .count()
    }
}

/// Upper bound of one request (headers + body); the agent's prompts are far smaller.
const MAX_REQUEST: usize = 16 * 1024 * 1024;

async fn serve(mut sock: TcpStream, script: &Mutex<Script>) -> std::io::Result<()> {
    loop {
        let Some((head, body)) = read_request(&mut sock).await? else {
            return Ok(());
        };
        let first = head.lines().next().unwrap_or_default().to_owned();
        if first.starts_with("GET ") && first.contains("/models") {
            let b = json!({ "object": "list", "data": [{ "id": MODEL, "object": "model", "owned_by": PROVIDER }] }).to_string();
            write_response(&mut sock, 200, "application/json", b.as_bytes()).await?;
            continue;
        }
        if !(first.starts_with("POST ") && first.contains("/chat/completions")) {
            write_response(
                &mut sock,
                404,
                "application/json",
                br#"{"error":{"message":"not found"}}"#,
            )
            .await?;
            continue;
        }
        let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let reply = pick(script, &req, &String::from_utf8_lossy(&body));
        let Some(reply) = reply else {
            let b = json!({ "error": { "message": "kor mock: script exhausted", "type": "server_error" } }).to_string();
            write_response(&mut sock, 500, "application/json", b.as_bytes()).await?;
            continue;
        };
        if reply.disconnect {
            return Ok(());
        }
        if let Some(status) = reply.status {
            let b = json!({ "error": { "message": reply.error, "type": "mock_error", "code": status } }).to_string();
            write_response(&mut sock, status, "application/json", b.as_bytes()).await?;
            continue;
        }
        if req["stream"] == true {
            let sse = sse_body(&reply);
            write_response(&mut sock, 200, "text/event-stream", sse.as_bytes()).await?;
        } else {
            write_response(
                &mut sock,
                200,
                "application/json",
                completion(&reply).to_string().as_bytes(),
            )
            .await?;
        }
    }
}

fn pick(script: &Mutex<Script>, req: &Value, raw: &str) -> Option<Reply> {
    let mut s = script.lock().unwrap_or_else(PoisonError::into_inner);
    s.requests.push(req.clone());
    let idx = (0..s.replies.len())
        .find(|&i| {
            !s.used[i]
                && s.replies[i]
                    .when
                    .as_deref()
                    .is_some_and(|m| raw.contains(m))
        })
        .or_else(|| (0..s.replies.len()).find(|&i| !s.used[i] && s.replies[i].when.is_none()))?;
    s.used[idx] = true;
    Some(s.replies[idx].clone())
}

async fn read_request(sock: &mut TcpStream) -> std::io::Result<Option<(String, Vec<u8>)>> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST {
            return Err(std::io::Error::other("request too large"));
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let len: usize = head
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0)
        .min(MAX_REQUEST);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Ok(Some((head, body)))
}

async fn write_response(
    sock: &mut TcpStream,
    status: u16,
    ctype: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        402 => "Payment Required",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: keep-alive\r\n\r\n",
        body.len()
    );
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(body).await?;
    sock.flush().await
}

fn usage_json(t: Tokens) -> Value {
    json!({ "prompt_tokens": t.prompt, "completion_tokens": t.completion, "total_tokens": t.prompt + t.completion })
}

fn tool_json(t: &ToolCall) -> Value {
    json!([{ "index": 0, "id": "call_kor_mock", "type": "function", "function": { "name": t.name, "arguments": t.input.to_string() } }])
}

/// Streamed reply: the text word by word, then the tool call, the finish reason and the usage.
fn sse_body(r: &Reply) -> String {
    let mut out = String::new();
    let mut push = |v: Value| {
        out.push_str("data: ");
        out.push_str(&v.to_string());
        out.push_str("\n\n");
    };
    let chunk = |delta: Value, finish: Value| json!({ "id": "chatcmpl-kor", "object": "chat.completion.chunk", "model": MODEL, "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] });
    push(chunk(
        json!({ "role": "assistant", "content": "" }),
        Value::Null,
    ));
    for piece in r.text.split_inclusive(' ') {
        push(chunk(json!({ "content": piece }), Value::Null));
    }
    if let Some(t) = &r.tool {
        push(chunk(json!({ "tool_calls": tool_json(t) }), Value::Null));
    }
    push(chunk(
        json!({}),
        json!(if r.tool.is_some() {
            "tool_calls"
        } else {
            "stop"
        }),
    ));
    push(
        json!({ "id": "chatcmpl-kor", "object": "chat.completion.chunk", "model": MODEL, "choices": [], "usage": usage_json(r.usage) }),
    );
    out.push_str("data: [DONE]\n\n");
    out
}

fn completion(r: &Reply) -> Value {
    let mut msg = json!({ "role": "assistant", "content": r.text });
    if let Some(t) = &r.tool {
        msg["tool_calls"] = tool_json(t);
    }
    json!({
        "id": "chatcmpl-kor", "object": "chat.completion", "model": MODEL,
        "choices": [{ "index": 0, "message": msg, "finish_reason": if r.tool.is_some() { "tool_calls" } else { "stop" } }],
        "usage": usage_json(r.usage),
    })
}

// ---------------------------------------------------------------------------------------------
// Isolated environment
// ---------------------------------------------------------------------------------------------

/// A Kor home, configuration and project that belong to one run only.
#[derive(Debug, Clone, Serialize)]
pub struct IsolatedEnv {
    pub root: PathBuf,
    pub project: PathBuf,
    pub home: PathBuf,
    pub trace: PathBuf,
    /// Variables to set on the surface's process.
    pub vars: BTreeMap<String, String>,
}

/// Create `root/{config,home,cache,project}`: the mock provider manifest, a config whose default model
/// is `mock/kor-mock` with privacy mode on (nothing leaves the machine), the scenario's project files and
/// trust. `surface` names the trace file (`trace-<surface>.ndjson`).
///
/// # Errors
/// When a file cannot be written.
pub fn isolated_env(
    root: &Path,
    scenario: &Scenario,
    base_url: &str,
    surface: &str,
) -> std::io::Result<IsolatedEnv> {
    isolated_env_with(root, scenario, base_url, surface, None)
}

/// [`isolated_env`] with the `kor` binary that replaces `${KOR}` in project files.
///
/// # Errors
/// When a file cannot be written or `git` fails for a `git: true` project.
pub fn isolated_env_with(
    root: &Path,
    scenario: &Scenario,
    base_url: &str,
    surface: &str,
    kor: Option<&Path>,
) -> std::io::Result<IsolatedEnv> {
    let dir = |n: &str| -> std::io::Result<PathBuf> {
        let p = root.join(n);
        std::fs::create_dir_all(&p)?;
        Ok(p)
    };
    let config = dir("config")?;
    let home = dir("home")?;
    let cache = dir("cache")?;
    let claude = dir("claude")?;
    let codex = dir("codex")?;
    // Windows' canonical form is `\\?\C:\…`; surfaces get the plain path (as a user's cwd would be).
    let project = dir("project")?.canonicalize()?;
    let project = project
        .to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .filter(|s| !s.starts_with("UNC"))
        .map_or(project.clone(), PathBuf::from);
    std::fs::create_dir_all(config.join("providers"))?;
    std::fs::write(
        config.join("providers").join(format!("{PROVIDER}.toml")),
        format!(
            "id = \"{PROVIDER}\"\nname = \"Kor mock (E2E)\"\ncategory = \"local\"\nadapter = \"openai_compat\"\n\
             base_url = \"{base_url}\"\nauth = {{ kind = \"none\" }}\nmodels_endpoint = \"/models\"\n\
             tool_calling = true\nverified = false\ndocs = \"https://kor.dev/docs/e2e\"\n\n\
             [privacy]\ntier = \"local\"\nretention = \"Never leaves this machine.\"\ntraining_on_data = false\n"
        ),
    )?;
    std::fs::write(
        config.join("config.toml"),
        format!(
            "[agent]\nmodel = \"{PROVIDER}/{MODEL}\"\nverify = \"off\"\n\n[intent]\nenabled = false\n\n[privacy]\nmode = true\n\n[update]\nauto = false\n\n{}\n",
            scenario.project.config
        ),
    )?;
    let kor_path = kor.map_or_else(
        || "kor".to_owned(),
        |k| k.display().to_string().replace('\\', "/"),
    );
    for (rel, content) in &scenario.project.files {
        let p = project.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, content.replace(KOR_PLACEHOLDER, &kor_path))?;
    }
    if scenario.project.git {
        git(&project, &["init", "-q", "-b", "main"])?;
        git(&project, &["add", "-A"])?;
        git(
            &project,
            &[
                "-c",
                "user.name=Kor E2E",
                "-c",
                "user.email=e2e@kor.invalid",
                "commit",
                "-q",
                "-m",
                "scenario",
            ],
        )?;
    }
    if scenario.project.trust {
        kor_compat::trust::set(&home.join("trusted.json"), &project, true)?;
    }
    let trace = root.join(format!("trace-{surface}.ndjson"));
    let _ = std::fs::remove_file(&trace);
    let s = |p: &Path| p.display().to_string();
    let vars = BTreeMap::from([
        ("KOR_CONFIG_DIR".to_owned(), s(&config)),
        ("KOR_HOME".to_owned(), s(&home)),
        ("KOR_CACHE_DIR".to_owned(), s(&cache)),
        ("CLAUDE_CONFIG_DIR".to_owned(), s(&claude)),
        ("CODEX_HOME".to_owned(), s(&codex)),
        (kor_core::ipc_trace::ENV.to_owned(), s(&trace)),
    ]);
    Ok(IsolatedEnv {
        root: root.to_owned(),
        project,
        home,
        trace,
        vars,
    })
}

fn git(dir: &Path, args: &[&str]) -> std::io::Result<()> {
    let st = std::process::Command::new("git")
        .no_window()
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()?;
    if st.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&st.stderr).trim()
        )))
    }
}

// ---------------------------------------------------------------------------------------------
// Checking a run
// ---------------------------------------------------------------------------------------------

/// What one surface produced for a scenario.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Outcome {
    pub surface: String,
    /// Normalized core event stream ([`kor_core::ipc_trace::normalize`]).
    pub events: Vec<Value>,
    pub problems: Vec<String>,
}

fn surface_of(s: &str) -> kor_core::ipc_trace::Surface {
    use kor_core::ipc_trace::Surface;
    match s {
        "tui" => Surface::Tui,
        "desktop" => Surface::Desktop,
        "core" => Surface::Core,
        _ => Surface::Cli,
    }
}

fn event_matches(want: &Value, got: &Value) -> bool {
    match want {
        Value::String(t) => got["type"] == *t,
        Value::Object(m) => m.iter().all(|(k, v)| got.get(k) == Some(v)),
        _ => false,
    }
}

/// Compare a finished run with the scenario: the traced core events, schema violations in the trace,
/// the project files, the cost ledger, the error and the surface's own output text.
pub fn check(scenario: &Scenario, env: &IsolatedEnv, surface: &str, output: &str) -> Outcome {
    check_with(scenario, env, surface, output, &[])
}

/// [`check`] plus the chat requests the mock received (for `expect.models`).
pub fn check_with(
    scenario: &Scenario,
    env: &IsolatedEnv,
    surface: &str,
    output: &str,
    requests: &[Value],
) -> Outcome {
    let mut problems = Vec::new();
    let records = kor_core::ipc_trace::read(&env.trace).unwrap_or_default();
    for (name, errs) in kor_core::ipc_trace::violations(&records) {
        problems.push(format!("schema: {name}: {}", errs.join("; ")));
    }
    let events = kor_core::ipc_trace::normalize(&records, surface_of(surface));
    let want = &scenario.expect.events;
    if !want.is_empty() {
        let ok = want.len() == events.len()
            && want.iter().zip(&events).all(|(w, g)| event_matches(w, g));
        if !ok {
            problems.push(format!(
                "events: want {} got {}",
                Value::Array(want.clone()),
                Value::Array(events.clone())
            ));
        }
    }
    let has_error = events.iter().any(|e| e["type"] == "error");
    if has_error != scenario.expect.error {
        problems.push(format!(
            "error: want {} got {has_error}",
            scenario.expect.error
        ));
    }
    for (rel, content) in &scenario.expect.files {
        // `branch:path` reads the file from a git ref (a task's result branch), else the work tree.
        let got = if rel.contains(':') {
            std::process::Command::new("git")
                .no_window()
                .args(["show", rel])
                .current_dir(&env.project)
                .stdin(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            std::fs::read_to_string(env.project.join(rel)).ok()
        };
        // Line endings are the platform's business (Windows checkouts, editors).
        let norm = |s: &str| s.replace("\r\n", "\n");
        if got.as_deref().map(norm) != content.as_deref().map(norm) {
            problems.push(format!("file {rel}: want {content:?} got {got:?}"));
        }
    }
    if let Some(n) = scenario.expect.usage_requests {
        let got = crate::usage::read(&env.home.join("usage.jsonl")).len();
        if got != n {
            problems.push(format!("usage.jsonl: want {n} requests got {got}"));
        }
    }
    if !scenario.expect.models.is_empty() {
        let got: Vec<String> = requests
            .iter()
            .filter_map(|r| r["model"].as_str().map(str::to_owned))
            .collect();
        if got != scenario.expect.models {
            problems.push(format!(
                "models: want {:?} got {got:?}",
                scenario.expect.models
            ));
        }
    }
    for text in &scenario.expect.output {
        if !output.contains(text.as_str()) {
            problems.push(format!("output lacks {text:?}"));
        }
    }
    Outcome {
        surface: surface.to_owned(),
        events,
        problems,
    }
}

/// The surfaces must agree: the same normalized core event stream everywhere.
pub fn compare(outcomes: &[Outcome]) -> Vec<String> {
    let Some(first) = outcomes.first() else {
        return Vec::new();
    };
    outcomes[1..]
        .iter()
        .filter_map(|o| {
            kor_core::ipc_trace::diff(&first.events, &o.events)
                .map(|d| format!("{} vs {}: {d}", first.surface, o.surface))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        serde_yaml::from_str(
            r#"
name: unit
title: unit
project:
  files: { "a.txt": "eski\n" }
  trust: true
mock:
  - text: "Yazıyorum."
    tool: { name: Write, input: { file_path: a.txt, content: "yeni\n" } }
    usage: { prompt: 10, completion: 5 }
  - match: "needle"
    text: "matched"
  - status: 401
    error: "invalid api key"
steps:
  - prompt: "a.txt güncelle"
  - approve: "yes"
expect:
  events: [status, { type: text, text: "Yazıyorum." }]
  files: { "a.txt": "yeni\n", "b.txt": ~ }
"#,
        )
        .unwrap()
    }

    #[test]
    fn scenarios_validate() {
        let s = scenario();
        s.validate().unwrap();
        assert!(s.on("tui"));
        let mut bad = s.clone();
        bad.steps.push(Step::default());
        assert!(bad.validate().is_err());
        let mut bad = s.clone();
        bad.expect.files.insert("../x".into(), None);
        assert!(bad.validate().is_err());
        let mut bad = s;
        bad.name = "a b".into();
        assert!(bad.validate().is_err());
    }

    #[tokio::test]
    async fn mock_serves_the_script_in_order_and_by_match() {
        let s = scenario();
        let mock = MockServer::start(s.mock.clone()).await.unwrap();
        let client = reqwest::Client::new();
        let post = |body: Value| {
            client
                .post(format!("{}/chat/completions", mock.url))
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
        };
        let r = post(json!({ "messages": [{ "role": "user", "content": "needle" }] }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "matched");
        let r = post(json!({ "stream": true, "messages": [] }))
            .await
            .unwrap();
        let text = r.text().await.unwrap();
        assert!(
            text.contains("\"tool_calls\"")
                && text.contains("Write")
                && text.ends_with("data: [DONE]\n\n"),
            "{text}"
        );
        assert!(text.contains("\"prompt_tokens\":10"), "{text}");
        let r = post(json!({ "messages": [] })).await.unwrap();
        assert_eq!(r.status(), 401);
        let r = post(json!({ "messages": [] })).await.unwrap();
        assert_eq!(r.status(), 500, "script exhausted");
        assert_eq!(mock.requests().len(), 4);
        assert_eq!(mock.unused(), 0);
        let models: Value = serde_json::from_str(
            &client
                .get(format!("{}/models", mock.url))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(models["data"][0]["id"], MODEL);
    }

    #[test]
    fn isolated_env_and_check() {
        let dir = tempfile::tempdir().unwrap();
        let s = scenario();
        let env = isolated_env(dir.path(), &s, "http://127.0.0.1:9/v1", "cli").unwrap();
        assert!(env.project.join("a.txt").exists());
        assert!(env.vars["KOR_CONFIG_DIR"].ends_with("config"));
        let manifest =
            std::fs::read_to_string(dir.path().join("config/providers/mock.toml")).unwrap();
        assert!(manifest.contains("base_url = \"http://127.0.0.1:9/v1\""));
        assert!(kor_compat::trust::is_trusted(
            &env.home.join("trusted.json"),
            &env.project
        ));
        // Nothing ran: no events, the file unchanged → problems are reported, not panics.
        let o = check(&s, &env, "cli", "");
        assert!(
            o.problems.iter().any(|p| p.starts_with("events")),
            "{:?}",
            o.problems
        );
        assert!(
            o.problems.iter().any(|p| p.starts_with("file a.txt")),
            "{:?}",
            o.problems
        );
        std::fs::write(env.project.join("a.txt"), "yeni\r\n").unwrap();
        std::fs::write(
            &env.trace,
            "{\"surface\":\"cli\",\"kind\":\"event\",\"name\":\"kor:event\",\"payload\":{\"type\":\"status\",\"verb\":\"v\",\"verbIndex\":0,\"elapsedMs\":1,\"tokens\":0,\"sessionId\":\"s\"}}\n\
             {\"surface\":\"cli\",\"kind\":\"event\",\"name\":\"kor:event\",\"payload\":{\"type\":\"text\",\"delta\":\"Yazıyorum.\",\"sessionId\":\"s\"}}\n",
        )
        .unwrap();
        let o = check(&s, &env, "cli", "");
        assert!(o.problems.is_empty(), "{:?}", o.problems);
        let other = Outcome {
            surface: "tui".into(),
            events: vec![json!({ "type": "status" })],
            problems: vec![],
        };
        assert_eq!(compare(&[o, other]).len(), 1);
    }

    #[test]
    fn checked_in_scenarios_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/e2e/scenarios");
        let all = Scenario::load_dir(&dir).unwrap();
        assert!(all.len() >= 12, "{}", all.len());
        let names: std::collections::BTreeSet<_> = all.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names.len(), all.len(), "unique names");
    }
}
