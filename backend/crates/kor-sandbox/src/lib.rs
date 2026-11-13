//! Wrap a command so the OS confines it: writes only inside the project (and temp),
//! no network unless allowed, memory/CPU capped where the backend can (ADR 0008, ADR 0033).
//!
//! - Linux: `bubblewrap` (`bwrap`) when installed. With a systemd user manager and the `cpu`
//!   controller delegated (cgroup v2), the command runs in a transient scope
//!   (`systemd-run --user --scope -p CPUQuota= -p MemoryMax=`): a hard CPU cap and a memory cap for
//!   the whole process tree. Without it the CPU share falls back to a `nice` priority.
//! - macOS: Seatbelt via `/usr/bin/sandbox-exec`. macOS has no per-process CPU quota without root:
//!   the share becomes a `taskpolicy` quality-of-service clamp (`utility`/`background`) plus `nice`, which yields
//!   to other work but is not a hard cap (`RLIMIT_CPU` limits total CPU seconds, not a rate).
//! - Windows: `AppContainer` + job object (`kor_platform::appcontainer`, ADR 0008): only the
//!   project and a private temp folder are reachable, memory and CPU are capped.
//! - Any OS: Docker (`[sandbox] backend = "docker"`, or when no native backend exists and the
//!   `docker` CLI is installed). The project is bind-mounted read-write; the rest is read-only.
//!
//! Network allowlists name hosts (`registry.npmjs.org:443`, `*.github.com:443`): the command is
//! pointed at a local filtering proxy ([`proxy`]) that resolves and connects outside the sandbox.
//! Seatbelt lets the command reach nothing but that proxy (enforced); bubblewrap keeps the network
//! shared, so only programs that honour `HTTPS_PROXY` are filtered; `AppContainer` (loopback
//! isolation) and Docker cannot reach a host proxy, so their allowlist opens the whole network.
//!
//! `[sandbox] overlay = true` keeps the project unchanged: writes land in a throwaway layer
//! ([`overlay`]).
//!
//! Commands start through [`spawn`], which picks the mechanism for the kind. What each backend
//! really enforces is reported by [`enforcement`]; blocks it leaves in a command's output are
//! found by [`denials`].

mod denial;
mod docker;
pub mod overlay;
pub mod proxy;

use kor_platform::NoWindow as _;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use kor_platform::{Os, appcontainer};

pub use denial::{Denial, DenialKind, denials};
pub use docker::{DEFAULT_IMAGE, DockerState, docker_pull, docker_state, valid_image};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxKind {
    None,
    Bubblewrap,
    Seatbelt,
    AppContainer,
    Docker,
}

impl SandboxKind {
    pub fn is_enforcing(self) -> bool {
        self != Self::None
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bubblewrap => "bubblewrap",
            Self::Seatbelt => "seatbelt",
            Self::AppContainer => "appcontainer",
            Self::Docker => "docker",
        }
    }
}

/// `[sandbox] backend`: which mechanism the user wants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Preference {
    /// The OS's own backend, else Docker when installed.
    #[default]
    Auto,
    Native,
    Docker,
}

/// Limits and network allowlist (`[sandbox]` in the config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// `host:port` the command may reach when [`Policy::network`] is on; empty = everything.
    /// Only some backends can filter (see [`Enforcement::filter`]).
    pub allow: Vec<String>,
    /// Writes to the project go to a throwaway layer ([`overlay`]).
    pub overlay: bool,
    /// Memory cap in MiB; 0 = none.
    pub memory_mb: u32,
    /// CPU cap in percent of the machine; 0 or ≥ 100 = none.
    pub cpu_percent: u8,
    /// Docker image (never pulled implicitly).
    pub image: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            overlay: false,
            memory_mb: 8 * 1024,
            cpu_percent: 90,
            image: DEFAULT_IMAGE.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Writable project root (absolute).
    pub root: PathBuf,
    pub network: bool,
    pub options: Options,
}

impl Policy {
    /// `root` writable, no network, default limits.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            network: false,
            options: Options::default(),
        }
    }
}

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// What this machine offers with the default preference.
pub fn detect() -> SandboxKind {
    select(Preference::Auto)
}

/// The OS's own backend, if any.
pub fn native() -> SandboxKind {
    match kor_platform::os() {
        Os::Mac if Path::new(SANDBOX_EXEC).is_file() => SandboxKind::Seatbelt,
        Os::Linux if on_path("bwrap").is_some() => SandboxKind::Bubblewrap,
        Os::Windows if appcontainer::available() => SandboxKind::AppContainer,
        _ => SandboxKind::None,
    }
}

