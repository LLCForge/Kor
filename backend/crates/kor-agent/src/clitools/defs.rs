//! Typed tool definitions (§26.2): a command template, typed arguments, a safety class.
//!
//! ```toml
//! # .kor/tools/thumbnail.toml
//! name = "thumbnail"
//! description = "Grab one frame of a video as a PNG"
//! command = ["ffmpeg", "-y", "-ss", "{at}", "-i", "{video}", "-frames:v", "1", "{out}"]
//! safety = "write"            # read | write | network | destructive
//! approval = false            # true: always ask, whatever the permission mode
//! [args.video]
//! type = "path"               # string | integer | number | boolean | path | enum | array
//! required = true
//! [args.at]
//! type = "string"
//! default = "00:00:01"
//! [args.out]
//! type = "path"
//! required = true
//! ```
//!
//! The program runs without a shell (each template token is one argument, values replace whole
//! tokens or `{name}` inside one), inside the session sandbox, network off unless the class is
//! `network`. Path arguments must stay inside the project. `read` is honored only for the user's
//! own definitions; project, team and model-generated ones always go through the permission check
//! like a shell command, and `destructive`/`approval` ones always ask.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

const MAX_FILE: u64 = 64 * 1024;
const MAX_DEFS: usize = 200;
const MAX_VALUE: usize = 4096;
const MAX_OUTPUT: usize = 30_000;
const DEFAULT_TIMEOUT_S: u64 = 120;
/// Programs a generated (model-written) definition may not wrap: they would run arbitrary code.
const INTERPRETERS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "cmd",
    "powershell",
    "pwsh",
    "python",
    "python3",
    "node",
    "deno",
    "bun",
    "perl",
    "ruby",
    "php",
    "env",
    "sudo",
    "doas",
    "xargs",
    "eval",
    "osascript",
    "wscript",
    "cscript",
    "mshta",
    "rundll32",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    Read,
    #[default]
    Write,
    Network,
    Destructive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgType {
    #[default]
    String,
    Integer,
    Number,
    Boolean,
    Path,
    Enum,
    Array,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArgDef {
    #[serde(rename = "type")]
    pub ty: ArgType,
    pub required: bool,
    pub description: String,
    /// Allowed values (`enum`), or allowed flag names for an `array` of flags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// `--flag value` (or just `--flag` for a boolean).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

fn command_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum C {
        One(String),
        Many(Vec<String>),
    }
    Ok(match C::deserialize(d)? {
        C::One(s) => s.split_whitespace().map(str::to_owned).collect(),
        C::Many(v) => v,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Template tokens: `["ffmpeg", "-i", "{input}", "{output}"]` (a string is split on spaces).
    #[serde(deserialize_with = "command_list")]
    pub command: Vec<String>,
    #[serde(default)]
    pub args: BTreeMap<String, ArgDef>,
    #[serde(default)]
    pub safety: Safety,
    #[serde(default)]
    pub approval: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u64>,
}

/// Where a definition came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    User,
    Project,
    Team,
    Generated,
}

/// Placeholders `{name}` in a token.
fn placeholders(token: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = token;
    while let Some(a) = rest.find('{') {
        let Some(b) = rest[a..].find('}') else { break };
        out.push(&rest[a + 1..a + b]);
        rest = &rest[a + b + 1..];
    }
    out
}

fn valid_tool_name(n: &str) -> bool {
    (2..=48).contains(&n.len())
        && n.starts_with(|c: char| c.is_ascii_lowercase())
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl ToolDef {
    /// Program (first template token).
    pub fn program(&self) -> &str {
        self.command.first().map_or("", String::as_str)
    }

    /// # Errors
    /// What is wrong with the definition.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_tool_name(&self.name) {
            return Err(format!(
                "name `{}`: 2-48 characters, lowercase letters, digits and `_`",
                self.name
            ));
        }
        let program = self.program();
        if !super::help::valid_name(program) {
            return Err(format!(
                "command must start with a program name, not `{program}`"
            ));
        }
        let mut used = Vec::new();
        for t in &self.command[1..] {
            for p in placeholders(t) {
                if !self.args.contains_key(p) {
                    return Err(format!("`{{{p}}}` is not an argument"));
                }
                if t != &format!("{{{p}}}")
                    && matches!(self.args[p].ty, ArgType::Array | ArgType::Boolean)
                {
                    return Err(format!(
                        "`{{{p}}}` ({:?}) must be a whole token",
                        self.args[p].ty
                    ));
                }
                used.push(p);
            }
        }
        for (n, a) in &self.args {
            if !used.contains(&n.as_str()) {
                return Err(format!("argument `{n}` is not used in the command"));
            }
            if a.ty == ArgType::Enum && a.values.is_empty() {
                return Err(format!("enum `{n}` needs `values`"));
            }
            if a.ty == ArgType::Boolean && a.flag.is_none() {
                return Err(format!("boolean `{n}` needs a `flag`"));
            }
        }
        Ok(())
    }

    /// JSON schema of the arguments, for the model.
    pub fn input_schema(&self) -> Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        for (n, a) in &self.args {
            let mut p = match a.ty {
                ArgType::String | ArgType::Path => json!({"type": "string"}),
                ArgType::Integer => json!({"type": "integer"}),
                ArgType::Number => json!({"type": "number"}),
                ArgType::Boolean => json!({"type": "boolean"}),
                ArgType::Enum => json!({"type": "string", "enum": a.values}),
                ArgType::Array if a.values.is_empty() => {
                    json!({"type": "array", "items": {"type": "string"}})
                }
                ArgType::Array => {
                    json!({"type": "array", "items": {"type": "string"}, "description": format!("each one of: {}", a.values.join(" "))})
                }
            };
            let desc = match (a.ty, a.description.is_empty()) {
                (ArgType::Path, true) => "path inside the project".to_owned(),
                (ArgType::Path, false) => format!("{} (path inside the project)", a.description),
                (_, _) => a.description.clone(),
            };
            if !desc.is_empty() {
                p["description"] = Value::String(desc);
            }
            if a.required {
                required.push(n.clone());
            }
            props.insert(n.clone(), p);
        }
        json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
    }

    /// The program and its arguments for `input`; path arguments resolved by `resolve`.
    ///
    /// # Errors
    /// A missing, unknown or invalid argument.
    pub fn render(
        &self,
        input: &Value,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Vec<String>, String> {
        let obj = input.as_object().cloned().unwrap_or_default();
        if let Some(k) = obj.keys().find(|k| !self.args.contains_key(*k)) {
            return Err(format!("unknown argument `{k}`"));
        }
        let mut values: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (n, a) in &self.args {
            let v = obj
                .get(n)
                .cloned()
                .filter(|v| !v.is_null())
                .or_else(|| a.default.clone().map(Value::String));
            let Some(v) = v else {
                if a.required {
                    return Err(format!("missing argument `{n}`"));
                }
                continue;
            };
            values.insert(n, Self::check(n, a, &v, resolve)?);
        }
        let mut argv = vec![self.program().to_owned()];
        for t in &self.command[1..] {
            let ps = placeholders(t);
            if ps.is_empty() {
                argv.push(t.clone());
            } else if ps.len() == 1 && *t == format!("{{{}}}", ps[0]) {
                let a = &self.args[ps[0]];
                if let Some(vs) = values.get(ps[0]) {
                    if let (Some(f), false) = (&a.flag, vs.is_empty() || a.ty == ArgType::Boolean) {
                        argv.push(f.clone());
                    }
                    argv.extend(vs.iter().cloned());
                }
            } else if ps.iter().all(|p| values.contains_key(p)) {
                let mut s = t.clone();
                for p in ps {
                    s = s.replace(
                        &format!("{{{p}}}"),
                        values[p].first().map_or("", String::as_str),
                    );
                }
                argv.push(s);
            }
        }
        Ok(argv)
    }

    fn check(
        n: &str,
        a: &ArgDef,
        v: &Value,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Vec<String>, String> {
        let text = |v: &Value| -> Result<String, String> {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(x) => x.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return Err(format!("`{n}` must be a {:?}", a.ty).to_lowercase()),
            };
            if s.len() > MAX_VALUE || s.contains(['\0', '\n', '\r']) {
                return Err(format!("`{n}`: invalid value"));
            }
            Ok(s)
        };
        // A value that starts with `-` would be read as an option by the program.
        let not_flag = |s: String| -> Result<String, String> {
            if s.starts_with('-') && a.ty != ArgType::Integer && a.ty != ArgType::Number {
                Err(format!("`{n}` may not start with `-`"))
            } else {
                Ok(s)
            }
        };
        match a.ty {
            ArgType::String => Ok(vec![not_flag(text(v)?)?]),
            ArgType::Path => Ok(vec![resolve(&not_flag(text(v)?)?)?]),
            ArgType::Integer => {
                let s = text(v)?;
                s.parse::<i64>()
                    .map_err(|_| format!("`{n}` must be an integer"))?;
                Ok(vec![s])
            }
            ArgType::Number => {
                let s = text(v)?;
                s.parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| format!("`{n}` must be a number"))?;
                Ok(vec![s])
            }
            ArgType::Boolean => match v {
                Value::Bool(true) => Ok(a.flag.clone().into_iter().collect()),
                Value::Bool(false) => Ok(Vec::new()),
                Value::String(s) if s == "true" => Ok(a.flag.clone().into_iter().collect()),
                Value::String(s) if s == "false" => Ok(Vec::new()),
                _ => Err(format!("`{n}` must be true or false")),
            },
            ArgType::Enum => {
                let s = text(v)?;
                if a.values.contains(&s) {
                    Ok(vec![s])
                } else {
                    Err(format!("`{n}` must be one of: {}", a.values.join(", ")))
                }
            }
            ArgType::Array => {
                let items = v
                    .as_array()
                    .ok_or_else(|| format!("`{n}` must be a list"))?;
                if items.len() > 64 {
                    return Err(format!("`{n}`: too many items"));
                }
                items
                    .iter()
                    .map(|i| {
                        let s = text(i)?;
                        if a.values.is_empty() {
                            not_flag(s)
                        } else if a.values.iter().any(|f| {
                            s == *f
                                || s.strip_prefix(f.as_str())
                                    .is_some_and(|r| r.starts_with('='))
                        }) {
                            Ok(s)
                        } else {
                            Err(format!("`{n}`: `{s}` is not one of the program's options"))
                        }
                    })
                    .collect()
            }
        }
    }
}

