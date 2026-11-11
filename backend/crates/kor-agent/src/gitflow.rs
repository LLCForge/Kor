//! `kor commit` and `kor changelog` (§15.5): Conventional Commits messages from the staged diff,
//! split proposals, the `prepare-commit-msg` hook, and Keep a Changelog sections from history.
//!
//! git runs through its CLI. The commit scope comes from the changed package or directory; the
//! spec's graph community name and `graph_impact` check for breaking changes arrive with the
//! graph (Faz 3). Breaking changes are detected from commit markers (`!`, `BREAKING CHANGE:`).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock};

use kor_core::{Message, Role};
use kor_platform::NoWindow as _;
use kor_providers::{CancellationToken, ChatRequest, Provider};
use regex::Regex;
use serde::Serialize;

/// Diff characters sent to the model; larger diffs are cut (the file list is always complete).
const MAX_DIFF: usize = 48_000;
const TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];
const HOOK_MARK: &str = "# kor prepare-commit-msg hook";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    GitMissing,
    NotRepo,
    NothingStaged,
    Git(String),
    Model(String),
    BadReply(String),
    HookExists(PathBuf),
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitMissing => f.write_str("git was not found"),
            Self::NotRepo => f.write_str("not a git repository"),
            Self::NothingStaged => f.write_str("nothing is staged"),
            Self::Git(e) | Self::Model(e) | Self::BadReply(e) => f.write_str(e),
            Self::HookExists(p) => write!(f, "{} exists", p.display()),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// git
// ---------------------------------------------------------------------------------------------

fn git_cmd(root: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(kor_platform::exe("git"));
    c.args(["-c", "core.quotepath=off"])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .no_window();
    c
}

fn run_git(mut c: Command, input: Option<&str>) -> Result<String, FlowError> {
    if input.is_some() {
        c.stdin(Stdio::piped());
    }
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => FlowError::GitMissing,
        _ => FlowError::Git(e.to_string()),
    })?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| FlowError::Git(e.to_string()))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| FlowError::Git(e.to_string()))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if err.contains("not a git repository") {
        Err(FlowError::NotRepo)
    } else {
        Err(FlowError::Git(err))
    }
}

pub fn git(root: &Path, args: &[&str]) -> Result<String, FlowError> {
    run_git(git_cmd(root, args), None)
}

/// Top level of the repository containing `dir`.
pub fn repo_root(dir: &Path) -> Result<PathBuf, FlowError> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(top.trim()))
}

/// Staged file names and the staged diff (cut at [`MAX_DIFF`] characters).
pub fn staged(root: &Path) -> Result<(Vec<String>, String), FlowError> {
    let files: Vec<String> = git(root, &["diff", "--cached", "--name-only"])?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    if files.is_empty() {
        return Err(FlowError::NothingStaged);
    }
    let mut diff = git(root, &["diff", "--cached", "--no-color", "--no-ext-diff"])?;
    if let Some((i, _)) = diff.char_indices().nth(MAX_DIFF) {
        diff.truncate(i);
        diff.push_str("\n[diff truncated]\n");
    }
    Ok((files, diff))
}

/// Commit the staged changes with `message`; returns git's summary line.
pub fn commit(root: &Path, message: &str) -> Result<String, FlowError> {
    let out = run_git(git_cmd(root, &["commit", "-F", "-"]), Some(message))?;
    Ok(out.lines().next().unwrap_or_default().to_owned())
}

// ---------------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------------

