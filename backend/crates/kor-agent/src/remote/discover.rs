//! Live system discovery (`kor discover`, spec §15.16, ADR 0044): a read-only scan of a machine
//! over SSH. Every step is one fixed POSIX `sh` command that only reads (no writes, no restarts,
//! env files listed by path, never opened). A step that hits "permission denied" is offered again
//! under `sudo -n` — only when the caller's `ask` answers yes for that step; `sudo` never prompts
//! for a password (batch SSH has no terminal), so a password-protected sudo is reported, not tried.
//!
//! The result is a [`kor_graph::InfraSnapshot`] plus its Markdown (`<kor home>/infra/<host>.md`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_graph::infra::{Disk, InfraSnapshot, Memory, Port, Schedule, Service, ToolVersion, VHost};
use serde::{Deserialize, Serialize};

use super::{Output, Shell, sh_quote};

/// One scan step: a fixed read-only command.
#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub id: &'static str,
    pub cmd: &'static str,
    /// Allowed under `sudo -n` when the plain run lacked permissions.
    pub sudo: bool,
    pub timeout_secs: u64,
}

const OS_CMD: &str = r#"echo "kernel=$(uname -s) $(uname -r)"; echo "arch=$(uname -m)"; echo "hostname=$(hostname 2>/dev/null || uname -n)"; echo "cpu_model=$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2-)"; echo "cpu_threads=$(nproc 2>/dev/null)"; echo "cpu_cores=$(grep -m1 'cpu cores' /proc/cpuinfo 2>/dev/null | cut -d: -f2-)"; echo "login_shell=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)"; [ -e /run/ostree-booted ] && echo "ostree=1"; for p in apt dnf yum pacman zypper apk nix-env emerge xbps-install snap flatpak brew; do command -v "$p" >/dev/null 2>&1 && echo "pm=$p"; done; ip -o addr show 2>/dev/null | awk '{print "netif=" $2 " " $4}'; cat /etc/os-release 2>/dev/null; true"#;

const SYSTEMD_CMD: &str = r#"command -v systemctl >/dev/null 2>&1 || exit 0; systemctl list-units --type=service --state=running --no-pager --no-legend --plain 2>/dev/null | head -n 300 | while read -r u _; do echo "== $u"; systemctl show -p MainPID -p WorkingDirectory -p ExecStart "$u" 2>/dev/null; done"#;

const DOCKER_CMD: &str =
    r"command -v docker >/dev/null 2>&1 || exit 0; docker ps --no-trunc --format '{{json .}}'";

const PROXMOX_CMD: &str = r"if command -v qm >/dev/null 2>&1; then echo '== qm'; qm list; fi; if command -v pct >/dev/null 2>&1; then echo '== pct'; pct list; fi; true";

const PORTS_CMD: &str = r"ss -H -tlnp 2>/dev/null || netstat -tlnp 2>/dev/null";

const CRON_CMD: &str = r"echo '== user'; crontab -l 2>/dev/null; echo '== system'; cat /etc/crontab /etc/cron.d/* 2>/dev/null; echo '== timers'; systemctl list-timers --all --no-pager --no-legend 2>/dev/null; true";

const WEB_CMD: &str = r#"for f in /etc/nginx/nginx.conf /etc/nginx/conf.d/*.conf /etc/nginx/sites-enabled/*; do [ -f "$f" ] && { echo "== nginx $f"; cat "$f"; }; done; for f in /etc/caddy/Caddyfile /etc/caddy/conf.d/*; do [ -f "$f" ] && { echo "== caddy $f"; cat "$f"; }; done; true"#;

const RESOURCES_CMD: &str = r"df -hP 2>/dev/null; echo '== free'; free -m 2>/dev/null; true";

const LANG_CMD: &str = r#"v() { command -v "$1" >/dev/null 2>&1 || return 0; o=$("$@" 2>&1 | head -n 1); printf '%s\t%s\n' "$1" "$o"; }; v python3 --version; v node --version; v go version; v rustc --version; v java -version; v php --version; v ruby --version; v dotnet --version; v perl -e 'print $^V'; v deno --version; v bun --version; v docker --version; v psql --version; v mysql --version; v redis-server --version; true"#;

const ENV_CMD: &str = r"find /srv /opt /var/www /home /root /etc /usr/local -maxdepth 5 \( -name node_modules -o -name .git -o -name proc \) -prune -o -type f \( -name .env -o -name '.env.*' -o -name '*.env' \) -print 2>&1 | head -n 300";

pub const STEPS: &[Step] = &[
    Step {
        id: "os",
        cmd: OS_CMD,
        sudo: false,
        timeout_secs: 30,
    },
    Step {
        id: "systemd",
        cmd: SYSTEMD_CMD,
        sudo: false,
        timeout_secs: 60,
    },
    Step {
        id: "docker",
        cmd: DOCKER_CMD,
        sudo: true,
        timeout_secs: 60,
    },
    Step {
        id: "proxmox",
        cmd: PROXMOX_CMD,
        sudo: true,
        timeout_secs: 60,
    },
    Step {
        id: "ports",
        cmd: PORTS_CMD,
        sudo: true,
        timeout_secs: 30,
    },
    Step {
        id: "cron",
        cmd: CRON_CMD,
        sudo: false,
        timeout_secs: 30,
    },
    Step {
        id: "web",
        cmd: WEB_CMD,
        sudo: true,
        timeout_secs: 30,
    },
    Step {
        id: "resources",
        cmd: RESOURCES_CMD,
        sudo: false,
        timeout_secs: 30,
    },
    Step {
        id: "languages",
        cmd: LANG_CMD,
        sudo: false,
        timeout_secs: 60,
    },
    Step {
        id: "envfiles",
        cmd: ENV_CMD,
        sudo: true,
        timeout_secs: 120,
    },
];

/// A step that could see more with `sudo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SudoOffer {
    pub step: String,
    /// Exactly what would run.
    pub command: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub snapshot: InfraSnapshot,
    /// The server in the same schema as this computer's `system.json` (`kor_health::system`), so
    /// the agent reads local and remote machines the same way (§15.16, §19.2).
    #[serde(default)]
    pub system: kor_health::system::Profile,
    /// Steps that lacked permissions; those not run with sudo stay in this list.
    pub sudo_offered: Vec<SudoOffer>,
    pub sudo_used: Vec<String>,
}