/// Quote for display (`a "b c"`).
pub fn display(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty()
                && a.chars()
                    .all(|c| c.is_alphanumeric() || "-_./:@=,+~\\%".contains(c))
            {
                a.clone()
            } else {
                format!("\"{}\"", a.replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where definitions live. `project` is `None` for an untrusted project.
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    /// `<kor home>/tools` (user `*.toml`, `team/`, `generated/`).
    pub home: Option<PathBuf>,
    /// `<project>/.kor/tools`, only for a trusted project.
    pub project: Option<PathBuf>,
}

/// A loaded definition.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub def: ToolDef,
    pub origin: Origin,
    pub file: PathBuf,
}

fn read_dir(dir: &Path, origin: Origin, out: &mut Vec<Loaded>, errors: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    for f in files {
        if out.len() >= MAX_DEFS {
            errors.push(format!(
                "{}: more than {MAX_DEFS} tool definitions",
                dir.display()
            ));
            return;
        }
        let parsed = match std::fs::metadata(&f) {
            Ok(m) if m.len() <= MAX_FILE => std::fs::read_to_string(&f)
                .map_err(|e| e.to_string())
                .and_then(|t| toml::from_str::<ToolDef>(&t).map_err(|e| e.to_string())),
            Ok(_) => Err("file too large".into()),
            Err(e) => Err(e.to_string()),
        };
        match parsed.and_then(|d| {
            d.validate()?;
            if origin == Origin::Generated
                && INTERPRETERS.contains(&d.program().trim_end_matches(".exe"))
            {
                return Err(format!("generated tools cannot wrap `{}`", d.program()));
            }
            Ok(d)
        }) {
            Ok(def) if out.iter().any(|l| l.def.name == def.name) => {
                errors.push(format!(
                    "{}: `{}` is already defined",
                    f.display(),
                    def.name
                ));
            }
            Ok(def) => out.push(Loaded {
                def,
                origin,
                file: f,
            }),
            Err(e) => errors.push(format!("{}: {e}", f.display())),
        }
    }
}

/// Every definition, the user's first (a project cannot replace a user tool), with the files
/// that could not be used.
pub fn load(dirs: &Dirs) -> (Vec<Loaded>, Vec<String>) {
    let (mut out, mut errors) = (Vec::new(), Vec::new());
    if let Some(h) = &dirs.home {
        read_dir(h, Origin::User, &mut out, &mut errors);
    }
    if let Some(p) = &dirs.project {
        read_dir(p, Origin::Project, &mut out, &mut errors);
    }
    if let Some(h) = &dirs.home {
        read_dir(&h.join("team"), Origin::Team, &mut out, &mut errors);
        read_dir(
            &h.join("generated"),
            Origin::Generated,
            &mut out,
            &mut errors,
        );
    }
    (out, errors)
}

/// Resolve a path argument inside the project (absolute result).
fn resolver(ctx: &ToolCtx) -> impl Fn(&str) -> Result<String, String> + '_ {
    move |p: &str| ctx.resolve(p).map(|x| x.display().to_string())
}

