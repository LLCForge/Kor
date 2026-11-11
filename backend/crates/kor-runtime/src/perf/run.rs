//! Profiler selection, running (environment wrapper + sandbox, ADR 0043 / 0033), stored
//! reports (`.kor/perf/<id>/`), graph mapping, before/after comparison and the optimization
//! prompt.

use kor_platform::NoWindow as _;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kor_i18n::{FluentArgs, Localizer};
use kor_sandbox::SandboxKind;
use kor_tools::Shell;
use kor_tools::devenv::{self, Session};
use serde::{Deserialize, Serialize};

use super::load::{self, LoadStats};
use super::{Frame, Profile, Unit, flame, parse, relative_to, short_name};

// ---------------------------------------------------------------------------------------------
// Languages and profilers
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lang {
    Rust,
    Go,
    Node,
    Python,
    Jvm,
    Dotnet,
    Native,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Node => "node",
            Self::Python => "python",
            Self::Jvm => "jvm",
            Self::Dotnet => "dotnet",
            Self::Native => "native",
        }
    }

    /// From the program a command starts with.
    pub fn of_command(cmd: &str) -> Option<Self> {
        let first = cmd.split_whitespace().next()?;
        let prog = first.rsplit(['/', '\\']).next().unwrap_or(first).to_ascii_lowercase();
        let prog = prog.trim_end_matches(".exe").trim_end_matches(".cmd");
        Some(match prog {
            "cargo" => Self::Rust,
            "go" => Self::Go,
            "node" | "npm" | "npx" | "yarn" | "pnpm" => Self::Node,
            "python" | "python3" | "py" | "pytest" | "uv" | "poetry" => Self::Python,
            "java" | "mvn" | "mvnw" | "gradle" | "gradlew" => Self::Jvm,
            "dotnet" => Self::Dotnet,
            _ if first.replace('\\', "/").contains("target/release/") || first.replace('\\', "/").contains("target/debug/") => Self::Rust,
            _ => return None,
        })
    }

    /// From the project's manifest files.
    pub fn of_project(root: &Path) -> Option<Self> {
        let has = |f: &str| root.join(f).is_file();
        let ext = |e: &str| {
            std::fs::read_dir(root).into_iter().flatten().flatten().any(|d| d.path().extension().is_some_and(|x| x.eq_ignore_ascii_case(e)))
        };
        if has("Cargo.toml") {
            Some(Self::Rust)
        } else if has("go.mod") {
            Some(Self::Go)
        } else if has("package.json") {
            Some(Self::Node)
        } else if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
            Some(Self::Python)
        } else if has("pom.xml") || has("build.gradle") || has("build.gradle.kts") {
            Some(Self::Jvm)
        } else if ext("csproj") || ext("sln") || ext("fsproj") {
            Some(Self::Dotnet)
        } else if has("CMakeLists.txt") || has("Makefile") {
            Some(Self::Native)
        } else {
            None
        }
    }

    /// `kor perf test`: the project's usual test command.
    pub fn test_command(self, root: &Path) -> Option<String> {
        Some(match self {
            Self::Rust => "cargo test".into(),
            Self::Go => "go test .".into(),
            Self::Node => "npm test".into(),
            Self::Python => format!("{} -m pytest", kor_platform::python_command()),
            Self::Jvm if root.join("pom.xml").is_file() => "mvn test".into(),
            Self::Jvm if root.join("gradlew").is_file() => "./gradlew test".into(),
            Self::Jvm => "gradle test".into(),
            Self::Dotnet => "dotnet test".into(),
            Self::Native => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profiler {
    Samply,
    Perf,
    Pprof,
    Node,
    PySpy,
    CProfile,
    AsyncProfiler,
    DotnetTrace,
    Sample,
}

pub const PROFILERS: &[Profiler] = &[
    Profiler::Samply,
    Profiler::Perf,
    Profiler::Pprof,
    Profiler::Node,
    Profiler::PySpy,
    Profiler::CProfile,
    Profiler::AsyncProfiler,
    Profiler::DotnetTrace,
    Profiler::Sample,
];

impl Profiler {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Samply => "samply",
            Self::Perf => "perf",
            Self::Pprof => "pprof",
            Self::Node => "node",
            Self::PySpy => "py-spy",
            Self::CProfile => "cprofile",
            Self::AsyncProfiler => "async-profiler",
            Self::DotnetTrace => "dotnet-trace",
            Self::Sample => "sample",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        PROFILERS.iter().copied().find(|p| p.as_str() == s).or(match s.as_str() {
            "go" | "go-pprof" => Some(Self::Pprof),
            "pyspy" => Some(Self::PySpy),
            "asprof" => Some(Self::AsyncProfiler),
            _ => None,
        })
    }

    /// The program Kor looks for on `PATH` (`None`: built into the toolchain).
    pub fn program(self) -> Option<&'static str> {
        match self {
            Self::Samply => Some("samply"),
            Self::Perf => Some("perf"),
            Self::PySpy => Some("py-spy"),
            Self::AsyncProfiler => Some("asprof"),
            Self::DotnetTrace => Some("dotnet-trace"),
            Self::Sample => Some("sample"),
            Self::Pprof | Self::Node | Self::CProfile => None,
        }
    }

    /// Samples another process from outside (ptrace, `perf_event`, ETW, task ports, the .NET
    /// diagnostics port): cannot run inside the sandbox. `attach` is a run against a pid.
    pub fn privileged(self, attach: bool) -> bool {
        match self {
            Self::Pprof | Self::Node | Self::CProfile => false,
            // Launched as a JVM agent it runs inside the program; asprof attaches.
            Self::AsyncProfiler => attach,
            _ => true,
        }
    }

    /// Install instructions. Kor never installs a profiler on its own.
    pub fn install_hint(self) -> &'static str {
        match self {
            Self::Samply => "cargo install --locked samply (or a release binary from https://github.com/mstange/samply/releases)",
            Self::Perf => "your distribution's perf package (Debian/Ubuntu: apt install linux-tools-common linux-tools-$(uname -r); Fedora: dnf install perf)",
            Self::Pprof => "part of the Go toolchain (https://go.dev/dl/)",
            Self::Node => "built into Node.js (https://nodejs.org/)",
            Self::PySpy => "pip install py-spy (or cargo install py-spy)",
            Self::CProfile => "built into Python",
            Self::AsyncProfiler => "a release from https://github.com/async-profiler/async-profiler/releases; put asprof on PATH and set ASYNC_PROFILER_HOME",
            Self::DotnetTrace => "dotnet tool install --global dotnet-trace",
            Self::Sample => "part of macOS",
        }
    }

    pub fn supports(self, lang: Lang) -> bool {
        use Lang::{Dotnet, Go, Jvm, Native, Node, Python, Rust};
        match self {
            Self::Samply | Self::Perf | Self::Sample => matches!(lang, Rust | Native | Go),
            Self::Pprof => lang == Go,
            Self::Node => lang == Node,
            Self::PySpy | Self::CProfile => lang == Python,
            Self::AsyncProfiler => lang == Jvm,
            Self::DotnetTrace => lang == Dotnet,
        }
    }

    /// Also records memory / allocations in the same run.
    pub fn memory(self) -> bool {
        matches!(self, Self::Pprof | Self::Node)
    }
}

fn is_python(cmd: &str) -> bool {
    matches!(Lang::of_command(cmd), Some(Lang::Python))
        && cmd.split_whitespace().next().is_some_and(|w| {
            let w = w.rsplit(['/', '\\']).next().unwrap_or(w).to_ascii_lowercase();
            matches!(w.trim_end_matches(".exe"), "python" | "python3" | "py")
        })
}

fn is_go_test(cmd: &str) -> bool {
    let mut w = cmd.split_whitespace();
    matches!((w.next(), w.next()), (Some("go" | "go.exe"), Some("test")))
}

fn async_profiler_lib() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("ASYNC_PROFILER_HOME")?);
    ["libasyncProfiler.so", "libasyncProfiler.dylib"]
        .iter()
        .map(|n| home.join("lib").join(n))
        .find(|p| p.is_file())
}

/// Why no profiler could be chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoProfiler {
    /// This one would be used; its program is not installed.
    Missing(Profiler),
    /// Nothing Kor can drive for this language and mode on this OS.
    Unsupported,
}

