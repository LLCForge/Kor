//! Operating-system-specific integrations (Master Build Prompt §27.10, ADR 0052).
//!
//! One catalog per OS; every item is installed only when the user asks
//! (`kor os-integration install <id>|--all`, or the desktop settings card) and uninstall removes
//! exactly what Kor created: written files and directories are listed in
//! `<data dir>/os-integration.json`, a file that is in the way and not Kor's is never touched.
//!
//! - Windows: `PowerToys` Run plugin, Windows Terminal "Kor" profile, WSL2 awareness (built in,
//!   [`crate::wsl`]), Start menu search (installer shortcut + `~/Kor` indexing), Jump List
//!   (desktop, [`crate::jumplist`]), Windows 11 Widgets (packaged with the sparse package).
//! - macOS: Shortcuts.app actions, Core Spotlight index of sessions, Universal Clipboard
//!   detection, Handoff (all three in the desktop process, [`mac`]), menu bar (tray) and Dock menu.
//! - Linux: GNOME Shell extension, KDE Plasma widget, `kor://` handler (`xdg-open`), Wayland
//!   portal permissions on one screen ([`portals`], desktop), `systemd --user` daemon service,
//!   Flatpak permission set (manifest).
//!
//! Paths (home, data, `LOCALAPPDATA`) and the program runner are injected ([`Env`]), so tests
//! never touch the real system.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::{NoWindow as _, Os};

pub mod mac;
pub mod portals;
pub mod shortcuts;

/// How an item is managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Files and commands written by this module.
    Install,
    /// Switched on here, carried out by the desktop app while it runs (Spotlight, Handoff…).
    DesktopFlag,
    /// Always on; status only.
    Builtin,
    /// Installed through another Kor surface (named), status shown here.
    Delegated(&'static str),
}

/// One integration in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub id: &'static str,
    pub os: Os,
    pub kind: Kind,
}

impl Entry {
    /// i18n id of the label (`osx-item-<id>`).
    pub fn label_id(&self) -> String {
        format!("osx-item-{}", self.id)
    }
}

const fn e(id: &'static str, os: Os, kind: Kind) -> Entry {
    Entry { id, os, kind }
}

/// Every §27.10 item.
pub const CATALOG: &[Entry] = &[
    e("powertoys-run", Os::Windows, Kind::Install),
    e("windows-terminal", Os::Windows, Kind::Install),
    e("wsl", Os::Windows, Kind::Builtin),
    e("start-search", Os::Windows, Kind::Builtin),
    e("jump-list", Os::Windows, Kind::Delegated("desktop")),
    e("widgets", Os::Windows, Kind::Install),
    e("shortcuts", Os::Mac, Kind::Install),
    e("spotlight", Os::Mac, Kind::DesktopFlag),
    e("universal-clipboard", Os::Mac, Kind::DesktopFlag),
    e("handoff", Os::Mac, Kind::DesktopFlag),
    e("menu-bar", Os::Mac, Kind::Delegated("desktop")),
    e("dock-menu", Os::Mac, Kind::Delegated("desktop")),
    e("gnome-shell", Os::Linux, Kind::Install),
    e("plasmoid", Os::Linux, Kind::Install),
    e("xdg-scheme", Os::Linux, Kind::Delegated("kor integrate")),
    e("portals", Os::Linux, Kind::DesktopFlag),
    e("systemd-user", Os::Linux, Kind::Install),
    e("flatpak", Os::Linux, Kind::Builtin),
];

/// Items of `os`.
pub fn catalog(os: Os) -> Vec<Entry> {
    CATALOG.iter().copied().filter(|e| e.os == os).collect()
}

/// The entry `id`.
pub fn entry(id: &str) -> Option<Entry> {
    CATALOG.iter().copied().find(|e| e.id == id)
}

/// Runs external programs (fake in tests).
pub trait Runner {
    /// Run `program args`; standard output on success.
    ///
    /// # Errors
    /// When it cannot start or exits with an error.
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<String>;
    /// `program` is on PATH.
    fn has(&self, program: &str) -> bool;
}

/// The real runner. Inside Flatpak (`host`) programs run on the host through
/// `flatpak-spawn --host` (the manifest grants `org.freedesktop.Flatpak`).
#[derive(Debug, Default)]
pub struct System {
    pub host: bool,
}

