//! Native messaging host registration for the Kor browser extension (§18.1, §18.4, ADR 0058).
//!
//! Chrome-family browsers and Firefox start a native host from a JSON manifest that names an
//! executable and the extensions allowed to talk to it. Kor's host is `kor chrome-native-host`;
//! browsers cannot pass extra arguments, so a one-line wrapper (`.cmd` on Windows, `sh` elsewhere)
//! next to the manifest runs it.
//!
//! Where browsers look:
//! - Windows: `HKCU\Software\<vendor>\NativeMessagingHosts\<name>` (default value = manifest path);
//!   Kor keeps the manifests under its data directory.
//! - macOS: `~/Library/Application Support/<vendor>/NativeMessagingHosts/<name>.json`.
//! - Linux: `~/.config/<vendor>/NativeMessagingHosts/<name>.json`, Firefox
//!   `~/.mozilla/native-messaging-hosts/<name>.json`.
//!
//! Paths and the registry are injected ([`Env`]) so tests never touch the real system.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Os;

#[cfg(windows)]
mod win;

/// Host name in manifests and `connectNative`.
pub const HOST_NAME: &str = "dev.kor.chrome_host";
/// Chrome extension id of the unpacked build: derived from the public `key` in its manifest
/// (`frontend/src/extension/manifest.ts`), so it is the same on every machine.
pub const CHROME_EXTENSION_ID: &str = "iokjlmpngkhnhndnklpildpdfpfdjddh";
/// Firefox add-on id (`browser_specific_settings.gecko.id`).
pub const FIREFOX_EXTENSION_ID: &str = "kor@kor.dev";

/// Browsers Kor registers its host for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Chromium,
    Firefox,
}

pub const ALL: &[Browser] = &[
    Browser::Chrome,
    Browser::Edge,
    Browser::Brave,
    Browser::Chromium,
    Browser::Firefox,
];

impl Browser {
    pub fn id(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Brave => "brave",
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ALL.iter().copied().find(|b| b.id() == s.trim().to_ascii_lowercase())
    }

    fn registry_key(self) -> String {
        let vendor = match self {
            Self::Chrome => r"Google\Chrome",
            Self::Edge => r"Microsoft\Edge",
            Self::Brave => r"BraveSoftware\Brave-Browser",
            Self::Chromium => "Chromium",
            Self::Firefox => "Mozilla",
        };
        format!(r"Software\{vendor}\NativeMessagingHosts\{HOST_NAME}")
    }

    /// Directory the browser reads manifests from on macOS and Linux.
    fn unix_dir(self, env: &Env) -> PathBuf {
        match (env.os, self) {
            (Os::Mac, b) => {
                let vendor = match b {
                    Self::Chrome => "Google/Chrome",
                    Self::Edge => "Microsoft Edge",
                    Self::Brave => "BraveSoftware/Brave-Browser",
                    Self::Chromium => "Chromium",
                    Self::Firefox => "Mozilla",
                };
                env.home
                    .join("Library/Application Support")
                    .join(vendor)
                    .join("NativeMessagingHosts")
            }
            (_, Self::Firefox) => env.home.join(".mozilla/native-messaging-hosts"),
            (_, b) => {
                let vendor = match b {
                    Self::Edge => "microsoft-edge",
                    Self::Brave => "BraveSoftware/Brave-Browser",
                    Self::Chromium => "chromium",
                    _ => "google-chrome",
                };
                env.config.join(vendor).join("NativeMessagingHosts")
            }
        }
    }
}

/// `HKCU` default values, injected for tests.
pub trait Registry {
    fn set_default(&self, key: &str, value: &str) -> io::Result<()>;
    fn get_default(&self, key: &str) -> io::Result<Option<String>>;
    /// Returns whether the key existed.
    fn delete(&self, key: &str) -> io::Result<bool>;
}

/// The real per-user registry (Windows); elsewhere every call fails.
#[derive(Debug, Default, Clone, Copy)]
pub struct UserRegistry;

