//! Layer C system prerequisites (spec §24.3, ADR 0064): the few things Kor cannot put in its own
//! folder (Docker Desktop / Podman, WSL2, Xcode Command Line Tools, `WebView2`) and the macOS privacy
//! permissions. Kor shows what is missing, why, and the exact command; it runs that command only
//! after the user said yes, and the OS asks for the password (UAC, `sudo`/`pkexec`), never Kor.
//!
//! [`plan`] is pure (a [`Host`] in, a [`Plan`] out) so every OS and package manager is tested on
//! any machine; [`Host::detect`] and [`command`] touch the real system.

use std::process::{Command, Stdio};

use serde::Serialize;

use crate::Os;

/// Something only the operating system can install or grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Prereq {
    /// Docker Desktop / Docker Engine: projects with a `Dockerfile` or compose file, Docker sandbox.
    Docker,
    /// Podman: the daemonless alternative to Docker.
    Podman,
    /// WSL2 (Windows): the Linux sandbox backend.
    Wsl2,
    /// Xcode Command Line Tools (macOS): `git` and compilers.
    XcodeClt,
    /// Microsoft Edge `WebView2` Runtime (Windows): the desktop window (installers bundle it).
    Webview2,
    /// macOS Accessibility permission: computer use (§19).
    Accessibility,
    /// macOS Screen Recording permission: screenshots and window capture (§19, §10).
    ScreenRecording,
    /// macOS Microphone permission: `kor voice` (§10).
    Microphone,
}

impl Prereq {
    pub const ALL: [Self; 8] = [
        Self::Docker,
        Self::Podman,
        Self::Wsl2,
        Self::XcodeClt,
        Self::Webview2,
        Self::Accessibility,
        Self::ScreenRecording,
        Self::Microphone,
    ];

    /// Stable id for the CLI, the desktop and translations (`docker`, `xcode-clt`, …).
    pub fn id(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Wsl2 => "wsl2",
            Self::XcodeClt => "xcode-clt",
            Self::Webview2 => "webview2",
            Self::Accessibility => "accessibility",
            Self::ScreenRecording => "screen-recording",
            Self::Microphone => "microphone",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Whether this exists as a separate thing on `os` at all.
    pub fn applies(self, os: Os) -> bool {
        match self {
            Self::Docker | Self::Podman => matches!(os, Os::Windows | Os::Mac | Os::Linux),
            Self::Wsl2 | Self::Webview2 => os == Os::Windows,
            Self::XcodeClt | Self::Accessibility | Self::ScreenRecording | Self::Microphone => os == Os::Mac,
        }
    }

    /// A permission (opened in System Settings) rather than a package.
    pub fn is_permission(self) -> bool {
        matches!(self, Self::Accessibility | Self::ScreenRecording | Self::Microphone)
    }
}

/// System package managers Kor knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Manager {
    Winget,
    Choco,
    Brew,
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Apk,
    Nix,
}

impl Manager {
    pub const ALL: [Self; 9] =
        [Self::Winget, Self::Choco, Self::Brew, Self::Apt, Self::Dnf, Self::Pacman, Self::Zypper, Self::Apk, Self::Nix];

    pub fn id(self) -> &'static str {
        match self {
            Self::Winget => "winget",
            Self::Choco => "choco",
            Self::Brew => "brew",
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
            Self::Nix => "nix",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The executable looked up on `PATH`.
    fn program(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            m => m.id(),
        }
    }

