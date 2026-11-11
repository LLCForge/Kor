//! Devcontainer / Nix / mise awareness (§15.7, ADR 0043).
//!
//! [`detect`] reads the project's environment files and version pins. A [`Session`] (one per agent
//! session, in `ToolCtx::dev_env`) turns every command the agent runs into a command inside that
//! environment ([`Session::command`]) and, before the first one, compares the versions the
//! environment really provides with the pinned ones ([`Session::gate`]): on a mismatch the command
//! is refused (`on_mismatch = "stop"`) or runs with a warning (`"warn"`), never silently.
//!
//! Sandbox composition: host-side wrappers (nix, mise, asdf shims, plain pins) run *inside* the
//! OS sandbox. Container environments (devcontainer, compose) run their command in a container the
//! Docker daemon starts, which no host sandbox can confine; such commands are spawned without the
//! sandbox and the runtime makes every `Bash` call ask first. Under the Docker sandbox backend the
//! image is the environment and no wrapper is applied.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use kor_platform::Os;
use kor_sandbox::SandboxKind;
use serde::Serialize;

use crate::Shell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Node,
    Rust,
    Python,
    Go,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "node" | "nodejs" => Some(Self::Node),
            "rust" => Some(Self::Rust),
            "python" => Some(Self::Python),
            "go" | "golang" => Some(Self::Go),
            _ => None,
        }
    }

    /// The command that prints the version.
    fn probe(self) -> &'static str {
        match self {
            Self::Node => "node --version",
            Self::Rust => "rustc --version",
            Self::Python if kor_platform::os() == Os::Windows => "python --version",
            Self::Python => "python3 --version",
            Self::Go => "go version",
        }
    }

    /// How to install version `v`.
    pub fn install_hint(self, v: &str) -> String {
        match self {
            Self::Node => format!("nvm install {v} / mise use node@{v}"),
            Self::Rust => format!("rustup toolchain install {v}"),
            Self::Python => format!("pyenv install {v} / mise use python@{v}"),
            Self::Go => format!("mise use go@{v} / https://go.dev/dl/"),
        }
    }
}

/// A version the project pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pin {
    pub tool: Tool,
    pub version: String,
    /// File it came from (project-relative).
    pub source: String,
}

/// The environment commands run in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    Devcontainer,
    Compose {
        file: String,
        /// Service commands run in (`[env] service`, else the first one in the file).
        service: Option<String>,
    },
    Nix {
        flake: bool,
    },
    Mise,
    Asdf,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Devcontainer => "devcontainer",
            Self::Compose { .. } => "compose",
            Self::Nix { .. } => "nix",
            Self::Mise => "mise",
            Self::Asdf => "asdf",
        }
    }

    /// The program the wrapper needs.
    pub fn program(&self) -> &'static str {
        match self {
            Self::Devcontainer => "devcontainer",
            Self::Compose { .. } => "docker",
            Self::Nix { flake: true } => "nix",
            Self::Nix { flake: false } => "nix-shell",
            Self::Mise => "mise",
            Self::Asdf => "asdf",
        }
    }

    /// Commands run in a container the Docker daemon starts (no host sandbox can confine them).
    pub fn container(&self) -> bool {
        matches!(self, Self::Devcontainer | Self::Compose { .. })
    }

    pub fn install_hint(&self) -> &'static str {
        match self {
            Self::Devcontainer => "npm install -g @devcontainers/cli",
            Self::Compose { .. } => "https://docs.docker.com/get-docker/",
            Self::Nix { .. } => "https://nixos.org/download",
            Self::Mise => "https://mise.jdx.dev/getting-started.html",
            Self::Asdf => "https://asdf-vm.com/guide/getting-started.html",
        }
    }
}

/// What the project's files say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Detected {
    /// Every environment or version file found (project-relative).
    pub files: Vec<String>,
    pub kind: Option<Kind>,
    pub pins: Vec<Pin>,
    /// `Dockerfile.dev` without a compose file or devcontainer: reported, Kor never builds images.
    pub dockerfile_dev: bool,
}

impl Detected {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Whether `name` (without extension) is on `PATH`.
pub fn on_path(name: &str) -> bool {
    let exts: &[&str] = if kor_platform::os() == Os::Windows {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| exts.iter().any(|e| d.join(format!("{name}{e}")).is_file()))
    })
}

