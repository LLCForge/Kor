//! Universal Git (§8.8, ADR 0040): one interface for GitHub, GitLab, Gitea/Forgejo, Bitbucket
//! Cloud and Azure DevOps (Services and Server, §28.7, ADR 0063). The forge comes from the `origin` remote (self-hosted hosts in `[forge.hosts]`), the
//! token from the environment or the OS keychain (`kor auth login github|gitlab|gitea|bitbucket`),
//! and every request goes through the network guard, so privacy/offline modes refuse non-local
//! forges with their own localized message. `git push` runs the user's git.

mod api;
mod azure;
pub mod respond;

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use kor_agent::gitflow::{self, Entry, FlowError};
use kor_i18n::{Localizer, args};
use kor_providers::Provider;
use kor_secrets::{SecretStore, SecretString, account_for};
use serde::Serialize;

pub use api::{Comment, Forge, InlineNote, PrInfo, ReviewPosted};

/// Diff characters given to the model for a description.
const MAX_DIFF: usize = 48_000;

/// A pull request to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRequest {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// An opened pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrRef {
    pub url: String,
    pub number: u64,
    pub forge: String,
}

/// Why a forge operation failed.
#[derive(Debug)]
pub enum ForgeError {
    /// No usable git remote (or not a git repository).
    NoRemote,
    /// The remote's host is not a known forge (add it to `[forge.hosts]`).
    Unsupported(String),
    /// No token stored for this forge (`kor auth login <forge>`).
    NoToken(String),
    /// Privacy or offline mode forbids reaching this host; the message is localized.
    Blocked(String),
    /// The forge answered with an error status.
    Http { status: u16, body: String },
    /// Transport failure.
    Network(String),
    /// A git command failed.
    Git(String),
    /// Unexpected response shape.
    Parse(String),
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRemote => f.write_str("no git remote"),
            Self::Unsupported(h) => write!(f, "unsupported forge host: {h}"),
            Self::NoToken(k) => write!(f, "no token for {k} (kor auth login {k})"),
            Self::Blocked(m) | Self::Network(m) | Self::Git(m) | Self::Parse(m) => f.write_str(m),
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
        }
    }
}

impl std::error::Error for ForgeError {}

impl From<FlowError> for ForgeError {
    fn from(e: FlowError) -> Self {
        match e {
            FlowError::NotRepo => Self::NoRemote,
            other => Self::Git(other.to_string()),
        }
    }
}

/// The user-facing text for `e`.
pub fn error_text(l: &Localizer, e: &ForgeError) -> String {
    match e {
        ForgeError::NoRemote => l.t("forge-no-remote"),
        ForgeError::Unsupported(h) => l.t_args("forge-unsupported", &args!("host" => h.clone())),
        ForgeError::NoToken(k) => l.t_args("forge-no-token", &args!("forge" => k.clone())),
        ForgeError::Blocked(m) => m.clone(),
        ForgeError::Http { status, body } => l.t_args(
            "forge-http",
            &args!("status" => u64::from(*status), "body" => body.clone()),
        ),
        ForgeError::Network(m) | ForgeError::Parse(m) => {
            l.t_args("forge-network", &args!("error" => m.clone()))
        }
        ForgeError::Git(m) => l.t_args("flow-git-failed", &args!("error" => m.clone())),
    }
}

// ---------------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------------

/// The forge software behind a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Github,
    Gitlab,
    /// Gitea and Forgejo (Codeberg) share the API.
    Gitea,
    /// Bitbucket Cloud.
    Bitbucket,
    /// Azure DevOps Services (dev.azure.com, *.visualstudio.com) and Azure DevOps Server.
    Azure,
}

impl Kind {
    pub const ALL: [Self; 5] = [Self::Github, Self::Gitlab, Self::Gitea, Self::Bitbucket, Self::Azure];

    pub fn id(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Gitea => "gitea",
            Self::Bitbucket => "bitbucket",
            Self::Azure => "azure",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "github" => Some(Self::Github),
            "gitlab" => Some(Self::Gitlab),
            "gitea" | "forgejo" | "codeberg" => Some(Self::Gitea),
            "bitbucket" => Some(Self::Bitbucket),
            "azure" | "azure-devops" | "azuredevops" | "ado" | "tfs" => Some(Self::Azure),
            _ => None,
        }
    }

    /// Environment variables checked before the keychain, in order.
    pub fn token_vars(self) -> &'static [&'static str] {
        match self {
            Self::Github => &["GITHUB_TOKEN", "GH_TOKEN"],
            Self::Gitlab => &["GITLAB_TOKEN"],
            Self::Gitea => &["GITEA_TOKEN", "FORGEJO_TOKEN"],
            Self::Bitbucket => &["BITBUCKET_TOKEN"],
            // A personal access token; `SYSTEM_ACCESSTOKEN` inside Azure Pipelines.
            Self::Azure => &["AZURE_DEVOPS_TOKEN", "AZURE_DEVOPS_EXT_PAT", "SYSTEM_ACCESSTOKEN"],
        }
    }

    /// Forge id for a `kor auth` provider name, if it is one.
    pub fn for_auth_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// A repository on a forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Remote {
    pub kind: Kind,
    pub host: String,
    /// Owner, workspace or (GitLab) group path.
    pub owner: String,
    pub repo: String,
    /// `https://host[:port]`.
    pub web: String,
    /// API base without a trailing slash.
    pub api: String,
}