/// Asked once per step that lacked permissions: run it with `sudo -n`?
pub type AskSudo<'a> = dyn Fn(&SudoOffer) -> bool + Send + Sync + 'a;

fn sudo_command(cmd: &str) -> String {
    format!("sudo -n sh -c {}", sh_quote(cmd))
}

fn lacks_permission(step: &str, out: &Output) -> Option<String> {
    let all = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    )
    .to_lowercase();
    for marker in [
        "permission denied",
        "must be root",
        "are you root",
        "only root",
        "requires root",
    ] {
        if all.contains(marker) {
            return Some(marker.to_owned());
        }
    }
    // Listening sockets of other users' processes show no process without root.
    if step == "ports" {
        let text = String::from_utf8_lossy(&out.stdout);
        if text
            .lines()
            .any(|l| !l.trim().is_empty() && !l.contains("users:(") && !l.contains('/'))
        {
            return Some("process names of other users' sockets are hidden".into());
        }
    }
    None
}

/// Scan the machine behind `sh`. `address` is shown in the report; `host` is the file-safe id.
///
/// # Errors
/// When the first step (OS) cannot run at all: SSH connection, host key or login failure.
pub async fn discover(
    sh: &dyn Shell,
    address: &str,
    host: &str,
    now: u64,
    ask: &AskSudo<'_>,
) -> Result<Discovery, String> {
    let mut outs: HashMap<&'static str, String> = HashMap::new();
    let mut notes = Vec::new();
    let mut offered = Vec::new();
    let mut used = Vec::new();
    for step in STEPS {
        let timeout = Duration::from_secs(step.timeout_secs);
        let out = match sh.run(step.cmd, None, timeout).await {
            Ok(o) => o,
            Err(e) if step.id == "os" => return Err(e),
            Err(e) => {
                notes.push(format!("{}: {e}", step.id));
                continue;
            }
        };
        let mut text = out.text();
        if step.sudo
            && let Some(reason) = lacks_permission(step.id, &out)
        {
            let offer = SudoOffer {
                step: step.id.to_owned(),
                command: sudo_command(step.cmd),
                reason,
            };
            if ask(&offer) {
                match sh.run(&offer.command, None, timeout).await {
                    Ok(o) if o.ok() || !o.stdout.is_empty() => {
                        text = o.text();
                        used.push(step.id.to_owned());
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr).trim().to_owned();
                        let why = if err.contains("password") {
                            "sudo needs a password (only passwordless sudo works over batch SSH)"
                                .to_owned()
                        } else {
                            err
                        };
                        notes.push(format!("{} (sudo): {why}", step.id));
                        offered.push(offer);
                    }
                    Err(e) => {
                        notes.push(format!("{} (sudo): {e}", step.id));
                        offered.push(offer);
                    }
                }
            } else {
                notes.push(format!("{}: {} (sudo not used)", step.id, offer.reason));
                offered.push(offer);
            }
        }
        outs.insert(step.id, text);
    }
    let get = |id: &str| outs.get(id).map_or("", String::as_str);
    let mut snap = InfraSnapshot {
        host: host.to_owned(),
        address: address.to_owned(),
        taken_at: now,
        notes,
        ..InfraSnapshot::default()
    };
    parse_os(get("os"), &mut snap);
    let (mut services, pids) = parse_systemd(get("systemd"));
    services.extend(parse_docker(get("docker")));
    services.extend(parse_proxmox(get("proxmox")));
    let ports = parse_ports(get("ports"));
    for (port, pid) in &ports {
        if let Some(pid) = pid
            && let Some(unit) = pids.get(pid)
            && let Some(s) = services.iter_mut().find(|s| &s.name == unit)
            && !s.ports.contains(&port.port)
        {
            s.ports.push(port.port);
        }
    }
    snap.ports = ports.into_iter().map(|(p, _)| p).collect();
    snap.vhosts = parse_web(get("web"));
    link_vhosts(&mut snap.vhosts, &mut services, &snap.ports);
    snap.services = services;
    snap.schedules = parse_cron(get("cron"));
    (snap.disks, snap.memory) = parse_resources(get("resources"));
    snap.languages = parse_languages(get("languages"));
    snap.env_files = get("envfiles")
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/') && !l.contains(": "))
        .map(str::to_owned)
        .collect();
    let system = profile(get("os"), &snap);
    Ok(Discovery {
        snapshot: snap,
        system,
        sudo_offered: offered,
        sudo_used: used,
    })
}

/// `79G`, `796M`, `1.1T` (`df -h`) → MiB.
fn human_mb(s: &str) -> u64 {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let v: f64 = num.trim().replace(',', ".").parse().unwrap_or(0.0);
    let mul = match unit.chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('K') => 1.0 / 1024.0,
        Some('G') => 1024.0,
        Some('T') => 1024.0 * 1024.0,
        Some('P') => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    // Display-sized values; the float is exact far beyond any disk.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mb = (v * mul).round().max(0.0) as u64;
    mb
}

