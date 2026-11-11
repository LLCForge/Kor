//! Chrome native messaging host registration (spec §24.4 "Chrome native host kaydını yapar",
//! ADR 0064). The installers call `kor install-hooks post-install`, which writes the host
//! manifest for Chrome, Chromium, Edge and Brave; `pre-uninstall` removes exactly those files.
//!
//! Chrome starts the host with the extension origin as its argument, so the manifest points at a
//! small launcher next to it that runs `kor chrome-native-host` without passing that on.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Os;

/// Per user (`HKCU`, home folder) or for every user (`HKLM`, `/etc`, `/Library`; needs root).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    System,
}

/// The host manifest Chrome reads.
pub fn manifest(name: &str, description: &str, launcher: &Path, extension_ids: &[String]) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "path": launcher.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": extension_ids.iter().map(|id| format!("chrome-extension://{id}/")).collect::<Vec<_>>(),
    })
}

/// Folders browsers read host manifests from (macOS / Linux). Windows uses the registry.
pub fn manifest_dirs(os: Os, scope: Scope, home: &Path) -> Vec<PathBuf> {
    match (os, scope) {
        (Os::Mac, Scope::User) => ["Google/Chrome", "Chromium", "Microsoft Edge", "BraveSoftware/Brave-Browser"]
            .iter()
            .map(|b| home.join("Library/Application Support").join(b).join("NativeMessagingHosts"))
            .collect(),
        (Os::Mac, Scope::System) => ["/Library/Google/Chrome/NativeMessagingHosts", "/Library/Application Support/Chromium/NativeMessagingHosts", "/Library/Microsoft/Edge/NativeMessagingHosts"]
            .iter()
            .map(PathBuf::from)
            .collect(),
        (Os::Linux | Os::Other, Scope::User) => ["google-chrome", "chromium", "microsoft-edge", "BraveSoftware/Brave-Browser"]
            .iter()
            .map(|b| home.join(".config").join(b).join("NativeMessagingHosts"))
            .collect(),
        (Os::Linux | Os::Other, Scope::System) => ["/etc/opt/chrome/native-messaging-hosts", "/etc/chromium/native-messaging-hosts", "/etc/opt/edge/native-messaging-hosts"]
            .iter()
            .map(PathBuf::from)
            .collect(),
        (Os::Windows, _) => Vec::new(),
    }
}

/// Registry keys (Windows) holding the manifest path, one per browser.
pub fn registry_keys(scope: Scope, name: &str) -> Vec<String> {
    let root = if scope == Scope::User { "HKCU" } else { "HKLM" };
    ["Google\\Chrome", "Chromium", "Microsoft\\Edge", "BraveSoftware\\Brave-Browser"]
        .iter()
        .map(|b| format!("{root}\\Software\\{b}\\NativeMessagingHosts\\{name}"))
        .collect()
}

/// Writes the launcher (`<name>.cmd` / `<name>.sh`) into `dir`; it runs `kor chrome-native-host`.
pub fn write_launcher(dir: &Path, name: &str, kor: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let (file, body) = if crate::os() == Os::Windows {
        (dir.join(format!("{name}.cmd")), format!("@echo off\r\n\"{}\" chrome-native-host\r\n", kor.display()))
    } else {
        (dir.join(format!("{name}.sh")), format!("#!/bin/sh\nexec '{}' chrome-native-host\n", kor.display().to_string().replace('\'', "'\\''")))
    };
    std::fs::write(&file, body)?;
    crate::make_executable(&file)?;
    Ok(file)
}

