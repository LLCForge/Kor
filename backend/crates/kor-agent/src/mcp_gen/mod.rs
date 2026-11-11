//! `kor mcp add <source>` (spec §8.3): MCP servers generated from a database, an OpenAPI/Swagger
//! document, a GraphQL schema, or a local service folder (ADR 0038).
//!
//! A generated server is a [`Definition`] (tools + how to call them) saved as
//! `<config>/kor/mcp-gen/<id>.json`, and an ordinary stdio entry in Kor's `mcp.json` that runs
//! `kor mcp run <id>`. Every MCP client (Kor itself, Claude Code, Cursor) starts it the same way;
//! [`serve::serve`] answers it in process. Nothing secret is stored in either file: database
//! passwords go to the keychain (`db-<id>`, as `kor db`), API tokens are vault names.
//!
//! A local folder needs no definition: the detected command becomes a plain stdio entry.

pub mod db;
pub mod graphql;
pub mod kor_serve;
pub mod local;
pub mod openapi;
pub mod serve;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::mcp_config::{self, Locations};

/// More tools than this would flood any model's tool list; the rest are cut (and reported).
pub const MAX_TOOLS: usize = 300;
/// Characters of one tool result handed back to the client.
pub const MAX_RESPONSE: usize = 200_000;
/// Bytes of a fetched schema or HTTP response body that are read at all.
pub const MAX_BODY: usize = 10 * 1024 * 1024;
const DEFAULT_ROWS: usize = 200;
const MAX_ROWS: usize = 1000;
const DEFAULT_TIMEOUT: u64 = 30;

/// Looks up a secret by vault name (environment first, then the vault); never logged.
pub type Secrets = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Openapi,
    Graphql,
    Database,
    Local,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Openapi => "openapi",
            Self::Graphql => "graphql",
            Self::Database => "database",
            Self::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "openapi" | "swagger" => Some(Self::Openapi),
            "graphql" => Some(Self::Graphql),
            "database" | "db" => Some(Self::Database),
            "local" | "folder" => Some(Self::Local),
            _ => None,
        }
    }
}

/// Where an API credential goes. `secret` is a vault name (`kor vault add NAME`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Auth {
    /// `bearer`, `header` or `query`.
    pub place: String,
    /// Header or query parameter name (unused for `bearer`).
    #[serde(default)]
    pub name: String,
    pub secret: String,
}

impl Auth {
    /// `bearer:NAME`, `header:X-Api-Key:NAME`, `query:api_key:NAME`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        let (place, name, secret) = match parts.as_slice() {
            ["bearer", secret] => ("bearer", "", *secret),
            [p @ ("header" | "query"), name, secret] => (*p, *name, *secret),
            _ => {
                return Err(
                    "auth: bearer:SECRET, header:NAME:SECRET or query:NAME:SECRET (SECRET is a `kor vault` name)"
                        .into(),
                );
            }
        };
        let token = |t: &str| {
            (1..=100).contains(&t.len())
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        };
        if !kor_secrets::vault::valid_name(secret) || (place != "bearer" && !token(name)) {
            return Err(format!("auth: invalid name in `{s}`"));
        }
        Ok(Self {
            place: place.into(),
            name: name.into(),
            secret: secret.into(),
        })
    }
}

/// A database the server talks to. The password is never here (keychain `db-<id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbTarget {
    /// `postgres`, `mysql` or `sqlite`.
    pub engine: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    /// `SQLite` file (absolute).
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub sslmode: Option<String>,
}

/// How a generated tool is carried out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "via", rename_all = "camelCase")]
pub enum Plan {
    Http {
        method: String,
        path: String,
        /// `(name, in)`: `path`, `query` or `header`.
        params: Vec<(String, String)>,
        /// The input has a `body` property sent as JSON.
        body: bool,
    },
    Graphql {
        /// `query` or `mutation`.
        operation: String,
        field: String,
        /// `(name, GraphQL type)` of the field's arguments.
        args: Vec<(String, String)>,
        /// Default selection set (`{ id name }`), empty for scalars.
        selection: String,
    },
    Sql {
        /// `list_tables`, `describe_table`, `query` or `execute`.
        op: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Changes something: annotated `destructiveHint`, Kor always asks before running it.
    #[serde(default)]
    pub write: bool,
    pub plan: Plan,
}

impl GenTool {
    /// The MCP `tools/list` entry.
    pub fn descriptor(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {"readOnlyHint": !self.write, "destructiveHint": self.write},
        })
    }
}