/// Run `argv` without a shell in the session sandbox.
async fn execute(argv: &[String], net: bool, timeout_s: u64, ctx: &ToolCtx) -> ToolOutput {
    let Some(program) = kor_platform::on_path::which(&argv[0]) else {
        return ToolOutput::err(format!("`{}` is not on PATH", argv[0]));
    };
    let mut policy = ctx.sandbox_policy();
    policy.network = policy.network && net;
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&argv[1..]).current_dir(
        ctx.cwd
            .lock()
            .map_or_else(|_| ctx.root.clone(), |c| c.clone()),
    );
    for v in &ctx.scrub_env {
        cmd.env_remove(v);
    }
    cmd.envs(ctx.env.iter().map(|(k, v)| (k, v)));
    let child = match kor_sandbox::spawn(ctx.sandbox, &policy, &cmd) {
        Ok(c) => c,
        Err(e) => return ToolOutput::err(format!("failed to start {}: {e}", argv[0])),
    };
    let out = tokio::select! {
        r = child.wait_with_output() => r,
        () = tokio::time::sleep(Duration::from_secs(timeout_s)) => {
            return ToolOutput::err(format!("command timed out after {timeout_s}s and was killed"));
        }
        () = ctx.cancel.cancelled() => return ToolOutput::err("command cancelled by the user"),
    };
    let out = match out {
        Ok(o) => o,
        Err(e) => return ToolOutput::err(e.to_string()),
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        text.push_str("\n[stderr]\n");
        text.push_str(&stderr);
    }
    let cut: String = text.trim_end().chars().take(MAX_OUTPUT).collect();
    let code = out
        .status
        .code()
        .map_or_else(|| "killed by signal".into(), |c| c.to_string());
    let mut body = format!("{cut}\n[exit code: {code}]");
    ctx.note_sandbox_blocks(&policy, &text, &mut body);
    if out.status.success() {
        ToolOutput::ok(body)
    } else {
        ToolOutput::err(body)
    }
}

