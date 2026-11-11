//! Azure DevOps (§28.7, ADR 0063) against a wiremock server: remote detection, create, list,
//! get, the diff assembled from iterations and items, comment threads, a review with an inline
//! note the server refuses (folded into the summary), replies, PAT Basic auth and api-version.

use std::collections::BTreeMap;

use kor_runtime::forge::{Comment, Forge, InlineNote, Kind, PrRequest, Remote};
use kor_secrets::SecretString;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BASE: &str = "/org/proj/_apis/git/repositories/repo";

fn forge(server: &MockServer) -> Forge {
    let hosts: BTreeMap<String, String> = [("127.0.0.1".to_owned(), "azure".to_owned())].into();
    let remote = Remote::parse(&format!("{}/org/proj/_git/repo", server.uri()), &hosts).unwrap();
    assert_eq!(remote.kind, Kind::Azure);
    assert_eq!(remote.api, format!("{}{BASE}", server.uri()));
    Forge::new(remote, SecretString::from("pat".to_owned())).unwrap()
}

fn ok(v: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

fn pr_json() -> serde_json::Value {
    json!({ "pullRequestId": 9, "title": "Add x", "status": "active", "isDraft": false,
        "sourceRefName": "refs/heads/feat/x", "targetRefName": "refs/heads/main",
        "createdBy": { "displayName": "Me", "uniqueName": "me@corp" },
        "repository": { "webUrl": "https://dev.azure.com/org/proj/_git/repo" } })
}

#[test]
fn public_remote_forms() {
    let none = BTreeMap::new();
    let r = Remote::parse("https://org@dev.azure.com/org/proj/_git/repo", &none).unwrap();
    assert_eq!(
        (r.kind, r.owner.as_str(), r.repo.as_str()),
        (Kind::Azure, "org/proj", "repo")
    );
    assert_eq!(
        r.api,
        "https://dev.azure.com/org/proj/_apis/git/repositories/repo"
    );
    let r = Remote::parse("git@ssh.dev.azure.com:v3/org/proj/repo", &none).unwrap();
    assert_eq!(
        r.api,
        "https://dev.azure.com/org/proj/_apis/git/repositories/repo"
    );
    let r = Remote::parse("https://org.visualstudio.com/proj/_git/repo", &none).unwrap();
    assert_eq!(
        r.api,
        "https://org.visualstudio.com/proj/_apis/git/repositories/repo"
    );
    assert_eq!(Kind::parse("azure-devops"), Some(Kind::Azure));
    assert_eq!(Kind::for_auth_id("azure"), Some(Kind::Azure));
}

#[tokio::test]
async fn pull_request_flow() {
    let s = MockServer::start().await;
    let f = forge(&s);
    // PAT auth is Basic with an empty user: base64(":pat").
    let auth = || header("authorization", "Basic OnBhdA==");
    let v = || query_param("api-version", "7.1");
    Mock::given(method("POST"))
        .and(path(format!("{BASE}/pullrequests")))
        .and(auth())
        .and(v())
        .and(body_partial_json(
            json!({ "sourceRefName": "refs/heads/feat/x", "targetRefName": "refs/heads/main",
            "title": "Add x", "description": "Body", "isDraft": true }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(pr_json()))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{BASE}/pullrequests")))
        .and(v())
        .and(query_param("searchCriteria.status", "active"))
        .respond_with(ok(json!({ "value": [pr_json()], "count": 1 })))
        .expect(2)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{BASE}/pullrequests/9")))
        .and(v())
        .respond_with(ok(pr_json()))
        .expect(1)
        .mount(&s)
        .await;

    let pr = f
        .create(&PrRequest {
            head: "feat/x".into(),
            base: "main".into(),
            title: "Add x".into(),
            body: "Body".into(),
            draft: true,
        })
        .await
        .unwrap();
    assert_eq!((pr.number, pr.forge.as_str()), (9, "azure"));
    assert_eq!(
        pr.url,
        "https://dev.azure.com/org/proj/_git/repo/pullrequest/9"
    );
    let list = f.list().await.unwrap();
    assert_eq!(list[0].head, "feat/x");
    assert_eq!(
        f.for_branch("feat/x").await.unwrap().map(|p| p.number),
        Some(9)
    );
    assert_eq!(f.get(9).await.unwrap().author, "me@corp");
}

#[tokio::test]
async fn diff_from_iterations_and_items() {
    let s = MockServer::start().await;
    let f = forge(&s);
    Mock::given(method("GET")).and(path(format!("{BASE}/pullrequests/9/iterations")))
        .respond_with(ok(json!({ "value": [
            { "id": 1, "sourceRefCommit": { "commitId": "h1" }, "commonRefCommit": { "commitId": "b1" } },
            { "id": 2, "sourceRefCommit": { "commitId": "h2" }, "commonRefCommit": { "commitId": "b2" } }
        ] }))).mount(&s).await;
    Mock::given(method("GET"))
        .and(path(format!("{BASE}/pullrequests/9/iterations/2/changes")))
        .respond_with(ok(json!({ "changeEntries": [
            { "item": { "path": "/src/a.rs" }, "changeType": "edit" },
            { "item": { "path": "/src/new.rs" }, "changeType": "add" },
            { "item": { "path": "/src", "isFolder": true }, "changeType": "edit" }
        ] })))
        .mount(&s)
        .await;
    for (p, commit, content) in [
        ("/src/a.rs", "b2", "fn a() {}\n"),
        ("/src/a.rs", "h2", "fn a() { b(); }\n"),
        ("/src/new.rs", "h2", "fn b() {}\n"),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("{BASE}/items")))
            .and(query_param("path", p))
            .and(query_param("versionDescriptor.version", commit))
            .respond_with(ok(json!({ "path": p, "content": content })))
            .mount(&s)
            .await;
    }
    let d = f.diff(9).await.unwrap();
    assert!(d.contains("diff --git a/src/a.rs b/src/a.rs"), "{d}");
    assert!(d.contains("-fn a() {}\n+fn a() { b(); }"), "{d}");
    assert!(d.contains("--- /dev/null\n+++ b/src/new.rs"), "{d}");
}

#[tokio::test]
async fn threads_review_and_reply() {
    let s = MockServer::start().await;
    let f = forge(&s);
    Mock::given(method("GET")).and(path(format!("{BASE}/pullrequests/9/threads")))
        .respond_with(ok(json!({ "value": [
            { "id": 31, "status": "active", "threadContext": { "filePath": "/src/a.rs", "rightFileStart": { "line": 2 } },
              "comments": [{ "id": 1, "content": "why?", "author": { "uniqueName": "rev@corp" }, "commentType": "text" }] },
            { "id": 32, "status": "closed", "comments": [{ "id": 1, "content": "ok" }] }
        ] }))).expect(1).mount(&s).await;
    // The inline note on line 99 is refused; it is folded into the summary thread.
    Mock::given(method("POST")).and(path(format!("{BASE}/pullrequests/9/threads")))
        .and(body_partial_json(json!({ "threadContext": { "filePath": "/src/a.rs", "rightFileStart": { "line": 99 } } })))
        .respond_with(ResponseTemplate::new(400).set_body_string("line out of range")).expect(1).mount(&s).await;
    Mock::given(method("POST"))
        .and(path(format!("{BASE}/pullrequests/9/threads")))
        .and(body_partial_json(
            json!({ "status": 1, "comments": [{ "parentCommentId": 0, "commentType": 1 }] }),
        ))
        .respond_with(ok(json!({ "id": 40 })))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{BASE}/pullrequests/9/threads/31/comments")))
        .and(body_partial_json(
            json!({ "content": "fixed", "parentCommentId": 1 }),
        ))
        .respond_with(ok(json!({ "id": 2 })))
        .expect(1)
        .mount(&s)
        .await;

    let cs = f.comments(9).await.unwrap();
    assert_eq!(cs.len(), 1);
    assert_eq!(
        (cs[0].path.as_deref(), cs[0].line, cs[0].author.as_str()),
        (Some("src/a.rs"), Some(2), "rev@corp")
    );
    let posted = f
        .post_review(
            9,
            "summary",
            &[InlineNote {
                path: "src/a.rs".into(),
                line: 99,
                body: "n".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!((posted.inline, posted.folded), (0, 1));
    let c: &Comment = &cs[0];
    f.reply(9, c, "fixed").await.unwrap();
}