impl Registry for UserRegistry {
    #[cfg(windows)]
    fn set_default(&self, key: &str, value: &str) -> io::Result<()> {
        win::set_default(key, value)
    }
    #[cfg(windows)]
    fn get_default(&self, key: &str) -> io::Result<Option<String>> {
        win::get_default(key)
    }
    #[cfg(windows)]
    fn delete(&self, key: &str) -> io::Result<bool> {
        win::delete(key)
    }
    #[cfg(not(windows))]
    fn set_default(&self, _key: &str, _value: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    #[cfg(not(windows))]
    fn get_default(&self, _key: &str) -> io::Result<Option<String>> {
        Ok(None)
    }
    #[cfg(not(windows))]
    fn delete(&self, _key: &str) -> io::Result<bool> {
        Ok(false)
    }
}

/// Where things go. [`Env::system`] for real use.
pub struct Env<'a> {
    pub os: Os,
    pub home: PathBuf,
    /// XDG config dir (`~/.config`) on Linux.
    pub config: PathBuf,
    /// Kor's data dir; Windows manifests and every wrapper live under `<data>/native-messaging`.
    pub data: PathBuf,
    pub registry: &'a dyn Registry,
}

impl std::fmt::Debug for Env<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env").field("os", &self.os).field("data", &self.data).finish_non_exhaustive()
    }
}

static REGISTRY: UserRegistry = UserRegistry;

impl Env<'static> {
    /// This machine. `None` when the home or data directory is unknown.
    pub fn system() -> Option<Self> {
        Some(Self {
            os: crate::os(),
            home: dirs::home_dir()?,
            config: dirs::config_dir()?,
            data: crate::data_dir()?,
            registry: &REGISTRY,
        })
    }
}

impl Env<'_> {
    fn host_dir(&self) -> PathBuf {
        self.data.join("native-messaging")
    }

    /// The manifest file for `browser`.
    pub fn manifest_path(&self, browser: Browser) -> PathBuf {
        let file = format!("{HOST_NAME}.json");
        match self.os {
            Os::Windows => self.host_dir().join(browser.id()).join(file),
            _ => browser.unix_dir(self).join(file),
        }
    }

    /// The wrapper script browsers start.
    pub fn wrapper_path(&self) -> PathBuf {
        self.host_dir().join(match self.os {
            Os::Windows => "kor-chrome-host.cmd",
            _ => "kor-chrome-host.sh",
        })
    }
}

/// The manifest JSON. Chrome-family manifests list extension origins, Firefox ones add-on ids.
pub fn manifest(browser: Browser, wrapper: &Path, chrome_ids: &[String]) -> Value {
    let mut m = json!({
        "name": HOST_NAME,
        "description": "Kor native messaging host",
        "path": wrapper.to_string_lossy(),
        "type": "stdio",
    });
    if browser == Browser::Firefox {
        m["allowed_extensions"] = json!([FIREFOX_EXTENSION_ID]);
    } else {
        let origins: Vec<String> = std::iter::once(CHROME_EXTENSION_ID.to_owned())
            .chain(chrome_ids.iter().cloned())
            .filter(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)))
            .map(|id| format!("chrome-extension://{id}/"))
            .collect();
        m["allowed_origins"] = json!(origins);
    }
    m
}

fn wrapper_text(os: Os, kor: &Path) -> String {
    // The browser passes the caller (`chrome-extension://<id>/`, or Firefox's manifest path and
    // add-on id) as arguments; the host reads them from `KOR_NATIVE_ORIGIN`, not its command line.
    let exe = kor.to_string_lossy();
    match os {
        Os::Windows => format!(
            "@echo off\r\nset \"KOR_NATIVE_ORIGIN=%~1 %~2\"\r\n\"{exe}\" chrome-native-host\r\n"
        ),
        _ => format!(
            "#!/bin/sh\nKOR_NATIVE_ORIGIN=\"$1 $2\" exec '{}' chrome-native-host\n",
            exe.replace('\'', r"'\''")
        ),
    }
}

fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Result of one registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub browser: Browser,
    pub manifest: PathBuf,
    /// Windows registry key (under `HKCU`).
    pub registry_key: Option<String>,
}

/// Register the host for `browser`, running `kor` (the CLI binary). `chrome_ids` adds ids of
/// other builds (the Web Store one) to the allowed origins.
///
/// # Errors
/// File or registry write failures.
pub fn install(env: &Env, browser: Browser, kor: &Path, chrome_ids: &[String]) -> io::Result<Installed> {
    let wrapper = env.wrapper_path();
    write_atomic(&wrapper, &wrapper_text(env.os, kor))?;
    make_executable(&wrapper)?;
    let manifest_path = env.manifest_path(browser);
    let text = serde_json::to_string_pretty(&manifest(browser, &wrapper, chrome_ids))
        .map_err(io::Error::other)?;
    write_atomic(&manifest_path, &text)?;
    let registry_key = if env.os == Os::Windows {
        let key = browser.registry_key();
        env.registry.set_default(&key, &manifest_path.to_string_lossy())?;
        Some(key)
    } else {
        None
    };
    Ok(Installed { browser, manifest: manifest_path, registry_key })
}