    /// Package name of `p` in this manager's main repository (`None`: not packaged there).
    fn package(self, p: Prereq) -> Option<&'static str> {
        use Manager as M;
        use Prereq as P;
        Some(match (p, self) {
            (P::Docker, M::Winget) => "Docker.DockerDesktop",
            (P::Docker, M::Choco) => "docker-desktop",
            (P::Docker, M::Apt) => "docker.io",
            (P::Docker, M::Dnf) => "moby-engine",
            (P::Docker, M::Brew | M::Pacman | M::Zypper | M::Apk) => "docker",
            (P::Docker, M::Nix) => "nixpkgs#docker",
            (P::Podman, M::Winget) => "RedHat.Podman",
            (P::Podman, M::Choco) => "podman-cli",
            (P::Podman, M::Nix) => "nixpkgs#podman",
            (P::Podman, _) => "podman",
            (P::Webview2, M::Winget) => "Microsoft.EdgeWebView2Runtime",
            (P::Webview2, M::Choco) => "webview2-runtime",
            _ => return None,
        })
    }

    /// Install command and how it gets elevated. `--yes`-style flags only skip the manager's own
    /// question: the user already answered Kor's, and the OS password prompt still appears.
    fn install(self, p: Prereq, package: &str) -> (Vec<String>, Elevation) {
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        match self {
            // winget asks for UAC by itself when the installer is machine-wide.
            Self::Winget => (
                v(&["install", "--id", package, "--exact", "--source", "winget", "--accept-package-agreements", "--accept-source-agreements"]),
                Elevation::None,
            ),
            Self::Choco => (v(&["install", package, "-y"]), Elevation::Uac),
            Self::Brew if p == Prereq::Docker => (v(&["install", "--cask", package]), Elevation::None),
            Self::Brew => (v(&["install", package]), Elevation::None),
            Self::Apt | Self::Dnf => (v(&["install", "-y", package]), Elevation::Sudo),
            Self::Pacman => (v(&["-S", "--needed", "--noconfirm", package]), Elevation::Sudo),
            Self::Zypper => (v(&["--non-interactive", "install", package]), Elevation::Sudo),
            Self::Apk => (v(&["add", package]), Elevation::Sudo),
            Self::Nix => (v(&["profile", "install", package]), Elevation::None),
        }
    }
}

/// How a command gets administrator rights; the OS asks, Kor never sees the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Elevation {
    None,
    /// Windows UAC prompt (`Start-Process -Verb RunAs`).
    Uac,
    /// `sudo` in a terminal, `pkexec` (polkit dialog) from the desktop app.
    Sudo,
}

/// What running the plan does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Install,
    /// Opens the System Settings pane where the user grants a permission.
    OpenSettings,
}

/// The exact command for one prerequisite on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub prereq: Prereq,
    pub manager: Option<Manager>,
    pub action: Action,
    pub program: String,
    pub args: Vec<String>,
    pub elevation: Elevation,
}

