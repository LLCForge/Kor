//! `kor pr respond`: draft answers to review comments and the code fixes they ask for. Fixes are
//! proposals (whole new file contents) that the caller reviews file by file before anything is
//! written (ADR 0030); paths outside the repository are dropped.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use kor_agent::Proposal;
use kor_agent::gitflow;
use kor_providers::Provider;
use serde::{Deserialize, Serialize};

use super::{Comment, ForgeError, PrInfo};

/// Files larger than this are not given to the model (and cannot be edited by a reply).
const MAX_FILE: u64 = 64 * 1024;
const MAX_FILES: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub body: String,
}

/// Drafted answers and file proposals.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Response {
    pub replies: Vec<Reply>,
    pub edits: Vec<Proposal>,
}

/// `p` inside `root` (relative, no `..`, no absolute or drive prefix).
pub fn safe_path(root: &Path, p: &str) -> Option<PathBuf> {
    let rel = Path::new(p.trim());
    let ok = !p.trim().is_empty()
        && rel
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    ok.then(|| root.join(rel))
}

fn read_small(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    (m.is_file() && m.len() <= MAX_FILE)
        .then(|| std::fs::read_to_string(path).ok())
        .flatten()
}

/// Current contents of the files the comments point at (small text files only).
pub fn context_files(root: &Path, comments: &[Comment]) -> Vec<(String, String)> {
    let paths: BTreeSet<&str> = comments.iter().filter_map(|c| c.path.as_deref()).collect();
    paths
        .into_iter()
        .filter_map(|p| Some((p.to_owned(), read_small(&safe_path(root, p)?)?)))
        .take(MAX_FILES)
        .collect()
}

pub fn respond_prompt(pr: &PrInfo, comments: &[Comment], files: &[(String, String)]) -> String {
    let mut s = format!(
        "You are the author of pull request #{} \"{}\" ({} → {}). Answer each review comment \
         below. When a comment asks for a code change you agree with, make it. Reply with JSON \
         only: {{\"replies\": [{{\"id\": \"<comment id>\", \"body\": \"<answer, Markdown, \
         concise; say what you changed or why not>\"}}], \"edits\": [{{\"path\": \"<file>\", \
         \"content\": \"<the complete new file content>\"}}]}}. Only edit files shown below; \
         give full contents, never fragments.\n\nComments:\n",
        pr.number, pr.title, pr.head, pr.base
    );
    for c in comments {
        let at = match (&c.path, c.line) {
            (Some(p), Some(l)) => format!(" on {p}:{l}"),
            (Some(p), None) => format!(" on {p}"),
            _ => String::new(),
        };
        let _ = writeln!(
            s,
            "- id {} by {}{at}: {}",
            c.id,
            c.author,
            c.body.replace('\n', " ")
        );
    }
    for (p, text) in files {
        let _ = write!(
            s,
            "\nFile {p}:\n```\n{}\n```\n",
            kor_tools::redact_secrets(text)
        );
    }
    s
}

/// Replies to known comments and proposals for safe, changed files.
///
/// # Errors
/// When the reply holds no JSON object.
pub fn parse_response(root: &Path, reply: &str, comments: &[Comment]) -> Result<Response, String> {
    #[derive(Deserialize)]
    struct RawEdit {
        path: String,
        content: String,
    }
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        replies: Vec<Reply>,
        #[serde(default)]
        edits: Vec<RawEdit>,
    }
    let (Some(a), Some(b)) = (reply.find('{'), reply.rfind('}')) else {
        return Err("no JSON object".into());
    };
    let raw: Raw =
        serde_json::from_str(reply.get(a..=b).unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut seen = BTreeSet::new();
    let replies = raw
        .replies
        .into_iter()
        .filter(|r| {
            !r.body.trim().is_empty()
                && comments.iter().any(|c| c.id == r.id)
                && seen.insert(r.id.clone())
        })
        .collect();
    let mut paths = BTreeSet::new();
    let edits =
        raw.edits
            .into_iter()
            .filter_map(|e| {
                let full = safe_path(root, &e.path)?;
                let before = match std::fs::metadata(&full) {
                    Ok(_) => Some(read_small(&full)?),
                    Err(_) => None,
                };
                let path = e.path.trim().replace('\\', "/");
                (before.as_deref() != Some(e.content.as_str()) && paths.insert(path.clone()))
                    .then_some(Proposal {
                        path,
                        before,
                        after: Some(e.content),
                    })
            })
            .collect();
    Ok(Response { replies, edits })
}