/// A generated server, saved as `mcp-gen/<id>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub id: String,
    pub kind: Kind,
    /// What it was made from, for display (never with a password).
    pub source: String,
    /// `OpenAPI` base URL or `GraphQL` endpoint.
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub allow_writes: bool,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub db: Option<DbTarget>,
    #[serde(default = "default_rows")]
    pub row_limit: usize,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    pub tools: Vec<GenTool>,
    /// Tools left out (over [`MAX_TOOLS`], or writes not enabled).
    #[serde(default)]
    pub skipped: usize,
}

fn default_rows() -> usize {
    DEFAULT_ROWS
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT
}

impl Definition {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.clamp(1, 300))
    }

    pub fn rows(&self) -> usize {
        self.row_limit.clamp(1, MAX_ROWS)
    }

    /// Keep at most [`MAX_TOOLS`] tools with unique names.
    fn finish(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let before = self.tools.len();
        self.tools.retain(|t| seen.insert(t.name.clone()));
        self.tools.truncate(MAX_TOOLS);
        self.skipped += before - self.tools.len();
    }
}

/// What `kor mcp add` was asked to do.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    pub source: String,
    pub id: Option<String>,
    pub kind: Option<Kind>,
    /// `OpenAPI` base URL / `GraphQL` endpoint override (needed for files without `servers`).
    pub endpoint: Option<String>,
    pub allow_writes: bool,
    pub auth: Option<Auth>,
    pub label: Option<String>,
    pub row_limit: Option<usize>,
    pub timeout_secs: Option<u64>,
}

/// A server ready to be saved.
#[derive(Debug)]
pub struct Added {
    pub id: String,
    /// The `mcpServers` entry.
    pub entry: Value,
    pub definition: Option<Definition>,
    /// Database password taken out of the URL, for the keychain.
    pub password: Option<kor_secrets::SecretString>,
    pub kind: Kind,
    pub tools: usize,
    pub note: Option<String>,
}

/// `[a-z0-9][a-z0-9-]*`, at most 40 bytes (also a valid keychain id for `db-<id>`).
pub fn valid_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A valid id from free text (`Swagger Petstore` → `swagger-petstore`).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "mcp".into() } else { out }
}

/// `<config>/kor/mcp-gen/<id>.json` next to Kor's `mcp.json`.
pub fn definition_path(kor_file: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| {
        kor_file
            .with_file_name("mcp-gen")
            .join(format!("{id}.json"))
    })
}

/// Read a saved definition.
pub fn load(kor_file: &Path, id: &str) -> Result<Definition, String> {
    let path = definition_path(kor_file, id).ok_or("invalid server id")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e} (add it with `kor mcp add`)", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Tool count of a generated server without starting it, `None` for others.
pub fn tool_count(kor_file: &Path, raw: &Value) -> Option<usize> {
    let kor = raw.get("kor")?;
    if kor.get("generated").and_then(Value::as_str) == Some("local") {
        return None;
    }
    let args = mcp_config::strings(raw.get("args"));
    let id = args.last()?;
    load(kor_file, id).ok().map(|d| d.tools.len())
}

/// Truncate a tool result to [`MAX_RESPONSE`] characters with a note.
pub fn cap(mut text: String) -> String {
    if let Some((i, _)) = text.char_indices().nth(MAX_RESPONSE) {
        text.truncate(i);
        text.push_str("\n… [truncated by Kor: response larger than the limit]");
    }
    text
}

