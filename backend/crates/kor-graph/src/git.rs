//! Git layer: co-change and churn from bounded history, and file contents at a revision (for `diff`).

use crate::{Error, Result};
use kor_platform::NoWindow as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Commits read for co-change and churn.
pub const HISTORY: usize = 500;
/// Commits touching more files than this are bulk changes (renames, formatting) and are skipped.
const MAX_FILES_PER_COMMIT: usize = 30;
const MAX_COCHANGE_EDGES: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GitStats {
    pub head: String,
    /// Commits per file in the last 30 days.
    pub churn: HashMap<String, u16>,
    /// `(a, b, ratio)` with `a < b`; ratio = shared commits / commits of the less-changed file.
    pub cochange: Vec<(String, String, f32)>,
}

fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.no_window();
    c.current_dir(root)
        .args(["-c", "core.quotepath=off"])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// Current `HEAD` commit, `None` outside a git repository.
pub fn head(root: &Path) -> Option<String> {
    let out = git(root).args(["rev-parse", "HEAD"]).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Read the last [`HISTORY`] commits under `root`. `None` outside a git repository.
pub fn stats(root: &Path, now_secs: u64) -> Option<GitStats> {
    let head = head(root)?;
    let out = git(root)
        .args([
            "log",
            "-n",
            &HISTORY.to_string(),
            "--no-renames",
            "--name-only",
            "--relative",
            "--format=%x01%ct",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut commits: Vec<(u64, Vec<String>)> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(ts) = line.strip_prefix('\u{1}') {
            commits.push((ts.trim().parse().unwrap_or(0), Vec::new()));
        } else if !line.trim().is_empty()
            && let Some(c) = commits.last_mut()
        {
            c.1.push(line.trim().replace('\\', "/"));
        }
    }
    Some(compute(head, &commits, now_secs))
}

fn compute(head: String, commits: &[(u64, Vec<String>)], now_secs: u64) -> GitStats {
    let since = now_secs.saturating_sub(30 * 24 * 3600);
    let mut churn: HashMap<String, u16> = HashMap::new();
    let mut count: HashMap<&str, u32> = HashMap::new();
    let mut pairs: HashMap<(&str, &str), u32> = HashMap::new();
    for (ts, files) in commits {
        if *ts >= since {
            for f in files {
                let c = churn.entry(f.clone()).or_insert(0);
                *c = c.saturating_add(1);
            }
        }
        if files.len() > MAX_FILES_PER_COMMIT {
            continue;
        }
        let mut fs: Vec<&str> = files.iter().map(String::as_str).collect();
        fs.sort_unstable();
        fs.dedup();
        for (i, a) in fs.iter().enumerate() {
            *count.entry(a).or_default() += 1;
            for b in &fs[i + 1..] {
                *pairs.entry((a, b)).or_default() += 1;
            }
        }
    }
    let mut cochange: Vec<(String, String, f32, u32)> = pairs
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .filter_map(|((a, b), n)| {
            let min = count
                .get(a)
                .copied()
                .unwrap_or(1)
                .min(count.get(b).copied().unwrap_or(1))
                .max(1);
            #[allow(clippy::cast_precision_loss)]
            let ratio = n as f32 / min as f32;
            (ratio >= 0.5).then(|| (a.to_owned(), b.to_owned(), ratio.min(1.0), n))
        })
        .collect();
    cochange.sort_by(|x, y| {
        y.2.total_cmp(&x.2)
            .then(y.3.cmp(&x.3))
            .then_with(|| x.0.cmp(&y.0))
            .then_with(|| x.1.cmp(&y.1))
    });
    cochange.truncate(MAX_COCHANGE_EDGES);
    GitStats {
        head,
        churn,
        cochange: cochange.into_iter().map(|(a, b, r, _)| (a, b, r)).collect(),
    }
}

/// Reject anything git could read as an option or that is not a plain revision expression.
pub fn validate_rev(rev: &str) -> Result<()> {
    let ok = !rev.is_empty()
        && rev.len() <= 200
        && !rev.starts_with('-')
        && rev
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "~^/._@{}-".contains(c));
    if ok {
        Ok(())
    } else {
        Err(Error::Git(format!("invalid revision: {rev:?}")))
    }
}

/// `(path relative to root, size, blob id)` of every file under `root` at `rev`.
pub fn tree(root: &Path, rev: &str) -> Result<Vec<(String, u64, String)>> {
    validate_rev(rev)?;
    let out = git(root)
        .args(["ls-tree", "-r", "-l", "-z", rev, "--", "."])
        .output()?;
    if !out.status.success() {
        return Err(Error::Git(format!("git ls-tree {rev} failed")));
    }
    let mut files = Vec::new();
    for rec in out.stdout.split(|b| *b == 0) {
        let rec = String::from_utf8_lossy(rec);
        // `<mode> <type> <object> <size>\t<path>`
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        let parts: Vec<&str> = meta.split_whitespace().collect();
        if let [_, "blob", id, size] = parts.as_slice() {
            files.push((
                path.to_owned(),
                size.parse().unwrap_or(u64::MAX),
                (*id).to_owned(),
            ));
        }
    }
    Ok(files)
}

/// Contents of the given blobs through one `git cat-file --batch` process.
pub fn blobs(root: &Path, ids: &[String]) -> Result<Vec<Vec<u8>>> {
    let mut child = git(root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Git("no stdin".into()))?;
    let mut input = String::with_capacity(ids.len() * 41);
    for i in ids {
        input.push_str(i);
        input.push('\n');
    }
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Git("no stdout".into()))?;
    let mut r = BufReader::new(stdout);
    let mut out = Vec::with_capacity(ids.len());
    for _ in ids {
        let mut header = String::new();
        r.read_line(&mut header)?;
        let size: usize = header
            .split_whitespace()
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut buf = vec![0; size];
        r.read_exact(&mut buf)?;
        let mut nl = [0u8; 1];
        if size > 0 || header.contains(" blob ") {
            r.read_exact(&mut nl)?;
        }
        out.push(buf);
    }
    let _ = writer.join();
    let _ = child.wait();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cochange_and_churn() {
        let now = 100 * 24 * 3600;
        let c = |ts: u64, f: &[&str]| (ts, f.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        let commits = vec![
            c(now, &["a.rs", "b.rs"]),
            c(now - 3600, &["a.rs", "b.rs", "c.rs"]),
            c(10, &["a.rs"]),
            c(10, &["c.rs"]),
        ];
        let s = compute("h".into(), &commits, now);
        assert_eq!(s.churn.get("a.rs"), Some(&2));
        assert_eq!(s.churn.get("c.rs"), Some(&1));
        assert_eq!(
            s.cochange,
            vec![("a.rs".to_owned(), "b.rs".to_owned(), 1.0)]
        );
    }

    #[test]
    fn rev_validation() {
        assert!(validate_rev("HEAD~5").is_ok());
        assert!(validate_rev("v1.2.0").is_ok());
        assert!(validate_rev("main@{1}").is_ok());
        assert!(validate_rev("--output=x").is_err());
        assert!(validate_rev("a b").is_err());
        assert!(validate_rev("").is_err());
    }
}
