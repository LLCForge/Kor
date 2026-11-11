//! Git history for the metrics: commits with per-file line counts, blame, renames and the last
//! change per file. One `git` process per question; output is parsed with record separators so
//! subjects and paths with spaces survive. Nothing is written to the repository.

use kor_platform::NoWindow as _;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// Run `git -c core.quotepath=off <args>` in `root`; stdout, or stderr as the error.
///
/// # Errors
/// git missing or a non-zero exit.
pub fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").no_window()
        .args(["-c", "core.quotepath=off"])
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|s| s.trim() == "true")
}

pub fn head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// `user.email` of the repository (the "me" of `kor style` and `kor stats`).
pub fn user_email(root: &Path) -> Option<String> {
    git(root, &["config", "user.email"])
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

/// Commit time of `rev` (Unix seconds).
pub fn commit_time(root: &Path, rev: &str) -> Option<i64> {
    kor_graph::git::validate_rev(rev).ok()?;
    git(root, &["log", "-1", "--format=%ct", rev, "--"])
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Last commit on HEAD made before `ts`.
pub fn rev_before(root: &Path, ts: i64) -> Option<String> {
    let before = format!("--before={ts}");
    git(root, &["rev-list", "-1", &before, "HEAD", "--"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileChange {
    /// Path after the commit (`/`-separated, repository-relative).
    pub path: String,
    /// Path before, when the commit renamed or moved the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `None` for binary files.
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    pub email: String,
    /// Author time, Unix seconds.
    pub time: i64,
    pub subject: String,
    pub files: Vec<FileChange>,
}

impl Commit {
    pub fn added(&self) -> u64 {
        self.files
            .iter()
            .filter_map(|f| f.added)
            .map(u64::from)
            .sum()
    }

    pub fn deleted(&self) -> u64 {
        self.files
            .iter()
            .filter_map(|f| f.deleted)
            .map(u64::from)
            .sum()
    }
}

#[derive(Debug, Clone, Default)]
pub struct LogOpts<'a> {
    /// Only commits touching this path.
    pub path: Option<&'a str>,
    /// Follow the path through renames (`--follow`).
    pub follow: bool,
    /// Author time lower bound (Unix seconds).
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// 0 = no limit.
    pub max: usize,
    /// Only commits by this author email (case-insensitive substring, as `git --author`).
    pub author: Option<&'a str>,
}

/// Non-merge commits on HEAD, newest first, with `--numstat -M` per file.
///
/// # Errors
/// git failures (not a repository, bad path).
pub fn log(root: &Path, o: &LogOpts) -> Result<Vec<Commit>, String> {
    let mut args: Vec<String> = [
        "log",
        "--no-merges",
        "-M",
        "--numstat",
        "--format=%x1e%H%x1f%an%x1f%ae%x1f%at%x1f%s",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if o.follow && o.path.is_some() {
        args.push("--follow".into());
    }
    if let Some(s) = o.since {
        args.push(format!("--since=@{s}"));
    }
    if let Some(u) = o.until {
        args.push(format!("--until=@{u}"));
    }
    if o.max > 0 {
        args.push(format!("-n{}", o.max));
    }
    if let Some(a) = o.author {
        // `--author` is a regex; escape so an email is matched literally.
        args.push(format!("--author={}", regex::escape(a)));
        args.push("--regexp-ignore-case".into());
    }
    args.push("HEAD".into());
    args.push("--".into());
    if let Some(p) = o.path {
        args.push(p.to_owned());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match git(root, &refs) {
        Ok(out) => Ok(parse_log(&out)),
        // A repository without commits has no HEAD: no history, not an error.
        Err(e) if e.contains("does not have any commits") || e.contains("unknown revision") => {
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

/// Parse `--format=%x1e%H%x1f%an%x1f%ae%x1f%at%x1f%s --numstat` output.
pub fn parse_log(out: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    for rec in out.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let Some(head) = lines.next() else { continue };
        let f: Vec<&str> = head.split('\x1f').collect();
        if f.len() < 5 {
            continue;
        }
        let files = lines
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                let (a, d, path) = (p.next()?, p.next()?, p.next()?);
                let (from, path) = rename_paths(path);
                Some(FileChange {
                    path,
                    from,
                    added: a.parse().ok(),
                    deleted: d.parse().ok(),
                })
            })
            .collect();
        commits.push(Commit {
            hash: f[0].to_owned(),
            author: f[1].to_owned(),
            email: f[2].to_lowercase(),
            time: f[3].parse().unwrap_or(0),
            subject: f[4].to_owned(),
            files,
        });
    }
    commits
}

/// `src/{a.rs => b.rs}` / `a.rs => b.rs` → (`Some(old)`, new); a plain path → (`None`, path).
pub fn rename_paths(p: &str) -> (Option<String>, String) {
    if let (Some(open), Some(close)) = (p.find('{'), p.rfind('}'))
        && open < close
        && let Some((a, b)) = p[open + 1..close].split_once(" => ")
    {
        let (pre, post) = (&p[..open], &p[close + 1..]);
        let join = |mid: &str| format!("{pre}{mid}{post}").replace("//", "/");
        return (Some(join(a)), join(b));
    }
    match p.split_once(" => ") {
        Some((a, b)) => (Some(a.to_owned()), b.to_owned()),
        None => (None, p.to_owned()),
    }
}

/// One line of `git blame`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlameLine {
    pub line: u32,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub hash: String,
}

/// `git blame --line-porcelain -w -M` of `file` at HEAD.
///
/// # Errors
/// git failures (untracked file, not a repository).
pub fn blame(root: &Path, file: &str) -> Result<Vec<BlameLine>, String> {
    let out = git(
        root,
        &["blame", "--line-porcelain", "-w", "-M", "HEAD", "--", file],
    )?;
    Ok(parse_blame(&out))
}

pub fn parse_blame(out: &str) -> Vec<BlameLine> {
    let mut rows = Vec::new();
    let mut cur = BlameLine {
        line: 0,
        author: String::new(),
        email: String::new(),
        time: 0,
        hash: String::new(),
    };
    for l in out.lines() {
        if l.starts_with('\t') {
            rows.push(cur.clone());
        } else if let Some(v) = l.strip_prefix("author-mail ") {
            cur.email = v.trim_matches(['<', '>']).to_lowercase();
        } else if let Some(v) = l.strip_prefix("author-time ") {
            cur.time = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = l.strip_prefix("author ") {
            v.clone_into(&mut cur.author);
        } else {
            let mut w = l.split(' ');
            if let (Some(h), Some(_), Some(n)) = (w.next(), w.next(), w.next())
                && h.len() >= 40
                && h.bytes().all(|b| b.is_ascii_hexdigit())
            {
                h.clone_into(&mut cur.hash);
                cur.line = n.parse().unwrap_or(0);
            }
        }
    }
    rows
}

/// Unix time of the last commit touching each file (current paths), from one `git log`.
pub fn last_change(root: &Path) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    let Ok(text) = git(
        root,
        &[
            "log",
            "--no-merges",
            "--format=%x1e%at",
            "--name-only",
            "HEAD",
            "--",
        ],
    ) else {
        return out;
    };
    for rec in text.split('\x1e') {
        let mut lines = rec.lines();
        let Some(t) = lines.next().and_then(|t| t.trim().parse::<i64>().ok()) else {
            continue;
        };
        for f in lines.map(str::trim).filter(|f| !f.is_empty()) {
            out.entry(f.to_owned()).or_insert(t);
        }
    }
    out
}

/// Commits per file in `[since, now]` (current paths only; renames count for the new name).
pub fn churn(commits: &[Commit], since: i64) -> HashMap<String, u32> {
    let mut m: HashMap<String, u32> = HashMap::new();
    for c in commits.iter().filter(|c| c.time >= since) {
        for f in &c.files {
            *m.entry(f.path.clone()).or_default() += 1;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_in_numstat_are_split() {
        assert_eq!(
            rename_paths("src/{old.rs => new.rs}"),
            (Some("src/old.rs".into()), "src/new.rs".into())
        );
        assert_eq!(
            rename_paths("{a => b}/x.rs"),
            (Some("a/x.rs".into()), "b/x.rs".into())
        );
        assert_eq!(
            rename_paths("src/{ => util}/x.rs"),
            (Some("src/x.rs".into()), "src/util/x.rs".into())
        );
        assert_eq!(rename_paths("a b.rs"), (None, "a b.rs".into()));
    }

    #[test]
    fn log_and_blame_parse() {
        let out = "\x1eabc\x1fAyşe\x1fA@X.com\x1f100\x1ffix: thing\n3\t1\tsrc/a.rs\n-\t-\timg.png\n\n\x1edef\x1fBo\x1fb@x\x1f50\x1finit\n10\t0\tsrc/a.rs\n";
        let c = parse_log(out);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].email, "a@x.com");
        assert_eq!(c[0].added(), 3);
        assert_eq!(c[0].files[1].added, None);
        let b = parse_blame(
            "0123456789012345678901234567890123456789 1 1 1\nauthor Ayşe\nauthor-mail <a@x.com>\nauthor-time 42\n\tfn x() {}\n",
        );
        assert_eq!(b[0].author, "Ayşe");
        assert_eq!((b[0].line, b[0].time), (1, 42));
    }
}
