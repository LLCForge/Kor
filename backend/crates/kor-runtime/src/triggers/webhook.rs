//! Signed webhook requests: HMAC-SHA256 (GitHub `X-Hub-Signature-256`, Gitea/Forgejo
//! `X-Gitea-Signature`, Kor's own `X-Kor-Signature`) or GitLab's shared `X-Gitlab-Token`, and the
//! push events of forge payloads.

use hmac::{Hmac, Mac as _};
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq as _;

/// A push read from a local hook or a forge payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Push {
    /// Where it was pushed: a local repository path or clone URLs of the forge repository.
    pub repos: Vec<String>,
    /// Branch name (`main`), from `refs/heads/main` (or a remote-tracking ref of a local hook).
    pub branch: String,
    pub before: String,
    pub after: String,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// `sha256=<hex>` of `body` under `secret` (what a sender puts in `X-Kor-Signature`).
///
/// # Panics
/// Never: HMAC takes keys of any length.
pub fn sign(secret: &[u8], body: &[u8]) -> String {
    #[allow(clippy::expect_used)] // HMAC accepts keys of any length
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(body);
    format!("sha256={}", hex(&mac.finalize().into_bytes()))
}

/// Whether a request is authentic for `secret`. `header` looks a header up case-insensitively.
/// Fails closed: no signature header, or an empty secret, is a rejection.
pub fn verify(secret: &[u8], body: &[u8], header: impl Fn(&str) -> Option<String>) -> bool {
    if secret.is_empty() {
        return false;
    }
    let expected = sign(secret, body);
    let eq = |a: &str, b: &str| bool::from(a.as_bytes().ct_eq(b.as_bytes()));
    for name in ["x-kor-signature", "x-hub-signature-256"] {
        if let Some(v) = header(name) {
            return eq(v.trim(), &expected);
        }
    }
    if let Some(v) = header("x-gitea-signature").or_else(|| header("x-forgejo-signature")) {
        return eq(&format!("sha256={}", v.trim()), &expected);
    }
    if let Some(v) = header("x-gitlab-token") {
        return eq(v.trim(), &String::from_utf8_lossy(secret));
    }
    false
}

/// `refs/heads/x` → `x`; a remote-tracking ref (`refs/remotes/origin/x`) → `x`.
pub fn branch_of(r#ref: &str) -> Option<String> {
    if let Some(b) = r#ref.strip_prefix("refs/heads/") {
        return Some(b.to_owned());
    }
    let rest = r#ref.strip_prefix("refs/remotes/")?;
    let (_, b) = rest.split_once('/')?;
    (b != "HEAD").then(|| b.to_owned())
}

const ZERO: &str = "0000000000000000000000000000000000000000";

/// The push in a GitHub, GitLab, Gitea/Forgejo or Bitbucket payload; `None` for other events,
/// tags and branch deletions.
pub fn forge_push(payload: &Value) -> Option<Push> {
    let str_at = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    // Bitbucket: push.changes[].new
    if let Some(change) = payload.pointer("/push/changes/0") {
        let new = change.get("new").filter(|n| n["type"] == "branch")?;
        let repo = &payload["repository"];
        let mut repos: Vec<String> = [str_at(repo, "/links/html/href"), str_at(repo, "/full_name")]
            .into_iter()
            .flatten()
            .collect();
        repos.dedup();
        return Some(Push {
            repos,
            branch: str_at(new, "/name")?,
            before: str_at(change, "/old/target/hash").unwrap_or_default(),
            after: str_at(new, "/target/hash")?,
        });
    }
    let branch = branch_of(payload["ref"].as_str()?)?;
    let after = payload["after"]
        .as_str()
        .or_else(|| payload["checkout_sha"].as_str())?
        .to_owned();
    if after == ZERO {
        return None;
    }
    let repo = &payload["repository"];
    let project = &payload["project"];
    let repos = [
        str_at(repo, "/clone_url"),
        str_at(repo, "/ssh_url"),
        str_at(repo, "/html_url"),
        str_at(repo, "/git_http_url"),
        str_at(repo, "/git_ssh_url"),
        str_at(project, "/git_http_url"),
        str_at(project, "/git_ssh_url"),
        str_at(project, "/web_url"),
        str_at(repo, "/full_name"),
        str_at(project, "/path_with_namespace"),
    ]
    .into_iter()
    .flatten()
    .collect();
    Some(Push {
        repos,
        branch,
        before: payload["before"].as_str().unwrap_or_default().to_owned(),
        after,
    })
}