impl System {
    fn command(&self, program: &str, args: &[&str]) -> std::process::Command {
        let mut c = if self.host {
            let mut c = std::process::Command::new("flatpak-spawn");
            c.args(["--host", program]);
            c
        } else {
            std::process::Command::new(program)
        };
        c.args(args).stdin(std::process::Stdio::null()).no_window();
        c
    }
}

impl Runner for System {
    fn run(&mut self, program: &str, args: &[&str]) -> io::Result<String> {
        let out = self.command(program, args).output()?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(io::Error::other(format!(
                "{program}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
    fn has(&self, program: &str) -> bool {
        if self.host {
            return self
                .command("sh", &["-c", &format!("command -v {program}")])
                .output()
                .is_ok_and(|o| o.status.success());
        }
        on_path(program)
    }
}

/// `program` (with the platform's executable suffixes) is in a `PATH` directory.
pub fn on_path(program: &str) -> bool {
    let exts: &[&str] = if crate::os() == Os::Windows {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| exts.iter().any(|x| d.join(format!("{program}{x}")).is_file()))
    })
}

/// The desktop bundle's resource directory for a `kor` that came with the desktop app (next to
/// the executable on Windows, `Contents/Resources` on macOS, `lib/Kor` for Linux packages):
/// the first candidate holding `integrations/`.
pub fn resources_near(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    [
        dir.to_path_buf(),
        dir.join("..").join("Resources"),
        dir.join("..").join("lib").join("Kor"),
        dir.join("..").join("lib").join("kor"),
    ]
    .into_iter()
    .find(|d| d.join("integrations").is_dir())
}

/// Where integrations go and what they point at.
pub struct Env {
    pub os: Os,
    pub home: PathBuf,
    /// Kor's data directory (record file).
    pub data: PathBuf,
    /// `%LOCALAPPDATA%` / `~/Library/Application Support` / `~/.local/share`.
    pub local: PathBuf,
    /// The `kor` CLI the integrations call.
    pub kor: PathBuf,
    /// The desktop bundle's resource directory (`PowerToys` plugin build), when known.
    pub resources: Option<PathBuf>,
    /// Icon for profiles (an `.ico`/`.png`, or the desktop executable).
    pub icon: Option<PathBuf>,
    /// Running inside Flatpak.
    pub flatpak: bool,
    pub runner: Box<dyn Runner>,
    /// `PowerShell` runner of the Windows 11 Widgets package (shared with the sparse menu package).
    pub sparse: Box<dyn crate::shell_integration::sparse::Runner>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("os", &self.os)
            .field("home", &self.home)
            .field("data", &self.data)
            .field("local", &self.local)
            .field("kor", &self.kor)
            .finish_non_exhaustive()
    }
}

impl Env {
    /// The current user's environment.
    ///
    /// # Errors
    /// When the home or data directory is unknown.
    pub fn current(kor: PathBuf, resources: Option<PathBuf>, icon: Option<PathBuf>) -> io::Result<Self> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, "home directory not found");
        let flatpak = std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists();
        Ok(Self {
            os: crate::os(),
            home: dirs::home_dir().ok_or_else(missing)?,
            data: crate::data_dir().ok_or_else(missing)?,
            local: dirs::data_local_dir().ok_or_else(missing)?,
            kor,
            resources,
            icon,
            flatpak,
            runner: Box::new(System { host: flatpak }),
            sparse: Box::new(crate::shell_integration::sparse::System),
        })
    }

    fn record_path(&self) -> PathBuf {
        self.data.join("os-integration.json")
    }

    fn powertoys_dir(&self) -> PathBuf {
        self.local
            .join("Microsoft")
            .join("PowerToys")
            .join("PowerToys Run")
            .join("Plugins")
            .join("Kor")
    }

    fn powertoys_build(&self) -> Option<PathBuf> {
        self.resources
            .as_ref()
            .map(|r| r.join("integrations").join("powertoys"))
            .filter(|d| d.join(POWERTOYS_DLL).is_file())
    }

    fn terminal_fragment(&self) -> PathBuf {
        self.local
            .join("Microsoft")
            .join("Windows Terminal")
            .join("Fragments")
            .join("Kor")
            .join("kor.json")
    }

    fn gnome_dir(&self) -> PathBuf {
        self.local.join("gnome-shell").join("extensions").join(GNOME_UUID)
    }

    fn plasmoid_dir(&self) -> PathBuf {
        self.local.join("plasma").join("plasmoids").join(PLASMOID_ID)
    }

    fn shortcuts_dir(&self) -> PathBuf {
        self.data.join("os-integration").join("shortcuts")
    }
}

