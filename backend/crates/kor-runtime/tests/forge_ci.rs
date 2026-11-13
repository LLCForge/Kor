//! §28.7 against wiremock servers: CI checks and failing-job logs (GitHub, GitLab, Bitbucket
//! Server), issues as tasks, releases, Bitbucket Server pull requests, and the features a forge
//! has no API for.

use std::collections::BTreeMap;

use kor_runtime::forge::{self, Forge, ForgeError, Kind, PrRequest, Remote};
use kor_secrets::SecretString;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn forge(server: &MockServer, kind: &str, repo_path: &str) -> Forge {
    let hosts: BTreeMap<String, String> = [("127.0.0.1".to_owned(), kind.to_owned())].into();
    let remote = Remote::parse(&format!("{}/{repo_path}", server.uri()), &hosts).unwrap();
    Forge::new(remote, SecretString::from("tok".to_owned())).unwrap()
}

fn ok(v: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

#[tokio::test]
async fn github_checks_logs_issue_and_release() {
    let s = MockServer::start().await;
    Mock::given(path(format!("/api/v3/repos/o/r/commits/{SHA}/check-runs")))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ok(json!({"check_runs": [
            {"id": 11, "name": "test", "status": "completed", "conclusion": "failure", "html_url": "https://ci/11"},
            {"id": 12, "name": "lint", "status": "completed", "conclusion": "success"}]})))
        .mount(&s)
        .await;
    Mock::given(path(format!("/api/v3/repos/o/r/commits/{SHA}/status")))
        .respond_with(ok(
            json!({"state": "pending", "statuses": [{"context": "deploy", "state": "pending"}]}),
        ))
        .mount(&s)
        .await;
    Mock::given(path("/api/v3/repos/o/r/actions/jobs/11/logs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "error[E0308]: mismatched types\nGITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        ))
        .mount(&s)
        .await;
    Mock::given(path("/api/v3/repos/o/r/issues/7"))
        .respond_with(ok(
            json!({"number": 7, "title": "Crash on start", "body": "Steps…",
            "html_url": "https://gh/o/r/issues/7", "state": "open", "user": {"login": "ada"}}),
        ))
        .mount(&s)
        .await;
    Mock::given(path("/api/v3/repos/o/r/issues/7/comments"))
        .respond_with(ok(json!([{"user": {"login": "bob"}, "body": "same here"}])))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/o/r/releases"))
        .and(body_partial_json(
            json!({"tag_name": "v1.2.0", "target_commitish": SHA, "draft": true}),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"html_url": "https://gh/o/r/releases/v1.2.0"})),
        )
        .mount(&s)
        .await;
    let f = forge(&s, "github", "o/r.git");
    let checks = f.checks(SHA).await.unwrap();
    assert_eq!(checks.len(), 3);
    let failed: Vec<_> = checks.into_iter().filter(forge::Check::failed).collect();
    assert_eq!(failed[0].name, "test");
    let log = f.check_log(&failed[0]).await.unwrap();
    assert!(
        log.contains("E0308") && !log.contains("ghp_abcdefghij"),
        "{log}"
    );
    let goal = forge::ci_goal("feat/x", SHA, &[(failed[0].clone(), Some(log))]);
    assert!(goal.contains("### test (https://ci/11)"));
    assert!(
        f.checks("main").await.is_err(),
        "a branch name is not a commit hash"
    );

    let i = f.issue(7).await.unwrap();
    assert_eq!((i.title.as_str(), i.comments.len()), ("Crash on start", 1));
    assert!(forge::issue_goal(&i).contains("- bob: same here"));

    let url = f
        .publish_release("v1.2.0", SHA, "- fix", true)
        .await
        .unwrap();
    assert_eq!(url, "https://gh/o/r/releases/v1.2.0");
}

