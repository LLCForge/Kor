//! Universal Git (§8.8, ADR 0040) against wiremock forges: the request shapes of create, list,
//! diff, comments, review submission and replies on GitHub, GitLab, Gitea/Forgejo and Bitbucket;
//! a push to a local bare repository; the description from a scripted model; the network guard.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_core::{FinishReason, ProviderError, StreamEvent};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::forge::{self, Forge, ForgeError, InlineNote, Kind, PrRequest, Remote};
use kor_secrets::SecretString;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn forge_for(server: &MockServer, kind: &str, repo: &str) -> Forge {
    let hosts: BTreeMap<String, String> = [("127.0.0.1".to_owned(), kind.to_owned())].into();
    let remote = Remote::parse(&format!("{}/{repo}.git", server.uri()), &hosts).unwrap();
    Forge::new(remote, SecretString::from("tok".to_owned())).unwrap()
}

fn req(draft: bool) -> PrRequest {
    PrRequest {
        head: "feat/x".into(),
        base: "main".into(),
        title: "Add x".into(),
        body: "Body".into(),
        draft,
    }
}

fn ok(v: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

async fn mount(server: &MockServer, m: Mock) {
    m.expect(1).mount(server).await;
}

#[tokio::test]
async fn github_pull_request_flow() {
    let s = MockServer::start().await;
    let f = forge_for(&s, "github", "o/r");
    let pr = json!({ "number": 5, "title": "Add x", "html_url": "https://gh/o/r/pull/5",
        "head": { "ref": "feat/x" }, "base": { "ref": "main" }, "user": { "login": "me" },
        "state": "open", "draft": true });
    let base = "/api/v3/repos/o/r";
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/pulls")))
        .and(header("authorization", "Bearer tok"))
        .and(header("accept", "application/vnd.github+json"))
        .and(body_partial_json(json!({ "title": "Add x", "head": "feat/x", "base": "main", "body": "Body", "draft": true })))
        .respond_with(ResponseTemplate::new(201).set_body_json(pr.clone()))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls")))
        .and(query_param("state", "open"))
        .respond_with(ok(json!([pr])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls/5")))
        .and(header("accept", "application/vnd.github.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("diff --git a/a b/a\n"))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls/5/comments")))
        .respond_with(ok(json!([
            { "id": 11, "user": { "login": "rev" }, "body": "fix this", "path": "src/a.rs", "line": 3 },
            { "id": 12, "user": { "login": "me" }, "body": "reply", "path": "src/a.rs", "line": 3, "in_reply_to_id": 11 }
        ])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/issues/5/comments")))
        .respond_with(ok(json!([{ "id": 13, "user": { "login": "rev" }, "body": "overall?" }])))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/pulls/5/reviews")))
        .and(body_partial_json(json!({ "event": "COMMENT", "body": "sum",
            "comments": [{ "path": "src/a.rs", "line": 3, "side": "RIGHT", "body": "n" }] })))
        .respond_with(ok(json!({ "id": 1 })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/pulls/5/comments/11/replies")))
        .and(body_partial_json(json!({ "body": "done" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 14 })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/issues/5/comments")))
        .and(body_partial_json(json!({ "body": "yes" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 15 })))).await;

    let r = f.create(&req(true)).await.unwrap();
    assert_eq!((r.number, r.forge.as_str(), r.url.as_str()), (5, "github", "https://gh/o/r/pull/5"));
    let list = f.list().await.unwrap();
    assert_eq!((list[0].head.as_str(), list[0].draft), ("feat/x", true));
    assert!(f.diff(5).await.unwrap().starts_with("diff --git"));
    let cs = f.comments(5).await.unwrap();
    assert_eq!(cs.len(), 2, "replies are part of their thread: {cs:?}");
    assert_eq!((cs[0].path.as_deref(), cs[0].line), (Some("src/a.rs"), Some(3)));
    assert_eq!(cs[1].path, None);
    let note = InlineNote { path: "src/a.rs".into(), line: 3, body: "n".into() };
    let posted = f.post_review(5, "sum", &[note]).await.unwrap();
    assert_eq!((posted.inline, posted.folded), (1, 0));
    f.reply(5, &cs[0], "done").await.unwrap();
    f.reply(5, &cs[1], "yes").await.unwrap();
}

#[tokio::test]
async fn github_folds_refused_inline_notes() {
    let s = MockServer::start().await;
    let f = forge_for(&s, "github", "o/r");
    let p = "/api/v3/repos/o/r/pulls/5/reviews";
    mount(&s, Mock::given(method("POST")).and(path(p))
        .and(body_partial_json(json!({ "comments": [{ "path": "a.rs" }] })))
        .respond_with(ResponseTemplate::new(422).set_body_string("line must be part of the diff"))).await;
    // The retry has no `comments` and carries the note in the body.
    Mock::given(method("POST")).and(path(p))
        .and(body_partial_json(json!({ "event": "COMMENT" })))
        .respond_with(ok(json!({ "id": 2 })))
        .mount(&s).await;
    let note = InlineNote { path: "a.rs".into(), line: 99, body: "far away".into() };
    let posted = f.post_review(5, "sum", &[note]).await.unwrap();
    assert_eq!((posted.inline, posted.folded), (0, 1));
    let reqs = s.received_requests().await.unwrap();
    let last: serde_json::Value = serde_json::from_slice(&reqs.last().unwrap().body).unwrap();
    assert!(last.get("comments").is_none());
    assert!(last["body"].as_str().unwrap().contains("**a.rs:99**\nfar away"));
}

#[tokio::test]
async fn gitlab_merge_request_flow() {
    let s = MockServer::start().await;
    let f = forge_for(&s, "gitlab", "grp/sub/proj");
    let base = "/api/v4/projects/grp%2Fsub%2Fproj/merge_requests";
    let mr = json!({ "iid": 8, "title": "Draft: Add x", "web_url": "https://gl/mr/8", "source_branch": "feat/x",
        "target_branch": "main", "author": { "username": "me" }, "state": "opened", "draft": true,
        "diff_refs": { "base_sha": "b", "start_sha": "s", "head_sha": "h" } });
    mount(&s, Mock::given(method("POST")).and(path(base))
        .and(header("private-token", "tok"))
        .and(body_partial_json(json!({ "source_branch": "feat/x", "target_branch": "main", "title": "Draft: Add x", "description": "Body" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(mr.clone()))).await;
    mount(&s, Mock::given(method("GET")).and(path(base)).and(query_param("state", "opened"))
        .respond_with(ok(json!([mr.clone()])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/8/changes")))
        .respond_with(ok(json!({ "changes": [{ "old_path": "a.rs", "new_path": "a.rs", "diff": "@@ -1 +1 @@\n-a\n+b\n" }] })))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/8/discussions")))
        .respond_with(ok(json!([
            { "id": "d1", "notes": [{ "author": { "username": "rev" }, "body": "why?", "position": { "new_path": "a.rs", "new_line": 1 } }] },
            { "id": "d2", "notes": [{ "system": true, "body": "added 1 commit" }] },
            { "id": "d3", "notes": [{ "resolved": true, "body": "old" }] }
        ])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/8"))).respond_with(ok(mr))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/8/discussions")))
        .and(body_partial_json(json!({ "body": "n", "position": { "position_type": "text", "base_sha": "b", "head_sha": "h", "start_sha": "s", "new_path": "a.rs", "new_line": 1 } })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "d4" })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/8/notes")))
        .and(body_partial_json(json!({ "body": "sum" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 1 })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/8/discussions/d1/notes")))
        .and(body_partial_json(json!({ "body": "because" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 2 })))).await;

    let r = f.create(&req(true)).await.unwrap();
    assert_eq!((r.number, r.forge.as_str()), (8, "gitlab"));
    assert!(f.list().await.unwrap()[0].draft);
    let d = f.diff(8).await.unwrap();
    assert!(d.starts_with("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@"), "{d}");
    let cs = f.comments(8).await.unwrap();
    assert_eq!(cs.len(), 1);
    assert_eq!((cs[0].id.as_str(), cs[0].line), ("d1", Some(1)));
    let note = InlineNote { path: "a.rs".into(), line: 1, body: "n".into() };
    assert_eq!(f.post_review(8, "sum", &[note]).await.unwrap().inline, 1);
    f.reply(8, &cs[0], "because").await.unwrap();
}

#[tokio::test]
async fn gitea_pull_request_flow() {
    let s = MockServer::start().await;
    let f = forge_for(&s, "forgejo", "o/r");
    let base = "/api/v1/repos/o/r";
    let pr = json!({ "number": 3, "title": "WIP: Add x", "html_url": "https://cb/o/r/pulls/3",
        "head": { "ref": "feat/x" }, "base": { "ref": "main" }, "user": { "login": "me" }, "state": "open" });
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/pulls")))
        .and(header("authorization", "token tok"))
        .and(body_partial_json(json!({ "head": "feat/x", "base": "main", "title": "WIP: Add x", "body": "Body" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(pr.clone()))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls"))).and(query_param("limit", "50"))
        .respond_with(ok(json!([pr])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls/3.diff")))
        .respond_with(ResponseTemplate::new(200).set_body_string("diff --git a/a b/a\n"))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls/3/reviews")))
        .respond_with(ok(json!([{ "id": 40 }])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/pulls/3/reviews/40/comments")))
        .respond_with(ok(json!([{ "id": 41, "user": { "login": "rev" }, "body": "rename", "path": "a.go", "position": 7 }])))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/issues/3/comments")))
        .respond_with(ok(json!([])))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/pulls/3/reviews")))
        .and(body_partial_json(json!({ "event": "COMMENT", "comments": [{ "path": "a.go", "new_position": 7, "body": "n" }] })))
        .respond_with(ok(json!({ "id": 42 })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/issues/3/comments")))
        .and(body_partial_json(json!({ "body": "> rename\n\nrenamed" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 43 })))).await;

    let r = f.create(&req(true)).await.unwrap();
    assert_eq!((r.number, r.forge.as_str()), (3, "gitea"));
    assert!(f.list().await.unwrap()[0].draft, "WIP title marks a draft");
    assert!(f.diff(3).await.unwrap().contains("diff --git"));
    let cs = f.comments(3).await.unwrap();
    assert_eq!((cs.len(), cs[0].line), (1, Some(7)));
    let note = InlineNote { path: "a.go".into(), line: 7, body: "n".into() };
    assert_eq!(f.post_review(3, "sum", &[note]).await.unwrap().inline, 1);
    f.reply(3, &cs[0], "renamed").await.unwrap();
}

#[tokio::test]
async fn bitbucket_pull_request_flow() {
    let s = MockServer::start().await;
    let hosts: BTreeMap<String, String> = [("127.0.0.1".to_owned(), "bitbucket".to_owned())].into();
    let remote = Remote::parse(&format!("{}/ws/repo.git", s.uri()), &hosts).unwrap();
    // `user:app-password` tokens use Basic auth.
    let f = Forge::new(remote, SecretString::from("me:pw".to_owned())).unwrap();
    let base = "/2.0/repositories/ws/repo/pullrequests";
    let pr = json!({ "id": 2, "title": "Add x", "links": { "html": { "href": "https://bb/pr/2" } },
        "source": { "branch": { "name": "feat/x" } }, "destination": { "branch": { "name": "main" } },
        "author": { "display_name": "Me" }, "state": "OPEN", "draft": false });
    mount(&s, Mock::given(method("POST")).and(path(base))
        .and(header("authorization", "Basic bWU6cHc="))
        .and(body_partial_json(json!({ "title": "Add x", "description": "Body", "draft": false,
            "source": { "branch": { "name": "feat/x" } }, "destination": { "branch": { "name": "main" } } })))
        .respond_with(ResponseTemplate::new(201).set_body_json(pr.clone()))).await;
    mount(&s, Mock::given(method("GET")).and(path(base)).and(query_param("state", "OPEN"))
        .respond_with(ok(json!({ "values": [pr] })))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/2/diff")))
        .respond_with(ResponseTemplate::new(200).set_body_string("diff --git a/a b/a\n"))).await;
    mount(&s, Mock::given(method("GET")).and(path(format!("{base}/2/comments")))
        .respond_with(ok(json!({ "values": [
            { "id": 21, "user": { "display_name": "Rev" }, "content": { "raw": "typo" }, "inline": { "path": "a.ts", "to": 4 } },
            { "id": 22, "content": { "raw": "gone" }, "deleted": true },
            { "id": 23, "content": { "raw": "child" }, "parent": { "id": 21 } }
        ] })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/2/comments")))
        .and(body_partial_json(json!({ "inline": { "path": "a.ts", "to": 4 }, "content": { "raw": "n" } })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 24 })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/2/comments")))
        .and(body_partial_json(json!({ "content": { "raw": "sum" } })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 25 })))).await;
    mount(&s, Mock::given(method("POST")).and(path(format!("{base}/2/comments")))
        .and(body_partial_json(json!({ "parent": { "id": 21 }, "content": { "raw": "fixed" } })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 26 })))).await;

    let r = f.create(&req(false)).await.unwrap();
    assert_eq!((r.number, r.url.as_str()), (2, "https://bb/pr/2"));
    assert_eq!(f.list().await.unwrap()[0].state, "open");
    assert!(f.diff(2).await.unwrap().contains("diff --git"));
    let cs = f.comments(2).await.unwrap();
    assert_eq!(cs.len(), 1, "{cs:?}");
    let note = InlineNote { path: "a.ts".into(), line: 4, body: "n".into() };
    assert_eq!(f.post_review(2, "sum", &[note]).await.unwrap().inline, 1);
    f.reply(2, &cs[0], "fixed").await.unwrap();
}

#[tokio::test]
async fn forge_errors_keep_the_status() {
    let s = MockServer::start().await;
    let f = forge_for(&s, "github", "o/r");
    Mock::given(method("GET")).and(path("/api/v3/repos/o/r/pulls/9"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
        .mount(&s).await;
    match f.get(9).await {
        Err(ForgeError::Http { status: 404, body }) => assert_eq!(body, "Not Found"),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// git: push to a local bare repository, description from a scripted model
// ---------------------------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(["-c", "user.name=Kor Test", "-c", "user.email=kor@test", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
}

fn repo_with_branch() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let bare = tmp.path().join("origin.git");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(tmp.path(), &["init", "--bare", "-q", bare.to_str().unwrap()]);
    git(&work, &["init", "-q"]);
    git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
    std::fs::write(work.join("a.txt"), "one\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "chore: init"]);
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["checkout", "-q", "-b", "feat/pr"]);
    std::fs::write(work.join("a.txt"), "one\ntwo\n").unwrap();
    git(&work, &["commit", "-q", "-am", "feat: add two"]);
    (tmp, work)
}

#[test]
fn push_to_a_local_bare_repository() {
    let (_tmp, work) = repo_with_branch();
    assert_eq!(forge::current_branch(&work).unwrap(), "feat/pr");
    assert!(!forge::has_upstream(&work, "feat/pr"));
    forge::push(&work, "feat/pr").unwrap();
    assert!(forge::has_upstream(&work, "feat/pr"));
    assert!(forge::push(&work, "--force").is_err());
    // A local remote has no forge.
    assert!(matches!(forge::detect(&work), Err(ForgeError::NoRemote)));
}

struct Scripted {
    manifest: ProviderManifest,
    replies: Mutex<VecDeque<String>>,
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl Provider for Scripted {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(&self, req: ChatRequest, _c: CancellationToken) -> Result<EventStream, ProviderError> {
        self.seen.lock().unwrap().push(format!("{:?}", req.messages));
        let text = self.replies.lock().unwrap().pop_front().expect("unexpected model call");
        let evs = vec![
            StreamEvent::TextDelta { text },
            StreamEvent::Finish { reason: FinishReason::Stop },
        ];
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn scripted(replies: &[&str]) -> Arc<Scripted> {
    Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        replies: Mutex::new(replies.iter().map(|s| (*s).to_owned()).collect()),
        seen: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn description_from_branch_commits_and_model() {
    let (_tmp, work) = repo_with_branch();
    let (d, err) = forge::describe(None, &work, "main", "feat/pr").await.unwrap();
    assert_eq!((d.title.as_str(), err), ("add two", None));
    assert!(d.body.contains("add two"));

    let p = scripted(&["Add the second line\n\n## Summary\nNeeded two."]);
    let dynp: Arc<dyn Provider> = p.clone();
    let (d, err) = forge::describe(Some((&dynp, "m")), &work, "main", "feat/pr").await.unwrap();
    assert_eq!((d.title.as_str(), err), ("Add the second line", None));
    assert!(d.body.starts_with("## Summary"));
    let seen = p.seen.lock().unwrap().join("");
    assert!(seen.contains("feat: add two") || seen.contains("add two"), "commits reach the model");
    assert!(seen.contains("+two"), "the diff reaches the model");
}

#[tokio::test]
async fn review_runs_prechecks_and_the_persona() {
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let dir = tempfile::tempdir().unwrap();
    let persona = kor_runtime::review::persona(dir.path(), "security").unwrap();
    let diff = "+++ b/cfg.rs\n@@ -0,0 +1,2 @@\n+let k = \"AKIAIOSFODNN7EXAMPLE\";\n+run(user_input);\n";
    let p = scripted(&[r#"[{"file":"cfg.rs","line":2,"severity":"high","rule":"cwe-78","message":"shell injection"}]"#]);
    let dynp: Arc<dyn Provider> = p.clone();
    let r = kor_runtime::review::review(&l, Some((&dynp, "m")), &persona, diff).await;
    assert_eq!(r.model_error, None);
    assert_eq!(r.findings.len(), 2, "{:?}", r.findings);
    assert_eq!(r.findings[0].source, "check", "critical secret first");
    assert_eq!(r.findings[1].rule, "cwe-78");
    let sent = p.seen.lock().unwrap().join("");
    assert!(sent.contains("application security engineer"), "persona prompt sent");
    assert!(!sent.contains("AKIAIOSFODNN7EXAMPLE"), "secrets are masked before the model");
    let offline = kor_runtime::review::review(&l, None, &persona, diff).await;
    assert_eq!(offline.findings.len(), 1);
    assert!(offline.model_error.is_some());
}

/// The guard refuses public forges in offline/privacy mode before any connection, with the
/// localized refusal; local remotes still push. One test: the policy is process-global and this
/// is its own test binary.
#[tokio::test]
async fn offline_mode_refuses_public_forges() {
    kor_core::net::set_policy(kor_core::net::Policy::Offline);
    let remote = Remote::parse("git@github.com:o/r.git", &BTreeMap::new()).unwrap();
    assert_eq!(remote.kind, Kind::Github);
    let f = Forge::new(remote, SecretString::from("t".to_owned())).unwrap();
    let e = f.list().await.unwrap_err();
    assert!(matches!(&e, ForgeError::Blocked(m) if m.contains("api.github.com")), "{e:?}");
    assert!(matches!(forge::check_push("https://gitlab.com/a/b.git"), Err(ForgeError::Blocked(_))));
    assert!(forge::check_push("/srv/git/b.git").is_ok());
    kor_core::net::set_policy(kor_core::net::Policy::Open);
}