/// `(scheme, authority, path)` of a remote URL; `None` for local paths.
fn split_remote(url: &str) -> Option<(String, String, String)> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let (auth, path) = rest.split_once('/')?;
        let auth = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
        let scheme = scheme.to_ascii_lowercase();
        return match scheme.as_str() {
            "http" | "https" => Some((scheme, auth.to_ascii_lowercase(), path.to_owned())),
            // ssh://git@host:2222/x → the web side is https on the default port.
            "ssh" | "git" | "git+ssh" => {
                let host = auth.split(':').next()?.to_ascii_lowercase();
                Some(("https".into(), host, path.to_owned()))
            }
            _ => None,
        };
    }
    // scp-like `git@host:owner/repo.git` (a Windows path `C:\x` has no '@' and a 1-char host).
    let (left, path) = url.split_once(':')?;
    let host = left.rsplit_once('@').map_or(left, |(_, h)| h);
    (host.len() > 1 && !path.starts_with('\\') && !path.starts_with('/'))
        .then(|| ("https".into(), host.to_ascii_lowercase(), path.to_owned()))
}

/// Which forge `host` runs: `[forge.hosts]` first, then the public services, then a name hint.
pub fn kind_for(host: &str, authority: &str, hosts: &BTreeMap<String, String>) -> Option<Kind> {
    let configured = hosts
        .iter()
        .find(|(h, _)| h.eq_ignore_ascii_case(authority) || h.eq_ignore_ascii_case(host))
        .and_then(|(_, k)| Kind::parse(k));
    configured.or(match host {
        "github.com" => Some(Kind::Github),
        "gitlab.com" => Some(Kind::Gitlab),
        "codeberg.org" => Some(Kind::Gitea),
        "bitbucket.org" => Some(Kind::Bitbucket),
        "dev.azure.com" | "ssh.dev.azure.com" | "vs-ssh.visualstudio.com" => Some(Kind::Azure),
        h if h.ends_with(".visualstudio.com") => Some(Kind::Azure),
        h if h.starts_with("gitlab.") => Some(Kind::Gitlab),
        h if h.starts_with("gitea.") || h.starts_with("forgejo.") => Some(Kind::Gitea),
        _ => None,
    })
}

impl Remote {
    /// The forge repository behind a remote URL.
    ///
    /// # Errors
    /// `NoRemote` for a local path or a URL without owner/repo, `Unsupported` for an unknown host.
    pub fn parse(url: &str, hosts: &BTreeMap<String, String>) -> Result<Self, ForgeError> {
        let (scheme, authority, path) = split_remote(url).ok_or(ForgeError::NoRemote)?;
        let host = authority.split(':').next().unwrap_or_default().to_owned();
        let kind = kind_for(&host, &authority, hosts)
            .ok_or_else(|| ForgeError::Unsupported(host.clone()))?;
        let path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        if kind == Kind::Azure {
            return azure::remote(&scheme, &authority, &host, path);
        }
        let (owner, repo) = path.rsplit_once('/').ok_or(ForgeError::NoRemote)?;
        if owner.is_empty() || repo.is_empty() {
            return Err(ForgeError::NoRemote);
        }
        let web = format!("{scheme}://{authority}");
        let api = match kind {
            Kind::Github if host == "github.com" => "https://api.github.com".to_owned(),
            Kind::Github => format!("{web}/api/v3"),
            Kind::Gitlab => format!("{web}/api/v4"),
            Kind::Gitea => format!("{web}/api/v1"),
            Kind::Bitbucket if host == "bitbucket.org" => "https://api.bitbucket.org/2.0".to_owned(),
            Kind::Bitbucket => format!("{web}/2.0"),
            Kind::Azure => unreachable!("handled above"),
        };
        Ok(Self {
            kind,
            host,
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            web,
            api,
        })
    }
}