#[tokio::test]
async fn gitlab_pipeline_jobs_and_trace() {
    let s = MockServer::start().await;
    Mock::given(path("/api/v4/projects/g%2Fp/pipelines"))
        .and(query_param("sha", SHA))
        .respond_with(ok(json!([{"id": 99}])))
        .mount(&s)
        .await;
    Mock::given(path("/api/v4/projects/g%2Fp/pipelines/99/jobs"))
        .respond_with(ok(
            json!([{"id": 5, "name": "rspec", "status": "failed", "web_url": "w"},
                                {"id": 6, "name": "build", "status": "success"}]),
        ))
        .mount(&s)
        .await;
    Mock::given(path("/api/v4/projects/g%2Fp/jobs/5/trace"))
        .and(header("private-token", "tok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("1 example, 1 failure"))
        .mount(&s)
        .await;
    let f = forge(&s, "gitlab", "g/p.git");
    let checks = f.checks(SHA).await.unwrap();
    let bad = checks.iter().find(|c| c.failed()).unwrap();
    assert_eq!(f.check_log(bad).await.unwrap(), "1 example, 1 failure");
}

#[tokio::test]
async fn bitbucket_server_prs_statuses_and_missing_apis() {
    let s = MockServer::start().await;
    let base = "/rest/api/1.0/projects/PROJ/repos/app";
    Mock::given(method("POST"))
        .and(path(format!("{base}/pull-requests")))
        .and(header("authorization", "Bearer tok"))
        .and(body_partial_json(
            json!({"fromRef": {"id": "refs/heads/feat"}, "toRef": {"id": "refs/heads/main"}}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"id": 3, "title": "T", "state": "OPEN",
            "fromRef": {"displayId": "feat"}, "toRef": {"displayId": "main"},
            "links": {"self": [{"href": "https://bb/pr/3"}]}}),
        ))
        .mount(&s)
        .await;
    Mock::given(path(format!("{base}/pull-requests/3/activities")))
        .respond_with(ok(json!({"values": [{"action": "COMMENTED",
            "comment": {"id": 8, "text": "why?", "author": {"name": "bob"}},
            "commentAnchor": {"path": "src/a.rs", "line": 4}}]})))
        .mount(&s)
        .await;
    Mock::given(path(format!("/rest/build-status/1.0/commits/{SHA}")))
        .respond_with(ok(json!({"values": [{"state": "FAILED", "key": "ci", "name": "Build", "url": "https://ci/1"}]})))
        .mount(&s)
        .await;
    let f = forge(&s, "bitbucket-server", "scm/PROJ/app.git");
    assert_eq!(f.remote.kind, Kind::BitbucketServer);
    let pr = f
        .create(&PrRequest {
            head: "feat".into(),
            base: "main".into(),
            title: "T".into(),
            body: "B".into(),
            draft: false,
        })
        .await
        .unwrap();
    assert_eq!((pr.number, pr.forge.as_str()), (3, "bitbucket-server"));
    let c = f.comments(3).await.unwrap();
    assert_eq!(
        (c[0].path.as_deref(), c[0].line),
        (Some("src/a.rs"), Some(4))
    );
    let checks = f.checks(SHA).await.unwrap();
    assert!(checks[0].failed());
    assert_eq!(f.check_log(&checks[0]).await, None);
    assert!(matches!(f.issue(1).await, Err(ForgeError::NotSupported(_))));
    assert!(matches!(
        f.publish_release("v1", SHA, "x", false).await,
        Err(ForgeError::NotSupported(_))
    ));
}

#[test]
fn forge_config_with_a_bad_ca_file_fails_cleanly() {
    let hosts: BTreeMap<String, String> = [("git.corp".to_owned(), "gitlab".to_owned())].into();
    let remote = Remote::parse("https://git.corp/g/p.git", &hosts).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("ca.pem");
    std::fs::write(&bad, "not a certificate").unwrap();
    let cfg = kor_config::Forge {
        ca_cert: Some(dir.path().join("missing.pem")),
        ..kor_config::Forge::default()
    };
    assert!(matches!(
        Forge::with_config(remote.clone(), SecretString::from("t".to_owned()), &cfg),
        Err(ForgeError::Network(_))
    ));
    let cfg = kor_config::Forge {
        proxy: Some("http://proxy.corp:3128".into()),
        ..kor_config::Forge::default()
    };
    assert!(Forge::with_config(remote, SecretString::from("t".to_owned()), &cfg).is_ok());
}