fn read(root: &Path, rel: &str) -> Option<String> {
    let p = root.join(rel);
    // Version files are tiny; anything large is not one.
    if std::fs::metadata(&p).ok()?.len() > 64 * 1024 {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

/// First non-empty, non-comment line.
fn first_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
}

/// First quoted string after `=` on a `key = "value"` / `key = ["value"]` line.
fn quoted(line: &str) -> Option<&str> {
    let v = line.split_once('=')?.1;
    let start = v.find('"')? + 1;
    let len = v[start..].find('"')?;
    Some(&v[start..start + len])
}

/// The first service under `services:` (two-space or any indented key directly below it).
/// shortcut: a line scan, not a YAML parser; `[env] service` overrides it.
fn first_service(yaml: &str) -> Option<String> {
    let mut in_services = false;
    let mut indent = None;
    for line in yaml.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with('#') || trimmed.trim().is_empty() {
            continue;
        }
        let depth = trimmed.len() - trimmed.trim_start().len();
        if depth == 0 {
            in_services = trimmed == "services:";
            continue;
        }
        if in_services && indent.is_none_or(|i| depth == i) {
            let key = trimmed.trim_start().strip_suffix(':')?;
            if !key.is_empty() && !key.contains(' ') {
                return Some(key.trim_matches(['"', '\'']).to_owned());
            }
            indent = Some(depth);
        }
    }
    None
}

/// Read the project's environment files. `has` says whether a program is installed (decides
/// between mise and asdf for `.tool-versions`); `service` is `[env] service`.
pub fn detect_with(root: &Path, service: &str, has: &dyn Fn(&str) -> bool) -> Detected {
    let mut d = Detected::default();
    let exists = |rel: &str| root.join(rel).is_file();
    let mut found = |rel: &str| -> bool {
        let e = exists(rel);
        if e {
            d.files.push(rel.to_owned());
        }
        e
    };
    let devcontainer = found(".devcontainer/devcontainer.json") | found(".devcontainer.json");
    let compose = ["docker-compose.dev.yml", "docker-compose.dev.yaml"]
        .into_iter()
        .find(|f| found(f));
    let flake = found("flake.nix");
    let shell_nix = found("shell.nix");
    let dockerfile = found("Dockerfile.dev");
    let mise = found("mise.toml") | found(".mise.toml");
    let tool_versions = found(".tool-versions");
    for f in [
        ".nvmrc",
        ".node-version",
        "rust-toolchain.toml",
        "rust-toolchain",
        ".python-version",
    ] {
        found(f);
    }

    d.kind = if devcontainer {
        Some(Kind::Devcontainer)
    } else if let Some(file) = compose {
        let service = Some(service.trim().to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| read(root, file).as_deref().and_then(first_service));
        Some(Kind::Compose {
            file: file.to_owned(),
            service,
        })
    } else if flake || shell_nix {
        Some(Kind::Nix { flake })
    } else if mise || (tool_versions && (has("mise") || !has("asdf"))) {
        Some(Kind::Mise)
    } else if tool_versions {
        Some(Kind::Asdf)
    } else {
        None
    };
    d.dockerfile_dev = dockerfile && !devcontainer && compose.is_none();
    read_pins(root, &mut d);
    d
}

