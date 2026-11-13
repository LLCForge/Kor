//! Bitbucket Server / Data Center shapes for the forge client (§28.7): remote URL forms, pull
//! request JSON, activities (comments) and the bodies Kor posts. Requests go through
//! [`super::api::Forge`] like every other forge.
//!
//! Remotes: `https://host[/context]/scm/{PROJECT}/{repo}.git` and
//! `ssh://git@host:7999/{project}/{repo}.git` (host in `[forge.hosts]` as `bitbucket-server`).
//! API: `{web}/rest/api/1.0/projects/{PROJECT}/repos/{repo}` (REST API 1.0, documented at
//! <https://developer.atlassian.com/server/bitbucket/rest/>); build statuses come from
//! `{web}/rest/build-status/1.0/commits/{sha}`.

use serde_json::{Value, json};

use super::api::{Comment, InlineNote, PrInfo};
use super::{ForgeError, Kind, PrRequest, Remote};

/// The repository behind a Bitbucket Server remote.
///
/// # Errors
/// `NoRemote` when the path has no project and repository.
pub fn remote(scheme: &str, authority: &str, host: &str, path: &str) -> Result<Remote, ForgeError> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // HTTP clone URLs carry `scm/`; whatever comes before it is the server's context path.
    let (context, rest) = match segs.iter().position(|s| *s == "scm") {
        Some(i) => (segs[..i].join("/"), &segs[i + 1..]),
        None => (String::new(), &segs[..]),
    };
    let [project, repo] = rest else {
        return Err(ForgeError::NoRemote);
    };
    let web = if context.is_empty() {
        format!("{scheme}://{authority}")
    } else {
        format!("{scheme}://{authority}/{context}")
    };
    Ok(Remote {
        kind: Kind::BitbucketServer,
        host: host.to_owned(),
        owner: (*project).to_owned(),
        repo: (*repo).to_owned(),
        api: format!("{web}/rest/api/1.0"),
        web,
    })
}

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A pull request from Bitbucket Server's JSON.
pub fn pr_from(v: &Value) -> Option<PrInfo> {
    let author = Some(s(v, "/author/user/name"))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| s(v, "/author/user/displayName"));
    Some(PrInfo {
        number: v.get("id")?.as_u64()?,
        title: s(v, "/title"),
        url: s(v, "/links/self/0/href"),
        head: s(v, "/fromRef/displayId"),
        base: s(v, "/toRef/displayId"),
        author,
        state: s(v, "/state").to_ascii_lowercase(),
        draft: v.get("draft").and_then(Value::as_bool) == Some(true),
    })
}

pub fn create_body(req: &PrRequest) -> Value {
    json!({
        "title": req.title,
        "description": req.body,
        "draft": req.draft,
        "fromRef": { "id": format!("refs/heads/{}", req.head) },
        "toRef": { "id": format!("refs/heads/{}", req.base) },
    })
}

/// Top-level comments of a PR's activity stream (replies hang under them).
pub fn comments(activities: &Value) -> Vec<Comment> {
    activities
        .get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| a.get("action").and_then(Value::as_str) == Some("COMMENTED"))
        .filter(|a| a.get("commentAction").and_then(Value::as_str) != Some("DELETED"))
        .filter_map(|a| {
            let c = a.get("comment")?;
            let anchor = a.get("commentAnchor").or_else(|| c.get("anchor"));
            Some(Comment {
                id: c.get("id")?.as_u64()?.to_string(),
                author: s(c, "/author/name"),
                body: s(c, "/text"),
                path: anchor.map(|x| s(x, "/path")).filter(|p| !p.is_empty()),
                line: anchor.and_then(|x| x.get("line")).and_then(Value::as_u64),
            })
        })
        .collect()
}

/// A comment: on a line of the PR's new side when `note` is given, else general.
pub fn comment(body: &str, note: Option<&InlineNote>) -> Value {
    let mut c = json!({ "text": body });
    if let Some(n) = note {
        c["anchor"] = json!({
            "path": n.path, "line": n.line, "lineType": "ADDED",
            "fileType": "TO", "diffType": "EFFECTIVE",
        });
    }
    c
}

pub fn reply(parent: &str, body: &str) -> Value {
    json!({ "text": body, "parent": { "id": parent.parse::<u64>().unwrap_or_default() } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_forms() {
        let r = remote("https", "git.corp", "git.corp", "scm/PROJ/app").unwrap();
        assert_eq!((r.owner.as_str(), r.repo.as_str()), ("PROJ", "app"));
        assert_eq!(r.api, "https://git.corp/rest/api/1.0");
        let r = remote("https", "corp:8443", "corp", "bitbucket/scm/P/x").unwrap();
        assert_eq!(r.web, "https://corp:8443/bitbucket");
        let r = remote("https", "git.corp", "git.corp", "proj/app").unwrap();
        assert_eq!(r.owner, "proj");
        assert!(remote("https", "git.corp", "git.corp", "scm/only").is_err());
    }

    #[test]
    fn prs_and_comments() {
        let v = json!({"id": 4, "title": "T", "state": "OPEN", "draft": false,
            "fromRef": {"displayId": "feat"}, "toRef": {"displayId": "main"},
            "author": {"user": {"name": "ada"}},
            "links": {"self": [{"href": "https://git.corp/projects/P/repos/r/pull-requests/4"}]}});
        let p = pr_from(&v).unwrap();
        assert_eq!(
            (p.number, p.state.as_str(), p.head.as_str()),
            (4, "open", "feat")
        );
        let a = json!({"values": [
            {"action": "COMMENTED", "comment": {"id": 9, "text": "why?", "author": {"name": "bob"}},
             "commentAnchor": {"path": "src/a.rs", "line": 3}},
            {"action": "APPROVED"},
            {"action": "COMMENTED", "comment": {"id": 10, "text": "ok", "author": {"name": "bob"}}}
        ]});
        let c = comments(&a);
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].path.as_deref(), c[0].line),
            (Some("src/a.rs"), Some(3))
        );
        assert_eq!(reply("9", "x")["parent"]["id"], 9);
        assert!(comment("x", None).get("anchor").is_none());
    }
}