/// The backend for `pref`. Docker counts when its CLI is installed; a stopped daemon makes the
/// command fail (closed), it never runs unconfined.
pub fn select(pref: Preference) -> SandboxKind {
    let docker = || on_path(&kor_platform::exe("docker")).is_some();
    match pref {
        Preference::Native => native(),
        Preference::Docker if docker() => SandboxKind::Docker,
        Preference::Docker => SandboxKind::None,
        Preference::Auto => match native() {
            SandboxKind::None if docker() => SandboxKind::Docker,
            k => k,
        },
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// How a backend can narrow the network once it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// All or nothing: an allowlist turns the whole network on.
    None,
    /// Host names are enforced: the command can reach only the filtering proxy (and allowed
    /// `localhost` ports).
    Domains,
    /// Host names are filtered for programs that use the proxy variables; others reach any host.
    Proxy,
}

impl Filter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Domains => "domains",
            Self::Proxy => "proxy",
        }
    }
}

/// What a backend really enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // a fact sheet, one flag per guarantee
pub struct Enforcement {
    /// Writes outside the project (and private temp) are blocked.
    pub files: bool,
    /// The network can be switched off.
    pub network_off: bool,
    pub filter: Filter,
    pub memory: bool,
    pub cpu: bool,
}

pub fn enforcement(kind: SandboxKind) -> Enforcement {
    let all = |filter, memory, cpu| Enforcement {
        files: true,
        network_off: true,
        filter,
        memory,
        cpu,
    };
    match kind {
        SandboxKind::None => Enforcement {
            files: false,
            network_off: false,
            filter: Filter::None,
            memory: false,
            cpu: false,
        },
        // RLIMIT_DATA per process, plus a cgroup scope (hard CPU cap) where systemd delegates one.
        SandboxKind::Bubblewrap => all(Filter::Proxy, true, cpu_scope_available()),
        // macOS ignores address-space rlimits and has no CPU quota; only the proxy is reachable.
        SandboxKind::Seatbelt => all(Filter::Domains, false, false),
        // Job object for the whole tree; capabilities are all or nothing.
        SandboxKind::AppContainer | SandboxKind::Docker => all(Filter::None, true, true),
    }
}

/// `nice -n N` for a CPU share below 100% where no hard cap is available: the share becomes a
/// lower scheduling priority (the smaller the share, the nicer), not a hard cap.
fn nice(cpu_percent: u8) -> Vec<OsString> {
    let n = match cpu_percent {
        0 | 100.. => return Vec::new(),
        1..=25 => "19",
        26..=50 => "10",
        _ => "5",
    };
    ["nice", "-n", n].map(OsString::from).to_vec()
}

const TASKPOLICY: &str = "/usr/sbin/taskpolicy";

/// macOS quality-of-service clamp for a small CPU share (`taskpolicy -c`): the scheduler runs the command on
/// efficiency cores and behind interactive work. Not a quota.
fn taskpolicy(cpu_percent: u8) -> Vec<OsString> {
    let clamp = match cpu_percent {
        1..=25 => "background",
        26..=50 => "utility",
        _ => return Vec::new(),
    };
    [TASKPOLICY, "-c", clamp].map(OsString::from).to_vec()
}

/// A systemd user manager on cgroup v2 with the `cpu` controller delegated to it: then
/// `systemd-run --user --scope -p CPUQuota=` is a hard cap.
fn cpu_scope_available() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        if kor_platform::os() != Os::Linux || on_path("systemd-run").is_none() {
            return false;
        }
        let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
            return false;
        };
        let Some((uid, _)) = kor_platform::owner_ids(&run) else {
            return false;
        };
        let reachable = run.join("bus").exists() || run.join("systemd/private").exists();
        let delegated = std::fs::read_to_string(format!(
            "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/cgroup.controllers"
        ))
        .is_ok_and(|c| c.split_whitespace().any(|w| w == "cpu"));
        reachable && delegated
    })
}

/// `systemd-run --user --scope …` arguments for a CPU share of the whole machine and a memory cap
/// of the whole tree; empty without a CPU share. `CPUQuota` counts one CPU as 100%.
fn cpu_scope(cpu_percent: u8, memory_mb: u32, cpus: usize) -> Vec<OsString> {
    if !(1..100).contains(&cpu_percent) {
        return Vec::new();
    }
    let quota = usize::from(cpu_percent).saturating_mul(cpus.max(1));
    let mut a: Vec<OsString> = ["--user", "--scope", "--quiet", "--collect", "-p"]
        .map(OsString::from)
        .to_vec();
    a.push(format!("CPUQuota={quota}%").into());
    if memory_mb > 0 {
        a.extend(["-p".into(), format!("MemoryMax={memory_mb}M").into()]);
    }
    a.push("--".into());
    a
}