impl Plan {
    /// The command line as it runs, elevation wrapper included (`gui`: from the desktop app).
    pub fn argv(&self, gui: bool) -> Vec<String> {
        let mut inner = vec![self.program.clone()];
        inner.extend(self.args.iter().cloned());
        match self.elevation {
            Elevation::None => inner,
            Elevation::Sudo => {
                let mut out = vec![if gui { "pkexec" } else { "sudo" }.to_owned()];
                out.extend(inner);
                out
            }
            Elevation::Uac => {
                let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
                let list = self.args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(",");
                let script = format!(
                    "$p = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
                    quote(&self.program),
                    if list.is_empty() { "@()".to_owned() } else { list }
                );
                vec!["powershell".into(), "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script]
            }
        }
    }

    /// [`Plan::argv`] as one line for the confirmation and `--dry-run` (arguments with spaces quoted).
    pub fn command_line(&self, gui: bool) -> String {
        self.argv(gui)
            .iter()
            .map(|a| if a.is_empty() || a.contains([' ', '"', '\'', '$', ';']) { format!("\"{}\"", a.replace('"', "\\\"")) } else { a.clone() })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Why there is no plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum NoPlan {
    /// Does not exist on this OS (WSL2 on Linux, Xcode on Windows).
    NotNeeded,
    /// None of the package managers that carry it was found.
    NoManager,
}

/// The machine as far as installing goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub os: Os,
    /// Found on `PATH`, the distribution's own manager first.
    pub managers: Vec<Manager>,
    /// Running as root / administrator already (no `sudo` wrapper).
    pub root: bool,
}

impl Host {
    /// The real machine: package managers on `PATH`, ordered by `/etc/os-release` on Linux.
    pub fn detect() -> Self {
        let os = crate::os();
        let found = std::env::var_os("PATH").map(|p| crate::on_path::executables_on(&p)).unwrap_or_default();
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let managers = preferred(os, &release).into_iter().filter(|m| found.contains_key(m.program())).collect();
        Self { os, managers, root: is_root() }
    }
}

/// Managers in the order to use them on `os`; `os_release` is `/etc/os-release` (Linux).
fn preferred(os: Os, os_release: &str) -> Vec<Manager> {
    match os {
        Os::Windows => vec![Manager::Winget, Manager::Choco],
        Os::Mac => vec![Manager::Brew, Manager::Nix],
        Os::Linux => {
            let ids: Vec<&str> = os_release
                .lines()
                .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
                .flat_map(|v| v.trim_matches('"').split_whitespace())
                .collect();
            let native = |m: Manager| -> bool {
                let family: &[&str] = match m {
                    Manager::Apt => &["debian", "ubuntu"],
                    Manager::Dnf => &["fedora", "rhel", "centos"],
                    Manager::Pacman => &["arch"],
                    Manager::Zypper => &["opensuse", "suse", "sles"],
                    Manager::Apk => &["alpine"],
                    Manager::Nix => &["nixos"],
                    _ => &[],
                };
                ids.iter().any(|id| family.contains(id))
            };
            let all = [Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Zypper, Manager::Apk, Manager::Nix];
            let mut out: Vec<Manager> = all.into_iter().filter(|m| native(*m)).collect();
            out.extend(all.into_iter().filter(|m| !native(*m)));
            out
        }
        Os::Other => Vec::new(),
    }
}

fn is_root() -> bool {
    match crate::os() {
        Os::Windows => Command::new("net").arg("session").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()),
        _ => Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout.trim_ascii() == b"0"),
    }
}

/// The command for `p` on `host`; `prefer` picks a manager when several are present.
pub fn plan(p: Prereq, host: &Host, prefer: Option<Manager>) -> Result<Plan, NoPlan> {
    if !p.applies(host.os) {
        return Err(NoPlan::NotNeeded);
    }
    let fixed = |program: &str, args: &[&str], action: Action, elevation: Elevation| Plan {
        prereq: p,
        manager: None,
        action,
        program: program.to_owned(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        elevation,
    };
    let settings = |pane: &str| {
        fixed("open", &[&format!("x-apple.systempreferences:com.apple.preference.security?{pane}")], Action::OpenSettings, Elevation::None)
    };
    match p {
        Prereq::Wsl2 => return Ok(fixed("wsl", &["--install", "--no-distribution"], Action::Install, Elevation::Uac)),
        // Apple's own installer dialog; no package manager carries it.
        Prereq::XcodeClt => return Ok(fixed("xcode-select", &["--install"], Action::Install, Elevation::None)),
        Prereq::Accessibility => return Ok(settings("Privacy_Accessibility")),
        Prereq::ScreenRecording => return Ok(settings("Privacy_ScreenCapture")),
        Prereq::Microphone => return Ok(settings("Privacy_Microphone")),
        Prereq::Docker | Prereq::Podman | Prereq::Webview2 => {}
    }
    let order = prefer.into_iter().chain(host.managers.iter().copied());
    let (manager, package) = order
        .filter(|m| host.managers.contains(m))
        .find_map(|m| m.package(p).map(|pkg| (m, pkg)))
        .ok_or(NoPlan::NoManager)?;
    let (args, elevation) = manager.install(p, package);
    let elevation = if host.root { Elevation::None } else { elevation };
    Ok(Plan { prereq: p, manager: Some(manager), action: Action::Install, program: manager.program().to_owned(), args, elevation })
}

/// Whether `p` is present: `Some(true/false)` when it can be probed, `None` for permissions
/// (only the OS knows; it asks on first use).
pub fn installed(p: Prereq) -> Option<bool> {
    let ok = |program: &str, args: &[&str]| {
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    };
    match p {
        Prereq::Docker => Some(ok("docker", &["--version"])),
        Prereq::Podman => Some(ok("podman", &["--version"])),
        Prereq::Wsl2 => Some(ok("wsl", &["--status"])),
        Prereq::XcodeClt => Some(ok("xcode-select", &["-p"])),
        // The Evergreen runtime registers its version (`pv`) per machine or per user.
        Prereq::Webview2 => Some(["HKLM\\SOFTWARE\\WOW6432Node", "HKLM\\SOFTWARE", "HKCU\\Software"].iter().any(|root| {
            ok("reg", &["query", &format!("{root}\\Microsoft\\EdgeUpdate\\Clients\\{{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}}"), "/v", "pv"])
        })),
        Prereq::Accessibility | Prereq::ScreenRecording | Prereq::Microphone => None,
    }
}

/// One row of `kor prereq list` / the desktop list.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub prereq: Prereq,
    pub installed: Option<bool>,
    pub plan: Result<Plan, NoPlan>,
}