const POWERTOYS_DLL: &str = "Community.PowerToys.Run.Plugin.Kor.dll";
pub const GNOME_UUID: &str = "kor@kor.dev";
pub const PLASMOID_ID: &str = "dev.kor.status";
/// Windows Terminal profile id (stable, so updates replace the same profile).
const WT_GUID: &str = "{6b0f6d1c-4c4b-5b8e-9a1f-6b6f72000001}";

/// What Kor wrote for one item.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Done {
    #[serde(default)]
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub dirs: Vec<PathBuf>,
    /// Item settings (Handoff URL…).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, String>,
}

/// `<data>/os-integration.json`: item id → what was done.
pub type Record = BTreeMap<String, Done>;

/// Read the record (empty when missing or unreadable).
pub fn load_record(env: &Env) -> Record {
    std::fs::read_to_string(env.record_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_record(env: &Env, r: &Record) -> io::Result<()> {
    let p = env.record_path();
    if r.is_empty() {
        return match std::fs::remove_file(&p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let text = serde_json::to_string_pretty(r).map_err(io::Error::other)?;
    std::fs::write(p, text)
}

/// Whether a desktop-flag item (Spotlight, Handoff, Universal Clipboard, portals) is on, and its
/// settings.
pub fn flag(env: &Env, id: &str) -> Option<BTreeMap<String, String>> {
    load_record(env).remove(id).map(|d| d.data)
}

/// Merge `data` into an installed item's settings (portal restore tokens…).
///
/// # Errors
/// When the item is not installed or the record cannot be written.
pub fn set_data(env: &Env, id: &str, data: BTreeMap<String, String>) -> io::Result<()> {
    let mut record = load_record(env);
    let done = record
        .get_mut(id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{id} is not installed")))?;
    done.data.extend(data);
    save_record(env, &record)
}

/// One item's state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    pub id: &'static str,
    pub os: &'static str,
    pub kind: &'static str,
    /// For [`Kind::Delegated`]: the surface that installs it.
    pub via: Option<&'static str>,
    pub installed: bool,
    /// Can be installed here (the target application exists).
    pub available: bool,
    /// Reason code when not available or for extra info (`osx-reason-<code>`).
    pub reason: Option<&'static str>,
}

fn os_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "windows",
        Os::Mac => "macos",
        Os::Linux => "linux",
        Os::Other => "other",
    }
}

/// Is the item's target present: `Ok(())` or a reason code.
fn availability(env: &Env, e: Entry) -> Result<(), &'static str> {
    if e.os != env.os {
        return Err("other-os");
    }
    match e.id {
        "powertoys-run" => {
            if !env.local.join("Microsoft").join("PowerToys").is_dir() {
                Err("no-powertoys")
            } else if env.powertoys_build().is_none() {
                Err("no-bundle")
            } else {
                Ok(())
            }
        }
        "windows-terminal" => {
            let wt = env.local.join("Microsoft").join("WindowsApps").join("wt.exe");
            (wt.exists() || env.runner.has("wt")).then_some(()).ok_or("no-terminal")
        }
        "widgets" => env
            .resources
            .as_ref()
            .is_some_and(|r| widgets_kind().msix(r).is_file())
            .then_some(())
            .ok_or("no-bundle"),
        "shortcuts" => env.runner.has("shortcuts").then_some(()).ok_or("no-shortcuts"),
        "gnome-shell" => env.runner.has("gnome-shell").then_some(()).ok_or("no-gnome"),
        "plasmoid" => env.runner.has("plasmashell").then_some(()).ok_or("no-plasma"),
        "systemd-user" => env.runner.has("systemctl").then_some(()).ok_or("no-systemd"),
        _ => Ok(()),
    }
}

/// State of one item.
pub fn status(env: &Env, e: Entry) -> Status {
    let record = load_record(env);
    let avail = availability(env, e);
    let (installed, reason) = match e.kind {
        Kind::Builtin => match e.id {
            "flatpak" => (env.flatpak, (!env.flatpak).then_some("not-flatpak")),
            "start-search" => (true, Some("start-search")),
            _ => (true, None),
        },
        // `kor://` for xdg-open is the same entry as the context menus (`shell_integration`).
        Kind::Delegated(_) if e.id == "xdg-scheme" => {
            let on = env.os == Os::Linux && crate::os() == Os::Linux && crate::shell_integration::scheme_registered();
            (on, Some("delegated"))
        }
        Kind::Delegated(_) => (record.contains_key(e.id), Some("delegated")),
        Kind::Install | Kind::DesktopFlag => {
            let ok = record.get(e.id).is_some_and(|d| {
                d.files.iter().all(|f| f.exists()) && d.dirs.iter().all(|d| d.is_dir())
            });
            (ok, avail.err())
        }
    };
    Status {
        id: e.id,
        os: os_name(e.os),
        kind: match e.kind {
            Kind::Install => "install",
            Kind::DesktopFlag => "desktop",
            Kind::Builtin => "builtin",
            Kind::Delegated(_) => "delegated",
        },
        via: match e.kind {
            Kind::Delegated(v) => Some(v),
            _ => None,
        },
        installed,
        available: avail.is_ok(),
        reason,
    }
}

/// States of every item of the current OS.
pub fn status_all(env: &Env) -> Vec<Status> {
    catalog(env.os).into_iter().map(|e| status(env, e)).collect()
}

/// Options some items take.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Handoff: web address that continues the session on another device (remote/PWA).
    pub handoff_url: Option<String>,
    /// Shortcuts.app texts (localized by the caller).
    pub texts: shortcuts::Texts,
}