/// What this machine adds to a wrapped command (detected; tests set it by hand).
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // independent host facts
struct Host {
    /// Port of the filtering proxy started for this command.
    proxy: Option<u16>,
    /// bubblewrap can mount the project as an overlay.
    tmp_overlay: bool,
    /// Hard CPU cap through a systemd scope.
    cpu_scope: bool,
    taskpolicy: bool,
}

impl Host {
    fn detect(kind: SandboxKind, proxy: Option<u16>) -> Self {
        Self {
            proxy,
            tmp_overlay: kind == SandboxKind::Bubblewrap && overlay::bwrap_has_overlay(),
            cpu_scope: kind == SandboxKind::Bubblewrap && cpu_scope_available(),
            taskpolicy: kind == SandboxKind::Seatbelt && Path::new(TASKPOLICY).is_file(),
        }
    }
}

/// Program and arguments that run `program args` under `kind` with `policy` (kinds that work by
/// wrapping the command line; `AppContainer` and Docker start through [`spawn`] instead).
/// Without the proxy [`spawn`] starts, a Seatbelt allowlist reaches only its `localhost` entries.
pub fn wrap(
    kind: SandboxKind,
    policy: &Policy,
    program: &OsStr,
    args: &[OsString],
) -> Result<(OsString, Vec<OsString>), String> {
    wrap_with(kind, policy, Host::detect(kind, None), program, args)
}

fn wrap_with(
    kind: SandboxKind,
    policy: &Policy,
    host: Host,
    program: &OsStr,
    args: &[OsString],
) -> Result<(OsString, Vec<OsString>), String> {
    let tail = std::iter::once(program.to_owned()).chain(args.iter().cloned());
    match kind {
        SandboxKind::None => Ok((program.to_owned(), args.to_vec())),
        SandboxKind::AppContainer | SandboxKind::Docker => {
            Err(format!("{} commands start through spawn()", kind.as_str()))
        }
        SandboxKind::Bubblewrap => {
            let root = policy.root.as_os_str();
            let mut a: Vec<OsString> = [
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
            ]
            .iter()
            .map(OsString::from)
            .collect();
            if policy.options.overlay && host.tmp_overlay {
                a.extend([
                    OsString::from("--overlay-src"),
                    root.into(),
                    "--tmp-overlay".into(),
                    root.into(),
                ]);
            } else {
                a.extend([OsString::from("--bind"), root.into(), root.into()]);
            }
            if !policy.network {
                a.push("--unshare-net".into());
            }
            a.extend(["--die-with-parent", "--new-session", "--chdir"].map(OsString::from));
            a.push(root.into());
            a.push("--".into());
            let kib = u64::from(policy.options.memory_mb) * 1024;
            if kib > 0 {
                // RLIMIT_DATA counts writable private memory, not reservations (V8, Go arenas).
                a.extend(
                    [
                        "/bin/sh",
                        "-c",
                        &format!("ulimit -d {kib} 2>/dev/null; exec \"$@\""),
                        "kor-sandbox",
                    ]
                    .map(OsString::from),
                );
            }
            let o = &policy.options;
            let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
            let mut scope = if host.cpu_scope {
                cpu_scope(o.cpu_percent, o.memory_mb, cpus)
            } else {
                Vec::new()
            };
            if scope.is_empty() {
                a.extend(nice(o.cpu_percent));
                a.extend(tail);
                return Ok(("bwrap".into(), a));
            }
            a.extend(tail);
            scope.push("bwrap".into());
            scope.extend(a);
            Ok(("systemd-run".into(), scope))
        }
        SandboxKind::Seatbelt => {
            let mut a = vec![
                OsString::from("-p"),
                seatbelt_profile(policy, host.proxy)?.into(),
            ];
            if host.taskpolicy {
                a.extend(taskpolicy(policy.options.cpu_percent));
            }
            a.extend(nice(policy.options.cpu_percent));
            a.extend(tail);
            Ok((SANDBOX_EXEC.into(), a))
        }
    }
}

/// `PowerShell` cannot set up its file-system drives inside an `AppContainer` (volume roots and
/// the folders above the project are not readable to it), so it starts in `C:\` with relative paths
/// broken. Give it the system drive back and the project as `Project:\`, then run the command.
/// Absolute paths inside the project and native programs keep working.
const POWERSHELL_PRELUDE: &str = "$__d = [Environment]::CurrentDirectory; $__r = [IO.Path]::GetPathRoot($__d); \
$null = New-PSDrive -Name $__r.Substring(0, 1) -PSProvider FileSystem -Root $__r -Scope Global -ErrorAction SilentlyContinue; \
$null = New-PSDrive -Name Project -PSProvider FileSystem -Root $__d -Scope Global -ErrorAction SilentlyContinue; \
Set-Location Project:\\ -ErrorAction SilentlyContinue; Remove-Variable __d, __r";