/// The scan as a `kor_health::system::Profile` (same schema as the local `system.json`).
fn profile(os_text: &str, snap: &InfraSnapshot) -> kor_health::system::Profile {
    use kor_health::system as hs;
    let field = |k: &str| {
        os_text
            .lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(|v| unquote(v).trim().to_owned())
            .unwrap_or_default()
    };
    let ostree = !field("ostree").is_empty();
    let distro = hs::parse_os_release(os_text, ostree);
    let kernel_name = snap
        .kernel
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut network: Vec<hs::NetIf> = Vec::new();
    for l in os_text.lines().filter_map(|l| l.strip_prefix("netif=")) {
        let mut w = l.split_whitespace();
        let (Some(name), Some(addr)) = (w.next(), w.next()) else {
            continue;
        };
        if name == "lo" {
            continue;
        }
        match network.iter_mut().find(|n| n.name == name) {
            Some(n) => n.addrs.push(addr.to_owned()),
            None => network.push(hs::NetIf {
                name: name.to_owned(),
                addrs: vec![addr.to_owned()],
            }),
        }
    }
    hs::Profile {
        collected_at: snap.taken_at,
        os: hs::OsInfo {
            family: if kernel_name == "darwin" {
                "macos".into()
            } else {
                kernel_name
            },
            name: distro
                .as_ref()
                .map_or_else(|| snap.os.clone(), |d| d.name.clone()),
            version: distro
                .as_ref()
                .map(|d| d.version.clone())
                .unwrap_or_default(),
            long_name: snap.os.clone(),
            kernel: snap.kernel.clone(),
            arch: snap.arch.clone(),
            hostname: snap.hostname.clone(),
            distro,
            package_managers: os_text
                .lines()
                .filter_map(|l| l.strip_prefix("pm="))
                .map(|s| s.trim().to_owned())
                .collect(),
            desktop: None,
        },
        cpu: hs::Cpu {
            model: field("cpu_model"),
            cores: field("cpu_cores").parse().ok(),
            threads: field("cpu_threads").parse().unwrap_or(0),
        },
        memory: snap
            .memory
            .as_ref()
            .map_or_else(hs::Memory::default, |m| hs::Memory {
                total_mb: m.total_mb,
                available_mb: m.available_mb,
                swap_total_mb: m.swap_total_mb,
                swap_used_mb: m.swap_used_mb,
            }),
        disks: snap
            .disks
            .iter()
            .map(|d| hs::Disk {
                mount: d.mount.clone(),
                name: d.mount.clone(),
                fs: String::new(),
                total_mb: human_mb(&d.size),
                free_mb: human_mb(&d.avail),
                removable: false,
            })
            .collect(),
        network,
        toolchains: snap
            .languages
            .iter()
            .map(|l| {
                (
                    if l.name == "python3" {
                        "python".to_owned()
                    } else {
                        l.name.clone()
                    },
                    l.version.clone(),
                )
            })
            .collect(),
        shell: Some(field("login_shell")).filter(|s| !s.is_empty()),
        ..hs::Profile::default()
    }
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"').trim_matches('\'')
}

fn parse_os(text: &str, snap: &mut InfraSnapshot) {
    let mut kv: BTreeMap<&str, &str> = BTreeMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim()).or_insert(unquote(v));
        }
    }
    for (k, field) in [
        ("kernel", &mut snap.kernel),
        ("arch", &mut snap.arch),
        ("hostname", &mut snap.hostname),
    ] {
        kv.get(k).copied().unwrap_or_default().clone_into(field);
    }
    snap.os = kv
        .get("PRETTY_NAME")
        .or_else(|| kv.get("NAME"))
        .copied()
        .unwrap_or_else(|| snap.kernel.split(' ').next().unwrap_or_default())
        .to_owned();
}

/// Running units with their command line, working directory and main pid.
fn parse_systemd(text: &str) -> (Vec<Service>, HashMap<u32, String>) {
    let mut out: Vec<Service> = Vec::new();
    let mut pids = HashMap::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            out.push(Service {
                kind: "systemd".into(),
                name: name.trim().to_owned(),
                state: "running".into(),
                ..Service::default()
            });
            continue;
        }
        let Some(s) = out.last_mut() else { continue };
        if let Some(v) = line.strip_prefix("MainPID=") {
            if let Ok(pid) = v.trim().parse::<u32>()
                && pid > 0
            {
                pids.insert(pid, s.name.clone());
            }
        } else if let Some(v) = line.strip_prefix("ExecStart=") {
            let argv = v
                .split_once("argv[]=")
                .map_or(v, |(_, a)| a)
                .split(" ; ignore_errors=")
                .next()
                .unwrap_or_default()
                .trim()
                .trim_end_matches('}')
                .trim();
            let mut d = argv.chars().take(300).collect::<String>();
            if !s.detail.is_empty() {
                d = format!("{d} {}", s.detail);
            }
            s.detail = d;
        } else if let Some(v) = line.strip_prefix("WorkingDirectory=") {
            let v = v.trim();
            if !v.is_empty() {
                let cwd = format!("(cwd {v})");
                s.detail = if s.detail.is_empty() {
                    cwd
                } else {
                    format!("{} {cwd}", s.detail)
                };
            }
        }
    }
    (out, pids)
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct DockerRow {
    names: String,
    image: String,
    state: String,
    status: String,
    labels: String,
    ports: String,
}

fn parse_docker(text: &str) -> Vec<Service> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(r) = serde_json::from_str::<DockerRow>(line.trim()) else {
            continue;
        };
        let labels: BTreeMap<&str, &str> = r
            .labels
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .collect();
        let mut ports: Vec<u16> = Vec::new();
        for part in r.ports.split(',') {
            if let Some((left, _)) = part.split_once("->")
                && let Some(p) = left.rsplit(':').next().and_then(|p| p.trim().parse().ok())
                && !ports.contains(&p)
            {
                ports.push(p);
            }
        }
        let dir = labels
            .get("com.docker.compose.project.working_dir")
            .map(|d| (*d).to_owned());
        out.push(Service {
            kind: "docker".into(),
            name: r.names.split(',').next().unwrap_or_default().to_owned(),
            state: if r.state.is_empty() {
                r.status
            } else {
                r.state
            },
            detail: dir.unwrap_or_else(|| r.image.clone()),
            image: (!r.image.is_empty()).then_some(r.image),
            compose_project: labels
                .get("com.docker.compose.project")
                .map(|s| (*s).to_owned()),
            compose_service: labels
                .get("com.docker.compose.service")
                .map(|s| (*s).to_owned()),
            ports,
            domains: Vec::new(),
        });
    }
    out
}