/// Remove what [`install`] wrote for `browser`. Returns whether anything was there. A manifest
/// that does not point at Kor's wrapper is left alone.
///
/// # Errors
/// File or registry failures.
pub fn uninstall(env: &Env, browser: Browser) -> io::Result<bool> {
    let mut removed = false;
    let manifest_path = env.manifest_path(browser);
    if is_ours(env, &manifest_path) {
        std::fs::remove_file(&manifest_path)?;
        removed = true;
    }
    if env.os == Os::Windows {
        let key = browser.registry_key();
        if env.registry.get_default(&key)?.is_some_and(|v| Path::new(&v) == manifest_path) {
            removed |= env.registry.delete(&key)?;
        }
    }
    if ALL.iter().all(|b| !is_ours(env, &env.manifest_path(*b))) {
        let _ = std::fs::remove_file(env.wrapper_path());
    }
    Ok(removed)
}

fn is_ours(env: &Env, manifest_path: &Path) -> bool {
    std::fs::read(manifest_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|m| m["path"].as_str().is_some_and(|p| Path::new(p) == env.wrapper_path()))
}

/// Whether `browser` will find Kor's host.
pub fn installed(env: &Env, browser: Browser) -> bool {
    let path = env.manifest_path(browser);
    if !is_ours(env, &path) {
        return false;
    }
    env.os != Os::Windows
        || env
            .registry
            .get_default(&browser.registry_key())
            .ok()
            .flatten()
            .is_some_and(|v| Path::new(&v) == path)
}

/// Chrome-family executable for `browser` on this machine (to open `chrome://extensions`).
pub fn browser_exe(browser: Browser) -> Option<PathBuf> {
    let os = crate::os();
    let candidates: Vec<PathBuf> = match os {
        Os::Windows => {
            let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
                .iter()
                .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
                .collect();
            let rel = match browser {
                Browser::Chrome => r"Google\Chrome\Application\chrome.exe",
                Browser::Edge => r"Microsoft\Edge\Application\msedge.exe",
                Browser::Brave => r"BraveSoftware\Brave-Browser\Application\brave.exe",
                Browser::Chromium => r"Chromium\Application\chrome.exe",
                Browser::Firefox => r"Mozilla Firefox\firefox.exe",
            };
            roots.iter().map(|r| r.join(rel)).collect()
        }
        Os::Mac => {
            let app = match browser {
                Browser::Chrome => "Google Chrome.app/Contents/MacOS/Google Chrome",
                Browser::Edge => "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                Browser::Brave => "Brave Browser.app/Contents/MacOS/Brave Browser",
                Browser::Chromium => "Chromium.app/Contents/MacOS/Chromium",
                Browser::Firefox => "Firefox.app/Contents/MacOS/firefox",
            };
            vec![Path::new("/Applications").join(app)]
        }
        _ => {
            let names: &[&str] = match browser {
                Browser::Chrome => &["google-chrome", "google-chrome-stable"],
                Browser::Edge => &["microsoft-edge", "microsoft-edge-stable"],
                Browser::Brave => &["brave-browser", "brave"],
                Browser::Chromium => &["chromium", "chromium-browser"],
                Browser::Firefox => &["firefox"],
            };
            let path = std::env::var_os("PATH").unwrap_or_default();
            std::env::split_paths(&path)
                .flat_map(|d| names.iter().map(move |n| d.join(n)))
                .collect()
        }
    };
    candidates.into_iter().find(|p| p.is_file())
}

/// Start `browser` on `url` (`chrome://extensions`, `about:debugging`), detached.
///
/// # Errors
/// The browser is not installed or cannot be started.
pub fn open_in(browser: Browser, url: &str) -> io::Result<()> {
    use crate::NoWindow as _;
    let exe = browser_exe(browser).ok_or(io::ErrorKind::NotFound)?;
    std::process::Command::new(exe).arg(url).no_window().spawn().map(drop)
}

