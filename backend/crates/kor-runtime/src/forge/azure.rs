//! Azure DevOps (Services and Server) shapes for the forge client (§28.7, ADR 0063): remote URL
//! forms, pull request JSON, threads (comments) and the bodies Kor posts. Requests go through
//! [`super::api::Forge`] like every other forge.
//!
//! Remotes: `https://dev.azure.com/{org}/{project}/_git/{repo}` (also with `{org}@`),
//! `git@ssh.dev.azure.com:v3/{org}/{project}/{repo}`, `https://{org}.visualstudio.com/[DefaultCollection/]{project}/_git/{repo}`
//! and Azure DevOps Server `https://host/tfs/{collection}/{project}/_git/{repo}` (host in
//! `[forge.hosts]` as `azure`). API: `{web}/{org}/{project}/_apis/git/repositories/{repo}` with
//! `api-version=7.1`.

use serde_json::{Value, json};
use std::fmt::Write as _;

use super::api::{Comment, InlineNote, PrInfo};
use super::{ForgeError, Kind, PrRequest, Remote};

pub const API_VERSION: &str = "7.1";

/// The repository behind an Azure DevOps remote.
///
/// # Errors
/// `NoRemote` when the path has no project and repository.
pub fn remote(scheme: &str, authority: &str, host: &str, path: &str) -> Result<Remote, ForgeError> {
    // SSH remotes: `ssh.dev.azure.com:v3/org/project/repo`, `vs-ssh.visualstudio.com:v3/…`.
    let (authority, host, path) =
        if host == "ssh.dev.azure.com" || host == "vs-ssh.visualstudio.com" {
            (
                "dev.azure.com",
                "dev.azure.com",
                path.strip_prefix("v3/").unwrap_or(path),
            )
        } else {
            (authority, host, path.strip_prefix("v3/").unwrap_or(path))
        };
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let (owner, repo) = match segs.iter().position(|s| *s == "_git") {
        Some(i) if i >= 1 && i + 1 < segs.len() => (segs[..i].join("/"), segs[i + 1].to_owned()),
        // SSH form: org/project/repo.
        None if segs.len() >= 2 => (
            segs[..segs.len() - 1].join("/"),
            segs[segs.len() - 1].to_owned(),
        ),
        Some(_) | None => return Err(ForgeError::NoRemote),
    };
    // dev.azure.com needs org/project; *.visualstudio.com and servers at least a project.
    if owner.is_empty() || (host == "dev.azure.com" && !owner.contains('/')) {
        return Err(ForgeError::NoRemote);
    }
    let web = format!("{scheme}://{authority}");
    let api = format!("{web}/{owner}/_apis/git/repositories/{repo}");
    Ok(Remote {
        kind: Kind::Azure,
        host: host.to_owned(),
        owner,
        repo,
        web,
        api,
    })
}

/// `?api-version=7.1` (or `&…`) for `path`.
pub fn versioned(path: &str) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}api-version={API_VERSION}")
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn branch(r: &str) -> String {
    r.strip_prefix("refs/heads/").unwrap_or(r).to_owned()
}

/// A pull request from Azure's JSON.
pub fn pr_from(v: &Value) -> Option<PrInfo> {
    let number = v.pointer("/pullRequestId").and_then(Value::as_u64)?;
    let web = s(v, "/repository/webUrl");
    let author = Some(s(v, "/createdBy/uniqueName"))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| s(v, "/createdBy/displayName"));
    Some(PrInfo {
        number,
        title: s(v, "/title"),
        url: if web.is_empty() {
            s(v, "/url")
        } else {
            format!("{web}/pullrequest/{number}")
        },
        head: branch(&s(v, "/sourceRefName")),
        base: branch(&s(v, "/targetRefName")),
        author,
        state: s(v, "/status"),
        draft: v.pointer("/isDraft").and_then(Value::as_bool) == Some(true),
    })
}

pub fn create_body(req: &PrRequest) -> Value {
    json!({
        "sourceRefName": format!("refs/heads/{}", req.head),
        "targetRefName": format!("refs/heads/{}", req.base),
        "title": req.title,
        "description": req.body,
        "isDraft": req.draft,
    })
}

/// Unresolved comment threads (system threads and resolved ones skipped). The comment id is the
/// thread id, which replies need.
pub fn comments(threads: &Value) -> Vec<Comment> {
    threads
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("isDeleted").and_then(Value::as_bool) != Some(true))
        .filter(|t| {
            matches!(
                t.get("status").and_then(Value::as_str),
                None | Some("active" | "pending" | "unknown")
            )
        })
        .filter_map(|t| {
            let first = t.pointer("/comments/0")?;
            if first.get("commentType").and_then(Value::as_str) == Some("system") {
                return None;
            }
            let path = s(t, "/threadContext/filePath");
            Some(Comment {
                id: t.get("id")?.as_u64()?.to_string(),
                author: Some(s(first, "/author/uniqueName"))
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| s(first, "/author/displayName")),
                body: s(first, "/content"),
                path: Some(path.trim_start_matches('/').to_owned()).filter(|p| !p.is_empty()),
                line: t
                    .pointer("/threadContext/rightFileStart/line")
                    .and_then(Value::as_u64),
            })
        })
        .collect()
}