fn with_powershell_prelude(program: &OsStr, args: &mut [OsString]) {
    let stem = Path::new(program)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase());
    if !matches!(stem.as_deref(), Some("pwsh" | "powershell")) {
        return;
    }
    let at = args
        .iter()
        .position(|a| a.eq_ignore_ascii_case("-Command") || a.eq_ignore_ascii_case("-c"));
    if let Some(cmd) = at.and_then(|i| args.get_mut(i + 1)) {
        let mut s = OsString::from(POWERSHELL_PRELUDE);
        s.push("\n");
        s.push(&*cmd);
        *cmd = s;
    }
}

/// A started command (with the filtering proxy and overlay copy it needs while it runs).
#[derive(Debug)]
pub struct Child {
    inner: Inner,
    proxy: Option<proxy::Proxy>,
    scratch: Option<overlay::Scratch>,
}

#[derive(Debug)]
enum Inner {
    Process(tokio::process::Child),
    Container(appcontainer::Child),
    Docker(tokio::process::Child, docker::Guard),
}

/// Read `r` to the end, keeping every byte and passing each finished line to `on_line`.
async fn pump(
    r: Option<impl tokio::io::AsyncRead + Unpin>,
    on_line: &(dyn Fn(&str) + Sync),
) -> io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt as _;
    let mut all = Vec::new();
    let Some(mut r) = r else { return Ok(all) };
    let mut buf = [0u8; 8192];
    let mut start = 0;
    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
        while let Some(i) = all[start..].iter().position(|&b| b == b'\n') {
            on_line(String::from_utf8_lossy(&all[start..start + i]).trim_end_matches('\r'));
            start += i + 1;
        }
    }
    if start < all.len() {
        on_line(String::from_utf8_lossy(&all[start..]).trim_end_matches('\r'));
    }
    Ok(all)
}

async fn stream_process(
    mut c: tokio::process::Child,
    on_line: &(dyn Fn(&str) + Sync),
) -> io::Result<Output> {
    let (out, err) = (c.stdout.take(), c.stderr.take());
    let (stdout, stderr, status) =
        tokio::try_join!(pump(out, on_line), pump(err, on_line), c.wait())?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

impl Child {
    /// Like [`Child::wait_with_output`], and each output line (stdout or stderr) goes to
    /// `on_line` as it arrives, for a live view. An `AppContainer` child reports at the end.
    pub async fn wait_with_output_streaming(
        self,
        on_line: &(dyn Fn(&str) + Sync),
    ) -> io::Result<Output> {
        let out = match self.inner {
            Inner::Process(c) => Box::pin(stream_process(c, on_line)).await,
            Inner::Container(c) => c.wait_with_output().await,
            Inner::Docker(c, mut guard) => {
                let out = Box::pin(stream_process(c, on_line)).await;
                guard.finished();
                out
            }
        };
        drop(self.scratch);
        with_blocked(out, self.proxy.as_ref())
    }

    /// Wait for exit and collect stdout/stderr. Dropping the future kills the command.
    pub async fn wait_with_output(self) -> io::Result<Output> {
        let out = match self.inner {
            Inner::Process(c) => c.wait_with_output().await,
            Inner::Container(c) => c.wait_with_output().await,
            Inner::Docker(c, mut guard) => {
                let out = c.wait_with_output().await;
                guard.finished();
                out
            }
        };
        drop(self.scratch);
        with_blocked(out, self.proxy.as_ref())
    }
}

/// Hosts the proxy refused, as lines in stderr ([`denials`] finds them): many programs print only
/// "403" or nothing for a refused tunnel.
fn with_blocked(out: io::Result<Output>, proxy: Option<&proxy::Proxy>) -> io::Result<Output> {
    let mut out = out?;
    for h in proxy
        .map(proxy::Proxy::blocked)
        .unwrap_or_default()
        .iter()
        .take(5)
    {
        out.stderr.extend_from_slice(
            format!("\nkor sandbox: {h} is not in sandbox.network\n").as_bytes(),
        );
    }
    Ok(out)
}

/// Start `cmd` (program, arguments, working directory and environment changes are taken from
/// it) confined by `kind`, stdin empty, stdout and stderr captured.
pub fn spawn(
    kind: SandboxKind,
    policy: &Policy,
    cmd: &tokio::process::Command,
) -> io::Result<Child> {
    let cmd = cmd.as_std();
    let args: Vec<OsString> = cmd.get_args().map(OsStr::to_owned).collect();
    let mut cwd = cmd.get_current_dir().unwrap_or(&policy.root).to_path_buf();
    // Overlay without kernel support: the command works on a temp copy of the project.
    let mut scratch = None;
    let copied;
    let mut policy = policy;
    if kind.is_enforcing()
        && policy.options.overlay
        && !(kind == SandboxKind::Bubblewrap && overlay::bwrap_has_overlay())
    {
        let s = overlay::Scratch::copy_of(&policy.root)?;
        cwd = s.map(&policy.root, &cwd);
        copied = Policy {
            root: s.root.clone(),
            ..policy.clone()
        };
        policy = &copied;
        scratch = Some(s);
    }
    let cwd = cwd.as_path();
    let proxy = match enforcement(kind).filter {
        Filter::Domains | Filter::Proxy if policy.network && !policy.options.allow.is_empty() => {
            Some(proxy::Proxy::start(policy.options.allow.clone())?)
        }
        _ => None,
    };
    let child = |inner| Child {
        inner,
        proxy: None,
        scratch: None,
    };
    if kind == SandboxKind::Docker {
        return docker::spawn(policy, cmd, &args, cwd).map(|(c, g)| Child {
            scratch,
            ..child(Inner::Docker(c, g))
        });
    }
    if kind == SandboxKind::AppContainer {
        let mut args = args;
        with_powershell_prelude(cmd.get_program(), &mut args);
        let mut env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        for (k, v) in cmd.get_envs() {
            env.retain(|(e, _)| !e.eq_ignore_ascii_case(k));
            if let Some(v) = v {
                env.push((k.to_owned(), v.to_owned()));
            }
        }
        let o = &policy.options;
        return appcontainer::spawn(&appcontainer::Spec {
            program: cmd.get_program(),
            args: &args,
            cwd,
            env: &env,
            root: &policy.root,
            network: policy.network,
            memory_bytes: u64::from(o.memory_mb) << 20,
            cpu_percent: o.cpu_percent,
        })
        .map(|c| Child {
            scratch,
            ..child(Inner::Container(c))
        });
    }
    let host = Host::detect(kind, proxy.as_ref().map(|p| p.port));
    let (program, args) =
        wrap_with(kind, policy, host, cmd.get_program(), &args).map_err(io::Error::other)?;
    let mut c = tokio::process::Command::new(program);
    c.no_window();
    c.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in cmd.get_envs() {
        match v {
            Some(v) => c.env(k, v),
            None => c.env_remove(k),
        };
    }
    if let Some(p) = &proxy {
        for (k, v) in p.env() {
            c.env(k, v);
        }
        // Everything goes through the proxy, `localhost` too (it checks the list).
        c.env_remove("NO_PROXY").env_remove("no_proxy");
    }
    let inner = Inner::Process(c.spawn()?);
    Ok(Child {
        inner,
        proxy,
        scratch,
    })
}

/// `host:port` with a numeric port (or `*`), or `*` alone. Rejects anything that could break out
/// of a Seatbelt profile string.
pub fn valid_allow(entry: &str) -> bool {
    if entry == "*" {
        return true;
    }
    let Some((host, port)) = entry.rsplit_once(':') else {
        return false;
    };
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'*' | b'[' | b']' | b':')
        });
    host_ok && (port == "*" || port.parse::<u16>().is_ok_and(|p| p > 0))
}