fn parse_proxmox(text: &str) -> Vec<Service> {
    let mut out = Vec::new();
    let mut section = "";
    for line in text.lines() {
        match line.trim() {
            "== qm" => {
                section = "vm";
                continue;
            }
            "== pct" => {
                section = "lxc";
                continue;
            }
            _ => {}
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() || t[0].parse::<u32>().is_err() {
            continue;
        }
        let (name, state) = match (section, t.len()) {
            ("vm", n) if n >= 3 => (t[1], t[2]),
            ("lxc", n) if n >= 3 => (t[n - 1], t[1]),
            _ => continue,
        };
        out.push(Service {
            kind: section.to_owned(),
            name: name.to_owned(),
            state: state.to_owned(),
            detail: format!("id {}", t[0]),
            ..Service::default()
        });
    }
    out
}

/// `ss -H -tlnp` or `netstat -tlnp` lines → listening ports with process and pid when visible.
fn parse_ports(text: &str) -> Vec<(Port, Option<u32>)> {
    let mut out: Vec<(Port, Option<u32>)> = Vec::new();
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let (local, process, pid) = if t.first().is_some_and(|f| f.starts_with("tcp")) {
            // netstat: proto recv send local foreign state pid/program
            if t.len() < 6 {
                continue;
            }
            let (pid, prog) = t
                .get(6)
                .and_then(|p| p.split_once('/'))
                .map_or((None, ""), |(a, b)| (a.parse().ok(), b));
            (t[3], prog.to_owned(), pid)
        } else if t.first() == Some(&"LISTEN") && t.len() >= 5 {
            let rest = t[5..].join(" ");
            let process = rest
                .split_once("((\"")
                .and_then(|(_, r)| r.split('"').next())
                .unwrap_or_default()
                .to_owned();
            let pid = rest
                .split_once("pid=")
                .and_then(|(_, r)| r.split([',', ')']).next())
                .and_then(|p| p.parse().ok());
            (t[3], process, pid)
        } else {
            continue;
        };
        let Some((addr, port)) = local.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        if out.iter().any(|(p, _)| p.port == port && p.address == addr) {
            continue;
        }
        out.push((
            Port {
                port,
                address: addr.to_owned(),
                process,
            },
            pid,
        ));
    }
    out.sort_by(|a, b| (a.0.port, &a.0.address).cmp(&(b.0.port, &b.0.address)));
    out
}

fn parse_cron(text: &str) -> Vec<Schedule> {
    let mut out = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let l = line.trim();
        if let Some(s) = l.strip_prefix("== ") {
            section = match s {
                "user" => "user",
                "system" => "system",
                _ => "timers",
            };
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let t: Vec<&str> = l.split_whitespace().collect();
        if section == "timers" {
            if let Some(i) = t
                .iter()
                .position(|w| w.to_ascii_lowercase().ends_with(".timer"))
            {
                let what = t.get(i + 1).copied().unwrap_or(t[i]);
                let when = t[..i.min(4)].join(" ");
                out.push(Schedule {
                    kind: "timer".into(),
                    when: if when.starts_with("n/a") || when.is_empty() {
                        t[i].to_owned()
                    } else {
                        format!("next {when}")
                    },
                    what: what.to_owned(),
                });
            }
            continue;
        }
        // Variable assignments (`PATH=...`, `MAILTO=x`).
        if t[0].contains('=') && !t[0].starts_with('@') {
            continue;
        }
        let user_field = usize::from(section == "system");
        let (when, rest) = if t[0].starts_with('@') {
            (t[0].to_owned(), &t[1..])
        } else if t.len() > 5 {
            (t[..5].join(" "), &t[5..])
        } else {
            continue;
        };
        let what = rest.get(user_field..).unwrap_or_default().join(" ");
        if what.is_empty() {
            continue;
        }
        out.push(Schedule {
            kind: "cron".into(),
            when,
            what: what.chars().take(300).collect(),
        });
    }
    out
}

/// Statements of an nginx config: `(depth, words)`, with `{` opening a block named by its words.
fn nginx_statements(text: &str) -> Vec<(usize, Vec<String>, bool)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    let clean: String = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    for c in clean.chars() {
        match c {
            ';' | '{' => {
                let words: Vec<String> = cur.split_whitespace().map(str::to_owned).collect();
                if !words.is_empty() {
                    out.push((depth, words, c == '{'));
                }
                cur.clear();
                if c == '{' {
                    depth += 1;
                }
            }
            '}' => {
                cur.clear();
                depth = depth.saturating_sub(1);
                out.push((depth, vec!["}".into()], false));
            }
            _ => cur.push(c),
        }
    }
    out
}

fn parse_nginx(text: &str, out: &mut Vec<VHost>) {
    let mut upstreams: HashMap<String, Vec<String>> = HashMap::new();
    let mut hosts: Vec<VHost> = Vec::new();
    // (block kind, depth at which it opened)
    let mut stack: Vec<(String, usize)> = Vec::new();
    for (depth, words, opens) in nginx_statements(text) {
        if words[0] == "}" {
            if stack.last().is_some_and(|(_, d)| *d == depth) {
                stack.pop();
            }
            continue;
        }
        if opens {
            let kind = match words[0].as_str() {
                "server" if stack.iter().all(|(k, _)| k != "server") => {
                    hosts.push(VHost {
                        server: "nginx".into(),
                        ..VHost::default()
                    });
                    "server".to_owned()
                }
                "upstream" if words.len() > 1 => format!("upstream {}", words[1]),
                other => other.to_owned(),
            };
            stack.push((kind, depth));
            continue;
        }
        let inside = stack
            .iter()
            .rev()
            .find(|(k, _)| k == "server" || k.starts_with("upstream "));
        match (inside.map(|(k, _)| k.as_str()), words[0].as_str()) {
            (Some(k), "server") if k.starts_with("upstream ") => {
                if let Some(target) = words.get(1) {
                    upstreams
                        .entry(k["upstream ".len()..].to_owned())
                        .or_default()
                        .push(target.clone());
                }
            }
            (Some("server"), "server_name") => {
                if let Some(h) = hosts.last_mut() {
                    h.names.extend(
                        words[1..]
                            .iter()
                            .filter(|n| *n != "_" && !n.is_empty())
                            .cloned(),
                    );
                }
            }
            (
                Some("server"),
                "proxy_pass" | "fastcgi_pass" | "uwsgi_pass" | "grpc_pass" | "root",
            ) => {
                if let (Some(h), Some(t)) = (hosts.last_mut(), words.get(1))
                    && !h.upstreams.contains(t)
                {
                    h.upstreams.push(t.clone());
                }
            }
            _ => {}
        }
    }
    for h in &mut hosts {
        let mut resolved = Vec::new();
        for u in &h.upstreams {
            let name = u
                .split("://")
                .nth(1)
                .unwrap_or(u)
                .split(['/', ':'])
                .next()
                .unwrap_or_default();
            match upstreams.get(name) {
                Some(targets) => resolved.extend(targets.iter().cloned()),
                None => resolved.push(u.clone()),
            }
        }
        h.upstreams = resolved;
    }
    out.extend(
        hosts
            .into_iter()
            .filter(|h| !h.names.is_empty() || !h.upstreams.is_empty()),
    );
}

