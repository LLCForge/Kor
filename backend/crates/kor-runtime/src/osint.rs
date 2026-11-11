//! Ways into Kor from the operating system (Master Build Prompt §25.1, §25.3, §27.6; ADR 0052).
//!
//! The context-menu entries, the jump list / Dock menu, the global hotkey and `kor://` links all
//! end up as command-line arguments of `kor-desktop` (a second launch is forwarded to the running
//! window by the single-instance plugin) or as a URL. Both are parsed here into one [`Request`] so
//! the CLI, the desktop and the tests share the rules.
//!
//! Desktop arguments (written by `kor_platform::shell_integration`):
//! - `--open <path>…`: open the project of each path; files are attached as `@file` context.
//! - `--edit <path>…`: batch "Kor ile düzenle" (§27.6) over the files.
//! - `--task explain|bugs|test <path>…`: §25.1 extended submenu, starts the task.
//! - `--terminal <path>…`: "Kor CLI ile aç" through the desktop (no console window on Windows).
//! - `--palette`: bring Kor to the front with the command palette (global hotkey, jump list).
//! - `kor://…`: a deep link.
//!
//! URLs: `kor://open?path=…`, `kor://task?prompt=…[&path=…]`, `kor://resume?id=…`,
//! `kor://edit?path=…&path=…`, `kor://palette`, `kor://new` (new task), `kor://running` (running tasks).
//!
//! Security: a `kor://` link can come from any web page, so a [`Request`] built from a URL is
//! marked [`Origin::Link`]: the desktop asks before switching projects and never sends a linked
//! prompt by itself (it lands in the composer). Every value is length-checked here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Longest prompt accepted from a link or argument.
pub const MAX_PROMPT: usize = 8 * 1024;
/// Most paths in one request.
pub const MAX_PATHS: usize = 256;
/// Longest path.
const MAX_PATH_LEN: usize = 4096;

/// §25.1 extended submenu: tasks started straight from the file manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickTask {
    /// "Kor: Projeyi açıkla".
    Explain,
    /// "Kor: Hata bul".
    Bugs,
    /// "Kor: Bu dosyayı test et".
    Test,
}

impl QuickTask {
    pub const ALL: [Self; 3] = [Self::Explain, Self::Bugs, Self::Test];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explain => "explain",
            Self::Bugs => "bugs",
            Self::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// i18n id of the menu label (`osint-task-<name>`).
    pub fn label_id(self) -> String {
        format!("osint-task-{}", self.as_str())
    }

    /// The prompt sent to the agent; `files` are mentioned as `@path` (relative to `root`).
    pub fn prompt(self, l: &kor_i18n::Localizer, root: &Path, files: &[PathBuf]) -> String {
        let mentions = mentions(root, files);
        let base = l.t(&format!("osint-prompt-{}", self.as_str()));
        if mentions.is_empty() {
            base
        } else {
            format!("{base}\n\n{mentions}")
        }
    }
}