fn kind_of(l: &Loaded) -> ToolKind {
    if l.def.safety == Safety::Read && l.origin == Origin::User {
        ToolKind::Read
    } else {
        ToolKind::Execute
    }
}

/// The permission target: the rendered command, marked to always ask when the class says so.
fn target_of(l: &Loaded, input: &Value) -> String {
    let argv = l
        .def
        .render(input, &|p| Ok(p.to_owned()))
        .unwrap_or_else(|_| l.def.command.clone());
    let line = display(&argv);
    if l.def.approval || l.def.safety == Safety::Destructive {
        format!("{} {line}", crate::mcp_tools::WRITE_MARK)
    } else {
        line
    }
}

fn touches_of(l: &Loaded, input: &Value) -> Vec<String> {
    if !matches!(l.def.safety, Safety::Write | Safety::Destructive) {
        return Vec::new();
    }
    l.def
        .args
        .iter()
        .filter(|(_, a)| a.ty == ArgType::Path)
        .filter_map(|(n, _)| input.get(n).and_then(Value::as_str).map(str::to_owned))
        .collect()
}

async fn run_loaded(l: &Loaded, input: &Value, ctx: &ToolCtx) -> ToolOutput {
    let argv = match l.def.render(input, &resolver(ctx)) {
        Ok(a) => a,
        Err(e) => return ToolOutput::err(format!("{e}\nschema: {}", l.def.input_schema())),
    };
    execute(
        &argv,
        l.def.safety == Safety::Network,
        l.def.timeout_s.unwrap_or(DEFAULT_TIMEOUT_S).clamp(1, 3600),
        ctx,
    )
    .await
}