/// `host/owner/repo` (lowercase, no scheme, user or `.git`) of a remote URL or scp-style address;
/// a local path stays a path. Used to match a pushed repository to a project's remote.
pub fn normalize_remote(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix(".git").unwrap_or(u);
    if let Some((_, rest)) = u.split_once("://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        return rest.to_lowercase();
    }
    // scp-like `git@host:owner/repo`
    if let Some((left, path)) = u.split_once(':')
        && !left.contains('/')
        && !left.contains('\\')
        && left.len() > 1
    {
        let host = left.rsplit_once('@').map_or(left, |(_, h)| h);
        return format!("{host}/{path}").to_lowercase();
    }
    u.to_owned()
}

/// Whether one of a push's repositories is the project remote `url` (`owner/repo` names match the
/// end of the remote).
pub fn same_repo(push_repo: &str, url: &str) -> bool {
    let local = |s: &str| {
        let p = s.trim().trim_start_matches("file://");
        // `file:///C:/x` → `C:/x`
        let p = if p.as_bytes().get(2) == Some(&b':') {
            &p[1..]
        } else {
            p
        };
        std::fs::canonicalize(p).ok()
    };
    if let (Some(x), Some(y)) = (local(push_repo), local(url)) {
        return x == y;
    }
    let (a, b) = (normalize_remote(push_repo), normalize_remote(url));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    // `owner/repo` (no host) against `host/owner/repo`
    !a.contains('.')
        && a.matches('/').count() == 1
        && b.to_lowercase()
            .ends_with(&format!("/{}", a.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signatures_fail_closed() {
        let body = br#"{"ref":"refs/heads/main"}"#;
        let sig = sign(b"s3cret", body);
        let h = |name: &'static str, v: String| move |n: &str| (n == name).then(|| v.clone());
        assert!(verify(
            b"s3cret",
            body,
            h("x-hub-signature-256", sig.clone())
        ));
        assert!(verify(b"s3cret", body, h("x-kor-signature", sig.clone())));
        assert!(verify(
            b"s3cret",
            body,
            h("x-gitea-signature", sig[7..].to_owned())
        ));
        assert!(verify(
            b"s3cret",
            body,
            h("x-gitlab-token", "s3cret".into())
        ));
        assert!(!verify(
            b"other",
            body,
            h("x-hub-signature-256", sig.clone())
        ));
        assert!(!verify(
            b"s3cret",
            b"tampered",
            h("x-hub-signature-256", sig.clone())
        ));
        assert!(!verify(b"s3cret", body, |_| None), "unsigned");
        assert!(
            !verify(b"", body, h("x-gitlab-token", String::new())),
            "empty secret"
        );
    }

    #[test]
    fn forge_pushes() {
        let gh = json!({"ref": "refs/heads/main", "before": "a", "after": "b",
            "repository": {"clone_url": "https://github.com/acme/app.git", "full_name": "acme/app"}});
        let p = forge_push(&gh).unwrap();
        assert_eq!((p.branch.as_str(), p.after.as_str()), ("main", "b"));
        assert!(
            p.repos
                .iter()
                .any(|r| same_repo(r, "git@github.com:Acme/App.git"))
        );
        let tag = json!({"ref": "refs/tags/v1", "after": "b"});
        assert!(forge_push(&tag).is_none());
        let deleted = json!({"ref": "refs/heads/x", "after": ZERO});
        assert!(forge_push(&deleted).is_none());
        let bb = json!({"push": {"changes": [{"new": {"type": "branch", "name": "dev", "target": {"hash": "c"}}, "old": {"target": {"hash": "a"}}}]},
            "repository": {"full_name": "acme/app", "links": {"html": {"href": "https://bitbucket.org/acme/app"}}}});
        let p = forge_push(&bb).unwrap();
        assert_eq!((p.branch.as_str(), p.after.as_str()), ("dev", "c"));
        assert!(same_repo("acme/app", "https://bitbucket.org/acme/app.git"));
        assert!(!same_repo(
            "acme/other",
            "https://bitbucket.org/acme/app.git"
        ));
    }

    #[test]
    fn refs_and_remotes() {
        assert_eq!(branch_of("refs/heads/feat/x").as_deref(), Some("feat/x"));
        assert_eq!(
            branch_of("refs/remotes/origin/main").as_deref(),
            Some("main")
        );
        assert_eq!(branch_of("refs/remotes/origin/HEAD"), None);
        assert_eq!(branch_of("refs/tags/v1"), None);
        assert_eq!(
            normalize_remote("ssh://git@host.com:22/a/b.git"),
            "host.com:22/a/b"
        );
        assert_eq!(normalize_remote("git@host.com:a/b.git"), "host.com/a/b");
    }
}