/// `@path` mentions of `files`, relative to `root` when inside it, one per line.
pub fn mentions(root: &Path, files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|f| {
            let shown = f.strip_prefix(root).unwrap_or(f);
            let s = shown.to_string_lossy().replace('\\', "/");
            if s.contains(char::is_whitespace) {
                format!("@\"{s}\"")
            } else {
                format!("@{s}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where a request came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// The user's own click in the file manager, jump list, Dock or hotkey.
    Shell,
    /// A `kor://` link (any web page or app can open one): confirm before acting.
    Link,
}

/// One thing the OS asked Kor to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Request {
    /// Open `root` as the project and attach `files`.
    Open {
        root: PathBuf,
        files: Vec<PathBuf>,
        origin: Origin,
    },
    /// Start a task. From a link the prompt is only put in the composer.
    Task {
        prompt: String,
        root: Option<PathBuf>,
        files: Vec<PathBuf>,
        quick: Option<QuickTask>,
        origin: Origin,
    },
    /// Open an earlier session.
    Resume { id: String, origin: Origin },
    /// §27.6 batch edit over `files`.
    Edit { files: Vec<PathBuf>, origin: Origin },
    /// Open `kor` in a terminal at `root` ("Kor CLI ile aç").
    Terminal { root: PathBuf, files: Vec<PathBuf> },
    /// Front window + command palette.
    Palette,
    /// Front window + new task.
    New,
    /// Front window + the running tasks view (jump list / Dock menu).
    Running,
}

/// Why an argument list or URL was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnknownScheme,
    UnknownAction(String),
    Missing(&'static str),
    TooLong(&'static str),
    BadId,
    RelativePath,
    BadEncoding,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownScheme => f.write_str("not a kor:// link"),
            Self::UnknownAction(a) => write!(f, "unknown kor:// action `{a}`"),
            Self::Missing(p) => write!(f, "missing `{p}`"),
            Self::TooLong(p) => write!(f, "`{p}` is too long"),
            Self::BadId => f.write_str("invalid session id"),
            Self::RelativePath => f.write_str("paths must be absolute"),
            Self::BadEncoding => f.write_str("invalid percent-encoding"),
        }
    }
}

impl std::error::Error for ParseError {}

/// The nearest ancestor of `path` (itself when it is a folder) holding `.git`, `AGENTS.md` or
/// `.kor/`; the folder itself (or a file's folder) when none does (§25.1).
pub fn project_root(path: &Path) -> PathBuf {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map_or_else(|| path.to_path_buf(), Path::to_path_buf)
    };
    start
        .ancestors()
        .find(|d| {
            d.join(".git").exists() || d.join("AGENTS.md").is_file() || d.join(".kor").is_dir()
        })
        .map_or_else(|| start.clone(), Path::to_path_buf)
}

/// Root and attached files for a set of selected paths: the project of the first one; files
/// (not folders) become context.
pub fn resolve(paths: &[PathBuf]) -> Option<(PathBuf, Vec<PathBuf>)> {
    let first = paths.first()?;
    let root = project_root(first);
    let files = paths.iter().filter(|p| !p.is_dir()).cloned().collect();
    Some((root, files))
}

fn check_paths(paths: &[PathBuf]) -> Result<(), ParseError> {
    if paths.len() > MAX_PATHS {
        return Err(ParseError::TooLong("path"));
    }
    for p in paths {
        if p.as_os_str().len() > MAX_PATH_LEN {
            return Err(ParseError::TooLong("path"));
        }
        if !p.is_absolute() {
            return Err(ParseError::RelativePath);
        }
    }
    Ok(())
}

fn clean_id(id: &str) -> Result<String, ParseError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| id.to_owned()).ok_or(ParseError::BadId)
}