/// Scope from the changed package or directory, when every file shares one.
pub fn scope_for(files: &[String]) -> Option<String> {
    const CONTAINERS: &[&str] = &[
        "crates", "packages", "apps", "libs", "modules", "services", "plugins",
    ];
    let mut scopes = BTreeSet::new();
    for f in files {
        let parts: Vec<&str> = f.split('/').collect();
        let pick = parts
            .iter()
            .position(|p| CONTAINERS.contains(p))
            .filter(|&i| i + 2 < parts.len())
            .map(|i| parts[i + 1])
            .or_else(|| (parts.len() > 1).then_some(parts[0]));
        scopes.insert(pick?.to_owned());
    }
    (scopes.len() == 1)
        .then(|| scopes.into_iter().next())
        .flatten()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitMsg {
    pub kind: String,
    pub scope: Option<String>,
    pub breaking: bool,
    pub subject: String,
    pub body: String,
}

impl CommitMsg {
    pub fn header(&self) -> String {
        let scope = self
            .scope
            .as_deref()
            .map(|s| format!("({s})"))
            .unwrap_or_default();
        let bang = if self.breaking && !has_breaking_footer(&self.body) {
            "!"
        } else {
            ""
        };
        format!("{}{scope}{bang}: {}", self.kind, self.subject)
    }

    pub fn render(&self) -> String {
        if self.body.is_empty() {
            format!("{}\n", self.header())
        } else {
            format!("{}\n\n{}\n", self.header(), self.body)
        }
    }
}

static HEADER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^([a-zA-Z]+)(?:\(([^()]*)\))?(!)?:\s*(.+)$").ok());

fn has_breaking_footer(body: &str) -> bool {
    body.lines()
        .any(|l| l.starts_with("BREAKING CHANGE:") || l.starts_with("BREAKING-CHANGE:"))
}

/// `type(scope)!: subject` → parts; `None` when the line is not a Conventional Commits header.
pub fn parse_header(line: &str) -> Option<(String, Option<String>, bool, String)> {
    let c = HEADER.as_ref()?.captures(line.trim())?;
    let kind = c.get(1)?.as_str().to_ascii_lowercase();
    let scope = c
        .get(2)
        .map(|m| m.as_str().trim().to_owned())
        .filter(|s| !s.is_empty());
    Some((
        kind,
        scope,
        c.get(3).is_some(),
        c.get(4)?.as_str().trim().to_owned(),
    ))
}

fn strip_fences(reply: &str) -> String {
    reply
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// A commit message from the model's reply. A header that is not Conventional Commits becomes
/// the subject of a `chore` with the suggested scope.
pub fn parse_commit(reply: &str, scope: Option<&str>) -> Result<CommitMsg, FlowError> {
    let text = strip_fences(reply);
    let mut lines = text.lines();
    let first = lines
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| FlowError::BadReply("empty reply".into()))?;
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_owned();
    let (kind, sc, bang, subject) = match parse_header(first) {
        Some((k, s, b, subj)) if TYPES.contains(&k.as_str()) => (k, s, b, subj),
        _ => (
            "chore".to_owned(),
            None,
            false,
            first.trim().trim_matches('"').to_owned(),
        ),
    };
    let subject = subject.trim_end_matches('.').to_owned();
    Ok(CommitMsg {
        breaking: bang || has_breaking_footer(&body),
        kind,
        scope: sc.or_else(|| scope.map(str::to_owned)),
        subject,
        body,
    })
}

fn rules(scope: Option<&str>) -> String {
    let scope = scope.map_or_else(
        || "No single scope fits; use a short scope only if obvious, else none.".to_owned(),
        |s| format!("Suggested scope: `{s}` (keep it unless clearly wrong)."),
    );
    format!(
        "Conventional Commits rules: header `type(scope)!: subject` with type one of {}; subject \
         in English, imperative, at most 72 characters, no final period; `!` only for breaking \
         changes. Then a blank line and a body (wrapped at 72) explaining WHY the change was \
         made, not a list of files. For breaking changes add a `BREAKING CHANGE: <what breaks>` \
         footer. {scope}",
        TYPES.join(", ")
    )
}

pub fn commit_prompt(files: &[String], diff: &str, scope: Option<&str>) -> String {
    format!(
        "Write the commit message for these staged changes.\n{}\nReply with the message only, no \
         code fences.\n\nFiles:\n{}\n\nDiff:\n{diff}",
        rules(scope),
        files.join("\n")
    )
}

pub fn split_prompt(files: &[String], diff: &str, scope: Option<&str>) -> String {
    format!(
        "Split these staged changes into the smallest set of logical commits (one when they belong \
         together). Every file goes into exactly one commit.\n{}\nReply with JSON only: \
         [{{\"files\": [\"path\", ...], \"message\": \"type(scope): subject\\n\\nbody\"}}, ...]\
         \n\nFiles:\n{}\n\nDiff:\n{diff}",
        rules(scope),
        files.join("\n")
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SplitPart {
    pub files: Vec<String>,
    pub message: CommitMsg,
}

/// Split proposal from the model's JSON. Unknown files are dropped, duplicates keep their first
/// commit, and staged files the model forgot join the last commit.
pub fn parse_split(reply: &str, staged: &[String]) -> Result<Vec<SplitPart>, FlowError> {
    #[derive(serde::Deserialize)]
    struct Raw {
        files: Vec<String>,
        message: String,
    }
    let text = strip_fences(reply);
    let (Some(a), Some(b)) = (text.find('['), text.rfind(']')) else {
        return Err(FlowError::BadReply("no JSON array".into()));
    };
    let raw: Vec<Raw> = serde_json::from_str(text.get(a..=b).unwrap_or_default())
        .map_err(|e| FlowError::BadReply(e.to_string()))?;
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    for r in raw {
        let files: Vec<String> = r
            .files
            .into_iter()
            .filter(|f| staged.contains(f) && seen.insert(f.clone()))
            .collect();
        if files.is_empty() {
            continue;
        }
        let message = parse_commit(&r.message, scope_for(&files).as_deref())?;
        parts.push(SplitPart { files, message });
    }
    let rest: Vec<String> = staged
        .iter()
        .filter(|f| !seen.contains(*f))
        .cloned()
        .collect();
    match parts.last_mut() {
        Some(last) => last.files.extend(rest),
        None => {
            return Err(FlowError::BadReply(
                "no usable commit in the proposal".into(),
            ));
        }
    }
    Ok(parts)
}

/// One request, text only.
pub async fn ask(
    provider: &Arc<dyn Provider>,
    model: &str,
    prompt: String,
    max_tokens: u32,
) -> Result<String, FlowError> {
    let req = ChatRequest {
        model: model.to_owned(),
        system: Some(
            "You are a precise assistant for software developers. Follow the format asked for."
                .into(),
        ),
        messages: vec![Message::text(Role::User, prompt)],
        max_tokens: Some(max_tokens),
        ..ChatRequest::default()
    };
    let stream = provider
        .complete(req, CancellationToken::new())
        .await
        .map_err(|e| FlowError::Model(e.message))?;
    let text = crate::subagent::collect_text(stream)
        .await
        .map_err(|e| FlowError::Model(e.message))?;
    if text.trim().is_empty() {
        Err(FlowError::BadReply("empty reply".into()))
    } else {
        Ok(text)
    }
}

/// What `kor commit` proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Proposal {
    pub files: Vec<String>,
    pub scope: Option<String>,
    pub message: CommitMsg,
    /// Only with `split`.
    pub parts: Vec<SplitPart>,
}

/// Read the staged changes under `dir` and ask the model for a message (or a split).
pub async fn propose(
    provider: &Arc<dyn Provider>,
    model: &str,
    dir: &Path,
    split: bool,
) -> Result<Proposal, FlowError> {
    let root = repo_root(dir)?;
    let (files, diff) = staged(&root)?;
    // Faz 3: the changed files' package or graph community, else the directory rule.
    let scope = crate::graph::commit_scope(&root, &files).or_else(|| scope_for(&files));
    let reply = ask(
        provider,
        model,
        commit_prompt(&files, &diff, scope.as_deref()),
        1024,
    )
    .await?;
    let message = parse_commit(&reply, scope.as_deref())?;
    let parts = if split {
        let reply = ask(
            provider,
            model,
            split_prompt(&files, &diff, scope.as_deref()),
            4096,
        )
        .await?;
        parse_split(&reply, &files)?
    } else {
        Vec::new()
    };
    Ok(Proposal {
        files,
        scope,
        message,
        parts,
    })
}

// ---------------------------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------------------------

/// The `prepare-commit-msg` hook: only for a plain `git commit` (no `-m`, `-F`, merge, amend),
/// keeps git's template below Kor's message, and never blocks the commit.
pub fn hook_script() -> String {
    format!(
        "#!/bin/sh\n{HOOK_MARK} (kor commit --install-hook)\n[ -n \"$2\" ] && exit 0\n\
         if kor commit --message-only > \"$1.kor\" 2>/dev/null && [ -s \"$1.kor\" ]; then\n  \
         cat \"$1\" >> \"$1.kor\" && mv \"$1.kor\" \"$1\"\nfi\nrm -f \"$1.kor\"\nexit 0\n"
    )
}

/// Install the hook where git looks for it (respects `core.hooksPath`). A hook that is not
/// Kor's is never overwritten.
pub fn install_hook(dir: &Path) -> Result<PathBuf, FlowError> {
    let root = repo_root(dir)?;
    let rel = git(
        &root,
        &["rev-parse", "--git-path", "hooks/prepare-commit-msg"],
    )?;
    let path = root.join(rel.trim());
    if let Ok(old) = std::fs::read_to_string(&path)
        && !old.contains(HOOK_MARK)
    {
        return Err(FlowError::HookExists(path));
    }
    let io = |e: std::io::Error| FlowError::Git(format!("{}: {e}", path.display()));
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(io)?;
    }
    std::fs::write(&path, hook_script()).map_err(io)?;
    kor_platform::make_executable(&path).map_err(io)?;
    Ok(path)
}

// ---------------------------------------------------------------------------------------------
// Changelog
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub hash: String,
    pub kind: Option<String>,
    pub scope: Option<String>,
    pub subject: String,
    pub breaking: Option<String>,
}

