//! Subscription OAuth logins ported from CLIProxyAPI `internal/auth` and `sdk/auth`.
//!
//! Each provider uses the OAuth client identity of the official app (Claude Code, Codex,
//! Antigravity, Kimi, Devin, GitHub Copilot): client id, scopes, redirect, and the request body
//! the token endpoint expects. [`begin`] returns the URL or device code the account owner must
//! approve; [`Pending::finish`] exchanges it and writes a CLIProxyAPI-compatible JSON auth file.
//!
//! Claude Code listens on `127.0.0.1:54545`, Codex on `127.0.0.1:1455`, Antigravity on
//! `127.0.0.1:51121`, because those redirect URIs are fixed by the official clients. Devin binds
//! an ephemeral port and puts it in the authorize URL. Claude token calls use the Claude Code
//! 2.1.220 OAuth ClientHello, so the control plane sees that client's TLS fingerprint. Saved
//! Claude accounts are loaded into the chat pool.

mod antigravity;
mod claude;
mod codex;
mod copilot;
mod devin;
mod fingerprint;
mod kimi;
mod session;

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Which official app identity the login uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    CodexDevice,
    Antigravity,
    Kimi,
    KimiAi,
    Devin,
    Copilot,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::CodexDevice => "codex-device",
            Self::Antigravity => "antigravity",
            Self::Kimi => "kimi",
            Self::KimiAi => "kimi-ai",
            Self::Devin => "devin",
            Self::Copilot => "copilot",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "codex-device" => Some(Self::CodexDevice),
            "antigravity" => Some(Self::Antigravity),
            "kimi" | "kimi.com" => Some(Self::Kimi),
            "kimi-ai" | "kimi.ai" => Some(Self::KimiAi),
            "devin" => Some(Self::Devin),
            "copilot" | "github-copilot" => Some(Self::Copilot),
            _ => None,
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the account owner has to do before [`Pending::finish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    Browser {
        url: String,
    },
    Device {
        user_code: String,
        verification_uri: String,
        verification_uri_complete: Option<String>,
    },
}

impl Prompt {
    pub fn instructions(&self) -> String {
        match self {
            Self::Browser { url } => format!("Open this URL to sign in:\n{url}"),
            Self::Device {
                user_code,
                verification_uri,
                verification_uri_complete,
            } => {
                let link = verification_uri_complete
                    .as_deref()
                    .unwrap_or(verification_uri);
                format!("Open {link}\nand enter code {user_code}")
            }
        }
    }

    fn open_target(&self) -> &str {
        match self {
            Self::Browser { url } => url,
            Self::Device {
                verification_uri,
                verification_uri_complete,
                ..
            } => verification_uri_complete
                .as_deref()
                .unwrap_or(verification_uri),
        }
    }
}

/// Where the login writes the auth file, and how long it waits.
#[derive(Debug, Clone)]
pub struct Options {
    pub auth_dir: PathBuf,
    /// Open the system browser on the authorize or device URL.
    pub open_browser: bool,
    /// Replace every OAuth HTTP origin. Production logins leave this empty; tests point it at a local server.
    pub http_origin: Option<String>,
    /// How long to wait for the browser or the device approval. Zero means 15 minutes.
    pub callback_timeout: Duration,
    /// Minimum gap between device-code polls when the server asks for less (Kimi) or omits it.
    /// Zero means 5 seconds, which is the upstream default.
    pub poll_floor: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auth_dir: default_auth_dir(),
            open_browser: true,
            http_origin: None,
            callback_timeout: Duration::from_secs(15 * 60),
            poll_floor: Duration::from_secs(5),
        }
    }
}

pub(crate) fn default_auth_dir() -> PathBuf {
    kor_config::user_dir().map_or_else(|| PathBuf::from("proxy-auth"), |dir| dir.join("proxy-auth"))
}

/// Auth file written by a finished login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub provider: Provider,
    pub path: PathBuf,
    pub file_name: String,
    pub label: String,
}

/// Login failed before an auth file was written.
#[derive(Debug)]
pub struct Error {
    message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

fn err(message: impl Into<String>) -> Error {
    Error {
        message: message.into(),
    }
}

/// A login that has shown the user a URL or device code and is waiting for approval.
#[must_use = "call finish to exchange the login and write the auth file"]
pub struct Pending {
    pub prompt: Prompt,
    inner: Inner,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("prompt", &self.prompt)
            .finish_non_exhaustive()
    }
}

enum Inner {
    Callback {
        net: Net,
        listener: TcpListener,
        state: String,
        verifier: String,
        timeout: Duration,
        auth_dir: PathBuf,
        kind: CallbackKind,
    },
    Device {
        net: Net,
        auth_dir: PathBuf,
        timeout: Duration,
        kind: DeviceKind,
    },
}

enum CallbackKind {
    Claude,
    Codex,
    Antigravity { redirect: String },
    Devin,
}

enum DeviceKind {
    Codex {
        device_auth_id: String,
        user_code: String,
        interval: Duration,
    },
    Kimi {
        ai: bool,
        device_code: String,
        device_id: String,
        interval: Duration,
    },
    Copilot {
        device_code: String,
        interval: Duration,
    },
}

struct Record {
    provider: Provider,
    file_name: String,
    label: String,
    body: Value,
    pretty: bool,
}

#[derive(Clone)]
struct Net {
    client: reqwest::Client,
    origin: Option<String>,
    poll_floor: Duration,
    timeout: Duration,
}

impl Net {
    fn new(opts: &Options) -> Result<Self, Error> {
        Ok(Self {
            client: http_client()?,
            origin: opts.http_origin.clone(),
            poll_floor: if opts.poll_floor.is_zero() {
                Duration::from_secs(5)
            } else {
                opts.poll_floor
            },
            timeout: if opts.callback_timeout.is_zero() {
                Duration::from_secs(15 * 60)
            } else {
                opts.callback_timeout
            },
        })
    }

    fn endpoint(&self, production: &str) -> String {
        let Some(origin) = &self.origin else {
            return production.to_owned();
        };
        let path = production
            .split_once("://")
            .and_then(|(_, rest)| rest.find('/').map(|index| &rest[index..]))
            .unwrap_or("/");
        format!("{}{path}", origin.trim_end_matches('/'))
    }

    async fn post_json(
        &self,
        url: &str,
        body: &impl Serialize,
        headers: &[(&str, &str)],
    ) -> Result<(u16, String), Error> {
        let payload = serde_json::to_string(body).map_err(|e| err(e.to_string()))?;
        let mut req = self
            .client
            .post(self.endpoint(url))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(payload);
        for (key, value) in headers {
            req = req.header(*key, *value);
        }
        send(req).await
    }

    async fn post_form(
        &self,
        url: &str,
        form_body: &str,
        headers: &[(&str, &str)],
    ) -> Result<(u16, String), Error> {
        let mut req = self
            .client
            .post(self.endpoint(url))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(form_body.to_owned());
        for (key, value) in headers {
            req = req.header(*key, *value);
        }
        send(req).await
    }

    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), Error> {
        let mut req = self
            .client
            .get(self.endpoint(url))
            .header("accept", "application/json");
        for (key, value) in headers {
            req = req.header(*key, *value);
        }
        send(req).await
    }
}

fn http_client() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| err(format!("http client: {e}")))
}

async fn send(req: reqwest::RequestBuilder) -> Result<(u16, String), Error> {
    let response = req
        .send()
        .await
        .map_err(|e| err(format!("oauth request: {e}")))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| err(format!("oauth response: {e}")))?;
    Ok((status, body))
}

fn json_ok(status: u16, body: &str) -> Result<Value, Error> {
    if !(200..300).contains(&status) {
        return Err(http_error(status, body));
    }
    serde_json::from_str(body)
        .map_err(|_| err(format!("oauth response was not JSON (HTTP {status})")))
}

fn http_error(status: u16, body: &str) -> Error {
    let snippet: String = body.chars().filter(|c| !c.is_control()).take(180).collect();
    err(format!("oauth request failed (HTTP {status}): {snippet}"))
}

/// Start a login. The returned [`Prompt`] is what the account owner must approve.
pub async fn begin(provider: Provider, opts: &Options) -> Result<Pending, Error> {
    match provider {
        Provider::Claude => claude_begin(opts).await,
        Provider::Codex => codex_begin(opts).await,
        Provider::CodexDevice => codex_device_begin(opts).await,
        Provider::Antigravity => antigravity_begin(opts).await,
        Provider::Kimi => kimi_begin(opts, false).await,
        Provider::KimiAi => kimi_begin(opts, true).await,
        Provider::Devin => devin_begin(opts).await,
        Provider::Copilot => copilot_begin(opts).await,
    }
}

async fn claude_begin(opts: &Options) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let codes = pkce(96)?;
    let state = oauth_state()?;
    let (listener, _) = listen(claude::PORT).await?;
    let url = claude::authorize_url(&codes.challenge, &state);
    Ok(browser_pending(
        opts,
        net,
        listener,
        state,
        codes.verifier,
        url,
        CallbackKind::Claude,
    ))
}

async fn codex_begin(opts: &Options) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let codes = pkce(96)?;
    let state = oauth_state()?;
    let (listener, _) = listen(codex::PORT).await?;
    let url = codex::authorize_url(&codes.challenge, &state);
    Ok(browser_pending(
        opts,
        net,
        listener,
        state,
        codes.verifier,
        url,
        CallbackKind::Codex,
    ))
}

async fn antigravity_begin(opts: &Options) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let state = oauth_state()?;
    let (listener, port) = listen(antigravity::PORT).await?;
    let redirect = format!("http://localhost:{port}/oauth-callback");
    let url = antigravity::authorize_url(&state, &redirect);
    Ok(browser_pending(
        opts,
        net,
        listener,
        state,
        String::new(),
        url,
        CallbackKind::Antigravity { redirect },
    ))
}

async fn devin_begin(opts: &Options) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let codes = pkce(64)?;
    let state = oauth_state()?;
    let (listener, port) = listen(0).await?;
    let redirect = format!("http://localhost:{port}/callback");
    let url = devin::authorize_url(&redirect, &codes.challenge, &state);
    Ok(browser_pending(
        opts,
        net,
        listener,
        state,
        codes.verifier,
        url,
        CallbackKind::Devin,
    ))
}

async fn codex_device_begin(opts: &Options) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let started = codex::start_device(&net).await?;
    let prompt = Prompt::Device {
        user_code: started.user_code.clone(),
        verification_uri: codex::DEVICE_VERIFY.to_owned(),
        verification_uri_complete: None,
    };
    Ok(device_pending(
        opts,
        net,
        prompt,
        DeviceKind::Codex {
            device_auth_id: started.device_auth_id,
            user_code: started.user_code,
            interval: started.interval,
        },
    ))
}

async fn kimi_begin(opts: &Options, ai: bool) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let started = kimi::start_device(&net, ai).await?;
    let prompt = Prompt::Device {
        user_code: started.user_code,
        verification_uri: started.verification_uri,
        verification_uri_complete: started.verification_uri_complete,
    };
    Ok(device_pending(
        opts,
        net,
        prompt,
        DeviceKind::Kimi {
            ai,
            device_code: started.device_code,
            device_id: started.device_id,
            interval: started.interval,
        },
    ))
}

async fn copilot_begin(opts: &Options) -> Result<Pending, Error> {
    let net = Net::new(opts)?;
    let started = copilot::start_device(&net).await?;
    let prompt = Prompt::Device {
        user_code: started.user_code,
        verification_uri: started.verification_uri,
        verification_uri_complete: started.verification_uri_complete,
    };
    Ok(device_pending(
        opts,
        net,
        prompt,
        DeviceKind::Copilot {
            device_code: started.device_code,
            interval: started.interval,
        },
    ))
}

fn browser_pending(
    opts: &Options,
    net: Net,
    listener: TcpListener,
    state: String,
    verifier: String,
    url: String,
    kind: CallbackKind,
) -> Pending {
    let prompt = Prompt::Browser { url };
    maybe_open(opts, prompt.open_target());
    let timeout = net.timeout;
    Pending {
        prompt,
        inner: Inner::Callback {
            net,
            listener,
            state,
            verifier,
            timeout,
            auth_dir: opts.auth_dir.clone(),
            kind,
        },
    }
}

fn device_pending(opts: &Options, net: Net, prompt: Prompt, kind: DeviceKind) -> Pending {
    maybe_open(opts, prompt.open_target());
    let timeout = net.timeout;
    Pending {
        prompt,
        inner: Inner::Device {
            net,
            auth_dir: opts.auth_dir.clone(),
            timeout,
            kind,
        },
    }
}

fn maybe_open(opts: &Options, url: &str) {
    if opts.open_browser {
        let _ = open_browser(url);
    }
}

impl Pending {
    /// Wait for approval, exchange the code, and write the auth file.
    pub async fn finish(self) -> Result<Saved, Error> {
        match self.inner {
            Inner::Callback {
                net,
                listener,
                state,
                verifier,
                timeout,
                auth_dir,
                kind,
            } => {
                let code = wait_code(listener, &state, timeout).await?;
                let record = match kind {
                    CallbackKind::Claude => {
                        claude::exchange(&net, &code, &state, &verifier).await?
                    }
                    CallbackKind::Codex => {
                        codex::exchange(&net, &code, &verifier, codex::REDIRECT).await?
                    }
                    CallbackKind::Antigravity { redirect } => {
                        antigravity::exchange(&net, &code, &redirect).await?
                    }
                    CallbackKind::Devin => devin::exchange(&net, &code, &verifier).await?,
                };
                commit(&auth_dir, record)
            }
            Inner::Device {
                net,
                auth_dir,
                timeout,
                kind,
            } => {
                let record = match kind {
                    DeviceKind::Codex {
                        device_auth_id,
                        user_code,
                        interval,
                    } => {
                        codex::finish_device(&net, &device_auth_id, &user_code, interval, timeout)
                            .await?
                    }
                    DeviceKind::Kimi {
                        ai,
                        device_code,
                        device_id,
                        interval,
                    } => {
                        kimi::finish_device(&net, ai, &device_code, &device_id, interval, timeout)
                            .await?
                    }
                    DeviceKind::Copilot {
                        device_code,
                        interval,
                    } => copilot::finish_device(&net, &device_code, interval, timeout).await?,
                };
                commit(&auth_dir, record)
            }
        }
    }
}

/// Refresh the access token in an auth file written by this module (or CLIProxyAPI).
pub(crate) use session::load_subscriptions;

pub async fn refresh_file(path: &Path, opts: &Options) -> Result<Saved, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| err(format!("read auth file: {e}")))?;
    let mut doc: Value =
        serde_json::from_str(&text).map_err(|e| err(format!("auth file is not JSON: {e}")))?;
    let net = Net::new(opts)?;
    let provider = match doc.get("type").and_then(Value::as_str).unwrap_or("") {
        "claude" => {
            claude::refresh(&net, &mut doc).await?;
            Provider::Claude
        }
        "codex" => {
            codex::refresh(&net, &mut doc).await?;
            Provider::Codex
        }
        "antigravity" => {
            antigravity::refresh(&net, &mut doc).await?;
            Provider::Antigravity
        }
        "kimi" => {
            kimi::refresh(&net, &mut doc).await?;
            Provider::Kimi
        }
        "kimi-ai" => {
            kimi::refresh(&net, &mut doc).await?;
            Provider::KimiAi
        }
        "github-copilot" => {
            copilot::refresh(&net, &mut doc).await?;
            Provider::Copilot
        }
        "devin" => return Err(err("devin session tokens are not refreshed")),
        other => return Err(err(format!("unsupported auth file type `{other}`"))),
    };
    write_json(path, &doc)?;
    let label = doc
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(provider.as_str())
        .to_owned();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_owned();
    Ok(Saved {
        provider,
        path: path.to_owned(),
        file_name,
        label,
    })
}

fn commit(dir: &Path, record: Record) -> Result<Saved, Error> {
    let path = save(dir, &record)?;
    Ok(Saved {
        provider: record.provider,
        file_name: record.file_name.clone(),
        path,
        label: record.label,
    })
}

fn save(dir: &Path, record: &Record) -> Result<PathBuf, Error> {
    if record.file_name.contains('/')
        || record.file_name.contains('\\')
        || Path::new(&record.file_name).components().count() != 1
    {
        return Err(err(
            "refusing to write an auth file outside the auth directory",
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| err(format!("create auth dir: {e}")))?;
    restrict_dir(dir);
    let path = dir.join(&record.file_name);
    let text = if record.pretty {
        serde_json::to_string_pretty(&record.body)
    } else {
        serde_json::to_string(&record.body)
    }
    .map_err(|e| err(e.to_string()))?;
    std::fs::write(&path, format!("{text}\n")).map_err(|e| err(format!("write auth file: {e}")))?;
    restrict_file(&path);
    Ok(path)
}

fn write_json(path: &Path, doc: &Value) -> Result<(), Error> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| err(e.to_string()))?;
    std::fs::write(path, format!("{text}\n")).map_err(|e| err(format!("write auth file: {e}")))?;
    restrict_file(path);
    Ok(())
}

fn restrict_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn open_browser(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(windows) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(format!("start \"\" \"{url}\""));
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(url);
        cmd
    } else {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().map(|_| ())
}

async fn listen(port: u16) -> Result<(TcpListener, u16), Error> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
        if port == 0 {
            err(format!("bind oauth callback: {e}"))
        } else {
            err(format!("bind oauth callback on 127.0.0.1:{port}: {e}"))
        }
    })?;
    let bound = listener
        .local_addr()
        .map_err(|e| err(e.to_string()))?
        .port();
    Ok((listener, bound))
}

async fn wait_code(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, Error> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(err("timed out waiting for the browser to finish login"));
        }
        let (mut sock, _) = tokio::time::timeout(remaining, listener.accept())
            .await
            .map_err(|_| err("timed out waiting for the browser to finish login"))?
            .map_err(|e| err(e.to_string()))?;
        let request = read_http(&mut sock).await?;
        let Some(query) = request_query(&request) else {
            reply(&mut sock, 404, "Not Found", "not found\n").await;
            continue;
        };
        if let Some(reason) = query_get(&query, "error") {
            reply(&mut sock, 400, "Bad Request", "login was not completed\n").await;
            return Err(err(format!("oauth login was rejected ({reason})")));
        }
        let Some(code) = query_get(&query, "code") else {
            reply(&mut sock, 404, "Not Found", "not found\n").await;
            continue;
        };
        let state = query_get(&query, "state").unwrap_or_default();
        if state != expected_state {
            reply(&mut sock, 400, "Bad Request", "state mismatch\n").await;
            return Err(err("oauth state mismatch"));
        }
        reply(
            &mut sock,
            200,
            "OK",
            "Kor login complete. You can close this tab.\n",
        )
        .await;
        return Ok(code);
    }
}

async fn read_http(sock: &mut TcpStream) -> Result<String, Error> {
    let mut buf = vec![0u8; 8192];
    let read = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf))
        .await
        .map_err(|_| err("oauth callback read timed out"))?
        .map_err(|e| err(e.to_string()))?;
    if read == 0 {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&buf[..read]).into_owned())
}

async fn reply(sock: &mut TcpStream, code: u16, reason: &str, body: &str) {
    let message = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(message.as_bytes()).await;
}

fn request_query(request: &str) -> Option<String> {
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let query = target.split_once('?')?.1;
    let query = query.split_once('#').map_or(query, |(head, _)| head);
    Some(query.to_owned())
}

fn query_get(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| pct_decode(value))
    })
}

fn pct_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &input[index + 1..index + 3];
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Pkce {
    verifier: String,
    challenge: String,
}

fn pkce(nbytes: usize) -> Result<Pkce, Error> {
    let verifier = URL_SAFE_NO_PAD.encode(rng(nbytes)?);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
    })
}

fn oauth_state() -> Result<String, Error> {
    Ok(URL_SAFE_NO_PAD.encode(rng(32)?))
}

fn rng(n: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| err(format!("random: {e}")))?;
    Ok(buf)
}

fn uuid_v4() -> Result<String, Error> {
    let mut bytes = rng(16)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn sha256_hex(data: &str) -> String {
    hex_encode(&Sha256::digest(data.as_bytes()))
}

fn hex_prefix8(data: &str) -> String {
    sha256_hex(data)[..8].to_owned()
}

fn b64_url(data: &str) -> Result<Vec<u8>, Error> {
    URL_SAFE_NO_PAD
        .decode(data)
        .or_else(|_| {
            let mut padded = data.to_owned();
            match data.len() % 4 {
                2 => padded.push_str("=="),
                3 => padded.push('='),
                _ => {}
            }
            URL_SAFE.decode(padded)
        })
        .map_err(|e| err(format!("jwt: {e}")))
}

fn query_escape(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte))
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut pairs = pairs.to_vec();
    pairs.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
    pairs
        .into_iter()
        .map(|(key, value)| format!("{}={}", query_escape(key), query_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn query_raw(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", query_escape(key), query_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn rfc3339_after(secs: i64) -> String {
    let when = chrono::Utc::now() + chrono::Duration::seconds(secs);
    when.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn rfc3339_now() -> String {
    rfc3339_after(0)
}

fn unix_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn json_i64(value: &Value, key: &str) -> i64 {
    match value.get(key) {
        Some(Value::Number(number)) => number
            .as_i64()
            .unwrap_or_else(|| number.as_f64().map_or(0, trunc_i64)),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn trunc_i64(value: f64) -> i64 {
    if value.is_finite() {
        #[allow(clippy::cast_possible_truncation)]
        let whole = value.trunc() as i64;
        whole
    } else {
        0
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_round_trip() {
        for provider in [
            Provider::Claude,
            Provider::Codex,
            Provider::CodexDevice,
            Provider::Antigravity,
            Provider::Kimi,
            Provider::KimiAi,
            Provider::Devin,
            Provider::Copilot,
        ] {
            assert_eq!(Provider::parse(provider.as_str()), Some(provider));
        }
        assert_eq!(Provider::parse("claude-code"), Some(Provider::Claude));
        assert_eq!(Provider::parse("github-copilot"), Some(Provider::Copilot));
        assert_eq!(Provider::parse("nope"), None);
    }

    #[test]
    fn claude_authorize_url_matches_the_official_query() {
        let url = claude::authorize_url("challenge", "state");
        assert_eq!(
            url,
            "https://claude.ai/oauth/authorize?client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&code=true&code_challenge=challenge&code_challenge_method=S256&redirect_uri=http%3A%2F%2Flocalhost%3A54545%2Fcallback&response_type=code&scope=org%3Acreate_api_key+user%3Aprofile+user%3Ainference+user%3Asessions%3Aclaude_code+user%3Amcp_servers+user%3Afile_upload&state=state"
        );
    }

    #[tokio::test]
    async fn callback_returns_the_code_only_when_the_state_matches() {
        let (listener, port) = listen(0).await.unwrap();
        let wait =
            tokio::spawn(async move { wait_code(listener, "good", Duration::from_secs(2)).await });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(b"GET /callback?code=abc%2B1&state=good HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        assert_eq!(wait.await.unwrap().unwrap(), "abc+1");
    }

    #[tokio::test]
    async fn callback_rejects_a_different_state() {
        let (listener, port) = listen(0).await.unwrap();
        let wait =
            tokio::spawn(async move { wait_code(listener, "good", Duration::from_secs(2)).await });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(b"GET /callback?code=abc&state=bad HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let error = wait.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("state"));
    }
}