/// Open a web URL or a folder with the system handler, detached.
///
/// # Errors
/// The opener cannot be started.
pub fn open(target: &str) -> io::Result<()> {
    use crate::NoWindow as _;
    let mut cmd = match crate::os() {
        Os::Windows => {
            let mut c = std::process::Command::new("explorer");
            c.arg(target);
            c
        }
        Os::Mac => {
            let mut c = std::process::Command::new("open");
            c.arg(target);
            c
        }
        _ => {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(target);
            c
        }
    };
    cmd.no_window().spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeRegistry(RefCell<HashMap<String, String>>);

    impl Registry for FakeRegistry {
        fn set_default(&self, key: &str, value: &str) -> io::Result<()> {
            self.0.borrow_mut().insert(key.to_owned(), value.to_owned());
            Ok(())
        }
        fn get_default(&self, key: &str) -> io::Result<Option<String>> {
            Ok(self.0.borrow().get(key).cloned())
        }
        fn delete(&self, key: &str) -> io::Result<bool> {
            Ok(self.0.borrow_mut().remove(key).is_some())
        }
    }

    fn env<'a>(os: Os, dir: &Path, reg: &'a FakeRegistry) -> Env<'a> {
        Env {
            os,
            home: dir.join("home"),
            config: dir.join("home/.config"),
            data: dir.join("data"),
            registry: reg,
        }
    }

    #[test]
    fn windows_writes_manifest_wrapper_and_registry_and_removes_them() {
        let dir = tempfile::tempdir().unwrap();
        let reg = FakeRegistry::default();
        let e = env(Os::Windows, dir.path(), &reg);
        let kor = Path::new(r"C:\Program Files\Kor\kor.exe");
        let i = install(&e, Browser::Chrome, kor, &["abcdefghijklmnopabcdefghijklmnop".into(), "bad".into()]).unwrap();
        assert_eq!(i.registry_key.as_deref(), Some(r"Software\Google\Chrome\NativeMessagingHosts\dev.kor.chrome_host"));
        let m: Value = serde_json::from_slice(&std::fs::read(&i.manifest).unwrap()).unwrap();
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(m["allowed_origins"].as_array().unwrap().len(), 2, "bad ids are dropped");
        let wrapper = std::fs::read_to_string(e.wrapper_path()).unwrap();
        assert!(wrapper.contains(r#""C:\Program Files\Kor\kor.exe" chrome-native-host"#));
        assert!(wrapper.contains("KOR_NATIVE_ORIGIN=%~1"));
        assert!(installed(&e, Browser::Chrome));
        assert!(!installed(&e, Browser::Edge));
        assert!(uninstall(&e, Browser::Chrome).unwrap());
        assert!(!installed(&e, Browser::Chrome));
        assert!(reg.0.borrow().is_empty());
        assert!(!e.wrapper_path().exists());
    }

    #[test]
    fn unix_paths_per_browser_and_firefox_lists_the_addon() {
        let dir = tempfile::tempdir().unwrap();
        let reg = FakeRegistry::default();
        let linux = env(Os::Linux, dir.path(), &reg);
        assert!(linux.manifest_path(Browser::Chrome).ends_with("home/.config/google-chrome/NativeMessagingHosts/dev.kor.chrome_host.json"));
        assert!(linux.manifest_path(Browser::Firefox).ends_with("home/.mozilla/native-messaging-hosts/dev.kor.chrome_host.json"));
        let mac = env(Os::Mac, dir.path(), &reg);
        assert!(mac.manifest_path(Browser::Edge).ends_with("Library/Application Support/Microsoft Edge/NativeMessagingHosts/dev.kor.chrome_host.json"));
        let i = install(&linux, Browser::Firefox, Path::new("/opt/kor/it's/kor"), &[]).unwrap();
        let m: Value = serde_json::from_slice(&std::fs::read(&i.manifest).unwrap()).unwrap();
        assert_eq!(m["allowed_extensions"], json!([FIREFOX_EXTENSION_ID]));
        assert!(m.get("allowed_origins").is_none());
        assert!(std::fs::read_to_string(linux.wrapper_path()).unwrap().contains(r"exec '/opt/kor/it'\''s/kor' chrome-native-host"));
        assert!(reg.0.borrow().is_empty(), "no registry outside Windows");
        // Someone else's manifest is not removed.
        std::fs::create_dir_all(linux.manifest_path(Browser::Chrome).parent().unwrap()).unwrap();
        std::fs::write(linux.manifest_path(Browser::Chrome), r#"{"path":"/other"}"#).unwrap();
        assert!(!uninstall(&linux, Browser::Chrome).unwrap());
        assert!(linux.manifest_path(Browser::Chrome).exists());
        assert!(uninstall(&linux, Browser::Firefox).unwrap());
    }

    #[test]
    fn browser_ids_parse() {
        assert_eq!(Browser::parse("Firefox"), Some(Browser::Firefox));
        assert_eq!(Browser::parse("safari"), None);
    }
}
