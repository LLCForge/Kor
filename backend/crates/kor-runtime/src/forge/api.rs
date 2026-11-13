//! The forge REST APIs behind one client: GitHub (v3), GitLab (v4), Gitea/Forgejo (v1),
//! Bitbucket Cloud (2.0), Bitbucket Server/Data Center (1.0, shapes in `bbs.rs`), Azure DevOps
//! (7.1, shapes in `azure.rs`). Response shapes are mapped to [`PrInfo`] and [`Comment`] here.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use kor_secrets::{ExposeSecret, KeychainStore, SecretString};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};

use super::{ForgeError, Kind, PrRef, PrRequest, Remote, azure, bbs};

/// Response bodies longer than this are cut in error messages.
const MAX_ERROR_BODY: usize = 500;

/// A pull (merge) request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head: String,
    pub base: String,
    pub author: String,
    /// The forge's own word (`open`, `opened`, `OPEN`, `merged`, `closed`…).
    pub state: String,
    pub draft: bool,
}

/// A review comment or a general PR comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    /// Comment id (GitLab: the discussion id, which is what replies need).
    pub id: String,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u64>,
}

/// One inline note of a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InlineNote {
    pub path: String,
    pub line: u64,
    pub body: String,
}

/// What a posted review became.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReviewPosted {
    /// Notes posted on their line.
    pub inline: usize,
    /// Notes the forge refused inline, folded into the summary instead.
    pub folded: usize,
}

/// A client for one repository.
pub struct Forge {
    pub remote: Remote,
    token: SecretString,
    http: reqwest::Client,
}

impl std::fmt::Debug for Forge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is never printed.
        f.debug_struct("Forge")
            .field("remote", &self.remote)
            .finish_non_exhaustive()
    }
}

pub(super) fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn n(v: &Value, ptr: &str) -> Option<u64> {
    v.pointer(ptr).and_then(Value::as_u64)
}