/// Pick the profiler for `lang`. `attach`: sample a running process by pid; `has`: is a
/// program on PATH.
pub fn choose(lang: Lang, cmd: Option<&str>, attach: bool, os: kor_platform::Os, has: &dyn Fn(&str) -> bool) -> Result<Profiler, NoProfiler> {
    use kor_platform::Os;
    let ok = |p: Profiler| if p.program().is_none_or(has) { Ok(p) } else { Err(NoProfiler::Missing(p)) };
    let cmd = cmd.unwrap_or("");
    match (lang, attach) {
        (Lang::Go, true) => Ok(Profiler::Pprof),
        (Lang::Go, false) if is_go_test(cmd) => Ok(Profiler::Pprof),
        (Lang::Rust | Lang::Native | Lang::Go, false) => {
            if has("samply") {
                Ok(Profiler::Samply)
            } else if os == Os::Linux && has("perf") {
                Ok(Profiler::Perf)
            } else {
                Err(NoProfiler::Missing(Profiler::Samply))
            }
        }
        (Lang::Rust | Lang::Native, true) => match os {
            Os::Linux => ok(Profiler::Perf),
            Os::Mac => ok(Profiler::Sample),
            _ => Err(NoProfiler::Unsupported),
        },
        (Lang::Node, _) => Ok(Profiler::Node),
        (Lang::Python, false) if !has("py-spy") && is_python(cmd) => Ok(Profiler::CProfile),
        (Lang::Python, _) => ok(Profiler::PySpy),
        (Lang::Jvm, true) => ok(Profiler::AsyncProfiler),
        (Lang::Jvm, false) if os != Os::Windows && async_profiler_lib().is_some() => Ok(Profiler::AsyncProfiler),
        (Lang::Jvm, false) => Err(if os == Os::Windows { NoProfiler::Unsupported } else { NoProfiler::Missing(Profiler::AsyncProfiler) }),
        (Lang::Dotnet, _) => ok(Profiler::DotnetTrace),
    }
}

// ---------------------------------------------------------------------------------------------
// Options, plan
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    Command,
    Test,
    Endpoint,
    /// `kor perf report <file>`: an existing profile, nothing run.
    File,
}

/// What `kor perf` / desktop `perf.run` asked for.
#[derive(Debug, Clone)]
pub struct Options {
    pub root: PathBuf,
    /// A command, `test`, or an `http(s)://` URL.
    pub target: String,
    pub profiler: Option<String>,
    /// Endpoint: sampling window (default 10 s). Command: time limit (default 600 s).
    pub duration: Option<u64>,
    pub requests: u32,
    pub concurrency: u32,
    pub pid: Option<u32>,
    /// Endpoint: start the server with this command (Kor stops it afterwards).
    pub server_cmd: Option<String>,
    pub allow_hosts: Vec<String>,
    /// Compare with this earlier run.
    pub baseline: Option<String>,
    pub lang: kor_i18n::Lang,
}

impl Options {
    pub fn new(root: PathBuf, target: impl Into<String>, lang: kor_i18n::Lang) -> Self {
        Self {
            root,
            target: target.into(),
            profiler: None,
            duration: None,
            requests: 200,
            concurrency: 10,
            pid: None,
            server_cmd: None,
            allow_hosts: Vec::new(),
            baseline: None,
            lang,
        }
    }
}

/// A checked run, ready for [`execute`]. When `ask` is set the run leaves the sandbox and the
/// front end must ask the user first.
#[derive(Debug)]
pub struct Plan {
    pub root: PathBuf,
    pub target: String,
    pub kind: TargetKind,
    pub command: Option<String>,
    pub url: Option<String>,
    pub lang: Lang,
    pub profiler: Profiler,
    /// The profiler program as invoked (tests point it at a shim).
    pub program: String,
    pub pid: Option<u32>,
    pub server_cmd: Option<String>,
    pub requests: u32,
    pub concurrency: u32,
    pub duration: Duration,
    pub allow_hosts: Vec<String>,
    /// The sandbox the profiled command runs in (`None` = unsandboxed).
    pub sandbox: SandboxKind,
    /// Localized question when the run would leave an enabled sandbox.
    pub ask: Option<String>,
    pub baseline: Option<String>,
    pub ui: kor_i18n::Lang,
    setup: crate::sandbox::Setup,
    env: Option<Session>,
    notes: Vec<String>,
}

type Note = (&'static str, Vec<(&'static str, String)>);