fn seatbelt_profile(policy: &Policy, proxy: Option<u16>) -> Result<String, String> {
    let root = policy
        .root
        .to_str()
        .ok_or("project path is not valid UTF-8")?;
    // The profile is a Scheme string literal; refuse anything that could break out of it.
    if root.contains(['"', '\\', '\n', '\r']) {
        return Err("project path contains characters not allowed in a sandbox profile".into());
    }
    let mut net = String::new();
    if !policy.network {
        net.push_str("(deny network*)\n");
    } else if !policy.options.allow.is_empty() {
        // Seatbelt can match only `*` or `localhost` as the host: host names go through the
        // filtering proxy, the only other address the command may reach.
        net.push_str(
            "(deny network*)\n(allow network-outbound (literal \"/private/var/run/mDNSResponder\")",
        );
        if let Some(port) = proxy {
            let _ = write!(net, " (remote ip \"localhost:{port}\")");
        }
        for entry in &policy.options.allow {
            if !valid_allow(entry) {
                return Err(format!("invalid network allowlist entry: {entry}"));
            }
            let (host, port) = entry.rsplit_once(':').unwrap_or(("*", "*"));
            if matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
                let _ = write!(net, " (remote ip \"localhost:{port}\")");
            }
        }
        net.push_str(")\n");
    }
    Ok(format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n\
         (allow file-write* (subpath \"{root}\") (subpath \"/private/tmp\") \
         (subpath \"/private/var/folders\") (subpath \"/dev\"))\n{net}"
    ))
}

/// Result of [`probe`].
#[derive(Debug, Clone)]
pub struct Probe {
    pub kind: SandboxKind,
    /// The file outside the project did not appear.
    pub blocked: bool,
    /// Where the probe tried to write.
    pub target: PathBuf,
    /// Blocks recognized in the command's output.
    pub detected: Vec<Denial>,
    /// Combined stdout and stderr.
    pub output: String,
}

