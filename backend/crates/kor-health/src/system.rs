//! Local system profile (§19.2): OS and distribution, package managers, desktop, CPU, RAM, GPUs
//! (VRAM), disks, battery, network, toolchains, shell and terminal, displays, keyboard layout.
//!
//! Written to `<kor home>/system.json` on start and every `health.refresh_minutes`; a refresh that
//! finds a different machine state (new disk, Docker installed, network changed…) reports the
//! changed keys as a `system.changed` event. The agent gets a short, budgeted summary in its
//! system prompt so it never asks "which OS?" or writes `apt` on Fedora.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_platform::desktop_probe::{Battery, Gpu};
use serde::{Deserialize, Serialize};

pub const FILE: &str = "system.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Distro {
    /// `ubuntu`, `fedora`, `arch`, `nixos`…
    pub id: String,
    /// `debian`, `rhel`, `arch`, `suse`, `alpine`, `nixos`, `gentoo` or the id itself.
    pub family: String,
    pub name: String,
    pub version: String,
    /// Image-based system (Silverblue, Kinoite, CoreOS, `ostree`): install with flatpak/toolbox.
    pub immutable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsInfo {
    /// `windows`, `macos`, `linux`, …
    pub family: String,
    pub name: String,
    pub version: String,
    pub long_name: String,
    pub kernel: String,
    pub arch: String,
    pub hostname: String,
    pub distro: Option<Distro>,
    pub package_managers: Vec<String>,
    /// Linux desktop and session type (`GNOME (wayland)`).
    pub desktop: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cpu {
    pub model: String,
    pub cores: Option<usize>,
    pub threads: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub total_mb: u64,
    pub available_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Disk {
    pub mount: String,
    pub name: String,
    pub fs: String,
    pub total_mb: u64,
    pub free_mb: u64,
    pub removable: bool,
}

impl Disk {
    pub fn used_percent(&self) -> u8 {
        if self.total_mb == 0 {
            return 0;
        }
        u8::try_from((self.total_mb - self.free_mb.min(self.total_mb)) * 100 / self.total_mb).unwrap_or(100)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetIf {
    pub name: String,
    pub addrs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub primary: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keyboard {
    /// As the OS names it (`0001041F`, `com.apple.keylayout.Turkish`, `tr:f`).
    pub raw: String,
    /// Language (`tr`, `en`, `fr`…), when known.
    pub lang: Option<String>,
    /// `F` or `Q` for Turkish, else the variant name.
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Unix seconds.
    pub collected_at: u64,
    pub os: OsInfo,
    pub cpu: Cpu,
    pub memory: Memory,
    pub gpus: Vec<Gpu>,
    pub disks: Vec<Disk>,
    pub battery: Option<Battery>,
    pub network: Vec<NetIf>,
    /// `node` → `22.11.0`.
    pub toolchains: BTreeMap<String, String>,
    pub shell: Option<String>,
    pub terminal: Option<String>,
    pub displays: Vec<Display>,
    pub keyboard: Option<Keyboard>,
}

/// Where profiles come from (tests use a fake).
pub trait Probe: Send + Sync {
    fn collect(&self) -> Profile;
}

/// The real machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealProbe;

impl Probe for RealProbe {
    fn collect(&self) -> Profile {
        collect_real()
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

const PACKAGE_MANAGERS: &[&str] = &[
    "winget", "choco", "scoop", "brew", "port", "apt", "dnf", "yum", "pacman", "zypper", "apk", "nix-env",
    "emerge", "xbps-install", "rpm-ostree", "flatpak", "snap",
];

/// Toolchains: name and the arguments that print its version.
const TOOLCHAINS: &[(&str, &[&str])] = &[
    ("node", &["--version"]),
    ("npm", &["--version"]),
    ("pnpm", &["--version"]),
    ("yarn", &["--version"]),
    ("bun", &["--version"]),
    ("deno", &["--version"]),
    ("python", &["--version"]),
    ("rustc", &["--version"]),
    ("cargo", &["--version"]),
    ("go", &["version"]),
    ("java", &["-version"]),
    ("dotnet", &["--version"]),
    ("php", &["--version"]),
    ("ruby", &["--version"]),
    ("docker", &["--version"]),
    ("podman", &["--version"]),
    ("git", &["--version"]),
    ("gcc", &["--version"]),
    ("clang", &["--version"]),
    ("cmake", &["--version"]),
    ("kubectl", &["version", "--client"]),
    ("terraform", &["--version"]),
];

/// First version-looking token (`v22.11.0` → `22.11.0`, `go1.23.2` → `1.23.2`).
pub fn version_of(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '(' || c == ')')
        .map(|t| t.trim_start_matches(|c: char| c.is_ascii_alphabetic()))
        .find(|t| {
            let mut parts = t.split('.');
            parts.next().is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
                && parts.next().is_some_and(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(|t| t.trim_end_matches(|c: char| !c.is_ascii_alphanumeric()).to_owned())
}

fn toolchains() -> BTreeMap<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut started = 0;
    for (name, args) in TOOLCHAINS {
        let exe = if *name == "python" { kor_platform::python_command() } else { name };
        let Some(path) = kor_platform::on_path::which(exe) else {
            continue;
        };
        started += 1;
        let tx = tx.clone();
        std::thread::spawn(move || {
            use kor_platform::NoWindow as _;
            let out = std::process::Command::new(path)
                .args(*args)
                .no_window()
                .stdin(std::process::Stdio::null())
                .output();
            let v = out.ok().and_then(|o| {
                // `java -version` writes to stderr.
                let text = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                version_of(&text)
            });
            let _ = tx.send((*name, v));
        });
    }
    drop(tx);
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut out = BTreeMap::new();
    for _ in 0..started {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok((name, Some(v))) => {
                out.insert(name.to_owned(), v);
            }
            Ok((_, None)) => {}
            Err(_) => break,
        }
    }
    out
}

/// `/etc/os-release` text → distribution.
pub fn parse_os_release(text: &str, ostree_booted: bool) -> Option<Distro> {
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_owned())
            .unwrap_or_default()
    };
    let id = field("ID");
    if id.is_empty() {
        return None;
    }
    let like = field("ID_LIKE");
    let variant = field("VARIANT_ID");
    let words: Vec<&str> = std::iter::once(id.as_str()).chain(like.split_whitespace()).collect();
    let family = ["debian", "rhel", "fedora", "arch", "suse", "alpine", "nixos", "gentoo", "void"]
        .iter()
        .find(|f| words.contains(f))
        .map_or_else(|| id.clone(), |f| if *f == "fedora" { "rhel".to_owned() } else { (*f).to_owned() });
    let immutable = ostree_booted
        || ["silverblue", "kinoite", "coreos", "iot", "sericea", "onyx"].contains(&variant.as_str())
        || ["fedora-coreos", "flatcar", "endless", "vanilla"].contains(&id.as_str());
    Some(Distro {
        name: {
            let n = field("NAME");
            if n.is_empty() { id.clone() } else { n }
        },
        version: field("VERSION_ID"),
        id,
        family,
        immutable,
    })
}

/// The OS's keyboard layout name → language and variant (Turkish F / Q awareness, §19.1).
pub fn keyboard(raw: &str) -> Keyboard {
    let raw = raw.trim();
    let (lang, variant): (Option<&str>, Option<String>) = if raw.len() == 8 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        // Windows KLID: low word is the language id, high word the layout variant.
        let langid = raw[4..].to_ascii_uppercase();
        let lang = match langid.as_str() {
            "041F" => Some("tr"),
            "0409" | "0809" | "1009" | "0C09" => Some("en"),
            "040C" | "080C" | "0C0C" | "100C" => Some("fr"),
            "0407" | "0807" => Some("de"),
            "040A" | "0C0A" | "080A" => Some("es"),
            "0410" => Some("it"),
            "0419" => Some("ru"),
            "0416" | "0816" => Some("pt"),
            "0413" => Some("nl"),
            "0415" => Some("pl"),
            "0411" => Some("ja"),
            "0412" => Some("ko"),
            "0804" | "0404" => Some("zh"),
            "0401" => Some("ar"),
            _ => None,
        };
        let variant = (lang == Some("tr")).then(|| if &raw[..4] == "0001" { "F" } else { "Q" }.to_owned());
        (lang, variant)
    } else if let Some(name) = raw.strip_prefix("com.apple.keylayout.") {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("turkish") {
            (Some("tr"), Some(if lower.contains("qwerty") { "Q" } else { "F" }.to_owned()))
        } else {
            let lang = [("us", "en"), ("abc", "en"), ("british", "en"), ("french", "fr"), ("german", "de"), ("spanish", "es"), ("italian", "it"), ("russian", "ru")]
                .iter()
                .find(|(k, _)| lower.starts_with(k))
                .map(|(_, l)| *l);
            (lang, None)
        }
    } else {
        // Linux `layout[:variant]`.
        let (layout, variant) = raw.split_once(':').map_or((raw, None), |(a, b)| (a, Some(b)));
        let lang = match layout {
            "us" | "gb" => Some("en"),
            "" => None,
            l => Some(l),
        };
        let variant = if layout == "tr" {
            Some(if variant == Some("f") { "F" } else { "Q" }.to_owned())
        } else {
            variant.map(str::to_owned)
        };
        (lang, variant)
    };
    Keyboard {
        raw: raw.to_owned(),
        lang: lang.map(str::to_owned),
        variant,
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn shell() -> Option<String> {
    env("SHELL")
        .map(|s| kor_platform::desktop_probe::app_name(&s))
        .or_else(|| env("PSModulePath").map(|_| "powershell".to_owned()))
        .or_else(|| env("ComSpec").map(|s| kor_platform::desktop_probe::app_name(&s)))
}

fn terminal() -> Option<String> {
    env("TERM_PROGRAM")
        .or_else(|| env("WT_SESSION").map(|_| "Windows Terminal".to_owned()))
        .or_else(|| env("TERMINAL_EMULATOR"))
        .or_else(|| env("KONSOLE_VERSION").map(|_| "konsole".to_owned()))
        .or_else(|| env("GNOME_TERMINAL_SCREEN").map(|_| "gnome-terminal".to_owned()))
        .or_else(|| env("TERM"))
}

fn desktop() -> Option<String> {
    let de = env("XDG_CURRENT_DESKTOP").or_else(|| env("DESKTOP_SESSION"))?;
    Some(match env("XDG_SESSION_TYPE") {
        Some(t) => format!("{de} ({t})"),
        None => de,
    })
}

fn displays() -> Vec<Display> {
    kor_computer::live::displays()
        .into_iter()
        .map(|d| Display {
            name: d.name,
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            scale: d.scale,
            primary: d.primary,
        })
        .collect()
}

fn collect_real() -> Profile {
    use sysinfo::{Disks, Networks, System};
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    let family = std::env::consts::OS.to_owned();
    let distro = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| parse_os_release(&t, Path::new("/run/ostree-booted").exists()));
    let os = OsInfo {
        family,
        name: System::name().unwrap_or_default(),
        version: System::os_version().unwrap_or_default(),
        long_name: System::long_os_version().unwrap_or_default(),
        kernel: System::kernel_version().unwrap_or_default(),
        arch: System::cpu_arch(),
        hostname: System::host_name().unwrap_or_default(),
        distro,
        package_managers: PACKAGE_MANAGERS
            .iter()
            .filter(|p| kor_platform::on_path::which(p).is_some())
            .map(|p| (*p).to_owned())
            .collect(),
        desktop: desktop(),
    };
    let cpu = Cpu {
        model: sys.cpus().first().map(|c| c.brand().trim().to_owned()).unwrap_or_default(),
        cores: System::physical_core_count(),
        threads: sys.cpus().len(),
    };
    let memory = Memory {
        total_mb: mb(sys.total_memory()),
        available_mb: mb(sys.available_memory()),
        swap_total_mb: mb(sys.total_swap()),
        swap_used_mb: mb(sys.used_swap()),
    };
    let disks = disks(&Disks::new_with_refreshed_list());
    let mut network: Vec<NetIf> = Networks::new_with_refreshed_list()
        .iter()
        .map(|(name, data)| NetIf {
            name: name.clone(),
            addrs: data
                .ip_networks()
                .iter()
                .filter(|n| !n.addr.is_loopback() && !n.addr.is_unspecified())
                .map(|n| n.addr.to_string())
                .collect(),
        })
        .filter(|n| !n.addrs.is_empty())
        .collect();
    network.sort_by(|a, b| a.name.cmp(&b.name));
    Profile {
        collected_at: now_secs(),
        os,
        cpu,
        memory,
        gpus: kor_platform::desktop_probe::gpus(),
        disks,
        battery: kor_platform::desktop_probe::battery(),
        network,
        toolchains: toolchains(),
        shell: shell(),
        terminal: terminal(),
        displays: displays(),
        keyboard: kor_platform::desktop_probe::keyboard_layout().map(|k| keyboard(&k)),
    }
}

/// Real, non-virtual disks.
pub(crate) fn disks(list: &sysinfo::Disks) -> Vec<Disk> {
    let mut v: Vec<Disk> = list
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| Disk {
            mount: d.mount_point().to_string_lossy().into_owned(),
            name: d.name().to_string_lossy().into_owned(),
            fs: d.file_system().to_string_lossy().into_owned(),
            total_mb: mb(d.total_space()),
            free_mb: mb(d.available_space()),
            removable: d.is_removable(),
        })
        .filter(|d| !["tmpfs", "devtmpfs", "overlay", "squashfs", "proc", "sysfs"].contains(&d.fs.as_str()))
        .collect();
    v.sort_by(|a, b| a.mount.cmp(&b.mount));
    v.dedup_by(|a, b| a.mount == b.mount);
    v
}

/// What differs between two profiles (stable facts only; free space, available RAM, battery charge,
/// shell and terminal, which depend on who asks, are not changes).
pub fn changes(old: &Profile, new: &Profile) -> Vec<String> {
    let mut out = Vec::new();
    let mut check = |key: &str, same: bool| {
        if !same {
            out.push(key.to_owned());
        }
    };
    check("os", old.os.version == new.os.version && old.os.name == new.os.name && old.os.distro == new.os.distro);
    check("package_managers", old.os.package_managers == new.os.package_managers);
    check("cpu", old.cpu.model == new.cpu.model);
    check("memory", old.memory.total_mb.abs_diff(new.memory.total_mb) < 512);
    check("gpus", old.gpus == new.gpus);
    let mounts = |p: &Profile| p.disks.iter().map(|d| d.mount.clone()).collect::<Vec<_>>();
    check("disks", mounts(old) == mounts(new));
    check("battery", old.battery.is_some() == new.battery.is_some());
    check("network", old.network == new.network);
    check("displays", old.displays.len() == new.displays.len()
        && old.displays.iter().zip(&new.displays).all(|(a, b)| (a.width, a.height, a.primary) == (b.width, b.height, b.primary)));
    check("keyboard", old.keyboard == new.keyboard);
    for (k, v) in &new.toolchains {
        if old.toolchains.get(k) != Some(v) {
            out.push(format!("toolchains.{k}"));
        }
    }
    for k in old.toolchains.keys() {
        if !new.toolchains.contains_key(k) {
            out.push(format!("toolchains.{k}"));
        }
    }
    out
}

/// `<home>/system.json`.
pub fn path(home: &Path) -> PathBuf {
    home.join(FILE)
}

pub fn load(home: &Path) -> Option<Profile> {
    serde_json::from_str(&std::fs::read_to_string(path(home)).ok()?).ok()
}

/// A refresh: the new profile and what changed since the stored one (`system.changed`).
#[derive(Debug, Clone, Serialize)]
pub struct Refresh {
    pub profile: Profile,
    /// Empty on the first run and when nothing stable changed.
    pub changed: Vec<String>,
}

/// Collect with `probe`, store in `home`, and report changes.
pub fn refresh(home: &Path, probe: &dyn Probe) -> std::io::Result<Refresh> {
    let profile = probe.collect();
    let changed = load(home).map(|old| changes(&old, &profile)).unwrap_or_default();
    std::fs::create_dir_all(home)?;
    let tmp = path(home).with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&profile).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path(home))?;
    Ok(Refresh { profile, changed })
}

/// The stored profile is missing or older than `minutes`.
pub fn stale(home: &Path, minutes: u32) -> bool {
    load(home).is_none_or(|p| now_secs().saturating_sub(p.collected_at) > u64::from(minutes.max(1)) * 60)
}

fn gb(mbytes: u64) -> String {
    if mbytes >= 10 * 1024 {
        format!("{} GB", (mbytes + 512) / 1024)
    } else {
        format!("{:.1} GB", mbytes as f64 / 1024.0)
    }
}

/// The profile for the agent's system prompt, at most `max_chars` characters.
#[allow(clippy::cast_precision_loss)] // sizes shown to one decimal
pub fn prompt_section(p: &Profile, max_chars: usize) -> String {
    let mut lines = Vec::new();
    let mut os = format!("OS: {}", if p.os.long_name.is_empty() { &p.os.name } else { &p.os.long_name });
    if !p.os.arch.is_empty() {
        os.push_str(&format!(" ({})", p.os.arch));
    }
    if let Some(d) = &p.os.distro {
        os.push_str(&format!("; distribution {} {} (family {}{})", d.id, d.version, d.family, if d.immutable { ", immutable" } else { "" }));
    }
    if let Some(de) = &p.os.desktop {
        os.push_str(&format!("; desktop {de}"));
    }
    lines.push(os);
    let mut env_line = Vec::new();
    if !p.os.package_managers.is_empty() {
        env_line.push(format!("package managers: {}", p.os.package_managers.join(", ")));
    }
    if let Some(s) = &p.shell {
        env_line.push(format!("shell: {s}"));
    }
    if let Some(t) = &p.terminal {
        env_line.push(format!("terminal: {t}"));
    }
    if let Some(k) = &p.keyboard {
        let mut s = format!("keyboard: {}", k.lang.as_deref().unwrap_or(&k.raw));
        if let Some(v) = &k.variant {
            s.push_str(&format!(" ({v})"));
        }
        env_line.push(s);
    }
    if !env_line.is_empty() {
        lines.push(env_line.join("; "));
    }
    let mut hw = format!(
        "CPU: {} ({} threads); RAM {} ({} free)",
        p.cpu.model,
        p.cpu.threads,
        gb(p.memory.total_mb),
        gb(p.memory.available_mb)
    );
    for g in &p.gpus {
        hw.push_str(&format!("; GPU {}", g.name));
        if let Some(v) = g.vram_mb {
            hw.push_str(&format!(" {} VRAM", gb(v)));
        }
    }
    lines.push(hw);
    if !p.disks.is_empty() {
        lines.push(format!(
            "Disks: {}",
            p.disks
                .iter()
                .map(|d| format!("{} {} free of {}", d.mount, gb(d.free_mb), gb(d.total_mb)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !p.toolchains.is_empty() {
        lines.push(format!(
            "Installed: {}",
            p.toolchains.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
        ));
    }
    if !p.displays.is_empty() {
        lines.push(format!(
            "Displays: {}",
            p.displays
                .iter()
                .map(|d| format!("{}x{}{}", d.width, d.height, if d.primary { " (primary)" } else { "" }))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(b) = p.battery {
        lines.push(format!("Battery: {}%{}", b.percent, if b.charging { ", charging" } else { "" }));
    }
    let mut out = String::from(
        "\n\n# This machine\nUse commands, package managers and paths for this system; do not ask the user which OS they use.\n",
    );
    for l in lines {
        if out.len() + l.len() + 1 > max_chars {
            break;
        }
        out.push_str(&l);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeProbe(Mutex<Vec<Profile>>);

    impl Probe for FakeProbe {
        fn collect(&self) -> Profile {
            self.0.lock().unwrap().remove(0)
        }
    }

    fn sample() -> Profile {
        let mut p = Profile {
            collected_at: now_secs(),
            ..Profile::default()
        };
        p.os = OsInfo {
            family: "linux".into(),
            name: "Fedora Linux".into(),
            version: "41".into(),
            long_name: "Linux 41 Fedora Linux".into(),
            arch: "x86_64".into(),
            distro: parse_os_release("NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=41\nVARIANT_ID=silverblue\n", false),
            package_managers: vec!["dnf".into(), "flatpak".into()],
            ..OsInfo::default()
        };
        p.memory.total_mb = 32 * 1024;
        p.memory.available_mb = 20 * 1024;
        p.gpus = vec![Gpu { name: "RTX 4070".into(), vram_mb: Some(12282) }];
        p.disks = vec![Disk { mount: "/".into(), total_mb: 512_000, free_mb: 40_000, ..Disk::default() }];
        p.toolchains.insert("node".into(), "22.11.0".into());
        p.keyboard = Some(keyboard("tr:f"));
        p
    }

    #[test]
    fn refresh_stores_and_reports_changes() {
        let d = tempfile::tempdir().unwrap();
        let a = sample();
        let mut b = a.clone();
        b.disks.push(Disk { mount: "/mnt/usb".into(), total_mb: 64_000, free_mb: 60_000, removable: true, ..Disk::default() });
        b.toolchains.insert("docker".into(), "27.3.1".into());
        b.memory.available_mb = 1024; // volatile: not a change
        b.collected_at += 10;
        let probe = FakeProbe(Mutex::new(vec![a.clone(), b, a]));
        assert!(stale(d.path(), 5));
        let r = refresh(d.path(), &probe).unwrap();
        assert!(r.changed.is_empty(), "first run reports nothing");
        assert!(!stale(d.path(), 5));
        let r = refresh(d.path(), &probe).unwrap();
        assert_eq!(r.changed, ["disks", "toolchains.docker"]);
        assert_eq!(load(d.path()).unwrap().disks.len(), 2);
        let r = refresh(d.path(), &probe).unwrap();
        assert_eq!(r.changed, ["disks", "toolchains.docker"], "removal is a change too");
    }

    #[test]
    fn prompt_section_is_budgeted_and_names_the_system() {
        let p = sample();
        let s = prompt_section(&p, 2000);
        assert!(s.contains("distribution fedora 41 (family rhel, immutable)"), "{s}");
        assert!(s.contains("package managers: dnf, flatpak"), "{s}");
        assert!(s.contains("keyboard: tr (F)"), "{s}");
        assert!(s.contains("GPU RTX 4070 12 GB VRAM"), "{s}");
        assert!(s.contains("node 22.11.0"), "{s}");
        let short = prompt_section(&p, 200);
        assert!(short.len() <= 200 && short.contains("OS: "), "{short}");
    }

    #[test]
    fn parsers() {
        let deb = parse_os_release("ID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\nNAME=\"Ubuntu\"", false).unwrap();
        assert_eq!((deb.family.as_str(), deb.immutable), ("debian", false));
        let nix = parse_os_release("ID=nixos\nNAME=NixOS", false).unwrap();
        assert_eq!(nix.family, "nixos");
        assert!(parse_os_release("ID=fedora", true).unwrap().immutable);
        assert!(parse_os_release("", false).is_none());

        assert_eq!(keyboard("0001041F").variant.as_deref(), Some("F"));
        assert_eq!(keyboard("0000041F").variant.as_deref(), Some("Q"));
        assert_eq!(keyboard("00000409").lang.as_deref(), Some("en"));
        assert_eq!(keyboard("com.apple.keylayout.Turkish").variant.as_deref(), Some("F"));
        assert_eq!(keyboard("com.apple.keylayout.Turkish-QWERTY-PC").variant.as_deref(), Some("Q"));
        assert_eq!(keyboard("com.apple.keylayout.French").lang.as_deref(), Some("fr"));
        assert_eq!(keyboard("tr").variant.as_deref(), Some("Q"));
        assert_eq!(keyboard("us").lang.as_deref(), Some("en"));

        assert_eq!(version_of("v22.11.0\n").as_deref(), Some("22.11.0"));
        assert_eq!(version_of("go version go1.23.2 windows/amd64").as_deref(), Some("1.23.2"));
        assert_eq!(version_of("openjdk version \"21.0.4\" 2024-07-16").as_deref(), Some("21.0.4"));
        assert_eq!(version_of("Docker version 27.3.1, build ce12230").as_deref(), Some("27.3.1"));
        assert_eq!(version_of("git version 2.47.0.windows.2").as_deref(), Some("2.47.0.windows.2"));
        assert_eq!(version_of("no digits"), None);

        let d = Disk { total_mb: 100, free_mb: 4, ..Disk::default() };
        assert_eq!(d.used_percent(), 96);
    }

    /// Reads the real machine (no changes). `cargo test -p kor-health -- --ignored real_profile`
    #[test]
    #[ignore = "probes the real machine"]
    fn real_profile() {
        let p = RealProbe.collect();
        eprintln!("{}", prompt_section(&p, 4000));
        assert!(!p.os.family.is_empty() && p.memory.total_mb > 0 && p.cpu.threads > 0);
    }
}