/// Every prerequisite that applies to `host`, probed.
pub fn status(host: &Host) -> Vec<Status> {
    Prereq::ALL
        .into_iter()
        .filter(|p| p.applies(host.os))
        .map(|p| Status { prereq: p, installed: installed(p), plan: plan(p, host, None) })
        .collect()
}

/// The process for `plan`; the caller wires stdio (terminal: inherit, desktop: piped) and must
/// have the user's yes first. Never adds flags that hide the OS elevation prompt.
pub fn command(plan: &Plan, gui: bool) -> Command {
    let argv = plan.argv(gui);
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(os: Os, managers: &[Manager]) -> Host {
        Host { os, managers: managers.to_vec(), root: false }
    }

    #[test]
    fn ids_round_trip() {
        for p in Prereq::ALL {
            assert_eq!(Prereq::parse(p.id()), Some(p));
        }
        for m in Manager::ALL {
            assert_eq!(Manager::parse(m.id()), Some(m));
        }
        assert_eq!(Prereq::parse("nope"), None);
    }

    #[test]
    fn windows_plans() {
        let h = host(Os::Windows, &[Manager::Winget, Manager::Choco]);
        let d = plan(Prereq::Docker, &h, None).unwrap();
        assert_eq!(d.manager, Some(Manager::Winget));
        assert_eq!(
            d.command_line(true),
            "winget install --id Docker.DockerDesktop --exact --source winget --accept-package-agreements --accept-source-agreements"
        );
        let c = plan(Prereq::Podman, &h, Some(Manager::Choco)).unwrap();
        assert_eq!(c.elevation, Elevation::Uac);
        let argv = c.argv(true);
        assert_eq!(argv[..4], ["powershell", "-NoProfile", "-NonInteractive", "-Command"]);
        assert_eq!(argv[4], "$p = Start-Process -FilePath 'choco' -ArgumentList 'install','podman-cli','-y' -Verb RunAs -Wait -PassThru; exit $p.ExitCode");
        let w = plan(Prereq::Wsl2, &h, None).unwrap();
        assert!(w.argv(false)[4].contains("-FilePath 'wsl' -ArgumentList '--install','--no-distribution' -Verb RunAs"));
        assert_eq!(plan(Prereq::Webview2, &host(Os::Windows, &[]), None), Err(NoPlan::NoManager));
        assert_eq!(plan(Prereq::XcodeClt, &h, None), Err(NoPlan::NotNeeded));
    }

    #[test]
    fn mac_plans() {
        let h = host(Os::Mac, &[Manager::Brew]);
        assert_eq!(plan(Prereq::Docker, &h, None).unwrap().command_line(true), "brew install --cask docker");
        assert_eq!(plan(Prereq::Podman, &h, None).unwrap().command_line(true), "brew install podman");
        assert_eq!(plan(Prereq::XcodeClt, &h, None).unwrap().command_line(true), "xcode-select --install");
        let s = plan(Prereq::ScreenRecording, &host(Os::Mac, &[]), None).unwrap();
        assert_eq!(s.action, Action::OpenSettings);
        assert_eq!(s.args, ["x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"]);
        assert_eq!(plan(Prereq::Wsl2, &h, None), Err(NoPlan::NotNeeded));
    }

    #[test]
    fn linux_plans_per_distribution() {
        let cases = [
            (Manager::Apt, "sudo apt-get install -y docker.io", "pkexec apt-get install -y docker.io"),
            (Manager::Dnf, "sudo dnf install -y moby-engine", "pkexec dnf install -y moby-engine"),
            (Manager::Pacman, "sudo pacman -S --needed --noconfirm docker", "pkexec pacman -S --needed --noconfirm docker"),
            (Manager::Zypper, "sudo zypper --non-interactive install docker", "pkexec zypper --non-interactive install docker"),
            (Manager::Apk, "sudo apk add docker", "pkexec apk add docker"),
            (Manager::Nix, "nix profile install nixpkgs#docker", "nix profile install nixpkgs#docker"),
        ];
        for (m, terminal, gui) in cases {
            let p = plan(Prereq::Docker, &host(Os::Linux, &[m]), None).unwrap();
            assert_eq!(p.command_line(false), terminal);
            assert_eq!(p.command_line(true), gui);
        }
        let root = Host { os: Os::Linux, managers: vec![Manager::Dnf], root: true };
        assert_eq!(plan(Prereq::Podman, &root, None).unwrap().command_line(false), "dnf install -y podman");
        assert_eq!(plan(Prereq::Webview2, &root, None), Err(NoPlan::NotNeeded));
    }

    #[test]
    fn distribution_order_from_os_release() {
        let fedora = "NAME=\"Fedora Linux\"\nID=fedora\n";
        assert_eq!(preferred(Os::Linux, fedora)[0], Manager::Dnf);
        let mint = "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        assert_eq!(preferred(Os::Linux, mint)[0], Manager::Apt);
        assert_eq!(preferred(Os::Linux, "ID=alpine\n")[0], Manager::Apk);
        assert_eq!(preferred(Os::Linux, "ID=arch\n")[0], Manager::Pacman);
        assert_eq!(preferred(Os::Linux, "ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n")[0], Manager::Zypper);
        assert_eq!(preferred(Os::Linux, "").len(), 6);
        // With apt and nix both present on Ubuntu, apt is used; `prefer` overrides.
        let h = host(Os::Linux, &[Manager::Apt, Manager::Nix]);
        assert_eq!(plan(Prereq::Podman, &h, None).unwrap().manager, Some(Manager::Apt));
        assert_eq!(plan(Prereq::Podman, &h, Some(Manager::Nix)).unwrap().manager, Some(Manager::Nix));
        // A preferred manager that is not installed is ignored.
        assert_eq!(plan(Prereq::Podman, &h, Some(Manager::Dnf)).unwrap().manager, Some(Manager::Apt));
    }

    #[test]
    fn quoting_and_status_shape() {
        let p = Plan {
            prereq: Prereq::Docker,
            manager: None,
            action: Action::Install,
            program: "it's".into(),
            args: vec!["a b".into()],
            elevation: Elevation::Uac,
        };
        assert!(p.argv(false)[4].starts_with("$p = Start-Process -FilePath 'it''s' -ArgumentList 'a b' "));
        let h = Host::detect();
        for s in status(&h) {
            assert!(s.prereq.applies(h.os));
            let json = serde_json::to_value(&s).unwrap();
            assert!(json["plan"].is_object(), "{json}");
        }
    }
}