/// Safe self-check (`kor sandbox test`): a confined command tries to write one file into Kor's
/// own data folder, outside a throwaway project. Both are removed afterwards.
pub async fn probe(kind: SandboxKind, options: &Options) -> io::Result<Probe> {
    if !kind.is_enforcing() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no sandbox backend on this machine",
        ));
    }
    let id = format!("{}-{}", std::process::id(), docker::next_id());
    let root = std::env::temp_dir().join(format!("kor-sandbox-probe-{id}"));
    let outside = kor_platform::data_dir()
        .ok_or_else(|| io::Error::other("no data directory"))?
        .join("sandbox-probe");
    std::fs::create_dir_all(&root)?;
    std::fs::create_dir_all(&outside)?;
    let root = root.canonicalize()?;
    let target = outside.join(format!("outside-{id}.txt"));
    let result = run_probe(kind, options, &root, &target).await;
    let blocked = !target.exists();
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_dir_all(&root);
    let (output, detected) = result?;
    Ok(Probe {
        kind,
        blocked,
        target,
        detected,
        output,
    })
}

async fn run_probe(
    kind: SandboxKind,
    options: &Options,
    root: &Path,
    target: &Path,
) -> io::Result<(String, Vec<Denial>)> {
    let t = target.to_string_lossy();
    let mut cmd = if kind == SandboxKind::AppContainer {
        let mut c = tokio::process::Command::new("powershell");
        c.no_window();
        let write = format!(
            // UTF-8 output: Windows PowerShell 5.1 otherwise prints errors in the OEM code page.
            "[Console]::OutputEncoding = [Text.Encoding]::UTF8; [IO.File]::WriteAllText('{}', 'kor')",
            t.replace('\'', "''")
        );
        c.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &write,
        ]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", &format!("echo kor > '{}'", t.replace('\'', "'\\''"))]);
        c
    };
    cmd.current_dir(root);
    let policy = Policy {
        root: root.to_path_buf(),
        network: false,
        options: options.clone(),
    };
    let out = spawn(kind, &policy, &cmd)?.wait_with_output().await?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let found = denials(kind, &policy, &text);
    Ok((text, found))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(network: bool) -> Policy {
        Policy {
            network,
            ..Policy::new("/work/proj")
        }
    }

    fn strs(v: &[OsString]) -> Vec<String> {
        v.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn none_passes_through() {
        let (p, a) = wrap(
            SandboxKind::None,
            &policy(false),
            "sh".as_ref(),
            &["-c".into(), "ls".into()],
        )
        .unwrap();
        assert_eq!(p, "sh");
        assert_eq!(strs(&a), ["-c", "ls"]);
    }

    fn bwrap(policy: &Policy, host: Host) -> (OsString, Vec<String>) {
        let (p, a) = wrap_with(
            SandboxKind::Bubblewrap,
            policy,
            host,
            "sh".as_ref(),
            &["-c".into(), "ls".into()],
        )
        .unwrap();
        (p, strs(&a))
    }

    #[test]
    fn bubblewrap_binds_root_drops_network_and_caps_memory() {
        let (p, a) = wrap_with(
            SandboxKind::Bubblewrap,
            &policy(false),
            Host::default(),
            "sh".as_ref(),
            &["-c".into(), "ls".into()],
        )
        .unwrap();
        let a = strs(&a);
        assert_eq!(p, "bwrap");
        assert!(
            a.windows(3)
                .any(|w| w == ["--bind", "/work/proj", "/work/proj"])
        );
        assert!(a.contains(&"--unshare-net".into()));
        assert!(
            a.iter()
                .any(|s| s == "ulimit -d 8388608 2>/dev/null; exec \"$@\"")
        );
        assert_eq!(
            &a[a.len() - 7..],
            ["kor-sandbox", "nice", "-n", "5", "sh", "-c", "ls"]
        );

        let mut open = policy(true);
        open.options.memory_mb = 0;
        open.options.cpu_percent = 0;
        let (_, a) = wrap_with(
            SandboxKind::Bubblewrap,
            &open,
            Host::default(),
            "sh".as_ref(),
            &[],
        )
        .unwrap();
        let a = strs(&a);
        assert!(!a.contains(&"--unshare-net".into()));
        assert_eq!(&a[a.len() - 2..], ["--", "sh"]);
    }

    #[test]
    fn bubblewrap_hard_cpu_cap_runs_in_a_systemd_scope() {
        let scoped = Host {
            cpu_scope: true,
            ..Host::default()
        };
        let (p, a) = bwrap(&policy(false), scoped);
        assert_eq!(p, "systemd-run");
        assert_eq!(&a[..5], ["--user", "--scope", "--quiet", "--collect", "-p"]);
        assert!(
            a[5].starts_with("CPUQuota=") && a.contains(&"MemoryMax=8192M".into()),
            "{a:?}"
        );
        let at = a.iter().position(|s| s == "bwrap").unwrap();
        assert_eq!(a[at - 1], "--");
        assert!(!a.contains(&"nice".into()), "the scope replaces nice");
        assert_eq!(&a[a.len() - 3..], ["sh", "-c", "ls"]);

        // No CPU share: no scope, even when systemd is there.
        let mut free = policy(false);
        free.options.cpu_percent = 100;
        assert_eq!(bwrap(&free, scoped).0, "bwrap");

        assert_eq!(
            strs(&cpu_scope(50, 0, 8)),
            [
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "-p",
                "CPUQuota=400%",
                "--"
            ]
        );
        assert!(cpu_scope(0, 1024, 8).is_empty() && cpu_scope(100, 1024, 8).is_empty());
        assert_eq!(strs(&cpu_scope(30, 0, 0))[5], "CPUQuota=30%");
    }

    #[test]
    fn bubblewrap_overlay_mounts_the_project_copy_on_write() {
        let mut p = policy(false);
        p.options.overlay = true;
        let with = Host {
            tmp_overlay: true,
            ..Host::default()
        };
        let (_, a) = bwrap(&p, with);
        assert!(
            a.windows(4)
                .any(|w| w == ["--overlay-src", "/work/proj", "--tmp-overlay", "/work/proj"]),
            "{a:?}"
        );
        assert!(!a.contains(&"--bind".into()));
        // Overlay off, or an old bubblewrap (spawn copies the project instead): a plain bind.
        assert!(bwrap(&policy(false), with).1.contains(&"--bind".into()));
        assert!(bwrap(&p, Host::default()).1.contains(&"--bind".into()));
    }

    #[test]
    fn macos_cpu_share_is_a_qos_clamp() {
        assert_eq!(strs(&taskpolicy(20)), [TASKPOLICY, "-c", "background"]);
        assert_eq!(strs(&taskpolicy(50))[2], "utility");
        assert!(taskpolicy(90).is_empty() && taskpolicy(0).is_empty());
        let mut p = policy(false);
        p.options.cpu_percent = 40;
        let host = Host {
            taskpolicy: true,
            ..Host::default()
        };
        let (_, a) = wrap_with(SandboxKind::Seatbelt, &p, host, "sh".as_ref(), &[]).unwrap();
        assert_eq!(
            &strs(&a)[2..],
            [TASKPOLICY, "-c", "utility", "nice", "-n", "10", "sh"]
        );
    }

    #[test]
    fn seatbelt_profile_limits_writes_reaches_only_the_proxy_and_rejects_injection() {
        let (p, a) = wrap(SandboxKind::Seatbelt, &policy(false), "sh".as_ref(), &[]).unwrap();
        assert_eq!(p, SANDBOX_EXEC);
        let profile = a[1].to_string_lossy().into_owned();
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains("(subpath \"/work/proj\")"));
        assert!(profile.contains("(deny network*)"));

        let mut allow = policy(true);
        allow.options.allow = vec!["registry.npmjs.org:443".into(), "localhost:5432".into()];
        let proxied = Host {
            proxy: Some(41_000),
            ..Host::default()
        };
        let (_, a) = wrap_with(SandboxKind::Seatbelt, &allow, proxied, "sh".as_ref(), &[]).unwrap();
        let profile = a[1].to_string_lossy().into_owned();
        assert!(
            !profile.contains("*:443"),
            "hosts go through the proxy: {profile}"
        );
        assert!(
            profile.contains("(remote ip \"localhost:41000\")"),
            "{profile}"
        );
        assert!(
            profile.contains("(remote ip \"localhost:5432\")"),
            "{profile}"
        );
        assert!(profile.contains("(deny network*)"));
        // Without the proxy only the localhost entry is reachable.
        let (_, a) = wrap(SandboxKind::Seatbelt, &allow, "sh".as_ref(), &[]).unwrap();
        assert!(!a[1].to_string_lossy().contains("41000"));

        allow.options.allow = vec!["x\") (allow default) (\":1".into()];
        assert!(wrap(SandboxKind::Seatbelt, &allow, "sh".as_ref(), &[]).is_err());
        let (_, a) = wrap(SandboxKind::Seatbelt, &policy(true), "sh".as_ref(), &[]).unwrap();
        assert!(!a[1].to_string_lossy().contains("network"));

        let evil = Policy::new(r#"/x") (allow file-write* (subpath "/"#);
        assert!(wrap(SandboxKind::Seatbelt, &evil, "sh".as_ref(), &[]).is_err());
    }

    #[test]
    fn allowlist_entries_are_host_port_pairs() {
        for ok in [
            "*",
            "github.com:443",
            "localhost:*",
            "[::1]:8080",
            "*.npmjs.org:443",
        ] {
            assert!(valid_allow(ok), "{ok}");
        }
        for bad in ["github.com", "a:0", "a:70000", ":443", "a b:1", "a\":1"] {
            assert!(!valid_allow(bad), "{bad}");
        }
    }

    #[test]
    fn cpu_share_becomes_nice_priority() {
        assert!(nice(0).is_empty() && nice(100).is_empty());
        assert_eq!(nice(20), ["nice", "-n", "19"].map(OsString::from));
        assert_eq!(nice(50)[2], "10");
        assert_eq!(nice(90)[2], "5");
    }

    #[test]
    fn enforcement_is_honest_per_backend() {
        assert!(!enforcement(SandboxKind::None).files);
        assert_eq!(enforcement(SandboxKind::Bubblewrap).filter, Filter::Proxy);
        assert!(!enforcement(SandboxKind::Seatbelt).memory);
        assert!(!enforcement(SandboxKind::Seatbelt).cpu);
        assert_eq!(enforcement(SandboxKind::Seatbelt).filter, Filter::Domains);
        assert_eq!(enforcement(SandboxKind::AppContainer).filter, Filter::None);
        assert_eq!(Filter::Domains.as_str(), "domains");
        let d = enforcement(SandboxKind::Docker);
        assert!(d.files && d.network_off && d.memory && d.cpu && d.filter == Filter::None);
    }

    #[test]
    fn powershell_gets_the_prelude_and_others_do_not() {
        let mut a: Vec<OsString> = ["-NoProfile", "-Command", "ls"]
            .map(OsString::from)
            .to_vec();
        with_powershell_prelude(OsStr::new("pwsh.exe"), &mut a);
        let cmd = a[2].to_string_lossy();
        assert!(
            cmd.starts_with(POWERSHELL_PRELUDE) && cmd.ends_with("\nls"),
            "{cmd}"
        );
        let mut b: Vec<OsString> = ["-c", "ls"].map(OsString::from).to_vec();
        with_powershell_prelude(OsStr::new("bash"), &mut b);
        assert_eq!(strs(&b), ["-c", "ls"]);
    }

    #[tokio::test]
    async fn spawn_runs_the_command_with_env_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // Unconfined, and confined by whatever this machine has (AppContainer on Windows).
        for kind in [SandboxKind::None, detect()] {
            if matches!(
                kind,
                SandboxKind::Bubblewrap | SandboxKind::Seatbelt | SandboxKind::Docker
            ) {
                continue; // covered by the argument tests above; may be unusable in CI containers
            }
            let mut cmd = if kind == SandboxKind::AppContainer || kor_platform::os() == Os::Windows
            {
                let mut c = tokio::process::Command::new("powershell");
                c.args([
                    "-NoProfile",
                    "-Command",
                    "Write-Output \"v=$env:KOR_SBX_PROBE\"",
                ]);
                c
            } else {
                let mut c = tokio::process::Command::new("sh");
                c.args(["-c", "echo v=$KOR_SBX_PROBE"]);
                c
            };
            cmd.env("KOR_SBX_PROBE", "42").current_dir(&root);
            let out = spawn(kind, &Policy::new(root.clone()), &cmd)
                .unwrap()
                .wait_with_output()
                .await
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && text.contains("v=42"),
                "{kind:?}: {text}"
            );
        }
    }

    #[tokio::test]
    async fn overlay_keeps_the_project_unchanged() {
        if native() != SandboxKind::AppContainer {
            return; // bubblewrap/Seatbelt overlay arguments are covered above
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.txt"), "before").unwrap();
        let mut c = tokio::process::Command::new("powershell");
        c.args([
            "-NoProfile",
            "-Command",
            "[IO.File]::WriteAllText((Join-Path (Get-Location).ProviderPath 'a.txt'), 'after'); Get-Content a.txt",
        ])
        .current_dir(&root);
        let mut p = Policy::new(root.clone());
        p.options.overlay = true;
        let out = spawn(SandboxKind::AppContainer, &p, &c)
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("after"),
            "{text}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "before"
        );
    }

    #[tokio::test]
    async fn probe_is_blocked_by_the_native_backend() {
        let kind = native();
        if !kind.is_enforcing() || kind != SandboxKind::AppContainer {
            return; // bubblewrap/Seatbelt: kor-agent's sandbox_denied test runs on CI hosts
        }
        let p = probe(kind, &Options::default()).await.unwrap();
        assert!(p.blocked, "{}", p.output);
        assert!(!p.detected.is_empty(), "{}", p.output);
        assert!(!p.target.exists());
    }
}