fn fargs(a: &[(&'static str, String)]) -> FluentArgs<'static> {
    let mut f = FluentArgs::new();
    for (k, v) in a {
        f.set(*k, v.clone());
    }
    f
}

fn tr(ui: kor_i18n::Lang, key: &str, a: &[(&'static str, String)]) -> String {
    Localizer::new(ui).t_args(key, &fargs(a))
}

fn missing(l: &Localizer, p: Profiler) -> String {
    let mut m = l.t_args(
        "perf-missing",
        &fargs(&[("profiler", p.as_str().into()), ("program", p.program().unwrap_or(p.as_str()).into()), ("hint", p.install_hint().into())]),
    );
    if kor_platform::os() == kor_platform::Os::Windows && matches!(p, Profiler::Samply) {
        m.push('\n');
        m.push_str(&l.t("perf-etw-hint"));
    }
    m
}

/// Check `opts` against the project and this machine: language, profiler (installed?),
/// sandbox and environment. Errors are localized.
#[allow(clippy::too_many_lines)] // one linear check of the request
pub fn plan(opts: &Options, config: &kor_config::Config) -> Result<Plan, String> {
    let l = Localizer::new(opts.lang);
    let root = opts.root.clone();
    let target = opts.target.trim().to_owned();
    let lower = target.to_ascii_lowercase();
    let endpoint = lower.starts_with("http://") || lower.starts_with("https://");
    let explicit = match opts.profiler.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(Profiler::parse(p).ok_or_else(|| {
            l.t_args(
                "perf-profiler-unknown",
                &fargs(&[("name", p.to_owned()), ("list", PROFILERS.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "))]),
            )
        })?),
        None => None,
    };
    let profiler_lang = |p: Profiler| match p {
        Profiler::Samply | Profiler::Perf | Profiler::Sample => Lang::Native,
        Profiler::Pprof => Lang::Go,
        Profiler::Node => Lang::Node,
        Profiler::PySpy | Profiler::CProfile => Lang::Python,
        Profiler::AsyncProfiler => Lang::Jvm,
        Profiler::DotnetTrace => Lang::Dotnet,
    };
    let (kind, command, url) = if endpoint {
        if opts.pid.is_none() && opts.server_cmd.as_deref().is_none_or(|c| c.trim().is_empty()) {
            return Err(l.t("perf-endpoint-needs-server"));
        }
        load::check_target(&target, &opts.allow_hosts)?;
        (TargetKind::Endpoint, None, Some(target.clone()))
    } else if lower == "test" {
        (TargetKind::Test, None, None)
    } else if target.is_empty() {
        return Err(l.t("perf-target-missing"));
    } else {
        (TargetKind::Command, Some(target.clone()), None)
    };
    let source = if endpoint { opts.server_cmd.clone() } else { command.clone() };
    let lang = source
        .as_deref()
        .and_then(Lang::of_command)
        .or_else(|| Lang::of_project(&root))
        .or_else(|| explicit.map(profiler_lang))
        .ok_or_else(|| l.t_args("perf-lang-unknown", &fargs(&[("target", target.clone())])))?;
    let command = match kind {
        TargetKind::Test => Some(lang.test_command(&root).ok_or_else(|| l.t_args("perf-test-unknown", &fargs(&[("lang", lang.as_str().into())])))?),
        _ => command,
    };
    // Node servers are started with --cpu-prof (Kor needs --cmd); everything else attaches.
    let attach = endpoint && lang != Lang::Node;
    if endpoint && lang == Lang::Node && opts.server_cmd.is_none() {
        return Err(l.t("perf-node-needs-cmd"));
    }
    let has = |p: &str| devenv::on_path(p);
    let os = kor_platform::os();
    let profiler = match explicit {
        Some(p) => {
            if !p.supports(lang) {
                return Err(l.t_args("perf-profiler-mismatch", &fargs(&[("profiler", p.as_str().into()), ("lang", lang.as_str().into())])));
            }
            p
        }
        None => choose(lang, command.as_deref(), attach, os, &has).map_err(|e| match e {
            NoProfiler::Missing(p) => missing(&l, p),
            NoProfiler::Unsupported => {
                let mut m = l.t_args("perf-unsupported", &fargs(&[("lang", lang.as_str().into())]));
                if os == kor_platform::Os::Windows {
                    m.push('\n');
                    m.push_str(&l.t("perf-etw-hint"));
                }
                m
            }
        })?,
    };
    // What each profiler can do with this target.
    let cmd = command.as_deref().unwrap_or("");
    match profiler {
        Profiler::Pprof if !endpoint && !is_go_test(cmd) => return Err(l.t("perf-pprof-go-test")),
        Profiler::CProfile if endpoint || !is_python(cmd) => return Err(l.t("perf-cprofile-python")),
        Profiler::Sample if !attach => return Err(l.t("perf-sample-attach")),
        Profiler::Samply if attach => return Err(l.t_args("perf-unsupported", &fargs(&[("lang", lang.as_str().into())]))),
        Profiler::AsyncProfiler if !attach && async_profiler_lib().is_none() => return Err(missing(&l, profiler)),
        Profiler::Perf if os != kor_platform::Os::Linux => return Err(l.t_args("perf-unsupported", &fargs(&[("lang", lang.as_str().into())]))),
        _ => {}
    }
    if let Some(prog) = profiler.program()
        && (attach || profiler != Profiler::AsyncProfiler)
        && !has(prog)
    {
        return Err(missing(&l, profiler));
    }

    let setup = crate::sandbox::setup(&l, &config.sandbox, crate::offline_mode());
    let (env, mut notes) = crate::devenv::session(&l, &config.env, &root, setup.kind);
    let reason = if endpoint {
        Some(l.t("perf-reason-server"))
    } else if profiler.privileged(attach) {
        Some(l.t("perf-reason-ptrace"))
    } else if setup.kind == SandboxKind::Docker {
        Some(l.t("perf-reason-docker"))
    } else if env.as_ref().is_some_and(|e| crate::devenv::asks(e, setup.kind)) {
        Some(l.t("perf-reason-container"))
    } else {
        None
    };
    let sandbox = match (&reason, &env) {
        (Some(_), _) => SandboxKind::None,
        (None, Some(e)) => e.sandbox_for(setup.kind),
        (None, None) => setup.kind,
    };
    let ask = reason.filter(|_| config.sandbox.mode != kor_config::SandboxMode::Off).map(|r| {
        l.t_args("perf-ask-unsandboxed", &fargs(&[("profiler", profiler.as_str().into()), ("reason", r)]))
    });
    if endpoint && opts.server_cmd.is_some() && env.as_ref().is_some_and(|e| e.wrap) {
        notes.push(l.t("perf-note-cmd-no-env"));
    }
    let default = if endpoint { 10 } else { 600 };
    Ok(Plan {
        root,
        target,
        kind,
        command,
        url,
        lang,
        profiler,
        program: profiler.program().unwrap_or("").to_owned(),
        pid: opts.pid,
        server_cmd: opts.server_cmd.clone().filter(|c| !c.trim().is_empty()),
        requests: opts.requests.clamp(1, load::MAX_REQUESTS),
        concurrency: opts.concurrency.clamp(1, load::MAX_CONCURRENCY),
        duration: Duration::from_secs(opts.duration.unwrap_or(default).clamp(1, 3600)),
        allow_hosts: opts.allow_hosts.clone(),
        sandbox,
        ask,
        baseline: opts.baseline.clone(),
        ui: opts.lang,
        setup,
        env,
        notes,
    })
}

// ---------------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------------

/// Kor's cProfile wrapper: runs a script or `-m module` under cProfile and writes
/// `caller;function µs` collapsed lines (cProfile keeps callers, not whole stacks).
const CPROFILE_PY: &str = r##"import cProfile, pstats, runpy, sys
out = sys.argv[1]
sys.argv = sys.argv[2:]
prof = cProfile.Profile()
def name(k):
    f, l, n = k
    return f"{n} ({f}:{l})".replace(";", ":")
try:
    prof.enable()
    if sys.argv and sys.argv[0] == "-m":
        sys.argv = sys.argv[1:]
        runpy.run_module(sys.argv[0], run_name="__main__", alter_sys=True)
    else:
        runpy.run_path(sys.argv[0], run_name="__main__")
except SystemExit:
    pass
finally:
    prof.disable()
    st = pstats.Stats(prof)
    with open(out, "w", encoding="utf-8") as fh:
        fh.write("# unit=microseconds\n")
        for k, (cc, nc, tt, ct, callers) in st.stats.items():
            if callers:
                for c, v in callers.items():
                    us = int(v[2] * 1e6)
                    if us > 0:
                        fh.write(f"{name(c)};{name(k)} {us}\n")
            elif int(tt * 1e6) > 0:
                fh.write(f"{name(k)} {int(tt * 1e6)}\n")
"##;

/// Lets a Node server started with `--cmd` exit normally (so `--cpu-prof` is written) when
/// Kor creates the stop file; no signals, so it works the same on every OS.
const NODE_STOP_JS: &str = "const fs = require('fs'); const f = process.env.KOR_PERF_STOP;\n\
if (f) { const t = setInterval(() => { if (fs.existsSync(f)) { clearInterval(t); process.exit(0); } }, 200); t.unref(); }\n";

fn powershell(shell: &Shell) -> bool {
    let p = shell.program.to_ascii_lowercase();
    p.contains("powershell") || p.contains("pwsh")
}

/// Single-quote for the shell the line runs in.
fn q(ps: bool, s: &str) -> String {
    if ps { format!("'{}'", s.replace('\'', "''")) } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

/// `--duration` for dotnet-trace: `dd:hh:mm:ss`.
fn dhms(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}:{:02}", secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

fn append_env(name: &str, extra: &str) -> (String, String) {
    let cur = std::env::var(name).unwrap_or_default();
    let v = if cur.trim().is_empty() { extra.to_owned() } else { format!("{cur} {extra}") };
    (name.to_owned(), v)
}

fn node_options(raw: &Path, stop_js: Option<&Path>) -> (String, String) {
    // NODE_OPTIONS treats `\` inside quotes as an escape; Windows accepts `/`.
    let d = raw.to_string_lossy().replace('\\', "/");
    let mut o = format!("--cpu-prof --cpu-prof-dir=\"{d}\" --heap-prof --heap-prof-dir=\"{d}\" --heap-prof-interval=32768");
    if let Some(js) = stop_js {
        let _ = write!(o, " --require \"{}\"", js.to_string_lossy().replace('\\', "/"));
    }
    append_env("NODE_OPTIONS", &o)
}

/// The shell line and extra environment for launching `cmd` under the profiler.
/// `rel` is the raw output folder relative to the project root (works inside containers too),
/// `abs` the same folder for environment variables.
fn launch_line(plan: &Plan, cmd: &str, ps: bool, rel: &str, abs: &Path) -> Result<(String, Vec<(String, String)>), String> {
    let p = &plan.program;
    let o = |f: &str| q(ps, &format!("{rel}/{f}"));
    Ok(match plan.profiler {
        Profiler::Samply => (format!("{p} record --save-only --unstable-presymbolicate -o {} -- {cmd}", o("profile.json")), vec![]),
        Profiler::Perf => (
            format!("{p} record -F 999 -g -o {d} -- {cmd}; {p} script -i {d} > {}", o("perf.txt"), d = o("perf.data")),
            vec![],
        ),
        Profiler::Pprof => {
            let rest = cmd.trim_start().splitn(3, char::is_whitespace).nth(2).unwrap_or("");
            let first = cmd.split_whitespace().next().unwrap_or("go");
            (format!("{first} test -cpuprofile {} -memprofile {} {rest}", o("cpu.pprof"), o("mem.pprof")), vec![])
        }
        Profiler::Node => (cmd.to_owned(), vec![node_options(abs, None)]),
        Profiler::PySpy => (format!("{p} record -f speedscope --subprocesses -o {} -- {cmd}", o("profile.speedscope.json")), vec![]),
        Profiler::CProfile => {
            std::fs::write(abs.join("kor_cprofile.py"), CPROFILE_PY).map_err(|e| e.to_string())?;
            let (first, rest) = cmd.trim_start().split_once(char::is_whitespace).unwrap_or((cmd.trim(), ""));
            (format!("{first} {} {} {rest}", o("kor_cprofile.py"), o("cprofile.collapsed")), vec![])
        }
        Profiler::AsyncProfiler => {
            let lib = async_profiler_lib().ok_or("ASYNC_PROFILER_HOME")?;
            let agent = format!("-agentpath:{}=start,event=cpu,collapsed,file={}/async-%p.collapsed", lib.display(), abs.display());
            (cmd.to_owned(), vec![append_env("JAVA_TOOL_OPTIONS", &agent)])
        }
        Profiler::DotnetTrace => (format!("{p} collect --format Speedscope -o {} -- {cmd}", o("trace.nettrace")), vec![]),
        Profiler::Sample => return Err("sample attaches to a running process".into()),
    })
}

/// The shell line that samples `pid` for `secs` seconds.
fn attach_line(plan: &Plan, pid: u32, secs: u64, ps: bool, rel: &str) -> Option<String> {
    let p = &plan.program;
    let o = |f: &str| q(ps, &format!("{rel}/{f}"));
    Some(match plan.profiler {
        Profiler::Perf => format!("{p} record -F 999 -g -p {pid} -o {d} -- sleep {secs}; {p} script -i {d} > {}", o("perf.txt"), d = o("perf.data")),
        Profiler::PySpy => format!("{p} record -p {pid} -d {secs} -f speedscope -o {}", o("profile.speedscope.json")),
        Profiler::AsyncProfiler => format!("{p} -d {secs} -o collapsed -f {} {pid}", o("async.collapsed")),
        Profiler::DotnetTrace => format!("{p} collect -p {pid} --duration {} --format Speedscope -o {}", dhms(secs), o("trace.nettrace")),
        Profiler::Sample => format!("{p} {pid} {secs} -file {}", o("sample.txt")),
        Profiler::Samply | Profiler::Pprof | Profiler::Node | Profiler::CProfile => return None,
    })
}

/// Words of a command line; `'…'` and `"…"` group.
fn split_words(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quote, mut any) = (Vec::new(), String::new(), None, false);
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            (_, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `name` → a path on PATH (Windows: `.exe`, `.cmd`, `.bat`; `Command` only adds `.exe`).
fn resolve_program(name: &str) -> PathBuf {
    if name.contains(['/', '\\']) {
        return PathBuf::from(name);
    }
    let exts: &[&str] = if kor_platform::os() == kor_platform::Os::Windows { &[".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p).find_map(|d| exts.iter().map(|e| d.join(format!("{name}{e}"))).find(|c| c.is_file()))
        })
        .unwrap_or_else(|| PathBuf::from(name))
}

fn tail(s: &str, max: usize) -> String {
    let start = s.len().saturating_sub(max);
    let start = (start..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(s.len());
    s[start..].trim().to_owned()
}

fn new_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    format!("{}-{:04x}", chrono::Local::now().format("%Y%m%d-%H%M%S"), nanos & 0xffff)
}

pub fn perf_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("perf")
}

fn run_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    let base = perf_dir(root);
    std::fs::create_dir_all(&base).map_err(|e| format!("{}: {e}", base.display()))?;
    let ignore = base.join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(&ignore, "*\n");
    }
    let dir = base.join(id);
    std::fs::create_dir_all(dir.join("raw")).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// Program and arguments for `line` in the project's environment (or the plain shell).
fn wrapped(plan: &Plan, shell: &Shell, line: &str) -> (String, Vec<String>) {
    plan.env.as_ref().map_or_else(
        || {
            let mut a = shell.prefix.clone();
            a.push(line.to_owned());
            (shell.program.clone(), a)
        },
        |e| e.command(shell, &plan.root, line),
    )
}

fn command_for(plan: &Plan, program: &str, args: &[String], extra: &[(String, String)]) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.args(args).current_dir(&plan.root);
    // Provider keys never reach the profiled program.
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(|k| k.ends_with("_API_KEY")) {
            c.env_remove(k);
        }
    }
    if crate::offline_mode() {
        c.envs(crate::OFFLINE_COMMAND_ENV.iter().map(|(k, v)| (*k, *v)));
    }
    c.envs(extra.iter().map(|(k, v)| (k, v)));
    c
}

struct Ran {
    wall_ms: f64,
    exit: Option<i32>,
    output: String,
    timed_out: bool,
}

async fn run_line(plan: &Plan, sandbox: SandboxKind, line: &str, extra: &[(String, String)], limit: Duration, wrap_env: bool) -> Result<Ran, String> {
    let shell = Shell::detect();
    let (program, args) = if wrap_env {
        wrapped(plan, &shell, line)
    } else {
        let mut a = shell.prefix.clone();
        a.push(line.to_owned());
        (shell.program.clone(), a)
    };
    let cmd = command_for(plan, &program, &args, extra);
    let policy = kor_sandbox::Policy {
        root: plan.root.clone(),
        network: plan.setup.network && !crate::offline_mode(),
        options: plan.setup.options.clone(),
    };
    let start = Instant::now();
    let child = kor_sandbox::spawn(sandbox, &policy, &cmd).map_err(|e| format!("{program}: {e}"))?;
    let (out, timed_out) = match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(r) => (Some(r.map_err(|e| e.to_string())?), false),
        Err(_) => (None, true),
    };
    let wall_ms = start.elapsed().as_secs_f64() * 1e3;
    Ok(match out {
        Some(o) => Ran {
            wall_ms,
            exit: o.status.code(),
            output: format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
            timed_out,
        },
        None => Ran { wall_ms, exit: None, output: String::new(), timed_out },
    })
}

/// Shell kind for quoting: `PowerShell` on Windows hosts, `sh` inside container environments.
fn quoting_ps(plan: &Plan) -> bool {
    let container = plan.env.as_ref().is_some_and(|e| e.wrap && e.detected.kind.as_ref().is_some_and(devenv::Kind::container));
    powershell(&Shell::detect()) && !container
}

/// Run the plan, parse what the profiler wrote and store the report. Errors are localized.
#[allow(clippy::too_many_lines)] // one linear flow: run → parse → report
pub async fn execute(plan: Plan) -> Result<Report, String> {
    let ui = plan.ui;
    let id = new_id();
    let dir = run_dir(&plan.root, &id)?;
    let raw = dir.join("raw");
    let rel = format!(".kor/perf/{id}/raw");
    let ps = quoting_ps(&plan);
    let mut notes: Vec<Note> = Vec::new();
    let wall_ms;
    let mut exit = None;
    let mut load_stats = None;
    let mut output = String::new();

    if plan.kind == TargetKind::Endpoint {
        let url = plan.url.clone().unwrap_or_default();
        load::check_target(&url, &plan.allow_hosts)?;
        let stop_file = dir.join("stop");
        let stop_js = dir.join("kor-perf-stop.js");
        let mut server = None;
        if let Some(cmd) = &plan.server_cmd {
            let words = split_words(cmd);
            let Some(first) = words.first() else { return Err(tr(ui, "perf-endpoint-needs-server", &[])) };
            let log = std::fs::File::create(dir.join("server.log")).map_err(|e| e.to_string())?;
            let log2 = log.try_clone().map_err(|e| e.to_string())?;
            let mut c = std::process::Command::new(resolve_program(first));
            c.no_window();
            c.args(&words[1..]).current_dir(&plan.root).stdin(std::process::Stdio::null()).stdout(log).stderr(log2);
            if plan.profiler == Profiler::Node {
                std::fs::write(&stop_js, NODE_STOP_JS).map_err(|e| e.to_string())?;
                let (k, v) = node_options(&raw, Some(&stop_js));
                c.env(k, v).env("KOR_PERF_STOP", &stop_file);
            }
            kor_platform::Detach::detach(&mut c);
            let mut tc = tokio::process::Command::from(c);
            tc.kill_on_drop(true);
            let child = tc.spawn().map_err(|e| format!("{first}: {e}"))?;
            server = Some(child);
            if !load::wait_ready(&url, Duration::from_secs(60)).await {
                if let Some(pid) = server.as_ref().and_then(tokio::process::Child::id) {
                    kor_platform::kill_tree(pid);
                }
                let log = std::fs::read_to_string(dir.join("server.log")).unwrap_or_default();
                return Err(tr(ui, "perf-server-not-ready", &[("url", url), ("log", tail(&log, 2000))]));
            }
        }
        let pid = plan.pid.or_else(|| server.as_ref().and_then(tokio::process::Child::id));
        let secs = plan.duration.as_secs();
        let base = {
            let rest = url.split_once("://").map_or("", |x| x.1);
            let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
            format!("{}://{auth}", url.split_once("://").map_or("http", |x| x.0))
        };
        let profile = async {
            match plan.profiler {
                Profiler::Node => Ok(None),
                Profiler::Pprof => fetch_pprof(&base, secs, &raw, &plan.allow_hosts).await.map(|()| None),
                _ => {
                    let pid = pid.ok_or_else(|| tr(ui, "perf-endpoint-needs-server", &[]))?;
                    let line = attach_line(&plan, pid, secs, ps, &rel).ok_or_else(|| tr(ui, "perf-unsupported", &[("lang", plan.lang.as_str().into())]))?;
                    run_line(&plan, SandboxKind::None, &line, &[], plan.duration + Duration::from_secs(120), false).await.map(Some)
                }
            }
        };
        let traffic = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            load::run(&url, plan.requests, plan.concurrency, &plan.allow_hosts).await
        };
        let (prof, traffic) = tokio::join!(profile, traffic);
        if let Some(mut child) = server {
            if plan.profiler == Profiler::Node {
                let _ = std::fs::write(&stop_file, b"stop");
                let _ = tokio::time::timeout(Duration::from_secs(15), child.wait()).await;
            }
            if let Some(pid) = child.id() {
                kor_platform::kill_tree(pid);
            }
        }
        let stats = traffic?;
        if let Some(r) = prof? {
            output = r.output;
        }
        wall_ms = Some(stats.wall_ms);
        if plan.profiler != Profiler::Node {
            notes.push(("perf-note-window", vec![("secs", secs.to_string()), ("ms", format!("{:.0}", stats.wall_ms))]));
        }
        load_stats = Some(stats);
    } else {
        let cmd = plan.command.clone().unwrap_or_default();
        let (line, extra) = launch_line(&plan, &cmd, ps, &rel, &raw)?;
        let r = run_line(&plan, plan.sandbox, &line, &extra, plan.duration, true).await?;
        if r.timed_out {
            notes.push(("perf-note-timeout", vec![("secs", plan.duration.as_secs().to_string())]));
        }
        if let Some(c) = r.exit.filter(|c| *c != 0) {
            notes.push(("perf-note-exit", vec![("code", c.to_string())]));
        }
        wall_ms = Some(r.wall_ms);
        exit = r.exit;
        output = r.output;
        notes.push(("perf-note-wall", vec![]));
    }
    let _ = std::fs::write(dir.join("output.log"), &output);

    // Everything the profiler wrote.
    let (mut cpu, mut memory): (Option<Profile>, Option<Profile>) = (None, None);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&raw).into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    for f in files {
        let ext = f.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "py" | "js" | "data" | "nettrace" | "log") || f.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".syms.json")) {
            continue;
        }
        match parse::parse_file(&f) {
            Ok(p) => {
                for (slot, part) in [(&mut cpu, p.cpu), (&mut memory, p.memory)] {
                    let Some(part) = part.filter(|x| !x.is_empty()) else { continue };
                    match slot {
                        Some(have) if have.unit != part.unit => {
                            notes.push(("perf-note-unit-mismatch", vec![("file", f.display().to_string())]));
                        }
                        Some(have) => have.merge(&part),
                        None => *slot = Some(part),
                    }
                }
            }
            Err(e) => notes.push(("perf-note-unreadable", vec![("error", e)])),
        }
    }
    let Some(cpu) = cpu else {
        return Err(tr(ui, "perf-no-profile", &[("output", tail(&output, 2000))]));
    };
    if plan.profiler == Profiler::CProfile {
        notes.push(("perf-note-cprofile", vec![]));
    }
    let memory_note = (!plan.profiler.memory()).then(|| tr(ui, "perf-note-no-memory", &[("profiler", plan.profiler.as_str().into())]));
    let head = Head {
        id,
        target: plan.target.clone(),
        kind: plan.kind,
        language: Some(plan.lang),
        profiler: plan.profiler.as_str().into(),
        command: plan.command.clone().or_else(|| plan.server_cmd.clone()),
        sandbox: plan.sandbox.as_str().into(),
        exit_code: exit,
        wall_ms,
        load: load_stats,
        memory_note,
    };
    let mut pre = plan.notes.clone();
    pre.extend(localize(ui, &notes));
    let mut report = build(&plan.root, &dir, head, &cpu, memory.as_ref(), pre, ui);
    if let Some(b) = &plan.baseline {
        let before = load_report(&plan.root, b)?;
        report.baseline = Some(before.id.clone());
        report.compare = compare(&before, &report);
    }
    save(&dir, &report)?;
    Ok(report)
}