/// Read a schema from a file or a URL (through the network guard), bounded.
pub(crate) async fn fetch_text(source: &str) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let client = http_client(Duration::from_secs(30))?;
        let resp = client
            .get(source)
            .header(
                "accept",
                "application/json, application/yaml, text/yaml, */*",
            )
            .send()
            .await
            .map_err(|e| kor_core::net::describe(&e))?;
        let status = resp.status();
        let body = read_body(resp).await?;
        if !status.is_success() {
            return Err(format!("{source}: HTTP {status}"));
        }
        return Ok(body);
    }
    let meta = std::fs::metadata(source).map_err(|e| format!("{source}: {e}"))?;
    if meta.len() > MAX_BODY as u64 {
        return Err(format!("{source} is larger than {MAX_BODY} bytes"));
    }
    std::fs::read_to_string(source).map_err(|e| format!("{source}: {e}"))
}

pub(crate) fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| e.to_string())
}

/// A response body, at most [`MAX_BODY`] bytes (more is an error, not silently cut).
pub(crate) async fn read_body(mut resp: reqwest::Response) -> Result<String, String> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| kor_core::net::describe(&e))?
    {
        if buf.len() + chunk.len() > MAX_BODY {
            return Err(format!("response larger than {MAX_BODY} bytes"));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Build the server for `req` (fetching or introspecting its source), without saving anything.
/// `kor_exe` is the `kor` program generated servers are started with; `root` resolves relative
/// paths.
#[allow(clippy::too_many_lines)] // one branch per source kind
pub async fn generate(req: &AddRequest, root: &Path, kor_exe: &Path) -> Result<Added, String> {
    let source = req.source.trim();
    if source.is_empty() || source.len() > 4096 {
        return Err("source: 1-4096 characters".into());
    }
    if let Some(id) = &req.id
        && !valid_id(id)
    {
        return Err("id: lowercase letters, digits and -, at most 40".into());
    }
    let lower = source.to_ascii_lowercase();
    let resolve = |p: &str| {
        let p = PathBuf::from(p);
        if p.is_absolute() { p } else { root.join(p) }
    };
    let mut password = None;
    let mut def = if ["postgres://", "postgresql://", "mysql://", "sqlite:"]
        .iter()
        .any(|p| lower.starts_with(p))
        || req.kind == Some(Kind::Database)
    {
        let (target, pw) = db::parse_url(source, root)?;
        password = pw;
        let id = req.id.clone().unwrap_or_else(|| db::default_id(&target));
        db::definition(&id, target, req).await?
    } else if req.kind == Some(Kind::Local) || (!is_url(source) && resolve(source).is_dir()) {
        let dir = resolve(source);
        let found = local::detect(&dir)?;
        let id = req.id.clone().unwrap_or_else(|| {
            slug(
                &dir.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            )
        });
        let mut entry = json!({
            "command": found.command,
            "args": found.args,
            "kor": {"generated": "local", "source": dir.to_string_lossy()},
        });
        if let Some(l) = &req.label {
            entry["label"] = json!(l);
        }
        return Ok(Added {
            id,
            entry,
            definition: None,
            password: None,
            kind: Kind::Local,
            tools: 0,
            note: Some(found.note),
        });
    } else {
        let path_text = (!is_url(source)).then(|| resolve(source).to_string_lossy().into_owned());
        let location = path_text.as_deref().unwrap_or(source);
        let graphql_file = [".graphql", ".gql", ".graphqls"]
            .iter()
            .any(|e| lower.ends_with(e));
        let kind = match req.kind {
            Some(k) => k,
            None if graphql_file || lower.ends_with("/graphql") => Kind::Graphql,
            None => Kind::Openapi,
        };
        if kind == Kind::Graphql {
            graphql::definition(location, req).await?
        } else {
            {
                let text = fetch_text(location).await?;
                // An introspection result saved to a file is a GraphQL schema too.
                if text.contains("__schema") && !text.contains("\"openapi\"") {
                    graphql::definition(location, req).await?
                } else {
                    let base = is_url(source).then_some(source);
                    openapi::definition(&text, base, req)?
                }
            }
        }
    };
    if let Some(id) = &req.id {
        def.id.clone_from(id);
    }
    if !valid_id(&def.id) {
        def.id = slug(&def.id);
    }
    if let Some(n) = req.row_limit {
        def.row_limit = n.clamp(1, MAX_ROWS);
    }
    if let Some(t) = req.timeout_secs {
        def.timeout_secs = t.clamp(1, 300);
    }
    def.finish();
    let mut entry = json!({
        "command": kor_exe.to_string_lossy(),
        "args": ["mcp", "run", def.id],
        "kor": {"generated": def.kind.key(), "source": def.source},
    });
    if let Some(l) = &req.label {
        entry["label"] = json!(l);
    }
    Ok(Added {
        id: def.id.clone(),
        kind: def.kind,
        tools: def.tools.len(),
        note: (def.skipped > 0).then(|| {
            format!(
                "{} tool(s) left out (limit {MAX_TOOLS}, or writes not enabled)",
                def.skipped
            )
        }),
        entry,
        definition: Some(def),
        password,
    })
}

/// Save a generated server: its definition, the keychain password, and the `mcp.json` entry
/// (replacing one with the same id).
pub fn install(
    loc: &Locations,
    added: &Added,
    store: &dyn kor_secrets::SecretStore,
) -> Result<(), String> {
    if !valid_id(&added.id) {
        return Err("invalid server id".into());
    }
    if let Some(def) = &added.definition {
        let path = definition_path(&loc.kor_file, &added.id).ok_or("invalid server id")?;
        mcp_config::write_atomic(
            &path,
            &serde_json::to_value(def).map_err(|e| e.to_string())?,
        )?;
    }
    if let Some(pw) = &added.password {
        kor_secrets::store_key(&format!("db-{}", added.id), pw, store)
            .map_err(|e| format!("keychain: {e}"))?;
    }
    mcp_config::edit_servers(&loc.kor_file, |servers| {
        servers.insert(added.id.clone(), added.entry.clone());
        Ok(())
    })
}

/// Build the in-process server of a saved definition.
pub fn backend(def: Definition, secrets: Secrets) -> Arc<dyn serve::Backend> {
    match def.kind {
        Kind::Database => Arc::new(db::Server::new(def)),
        Kind::Graphql => Arc::new(graphql::Server::new(def, secrets)),
        Kind::Openapi | Kind::Local => Arc::new(openapi::Server::new(def, secrets)),
    }
}

/// Environment variable first (`NAME`), then the given store's `vault:NAME`.
pub fn secrets_from(store: Arc<dyn kor_secrets::SecretStore>) -> Secrets {
    Arc::new(move |name: &str| {
        if let Ok(v) = std::env::var(name)
            && !v.is_empty()
        {
            return Some(v);
        }
        store
            .get(&format!("vault:{name}"))
            .ok()
            .flatten()
            .map(|s| kor_secrets::ExposeSecret::expose_secret(&s).to_owned())
    })
}

// ---------------------------------------------------------------------------------------------
// `kor mcp test`
// ---------------------------------------------------------------------------------------------

/// Result of `kor mcp test <id>`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub id: String,
    pub connect_ms: u64,
    pub list_ms: u64,
    pub tools: Vec<String>,
    /// The read-only, argument-free tool that was called, if there was one.
    pub probe: Option<String>,
    pub probe_ms: u64,
    pub probe_ok: bool,
    /// First characters of the probe's answer, or the error.
    pub probe_output: String,
    pub error: Option<String>,
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Start the server, list its tools, and call one safe tool: annotated read-only and without
/// required arguments (`list_tables`, `GET /pets`…). Nothing that writes is ever called.
pub async fn test(spec: &crate::mcp_tools::ServerSpec, cwd: &Path) -> TestReport {
    let mut r = TestReport {
        id: spec.name.clone(),
        ..TestReport::default()
    };
    if let Some(b) = kor_core::net::refusal(&spec.name) {
        r.error = Some(b.message);
        return r;
    }
    let t = std::time::Instant::now();
    let client = match crate::mcp_tools::McpClient::start(spec, cwd).await {
        Ok(c) => c,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    r.connect_ms = ms(t.elapsed());
    let t = std::time::Instant::now();
    let tools = match client.list_tools().await {
        Ok(t) => t,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    r.list_ms = ms(t.elapsed());
    r.tools = tools.iter().map(|t| t.name.clone()).collect();
    let safe = tools.iter().find(|t| {
        t.annotations.get("readOnlyHint").and_then(Value::as_bool) == Some(true)
            && t.input_schema
                .get("required")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
    });
    if let Some(tool) = safe {
        r.probe = Some(tool.name.clone());
        let t = std::time::Instant::now();
        let out = client
            .request(
                "tools/call",
                json!({"name": tool.name, "arguments": {}}),
                Duration::from_secs(60),
                None,
            )
            .await;
        r.probe_ms = ms(t.elapsed());
        let (ok, text) = match out {
            Ok(v) => {
                let o = crate::mcp_tools::to_output(&v);
                (!o.is_error, o.content)
            }
            Err(e) => (false, e),
        };
        r.probe_ok = ok;
        r.probe_output = text.chars().take(300).collect();
    }
    r
}

/// Light JSON-Schema check of tool arguments at the trust boundary: object shape, required
/// keys, declared primitive types, enums and string lengths (nested up to 8 levels).
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, value, "arguments", 0)
}

fn check(schema: &Value, v: &Value, at: &str, depth: usize) -> Result<(), String> {
    if depth > 8 {
        return Ok(());
    }
    let ty = schema.get("type").and_then(Value::as_str);
    let ok = match ty {
        Some("object") => v.is_object(),
        Some("array") => v.is_array(),
        Some("string") => v.is_string(),
        Some("integer") => v.is_i64() || v.is_u64(),
        Some("number") => v.is_number(),
        Some("boolean") => v.is_boolean(),
        _ => true,
    };
    if !ok {
        return Err(format!("{at}: expected {}", ty.unwrap_or("value")));
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array)
        && !choices.contains(v)
    {
        return Err(format!("{at}: not one of the allowed values"));
    }
    if let (Some(s), Some(max)) = (v.as_str(), schema.get("maxLength").and_then(Value::as_u64))
        && s.chars().count() as u64 > max
    {
        return Err(format!("{at}: longer than {max} characters"));
    }
    if let Some(o) = v.as_object() {
        for req in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if o.get(req).is_none_or(Value::is_null) {
                return Err(format!("{at}: `{req}` is required"));
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        for (k, item) in o {
            match props.and_then(|p| p.get(k)) {
                Some(s) => check(s, item, &format!("{at}.{k}"), depth + 1)?,
                None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    return Err(format!("{at}: unknown property `{k}`"));
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(a)) = (schema.get("items"), v.as_array()) {
        for (i, item) in a.iter().enumerate() {
            check(items, item, &format!("{at}[{i}]"), depth + 1)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_auth_and_validation() {
        assert_eq!(
            slug("Swagger Petstore - OpenAPI 3.0"),
            "swagger-petstore-openapi-3-0"
        );
        assert_eq!(slug("…"), "mcp");
        assert!(valid_id("petstore") && !valid_id("Pet") && !valid_id("-x"));
        let a = Auth::parse("header:X-Api-Key:PET_TOKEN").unwrap();
        assert_eq!((a.place.as_str(), a.name.as_str()), ("header", "X-Api-Key"));
        assert!(Auth::parse("bearer:TOK").is_ok());
        for bad in ["cookie:a:b", "header:a b:T", "bearer:1x-y", "bearer"] {
            assert!(Auth::parse(bad).is_err(), "{bad}");
        }
        let schema = json!({"type": "object", "required": ["id"], "additionalProperties": false,
            "properties": {"id": {"type": "integer"}, "tag": {"type": "string", "enum": ["a", "b"]}}});
        assert!(validate(&schema, &json!({"id": 1, "tag": "a"})).is_ok());
        for bad in [
            json!({}),
            json!({"id": "1"}),
            json!({"id": 1, "tag": "c"}),
            json!({"id": 1, "x": 1}),
            json!([]),
        ] {
            assert!(validate(&schema, &bad).is_err(), "{bad}");
        }
        let long = "é".repeat(MAX_RESPONSE + 5);
        assert!(cap(long).ends_with("limit]"));
    }
}