/// Versions pinned by `.nvmrc`, `rust-toolchain(.toml)`, `.python-version`, `.tool-versions`, mise.
fn read_pins(root: &Path, d: &mut Detected) {
    let mut pin = |tool: Tool, version: &str, source: &str| {
        let version = version.trim().trim_matches(['"', '\'']);
        if !version.is_empty()
            && !d
                .pins
                .iter()
                .any(|p| p.tool == tool && p.version == version)
        {
            d.pins.push(Pin {
                tool,
                version: version.to_owned(),
                source: source.to_owned(),
            });
        }
    };
    for f in [".nvmrc", ".node-version"] {
        if let Some(v) = read(root, f).as_deref().and_then(first_line) {
            pin(Tool::Node, v, f);
        }
    }
    if let Some(t) = read(root, "rust-toolchain.toml")
        && let Some(v) = t
            .lines()
            .find(|l| l.trim_start().starts_with("channel"))
            .and_then(quoted)
    {
        pin(Tool::Rust, v, "rust-toolchain.toml");
    } else if let Some(v) = read(root, "rust-toolchain").as_deref().and_then(first_line) {
        pin(Tool::Rust, v, "rust-toolchain");
    }
    if let Some(v) = read(root, ".python-version")
        .as_deref()
        .and_then(first_line)
    {
        pin(Tool::Python, v, ".python-version");
    }
    if let Some(t) = read(root, ".tool-versions") {
        for line in t.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
            let mut w = line.split_whitespace();
            if let (Some(name), Some(v)) = (w.next(), w.next())
                && let Some(tool) = Tool::from_name(name)
            {
                pin(tool, v, ".tool-versions");
            }
        }
    }
    for f in ["mise.toml", ".mise.toml"] {
        let Some(t) = read(root, f) else { continue };
        let mut in_tools = false;
        for line in t.lines().map(str::trim) {
            if line.starts_with('[') {
                in_tools = line == "[tools]";
            } else if in_tools
                && let Some((name, _)) = line.split_once('=')
                && let Some(tool) = Tool::from_name(name.trim().trim_matches('"'))
                && let Some(v) = quoted(line)
            {
                pin(tool, v, f);
            }
        }
    }
}

/// [`detect_with`] using the real `PATH`.
pub fn detect(root: &Path, service: &str) -> Detected {
    detect_with(root, service, &on_path)
}

/// Numeric parts of a version (`v20.11.1` → `[20, 11, 1]`); `None` for `lts/*`, `stable`, ….
fn numbers(v: &str) -> Option<Vec<u64>> {
    let v = v
        .trim()
        .trim_start_matches(['v', 'V'])
        .trim_start_matches("go");
    let parts: Vec<u64> = v
        .split(['.', '-', '+'])
        .map_while(|p| p.parse().ok())
        .collect();
    (!parts.is_empty() && v.starts_with(|c: char| c.is_ascii_digit())).then_some(parts)
}

/// Whether `found` satisfies the pin `expected` (`20` accepts `20.11.1`, `1.82.0` needs exactly
/// that); `None` when the pin is not a number (`lts/*`, `stable`, `nightly`).
pub fn matches(expected: &str, found: &str) -> Option<bool> {
    let want = numbers(expected)?;
    let have = numbers(found)?;
    Some(have.len() >= want.len() && have[..want.len()] == want[..])
}

/// First `N.N[.N]` in a version command's output.
pub fn parse_version(out: &str) -> Option<String> {
    let bytes = out.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit()
            && (i == 0
                || !bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b'v'
                || out[..i].ends_with("go"))
        {
            let rest = &out[i..];
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(rest.len());
            let v = rest[..end].trim_end_matches('.');
            if v.contains('.') {
                return Some(v.to_owned());
            }
            i += end.max(1);
        } else {
            i += 1;
        }
    }
    None
}

/// One pinned tool compared with what the environment has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Version {
    pub tool: Tool,
    pub expected: String,
    pub source: String,
    pub found: Option<String>,
    /// `None`: not comparable (`lts/*`) or not checked.
    pub ok: Option<bool>,
}

impl Version {
    /// Missing, or a different version.
    pub fn bad(&self) -> bool {
        self.ok == Some(false) || (self.found.is_none() && numbers(&self.expected).is_some())
    }
}