/// A new thread: on a line when `note` is given, else a general comment.
pub fn thread(body: &str, note: Option<&InlineNote>) -> Value {
    let mut t = json!({
        "comments": [{ "parentCommentId": 0, "content": body, "commentType": 1 }],
        "status": 1,
    });
    if let Some(n) = note {
        t["threadContext"] = json!({
            "filePath": format!("/{}", n.path.trim_start_matches('/')),
            "rightFileStart": { "line": n.line, "offset": 1 },
            "rightFileEnd": { "line": n.line, "offset": 1 },
        });
    }
    t
}

pub fn reply(body: &str) -> Value {
    json!({ "content": body, "parentCommentId": 1, "commentType": 1 })
}

/// `(base commit, head commit)` of a PR's last iteration.
pub fn last_iteration(iterations: &Value) -> Option<(u64, String, String)> {
    let last = iterations.get("value")?.as_array()?.last()?;
    Some((
        last.get("id")?.as_u64()?,
        s(last, "/commonRefCommit/commitId"),
        s(last, "/sourceRefCommit/commitId"),
    ))
}

/// `(path, change type)` of an iteration's changes (folders skipped).
pub fn changed_paths(changes: &Value) -> Vec<(String, String)> {
    changes
        .get("changeEntries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.pointer("/item/isFolder").and_then(Value::as_bool) != Some(true))
        .filter_map(|c| {
            let p = c.pointer("/item/path")?.as_str()?;
            Some((p.trim_start_matches('/').to_owned(), s(c, "/changeType")))
        })
        .collect()
}

/// Items API path for `path` at `commit`, with content.
pub fn item_path(path: &str, commit: &str) -> String {
    let mut enc = String::new();
    for b in format!("/{path}").bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            enc.push(char::from(b));
        } else {
            let _ = write!(enc, "%{b:02X}");
        }
    }
    format!(
        "/items?path={enc}&versionDescriptor.versionType=commit&versionDescriptor.version={commit}&includeContent=true"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_remote_form() {
        let r = remote(
            "https",
            "dev.azure.com",
            "dev.azure.com",
            "contoso/Web/_git/site",
        )
        .unwrap();
        assert_eq!((r.owner.as_str(), r.repo.as_str()), ("contoso/Web", "site"));
        assert_eq!(
            r.api,
            "https://dev.azure.com/contoso/Web/_apis/git/repositories/site"
        );
        let r = remote(
            "https",
            "ssh.dev.azure.com",
            "ssh.dev.azure.com",
            "v3/contoso/Web/site",
        )
        .unwrap();
        assert_eq!(r.web, "https://dev.azure.com");
        assert_eq!(r.owner, "contoso/Web");
        let r = remote(
            "https",
            "contoso.visualstudio.com",
            "contoso.visualstudio.com",
            "DefaultCollection/Web/_git/site",
        )
        .unwrap();
        assert_eq!(
            r.api,
            "https://contoso.visualstudio.com/DefaultCollection/Web/_apis/git/repositories/site"
        );
        let r = remote(
            "https",
            "tfs.corp:8080",
            "tfs.corp",
            "tfs/Main/Proj/_git/app",
        )
        .unwrap();
        assert_eq!(
            r.api,
            "https://tfs.corp:8080/tfs/Main/Proj/_apis/git/repositories/app"
        );
        assert!(remote("https", "dev.azure.com", "dev.azure.com", "contoso/_git/x").is_err());
        assert!(remote("https", "dev.azure.com", "dev.azure.com", "x").is_err());
        assert_eq!(versioned("/pullrequests"), "/pullrequests?api-version=7.1");
        assert_eq!(versioned("/p?a=1"), "/p?a=1&api-version=7.1");
    }

    #[test]
    fn prs_threads_and_bodies() {
        let v = json!({"pullRequestId": 7, "title": "T", "status": "active", "isDraft": true,
            "sourceRefName": "refs/heads/feat", "targetRefName": "refs/heads/main",
            "createdBy": {"displayName": "Ada", "uniqueName": "ada@x"},
            "repository": {"webUrl": "https://dev.azure.com/o/p/_git/r"}});
        let p = pr_from(&v).unwrap();
        assert_eq!(
            (p.head.as_str(), p.base.as_str(), p.author.as_str()),
            ("feat", "main", "ada@x")
        );
        assert_eq!(p.url, "https://dev.azure.com/o/p/_git/r/pullrequest/7");
        assert!(p.draft);
        let threads = json!({"value": [
            {"id": 1, "status": "active", "threadContext": {"filePath": "/src/a.rs", "rightFileStart": {"line": 3}},
             "comments": [{"content": "why?", "author": {"displayName": "Bob"}, "commentType": "text"}]},
            {"id": 2, "status": "fixed", "comments": [{"content": "done"}]},
            {"id": 3, "comments": [{"content": "voted", "commentType": "system"}]},
            {"id": 4, "status": "active", "comments": [{"content": "general"}]}
        ]});
        let c = comments(&threads);
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].id.as_str(), c[0].path.as_deref(), c[0].line),
            ("1", Some("src/a.rs"), Some(3))
        );
        assert_eq!(c[1].path, None);
        let t = thread(
            "x",
            Some(&InlineNote {
                path: "src/a.rs".into(),
                line: 5,
                body: "x".into(),
            }),
        );
        assert_eq!(t["threadContext"]["filePath"], "/src/a.rs");
        assert!(thread("x", None).get("threadContext").is_none());
        assert_eq!(
            item_path("a b.rs", "abc"),
            "/items?path=/a%20b.rs&versionDescriptor.versionType=commit&versionDescriptor.version=abc&includeContent=true"
        );
    }
}