/// The most recent tag reachable from HEAD.
pub fn latest_tag(root: &Path) -> Option<String> {
    git(root, &["describe", "--tags", "--abbrev=0"])
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

/// Commits after `since` (all of HEAD when `None`), newest first, merges skipped.
pub fn log(root: &Path, since: Option<&str>) -> Result<Vec<Entry>, FlowError> {
    if since.is_some_and(|s| s.starts_with('-')) {
        return Err(FlowError::Git("invalid revision".into()));
    }
    let range = since.map_or_else(|| "HEAD".to_owned(), |s| format!("{s}..HEAD"));
    let out = git(
        root,
        &["log", "--no-merges", "--format=%h%x1f%B%x1e", &range, "--"],
    )?;
    Ok(out
        .split('\x1e')
        .filter_map(|rec| {
            let (hash, msg) = rec.trim().split_once('\x1f')?;
            let mut lines = msg.trim().lines();
            let first = lines.next()?.trim();
            let body: Vec<&str> = lines.collect();
            let footer = body.iter().find_map(|l| {
                l.strip_prefix("BREAKING CHANGE:")
                    .or_else(|| l.strip_prefix("BREAKING-CHANGE:"))
                    .map(|n| n.trim().to_owned())
            });
            Some(match parse_header(first) {
                Some((kind, scope, bang, subject)) => Entry {
                    hash: hash.to_owned(),
                    breaking: footer.or_else(|| bang.then(String::new)),
                    kind: Some(kind),
                    scope,
                    subject,
                },
                None => Entry {
                    hash: hash.to_owned(),
                    kind: None,
                    scope: None,
                    subject: first.to_owned(),
                    breaking: footer,
                },
            })
        })
        .collect())
}

/// Keep a Changelog group for a commit; `None` for changes users do not see (docs, tests, CI…).
fn group(e: &Entry) -> Option<&'static str> {
    let subj = e.subject.to_ascii_lowercase();
    if subj.starts_with("deprecate") {
        return Some("Deprecated");
    }
    if ["remove ", "drop ", "delete "]
        .iter()
        .any(|p| subj.starts_with(p))
    {
        return Some("Removed");
    }
    let security = e.scope.as_deref() == Some("security") || subj.starts_with("security");
    match e.kind.as_deref() {
        Some("feat") => Some("Added"),
        Some("fix") if security => Some("Security"),
        Some("fix") => Some("Fixed"),
        Some("perf" | "refactor" | "revert") | None => Some("Changed"),
        _ => None,
    }
}