fn parse_caddy(text: &str, out: &mut Vec<VHost>) {
    let mut depth = 0usize;
    let mut cur: Option<VHost> = None;
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or_default().trim();
        if l.is_empty() {
            continue;
        }
        if depth == 0 && l.ends_with('{') {
            let addrs = l.trim_end_matches('{').trim();
            if !addrs.is_empty() && !addrs.starts_with('(') {
                cur = Some(VHost {
                    server: "caddy".into(),
                    names: addrs
                        .split([',', ' '])
                        .filter(|a| !a.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    ..VHost::default()
                });
            }
            depth = 1;
            continue;
        }
        let words: Vec<&str> = l.split_whitespace().collect();
        if let Some(h) = cur.as_mut()
            && matches!(
                words.first(),
                Some(&"reverse_proxy" | &"php_fastcgi" | &"root")
            )
        {
            let targets = words[1..]
                .iter()
                .filter(|w| **w != "*" && **w != "{" && !w.starts_with('@'));
            h.upstreams.extend(targets.map(|w| (*w).to_owned()));
        }
        depth += l.matches('{').count();
        depth = depth.saturating_sub(l.matches('}').count());
        if depth == 0
            && let Some(h) = cur.take()
        {
            out.push(h);
        }
    }
    if let Some(h) = cur {
        out.push(h);
    }
}

fn parse_web(text: &str) -> Vec<VHost> {
    fn flush(kind: &str, body: &str, out: &mut Vec<VHost>) {
        match kind {
            "nginx" => parse_nginx(body, out),
            "caddy" => parse_caddy(body, out),
            _ => {}
        }
    }
    let mut out = Vec::new();
    let (mut kind, mut body) = ("", String::new());
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("== ") {
            flush(kind, &body, &mut out);
            body.clear();
            kind = if h.starts_with("caddy") {
                "caddy"
            } else {
                "nginx"
            };
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    flush(kind, &body, &mut out);
    out
}

fn upstream_port(u: &str) -> Option<u16> {
    let hostport = u.split("://").nth(1).unwrap_or(u);
    let hostport = hostport.split('/').next().unwrap_or(hostport);
    hostport.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
}

/// vhost → services by upstream port; services learn their domains.
fn link_vhosts(vhosts: &mut [VHost], services: &mut [Service], ports: &[Port]) {
    for v in vhosts.iter_mut() {
        for u in &v.upstreams {
            let Some(port) = upstream_port(u) else {
                continue;
            };
            let mut hit: Vec<usize> = services
                .iter()
                .enumerate()
                .filter(|(_, s)| s.ports.contains(&port))
                .map(|(i, _)| i)
                .collect();
            if hit.is_empty()
                && let Some(p) = ports
                    .iter()
                    .find(|p| p.port == port && !p.process.is_empty())
            {
                hit = services
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.name.trim_end_matches(".service") == p.process)
                    .map(|(i, _)| i)
                    .collect();
            }
            for i in hit {
                let s = &mut services[i];
                if !v.services.contains(&s.name) {
                    v.services.push(s.name.clone());
                }
                for n in &v.names {
                    if !s.domains.contains(n) {
                        s.domains.push(n.clone());
                    }
                }
            }
        }
    }
}

fn mb(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}

fn parse_resources(text: &str) -> (Vec<Disk>, Option<Memory>) {
    let mut disks = Vec::new();
    let mut mem: Option<Memory> = None;
    let mut in_free = false;
    for line in text.lines() {
        if line.trim() == "== free" {
            in_free = true;
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        if in_free {
            let m = mem.get_or_insert_with(Memory::default);
            match t.first() {
                Some(&"Mem:") if t.len() >= 3 => {
                    m.total_mb = mb(t[1]);
                    m.used_mb = mb(t[2]);
                    m.available_mb = t.get(6).map_or(0, |v| mb(v));
                }
                Some(&"Swap:") if t.len() >= 3 => {
                    m.swap_total_mb = mb(t[1]);
                    m.swap_used_mb = mb(t[2]);
                }
                _ => {}
            }
            continue;
        }
        if t.len() < 6 || t[0] == "Filesystem" {
            continue;
        }
        let fs = t[0];
        if ["tmpfs", "devtmpfs", "udev", "overlay", "shm", "none"].contains(&fs)
            || fs.starts_with("/dev/loop")
        {
            continue;
        }
        disks.push(Disk {
            mount: t[5..].join(" "),
            size: t[1].to_owned(),
            used: t[2].to_owned(),
            avail: t[3].to_owned(),
            use_pct: t[4].to_owned(),
        });
    }
    (disks, mem.filter(|m| m.total_mb > 0))
}

fn parse_languages(text: &str) -> Vec<ToolVersion> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let version = rest
            .split(|c: char| c.is_whitespace() || c == '"' || c == ',')
            .map(|w| w.trim_start_matches(|c: char| c.is_ascii_alphabetic()))
            .find(|w| {
                w.split('.').count() >= 2 && w.chars().next().is_some_and(|c| c.is_ascii_digit())
            })
            .unwrap_or(rest.trim())
            .chars()
            .take(60)
            .collect();
        out.push(ToolVersion {
            name: name.trim().to_owned(),
            version,
        });
    }
    out
}