/// Why an install/uninstall did not happen.
#[derive(Debug)]
pub enum Error {
    Unknown(String),
    Unavailable(&'static str),
    /// Builtin or installed elsewhere: nothing to do here.
    NotManaged(&'static str),
    /// A file Kor did not write is in the way.
    Foreign(PathBuf),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "unknown integration `{id}`"),
            Self::Unavailable(r) => write!(f, "not available here ({r})"),
            Self::NotManaged(r) => write!(f, "not installed by this command ({r})"),
            Self::Foreign(p) => write!(f, "{} belongs to something else", p.display()),
            Self::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// JSON string literal for JS/QML sources.
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"kor\"".into())
}

/// Files an item writes: `(path, contents)`.
fn files(env: &Env, id: &str, opts: &Options) -> Result<Vec<(PathBuf, Vec<u8>)>, Error> {
    let kor = env.kor.to_string_lossy();
    Ok(match id {
        "windows-terminal" => vec![(env.terminal_fragment(), terminal_fragment(env).into_bytes())],
        "gnome-shell" => {
            let d = env.gnome_dir();
            vec![
                (d.join("metadata.json"), include_bytes!("os_integration/gnome/metadata.json").to_vec()),
                (
                    d.join("extension.js"),
                    include_str!("os_integration/gnome/extension.js")
                        .replace("@KOR_JSON@", &js_string(&kor))
                        .into_bytes(),
                ),
            ]
        }
        "plasmoid" => {
            let d = env.plasmoid_dir();
            vec![
                (d.join("metadata.json"), include_bytes!("os_integration/plasmoid/metadata.json").to_vec()),
                (
                    d.join("contents").join("ui").join("main.qml"),
                    include_str!("os_integration/plasmoid/main.qml")
                        .replace("@KOR_JSON@", &js_string(&kor))
                        .into_bytes(),
                ),
            ]
        }
        "powertoys-run" => {
            let src = env.powertoys_build().ok_or(Error::Unavailable("no-bundle"))?;
            let mut out = Vec::new();
            copy_list(&src, &env.powertoys_dir(), &mut out)?;
            // The plugin runs this `kor` (it has no PATH guarantee inside PowerToys).
            out.push((env.powertoys_dir().join("kor-path.txt"), kor.as_bytes().to_vec()));
            out
        }
        "shortcuts" => shortcuts::ALL
            .iter()
            .map(|s| {
                (
                    env.shortcuts_dir().join(format!("{}.unsigned.shortcut", s.file)),
                    shortcuts::workflow_with(*s, &kor, &opts.texts).into_bytes(),
                )
            })
            .collect(),
        "handoff" => {
            if let Some(u) = &opts.handoff_url
                && !(u.starts_with("https://") && u.len() <= 2048)
            {
                return Err(Error::Unavailable("bad-url"));
            }
            Vec::new()
        }
        _ => Vec::new(),
    })
}

fn copy_list(src: &Path, dst: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) -> io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            copy_list(&p, &dst.join(e.file_name()), out)?;
        } else {
            out.push((dst.join(e.file_name()), std::fs::read(&p)?));
        }
    }
    Ok(())
}

