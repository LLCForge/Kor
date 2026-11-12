//! Faz 6 acceptance test 2 (§14 item 7, ADR 0039): a git push trigger completes test → fix → PR
//! end to end. A local bare "origin" with Kor's `post-receive` hook, a clone with a `git_push`
//! trigger on `main`; pushing a commit with a failing test fires the trigger, the (scripted)
//! agent runs the test, fixes the file (each step approved over a mocked Telegram bot), reruns
//! the test green; Kor commits on a `kor/…` branch, pushes it to origin and opens the PR through
//! `forge::create_pull_request` against a mocked GitHub API.
//!
//! The forge token comes from `GITHUB_TOKEN`; tests may not set environment variables in
//! process (`unsafe`), so the test re-runs itself as a child process with it set.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::{Agent, Permissions};
use kor_config::Trigger;
use kor_core::{
    ContentPart, FinishReason, Message, PermissionMode, ProviderError, StreamEvent, Usage,
};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::triggers::{self, Status, daemon, notify, runner};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const CHILD: &str = "KOR_I3_E2E_CHILD";

struct Scripted {
    manifest: ProviderManifest,
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
    seen: Arc<Mutex<Vec<Vec<Message>>>>,
}

#[async_trait]
impl Provider for Scripted {
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
        self.seen.lock().unwrap().push(req.messages);
        let evs = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("extra model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn call(id: &str, name: &str, args: &Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: id.into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: args.to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

const SUMMARY: &str = "Fixed value.txt: the test expects 42 and the file said 41.";
const TEST: &str = "git diff --no-index --quiet expected.txt value.txt";

/// The agent the daemon runs: the real loop and tools, a scripted model.
struct Factory {
    seen: Arc<Mutex<Vec<Vec<Message>>>>,
}

#[async_trait]
impl runner::AgentFactory for Factory {
    async fn build(
        &self,
        dir: &Path,
        _project: &Path,
        _t: &Trigger,
        _job: &str,
    ) -> Result<runner::Assembled, String> {
        let turns = [
            call("c1", "Bash", &json!({ "command": TEST })),
            call("c2", "Read", &json!({ "file_path": "value.txt" })),
            call(
                "c3",
                "Write",
                &json!({ "file_path": "value.txt", "content": "42\n" }),
            ),
            call("c4", "Bash", &json!({ "command": TEST })),
            vec![
                StreamEvent::TextDelta {
                    text: SUMMARY.into(),
                },
                StreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ],
        ];
        Ok(runner::Assembled {
            agent: Agent {
                provider: Arc::new(Scripted {
                    manifest: Registry::builtin().get("local").unwrap().clone(),
                    turns: Mutex::new(turns.into()),
                    seen: self.seen.clone(),
                }),
                model: "m".into(),
                tools: default_tools(),
                permissions: Permissions::new(PermissionMode::Default, &[], &[], &[]).unwrap(),
                ctx: ToolCtx::new(dir).unwrap(),
                system: "sys".into(),
                messages: Vec::new(),
                usage: Usage::default(),
                max_turns: 10,
                max_tokens: None,
                reasoning: None,
                redactor: Redactor::default(),
                hooks: None,
                rules: Vec::new(),
                auto_compact: false,
                context_window: kor_agent::compact::DEFAULT_CONTEXT_WINDOW,
                memory_extract: false,
                memory_base: None,
                session_notes: None,
                output_budget: None,
                dream: false,
                tool_mode: kor_agent::toolcall::ToolMode::Native,
                verify: kor_agent::verify::Verify::default(),
                stats: kor_agent::repair::ToolStats::default(),
                context: kor_agent::context::Budget::default(),
                cost: None,
                shadow: None,
                repair: None,
            },
            cost: None,
            transcript: None,
        })
    }
}

/// Telegram: remembers each approval's id from `sendMessage` and presses Approve on the next
/// `getUpdates`.
#[derive(Clone, Default)]
struct Bot {
    asks: Arc<Mutex<VecDeque<String>>>,
    sent: Arc<Mutex<Vec<Value>>>,
    update: Arc<Mutex<i64>>,
}

impl Respond for Bot {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let ok = |result: Value| {
            ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": result }))
        };
        if req.url.path().ends_with("/sendMessage") {
            if let Some(cb) = body
                .pointer("/reply_markup/inline_keyboard/0/0/callback_data")
                .and_then(Value::as_str)
            {
                self.asks
                    .lock()
                    .unwrap()
                    .push_back(cb.trim_start_matches("a:").to_owned());
            }
            self.sent.lock().unwrap().push(body);
            return ok(json!({}));
        }
        if req.url.path().ends_with("/getUpdates") {
            let updates: Vec<Value> = self
                .asks
                .lock()
                .unwrap()
                .drain(..)
                .map(|id| {
                    let mut n = self.update.lock().unwrap();
                    *n += 1;
                    json!({ "update_id": *n, "callback_query": { "id": format!("q{n}"), "data": format!("a:{id}"), "message": { "chat": { "id": 4242 } } } })
                })
                .collect();
            return ok(json!(updates));
        }
        ok(json!(true))
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn git_push_trigger_runs_test_fix_and_opens_a_pr() {
    if std::env::var_os(CHILD).is_none() {
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "git_push_trigger_runs_test_fix_and_opens_a_pr",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env("GITHUB_TOKEN", "test-token")
            .env_remove("GH_TOKEN")
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && text.contains("1 passed"),
            "child run failed:\n{text}"
        );
        return;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(scenario());
}

#[allow(clippy::too_many_lines)] // one end-to-end story
async fn scenario() {
    let tmp = tempfile::tempdir().unwrap();
    let base = kor_tools::without_verbatim(&std::fs::canonicalize(tmp.path()).unwrap());
    let origin = base.join("origin.git");
    let work = base.join("work");
    let state = base.join("daemon");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let github = MockServer::start().await;
    let port = github.address().port();
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/acme/app/pulls"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "number": 7, "html_url": "https://github.example/acme/app/pull/7",
            "head": { "ref": "x" }, "base": { "ref": "main" }, "user": { "login": "kor" }, "title": "t", "state": "open"
        })))
        .expect(1)
        .mount(&github)
        .await;
    let telegram = MockServer::start().await;
    let bot = Bot::default();
    Mock::given(method("POST"))
        .respond_with(bot.clone())
        .mount(&telegram)
        .await;

    // origin + clone; fetch URL is the (mocked) GitHub repo, pushes go to the local bare repo.
    git(&origin, &["init", "--quiet", "--bare", "-b", "main"]);
    git(&work, &["init", "--quiet", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Dev"),
        ("user.email", "dev@example.com"),
        ("core.autocrlf", "false"),
    ] {
        git(&work, &["config", k, v]);
    }
    git(
        &work,
        &[
            "remote",
            "add",
            "origin",
            &format!("http://127.0.0.1:{port}/acme/app.git"),
        ],
    );
    // Forward slashes: git reads `C:\x` as an ssh host `C`.
    git(
        &work,
        &[
            "config",
            "remote.origin.pushurl",
            &origin.display().to_string().replace('\\', "/"),
        ],
    );
    std::fs::create_dir_all(work.join(".kor")).unwrap();
    std::fs::write(
        triggers::project_config(&work),
        "[forge.hosts]\n\"127.0.0.1\" = \"github\"\n\n[[triggers]]\nname = \"ci\"\nevent = \"git_push\"\nbranch = \"main\"\ntask = \"run the tests, fix failures, open a PR\"\npr = true\n",
    )
    .unwrap();
    std::fs::write(work.join("value.txt"), "41\n").unwrap();
    std::fs::write(work.join("expected.txt"), "41\n").unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "--quiet", "-m", "init"]);
    git(&work, &["push", "--quiet", "origin", "main"]);

    // Consent step of a real setup: `kor triggers hook install` in origin, project watched.
    let hook = triggers::hook::install(&origin, &daemon::inbox(&state)).unwrap();
    assert!(hook.ends_with("post-receive"));
    triggers::register(&state, &work).unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut tg = notify::Telegram::new("BOT".into(), "4242".into());
    tg.api = telegram.uri();
    tg.wait_secs = 0;
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let running = daemon::start(daemon::Options {
        l,
        dir: state.clone(),
        user_config: None,
        listen: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        approval_timeout: Duration::from_secs(60),
        factory: Arc::new(Factory { seen: seen.clone() }),
        notifiers: vec![Box::new(tg)],
        secrets: Arc::new(|_| None),
        tick: Duration::from_millis(100),
        routines: false,
    })
    .await
    .unwrap();
    assert!(daemon::running_in(&state));

    // A commit with a failing test, pushed to main.
    std::fs::write(work.join("expected.txt"), "42\n").unwrap();
    git(&work, &["commit", "--quiet", "-am", "test: expect 42"]);
    let pushed = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "--quiet", "origin", "main"]);

    let mut job = None;
    for _ in 0..600 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(j) = triggers::history(&state, Some("ci"), 1).into_iter().next()
            && matches!(j.status, Status::Done | Status::Failed)
        {
            job = Some(j);
            break;
        }
    }
    // The job is saved as done before the result is broadcast: wait for the message before stopping.
    for _ in 0..100 {
        if bot
            .sent
            .lock()
            .unwrap()
            .iter()
            .any(|m| m["text"].as_str().is_some_and(|t| t.contains("pull/7")))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    running.stop().await;
    let job = job.expect("the push fired no job within 60 s");
    assert_eq!(job.status, Status::Done, "{job:#?}");
    assert_eq!(job.sha.as_deref(), Some(pushed.as_str()));
    assert_eq!(job.base.as_deref(), Some("main"));
    assert_eq!(job.summary, SUMMARY);
    assert_eq!(
        job.pr_url.as_deref(),
        Some("https://github.example/acme/app/pull/7"),
        "{job:#?}"
    );
    assert!(job.approvals >= 1, "approvals went over Telegram: {job:#?}");

    // The model saw the test fail, then pass.
    let seen = seen.lock().unwrap().clone();
    let result_of = |turn: usize| -> (String, bool) {
        seen[turn]
            .iter()
            .flat_map(|m| &m.parts)
            .filter_map(|p| match p {
                ContentPart::ToolResult {
                    content, is_error, ..
                } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .next_back()
            .unwrap()
    };
    assert_eq!(seen.len(), 5);
    assert!(result_of(1).1, "first test run fails: {:?}", result_of(1));
    assert!(!result_of(3).1, "the fix is written: {:?}", result_of(3));
    assert!(
        !result_of(4).1,
        "second test run passes: {:?}",
        result_of(4)
    );

    // The PR request.
    let reqs = github.received_requests().await.unwrap();
    let pr = reqs
        .iter()
        .find(|r| r.url.path().ends_with("/pulls"))
        .unwrap();
    assert_eq!(
        pr.headers.get("authorization").unwrap().to_str().unwrap(),
        "Bearer test-token"
    );
    let body: Value = serde_json::from_slice(&pr.body).unwrap();
    let branch = job.branch.clone().unwrap();
    assert!(branch.starts_with("kor/ci-"), "{branch}");
    assert_eq!(body["head"], branch.as_str());
    assert_eq!(body["base"], "main");
    assert_eq!(body["title"], "kor: run the tests, fix failures, open a PR");
    assert_eq!(body["draft"], false);
    let text = body["body"].as_str().unwrap();
    assert!(
        text.starts_with(SUMMARY) && text.contains("Kor trigger `ci`") && text.contains(&job.id),
        "{text}"
    );

    // The branch exists in origin with the fix on top of the pushed commit.
    git(
        &origin,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    );
    assert_eq!(
        git(&origin, &["show", &format!("{branch}:value.txt")]),
        "42"
    );
    assert_eq!(git(&origin, &["rev-parse", &format!("{branch}~1")]), pushed);
    // The worktree is gone, the hook ignored Kor's own branch push, nothing else queued.
    assert!(job.worktree.is_none());
    assert_eq!(triggers::history(&state, None, 10).len(), 1);

    // Telegram got the approval request(s) and the result.
    let sent = bot.sent.lock().unwrap();
    assert!(sent.iter().any(|m| {
        m["reply_markup"]["inline_keyboard"][0]
            .as_array()
            .is_some_and(|b| b.len() == 3)
    }));
    assert!(sent.iter().any(|m| {
        m["text"]
            .as_str()
            .is_some_and(|t| t.contains("finished") && t.contains("pull/7"))
    }));
    drop(sent);
    let _: PathBuf = hook;
}