/// Markdown report (`<kor home>/infra/<host>.md`), the memory form of a scan.
#[allow(clippy::too_many_lines)] // one section after another
pub fn markdown(d: &Discovery) -> String {
    let s = &d.snapshot;
    let mut o = String::new();
    let when = chrono::DateTime::from_timestamp(i64::try_from(s.taken_at).unwrap_or(0), 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();
    let _ = writeln!(
        o,
        "# {} ({})\n",
        if s.hostname.is_empty() {
            &s.host
        } else {
            &s.hostname
        },
        s.address
    );
    let _ = writeln!(
        o,
        "> {}\n> Read-only scan by `kor discover` at {when}. Re-run it to refresh.\n",
        super::UNTRUSTED
    );
    let _ = writeln!(
        o,
        "- OS: {}\n- Kernel: {}\n- Arch: {}",
        s.os, s.kernel, s.arch
    );
    if let Some(m) = &s.memory {
        let _ = writeln!(
            o,
            "- Memory: {} MB used of {} MB ({} MB available), swap {}/{} MB",
            m.used_mb, m.total_mb, m.available_mb, m.swap_used_mb, m.swap_total_mb
        );
    }
    let section = |o: &mut String, title: &str, empty: bool| {
        let _ = writeln!(o, "\n## {title}\n");
        if empty {
            o.push_str("_None found._\n");
        }
        !empty
    };
    if section(&mut o, "Services", s.services.is_empty()) {
        o.push_str(
            "| Kind | Name | State | Ports | Domains | Detail |\n|---|---|---|---|---|---|\n",
        );
        for v in &s.services {
            let ports: Vec<String> = v.ports.iter().map(u16::to_string).collect();
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} | {} | {} |",
                v.kind,
                cell(&v.name),
                cell(&v.state),
                ports.join(", "),
                cell(&v.domains.join(", ")),
                cell(&v.detail)
            );
        }
    }
    if section(&mut o, "Open ports (TCP, listening)", s.ports.is_empty()) {
        o.push_str("| Port | Address | Process |\n|---|---|---|\n");
        for p in &s.ports {
            let _ = writeln!(
                o,
                "| {} | {} | {} |",
                p.port,
                cell(&p.address),
                cell(&p.process)
            );
        }
    }
    if section(&mut o, "Virtual hosts", s.vhosts.is_empty()) {
        o.push_str("| Server | Names | Upstreams | Services |\n|---|---|---|---|\n");
        for v in &s.vhosts {
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} |",
                v.server,
                cell(&v.names.join(", ")),
                cell(&v.upstreams.join(", ")),
                cell(&v.services.join(", "))
            );
        }
    }
    if section(&mut o, "Cron jobs and timers", s.schedules.is_empty()) {
        for c in &s.schedules {
            let _ = writeln!(o, "- {} `{}`: {}", c.kind, c.when, cell(&c.what));
        }
    }
    if section(&mut o, "Disks", s.disks.is_empty()) {
        o.push_str("| Mount | Size | Used | Free | Use |\n|---|---|---|---|---|\n");
        for d in &s.disks {
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} | {} |",
                cell(&d.mount),
                d.size,
                d.used,
                d.avail,
                d.use_pct
            );
        }
    }
    if section(&mut o, "Languages and tools", s.languages.is_empty()) {
        for l in &s.languages {
            let _ = writeln!(o, "- {} {}", l.name, cell(&l.version));
        }
    }
    if section(
        &mut o,
        "Env files (paths only, contents never read)",
        s.env_files.is_empty(),
    ) {
        for f in &s.env_files {
            let _ = writeln!(o, "- `{f}`");
        }
    }
    if !s.notes.is_empty() {
        let _ = writeln!(o, "\n## Notes\n");
        for n in &s.notes {
            let _ = writeln!(o, "- {}", cell(n));
        }
    }
    o
}

/// One table cell: no pipes or newlines from the remote side.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Write `<dir>/<host>.md` and `<dir>/<host>.json` (Kor home `infra/`). Returns both paths.
///
/// # Errors
/// If the host id is invalid or the files cannot be written.
pub fn save(dir: &Path, d: &Discovery) -> Result<(PathBuf, PathBuf), String> {
    let json = kor_graph::infra::save(dir, &d.snapshot).map_err(|e| e.to_string())?;
    // `<host>.system.json`: same schema as this computer's `system.json`.
    let sys = dir.join(format!("{}.system.json", d.snapshot.host));
    let body = serde_json::to_vec_pretty(&d.system).map_err(|e| e.to_string())?;
    std::fs::write(&sys, body).map_err(|e| e.to_string())?;
    let md = dir.join(format!("{}.md", d.snapshot.host));
    let tmp = dir.join(format!("{}.md.tmp", d.snapshot.host));
    std::fs::write(&tmp, markdown(d)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &md).map_err(|e| e.to_string())?;
    Ok((md, json))
}