/// `[forge.hosts]` of the configuration that applies to `root` (empty when it cannot be read).
pub fn configured_hosts(root: &Path) -> BTreeMap<String, String> {
    kor_config::load(&kor_config::Paths::discover(root), std::env::vars(), &[])
        .map(|l| l.config.forge.hosts)
        .unwrap_or_default()
}

/// URL of `origin` (else the first remote).
///
/// # Errors
/// `NoRemote` when the repository has none.
pub fn remote_url(root: &Path) -> Result<String, ForgeError> {
    if let Ok(u) = gitflow::git(root, &["remote", "get-url", "origin"]) {
        return Ok(u.trim().to_owned());
    }
    let first = gitflow::git(root, &["remote"])?
        .lines()
        .next()
        .map(str::to_owned)
        .ok_or(ForgeError::NoRemote)?;
    Ok(gitflow::git(root, &["remote", "get-url", &first])?
        .trim()
        .to_owned())
}

/// The forge repository of `root`'s remote.
///
/// # Errors
/// See [`Remote::parse`].
pub fn detect(root: &Path) -> Result<Remote, ForgeError> {
    Remote::parse(&remote_url(root)?, &configured_hosts(root))
}

/// Token for `kind`: `GITHUB_TOKEN`/`GH_TOKEN`… first, then the keychain entry `kor auth login
/// <forge>` stored. shortcut: one token per forge kind, not per self-hosted host; add host-scoped
/// accounts when someone uses two instances of the same forge.
///
/// # Errors
/// `NoToken` when neither is set.
pub fn token(
    kind: Kind,
    env: impl Fn(&str) -> Option<String>,
    store: &dyn SecretStore,
) -> Result<SecretString, ForgeError> {
    for v in kind.token_vars() {
        if let Some(t) = env(v).filter(|t| !t.trim().is_empty()) {
            return Ok(SecretString::from(t.trim().to_owned()));
        }
    }
    let account = account_for(kind.id()).map_err(|e| ForgeError::Parse(e.to_string()))?;
    store
        .get(&account)
        .ok()
        .flatten()
        .ok_or_else(|| ForgeError::NoToken(kind.id().to_owned()))
}

/// PR number from `12`, `#12` or a PR/MR web URL.
pub fn pr_number(s: &str) -> Option<u64> {
    let s = s.trim().trim_start_matches('#');
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    let path = s.split(['?', '#']).next()?;
    path.split('/').rev().find_map(|seg| seg.parse().ok())
}

// ---------------------------------------------------------------------------------------------
// git
// ---------------------------------------------------------------------------------------------

/// The checked-out branch.
///
/// # Errors
/// `Git` on a detached HEAD.
pub fn current_branch(root: &Path) -> Result<String, ForgeError> {
    let b = gitflow::git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned();
    if b.is_empty() || b == "HEAD" {
        return Err(ForgeError::Git("detached HEAD".into()));
    }
    Ok(b)
}

/// Has this branch been pushed before (does it track a remote branch)?
pub fn has_upstream(root: &Path, branch: &str) -> bool {
    gitflow::git(
        root,
        &["rev-parse", "--abbrev-ref", &format!("{branch}@{{upstream}}")],
    )
    .is_ok()
}

/// The remote's default branch, else `main`/`master` when one exists locally, else `main`.
pub fn default_base(root: &Path) -> String {
    if let Ok(r) = gitflow::git(root, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        let r = r.trim();
        if let Some(b) = r.strip_prefix("origin/") {
            return b.to_owned();
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|b| gitflow::git(root, &["rev-parse", "--verify", "--quiet", b]).is_ok())
        .unwrap_or("main")
        .to_owned()
}

fn safe_ref(r: &str) -> Result<&str, ForgeError> {
    if r.is_empty() || r.starts_with('-') || r.contains("..") || r.chars().any(char::is_whitespace) {
        Err(ForgeError::Git(format!("invalid branch name: {r}")))
    } else {
        Ok(r)
    }
}

/// The network guard's answer for pushing to `url` (local paths are always allowed).
///
/// # Errors
/// `Blocked` with the localized privacy/offline refusal.
pub fn check_push(url: &str) -> Result<(), ForgeError> {
    match split_remote(url) {
        None => Ok(()),
        Some((_, authority, _)) => {
            let host = authority.split(':').next().unwrap_or_default();
            kor_core::net::check_host(host).map_err(|b| ForgeError::Blocked(b.message))
        }
    }
}

/// `git push -u <remote> <branch>` with the user's git (credentials are git's business).
///
/// # Errors
/// `Blocked` under privacy/offline for a non-local remote, `Git` when the push fails.
pub fn push(root: &Path, branch: &str) -> Result<String, ForgeError> {
    let branch = safe_ref(branch)?;
    check_push(&remote_url(root)?)?;
    let remote = if gitflow::git(root, &["remote", "get-url", "origin"]).is_ok() {
        "origin".to_owned()
    } else {
        gitflow::git(root, &["remote"])?
            .lines()
            .next()
            .ok_or(ForgeError::NoRemote)?
            .to_owned()
    };
    let out = gitflow::git(root, &["push", "-u", &remote, branch])?;
    Ok(out.trim().to_owned())
}

// ---------------------------------------------------------------------------------------------
// Description
// ---------------------------------------------------------------------------------------------

/// Title and Markdown body of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Description {
    pub title: String,
    pub body: String,
}