/// Ask the model for answers and fixes.
///
/// # Errors
/// `Parse` when the model fails or its reply cannot be used.
pub async fn draft(
    provider: &Arc<dyn Provider>,
    model: &str,
    root: &Path,
    pr: &PrInfo,
    comments: &[Comment],
) -> Result<Response, ForgeError> {
    let files = context_files(root, comments);
    let reply = gitflow::ask(
        provider,
        model,
        respond_prompt(pr, comments, &files),
        16_384,
    )
    .await
    .map_err(|e| ForgeError::Parse(e.to_string()))?;
    parse_response(root, &reply, comments).map_err(ForgeError::Parse)
}

/// Write accepted proposals (`after: None` is never produced here).
///
/// # Errors
/// The first write that fails.
pub fn write(root: &Path, accepted: &[(String, String)]) -> Result<(), ForgeError> {
    for (p, content) in accepted {
        let full =
            safe_path(root, p).ok_or_else(|| ForgeError::Git(format!("unsafe path: {p}")))?;
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ForgeError::Git(format!("{p}: {e}")))?;
        }
        std::fs::write(&full, content).map_err(|e| ForgeError::Git(format!("{p}: {e}")))?;
    }
    Ok(())
}

/// Commit exactly `files` with `message` (other staged changes stay out of the commit).
///
/// # Errors
/// git failures.
pub fn commit(root: &Path, files: &[String], message: &str) -> Result<String, ForgeError> {
    let mut add = vec!["add", "--"];
    add.extend(files.iter().map(String::as_str));
    gitflow::git(root, &add)?;
    let mut c = vec!["commit", "-q", "-m", message, "--"];
    c.extend(files.iter().map(String::as_str));
    gitflow::git(root, &c)?;
    Ok(gitflow::git(root, &["log", "-1", "--format=%h %s"])?
        .trim()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, path: Option<&str>) -> Comment {
        Comment {
            id: id.into(),
            author: "rev".into(),
            body: "please rename".into(),
            path: path.map(str::to_owned),
            line: Some(1),
        }
    }

    #[test]
    fn paths_stay_inside_the_repository() {
        let r = Path::new("/repo");
        assert!(safe_path(r, "src/a.rs").is_some());
        assert!(safe_path(r, "./a.rs").is_some());
        assert!(safe_path(r, "../x").is_none());
        assert!(safe_path(r, "/etc/passwd").is_none());
        assert!(safe_path(r, "").is_none());
    }

    #[test]
    fn response_keeps_known_replies_and_real_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "old\n").unwrap();
        std::fs::write(dir.path().join("same.rs"), "x\n").unwrap();
        let cs = [c("1", Some("a.rs")), c("2", None)];
        let reply = r#"Sure: {"replies":[{"id":"1","body":"renamed"},{"id":"9","body":"?"},{"id":"1","body":"dup"}],
            "edits":[{"path":"a.rs","content":"new\n"},{"path":"same.rs","content":"x\n"},{"path":"../evil","content":"x"},{"path":"new.rs","content":"n"}]}"#;
        let r = parse_response(dir.path(), reply, &cs).unwrap();
        assert_eq!(
            r.replies,
            vec![Reply {
                id: "1".into(),
                body: "renamed".into()
            }]
        );
        let paths: Vec<&str> = r.edits.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.rs", "new.rs"]);
        assert_eq!(r.edits[0].before.as_deref(), Some("old\n"));
        assert_eq!(r.edits[1].before, None);
        assert_eq!(
            context_files(dir.path(), &cs),
            vec![("a.rs".to_owned(), "old\n".to_owned())]
        );
        assert!(parse_response(dir.path(), "no", &cs).is_err());
    }
}