/// One `## [title]` section in Keep a Changelog form (also usable as release notes).
pub fn section(entries: &[Entry], title: &str) -> String {
    use std::fmt::Write as _;
    let line = |e: &Entry| {
        let scope = e
            .scope
            .as_deref()
            .map(|s| format!("**{s}:** "))
            .unwrap_or_default();
        format!("- {scope}{} ({})\n", e.subject, e.hash)
    };
    let mut s = format!("## [{title}]\n");
    let breaking: Vec<&Entry> = entries.iter().filter(|e| e.breaking.is_some()).collect();
    if !breaking.is_empty() {
        s.push_str("\n### Breaking changes\n\n");
        for e in breaking {
            let note = e.breaking.as_deref().filter(|n| !n.is_empty());
            match note {
                Some(n) => {
                    let _ = writeln!(s, "{} — {n}", line(e).trim_end());
                }
                None => s.push_str(&line(e)),
            }
        }
    }
    for g in [
        "Added",
        "Changed",
        "Deprecated",
        "Removed",
        "Fixed",
        "Security",
    ] {
        let items: Vec<String> = entries
            .iter()
            .filter(|e| e.breaking.is_none() && group(e) == Some(g))
            .map(line)
            .collect();
        if !items.is_empty() {
            let _ = write!(s, "\n### {g}\n\n{}", items.concat());
        }
    }
    s
}