/// Percent-encode one path segment (GitLab's `group%2Fproject` ids).
pub(super) fn enc(seg: &str) -> String {
    let mut out = String::new();
    for b in seg.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn draft_title(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    l.starts_with("draft:") || l.starts_with("wip:") || l.starts_with("[wip]")
}

/// A PR from any forge's JSON.
pub(crate) fn pr_from(kind: Kind, v: &Value) -> Option<PrInfo> {
    let pr = match kind {
        Kind::Github | Kind::Gitea => PrInfo {
            number: n(v, "/number")?,
            title: s(v, "/title"),
            url: s(v, "/html_url"),
            head: s(v, "/head/ref"),
            base: s(v, "/base/ref"),
            author: s(v, "/user/login"),
            state: if v.pointer("/merged").and_then(Value::as_bool) == Some(true) {
                "merged".into()
            } else {
                s(v, "/state")
            },
            draft: v.pointer("/draft").and_then(Value::as_bool) == Some(true)
                || draft_title(&s(v, "/title")),
        },
        Kind::Gitlab => PrInfo {
            number: n(v, "/iid")?,
            title: s(v, "/title"),
            url: s(v, "/web_url"),
            head: s(v, "/source_branch"),
            base: s(v, "/target_branch"),
            author: s(v, "/author/username"),
            state: s(v, "/state"),
            draft: v.pointer("/draft").and_then(Value::as_bool) == Some(true)
                || v.pointer("/work_in_progress").and_then(Value::as_bool) == Some(true),
        },
        Kind::Bitbucket => PrInfo {
            number: n(v, "/id")?,
            title: s(v, "/title"),
            url: s(v, "/links/html/href"),
            head: s(v, "/source/branch/name"),
            base: s(v, "/destination/branch/name"),
            author: s(v, "/author/display_name"),
            state: s(v, "/state").to_ascii_lowercase(),
            draft: v.pointer("/draft").and_then(Value::as_bool) == Some(true),
        },
        Kind::BitbucketServer => bbs::pr_from(v)?,
        Kind::Azure => azure::pr_from(v)?,
    };
    Some(pr)
}

pub(super) fn items(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        other => other
            .get("values")
            .or_else(|| other.get("value"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

/// `--- a/x` / `+++ b/x` headers for GitLab's per-file `changes`.
pub(crate) fn gitlab_diff(changes: &Value) -> String {
    let mut out = String::new();
    for c in changes
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (old, new) = (s(c, "/old_path"), s(c, "/new_path"));
        let from = if c.get("new_file").and_then(Value::as_bool) == Some(true) {
            "/dev/null".to_owned()
        } else {
            format!("a/{old}")
        };
        let to = if c.get("deleted_file").and_then(Value::as_bool) == Some(true) {
            "/dev/null".to_owned()
        } else {
            format!("b/{new}")
        };
        let _ = write!(
            out,
            "diff --git a/{old} b/{new}\n--- {from}\n+++ {to}\n{}",
            s(c, "/diff")
        );
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

impl Forge {
    /// A client for `remote` with `token`, built on the network guard.
    ///
    /// # Errors
    /// `Network` when the HTTP client cannot be built.
    pub fn new(remote: Remote, token: SecretString) -> Result<Self, ForgeError> {
        Self::with_config(remote, token, &kor_config::Forge::default())
    }

    /// [`Forge::new`] with `[forge]`'s private CA, client certificate (mTLS) and proxy.
    ///
    /// # Errors
    /// `Network` when a PEM file cannot be read or parsed, or the client cannot be built.
    pub fn with_config(
        remote: Remote,
        token: SecretString,
        cfg: &kor_config::Forge,
    ) -> Result<Self, ForgeError> {
        let pem = |p: &Path| {
            std::fs::read(p).map_err(|e| ForgeError::Network(format!("{}: {e}", p.display())))
        };
        let bad = |e: reqwest::Error| ForgeError::Network(e.to_string());
        let mut b = kor_core::net::client_builder()
            .user_agent(concat!("kor/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60));
        if let Some(p) = &cfg.ca_cert {
            for c in reqwest::Certificate::from_pem_bundle(&pem(p)?).map_err(bad)? {
                b = b.add_root_certificate(c);
            }
        }
        if let Some(p) = &cfg.client_cert {
            b = b.identity(reqwest::Identity::from_pem(&pem(p)?).map_err(bad)?);
        }
        // Privacy/offline modes turn proxies off (the guard must see the real host).
        if let Some(px) = cfg.proxy.as_deref().filter(|s| !s.trim().is_empty())
            && kor_core::net::policy() == kor_core::net::Policy::Open
        {
            b = b.proxy(reqwest::Proxy::all(px.trim()).map_err(bad)?);
        }
        let http = b.build().map_err(bad)?;
        Ok(Self {
            remote,
            token,
            http,
        })
    }

    /// The forge of `root`'s remote with its token from the environment or keychain.
    ///
    /// # Errors
    /// See [`super::detect`] and [`super::token`].
    pub fn open(root: &Path) -> Result<Self, ForgeError> {
        let cfg = super::configured(root);
        let remote = Remote::parse(&super::remote_url(root)?, &cfg.hosts)?;
        let token = super::token(remote.kind, |v| std::env::var(v).ok(), &KeychainStore)?;
        Self::with_config(remote, token, &cfg)
    }

    pub(super) fn repo(&self) -> String {
        let r = &self.remote;
        match r.kind {
            Kind::Github | Kind::Gitea => format!("/repos/{}/{}", r.owner, r.repo),
            Kind::Gitlab => format!("/projects/{}", enc(&format!("{}/{}", r.owner, r.repo))),
            Kind::Bitbucket => format!("/repositories/{}/{}", r.owner, r.repo),
            Kind::BitbucketServer => format!("/projects/{}/repos/{}", r.owner, r.repo),
            // The API base already names the repository.
            Kind::Azure => String::new(),
        }
    }

    fn prs(&self) -> String {
        let coll = match self.remote.kind {
            Kind::Github | Kind::Gitea => "pulls",
            Kind::Gitlab => "merge_requests",
            Kind::Bitbucket | Kind::Azure => "pullrequests",
            Kind::BitbucketServer => "pull-requests",
        };
        format!("{}/{coll}", self.repo())
    }

    /// `path` under the API base, or an absolute URL on the same forge.
    pub(super) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        accept: Option<&str>,
    ) -> Result<String, ForgeError> {
        let url = if path.starts_with("https://") || path.starts_with("http://") {
            // The token goes only to the forge's own host.
            if !path.starts_with(&format!("{}/", self.remote.web)) {
                return Err(ForgeError::Parse(format!("URL outside the forge: {path}")));
            }
            if self.remote.kind == Kind::Azure {
                azure::versioned(path)
            } else {
                path.to_owned()
            }
        } else if self.remote.kind == Kind::Azure {
            format!("{}{}", self.remote.api, azure::versioned(path))
        } else {
            format!("{}{path}", self.remote.api)
        };
        kor_core::net::check_url(&url).map_err(|b| ForgeError::Blocked(b.message))?;
        let t = self.token.expose_secret();
        let mut rb = self.http.request(method, &url);
        rb = match self.remote.kind {
            Kind::Github => rb
                .bearer_auth(t)
                .header("X-GitHub-Api-Version", "2022-11-28"),
            Kind::Gitlab => rb.header("PRIVATE-TOKEN", t),
            Kind::Gitea => rb.header("Authorization", format!("token {t}")),
            // `user:app-password` → Basic; an access token → Bearer.
            Kind::Bitbucket | Kind::BitbucketServer => match t.split_once(':') {
                Some((u, p)) => rb.basic_auth(u, Some(p)),
                None => rb.bearer_auth(t),
            },
            // A personal access token is Basic with an empty user; `Bearer <jwt>` (pipelines) as is.
            Kind::Azure => match t.strip_prefix("Bearer ") {
                Some(b) => rb.bearer_auth(b),
                None => rb.basic_auth("", Some(t)),
            },
        };
        rb = rb.header(
            "Accept",
            accept.unwrap_or(if self.remote.kind == Kind::Github {
                "application/vnd.github+json"
            } else {
                "application/json"
            }),
        );
        if let Some(b) = body {
            rb = rb
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(b).map_err(|e| ForgeError::Parse(e.to_string()))?);
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| match kor_core::net::blocked_in(&e) {
                Some(b) => ForgeError::Blocked(b.message.clone()),
                None => ForgeError::Network(kor_core::net::describe(&e)),
            })?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ForgeError::Network(e.to_string()))?;
        if status.is_success() {
            Ok(text)
        } else {
            let mut body = kor_secrets::mask_outbound(&text).into_owned();
            if let Some((i, _)) = body.char_indices().nth(MAX_ERROR_BODY) {
                body.truncate(i);
                body.push('…');
            }
            Err(ForgeError::Http {
                status: status.as_u16(),
                body,
            })
        }
    }

    pub(super) async fn json(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ForgeError> {
        let text = self.send(method, path, body, None).await?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| ForgeError::Parse(e.to_string()))
    }

    /// Open a pull request.
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    pub async fn create(&self, req: &PrRequest) -> Result<PrRef, ForgeError> {
        let kind = self.remote.kind;
        let prefixed = |p: &str| {
            if req.draft && !draft_title(&req.title) {
                format!("{p}{}", req.title)
            } else {
                req.title.clone()
            }
        };
        let body = match kind {
            Kind::Github => json!({
                "title": req.title, "head": req.head, "base": req.base,
                "body": req.body, "draft": req.draft,
            }),
            Kind::Gitlab => json!({
                "source_branch": req.head, "target_branch": req.base,
                "title": prefixed("Draft: "), "description": req.body,
            }),
            Kind::Gitea => json!({
                "head": req.head, "base": req.base,
                "title": prefixed("WIP: "), "body": req.body,
            }),
            Kind::Bitbucket => json!({
                "title": req.title, "description": req.body, "draft": req.draft,
                "source": { "branch": { "name": req.head } },
                "destination": { "branch": { "name": req.base } },
            }),
            Kind::BitbucketServer => bbs::create_body(req),
            Kind::Azure => azure::create_body(req),
        };
        let v = self.json(Method::POST, &self.prs(), Some(&body)).await?;
        let pr = pr_from(kind, &v).ok_or_else(|| ForgeError::Parse("no PR number".into()))?;
        Ok(PrRef {
            url: pr.url,
            number: pr.number,
            forge: kind.id().to_owned(),
        })
    }

    /// Open pull requests, newest first (at most 50).
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    pub async fn list(&self) -> Result<Vec<PrInfo>, ForgeError> {
        let q = match self.remote.kind {
            Kind::Github => "?state=open&per_page=50",
            Kind::Gitlab => "?state=opened&per_page=50",
            Kind::Gitea => "?state=open&limit=50",
            Kind::Bitbucket => "?state=OPEN&pagelen=50",
            Kind::BitbucketServer => "?state=OPEN&limit=50",
            Kind::Azure => "?searchCriteria.status=active&$top=50",
        };
        let v = self
            .json(Method::GET, &format!("{}{q}", self.prs()), None)
            .await?;
        Ok(items(&v)
            .iter()
            .filter_map(|p| pr_from(self.remote.kind, p))
            .collect())
    }

    /// One pull request.
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    pub async fn get(&self, number: u64) -> Result<PrInfo, ForgeError> {
        let v = self
            .json(Method::GET, &format!("{}/{number}", self.prs()), None)
            .await?;
        pr_from(self.remote.kind, &v).ok_or_else(|| ForgeError::Parse("no PR".into()))
    }

    /// The open pull request whose head is `branch`, if any.
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    pub async fn for_branch(&self, branch: &str) -> Result<Option<PrInfo>, ForgeError> {
        Ok(self.list().await?.into_iter().find(|p| p.head == branch))
    }

    /// Unified diff of a pull request.
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    pub async fn diff(&self, number: u64) -> Result<String, ForgeError> {
        let prs = self.prs();
        match self.remote.kind {
            Kind::Github => {
                self.send(
                    Method::GET,
                    &format!("{prs}/{number}"),
                    None,
                    Some("application/vnd.github.diff"),
                )
                .await
            }
            Kind::Gitea | Kind::BitbucketServer => {
                self.send(
                    Method::GET,
                    &format!("{prs}/{number}.diff"),
                    None,
                    Some("text/plain"),
                )
                .await
            }
            Kind::Bitbucket => {
                self.send(
                    Method::GET,
                    &format!("{prs}/{number}/diff"),
                    None,
                    Some("text/plain"),
                )
                .await
            }
            Kind::Gitlab => {
                let v = self
                    .json(Method::GET, &format!("{prs}/{number}/changes"), None)
                    .await?;
                Ok(gitlab_diff(&v))
            }
            Kind::Azure => self.azure_diff(number).await,
        }
    }

    /// Content of `path` at `commit` (Azure items API); `None` when it does not exist there.
    async fn azure_item(&self, path: &str, commit: &str) -> Option<String> {
        if commit.is_empty() {
            return None;
        }
        let v = self
            .json(Method::GET, &azure::item_path(path, commit), None)
            .await
            .ok()?;
        v.get("content").and_then(Value::as_str).map(str::to_owned)
    }

    /// Azure DevOps has no unified diff endpoint: the last iteration's changed files, both
    /// sides fetched and diffed here (at most 50 files).
    async fn azure_diff(&self, number: u64) -> Result<String, ForgeError> {
        let prs = self.prs();
        let its = self
            .json(Method::GET, &format!("{prs}/{number}/iterations"), None)
            .await?;
        let (it, base, head) =
            azure::last_iteration(&its).ok_or_else(|| ForgeError::Parse("no iteration".into()))?;
        let ch = self
            .json(
                Method::GET,
                &format!("{prs}/{number}/iterations/{it}/changes"),
                None,
            )
            .await?;
        let mut out = String::new();
        for (path, kind) in azure::changed_paths(&ch).into_iter().take(50) {
            let before = if kind.contains("add") {
                None
            } else {
                self.azure_item(&path, &base).await
            };
            let after = if kind.contains("delete") {
                None
            } else {
                self.azure_item(&path, &head).await
            };
            let _ = writeln!(out, "diff --git a/{path} b/{path}");
            out.push_str(&crate::textdiff::unified(
                &path,
                before.as_deref(),
                after.as_deref(),
            ));
        }
        Ok(out)
    }

    /// Review and general comments that may need an answer (GitLab: unresolved discussions).
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    #[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
    pub async fn comments(&self, number: u64) -> Result<Vec<Comment>, ForgeError> {
        let (prs, repo) = (self.prs(), self.repo());
        let mut out = Vec::new();
        let gh_like = |v: &Value, inline: bool| Comment {
            id: n(v, "/id").map(|i| i.to_string()).unwrap_or_default(),
            author: s(v, "/user/login"),
            body: s(v, "/body"),
            path: inline.then(|| s(v, "/path")).filter(|p| !p.is_empty()),
            line: n(v, "/line")
                .or_else(|| n(v, "/original_line"))
                .or_else(|| n(v, "/position"))
                .filter(|_| inline),
        };
        match self.remote.kind {
            Kind::Github => {
                let rc = self
                    .json(
                        Method::GET,
                        &format!("{prs}/{number}/comments?per_page=100"),
                        None,
                    )
                    .await?;
                // Replies belong to their thread; answer the thread's first comment.
                out.extend(
                    items(&rc)
                        .iter()
                        .filter(|c| c.get("in_reply_to_id").is_none_or(Value::is_null))
                        .map(|c| gh_like(c, true)),
                );
                let ic = self
                    .json(
                        Method::GET,
                        &format!("{repo}/issues/{number}/comments?per_page=100"),
                        None,
                    )
                    .await?;
                out.extend(items(&ic).iter().map(|c| gh_like(c, false)));
            }
            Kind::Gitea => {
                let reviews = self
                    .json(Method::GET, &format!("{prs}/{number}/reviews"), None)
                    .await?;
                for r in items(&reviews) {
                    let Some(id) = n(&r, "/id") else { continue };
                    let cs = self
                        .json(
                            Method::GET,
                            &format!("{prs}/{number}/reviews/{id}/comments"),
                            None,
                        )
                        .await?;
                    out.extend(items(&cs).iter().map(|c| gh_like(c, true)));
                }
                let ic = self
                    .json(
                        Method::GET,
                        &format!("{repo}/issues/{number}/comments"),
                        None,
                    )
                    .await?;
                out.extend(items(&ic).iter().map(|c| gh_like(c, false)));
            }
            Kind::Gitlab => {
                let ds = self
                    .json(
                        Method::GET,
                        &format!("{prs}/{number}/discussions?per_page=100"),
                        None,
                    )
                    .await?;
                for d in items(&ds) {
                    let Some(first) = d.pointer("/notes/0") else {
                        continue;
                    };
                    if first.get("system").and_then(Value::as_bool) == Some(true)
                        || first.get("resolved").and_then(Value::as_bool) == Some(true)
                    {
                        continue;
                    }
                    out.push(Comment {
                        id: s(&d, "/id"),
                        author: s(first, "/author/username"),
                        body: s(first, "/body"),
                        path: Some(s(first, "/position/new_path")).filter(|p| !p.is_empty()),
                        line: n(first, "/position/new_line"),
                    });
                }
            }
            Kind::Bitbucket => {
                let cs = self
                    .json(
                        Method::GET,
                        &format!("{prs}/{number}/comments?pagelen=100"),
                        None,
                    )
                    .await?;
                out.extend(
                    items(&cs)
                        .iter()
                        .filter(|c| {
                            c.get("deleted").and_then(Value::as_bool) != Some(true)
                                && c.get("parent").is_none_or(Value::is_null)
                        })
                        .map(|c| Comment {
                            id: n(c, "/id").map(|i| i.to_string()).unwrap_or_default(),
                            author: s(c, "/user/display_name"),
                            body: s(c, "/content/raw"),
                            path: Some(s(c, "/inline/path")).filter(|p| !p.is_empty()),
                            line: n(c, "/inline/to"),
                        }),
                );
            }
            Kind::BitbucketServer => {
                let a = self
                    .json(
                        Method::GET,
                        &format!("{prs}/{number}/activities?limit=100"),
                        None,
                    )
                    .await?;
                out.extend(bbs::comments(&a));
            }
            Kind::Azure => {
                let ts = self
                    .json(Method::GET, &format!("{prs}/{number}/threads"), None)
                    .await?;
                out.extend(azure::comments(&ts));
            }
        }
        Ok(out)
    }

    /// Post a review: `body` as the summary and `notes` on their lines. Notes the forge refuses
    /// inline (a line outside the diff) are folded into the summary rather than lost.
    ///
    /// # Errors
    /// HTTP, network and guard errors of the summary.
    #[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
    pub async fn post_review(
        &self,
        number: u64,
        body: &str,
        notes: &[InlineNote],
    ) -> Result<ReviewPosted, ForgeError> {
        let prs = self.prs();
        let folded = |body: &str, notes: &[&InlineNote]| {
            let mut b = body.to_owned();
            for x in notes {
                let _ = write!(b, "\n\n**{}:{}**\n{}", x.path, x.line, x.body);
            }
            b
        };
        match self.remote.kind {
            Kind::Github | Kind::Gitea => {
                let comments: Vec<Value> = notes
                    .iter()
                    .map(|x| {
                        if self.remote.kind == Kind::Github {
                            json!({ "path": x.path, "line": x.line, "side": "RIGHT", "body": x.body })
                        } else {
                            json!({ "path": x.path, "new_position": x.line, "body": x.body })
                        }
                    })
                    .collect();
                let path = format!("{prs}/{number}/reviews");
                let full = json!({ "body": body, "event": "COMMENT", "comments": comments });
                match self.json(Method::POST, &path, Some(&full)).await {
                    Ok(_) => Ok(ReviewPosted {
                        inline: notes.len(),
                        folded: 0,
                    }),
                    Err(ForgeError::Http { status: 422, .. }) if !notes.is_empty() => {
                        let all: Vec<&InlineNote> = notes.iter().collect();
                        let only = json!({ "body": folded(body, &all), "event": "COMMENT" });
                        self.json(Method::POST, &path, Some(&only)).await?;
                        Ok(ReviewPosted {
                            inline: 0,
                            folded: notes.len(),
                        })
                    }
                    Err(e) => Err(e),
                }
            }
            Kind::Gitlab => {
                let mr = self
                    .json(Method::GET, &format!("{prs}/{number}"), None)
                    .await?;
                let mut refused = Vec::new();
                for x in notes {
                    let pos = json!({
                        "body": x.body,
                        "position": {
                            "position_type": "text",
                            "base_sha": s(&mr, "/diff_refs/base_sha"),
                            "start_sha": s(&mr, "/diff_refs/start_sha"),
                            "head_sha": s(&mr, "/diff_refs/head_sha"),
                            "new_path": x.path, "old_path": x.path, "new_line": x.line,
                        }
                    });
                    if self
                        .json(
                            Method::POST,
                            &format!("{prs}/{number}/discussions"),
                            Some(&pos),
                        )
                        .await
                        .is_err()
                    {
                        refused.push(x);
                    }
                }
                let note = json!({ "body": folded(body, &refused) });
                self.json(Method::POST, &format!("{prs}/{number}/notes"), Some(&note))
                    .await?;
                Ok(ReviewPosted {
                    inline: notes.len() - refused.len(),
                    folded: refused.len(),
                })
            }
            Kind::Bitbucket => {
                let path = format!("{prs}/{number}/comments");
                let mut refused = Vec::new();
                for x in notes {
                    let c = json!({ "content": { "raw": x.body }, "inline": { "path": x.path, "to": x.line } });
                    if self.json(Method::POST, &path, Some(&c)).await.is_err() {
                        refused.push(x);
                    }
                }
                let c = json!({ "content": { "raw": folded(body, &refused) } });
                self.json(Method::POST, &path, Some(&c)).await?;
                Ok(ReviewPosted {
                    inline: notes.len() - refused.len(),
                    folded: refused.len(),
                })
            }
            Kind::BitbucketServer => {
                let path = format!("{prs}/{number}/comments");
                let mut refused = Vec::new();
                for x in notes {
                    let c = bbs::comment(&x.body, Some(x));
                    if self.json(Method::POST, &path, Some(&c)).await.is_err() {
                        refused.push(x);
                    }
                }
                let c = bbs::comment(&folded(body, &refused), None);
                self.json(Method::POST, &path, Some(&c)).await?;
                Ok(ReviewPosted {
                    inline: notes.len() - refused.len(),
                    folded: refused.len(),
                })
            }
            Kind::Azure => {
                let path = format!("{prs}/{number}/threads");
                let mut refused = Vec::new();
                for x in notes {
                    let t = azure::thread(&x.body, Some(x));
                    if self.json(Method::POST, &path, Some(&t)).await.is_err() {
                        refused.push(x);
                    }
                }
                let summary = azure::thread(&folded(body, &refused), None);
                self.json(Method::POST, &path, Some(&summary)).await?;
                Ok(ReviewPosted {
                    inline: notes.len() - refused.len(),
                    folded: refused.len(),
                })
            }
        }
    }

    /// Answer `c` in its thread (Gitea has no reply API: a quoting PR comment).
    ///
    /// # Errors
    /// HTTP, network and guard errors.
    pub async fn reply(&self, number: u64, c: &Comment, body: &str) -> Result<(), ForgeError> {
        let (prs, repo) = (self.prs(), self.repo());
        let (path, payload) = match self.remote.kind {
            Kind::Github if c.path.is_some() => (
                format!("{prs}/{number}/comments/{}/replies", c.id),
                json!({ "body": body }),
            ),
            Kind::Github => (
                format!("{repo}/issues/{number}/comments"),
                json!({ "body": body }),
            ),
            Kind::Gitea => {
                let quote = c.body.lines().next().unwrap_or_default();
                (
                    format!("{repo}/issues/{number}/comments"),
                    json!({ "body": format!("> {quote}\n\n{body}") }),
                )
            }
            Kind::Gitlab => (
                format!("{prs}/{number}/discussions/{}/notes", c.id),
                json!({ "body": body }),
            ),
            Kind::Bitbucket => (
                format!("{prs}/{number}/comments"),
                json!({
                    "content": { "raw": body },
                    "parent": { "id": c.id.parse::<u64>().unwrap_or_default() },
                }),
            ),
            Kind::BitbucketServer => (format!("{prs}/{number}/comments"), bbs::reply(&c.id, body)),
            Kind::Azure => (
                format!("{prs}/{number}/threads/{}/comments", c.id),
                azure::reply(body),
            ),
        };
        self.json(Method::POST, &path, Some(&payload))
            .await
            .map(|_| ())
    }
}