/// One definition as an agent tool.
#[derive(Debug)]
pub struct DefinedTool {
    loaded: Loaded,
    name: &'static str,
}

impl DefinedTool {
    pub fn new(loaded: Loaded) -> Self {
        Self {
            name: crate::mcp_tools::intern(loaded.def.name.clone()),
            loaded,
        }
    }

    pub fn safety(&self) -> Safety {
        self.loaded.def.safety
    }
}

#[async_trait]
impl Tool for DefinedTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn rule_name(&self) -> &'static str {
        // `Bash(ffmpeg *)` rules match the rendered command, like a shell command.
        "Bash"
    }
    fn kind(&self) -> ToolKind {
        kind_of(&self.loaded)
    }
    fn spec(&self) -> ToolSpec {
        let d = &self.loaded.def;
        let mut description = if d.description.is_empty() {
            format!("Run {}", d.program())
        } else {
            d.description.clone()
        };
        let _ = write!(
            description,
            ". Runs `{}` (no shell, sandboxed).",
            d.command.join(" ")
        );
        for e in d.examples.iter().take(3) {
            let _ = write!(description, " Example: {e}.");
        }
        ToolSpec {
            name: d.name.clone(),
            description,
            input_schema: d.input_schema(),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        Some(target_of(&self.loaded, input))
    }
    fn touches(&self, input: &Value) -> Vec<String> {
        touches_of(&self.loaded, input)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        run_loaded(&self.loaded, &input, ctx).await
    }
}

/// `cli_tool`: run any definition by name, including one the model generated this session.
pub struct CliTool {
    dirs: Dirs,
    permits: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    privacy: bool,
}

impl std::fmt::Debug for CliTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliTool")
            .field("dirs", &self.dirs)
            .field("privacy", &self.privacy)
            .finish_non_exhaustive()
    }
}

impl CliTool {
    pub fn new(
        dirs: Dirs,
        permits: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        privacy: bool,
    ) -> Self {
        Self {
            dirs,
            permits,
            privacy,
        }
    }

    fn find(&self, input: &Value) -> Result<(Loaded, Value), String> {
        let name = input
            .get("tool")
            .and_then(Value::as_str)
            .ok_or("`tool` is required")?;
        let (all, _) = load(&self.dirs);
        let l = all
            .into_iter()
            .find(|l| l.def.name == name)
            .ok_or_else(|| format!("no tool definition `{name}`"))?;
        if !(self.permits)(l.def.program()) {
            return Err(format!(
                "`{}` is not allowed by [tools] in the configuration",
                l.def.program()
            ));
        }
        if self.privacy && l.def.safety == Safety::Network {
            return Err(format!(
                "`{name}` needs the network, which privacy mode turns off"
            ));
        }
        Ok((l, input.get("args").cloned().unwrap_or_else(|| json!({}))))
    }
}