const CHANGELOG_HEAD: &str = "# Changelog\n\nAll notable changes to this project will be \
    documented in this file.\n\nThe format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),\n\
    and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n";

/// `CHANGELOG.md` with `section` as its `[Unreleased]` part (replaced if present, else inserted
/// above the newest release).
pub fn merge_changelog(existing: Option<&str>, section: &str) -> String {
    let doc = existing
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(CHANGELOG_HEAD)
        .replace("\r\n", "\n");
    let section = format!("{}\n", section.trim_end());
    let heading = |i: usize, line: &str| (line.starts_with("## ")).then_some(i);
    let mut offsets = Vec::new();
    let mut pos = 0;
    for line in doc.split_inclusive('\n') {
        offsets.push((pos, line));
        pos += line.len();
    }
    let unreleased = offsets
        .iter()
        .position(|(_, l)| l.to_ascii_lowercase().starts_with("## [unreleased]"));
    match unreleased {
        Some(u) => {
            let start = offsets[u].0;
            let end = offsets
                .iter()
                .skip(u + 1)
                .find_map(|(i, l)| heading(*i, l))
                .unwrap_or(doc.len());
            let tail = &doc[end..];
            let sep = if tail.is_empty() { "" } else { "\n" };
            format!("{}{section}{sep}{tail}", &doc[..start])
        }
        None => match offsets.iter().find_map(|(i, l)| heading(*i, l)) {
            Some(i) => format!("{}{section}\n{}", &doc[..i], &doc[i..]),
            None => format!("{}\n\n{section}", doc.trim_end_matches('\n')),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use kor_core::{ProviderError, StreamEvent};
    use kor_providers::{EventStream, ModelInfo, ProviderManifest, Registry};
    use std::sync::Mutex;

    #[test]
    fn scope_comes_from_the_shared_package_or_dir() {
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            scope_for(&v(&[
                "backend/crates/kor-agent/src/a.rs",
                "backend/crates/kor-agent/Cargo.toml"
            ])),
            Some("kor-agent".into())
        );
        assert_eq!(
            scope_for(&v(&["docs/a.md", "docs/b/c.md"])),
            Some("docs".into())
        );
        assert_eq!(scope_for(&v(&["docs/a.md", "src/b.rs"])), None);
        assert_eq!(scope_for(&v(&["README.md"])), None);
        assert_eq!(scope_for(&v(&["crates/x"])), Some("crates".into()));
    }

    #[test]
    fn commit_replies_parse_and_render() {
        let m = parse_commit(
            "```\nfeat(cli)!: add kor commit.\n\nUsers asked for it.\n```",
            Some("x"),
        )
        .unwrap();
        assert_eq!(m.kind, "feat");
        assert_eq!(m.scope.as_deref(), Some("cli"));
        assert!(m.breaking);
        assert_eq!(
            m.render(),
            "feat(cli)!: add kor commit\n\nUsers asked for it.\n"
        );
        let m = parse_commit("Fix the thing", Some("core")).unwrap();
        assert_eq!(m.header(), "chore(core): Fix the thing");
        let m = parse_commit("fix: x\n\nBREAKING CHANGE: y", None).unwrap();
        assert!(m.breaking);
        assert_eq!(m.header(), "fix: x");
        assert!(parse_commit("  \n", None).is_err());
        let p = commit_prompt(&["a.rs".into()], "diff", Some("core"));
        assert!(p.contains("`core`") && p.contains("WHY") && p.contains("a.rs"));
    }

    #[test]
    fn split_replies_cover_every_staged_file_once() {
        let staged = vec!["a.rs".to_owned(), "b.rs".to_owned(), "c.rs".to_owned()];
        let reply = r#"Here: [{"files":["a.rs","zzz.rs"],"message":"feat: a"},
            {"files":["a.rs","b.rs"],"message":"fix(b): b\n\nwhy"},{"files":[],"message":"x"}]"#;
        let parts = parse_split(reply, &staged).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].files, vec!["a.rs"]);
        assert_eq!(parts[1].files, vec!["b.rs", "c.rs"]);
        assert_eq!(parts[1].message.body, "why");
        assert!(parse_split("no json", &staged).is_err());
        assert!(parse_split(r#"[{"files":["q"],"message":"m"}]"#, &staged).is_err());
    }

    struct Mock {
        manifest: ProviderManifest,
        replies: Mutex<Vec<String>>,
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Provider for Mock {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            req: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, ProviderError> {
            let prompt = req.messages[0].plain_text();
            self.seen.lock().unwrap().push(prompt);
            let text = self.replies.lock().unwrap().remove(0);
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                StreamEvent::TextDelta { text },
            )])))
        }
    }

    fn git_ok() -> bool {
        Command::new(kor_platform::exe("git"))
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for a in [
            &["init", "-q"][..],
            &["config", "user.name", "kor"],
            &["config", "user.email", "kor@example.com"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.hooksPath", ".git/hooks"],
        ] {
            git(d.path(), a).unwrap();
        }
        d
    }

    fn commit_file(d: &Path, file: &str, msg: &str) {
        std::fs::write(d.join(file), msg).unwrap();
        git(d, &["add", file]).unwrap();
        commit(d, msg).unwrap();
    }

    #[tokio::test]
    async fn propose_and_commit_with_a_mock_provider() {
        if !git_ok() {
            eprintln!("git missing; skipped");
            return;
        }
        let d = repo();
        let mock = Arc::new(Mock {
            manifest: Registry::builtin().get("local").unwrap().clone(),
            replies: Mutex::new(vec![
                "feat(docs): add guide\n\nNew users need it.".into(),
                r#"[{"files":["docs/g.md"],"message":"docs: add guide"}]"#.into(),
            ]),
            seen: Mutex::new(Vec::new()),
        });
        let p: Arc<dyn Provider> = mock.clone();
        assert_eq!(
            propose(&p, "m", d.path(), false).await,
            Err(FlowError::NothingStaged)
        );
        std::fs::create_dir_all(d.path().join("docs")).unwrap();
        std::fs::write(d.path().join("docs/g.md"), "hello\n").unwrap();
        git(d.path(), &["add", "."]).unwrap();
        let prop = propose(&p, "m", d.path(), true).await.unwrap();
        assert_eq!(prop.scope.as_deref(), Some("docs"));
        assert_eq!(prop.message.header(), "feat(docs): add guide");
        assert_eq!(prop.parts.len(), 1);
        assert!(mock.seen.lock().unwrap()[0].contains("+hello"));
        let summary = commit(d.path(), &prop.message.render()).unwrap();
        assert!(summary.contains("feat(docs): add guide"), "{summary}");

        let not_repo = tempfile::tempdir().unwrap();
        assert_eq!(repo_root(not_repo.path()), Err(FlowError::NotRepo));

        let hook = install_hook(d.path()).unwrap();
        assert!(std::fs::read_to_string(&hook).unwrap().contains(HOOK_MARK));
        assert_eq!(install_hook(d.path()).unwrap(), hook);
        std::fs::write(&hook, "#!/bin/sh\necho mine\n").unwrap();
        assert!(matches!(
            install_hook(d.path()),
            Err(FlowError::HookExists(_))
        ));
    }

    #[test]
    fn changelog_groups_commits_since_a_tag() {
        if !git_ok() {
            eprintln!("git missing; skipped");
            return;
        }
        let d = repo();
        let p = d.path();
        commit_file(p, "a", "feat: first release");
        git(p, &["tag", "v0.1.0"]).unwrap();
        commit_file(p, "b", "feat(cli): add changelog");
        commit_file(p, "c", "fix: crash on empty repo");
        commit_file(p, "d", "docs: readme");
        commit_file(p, "e", "refactor(core)!: rename config keys");
        commit_file(p, "f", "Update deps\n\nBREAKING CHANGE: needs Rust 1.85");
        commit_file(p, "g", "fix(security): escape paths");
        assert_eq!(latest_tag(p).as_deref(), Some("v0.1.0"));
        let entries = log(p, Some("v0.1.0")).unwrap();
        assert_eq!(entries.len(), 6);
        let s = section(&entries, "Unreleased");
        assert!(s.starts_with("## [Unreleased]\n\n### Breaking changes\n"));
        assert!(s.contains("- **core:** rename config keys ("));
        assert!(s.contains("Update deps (") && s.contains(" — needs Rust 1.85"));
        assert!(s.contains("### Added\n\n- **cli:** add changelog ("));
        assert!(s.contains("### Fixed\n\n- crash on empty repo ("));
        assert!(s.contains("### Security\n\n- **security:** escape paths ("));
        assert!(!s.contains("readme") && !s.contains("first release"));
        assert_eq!(log(p, None).unwrap().len(), 7);
        assert!(log(p, Some("--output=x")).is_err());
    }

    #[test]
    fn changelog_file_is_created_and_updated() {
        let new = merge_changelog(None, "## [Unreleased]\n\n### Added\n\n- a (1)\n");
        assert!(new.starts_with("# Changelog\n"));
        assert!(new.ends_with("\n## [Unreleased]\n\n### Added\n\n- a (1)\n"));
        let old = "# Changelog\n\n## [Unreleased]\n\n- old\n\n## [0.1.0] - 2026-01-01\n\n- x\n";
        let up = merge_changelog(Some(old), "## [Unreleased]\n\n- new\n");
        assert_eq!(
            up,
            "# Changelog\n\n## [Unreleased]\n\n- new\n\n## [0.1.0] - 2026-01-01\n\n- x\n"
        );
        let no_unrel = "# Changelog\n\n## [0.1.0]\n\n- x\n";
        let up = merge_changelog(Some(no_unrel), "## [Unreleased]\n\n- new");
        assert_eq!(
            up,
            "# Changelog\n\n## [Unreleased]\n\n- new\n\n## [0.1.0]\n\n- x\n"
        );
        assert!(
            hook_script().starts_with("#!/bin/sh\n") && hook_script().contains("--message-only")
        );
    }
}