/// The Windows Terminal fragment (`profiles` with one "Kor" profile).
pub fn terminal_fragment(env: &Env) -> String {
    let icon = env
        .icon
        .as_ref()
        .unwrap_or(&env.kor)
        .to_string_lossy()
        .into_owned();
    let v = serde_json::json!({
        "profiles": [{
            "guid": WT_GUID,
            "name": "Kor",
            "commandline": format!("\"{}\"", env.kor.display()),
            "icon": icon,
            "startingDirectory": "%USERPROFILE%",
            "hidden": false
        }]
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Install `id`. Idempotent.
///
/// # Errors
/// Unknown/unavailable item, a foreign file in the way, or a write / command failure.
pub fn install(env: &mut Env, id: &str, opts: &Options) -> Result<Status, Error> {
    let e = entry(id).ok_or_else(|| Error::Unknown(id.to_owned()))?;
    if id == "xdg-scheme" {
        availability(env, e).map_err(Error::Unavailable)?;
        if crate::os() != Os::Linux {
            return Err(Error::Unavailable("other-os"));
        }
        let apps = crate::shell_integration::detect_apps(Some(env.kor.clone()));
        crate::shell_integration::register_scheme(&apps)?;
        return Ok(status(env, e));
    }
    match e.kind {
        Kind::Builtin => return Err(Error::NotManaged("builtin")),
        Kind::Delegated(_) => return Err(Error::NotManaged("delegated")),
        Kind::Install | Kind::DesktopFlag => {}
    }
    availability(env, e).map_err(Error::Unavailable)?;
    let mut record = load_record(env);
    let prev = record.remove(id).unwrap_or_default();
    let mut done = Done::default();
    for (path, body) in files(env, id, opts)? {
        if path.exists() && !prev.files.contains(&path) {
            return Err(Error::Foreign(path));
        }
        let mut missing = Vec::new();
        let mut d = path.parent();
        while let Some(dir) = d.filter(|d| !d.exists()) {
            missing.push(dir.to_path_buf());
            d = dir.parent();
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, body)?;
        done.files.push(path);
        done.dirs.extend(missing);
    }
    for d in prev.dirs {
        if !done.dirs.contains(&d) && d.is_dir() {
            done.dirs.push(d);
        }
    }
    // Keep files of an older install that this one no longer writes, so uninstall still removes them.
    for f in prev.files {
        if !done.files.contains(&f) && f.exists() {
            done.files.push(f);
        }
    }
    after_install(env, id, opts, &mut done)?;
    record.insert(id.to_owned(), done);
    save_record(env, &record)?;
    Ok(status(env, e))
}

fn widgets_kind() -> crate::shell_integration::sparse::Kind {
    crate::shell_integration::sparse::Kind::Widgets
}

fn after_install(env: &mut Env, id: &str, opts: &Options, done: &mut Done) -> Result<(), Error> {
    match id {
        "widgets" => {
            // The package registers with the install folder as external location (needs the
            // build's signing certificate to be trusted; see ADR 0052).
            let dir = env.resources.clone().ok_or(Error::Unavailable("no-bundle"))?;
            crate::shell_integration::sparse::register(&mut *env.sparse, widgets_kind(), &dir)?;
        }
        "shortcuts" => {
            for s in shortcuts::ALL {
                let unsigned = env.shortcuts_dir().join(format!("{}.unsigned.shortcut", s.file));
                let signed = env.shortcuts_dir().join(format!("{}.shortcut", s.file));
                let (u, sg) = (unsigned.to_string_lossy(), signed.to_string_lossy());
                env.runner
                    .run("shortcuts", &["sign", "--mode", "anyone", "--input", &u, "--output", &sg])?;
                if !done.files.contains(&signed) {
                    done.files.push(signed.clone());
                }
                // Opens the "Add Shortcut" sheet in Shortcuts.app; the user confirms each.
                env.runner.run("open", &[&sg])?;
            }
        }
        "gnome-shell" => {
            // A new extension is seen by GNOME Shell after the next login on Wayland; enabling
            // now records the choice either way.
            let _ = env.runner.run("gnome-extensions", &["enable", GNOME_UUID]);
        }
        "systemd-user" => {
            let unit = crate::service::install(&env.kor)?;
            done.data.insert("unit".into(), unit);
        }
        "handoff" => {
            if let Some(u) = &opts.handoff_url {
                done.data.insert("url".into(), u.clone());
            }
        }
        "spotlight" | "universal-clipboard" | "portals" => {
            done.data.insert("enabled".into(), "true".into());
        }
        _ => {}
    }
    Ok(())
}

/// Remove what Kor installed for `id`. Idempotent; returns the reason code of anything left for
/// the user (Shortcuts.app has no removal API).
///
/// # Errors
/// Unknown item, or a delete / command failure.
pub fn uninstall(env: &mut Env, id: &str) -> Result<(Status, Option<&'static str>), Error> {
    let e = entry(id).ok_or_else(|| Error::Unknown(id.to_owned()))?;
    if id == "xdg-scheme" {
        availability(env, e).map_err(Error::Unavailable)?;
        if crate::os() != Os::Linux {
            return Err(Error::Unavailable("other-os"));
        }
        crate::shell_integration::unregister_scheme()?;
        return Ok((status(env, e), None));
    }
    if let Kind::Builtin | Kind::Delegated(_) = e.kind {
        return Err(Error::NotManaged(if e.kind == Kind::Builtin { "builtin" } else { "delegated" }));
    }
    let mut record = load_record(env);
    let Some(done) = record.remove(id) else {
        return Ok((status(env, e), None));
    };
    let mut left = None;
    match id {
        "gnome-shell" => {
            let _ = env.runner.run("gnome-extensions", &["disable", GNOME_UUID]);
        }
        "systemd-user" => crate::service::uninstall()?,
        "widgets" => {
            crate::shell_integration::sparse::unregister(&mut *env.sparse, widgets_kind())?;
        }
        "shortcuts" => left = Some("shortcuts-manual"),
        _ => {}
    }
    for f in &done.files {
        match std::fs::remove_file(f) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    // Deepest first; only empty directories (never something the user added).
    let mut dirs = done.dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let _ = std::fs::remove_dir(d);
    }
    save_record(env, &record)?;
    Ok((status(env, e), left))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct Fake {
        have: Vec<&'static str>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[&str]) -> io::Result<String> {
            self.calls.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            if program == "shortcuts" {
                // `sign --input U --output S`: produce the signed file like the real tool.
                let out = args.iter().position(|a| *a == "--output").map(|i| args[i + 1]);
                if let Some(o) = out {
                    std::fs::write(o, b"signed")?;
                }
            }
            Ok(String::new())
        }
        fn has(&self, program: &str) -> bool {
            self.have.contains(&program)
        }
    }

    #[derive(Default, Clone)]
    struct FakePs {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl crate::shell_integration::sparse::Runner for FakePs {
        fn run(&mut self, program: &str, args: &[String]) -> io::Result<crate::shell_integration::sparse::Output> {
            let line = format!("{program} {}", args.join(" "));
            let stdout = if line.contains("Version.Build") {
                "22631".to_owned()
            } else if line.contains(".Version") {
                "6000.519.329.0".to_owned()
            } else if line.contains("PackageFullName") {
                "Kor.Widgets_1.0.0.0_x64__abc".to_owned()
            } else {
                String::new()
            };
            self.calls.lock().unwrap().push(line);
            Ok(crate::shell_integration::sparse::Output { code: 0, stdout, stderr: String::new() })
        }
    }

    fn env(root: &Path, os: Os, fake: &Fake) -> Env {
        Env {
            os,
            home: root.join("home"),
            data: root.join("data"),
            local: root.join("local"),
            kor: PathBuf::from("/opt/kor/bin/kor"),
            resources: Some(root.join("res")),
            icon: None,
            flatpak: false,
            runner: Box::new(fake.clone()),
            sparse: Box::new(FakePs::default()),
        }
    }

    #[test]
    fn catalog_covers_each_os() {
        let ids = |os| catalog(os).into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(
            ids(Os::Windows),
            ["powertoys-run", "windows-terminal", "wsl", "start-search", "jump-list", "widgets"]
        );
        assert_eq!(entry("widgets").unwrap().kind, Kind::Install);
        assert_eq!(
            ids(Os::Mac),
            ["shortcuts", "spotlight", "universal-clipboard", "handoff", "menu-bar", "dock-menu"]
        );
        assert_eq!(
            ids(Os::Linux),
            ["gnome-shell", "plasmoid", "xdg-scheme", "portals", "systemd-user", "flatpak"]
        );
        assert!(catalog(Os::Other).is_empty());
        let unique: std::collections::BTreeSet<_> = CATALOG.iter().map(|e| e.id).collect();
        assert_eq!(unique.len(), CATALOG.len());
    }

    #[test]
    fn gnome_and_plasmoid_install_exactly_and_uninstall_cleanly() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake { have: vec!["gnome-shell", "plasmashell"], ..Fake::default() };
        let mut env = env(d.path(), Os::Linux, &fake);
        // The user's own extension next to ours must survive.
        let other = env.local.join("gnome-shell").join("extensions").join("x@y");
        std::fs::create_dir_all(&other).unwrap();

        let s = install(&mut env, "gnome-shell", &Options::default()).unwrap();
        assert!(s.installed && s.available);
        install(&mut env, "gnome-shell", &Options::default()).unwrap();
        let js = std::fs::read_to_string(env.gnome_dir().join("extension.js")).unwrap();
        assert!(js.contains(r#"const KOR = "/opt/kor/bin/kor";"#), "{js}");
        assert!(!js.contains("@KOR_JSON@"));
        let meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(env.gnome_dir().join("metadata.json")).unwrap()).unwrap();
        assert_eq!(meta["uuid"], GNOME_UUID);
        assert!(meta["shell-version"].as_array().unwrap().iter().any(|v| v == "47"));
        assert!(fake.calls.lock().unwrap().iter().any(|c| c == "gnome-extensions enable kor@kor.dev"));

        install(&mut env, "plasmoid", &Options::default()).unwrap();
        let qml = std::fs::read_to_string(env.plasmoid_dir().join("contents/ui/main.qml")).unwrap();
        assert!(qml.contains(r#"readonly property string kor: "/opt/kor/bin/kor""#));
        let pm: serde_json::Value =
            serde_json::from_slice(&std::fs::read(env.plasmoid_dir().join("metadata.json")).unwrap()).unwrap();
        assert_eq!(pm["KPlugin"]["Id"], PLASMOID_ID);
        assert_eq!(pm["KPackageStructure"], "Plasma/Applet");

        uninstall(&mut env, "gnome-shell").unwrap();
        uninstall(&mut env, "plasmoid").unwrap();
        uninstall(&mut env, "plasmoid").unwrap();
        assert!(!env.gnome_dir().exists() && !env.plasmoid_dir().exists());
        assert!(other.is_dir(), "foreign extension kept");
        assert!(!env.record_path().exists());
        assert!(!status(&env, entry("plasmoid").unwrap()).installed);
    }

    #[test]
    fn foreign_files_are_never_overwritten_and_unavailable_items_refuse() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake { have: vec!["wt"], ..Fake::default() };
        let mut env = env(d.path(), Os::Windows, &fake);
        let frag = env.terminal_fragment();
        std::fs::create_dir_all(frag.parent().unwrap()).unwrap();
        std::fs::write(&frag, "{}").unwrap();
        assert!(matches!(install(&mut env, "windows-terminal", &Options::default()), Err(Error::Foreign(_))));
        std::fs::remove_file(&frag).unwrap();
        install(&mut env, "windows-terminal", &Options::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&frag).unwrap()).unwrap();
        let p = &v["profiles"][0];
        assert_eq!(p["name"], "Kor");
        assert_eq!(p["guid"], WT_GUID);
        assert_eq!(p["commandline"], "\"/opt/kor/bin/kor\"");
        uninstall(&mut env, "windows-terminal").unwrap();
        assert!(!frag.exists());
        // The folder existed before the install (the test made it): it stays.
        assert!(frag.parent().unwrap().is_dir());

        assert!(matches!(install(&mut env, "powertoys-run", &Options::default()), Err(Error::Unavailable("no-powertoys"))));
        assert!(matches!(install(&mut env, "gnome-shell", &Options::default()), Err(Error::Unavailable("other-os"))));
        assert!(matches!(install(&mut env, "wsl", &Options::default()), Err(Error::NotManaged("builtin"))));
        assert!(matches!(install(&mut env, "jump-list", &Options::default()), Err(Error::NotManaged("delegated"))));
        assert!(matches!(install(&mut env, "nope", &Options::default()), Err(Error::Unknown(_))));
    }

    #[test]
    fn powertoys_plugin_is_copied_from_the_bundle() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let mut env = env(d.path(), Os::Windows, &fake);
        std::fs::create_dir_all(env.local.join("Microsoft").join("PowerToys")).unwrap();
        assert_eq!(status(&env, entry("powertoys-run").unwrap()).reason, Some("no-bundle"));
        let build = d.path().join("res").join("integrations").join("powertoys");
        std::fs::create_dir_all(build.join("Images")).unwrap();
        std::fs::write(build.join(POWERTOYS_DLL), b"dll").unwrap();
        std::fs::write(build.join("plugin.json"), b"{}").unwrap();
        std::fs::write(build.join("Images").join("kor.png"), b"png").unwrap();
        install(&mut env, "powertoys-run", &Options::default()).unwrap();
        assert_eq!(std::fs::read(env.powertoys_dir().join(POWERTOYS_DLL)).unwrap(), b"dll");
        assert!(env.powertoys_dir().join("Images").join("kor.png").is_file());
        assert_eq!(std::fs::read_to_string(env.powertoys_dir().join("kor-path.txt")).unwrap(), "/opt/kor/bin/kor");
        uninstall(&mut env, "powertoys-run").unwrap();
        assert!(!env.powertoys_dir().exists());
        assert!(!env.local.join("Microsoft").join("PowerToys").join("PowerToys Run").exists());
    }

    #[test]
    fn shortcuts_are_generated_signed_and_imported() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake { have: vec!["shortcuts"], ..Fake::default() };
        let mut env = env(d.path(), Os::Mac, &fake);
        install(&mut env, "shortcuts", &Options::default()).unwrap();
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().filter(|c| c.starts_with("shortcuts sign --mode anyone")).count(), 4);
        assert_eq!(calls.iter().filter(|c| c.starts_with("open ")).count(), 4);
        for s in shortcuts::ALL {
            assert!(env.shortcuts_dir().join(format!("{}.shortcut", s.file)).is_file());
        }
        let (st, left) = uninstall(&mut env, "shortcuts").unwrap();
        assert!(!st.installed);
        assert_eq!(left, Some("shortcuts-manual"));
        assert!(!env.shortcuts_dir().exists());
    }

    #[test]
    fn desktop_flags_and_settings_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let mut env = env(d.path(), Os::Mac, &fake);
        assert!(flag(&env, "handoff").is_none());
        let bad = Options { handoff_url: Some("http://x".into()), ..Options::default() };
        assert!(matches!(install(&mut env, "handoff", &bad), Err(Error::Unavailable("bad-url"))));
        let opts = Options { handoff_url: Some("https://kor.example/s".into()), ..Options::default() };
        assert!(install(&mut env, "handoff", &opts).unwrap().installed);
        assert_eq!(flag(&env, "handoff").unwrap()["url"], "https://kor.example/s");
        install(&mut env, "spotlight", &Options::default()).unwrap();
        assert!(flag(&env, "spotlight").is_some());
        uninstall(&mut env, "spotlight").unwrap();
        assert!(flag(&env, "spotlight").is_none() && flag(&env, "handoff").is_some());
        let all = status_all(&env);
        assert_eq!(all.len(), 6);
        assert!(all.iter().any(|s| s.id == "dock-menu" && s.kind == "delegated" && s.via == Some("desktop")));
    }

    #[test]
    fn builtins_report_their_state() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let mut env = env(d.path(), Os::Linux, &fake);
        let fp = entry("flatpak").unwrap();
        assert_eq!(status(&env, fp).reason, Some("not-flatpak"));
        env.flatpak = true;
        assert!(status(&env, fp).installed);
        assert_eq!(status(&env, entry("systemd-user").unwrap()).reason, Some("no-systemd"));
    }

    #[test]
    fn widgets_register_the_bundled_package() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::default();
        let mut env = env(d.path(), Os::Windows, &fake);
        let ps = FakePs::default();
        env.sparse = Box::new(ps.clone());
        assert_eq!(status(&env, entry("widgets").unwrap()).reason, Some("no-bundle"));
        let msix = widgets_kind().msix(&d.path().join("res"));
        std::fs::create_dir_all(msix.parent().unwrap()).unwrap();
        std::fs::write(&msix, b"msix").unwrap();
        assert!(install(&mut env, "widgets", &Options::default()).unwrap().installed);
        assert!(ps.calls.lock().unwrap().iter().any(|c| c.contains("Add-AppxPackage")));
        uninstall(&mut env, "widgets").unwrap();
        assert!(ps.calls.lock().unwrap().iter().any(|c| c.contains("Remove-AppxPackage")));
        assert!(!status(&env, entry("widgets").unwrap()).installed);
    }
}