/// Requests in a desktop argument list (`argv` without the program). Unknown arguments (Tauri,
/// macOS `-psn_…`) are ignored; malformed ones are returned as errors next to the good ones.
pub fn parse_args(args: &[String], cwd: &Path) -> Vec<Result<Request, ParseError>> {
    let mut out = Vec::new();
    let mut i = 0;
    let abs = |s: &str| {
        let p = PathBuf::from(s);
        if p.is_absolute() { p } else { cwd.join(p) }
    };
    while i < args.len() {
        let a = args[i].as_str();
        let take_paths = |from: usize| -> (Vec<PathBuf>, usize) {
            let mut j = from;
            let mut v = Vec::new();
            while j < args.len() && !args[j].starts_with("--") && !args[j].starts_with("kor:") {
                v.push(abs(&args[j]));
                j += 1;
            }
            (v, j)
        };
        match a {
            "--open" | "--edit" | "--terminal" => {
                let (paths, next) = take_paths(i + 1);
                i = next;
                out.push(from_paths(a, paths));
            }
            "--task" => {
                let quick = args.get(i + 1).and_then(|t| QuickTask::parse(t));
                let (paths, next) = take_paths(i + 2);
                i = next;
                out.push(match quick {
                    None => Err(ParseError::Missing("task")),
                    Some(q) => check_paths(&paths).and_then(|()| {
                        let (root, files) = resolve(&paths).ok_or(ParseError::Missing("path"))?;
                        Ok(Request::Task {
                            prompt: String::new(),
                            root: Some(root),
                            files,
                            quick: Some(q),
                            origin: Origin::Shell,
                        })
                    }),
                });
            }
            "--palette" => {
                out.push(Ok(Request::Palette));
                i += 1;
            }
            url if url.starts_with("kor:") => {
                out.push(parse_url(url));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

fn from_paths(flag: &str, paths: Vec<PathBuf>) -> Result<Request, ParseError> {
    check_paths(&paths)?;
    if paths.is_empty() {
        return Err(ParseError::Missing("path"));
    }
    Ok(match flag {
        "--edit" => Request::Edit {
            files: paths.into_iter().filter(|p| !p.is_dir()).collect(),
            origin: Origin::Shell,
        },
        "--terminal" => {
            let (root, files) = resolve(&paths).ok_or(ParseError::Missing("path"))?;
            Request::Terminal { root, files }
        }
        _ => {
            let (root, files) = resolve(&paths).ok_or(ParseError::Missing("path"))?;
            Request::Open {
                root,
                files,
                origin: Origin::Shell,
            }
        }
    })
}

/// Parse a `kor://` link.
///
/// # Errors
/// On another scheme, an unknown action, a missing or oversized value, or a relative path.
pub fn parse_url(url: &str) -> Result<Request, ParseError> {
    let rest = url
        .strip_prefix("kor://")
        .or_else(|| url.strip_prefix("kor:"))
        .ok_or(ParseError::UnknownScheme)?;
    if rest.len() > MAX_PROMPT * 4 {
        return Err(ParseError::TooLong("url"));
    }
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    let action = action.trim_end_matches('/');
    // An unknown action is reported as such, before its parameters are checked.
    if !matches!(
        action,
        "open" | "task" | "resume" | "edit" | "palette" | "new" | "running"
    ) {
        return Err(ParseError::UnknownAction(action.to_owned()));
    }
    let mut params: Vec<(String, String)> = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        params.push((decode(k)?, decode(v)?));
    }
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let paths: Vec<PathBuf> = params
        .iter()
        .filter(|(n, _)| n == "path")
        .map(|(_, v)| PathBuf::from(v))
        .collect();
    check_paths(&paths)?;
    match action {
        "open" => {
            let (root, files) = resolve(&paths).ok_or(ParseError::Missing("path"))?;
            Ok(Request::Open {
                root,
                files,
                origin: Origin::Link,
            })
        }
        "task" => {
            let prompt = get("prompt").ok_or(ParseError::Missing("prompt"))?;
            if prompt.len() > MAX_PROMPT {
                return Err(ParseError::TooLong("prompt"));
            }
            let (root, files) = match resolve(&paths) {
                Some((r, f)) => (Some(r), f),
                None => (None, Vec::new()),
            };
            Ok(Request::Task {
                prompt,
                root,
                files,
                quick: None,
                origin: Origin::Link,
            })
        }
        "resume" => Ok(Request::Resume {
            id: clean_id(&get("id").ok_or(ParseError::Missing("id"))?)?,
            origin: Origin::Link,
        }),
        "edit" => {
            if paths.is_empty() {
                return Err(ParseError::Missing("path"));
            }
            Ok(Request::Edit {
                files: paths,
                origin: Origin::Link,
            })
        }
        "palette" => Ok(Request::Palette),
        "new" => Ok(Request::New),
        "running" => Ok(Request::Running),
        other => Err(ParseError::UnknownAction(other.to_owned())),
    }
}

/// Percent-encode a query value (RFC 3986 unreserved characters kept).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> Result<String, ParseError> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3).ok_or(ParseError::BadEncoding)?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| ParseError::BadEncoding)?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| ParseError::BadEncoding)
}

/// `kor://open?path=…` for a folder (jump list, Dock menu, editor "Kor'da aç").
pub fn open_url(path: &Path) -> String {
    format!("kor://open?path={}", encode(&path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn project_root_walks_up_to_markers() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("proj");
        let deep = proj.join("src").join("a");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        let file = deep.join("x.rs");
        std::fs::write(&file, "").unwrap();
        assert_eq!(project_root(&file), proj);
        assert_eq!(project_root(&deep), proj);
        let lone = d.path().join("lone");
        std::fs::create_dir_all(&lone).unwrap();
        assert_eq!(project_root(&lone), lone);
        std::fs::write(lone.join("AGENTS.md"), "").unwrap();
        let sub = lone.join("b");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(project_root(&sub), lone);
    }

    #[test]
    fn desktop_arguments_become_requests() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        std::fs::create_dir_all(root.join(".kor")).unwrap();
        let f1 = root.join("a.txt");
        let f2 = root.join("b c.txt");
        std::fs::write(&f1, "").unwrap();
        std::fs::write(&f2, "").unwrap();
        let args = s(&[
            "-psn_0_1",
            "--open",
            &f1.to_string_lossy(),
            "--edit",
            &f1.to_string_lossy(),
            &f2.to_string_lossy(),
            "--task",
            "bugs",
            &root.to_string_lossy(),
            "--palette",
            "kor://resume?id=abc-1",
        ]);
        let got: Vec<Request> = parse_args(&args, &root)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            got[0],
            Request::Open {
                root: root.clone(),
                files: vec![f1.clone()],
                origin: Origin::Shell
            }
        );
        assert_eq!(
            got[1],
            Request::Edit {
                files: vec![f1.clone(), f2.clone()],
                origin: Origin::Shell
            }
        );
        assert!(
            matches!(&got[2], Request::Task { quick: Some(QuickTask::Bugs), root: Some(r), files, .. } if *r == root && files.is_empty())
        );
        assert_eq!(got[3], Request::Palette);
        assert_eq!(
            got[4],
            Request::Resume {
                id: "abc-1".into(),
                origin: Origin::Link
            }
        );
        // Relative paths are taken from the second launch's working directory.
        let rel = parse_args(&s(&["--open", "a.txt"]), &root);
        assert!(matches!(&rel[0], Ok(Request::Open { files, .. }) if files == &vec![f1.clone()]));
        assert!(parse_args(&s(&["--task", "nope", "x"]), &root)[0].is_err());
        assert!(parse_args(&s(&["--open"]), &root)[0].is_err());
    }

    #[test]
    fn links_are_decoded_checked_and_marked() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("my proj");
        std::fs::create_dir_all(&p).unwrap();
        let url = open_url(&p);
        assert!(!url.contains(' '));
        assert_eq!(
            parse_url(&url).unwrap(),
            Request::Open {
                root: p.clone(),
                files: vec![],
                origin: Origin::Link
            }
        );
        let t = parse_url("kor://task?prompt=fix%20the%20%C3%A7%C4%B1kt%C4%B1+now").unwrap();
        assert!(
            matches!(t, Request::Task { ref prompt, origin: Origin::Link, root: None, .. } if prompt == "fix the çıktı now")
        );
        assert_eq!(parse_url("kor://palette/").unwrap(), Request::Palette);
        assert_eq!(parse_url("kor:new").unwrap(), Request::New);
        assert_eq!(parse_url("kor://running").unwrap(), Request::Running);
        assert_eq!(parse_url("http://x"), Err(ParseError::UnknownScheme));
        assert_eq!(
            parse_url("kor://rm?path=/"),
            Err(ParseError::UnknownAction("rm".into()))
        );
        assert_eq!(
            parse_url("kor://resume?id=../../etc"),
            Err(ParseError::BadId)
        );
        assert_eq!(
            parse_url("kor://open?path=relative/x"),
            Err(ParseError::RelativePath)
        );
        assert_eq!(
            parse_url("kor://task?prompt=%ZZ"),
            Err(ParseError::BadEncoding)
        );
        assert_eq!(parse_url("kor://task"), Err(ParseError::Missing("prompt")));
        let long = format!("kor://task?prompt={}", "a".repeat(MAX_PROMPT + 1));
        assert_eq!(parse_url(&long), Err(ParseError::TooLong("prompt")));
    }

    #[test]
    fn mentions_are_relative_and_quoted() {
        let root = Path::new("/p");
        let m = mentions(
            root,
            &[PathBuf::from("/p/src/a.rs"), PathBuf::from("/p/b c.md")],
        );
        assert_eq!(m, "@src/a.rs\n@\"b c.md\"");
        assert_eq!(QuickTask::parse("test"), Some(QuickTask::Test));
    }
}