/// Steps offered for sudo, by id (for `--sudo a,b` checks).
pub fn step_ids() -> BTreeSet<&'static str> {
    STEPS.iter().filter(|s| s.sudo).map(|s| s.id).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Fake `ssh`: replays recorded outputs by the first command fragment that matches and keeps
    /// every command it was asked to run.
    pub(crate) struct Recorded {
        pub(crate) replies: Vec<(&'static str, Output)>,
        pub(crate) seen: Mutex<Vec<String>>,
    }

    pub(crate) fn out(stdout: &str) -> Output {
        Output {
            code: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[async_trait]
    impl Shell for Recorded {
        async fn run(&self, cmd: &str, _: Option<&[u8]>, _: Duration) -> Result<Output, String> {
            self.seen.lock().expect("lock").push(cmd.to_owned());
            Ok(self
                .replies
                .iter()
                .find(|(frag, _)| cmd.contains(frag))
                .map(|(_, o)| o.clone())
                .unwrap_or_default())
        }
    }

    const OS: &str = "kernel=Linux 6.1.0-18-amd64\narch=x86_64\nhostname=web1\ncpu_model= Intel(R) Xeon(R) CPU E5-2680 v4\ncpu_threads=4\ncpu_cores= 2\nlogin_shell=/bin/bash\npm=apt\npm=snap\nnetif=lo 127.0.0.1/8\nnetif=eth0 10.0.0.5/24\nnetif=eth0 fe80::1/64\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nNAME=\"Debian GNU/Linux\"\nID=debian\nVERSION_ID=\"12\"\n";
    const SYSTEMD: &str = "== nginx.service\nMainPID=812\nWorkingDirectory=\nExecStart={ path=/usr/sbin/nginx ; argv[]=/usr/sbin/nginx -g daemon on; master_process on; ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }\n== shop-api.service\nMainPID=1201\nWorkingDirectory=/srv/shop\nExecStart={ path=/srv/shop/bin/server ; argv[]=/srv/shop/bin/server --port 3000 ; ignore_errors=no ; start_time=[n/a] }\n";
    const DOCKER: &str = r#"{"Command":"\"docker-entrypoint.s…\"","ID":"a1","Image":"ghcr.io/acme/blog:2.1","Labels":"com.docker.compose.project=blog,com.docker.compose.service=web,com.docker.compose.project.working_dir=/opt/blog","Names":"blog-web-1","Ports":"0.0.0.0:8080->80/tcp, :::8080->80/tcp","State":"running","Status":"Up 3 days"}
{"ID":"b2","Image":"redis:7","Labels":"","Names":"cache","Ports":"6379/tcp","State":"running","Status":"Up 3 days"}
"#;
    const PORTS_PLAIN: &str = "LISTEN 0 511 0.0.0.0:80 0.0.0.0:*\nLISTEN 0 4096 127.0.0.1:3000 0.0.0.0:*\nLISTEN 0 4096 0.0.0.0:8080 0.0.0.0:*\nLISTEN 0 128 [::]:22 [::]:*\n";
    const PORTS_ROOT: &str = "LISTEN 0 511 0.0.0.0:80 0.0.0.0:* users:((\"nginx\",pid=812,fd=6),(\"nginx\",pid=813,fd=6))\nLISTEN 0 4096 127.0.0.1:3000 0.0.0.0:* users:((\"server\",pid=1201,fd=9))\nLISTEN 0 4096 0.0.0.0:8080 0.0.0.0:* users:((\"docker-proxy\",pid=990,fd=4))\nLISTEN 0 128 [::]:22 [::]:* users:((\"sshd\",pid=700,fd=4))\n";
    const CRON: &str = "== user\n# m h dom mon dow command\nMAILTO=ops@example.com\n*/5 * * * * /srv/shop/bin/sync >/dev/null 2>&1\n== system\nSHELL=/bin/sh\n17 *\t* * *\troot    cd / && run-parts --report /etc/cron.hourly\n@reboot root /usr/local/bin/warm\n== timers\nMon 2026-11-10 00:00:00 UTC 5h left Sun 2026-11-09 00:00:00 UTC 18h ago logrotate.timer logrotate.service\n";
    const WEB: &str = "== nginx /etc/nginx/sites-enabled/shop\nupstream shop_api { server 127.0.0.1:3000; }\nserver {\n  listen 443 ssl;\n  server_name shop.example.com www.shop.example.com; # main\n  location / { proxy_pass http://shop_api; }\n}\n== caddy /etc/caddy/Caddyfile\n{\n  email ops@example.com\n}\nblog.example.com {\n  reverse_proxy localhost:8080\n}\n";
    const RES: &str = "Filesystem      Size  Used Avail Use% Mounted on\nudev            3.9G     0  3.9G   0% /dev\n/dev/sda1        79G   31G   45G  41% /\ntmpfs           796M  1.1M  795M   1% /run\n== free\n               total        used        free      shared  buff/cache   available\nMem:            7951        2810         410          42        4730        4870\nSwap:           1023          12        1011\n";
    const LANGS: &str = "python3\tPython 3.11.2\nnode\tv20.11.1\ngo\tgo version go1.22.1 linux/amd64\njava\topenjdk version \"17.0.10\" 2024-01-16\n";
    const ENVS: &str =
        "/srv/shop/.env\n/opt/blog/.env.production\nfind: '/root': Permission denied\n";

    fn denied() -> Output {
        Output {
            code: Some(1),
            stdout: Vec::new(),
            stderr: b"permission denied while trying to connect to the Docker daemon socket"
                .to_vec(),
        }
    }

    fn server(sudo_ok: bool) -> Recorded {
        let mut replies = vec![
            ("uname -s", out(OS)),
            ("list-units", out(SYSTEMD)),
            ("crontab -l", out(CRON)),
            ("/etc/nginx", out(WEB)),
            ("df -hP", out(RES)),
            ("v python3", out(LANGS)),
            ("find /srv", out(ENVS)),
            ("qm list", out("")),
        ];
        if sudo_ok {
            replies.insert(0, ("sudo -n sh -c 'command -v docker", out(DOCKER)));
            replies.insert(0, ("sudo -n sh -c 'ss -H", out(PORTS_ROOT)));
        } else {
            replies.insert(
                0,
                (
                    "sudo -n",
                    Output {
                        code: Some(1),
                        stdout: Vec::new(),
                        stderr: b"sudo: a password is required".to_vec(),
                    },
                ),
            );
        }
        replies.push(("docker ps", denied()));
        replies.push(("ss -H", out(PORTS_PLAIN)));
        Recorded {
            replies,
            seen: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn full_scan_from_recorded_outputs() {
        let sh = server(true);
        let allow = |o: &SudoOffer| o.step == "docker" || o.step == "ports";
        let d = discover(&sh, "deploy@web1", "web1", 1_762_732_800, &allow)
            .await
            .expect("scan");
        let s = &d.snapshot;
        assert_eq!(s.os, "Debian GNU/Linux 12 (bookworm)");
        // Same schema as the local `system.json`.
        let p = &d.system;
        assert_eq!(
            (
                p.os.family.as_str(),
                p.os.name.as_str(),
                p.os.version.as_str()
            ),
            ("linux", "Debian GNU/Linux", "12")
        );
        assert_eq!(
            p.os.distro.as_ref().map(|x| x.family.as_str()),
            Some("debian")
        );
        assert_eq!(p.os.package_managers, ["apt", "snap"]);
        assert_eq!(
            (p.cpu.model.as_str(), p.cpu.cores, p.cpu.threads),
            ("Intel(R) Xeon(R) CPU E5-2680 v4", Some(2), 4)
        );
        assert_eq!((p.memory.total_mb, p.memory.available_mb), (7951, 4870));
        assert_eq!(p.disks.len(), 1);
        assert_eq!(
            (
                p.disks[0].mount.as_str(),
                p.disks[0].total_mb,
                p.disks[0].free_mb
            ),
            ("/", 79 * 1024, 45 * 1024)
        );
        assert_eq!(p.network.len(), 1);
        assert_eq!(p.network[0].addrs, ["10.0.0.5/24", "fe80::1/64"]);
        assert_eq!(
            p.toolchains.get("python").map(String::as_str),
            Some("3.11.2")
        );
        assert_eq!(p.shell.as_deref(), Some("/bin/bash"));
        assert_eq!((s.arch.as_str(), s.hostname.as_str()), ("x86_64", "web1"));
        let svc = |n: &str| s.services.iter().find(|x| x.name == n).expect(n);
        let api = svc("shop-api.service");
        assert_eq!(api.ports, vec![3000]);
        assert!(
            api.detail.contains("/srv/shop/bin/server --port 3000")
                && api.detail.contains("(cwd /srv/shop)")
        );
        assert_eq!(
            api.domains,
            vec!["shop.example.com", "www.shop.example.com"]
        );
        assert_eq!(svc("nginx.service").ports, vec![80]);
        let blog = svc("blog-web-1");
        assert_eq!(blog.compose_project.as_deref(), Some("blog"));
        assert_eq!(blog.ports, vec![8080]);
        assert_eq!(blog.domains, vec!["blog.example.com"]);
        assert_eq!(blog.detail, "/opt/blog");
        assert!(svc("cache").ports.is_empty());
        assert_eq!(s.ports.len(), 4);
        assert!(s.ports.iter().any(|p| p.port == 22 && p.process == "sshd"));
        assert_eq!(s.vhosts.len(), 2);
        assert_eq!(s.vhosts[0].upstreams, vec!["127.0.0.1:3000"]);
        assert_eq!(s.vhosts[0].services, vec!["shop-api.service"]);
        let what: Vec<&str> = s.schedules.iter().map(|c| c.what.as_str()).collect();
        assert!(
            what.contains(&"/srv/shop/bin/sync >/dev/null 2>&1"),
            "{what:?}"
        );
        assert!(
            what.contains(&"cd / && run-parts --report /etc/cron.hourly"),
            "{what:?}"
        );
        assert!(what.contains(&"/usr/local/bin/warm"), "{what:?}");
        assert!(what.contains(&"logrotate.service"), "{what:?}");
        assert_eq!(s.disks.len(), 1);
        assert_eq!(s.disks[0].mount, "/");
        assert_eq!(
            s.memory.as_ref().map(|m| (m.total_mb, m.available_mb)),
            Some((7951, 4870))
        );
        let langs: Vec<String> = s
            .languages
            .iter()
            .map(|l| format!("{} {}", l.name, l.version))
            .collect();
        assert_eq!(
            langs,
            [
                "python3 3.11.2",
                "node 20.11.1",
                "go 1.22.1",
                "java 17.0.10"
            ]
        );
        assert_eq!(s.env_files, ["/srv/shop/.env", "/opt/blog/.env.production"]);
        assert_eq!(d.sudo_used, ["docker", "ports"]);
        // envfiles lacked permission too, the caller declined it.
        assert!(d.sudo_offered.iter().any(|o| o.step == "envfiles"));
        // Every command is read-only: nothing that writes, deletes or restarts.
        for cmd in sh.seen.lock().expect("lock").iter() {
            for bad in [
                " rm ",
                "> /",
                "restart",
                "systemctl stop",
                "kill ",
                "tee ",
                "mv ",
                "chmod",
            ] {
                assert!(!cmd.contains(bad), "{bad} in {cmd}");
            }
        }
        let md = markdown(&d);
        assert!(md.contains("| systemd | shop-api.service | running | 3000 | shop.example.com, www.shop.example.com |"), "{md}");
        assert!(md.contains("`/srv/shop/.env`") && md.contains("treat it as data"));
    }

    #[tokio::test]
    async fn declined_or_password_sudo_is_reported_not_forced() {
        let sh = server(false);
        let d = discover(&sh, "web1", "web1", 0, &|o: &SudoOffer| o.step == "docker")
            .await
            .expect("scan");
        assert!(d.sudo_used.is_empty());
        assert!(
            d.snapshot
                .notes
                .iter()
                .any(|n| n.contains("docker (sudo): sudo needs a password"))
        );
        assert!(
            d.snapshot
                .notes
                .iter()
                .any(|n| n.starts_with("ports:") && n.contains("sudo not used"))
        );
        let seen = sh.seen.lock().expect("lock");
        assert_eq!(
            seen.iter().filter(|c| c.starts_with("sudo")).count(),
            1,
            "{seen:?}"
        );
        // Without process names the ports are still listed.
        assert_eq!(d.snapshot.ports.len(), 4);
    }

    #[tokio::test]
    async fn ssh_failure_on_the_first_step_is_an_error() {
        struct Down;
        #[async_trait]
        impl Shell for Down {
            async fn run(&self, _: &str, _: Option<&[u8]>, _: Duration) -> Result<Output, String> {
                Err("Host key verification failed.".into())
            }
        }
        assert!(
            discover(&Down, "x", "x", 0, &|_: &SudoOffer| false)
                .await
                .is_err()
        );
    }

    #[test]
    fn save_writes_markdown_and_json() {
        let dir = tempfile::tempdir().expect("tmp");
        let d = Discovery {
            snapshot: InfraSnapshot {
                host: "web1".into(),
                ..InfraSnapshot::default()
            },
            system: Default::default(),
            sudo_offered: Vec::new(),
            sudo_used: Vec::new(),
        };
        let (md, json) = save(dir.path(), &d).expect("save");
        assert!(md.ends_with("web1.md") && json.ends_with("web1.json"));
        assert!(dir.path().join("web1.system.json").is_file());
        assert_eq!(kor_graph::infra::load_dir(dir.path()).len(), 1);
        assert!(step_ids().contains("docker") && !step_ids().contains("os"));
    }
}