async fn fetch_pprof(base: &str, secs: u64, raw: &Path, allow: &[String]) -> Result<(), String> {
    let client = kor_core::net::client_builder().timeout(Duration::from_secs(secs + 60)).build().map_err(|e| e.to_string())?;
    for (path, file, required) in [(format!("/debug/pprof/profile?seconds={secs}"), "cpu.pprof", true), ("/debug/pprof/heap".to_owned(), "mem.pprof", false)] {
        let url = format!("{base}{path}");
        load::check_target(&url, allow)?;
        let r = client.get(&url).send().await.map_err(|e| kor_core::net::describe(&e));
        match r {
            Ok(resp) if resp.status().is_success() => {
                let b = resp.bytes().await.map_err(|e| e.to_string())?;
                std::fs::write(raw.join(file), &b).map_err(|e| e.to_string())?;
            }
            Ok(resp) if required => {
                return Err(format!("{url}: HTTP {} — the server needs `import _ \"net/http/pprof\"` to be profiled over HTTP", resp.status()));
            }
            Err(e) if required => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

fn localize(ui: kor_i18n::Lang, notes: &[Note]) -> Vec<String> {
    let l = Localizer::new(ui);
    notes.iter().map(|(k, a)| l.t_args(k, &fargs(a))).collect()
}

// ---------------------------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------------------------

/// A project-graph node a frame maps to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeLink {
    pub id: String,
    pub name: String,
    pub file: String,
    pub line: Option<u32>,
    /// `line` (`file:line` inside the node's span) or `name` (same name in the same file, or the
    /// only node with that name).
    pub by: String,
}

/// One function in a top table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub name: String,
    /// Project-relative when inside the project, else as the profiler wrote it.
    pub file: Option<String>,
    pub line: Option<u32>,
    pub self_weight: u64,
    pub total_weight: u64,
    pub self_pct: f64,
    pub total_pct: f64,
    pub node: Option<NodeLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReport {
    pub unit: Unit,
    pub total: u64,
    pub count: Option<u64>,
    pub top: Vec<Row>,
}

/// One line of the before/after table. Both values are measurements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareRow {
    /// `wall_ms`, `samples`, `top_share`, `rps`, `p50_ms`, `p95_ms`, `memory`.
    pub metric: String,
    /// The function for `top_share`.
    pub subject: Option<String>,
    pub before: f64,
    /// `None`: not measurable in the later run (e.g. the function left the top table).
    pub after: Option<f64>,
    pub change_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: String,
    pub created: String,
    pub target: String,
    pub kind: TargetKind,
    pub language: Option<Lang>,
    pub profiler: String,
    pub command: Option<String>,
    /// Sandbox backend the profiled command ran in (`none` = outside).
    pub sandbox: String,
    pub exit_code: Option<i32>,
    /// Measured by Kor around the run (endpoint: the load run).
    pub wall_ms: Option<f64>,
    pub unit: Unit,
    pub total: u64,
    pub top: Vec<Row>,
    /// Hottest function that maps to a project node (the optimization target).
    pub hottest: Option<Row>,
    pub memory: Option<MemoryReport>,
    pub memory_note: Option<String>,
    pub load: Option<LoadStats>,
    /// Project-relative paths of the stored files.
    pub flamegraph: String,
    pub memory_flamegraph: Option<String>,
    pub collapsed: String,
    pub notes: Vec<String>,
    pub baseline: Option<String>,
    pub compare: Vec<CompareRow>,
}

struct Head {
    id: String,
    target: String,
    kind: TargetKind,
    language: Option<Lang>,
    profiler: String,
    command: Option<String>,
    sandbox: String,
    exit_code: Option<i32>,
    wall_ms: Option<f64>,
    load: Option<LoadStats>,
    memory_note: Option<String>,
}

/// Function nodes of the project graph by file, and by short name.
#[derive(Debug, Default)]
pub struct GraphIndex {
    by_file: HashMap<String, Vec<(u32, u32, NodeLink)>>,
    by_name: HashMap<String, Vec<NodeLink>>,
}

impl GraphIndex {
    /// Open (and bring up to date) the project graph.
    pub fn open(root: &Path) -> Result<Self, String> {
        let mut g = kor_graph::Graph::open(root).map_err(|e| e.to_string())?;
        g.update().map_err(|e| e.to_string())?;
        Ok(Self::from_nodes(g.nodes()))
    }

    pub fn from_nodes<'a>(nodes: impl Iterator<Item = &'a kor_graph::GraphNode>) -> Self {
        use kor_graph::NodeKind;
        let mut ix = Self::default();
        for n in nodes {
            if !matches!(n.kind, NodeKind::Function | NodeKind::Struct | NodeKind::Trait) {
                continue;
            }
            let Some(file) = n.file.as_ref().map(|f| f.to_string_lossy().replace('\\', "/")) else { continue };
            let link = NodeLink {
                id: n.id.clone(),
                name: n.name.clone(),
                file: file.clone(),
                line: n.span.map(|s| s.0),
                by: String::new(),
            };
            if let Some((a, b)) = n.span {
                ix.by_file.entry(file).or_default().push((a, b, link.clone()));
            }
            if n.kind == NodeKind::Function {
                ix.by_name.entry(short_name(&n.name).to_owned()).or_default().push(link);
            }
        }
        ix
    }

    /// The node for a frame: the innermost span containing `file:line`, else the same short name
    /// in the same file, else the only function with that name in the project.
    pub fn find(&self, rel: Option<&str>, frame: &Frame) -> Option<NodeLink> {
        let short = short_name(&frame.name);
        if let Some(rel) = rel {
            let in_file = self.by_file.get(rel);
            if let (Some(list), Some(line)) = (in_file, frame.line) {
                let best = list.iter().filter(|(a, b, _)| *a <= line && line <= *b).min_by_key(|(a, b, _)| b - a);
                if let Some((.., l)) = best {
                    return Some(NodeLink { by: "line".into(), ..l.clone() });
                }
            }
            if let Some(l) = in_file.into_iter().flatten().map(|e| &e.2).find(|l| short_name(&l.name) == short) {
                return Some(NodeLink { by: "name".into(), ..l.clone() });
            }
            return None;
        }
        match self.by_name.get(short).map(Vec::as_slice) {
            Some([only]) => Some(NodeLink { by: "name".into(), ..only.clone() }),
            _ => None,
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn pct(part: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { (part as f64 * 10_000.0 / total as f64).round() / 100.0 }
}

fn rows(root: &Path, p: &Profile, n: usize, graph: Option<&GraphIndex>) -> Vec<Row> {
    let total = p.total();
    p.top(n)
        .into_iter()
        .map(|s| {
            let f = &p.frames[s.frame as usize];
            let rel = f.file.as_deref().and_then(|x| relative_to(root, x));
            Row {
                name: f.name.clone(),
                file: rel.clone().or_else(|| f.file.clone()),
                line: f.line,
                self_weight: s.self_weight,
                total_weight: s.total_weight,
                self_pct: pct(s.self_weight, total),
                total_pct: pct(s.total_weight, total),
                node: graph.and_then(|g| g.find(rel.as_deref(), f)),
            }
        })
        .collect()
}

/// Rows kept in a report (the comparison looks functions up here).
const TOP_ROWS: usize = 50;

fn build(root: &Path, dir: &Path, h: Head, cpu: &Profile, memory: Option<&Profile>, mut notes: Vec<String>, ui: kor_i18n::Lang) -> Report {
    let graph = match GraphIndex::open(root) {
        Ok(g) => Some(g),
        Err(e) => {
            notes.push(tr(ui, "perf-note-graph", &[("error", e)]));
            None
        }
    };
    if cpu.frames.iter().any(|f| f.name.starts_with("0x")) {
        notes.push(tr(ui, "perf-note-unsymbolicated", &[]));
    }
    let top = rows(root, cpu, TOP_ROWS, graph.as_ref());
    let hottest = top.iter().find(|r| r.node.is_some() && r.self_weight > 0).cloned();
    let title = format!("kor perf · {} · {} · {} {}", h.target, h.profiler, cpu.total(), cpu.unit.as_str());
    let _ = std::fs::write(dir.join("flame.svg"), flame::svg(cpu, &title));
    let _ = std::fs::write(dir.join("profile.collapsed"), cpu.to_collapsed());
    let rel = |f: &str| relative_to(root, &dir.join(f).to_string_lossy()).unwrap_or_else(|| dir.join(f).to_string_lossy().into_owned());
    let mem = memory.map(|m| {
        let _ = std::fs::write(dir.join("memory.svg"), flame::svg(m, &format!("kor perf · {} · memory · {} {}", h.target, m.total(), m.unit.as_str())));
        MemoryReport {
            unit: m.unit,
            total: m.total(),
            count: m.count,
            top: rows(root, m, 20, graph.as_ref()),
        }
    });
    Report {
        created: chrono::Local::now().to_rfc3339(),
        id: h.id,
        target: h.target,
        kind: h.kind,
        language: h.language,
        profiler: h.profiler,
        command: h.command,
        sandbox: h.sandbox,
        exit_code: h.exit_code,
        wall_ms: h.wall_ms,
        unit: cpu.unit,
        total: cpu.total(),
        top,
        hottest,
        memory_flamegraph: mem.as_ref().map(|_| rel("memory.svg")),
        memory_note: if mem.is_some() { None } else { h.memory_note },
        memory: mem,
        load: h.load,
        flamegraph: rel("flame.svg"),
        collapsed: rel("profile.collapsed"),
        notes,
        baseline: None,
        compare: Vec::new(),
    }
}

fn save(dir: &Path, r: &Report) -> Result<(), String> {
    let json = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("report.json"), json).map_err(|e| format!("{}: {e}", dir.display()))
}

/// `kor perf report <file>`: parse an existing profile (any supported format), map it to the
/// graph and store it like a run.
pub fn report_file(root: &Path, path: &Path, ui: kor_i18n::Lang) -> Result<Report, String> {
    let parsed = parse::parse_file(path)?;
    let format = parsed.format.map_or("?", parse::Format::as_str);
    let (cpu, memory) = match (parsed.cpu, parsed.memory) {
        (Some(c), m) => (c, m),
        (None, Some(m)) => (m, None),
        (None, None) => return Err(tr(ui, "perf-no-profile", &[("output", path.display().to_string())])),
    };
    let id = new_id();
    let dir = run_dir(root, &id)?;
    let head = Head {
        id,
        target: path.display().to_string(),
        kind: TargetKind::File,
        language: None,
        profiler: format.into(),
        command: None,
        sandbox: "none".into(),
        exit_code: None,
        wall_ms: None,
        load: None,
        memory_note: None,
    };
    let r = build(root, &dir, head, &cpu, memory.as_ref(), Vec::new(), ui);
    save(&dir, &r)?;
    Ok(r)
}

/// A stored run, for lists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub created: String,
    pub target: String,
    pub kind: TargetKind,
    pub profiler: String,
    pub wall_ms: Option<f64>,
    pub hottest: Option<String>,
    pub hottest_pct: Option<f64>,
}

/// Stored runs, newest first.
pub fn list(root: &Path) -> Vec<Summary> {
    let mut v: Vec<Summary> = std::fs::read_dir(perf_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| load_report(root, &e.file_name().to_string_lossy()).ok())
        .map(|r| {
            let hot = r.hottest.as_ref().or(r.top.first());
            Summary {
                hottest: hot.map(|h| h.name.clone()),
                hottest_pct: hot.map(|h| h.self_pct),
                id: r.id,
                created: r.created,
                target: r.target,
                kind: r.kind,
                profiler: r.profiler,
                wall_ms: r.wall_ms,
            }
        })
        .collect();
    v.sort_by(|a, b| b.id.cmp(&a.id));
    v
}

/// A stored report by id (ids are checked: no path can be passed).
pub fn load_report(root: &Path, id: &str) -> Result<Report, String> {
    if !crate::valid_session_id(id) {
        return Err(format!("invalid perf run id `{id}`"));
    }
    let p = perf_dir(root).join(id).join("report.json");
    let s = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

/// The flamegraph SVG of a stored run (`memory`: the allocation flamegraph).
pub fn flamegraph(root: &Path, id: &str, memory: bool) -> Result<String, String> {
    let r = load_report(root, id)?;
    let name = if memory { "memory.svg" } else { "flame.svg" };
    if memory && r.memory_flamegraph.is_none() {
        return Err("no memory profile in this run".into());
    }
    let p = perf_dir(root).join(id).join(name);
    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
}

fn row_change(metric: &str, subject: Option<String>, before: f64, after: Option<f64>) -> CompareRow {
    CompareRow {
        metric: metric.into(),
        subject,
        before,
        after,
        change_pct: after.filter(|_| before > 0.0).map(|a| ((a - before) / before * 10_000.0).round() / 100.0),
    }
}

/// Before/after table from two measured reports.
#[allow(clippy::cast_precision_loss)]
pub fn compare(before: &Report, after: &Report) -> Vec<CompareRow> {
    let mut v = Vec::new();
    if let (Some(b), Some(a)) = (before.wall_ms, after.wall_ms) {
        v.push(row_change("wall_ms", None, b, Some(a)));
    }
    if before.unit == after.unit {
        v.push(row_change("samples", None, before.total as f64, Some(after.total as f64)));
    }
    if let Some(h) = before.hottest.as_ref().or(before.top.first()) {
        let now = after.top.iter().find(|r| r.name == h.name && r.file == h.file).map(|r| r.self_pct);
        v.push(row_change("top_share", Some(h.name.clone()), h.self_pct, now));
    }
    if let (Some(b), Some(a)) = (&before.load, &after.load) {
        v.push(row_change("rps", None, b.rps, Some(a.rps)));
        v.push(row_change("p50_ms", None, b.p50_ms, Some(a.p50_ms)));
        v.push(row_change("p95_ms", None, b.p95_ms, Some(a.p95_ms)));
    }
    if let (Some(b), Some(a)) = (&before.memory, &after.memory)
        && b.unit == a.unit
    {
        v.push(row_change("memory", None, b.total as f64, Some(a.total as f64)));
    }
    v
}

/// The agent prompt for `--optimize`: the hottest project function with its measured share.
/// `None` when no hot frame maps to project code.
pub fn optimize_prompt(r: &Report) -> Option<String> {
    let h = r.hottest.as_ref()?;
    let node = h.node.as_ref()?;
    let mut s = format!(
        "Performance profile of `{}` (profiler: {}, measured by Kor): the hottest function in this project is `{}` \
         at {}:{} — {:.2}% of all {} by itself, {:.2}% including what it calls.\n\nTop functions:\n",
        r.target,
        r.profiler,
        node.name,
        node.file,
        node.line.unwrap_or(0),
        h.self_pct,
        r.unit.as_str(),
        h.total_pct
    );
    for row in r.top.iter().take(8) {
        let loc = row.node.as_ref().map(|n| format!(" ({}:{})", n.file, n.line.unwrap_or(0))).unwrap_or_default();
        let _ = writeln!(s, "- {}{loc}: self {:.2}%, total {:.2}%", row.name, row.self_pct, row.total_pct);
    }
    s.push_str(
        "\nRead that function, find why it is expensive and make one focused change that keeps its behavior \
         identical (same results, same public API, tests unchanged). Explain the expected effect briefly. \
         Do not run benchmarks or claim numbers: Kor re-runs the same measurement afterwards and shows \
         the before/after table.",
    );
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_platform::Os;

    #[test]
    fn languages_and_tests() {
        assert_eq!(Lang::of_command("cargo test --release"), Some(Lang::Rust));
        assert_eq!(Lang::of_command("C:\\x\\target\\release\\hot.exe --n 3"), Some(Lang::Rust));
        assert_eq!(Lang::of_command("npm.cmd run bench"), Some(Lang::Node));
        assert_eq!(Lang::of_command("python3 -m pytest"), Some(Lang::Python));
        assert_eq!(Lang::of_command("./mvnw verify"), Some(Lang::Jvm));
        assert_eq!(Lang::of_command("make"), None);
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Lang::of_project(d.path()), None);
        std::fs::write(d.path().join("app.csproj"), "<Project/>").unwrap();
        assert_eq!(Lang::of_project(d.path()), Some(Lang::Dotnet));
        std::fs::write(d.path().join("go.mod"), "module x").unwrap();
        assert_eq!(Lang::of_project(d.path()), Some(Lang::Go));
        assert_eq!(Lang::Go.test_command(d.path()).as_deref(), Some("go test ."));
        assert_eq!(Lang::Native.test_command(d.path()), None);
        for p in PROFILERS {
            assert_eq!(Profiler::parse(p.as_str()), Some(*p));
        }
        assert_eq!(Profiler::parse("asprof"), Some(Profiler::AsyncProfiler));
    }

    #[test]
    fn profiler_choice_never_installs_anything() {
        let none = |_: &str| false;
        let all = |_: &str| true;
        let only = |x: &'static str| move |p: &str| p == x;
        assert_eq!(choose(Lang::Rust, Some("cargo run"), false, Os::Windows, &all), Ok(Profiler::Samply));
        assert_eq!(choose(Lang::Rust, Some("cargo run"), false, Os::Windows, &none), Err(NoProfiler::Missing(Profiler::Samply)));
        assert_eq!(choose(Lang::Rust, Some("cargo run"), false, Os::Linux, &only("perf")), Ok(Profiler::Perf));
        assert_eq!(choose(Lang::Rust, None, true, Os::Windows, &all), Err(NoProfiler::Unsupported));
        assert_eq!(choose(Lang::Native, None, true, Os::Mac, &all), Ok(Profiler::Sample));
        assert_eq!(choose(Lang::Go, Some("go test ./x"), false, Os::Windows, &none), Ok(Profiler::Pprof));
        assert_eq!(choose(Lang::Go, None, true, Os::Windows, &none), Ok(Profiler::Pprof));
        assert_eq!(choose(Lang::Node, Some("npm test"), false, Os::Windows, &none), Ok(Profiler::Node));
        assert_eq!(choose(Lang::Python, Some("python app.py"), false, Os::Windows, &none), Ok(Profiler::CProfile));
        assert_eq!(choose(Lang::Python, Some("pytest"), false, Os::Windows, &none), Err(NoProfiler::Missing(Profiler::PySpy)));
        assert_eq!(choose(Lang::Python, Some("python app.py"), false, Os::Windows, &only("py-spy")), Ok(Profiler::PySpy));
        assert_eq!(choose(Lang::Dotnet, Some("dotnet run"), false, Os::Linux, &none), Err(NoProfiler::Missing(Profiler::DotnetTrace)));
        assert_eq!(choose(Lang::Jvm, Some("mvn test"), false, Os::Windows, &all), Err(NoProfiler::Unsupported));
        assert!(Profiler::PySpy.privileged(false) && !Profiler::Node.privileged(false));
        assert!(!Profiler::AsyncProfiler.privileged(false) && Profiler::AsyncProfiler.privileged(true));
    }

    #[test]
    fn command_lines() {
        assert_eq!(split_words(r#"node "my server.js" --port 3000 '' x"#), ["node", "my server.js", "--port", "3000", "", "x"]);
        assert_eq!(q(true, "a'b c"), "'a''b c'");
        assert_eq!(q(false, "a'b"), r"'a'\''b'");
        assert_eq!(dhms(3725), "00:01:02:05");
        assert!(is_go_test("go test -run X ./..."));
        assert!(!is_go_test("go run ."));
        assert!(is_python("python3 app.py") && !is_python("pytest -q"));
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ağç", 2), "ç");
    }

    fn report(id: &str, wall: f64, hot_pct: f64) -> Report {
        let row = Row {
            name: "fib".into(),
            file: Some("src/lib.rs".into()),
            line: Some(3),
            self_weight: 10,
            total_weight: 10,
            self_pct: hot_pct,
            total_pct: hot_pct,
            node: Some(NodeLink { id: "src/lib.rs#fib".into(), name: "fib".into(), file: "src/lib.rs".into(), line: Some(3), by: "line".into() }),
        };
        Report {
            id: id.into(),
            created: String::new(),
            target: "cargo test".into(),
            kind: TargetKind::Command,
            language: Some(Lang::Rust),
            profiler: "samply".into(),
            command: None,
            sandbox: "none".into(),
            exit_code: Some(0),
            wall_ms: Some(wall),
            unit: Unit::Samples,
            total: 200,
            top: vec![row.clone()],
            hottest: Some(row),
            memory: None,
            memory_note: None,
            load: None,
            flamegraph: String::new(),
            memory_flamegraph: None,
            collapsed: String::new(),
            notes: vec![],
            baseline: None,
            compare: vec![],
        }
    }

    #[test]
    fn before_after_and_prompt() {
        let b = report("a", 1000.0, 40.0);
        let mut a = report("b", 750.0, 10.0);
        a.total = 150;
        let t = compare(&b, &a);
        assert_eq!(t[0].metric, "wall_ms");
        assert_eq!(t[0].change_pct, Some(-25.0));
        assert_eq!(t[1].after, Some(150.0));
        assert_eq!((t[2].subject.as_deref(), t[2].before, t[2].after), (Some("fib"), 40.0, Some(10.0)));
        a.top.clear();
        assert_eq!(compare(&b, &a)[2].after, None);
        let p = optimize_prompt(&b).unwrap();
        assert!(p.contains("`fib` at src/lib.rs:3 — 40.00%"), "{p}");
        let mut none = b.clone();
        none.hottest = None;
        assert!(optimize_prompt(&none).is_none());
    }

    fn test_plan(root: &Path, command: &str, profiler: Profiler, program: String) -> Plan {
        let l = Localizer::new(kor_i18n::Lang::En);
        Plan {
            root: root.to_owned(),
            target: command.into(),
            kind: TargetKind::Command,
            command: Some(command.into()),
            url: None,
            lang: Lang::Python,
            profiler,
            program,
            pid: None,
            server_cmd: None,
            requests: 1,
            concurrency: 1,
            duration: Duration::from_secs(120),
            allow_hosts: vec![],
            sandbox: SandboxKind::None,
            ask: None,
            baseline: None,
            ui: kor_i18n::Lang::En,
            setup: crate::sandbox::setup(&l, &kor_config::Config::default().sandbox, false),
            env: None,
            notes: vec![],
        }
    }

    #[test]
    fn execute_future_is_send() {
        fn assert_send<T: Send>(_: T) {}
        let d = tempfile::tempdir().unwrap();
        assert_send(execute(test_plan(d.path(), "x", Profiler::PySpy, String::new())));
    }

    /// The whole command path with a fake profiler: a Node script standing in for py-spy
    /// writes a fixture speedscope profile to `-o` and runs the command after `--`.
    #[tokio::test]
    async fn end_to_end_with_a_shim_profiler() {
        if !devenv::on_path("node") {
            eprintln!("skipped: node not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let root = kor_tools::without_verbatim(&root);
        let fixture = root.join("fixture.json");
        std::fs::write(&fixture, include_str!("fixtures/profile.speedscope.json")).unwrap();
        let shim = root.join("shim.js");
        std::fs::write(
            &shim,
            format!(
                "const fs=require('fs'),cp=require('child_process');const a=process.argv.slice(2);const i=a.indexOf('-o');\
                 fs.copyFileSync({}, a[i+1]);let r=a.slice(i+2);if(r[0]==='--')r=r.slice(1);\
                 const p=cp.spawnSync(r.join(' '),{{shell:true,stdio:'inherit'}});process.exit(p.status===null?1:p.status);",
                serde_json::to_string(&fixture.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        let ps = powershell(&Shell::detect());
        let program = format!("node {}", q(ps, &shim.to_string_lossy()));
        let r = execute(test_plan(&root, "echo hi", Profiler::PySpy, program.clone())).await.unwrap();
        assert_eq!(r.exit_code, Some(0), "{:?}", r.notes);
        assert_eq!((r.unit, r.total), (Unit::Microseconds, 45_000));
        assert_eq!(r.top[0].name, "fib");
        assert!(r.wall_ms.unwrap() > 0.0);
        assert!(r.memory.is_none() && r.memory_note.as_deref().is_some_and(|n| n.contains("py-spy")));
        let svg = flamegraph(&root, &r.id, false).unwrap();
        assert!(svg.contains("fib") && !svg.contains("<script"));
        assert!(root.join(&r.collapsed).is_file());
        assert_eq!(list(&root)[0].id, r.id);
        // Same measurement again with the first as baseline: a before/after table.
        let mut p2 = test_plan(&root, "echo hi", Profiler::PySpy, program);
        p2.baseline = Some(r.id.clone());
        let r2 = execute(p2).await.unwrap();
        assert_eq!(r2.baseline.as_deref(), Some(r.id.as_str()));
        assert!(r2.compare.iter().any(|c| c.metric == "wall_ms"));
        assert!(r2.compare.iter().any(|c| c.metric == "samples" && c.change_pct == Some(0.0)));
        // A profiler that writes nothing is an error with its output.
        let e = execute(test_plan(&root, "echo hi", Profiler::PySpy, "echo".into())).await.unwrap_err();
        assert!(e.contains("no profile"), "{e}");
        assert!(load_report(&root, "../x").is_err());
    }

    /// Real profilers (not installed on every machine): `cargo test -p kor-runtime perf -- --ignored`.
    #[tokio::test]
    #[ignore = "runs real node / go / python profilers"]
    async fn real_profilers() {
        let dir = tempfile::tempdir().unwrap();
        let root = kor_tools::without_verbatim(&dir.path().canonicalize().unwrap());
        // Sandbox off: on Windows the AppContainer cannot start per-machine toolchains reliably
        // (ADR 0033); this test checks the profilers, not the sandbox.
        let mut cfg = kor_config::Config::default();
        cfg.sandbox.mode = kor_config::SandboxMode::Off;
        let run = |target: &str| {
            let opts = Options::new(root.clone(), target, kor_i18n::Lang::En);
            let p = plan(&opts, &cfg).unwrap();
            assert!(p.ask.is_none() || cfg.sandbox.mode != kor_config::SandboxMode::Off);
            execute(p)
        };
        std::fs::write(root.join("a.js"), "function fib(n){return n<2?n:fib(n-1)+fib(n-2)}\nlet k=[];for(let i=0;i<30;i++){fib(24);k.push(new Array(5000).fill(i))}\n").unwrap();
        let r = run("node a.js").await.unwrap();
        assert_eq!(r.profiler, "node");
        assert!(r.top.iter().any(|t| t.name == "fib"), "{:?}", r.top);
        assert!(r.memory.is_some());
        std::fs::write(root.join("app.py"), "def fib(n):\n    return n if n < 2 else fib(n-1) + fib(n-2)\nfib(22)\n").unwrap();
        let r = run(&format!("{} app.py", kor_platform::python_command())).await.unwrap();
        assert!(r.top.iter().any(|t| t.name == "fib"), "{:?}", r.top);
        std::fs::write(root.join("go.mod"), "module hot\ngo 1.21\n").unwrap();
        std::fs::write(root.join("hot.go"), "package hot\nfunc Fib(n int) int { if n < 2 { return n }; return Fib(n-1) + Fib(n-2) }\n").unwrap();
        std::fs::write(root.join("hot_test.go"), "package hot\nimport \"testing\"\nfunc TestHot(t *testing.T) { for i := 0; i < 20; i++ { Fib(32) } }\n").unwrap();
        let r = run("go test .").await.unwrap();
        assert_eq!(r.profiler, "pprof");
        assert!(r.top.iter().any(|t| t.name == "hot.Fib"), "{:?}", r.top);
    }

    #[test]
    fn frames_map_to_graph_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "mod auth;\npub fn main_entry() { auth::login(\"u\"); }\n").unwrap();
        std::fs::write(root.join("src/auth.rs"), "// auth\n\npub fn login(u: &str) -> bool {\n    u.len() > 3\n}\npub fn logout() {}\n").unwrap();
        let g = GraphIndex::open(root).unwrap();
        // Line 4 is inside `login` (lines 3–5).
        let f = Frame { name: "app::auth::login::h0123456789abcdef".into(), file: Some(root.join("src").join("auth.rs").to_string_lossy().into_owned()), line: Some(4) };
        let rel = relative_to(root, f.file.as_deref().unwrap());
        assert_eq!(rel.as_deref(), Some("src/auth.rs"));
        let n = g.find(rel.as_deref(), &f).unwrap();
        assert_eq!((n.name.as_str(), n.file.as_str(), n.by.as_str()), ("login", "src/auth.rs", "line"));
        // No line: same name in the file.
        let f2 = Frame { line: None, ..f.clone() };
        assert_eq!(g.find(rel.as_deref(), &f2).unwrap().by, "name");
        // No file: the only function with that name in the project.
        let f3 = Frame { name: "logout".into(), file: None, line: None };
        assert_eq!(g.find(None, &f3).unwrap().file, "src/auth.rs");
        assert!(g.find(None, &Frame { name: "nothing".into(), file: None, line: None }).is_none());
    }
}
