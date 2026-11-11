//! Recognize sandbox blocks in a confined command's output (ADR 0033).
//!
//! The OS does the blocking; this only reads what the failed call printed, so it is best effort:
//! a program that swallows its own error leaves nothing to find. A path inside the project is
//! never a sandbox block (it is a plain permission problem), so such lines are skipped.

use crate::{Policy, SandboxKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenialKind {
    /// A file write (or read) outside the allowed area.
    File,
    /// A network connection while the network is off.
    Network,
}

impl DenialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Network => "network",
        }
    }
}

/// One block found in the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    pub kind: DenialKind,
    /// The path or host, when the message names one.
    pub target: Option<String>,
    pub backend: SandboxKind,
    /// The output line it was found in (at most 300 characters).
    pub line: String,
}

impl Denial {
    /// One line for the tool result and notices.
    pub fn describe(&self) -> String {
        let what = match self.kind {
            DenialKind::File => "access outside the project",
            DenialKind::Network => "network access",
        };
        match &self.target {
            Some(t) => format!("blocked by {}: {what}: {t}", self.backend.as_str()),
            None => format!("blocked by {}: {what}", self.backend.as_str()),
        }
    }
}

/// Messages that name a path only sometimes; a line with no path still counts. Windows
/// `PowerShell` 5.1 prints localized text in the OEM code page, so the error id is the reliable
/// part there; the Turkish words are ASCII-only on purpose.
const FILE_ALWAYS: &[&str] = &[
    "read-only file system",
    "access to the path",
    "access is denied",
    "unauthorizedaccess",
    "reddedildi",
    "engellendi",
];
/// Messages common outside sandboxes too; they count only with an absolute path outside the root.
const FILE_WITH_PATH: &[&str] = &["permission denied", "operation not permitted"];
const NETWORK: &[&str] = &[
    "network is unreachable",
    "could not resolve host",
    "temporary failure in name resolution",
    "name or service not known",
    "no such host is known",
    "forbidden by its access permissions",
    "getaddrinfo",
    "enotfound",
    "eai_again",
    "bad address",
];
const MAX: usize = 10;

/// Blocks in `output` of a command confined by `kind` with `policy`.
pub fn denials(kind: SandboxKind, policy: &Policy, output: &str) -> Vec<Denial> {
    if !kind.is_enforcing() {
        return Vec::new();
    }
    let output = strip_ansi(output);
    // PowerShell wraps long messages mid-path; the joined text has the path whole again.
    let joined: String = output.lines().collect();
    let mut wrapped = None;
    let mut found: Vec<Denial> = Vec::new();
    for line in output.lines() {
        let lower = line.to_lowercase();
        let has = |pats: &[&str]| pats.iter().any(|p| lower.contains(p));
        let path = absolute_path(line);
        let d = if has(FILE_ALWAYS) || has(FILE_WITH_PATH) {
            match path {
                Some(p) if inside(&p, policy) => continue,
                None if !has(FILE_ALWAYS) => continue,
                None => {
                    let p = wrapped
                        .get_or_insert_with(|| quoted_paths(&joined).find(|p| !inside(p, policy)))
                        .clone();
                    (DenialKind::File, p)
                }
                p => (DenialKind::File, p),
            }
        } else if !policy.network && has(NETWORK) {
            (DenialKind::Network, host(line))
        } else {
            continue;
        };
        if found.iter().any(|f| f.kind == d.0 && f.target == d.1) {
            continue;
        }
        found.push(Denial {
            kind: d.0,
            target: d.1,
            backend: kind,
            line: line.trim().chars().take(300).collect(),
        });
        if found.len() == MAX {
            break;
        }
    }
    found
}

fn is_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('/')
        || s.starts_with(r"\\")
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/'))
}

fn trim_path(s: &str) -> String {
    s.trim_end_matches([':', ',', '.', ';', ')']).to_owned()
}

/// Absolute paths in quotes, in order.
fn quoted_paths(text: &str) -> impl Iterator<Item = String> + '_ {
    [('\'', '\''), ('"', '"'), ('‘', '’'), ('`', '\'')]
        .into_iter()
        .flat_map(move |(open, close)| {
            let mut rest = text;
            std::iter::from_fn(move || {
                loop {
                    let i = rest.find(open)?;
                    let after = &rest[i + open.len_utf8()..];
                    let j = after.find(close)?;
                    let quoted = &after[..j];
                    rest = &after[j + close.len_utf8()..];
                    if is_absolute(quoted) {
                        return Some(trim_path(quoted));
                    }
                }
            })
        })
}

/// The first absolute path in `line`: a quoted one first, else a bare word.
fn absolute_path(line: &str) -> Option<String> {
    quoted_paths(line).next().or_else(|| {
        line.split(|c: char| c.is_whitespace() || c == '\'' || c == '"')
            .map(|w| w.trim_start_matches(['(', '[']))
            .find(|w| is_absolute(w) && w.len() > 1)
            .map(trim_path)
    })
}

/// `text` without terminal color codes (`ESC [ … letter`), which pwsh 7 prints even into pipes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn inside(path: &str, policy: &Policy) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let root = norm(&policy.root.to_string_lossy());
    let p = norm(path);
    p == root || p.starts_with(&format!("{root}/"))
}