/// One shell line that prints `@@tool` then the version of every pinned tool.
pub fn probe_line(pins: &[Pin]) -> Option<String> {
    let mut tools: Vec<Tool> = pins.iter().map(|p| p.tool).collect();
    tools.sort();
    tools.dedup();
    (!tools.is_empty()).then(|| {
        tools
            .iter()
            .map(|t| format!("echo '@@{}'; {} 2>&1", t.as_str(), t.probe()))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// Versions found in [`probe_line`]'s output, compared with the pins.
pub fn compare(pins: &[Pin], output: &str) -> Vec<Version> {
    let mut found: Vec<(Tool, String)> = Vec::new();
    let mut current = None;
    for line in output.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("@@") {
            current = Tool::from_name(name.trim());
        } else if let Some(t) = current
            && !found.iter().any(|(f, _)| *f == t)
            && let Some(v) = parse_version(line)
        {
            found.push((t, v));
        }
    }
    pins.iter()
        .map(|p| {
            let f = found
                .iter()
                .find(|(t, _)| *t == p.tool)
                .map(|(_, v)| v.clone());
            Version {
                tool: p.tool,
                expected: p.version.clone(),
                source: p.source.clone(),
                ok: f.as_deref().and_then(|f| matches(&p.version, f)),
                found: f,
            }
        })
        .collect()
}

/// What to do when a pinned version does not match (`[env] on_mismatch`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OnMismatch {
    #[default]
    Stop,
    Warn,
}

/// The environment of one agent session.
#[derive(Debug)]
pub struct Session {
    pub detected: Detected,
    pub on_mismatch: OnMismatch,
    /// Commands are wrapped (`false` when the tool is missing or the Docker sandbox is the env).
    pub wrap: bool,
    /// The environment's program is not installed.
    pub tool_missing: bool,
    /// Why no wrapper applies, for the model (English).
    pub reason: Option<String>,
    checked: OnceLock<Vec<Version>>,
    warned: AtomicBool,
}

impl Session {
    /// `None` when the project has nothing to honor.
    pub fn new(
        detected: Detected,
        on_mismatch: OnMismatch,
        sandbox: SandboxKind,
        has: &dyn Fn(&str) -> bool,
    ) -> Option<Self> {
        if detected.kind.is_none() && detected.pins.is_empty() {
            return None;
        }
        let mut wrap = detected.kind.is_some();
        let mut tool_missing = false;
        let mut reason = None;
        if let Some(k) = &detected.kind {
            if sandbox == SandboxKind::Docker {
                wrap = false;
                reason = Some(format!(
                    "the Docker sandbox image is the environment; {} is not used",
                    k.name()
                ));
            } else if matches!(k, Kind::Compose { service: None, .. }) {
                wrap = false;
                tool_missing = true;
                reason =
                    Some("docker-compose.dev.yml names no service; set [env] service".to_owned());
            } else if *k != Kind::Asdf && !has(k.program()) {
                wrap = false;
                tool_missing = true;
                reason = Some(format!(
                    "this project runs its commands with {} but `{}` is not installed (install: {})",
                    k.name(),
                    k.program(),
                    k.install_hint()
                ));
            }
        }
        Some(Self {
            detected,
            on_mismatch,
            wrap,
            tool_missing,
            reason,
            checked: OnceLock::new(),
            warned: AtomicBool::new(false),
        })
    }

    /// Program and arguments that run `command` with `shell` inside the environment.
    pub fn command(&self, shell: &Shell, root: &Path, command: &str) -> (String, Vec<String>) {
        let host = || {
            let mut a = shell.prefix.clone();
            a.push(command.to_owned());
            a
        };
        let wrapped = |program: &str, mut pre: Vec<String>| {
            pre.push(shell.program.clone());
            pre.extend(host());
            (program.to_owned(), pre)
        };
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        match self.detected.kind.as_ref().filter(|_| self.wrap) {
            None | Some(Kind::Asdf) => (shell.program.clone(), host()),
            Some(Kind::Devcontainer) => {
                let mut a = s(&["exec", "--workspace-folder"]);
                a.push(root.to_string_lossy().into_owned());
                a.extend(s(&["sh", "-c", command]));
                ("devcontainer".into(), a)
            }
            Some(Kind::Compose { file, service }) => {
                let mut a = s(&["compose", "-f", file, "exec", "-T"]);
                a.push(service.clone().unwrap_or_default());
                a.extend(s(&["sh", "-c", command]));
                ("docker".into(), a)
            }
            Some(Kind::Nix { flake: true }) => wrapped("nix", s(&["develop", "--command"])),
            Some(Kind::Nix { flake: false }) => ("nix-shell".into(), s(&["--run", command])),
            Some(Kind::Mise) => wrapped("mise", s(&["exec", "--"])),
        }
    }

    /// The sandbox a wrapped command runs under: none for container environments (the runtime
    /// makes those commands ask), else the session's.
    pub fn sandbox_for(&self, sandbox: SandboxKind) -> SandboxKind {
        match &self.detected.kind {
            Some(k) if self.wrap && k.container() => SandboxKind::None,
            _ => sandbox,
        }
    }

    /// Run the version probe once, inside the environment and under the same sandbox as the
    /// commands, so the versions found are the ones commands will see.
    pub async fn check(&self, ctx: &crate::ToolCtx, shell: &Shell) -> &[Version] {
        if let Some(v) = self.checked() {
            return v;
        }
        let pins = &self.detected.pins;
        let Some(line) = probe_line(pins) else {
            return self.set_checked(Vec::new());
        };
        let (program, args) = self.command(shell, &ctx.root, &line);
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).current_dir(&ctx.root);
        for v in &ctx.scrub_env {
            cmd.env_remove(v);
        }
        cmd.envs(ctx.env.iter().map(|(k, v)| (k, v)));
        let kind = self.sandbox_for(ctx.sandbox);
        let out = match kor_sandbox::spawn(kind, &ctx.sandbox_policy(), &cmd) {
            Ok(child) => tokio::time::timeout(
                std::time::Duration::from_secs(120),
                child.wait_with_output(),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .map(|o| {
                format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default(),
            Err(_) => String::new(),
        };
        self.set_checked(compare(pins, &out))
    }

    /// Versions checked so far (after the first command).
    pub fn checked(&self) -> Option<&[Version]> {
        self.checked.get().map(Vec::as_slice)
    }

    /// Store the result of the version probe (first one wins).
    pub fn set_checked(&self, v: Vec<Version>) -> &[Version] {
        self.checked.get_or_init(|| v)
    }

    /// Problems with this environment as one message (English, for the model and the user), or
    /// `None` when everything matches.
    pub fn problems(&self) -> Option<String> {
        let bad: Vec<&Version> = self
            .checked()
            .unwrap_or_default()
            .iter()
            .filter(|v| v.bad())
            .collect();
        if bad.is_empty() && !self.tool_missing {
            return None;
        }
        let mut m = String::new();
        if let Some(r) = &self.reason
            && self.tool_missing
        {
            let _ = writeln!(m, "Environment: {r}.");
        }
        if self.tool_missing && bad.is_empty() && !self.detected.pins.is_empty() {
            m.push_str("Expected versions:\n");
            for p in &self.detected.pins {
                let _ = writeln!(
                    m,
                    "- {}: {} ({}); install: {}",
                    p.tool.as_str(),
                    p.version,
                    p.source,
                    p.tool.install_hint(&p.version)
                );
            }
        }
        if !bad.is_empty() {
            m.push_str("The project pins versions this environment does not provide:\n");
            for v in &bad {
                let found = v.found.as_deref().unwrap_or("not found");
                let _ = writeln!(
                    m,
                    "- {}: expected {} ({}), found {found}; install: {}",
                    v.tool.as_str(),
                    v.expected,
                    v.source,
                    v.tool.install_hint(&v.expected)
                );
            }
        }
        Some(m)
    }

    /// Before a command: `Err` refuses it (stop), `Ok(Some)` is a one-time warning to append
    /// (warn), `Ok(None)` runs it as is. Call after [`Session::set_checked`].
    ///
    /// # Errors
    /// The refusal message when `on_mismatch = "stop"` and something does not match.
    pub fn gate(&self) -> Result<Option<String>, String> {
        let Some(p) = self.problems() else {
            return Ok(None);
        };
        match self.on_mismatch {
            OnMismatch::Stop => Err(format!(
                "Kor did not run this command.\n{p}Install the expected versions, or set [env] on_mismatch = \"warn\". \
                 Do not work around this check; tell the user what is missing."
            )),
            OnMismatch::Warn if !self.warned.swap(true, Ordering::Relaxed) => Ok(Some(format!(
                "[env] warning (on_mismatch = \"warn\"):\n{p}"
            ))),
            OnMismatch::Warn => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let p = dir.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        dir
    }

    fn shell() -> Shell {
        Shell {
            name: "bash",
            program: "bash".into(),
            prefix: vec!["-c".into()],
        }
    }

    #[test]
    fn detects_environments_in_priority_order_and_reads_pins() {
        let all = |_: &str| true;
        let p = project(&[
            (".devcontainer/devcontainer.json", "{ // jsonc\n}"),
            ("flake.nix", "{}"),
            (".nvmrc", "v20.11.0\n"),
            ("rust-toolchain.toml", "[toolchain]\nchannel = \"1.82.0\"\n"),
            (
                ".tool-versions",
                "# c\nnodejs 20.11.0\npython 3.12.1\ngolang 1.22.0\n",
            ),
            (
                "mise.toml",
                "[env]\nX = \"1\"\n[tools]\nnode = \"20\"\ngo = [\"1.22\"]\n",
            ),
        ]);
        let d = detect_with(p.path(), "", &all);
        assert_eq!(d.kind, Some(Kind::Devcontainer));
        let pins: Vec<(Tool, &str, &str)> = d
            .pins
            .iter()
            .map(|p| (p.tool, p.version.as_str(), p.source.as_str()))
            .collect();
        assert_eq!(
            pins,
            [
                (Tool::Node, "v20.11.0", ".nvmrc"),
                (Tool::Rust, "1.82.0", "rust-toolchain.toml"),
                (Tool::Node, "20.11.0", ".tool-versions"),
                (Tool::Python, "3.12.1", ".tool-versions"),
                (Tool::Go, "1.22.0", ".tool-versions"),
                (Tool::Node, "20", "mise.toml"),
                (Tool::Go, "1.22", "mise.toml"),
            ]
        );

        let p = project(&[
            (
                "docker-compose.dev.yml",
                "version: '3'\nservices:\n  # db first? no\n  app:\n    image: x\n  db:\n    image: y\n",
            ),
            ("Dockerfile.dev", "FROM x"),
        ]);
        let d = detect_with(p.path(), "", &all);
        assert_eq!(
            d.kind,
            Some(Kind::Compose {
                file: "docker-compose.dev.yml".into(),
                service: Some("app".into())
            })
        );
        assert!(!d.dockerfile_dev);
        assert_eq!(
            detect_with(p.path(), "db", &all).kind,
            Some(Kind::Compose {
                file: "docker-compose.dev.yml".into(),
                service: Some("db".into())
            })
        );

        let p = project(&[("shell.nix", "{}"), ("Dockerfile.dev", "FROM x")]);
        let d = detect_with(p.path(), "", &all);
        assert_eq!(d.kind, Some(Kind::Nix { flake: false }));
        assert!(d.dockerfile_dev);

        let p = project(&[(".tool-versions", "nodejs 18\n")]);
        assert_eq!(
            detect_with(p.path(), "", &|n| n == "asdf").kind,
            Some(Kind::Asdf)
        );
        assert_eq!(detect_with(p.path(), "", &|_| false).kind, Some(Kind::Mise));
        assert!(detect_with(project(&[]).path(), "", &all).is_empty());
    }

    #[test]
    fn versions_parse_and_match_by_prefix() {
        assert_eq!(parse_version("v20.11.1").as_deref(), Some("20.11.1"));
        assert_eq!(
            parse_version("rustc 1.82.0 (f6e511eec 2024-10-15)").as_deref(),
            Some("1.82.0")
        );
        assert_eq!(
            parse_version("go version go1.22.0 linux/amd64").as_deref(),
            Some("1.22.0")
        );
        assert_eq!(parse_version("Python 3.12.1").as_deref(), Some("3.12.1"));
        assert_eq!(
            parse_version("The term 'node' is not recognized. At line:1 char:15"),
            None
        );
        assert_eq!(matches("20", "20.11.1"), Some(true));
        assert_eq!(matches("v20.11.0", "20.11.1"), Some(false));
        assert_eq!(matches("1.82.0", "1.82.0"), Some(true));
        assert_eq!(matches("3.12", "3.11.9"), Some(false));
        assert_eq!(matches("lts/*", "20.1.0"), None);
        assert_eq!(matches("stable", "1.82.0"), None);
    }

    #[test]
    fn probe_output_is_compared_with_the_pins() {
        let pins = vec![
            Pin {
                tool: Tool::Node,
                version: "20".into(),
                source: ".nvmrc".into(),
            },
            Pin {
                tool: Tool::Rust,
                version: "1.82.0".into(),
                source: "rust-toolchain.toml".into(),
            },
            Pin {
                tool: Tool::Go,
                version: "1.22".into(),
                source: ".tool-versions".into(),
            },
        ];
        let line = probe_line(&pins).unwrap();
        assert!(
            line.contains("echo '@@node'; node --version 2>&1"),
            "{line}"
        );
        let out =
            "@@go\ngo: command not found\n@@node\nv18.19.0\n@@rust\nrustc 1.82.0 (x 2024-10-15)\n";
        let v = compare(&pins, out);
        assert_eq!(v[0].found.as_deref(), Some("18.19.0"));
        assert!(v[0].bad());
        assert_eq!(v[1].ok, Some(true));
        assert!(v[2].found.is_none() && v[2].bad());
    }

    #[test]
    #[allow(clippy::many_single_char_names, clippy::too_many_lines)] // one scenario table; p(rogram), a(rgs), s(ession)
    fn session_wraps_gates_and_composes_with_the_sandbox() {
        let all = |_: &str| true;
        let root = Path::new("/w/p");
        let det = |kind| Detected {
            files: vec!["x".into()],
            kind,
            pins: vec![Pin {
                tool: Tool::Node,
                version: "20".into(),
                source: ".nvmrc".into(),
            }],
            dockerfile_dev: false,
        };

        let s = Session::new(
            det(Some(Kind::Nix { flake: true })),
            OnMismatch::Stop,
            SandboxKind::Bubblewrap,
            &all,
        )
        .unwrap();
        let (p, a) = s.command(&shell(), root, "cargo test");
        assert_eq!(
            (p.as_str(), a),
            (
                "nix",
                vec![
                    "develop".into(),
                    "--command".into(),
                    "bash".into(),
                    "-c".into(),
                    "cargo test".into()
                ]
            )
        );
        assert_eq!(
            s.sandbox_for(SandboxKind::Bubblewrap),
            SandboxKind::Bubblewrap
        );

        let s = Session::new(
            det(Some(Kind::Devcontainer)),
            OnMismatch::Stop,
            SandboxKind::AppContainer,
            &all,
        )
        .unwrap();
        let (p, a) = s.command(&shell(), root, "npm test");
        assert_eq!(p, "devcontainer");
        assert_eq!(a[..2], ["exec", "--workspace-folder"]);
        assert_eq!(a[3..], ["sh", "-c", "npm test"]);
        assert_eq!(s.sandbox_for(SandboxKind::AppContainer), SandboxKind::None);

        let compose = Kind::Compose {
            file: "docker-compose.dev.yml".into(),
            service: Some("app".into()),
        };
        let s = Session::new(
            det(Some(compose)),
            OnMismatch::Stop,
            SandboxKind::None,
            &all,
        )
        .unwrap();
        assert_eq!(
            s.command(&shell(), root, "ls").1.join(" "),
            "compose -f docker-compose.dev.yml exec -T app sh -c ls"
        );

        // Docker sandbox: the image is the environment.
        let s = Session::new(
            det(Some(Kind::Mise)),
            OnMismatch::Stop,
            SandboxKind::Docker,
            &all,
        )
        .unwrap();
        assert!(!s.wrap);
        assert_eq!(s.command(&shell(), root, "ls").0, "bash");

        // Missing tool: stop refuses with expected versions and an install hint.
        let s = Session::new(
            det(Some(Kind::Nix { flake: true })),
            OnMismatch::Stop,
            SandboxKind::None,
            &|_| false,
        )
        .unwrap();
        assert!(s.tool_missing && !s.wrap);
        let e = s.gate().unwrap_err();
        assert!(
            e.contains("`nix` is not installed")
                && e.contains("node: 20 (.nvmrc)")
                && e.contains("nixos.org"),
            "{e}"
        );

        // Mismatch: stop refuses, warn warns once.
        for (mode, refused) in [(OnMismatch::Stop, true), (OnMismatch::Warn, false)] {
            let s = Session::new(det(None), mode, SandboxKind::None, &all).unwrap();
            s.set_checked(compare(&s.detected.pins, "@@node\nv18.0.0\n"));
            let g = s.gate();
            assert_eq!(g.is_err(), refused);
            if !refused {
                assert!(
                    g.unwrap()
                        .unwrap()
                        .contains("expected 20 (.nvmrc), found 18.0.0")
                );
                assert_eq!(s.gate(), Ok(None));
            }
        }
        let s = Session::new(det(None), OnMismatch::Stop, SandboxKind::None, &all).unwrap();
        s.set_checked(compare(&s.detected.pins, "@@node\nv20.3.1\n"));
        assert_eq!(s.gate(), Ok(None));
        assert!(
            Session::new(
                Detected::default(),
                OnMismatch::Stop,
                SandboxKind::None,
                &all
            )
            .is_none()
        );
    }
}