/// Commits on HEAD that `base` does not have, and their diff (secrets masked, cut at 48K chars).
///
/// # Errors
/// git failures.
pub fn branch_changes(root: &Path, base: &str) -> Result<(Vec<Entry>, String), ForgeError> {
    let base = safe_ref(base)?;
    let remote_base = format!("origin/{base}");
    let b = if gitflow::git(root, &["rev-parse", "--verify", "--quiet", &remote_base]).is_ok() {
        remote_base
    } else {
        base.to_owned()
    };
    let entries = gitflow::log(root, Some(&b))?;
    let mut diff = gitflow::git(
        root,
        &["diff", "--no-color", "--no-ext-diff", &format!("{b}...HEAD"), "--"],
    )?;
    if let Some((i, _)) = diff.char_indices().nth(MAX_DIFF) {
        diff.truncate(i);
        diff.push_str("\n[diff truncated]\n");
    }
    Ok((entries, kor_tools::redact_secrets(&diff)))
}

/// A description without a model: the single commit's subject (else the branch name) and a
/// Keep a Changelog list of the commits.
pub fn fallback_description(branch: &str, entries: &[Entry]) -> Description {
    let title = match entries {
        [one] => one.subject.clone(),
        _ => branch.replace(['-', '_', '/'], " ").trim().to_owned(),
    };
    let section = gitflow::section(entries, "Changes");
    let body = section
        .split_once('\n')
        .map_or(section.as_str(), |(_, rest)| rest)
        .trim()
        .to_owned();
    Description { title, body }
}

pub fn description_prompt(entries: &[Entry], diff: &str) -> String {
    let commits = entries.iter().fold(String::new(), |mut s, e| {
        let _ = writeln!(s, "- {} {}", e.hash, e.subject);
        s
    });
    format!(
        "Write the pull request title and description for these changes.\nFirst line: the title \
         (English, imperative, at most 72 characters, no final period, no `#`). Then a blank \
         line and a Markdown body with the sections `## Summary` (why, 1-3 sentences), \
         `## Changes` (bullets of what changed for reviewers) and `## Testing` (how it was or \
         should be verified). Mention breaking changes explicitly. No code fences around the \
         whole reply.\n\nCommits:\n{commits}\nDiff:\n{diff}"
    )
}

/// Title and body from the model's reply; `None` when it is empty.
pub fn parse_description(reply: &str) -> Option<Description> {
    let text: String = reply
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lines = text.trim().lines();
    let title = lines
        .next()?
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_start_matches("Title:")
        .trim()
        .trim_matches('"')
        .trim_end_matches('.')
        .to_owned();
    if title.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_owned();
    Some(Description { title, body })
}

/// Description from the branch's commits and diff: the model when one is given, else
/// [`fallback_description`]. A failed model request also falls back (the error is returned
/// alongside so callers can say so).
pub async fn describe(
    model: Option<(&Arc<dyn Provider>, &str)>,
    root: &Path,
    base: &str,
    branch: &str,
) -> Result<(Description, Option<String>), ForgeError> {
    let (entries, diff) = branch_changes(root, base)?;
    let fallback = fallback_description(branch, &entries);
    let Some((provider, model)) = model else {
        return Ok((fallback, None));
    };
    match gitflow::ask(provider, model, description_prompt(&entries, &diff), 2048).await {
        Ok(reply) => Ok(parse_description(&reply).map_or_else(
            || (fallback.clone(), Some("empty reply".into())),
            |d| (d, None),
        )),
        Err(e) => Ok((fallback, Some(e.to_string()))),
    }
}

/// Open a pull request for `req.head` → `req.base` on the forge behind `root`'s remote. Nothing
/// is pushed here (see [`push`]).
///
/// # Errors
/// See [`ForgeError`].
pub async fn create_pull_request(root: &Path, req: &PrRequest) -> Result<PrRef, ForgeError> {
    Forge::open(root)?.create(req).await
}

#[cfg(test)]
mod tests;
