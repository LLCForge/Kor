//! WSL2 awareness (Master Build Prompt §27.10, ADR 0052): a project opened from Windows at
//! `\\wsl$\<distro>\…` or `\\wsl.localhost\<distro>\…` lives inside a Linux distro. Its commands
//! run there (`wsl.exe -d <distro> --cd <linux path> --exec sh -lc <command>`), and paths are
//! mapped both ways so Windows-side tools get a Windows path and Linux tools a Linux one.

use std::path::{Path, PathBuf};

/// A path inside a WSL distro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslPath {
    pub distro: String,
    /// Absolute Linux path (`/home/x/proj`).
    pub path: String,
}

/// `\\wsl$\Ubuntu\home\x` / `\\wsl.localhost\Ubuntu\home\x` (also with `/` or `\\?\UNC\`) →
/// distro and Linux path. `None` for any other path.
pub fn parse(p: &Path) -> Option<WslPath> {
    let s = p.to_string_lossy().replace('/', "\\");
    let s = s
        .strip_prefix(r"\\?\UNC\")
        .map(|r| format!(r"\\{r}"))
        .unwrap_or(s);
    let rest = s.strip_prefix(r"\\")?;
    let mut parts = rest.split('\\').filter(|c| !c.is_empty());
    let host = parts.next()?.to_ascii_lowercase();
    if host != "wsl$" && host != "wsl.localhost" {
        return None;
    }
    let distro = parts.next()?.to_owned();
    if distro.contains(['<', '>', ':', '"', '|', '?', '*']) {
        return None;
    }
    let path: Vec<&str> = parts.collect();
    Some(WslPath {
        distro,
        path: format!("/{}", path.join("/")),
    })
}

/// The Windows (UNC) path of `linux` inside `distro`.
pub fn to_windows(distro: &str, linux: &str) -> PathBuf {
    let tail = linux.trim_start_matches('/').replace('/', "\\");
    PathBuf::from(format!(r"\\wsl.localhost\{distro}\{tail}"))
}

/// A Windows path seen from Linux inside WSL: `C:\x\y` → `/mnt/c/x/y`; a `\\wsl…` path → its
/// Linux path. `None` for other UNC paths.
pub fn to_linux(p: &Path) -> Option<String> {
    if let Some(w) = parse(p) {
        return Some(w.path);
    }
    let s = p.to_string_lossy().replace('\\', "/");
    let b = s.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| {
        let drive = char::from(b[0]).to_ascii_lowercase();
        format!("/mnt/{drive}{}", &s[2..])
    })
}

/// Program and arguments that run `command` in a shell inside the distro of `cwd`, in `cwd`.
/// `None` when not on Windows or `cwd` is not a WSL path (run it normally).
pub fn command(cwd: &Path, command: &str) -> Option<(String, Vec<String>)> {
    if crate::os() != crate::Os::Windows {
        return None;
    }
    command_for(cwd, command)
}

fn command_for(cwd: &Path, command: &str) -> Option<(String, Vec<String>)> {
    let w = parse(cwd)?;
    Some((
        "wsl.exe".into(),
        vec![
            "-d".into(),
            w.distro,
            "--cd".into(),
            w.path,
            // `--exec`: no extra shell parsing by wsl.exe; the command reaches `sh` as one argument.
            "--exec".into(),
            "sh".into(),
            "-lc".into(),
            command.into(),
        ],
    ))
}

/// Installed distros (`wsl.exe -l -q`; UTF-16 output). Empty when WSL is missing.
pub fn distros() -> Vec<String> {
    use crate::NoWindow as _;
    if crate::os() != crate::Os::Windows {
        return Vec::new();
    }
    std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .no_window()
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_list(&o.stdout))
        .unwrap_or_default()
}

fn parse_list(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).all(|&b| b == 0) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_paths_map_both_ways() {
        let w = parse(Path::new(r"\\wsl$\Ubuntu-22.04\home\me\proj")).unwrap();
        assert_eq!(
            w,
            WslPath {
                distro: "Ubuntu-22.04".into(),
                path: "/home/me/proj".into()
            }
        );
        let w2 = parse(Path::new("//wsl.localhost/Debian/")).unwrap();
        assert_eq!(w2.path, "/");
        let w3 = parse(Path::new(r"\\?\UNC\wsl.localhost\Arch\srv")).unwrap();
        assert_eq!((w3.distro.as_str(), w3.path.as_str()), ("Arch", "/srv"));
        assert!(parse(Path::new(r"\\server\share\x")).is_none());
        assert!(parse(Path::new(r"C:\Users\me")).is_none());
        assert!(parse(Path::new(r"\\wsl$")).is_none());
        assert_eq!(
            to_windows("Ubuntu", "/home/me/a b"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\a b")
        );
        assert_eq!(
            parse(&to_windows("Ubuntu", "/home/me")).unwrap().path,
            "/home/me"
        );
        assert_eq!(
            to_linux(Path::new(r"C:\Users\me\x.txt")).unwrap(),
            "/mnt/c/Users/me/x.txt"
        );
        assert_eq!(to_linux(Path::new(r"\\wsl$\U\etc")).unwrap(), "/etc");
        assert!(to_linux(Path::new(r"\\nas\x")).is_none());
    }

    #[test]
    fn commands_run_inside_the_distro() {
        let (p, a) = command_for(
            Path::new(r"\\wsl$\Ubuntu\home\me\p"),
            "cargo test && echo 'ok'",
        )
        .unwrap();
        assert_eq!(p, "wsl.exe");
        assert_eq!(
            a,
            [
                "-d",
                "Ubuntu",
                "--cd",
                "/home/me/p",
                "--exec",
                "sh",
                "-lc",
                "cargo test && echo 'ok'"
            ]
        );
        assert!(command_for(Path::new(r"C:\p"), "ls").is_none());
        if crate::os() != crate::Os::Windows {
            assert!(command(Path::new(r"\\wsl$\Ubuntu\p"), "ls").is_none());
        }
    }

    #[test]
    fn distro_list_is_read_as_utf16() {
        let utf16: Vec<u8> = "Ubuntu\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(parse_list(&utf16), ["Ubuntu", "Debian"]);
        assert_eq!(parse_list(b"Alpine\n"), ["Alpine"]);
    }
}