#[async_trait]
impl Tool for CliTool {
    fn name(&self) -> &'static str {
        "cli_tool"
    }
    fn rule_name(&self) -> &'static str {
        "Bash"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cli_tool".into(),
            description: "Run a typed command-line tool definition by name (user, project, team or generated with `tool_define`). `args` must match its schema; see `tools_search`.".into(),
            input_schema: json!({"type":"object","properties":{"tool":{"type":"string"},"args":{"type":"object"}},"required":["tool"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        match self.find(input) {
            Ok((l, a)) => Some(target_of(&l, &a)),
            Err(_) => input.get("tool").and_then(Value::as_str).map(str::to_owned),
        }
    }
    fn touches(&self, input: &Value) -> Vec<String> {
        self.find(input)
            .map(|(l, a)| touches_of(&l, &a))
            .unwrap_or_default()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        match self.find(&input) {
            Ok((l, a)) => run_loaded(&l, &a, ctx).await,
            Err(e) => ToolOutput::err(e),
        }
    }
}

/// `tool_define`: the model saves a typed schema for a CLI (from its `--help`) to
/// `<kor home>/tools/generated/<name>.toml`; later calls through `cli_tool` are validated.
#[derive(Debug)]
pub struct ToolDefine {
    generated: PathBuf,
}

impl ToolDefine {
    pub fn new(generated: PathBuf) -> Self {
        Self { generated }
    }

    fn parse(input: &Value) -> Result<ToolDef, String> {
        let mut def: ToolDef = serde_json::from_value(input.clone()).map_err(|e| e.to_string())?;
        def.validate()?;
        if INTERPRETERS.contains(&def.program().trim_end_matches(".exe")) {
            return Err(format!(
                "generated tools cannot wrap `{}`; use the shell tool",
                def.program()
            ));
        }
        if kor_platform::on_path::which(def.program()).is_none() {
            return Err(format!("`{}` is not on PATH", def.program()));
        }
        // A model cannot declare its own tool read-only.
        if def.safety == Safety::Read {
            def.safety = Safety::Write;
        }
        Ok(def)
    }
}

#[async_trait]
impl Tool for ToolDefine {
    fn name(&self) -> &'static str {
        "tool_define"
    }
    fn rule_name(&self) -> &'static str {
        "tool_define"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "tool_define".into(),
            description: "Save a typed definition for a command-line program (after reading its help with `tools_search` describe) so later calls through `cli_tool` are checked. Fields: name (snake_case), description, command (template tokens, e.g. [\"ffmpeg\",\"-i\",\"{input}\",\"{output}\"]), args ({name: {type: string|integer|number|boolean|path|enum|array, required, description, values, flag, default}}), safety (write|network|destructive).".into(),
            input_schema: json!({"type":"object","properties":{
                "name":{"type":"string"},"description":{"type":"string"},
                "command":{"type":"array","items":{"type":"string"}},
                "args":{"type":"object"},"safety":{"type":"string","enum":["write","network","destructive"]},
                "examples":{"type":"array","items":{"type":"string"}}},
                "required":["name","command"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        Some(format!(
            "tool_define {}: {}",
            input.get("name").and_then(Value::as_str).unwrap_or(""),
            input
                .get("command")
                .map(Value::to_string)
                .unwrap_or_default()
        ))
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let def = match Self::parse(&input) {
            Ok(d) => d,
            Err(e) => return ToolOutput::err(e),
        };
        let file = self.generated.join(format!("{}.toml", def.name));
        let text = match toml::to_string_pretty(&def) {
            Ok(t) => t,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let written =
            std::fs::create_dir_all(&self.generated).and_then(|()| std::fs::write(&file, text));
        match written {
            Ok(()) => ToolOutput::ok(format!(
                "saved {}; call it with cli_tool {{\"tool\": \"{}\", \"args\": {{…}}}}",
                file.display(),
                def.name
            )),
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

/// `kor tool add`: a definition from a one-line template; every `{placeholder}` becomes a
/// required string argument (`path` when its name says so).
///
/// # Errors
/// An invalid name or template.
pub fn scaffold(
    name: &str,
    template: &str,
    safety: Safety,
    description: &str,
) -> Result<ToolDef, String> {
    let command: Vec<String> = template.split_whitespace().map(str::to_owned).collect();
    let mut args = BTreeMap::new();
    for t in &command {
        for p in placeholders(t) {
            let pathish = [
                "file", "path", "dir", "input", "output", "out", "src", "dest",
            ]
            .iter()
            .any(|w| p.contains(w));
            args.entry(p.to_owned()).or_insert(ArgDef {
                ty: if pathish {
                    ArgType::Path
                } else {
                    ArgType::String
                },
                required: true,
                ..ArgDef::default()
            });
        }
    }
    let def = ToolDef {
        name: name.to_owned(),
        description: description.to_owned(),
        command,
        args,
        safety,
        approval: false,
        examples: Vec::new(),
        timeout_s: None,
    };
    def.validate()?;
    Ok(def)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THUMB: &str = r#"
name = "thumbnail"
description = "Grab one frame"
command = ["ffmpeg", "-y", "-ss", "{at}", "-i", "{video}", "-frames:v", "1", "{out}", "{quiet}", "--scale={w}"]
safety = "write"
[args.video]
type = "path"
required = true
[args.at]
default = "00:00:01"
[args.out]
type = "path"
required = true
[args.quiet]
type = "boolean"
flag = "-loglevel=error"
[args.w]
type = "integer"
"#;

    fn ok(p: &str) -> Result<String, String> {
        if p.contains("..") {
            Err("outside".into())
        } else {
            Ok(format!("/p/{p}"))
        }
    }

    #[test]
    fn templates_render_typed_arguments_without_a_shell() {
        let d: ToolDef = toml::from_str(THUMB).unwrap();
        d.validate().unwrap();
        let argv = d
            .render(
                &json!({"video": "a.mp4", "out": "a.png", "quiet": true, "w": 320}),
                &ok,
            )
            .unwrap();
        assert_eq!(
            argv,
            [
                "ffmpeg",
                "-y",
                "-ss",
                "00:00:01",
                "-i",
                "/p/a.mp4",
                "-frames:v",
                "1",
                "/p/a.png",
                "-loglevel=error",
                "--scale=320"
            ]
        );
        // Optional values left out drop their token.
        let argv = d
            .render(&json!({"video": "a.mp4", "out": "a.png"}), &ok)
            .unwrap();
        assert_eq!(argv.last().map(String::as_str), Some("/p/a.png"));
        // `; rm -rf /` is one argument, never a command.
        let argv = d
            .render(&json!({"video": "a.mp4", "out": "x; rm -rf /"}), &ok)
            .unwrap();
        assert!(argv.contains(&"/p/x; rm -rf /".to_owned()));
        assert!(
            d.render(&json!({"out": "a.png"}), &ok)
                .unwrap_err()
                .contains("missing argument `video`")
        );
        assert!(
            d.render(&json!({"video": "../etc/passwd", "out": "a"}), &ok)
                .is_err()
        );
        assert!(
            d.render(&json!({"video": "-i", "out": "a"}), &ok)
                .unwrap_err()
                .contains("may not start with `-`")
        );
        assert!(
            d.render(&json!({"video": "a", "out": "b", "w": "wide"}), &ok)
                .is_err()
        );
        assert!(
            d.render(&json!({"video": "a", "out": "b", "zzz": 1}), &ok)
                .unwrap_err()
                .contains("unknown argument")
        );
        let schema = d.input_schema();
        assert_eq!(schema["required"], json!(["out", "video"]));
        assert_eq!(schema["properties"]["quiet"]["type"], "boolean");
    }

    #[test]
    fn invalid_definitions_are_rejected() {
        let bad = |s: &str| {
            toml::from_str::<ToolDef>(s)
                .map_err(|e| e.to_string())
                .and_then(|d| d.validate())
        };
        assert!(bad("name = \"x\"\ncommand = \"git\"").is_err()); // name too short
        assert!(bad("name = \"Bad-Name\"\ncommand = \"git\"").is_err());
        assert!(bad("name = \"gs\"\ncommand = \"git {nope}\"").is_err());
        assert!(bad("name = \"gs\"\ncommand = \"../git status\"").is_err());
        assert!(bad("name = \"gs\"\ncommand = \"git status\"\n[args.unused]\n").is_err());
        assert!(
            bad("name = \"gs\"\ncommand = \"git status {b}\"\n[args.b]\ntype = \"boolean\"")
                .is_err()
        );
        assert!(bad("name = \"gs\"\ncommand = \"git status\"").is_ok());
    }

    #[test]
    fn origins_order_and_safety() {
        let home = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("gs.toml"),
            "name = \"gs\"\ncommand = \"git status\"\nsafety = \"read\"",
        )
        .unwrap();
        std::fs::write(
            proj.path().join("gs.toml"),
            "name = \"gs\"\ncommand = \"git push --force\"",
        )
        .unwrap();
        std::fs::write(
            proj.path().join("nuke.toml"),
            "name = \"nuke\"\ncommand = \"git clean -fdx\"\nsafety = \"read\"",
        )
        .unwrap();
        std::fs::write(proj.path().join("broken.toml"), "name = ").unwrap();
        std::fs::create_dir_all(home.path().join("generated")).unwrap();
        std::fs::write(
            home.path().join("generated/sh_run.toml"),
            "name = \"sh_run\"\ncommand = \"bash -c {s}\"\n[args.s]\n",
        )
        .unwrap();
        let dirs = Dirs {
            home: Some(home.path().into()),
            project: Some(proj.path().into()),
        };
        let (all, errors) = load(&dirs);
        let names: Vec<&str> = all.iter().map(|l| l.def.name.as_str()).collect();
        assert_eq!(names, ["gs", "nuke"]);
        assert_eq!(errors.len(), 3, "{errors:?}"); // duplicate, broken, generated shell
        let gs = DefinedTool::new(all[0].clone());
        assert_eq!(gs.kind(), ToolKind::Read);
        assert_eq!(gs.target(&json!({})).as_deref(), Some("git status"));
        // A project cannot declare itself read-only.
        let nuke = DefinedTool::new(all[1].clone());
        assert_eq!(nuke.kind(), ToolKind::Execute);
        let mut d = all[1].clone();
        d.def.safety = Safety::Destructive;
        assert!(
            DefinedTool::new(d)
                .target(&json!({}))
                .unwrap()
                .starts_with(crate::mcp_tools::WRITE_MARK)
        );
    }

    #[test]
    fn scaffold_and_define() {
        let d = scaffold(
            "cut_video",
            "ffmpeg -i {input} -t {seconds} {output}",
            Safety::Write,
            "Cut",
        )
        .unwrap();
        assert_eq!(d.args["input"].ty, ArgType::Path);
        assert_eq!(d.args["seconds"].ty, ArgType::String);
        assert!(scaffold("x", "ffmpeg", Safety::Write, "").is_err());
        assert!(
            ToolDefine::parse(
                &json!({"name": "shx", "command": ["bash", "-c", "{s}"], "args": {"s": {}}})
            )
            .is_err()
        );
        if kor_platform::on_path::which("git").is_some() {
            let d = ToolDefine::parse(&json!({"name": "git_log", "command": ["git", "log", "-n", "{n}"], "args": {"n": {"type": "integer", "required": true}}, "safety": "write"})).unwrap();
            assert_eq!(d.args["n"].ty, ArgType::Integer);
        }
        assert_eq!(display(&["a".into(), "b c".into()]), "a \"b c\"");
    }

    #[tokio::test]
    async fn defined_tools_run_sandboxed_programs() {
        if kor_platform::on_path::which("git").is_none() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("gv.toml"),
            "name = \"git_version\"\ncommand = \"git --version\"\nsafety = \"read\"",
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(root.path()).unwrap();
        let dirs = Dirs {
            home: Some(home.path().into()),
            project: None,
        };
        let runner = CliTool::new(dirs.clone(), Arc::new(|_| true), false);
        let out = runner.call(json!({"tool": "git_version"}), &ctx).await;
        assert!(
            !out.is_error && out.content.contains("git version"),
            "{}",
            out.content
        );
        let denied = CliTool::new(dirs, Arc::new(|n| n != "git"), false);
        assert!(
            denied
                .call(json!({"tool": "git_version"}), &ctx)
                .await
                .is_error
        );
        assert!(runner.call(json!({"tool": "nope"}), &ctx).await.is_error);
        let def = ToolDefine::new(home.path().join("generated"));
        let out = def
            .call(
                json!({"name": "git_short", "command": ["git", "--version"]}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        let (all, _) = load(&Dirs {
            home: Some(home.path().into()),
            project: None,
        });
        let short = all.iter().find(|l| l.def.name == "git_short").unwrap();
        assert_eq!(
            (short.origin, short.def.safety),
            (Origin::Generated, Safety::Write)
        );
    }
}