/// Where Kor keeps its own copy of the manifest and launcher for `scope`.
fn own_dir(scope: Scope) -> io::Result<PathBuf> {
    let slug = &crate::brand().product.slug;
    let base = match scope {
        Scope::User => crate::data_dir(),
        Scope::System if crate::os() == Os::Windows => std::env::var_os("ProgramData").map(|d| PathBuf::from(d).join(slug)),
        Scope::System => Some(PathBuf::from("/usr/local/lib").join(slug)),
    };
    base.map(|d| d.join("native-host")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data directory"))
}

fn reg(args: &[&str]) -> io::Result<()> {
    let ok = Command::new("reg").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status()?.success();
    if ok { Ok(()) } else { Err(io::Error::other(format!("reg {} failed", args.first().unwrap_or(&"")))) }
}

/// Registers `kor` as native host `name` for `extension_ids`. Returns the files and keys written.
/// No extension ids: nothing to allow, nothing written.
pub fn register(scope: Scope, name: &str, description: &str, kor: &Path, extension_ids: &[String]) -> io::Result<Vec<String>> {
    if extension_ids.is_empty() {
        return Ok(Vec::new());
    }
    let own = own_dir(scope)?;
    let launcher = write_launcher(&own, name, kor)?;
    let text = serde_json::to_string_pretty(&manifest(name, description, &launcher, extension_ids)).map_err(io::Error::other)?;
    let mut done = Vec::new();
    if crate::os() == Os::Windows {
        let file = own.join(format!("{name}.json"));
        std::fs::write(&file, &text)?;
        done.push(file.display().to_string());
        for key in registry_keys(scope, name) {
            reg(&["add", &key, "/ve", "/t", "REG_SZ", "/d", &file.to_string_lossy(), "/f"])?;
            done.push(key);
        }
        return Ok(done);
    }
    let home = dirs::home_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    for dir in manifest_dirs(crate::os(), scope, &home) {
        // A browser that was never installed has no profile folder; Chrome creates the
        // NativeMessagingHosts folder itself only on first use, so the parent is the signal.
        if scope == Scope::User && !dir.parent().is_some_and(Path::exists) {
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{name}.json"));
        std::fs::write(&file, &text)?;
        done.push(file.display().to_string());
    }
    Ok(done)
}

/// Removes what [`register`] wrote for `name` (missing files and keys are fine).
pub fn unregister(scope: Scope, name: &str) -> io::Result<()> {
    if crate::os() == Os::Windows {
        for key in registry_keys(scope, name) {
            let _ = reg(&["delete", &key, "/f"]);
        }
    } else if let Some(home) = dirs::home_dir() {
        for dir in manifest_dirs(crate::os(), scope, &home) {
            let _ = std::fs::remove_file(dir.join(format!("{name}.json")));
        }
    }
    let own = own_dir(scope)?;
    for f in [format!("{name}.json"), format!("{name}.cmd"), format!("{name}.sh")] {
        let _ = std::fs::remove_file(own.join(f));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_shape() {
        let ids = vec!["abcdefghijklmnopabcdefghijklmnop".to_owned()];
        let m = manifest("dev.kor.chrome", "Kor", Path::new("/opt/kor/dev.kor.chrome.sh"), &ids);
        assert_eq!(m["type"], "stdio");
        assert_eq!(m["allowed_origins"][0], "chrome-extension://abcdefghijklmnopabcdefghijklmnop/");
        assert_eq!(m["path"], "/opt/kor/dev.kor.chrome.sh");
    }

    #[test]
    fn locations_per_os() {
        let home = Path::new("/home/u");
        let linux = manifest_dirs(Os::Linux, Scope::User, home);
        assert_eq!(linux[0], Path::new("/home/u/.config/google-chrome/NativeMessagingHosts"));
        assert_eq!(manifest_dirs(Os::Linux, Scope::System, home)[0], Path::new("/etc/opt/chrome/native-messaging-hosts"));
        let mac = manifest_dirs(Os::Mac, Scope::User, Path::new("/Users/u"));
        assert_eq!(mac[0], Path::new("/Users/u/Library/Application Support/Google/Chrome/NativeMessagingHosts"));
        assert!(manifest_dirs(Os::Windows, Scope::User, home).is_empty());
        assert_eq!(registry_keys(Scope::User, "h")[0], "HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\h");
        assert_eq!(registry_keys(Scope::System, "h")[2], "HKLM\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\h");
    }

    #[test]
    fn launcher_runs_the_host_subcommand() {
        let dir = tempfile::tempdir().unwrap();
        let f = write_launcher(dir.path(), "h", Path::new("/x/kor")).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("chrome-native-host"), "{text}");
        assert!(!text.contains("%*") && !text.contains("\"$@\""), "the origin argument is not passed on");
    }

    #[test]
    fn nothing_registered_without_extension_ids() {
        assert!(register(Scope::User, "dev.example.test", "t", Path::new("k"), &[]).unwrap().is_empty());
    }
}