/// A host named in a network error (`Could not resolve host: example.com`), if any.
fn host(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let at = [
        "could not resolve host:",
        "getaddrinfo enotfound",
        "getaddrinfo eai_again",
    ]
    .iter()
    .find_map(|p| lower.find(p).map(|i| i + p.len()))?;
    line[at..]
        .split_whitespace()
        .next()
        .map(|h| {
            h.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != ':')
                .to_owned()
        })
        .filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(root: &str, network: bool) -> Policy {
        Policy {
            network,
            ..Policy::new(root)
        }
    }

    #[test]
    fn appcontainer_access_denied_names_the_path() {
        let p = policy(r"C:\Work\Proj", false);
        let out = "Set-Content : Access to the path 'C:\\Users\\user\\AppData\\Local\\Temp\\o\\x.txt' is denied.\n\
                   At line:2 char:1";
        let d = denials(SandboxKind::AppContainer, &p, out);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].kind, DenialKind::File);
        assert_eq!(
            d[0].target.as_deref(),
            Some(r"C:\Users\user\AppData\Local\Temp\o\x.txt")
        );
        assert!(d[0].describe().contains("appcontainer"));
        // Inside the project: not a sandbox block.
        let inside = "Access to the path 'c:\\work\\proj\\locked.txt' is denied.";
        assert!(denials(SandboxKind::AppContainer, &p, inside).is_empty());
        // cmd.exe names no path.
        let d = denials(SandboxKind::AppContainer, &p, "Access is denied.");
        assert_eq!((d.len(), d[0].target.as_deref()), (1, None));
    }

    #[test]
    fn localized_powershell_5_and_colored_pwsh_7() {
        let p = policy(r"C:\Users\user\AppData\Local\Temp\kor-sandbox-probe-1", false);
        // Windows PowerShell 5.1 on a Turkish system: OEM code page, path wrapped mid-name.
        let ps5 = "Attempting to perform the InitializeDefaultDrives operation on the 'FileSystem' provider failed.\r\n\
                   Exception calling \"WriteAllText\" with \"2\" argument(s): \"'C:\\Users\\user\\AppData\\Local\\kor\\sandbox-probe\\outside-1.txt\r\n\
                   ' yoluna eri\u{fffd}im reddedildi.\"\r\n\
                   At line:2 char:1\r\n\
                   + [IO.File]::WriteAllText('C:\\Users\\user\\AppData\\Local\\kor\\sandbox-probe\\o ...\r\n\
                   \x20   + FullyQualifiedErrorId : UnauthorizedAccessException\r\n";
        let d = denials(SandboxKind::AppContainer, &p, ps5);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(
            d[0].target.as_deref(),
            Some(r"C:\Users\user\AppData\Local\kor\sandbox-probe\outside-1.txt")
        );
        // pwsh 7 colors its errors even into a pipe.
        let ps7 = "\u{1b}[31;1mSet-Content: \u{1b}[0m\n\u{1b}[31;1m\u{1b}[36;1m     | \u{1b}[31;1mAccess to the path 'C:\\Users\\user\\o.txt' is denied.\u{1b}[0m";
        let d = denials(SandboxKind::AppContainer, &p, ps7);
        assert_eq!(d[0].target.as_deref(), Some(r"C:\Users\user\o.txt"), "{d:?}");
        // A normal sandboxed PowerShell start prints this warning; it is not a block.
        let warn = "'FileSystem' sa\u{fffd}lay\u{fffd}c\u{fffd}s\u{fffd}nda InitializeDefaultDrives i\u{fffd}lemini ger\u{fffd}ekle\u{fffd}tirme giri\u{fffd}imi ba\u{fffd}ar\u{fffd}s\u{fffd}z oldu.";
        assert!(denials(SandboxKind::AppContainer, &p, warn).is_empty());
    }

    #[test]
    fn unix_read_only_and_eperm_messages() {
        let p = policy("/home/u/proj", false);
        let out = "sh: 1: cannot create /home/u/.bashrc: Read-only file system\n\
                   touch: cannot touch '/etc/x': Read-only file system\n\
                   PermissionError: [Errno 1] Operation not permitted: '/Users/u/secret'\n\
                   bash: ./run.sh: Permission denied\n\
                   cp: cannot create regular file '/home/u/proj/a': Permission denied";
        let d = denials(SandboxKind::Bubblewrap, &p, out);
        let targets: Vec<_> = d.iter().filter_map(|d| d.target.as_deref()).collect();
        assert_eq!(targets, ["/home/u/.bashrc", "/etc/x", "/Users/u/secret"]);
    }

    #[test]
    fn network_only_when_the_network_is_off() {
        let out = "curl: (6) Could not resolve host: example.com\nconnect: Network is unreachable";
        let d = denials(SandboxKind::Docker, &policy("/p", false), out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].target.as_deref(), Some("example.com"));
        assert!(d.iter().all(|d| d.kind == DenialKind::Network));
        assert!(denials(SandboxKind::Docker, &policy("/p", true), out).is_empty());
        assert!(denials(SandboxKind::None, &policy("/p", false), out).is_empty());
    }

    #[test]
    fn repeated_lines_are_reported_once_and_capped() {
        let line = "touch: cannot touch '/etc/x': Read-only file system\n";
        assert_eq!(
            denials(
                SandboxKind::Bubblewrap,
                &policy("/p", false),
                &line.repeat(5)
            )
            .len(),
            1
        );
        let many = (0..30).fold(String::new(), |mut s, i| {
            s.push_str("cannot create /etc/f");
            s.push_str(&i.to_string());
            s.push_str(": Read-only file system\n");
            s
        });
        assert_eq!(
            denials(SandboxKind::Bubblewrap, &policy("/p", false), &many).len(),
            MAX
        );
    }
}
