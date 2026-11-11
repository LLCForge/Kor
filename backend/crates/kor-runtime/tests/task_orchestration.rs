//! Spec §14 Faz 6 acceptance 1 (`kor task`, ADR 0037) with scripted models: an orchestrator
//! plans five sub-agents, each edits its own files in its own worktree (two of them change one
//! shared file in hunks that do not overlap), a reviewer approves each diff, everything merges
//! into the task branch without conflict and the test command passes. Second: two agents edit
//! the same lines, the conflict is detected, the task branch is not half-merged, the conflict
//! diff is reported, and re-planning that part finishes the task.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{Agent, Permissions};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::cost::{Control, Files, Prices, Target};
use kor_runtime::task::{
    self, Engine, Model, Resolution, Settings, SubStatus, Spawned, Spawner, TaskStatus,
};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::{Value, json};

type Turn = Vec<StreamEvent>;

struct Scripted {
    manifest: ProviderManifest,
    turns: Mutex<VecDeque<Turn>>,
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
        _req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let evs = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model call");
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn scripted(turns: Vec<Turn>) -> Arc<Scripted> {
    Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(turns.into()),
    })
}

fn usage(v: &mut Turn, cost: Option<f64>) {
    if let Some(c) = cost {
        v.push(StreamEvent::Raw {
            value: json!({"prompt_tokens": 100, "cost": c}),
        });
    }
    v.push(StreamEvent::Usage {
        usage: Usage {
            input: 100,
            output: 10,
            ..Usage::default()
        },
    });
}

fn tool(name: &str, input: &Value) -> Turn {
    let mut v = vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: format!("call-{name}"),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: input.to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
    ];
    usage(&mut v, None);
    v.push(StreamEvent::Finish {
        reason: FinishReason::ToolUse,
    });
    v
}

fn say(text: &str) -> Turn {
    let mut v = vec![StreamEvent::TextDelta { text: text.into() }];
    usage(&mut v, Some(0.001));
    v.push(StreamEvent::Finish {
        reason: FinishReason::Stop,
    });
    v
}

fn write(path: &str, content: &str) -> Turn {
    tool("Write", &json!({"file_path": path, "content": content}))
}

fn read(path: &str) -> Turn {
    tool("Read", &json!({"file_path": path}))
}

fn edit(path: &str, old: &str, new: &str) -> Turn {
    tool(
        "Edit",
        &json!({"file_path": path, "old_string": old, "new_string": new}),
    )
}

/// Scripted sub-agents: per role, one list of turns per time it is spawned.
struct TestSpawner {
    dir: std::path::PathBuf,
    scripts: Mutex<HashMap<String, VecDeque<Vec<Turn>>>>,
}

impl Spawner for TestSpawner {
    fn spawn(&self, worktree: &Path, role: &str, _model: Option<&str>) -> Result<Spawned, String> {
        let turns = self
            .scripts
            .lock()
            .unwrap()
            .get_mut(role)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| format!("no script for {role}"))?;
        let p = scripted(turns);
        let cost = Arc::new(Control::new(
            kor_i18n::Lang::En,
            kor_config::Routing::default(),
            kor_config::Budget::default(),
            Target {
                provider: p.clone(),
                model: role.into(),
                tool_mode: kor_agent::toolcall::ToolMode::Native,
            },
            Vec::new(),
            Prices::default(),
            files(&self.dir),
        ));
        let agent = Agent {
            provider: p,
            model: role.into(),
            tools: default_tools(),
            permissions: Permissions::new(PermissionMode::AcceptEdits, &[], &[], &[]).unwrap(),
            ctx: ToolCtx::new(worktree).map_err(|e| e.to_string())?,
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
            cost: Some(cost.clone()),
            shadow: None,
            repair: None,
        };
        Ok(Spawned {
            agent,
            cost,
            session: None,
        })
    }

    fn command_ctx(&self, dir: &Path) -> Result<ToolCtx, String> {
        ToolCtx::new(dir).map_err(|e| e.to_string())
    }
}

fn files(dir: &Path) -> Files {
    Files {
        ledger: Some(dir.join("usage.jsonl")),
        log: None,
    }
}

fn model(dir: &Path, turns: Vec<Turn>) -> Model {
    let p = scripted(turns);
    let cost = Arc::new(Control::new(
        kor_i18n::Lang::En,
        kor_config::Routing::default(),
        kor_config::Budget::default(),
        Target {
            provider: p.clone(),
            model: "m".into(),
            tool_mode: kor_agent::toolcall::ToolMode::Native,
        },
        Vec::new(),
        Prices::default(),
        files(dir),
    ));
    Model {
        provider: p,
        model: "m".into(),
        cost: Some(cost),
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with `shared.txt` (30 numbered lines) and one commit.
fn fixture(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
    std::fs::write(dir.join("shared.txt"), lines.join("\n") + "\n").unwrap();
    std::fs::write(dir.join("README.md"), "# fixture\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn plan(subtasks: &Value) -> Turn {
    tool(
        "submit_plan",
        &json!({"summary": "split by area", "subtasks": subtasks}),
    )
}

fn approve() -> Turn {
    tool("submit_review", &json!({"approved": true, "notes": "looks right"}))
}

fn settings(agents: usize) -> Settings {
    Settings {
        agents,
        max_parallel: 3,
        test_command: "echo tests-ok".into(),
        ..Settings::default()
    }
}

fn snapshots() -> (task::Progress, Arc<Mutex<Vec<TaskStatus>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (
        Arc::new(move |st: &task::TaskState| s.lock().unwrap().push(st.status)),
        seen,
    )
}

/// The desktop runs the engine on a spawned task: its future must be `Send`.
fn send<F: std::future::Future + Send>(f: F) -> F {
    f
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_agents_merge_without_conflict() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fixture(repo.path());
    let roles = ["architecture", "backend", "frontend", "test", "docs"];
    let subtasks: Vec<Value> = roles
        .iter()
        .map(|r| json!({"role": r, "goal": format!("do the {r} part"), "files": [format!("{r}.txt"), "shared.txt"]}))
        .collect();
    let mut scripts: HashMap<String, VecDeque<Vec<Turn>>> = HashMap::new();
    for r in roles {
        let mut turns = vec![write(&format!("{r}.txt"), &format!("{r} work\n"))];
        // Two agents also change the shared file, far apart: line 2 and line 28.
        match r {
            "backend" => {
                turns.push(read("shared.txt"));
                turns.push(edit("shared.txt", "line 2\n", "line 2 (backend)\n"));
            }
            "frontend" => {
                turns.push(read("shared.txt"));
                turns.push(edit("shared.txt", "line 28\n", "line 28 (frontend)\n"));
            }
            _ => {}
        }
        turns.push(say(&format!("{r} done")));
        scripts.insert(r.into(), VecDeque::from([turns]));
    }
    let engine = Engine {
        root: repo.path().to_path_buf(),
        lang: kor_i18n::Lang::En,
        orchestrator: model(home.path(), vec![plan(&json!(subtasks))]),
        reviewer: model(home.path(), (0..5).map(|_| approve()).collect()),
        spawner: Arc::new(TestSpawner {
            dir: home.path().to_path_buf(),
            scripts: Mutex::new(scripts),
        }),
        progress: snapshots().0,
    };
    let (root, mut st) = task::create(repo.path(), "build the feature", settings(5)).unwrap();
    assert_eq!(st.id, "1");
    send(engine.run(&mut st, &CancellationToken::new()))
        .await
        .unwrap();

    assert_eq!(st.status, TaskStatus::Done, "{st:#?}");
    assert_eq!(st.subtasks.len(), 5);
    assert!(st.conflicts.is_empty());
    for s in &st.subtasks {
        assert_eq!(s.status, SubStatus::Merged, "{s:#?}");
        assert!(s.cost_usd > 0.0, "cost per sub-agent: {s:#?}");
        assert!(s.test.as_ref().is_some_and(|t| t.ok));
        assert!(!root.join(&s.worktree).exists(), "worktree cleaned up");
    }
    assert!(st.test.as_ref().is_some_and(|t| t.ok && t.output.contains("tests-ok")));

    // The task branch holds every change; the user's branch is untouched.
    let shared = git(&root, &["show", "kor/task/1/merged:shared.txt"]);
    assert!(shared.contains("line 2 (backend)\n") && shared.contains("line 28 (frontend)\n"));
    for r in roles {
        assert_eq!(
            git(&root, &["show", &format!("kor/task/1/merged:{r}.txt")]),
            format!("{r} work\n")
        );
    }
    assert!(!root.join("backend.txt").exists());
    let branches = git(&root, &["branch", "--list", "kor/task/*"]);
    assert_eq!(branches.trim().trim_start_matches(['*', '+', ' ']), "kor/task/1/merged");
    assert!(git(&root, &["status", "--porcelain"]).trim().is_empty(), "{}", git(&root, &["status", "--porcelain"]));

    // Checkpoint and ledger: per-role costs are tagged.
    let saved = task::load(&root, "1").unwrap();
    assert_eq!(saved, st);
    assert_eq!(task::list(&root).len(), 1);
    let ledger = kor_runtime::usage::read(&home.path().join("usage.jsonl"));
    for r in roles {
        assert!(
            ledger.iter().any(|e| e.role.as_deref() == Some(&format!("task:1/{r}"))),
            "{r}"
        );
    }
    assert!(ledger.iter().any(|e| e.role.as_deref() == Some("task:1/orchestrator")));
    assert!(ledger.iter().any(|e| e.role.as_deref() == Some("task:1/reviewer")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_lines_conflict_is_reported_and_replanned() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fixture(repo.path());
    let subtasks = json!([
        {"role": "backend", "goal": "rename line 5", "files": ["shared.txt"]},
        {"role": "frontend", "goal": "rename line 5 too", "files": ["shared.txt"]}
    ]);
    let mut scripts: HashMap<String, VecDeque<Vec<Turn>>> = HashMap::new();
    scripts.insert(
        "backend".into(),
        VecDeque::from([vec![
            read("shared.txt"),
            edit("shared.txt", "line 5\n", "line 5 backend\n"),
            say("done"),
        ]]),
    );
    scripts.insert(
        "frontend".into(),
        VecDeque::from([
            vec![
                read("shared.txt"),
                edit("shared.txt", "line 5\n", "line 5 frontend\n"),
                say("done"),
            ],
            // After re-planning it starts from the task branch, which has the backend change.
            vec![
                read("shared.txt"),
                edit("shared.txt", "line 5 backend\n", "line 5 backend+frontend\n"),
                say("done"),
            ],
        ]),
    );
    let replan = tool(
        "submit_replan",
        &json!({"goal": "extend the backend's line 5", "files": ["shared.txt"]}),
    );
    let engine = Engine {
        root: repo.path().to_path_buf(),
        lang: kor_i18n::Lang::En,
        orchestrator: model(home.path(), vec![plan(&subtasks), replan]),
        reviewer: model(home.path(), (0..3).map(|_| approve()).collect()),
        spawner: Arc::new(TestSpawner {
            dir: home.path().to_path_buf(),
            scripts: Mutex::new(scripts),
        }),
        progress: snapshots().0,
    };
    let (root, mut st) = task::create(repo.path(), "rename", settings(2)).unwrap();
    let cancel = CancellationToken::new();
    engine.run(&mut st, &cancel).await.unwrap();

    assert_eq!(st.status, TaskStatus::Conflict, "{st:#?}");
    assert_eq!(st.subtasks[0].status, SubStatus::Merged);
    assert_eq!(st.subtasks[1].status, SubStatus::Conflict);
    let c = &st.conflicts[0];
    assert_eq!((c.index, c.role.as_str()), (1, "frontend"));
    assert_eq!(c.files.len(), 1);
    assert_eq!(c.files[0].path, "shared.txt");
    assert!(c.files[0].ours.contains("line 5 backend\n"));
    assert!(c.files[0].theirs.contains("line 5 frontend\n"));
    assert!(c.diff.contains("<<<<<<<") && c.diff.contains("line 5 frontend"), "{}", c.diff);

    // Nothing half-merged: the task branch has only the backend change, the merge worktree is
    // clean and no merge is in progress.
    let merged_wt = root.join(".kor/worktrees").join(&st.id).join("merged");
    assert!(git(&merged_wt, &["status", "--porcelain"]).trim().is_empty());
    let on_branch = git(&root, &["show", "kor/task/1/merged:shared.txt"]);
    assert!(on_branch.contains("line 5 backend\n") && !on_branch.contains("frontend"));
    assert!(!on_branch.contains("<<<<<<<"));
    let pending = Command::new("git")
        .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .current_dir(&merged_wt)
        .status()
        .unwrap();
    assert!(!pending.success(), "no merge in progress");
    // The conflicting branch is kept for the user.
    assert!(git(&root, &["branch", "--list", "kor/task/1/2-frontend"]).contains("2-frontend"));
    assert_eq!(task::load(&root, "1").unwrap().status, TaskStatus::Conflict);

    // Re-plan that part: the orchestrator rewrites it on top of the task branch.
    engine
        .resolve(&mut st, 1, Resolution::Replan, &cancel)
        .await
        .unwrap();
    assert_eq!(st.status, TaskStatus::Done, "{st:#?}");
    assert!(st.conflicts.is_empty());
    assert_eq!(st.subtasks[1].goal, "extend the backend's line 5");
    let final_text = git(&root, &["show", "kor/task/1/merged:shared.txt"]);
    assert!(final_text.contains("line 5 backend+frontend\n"), "{final_text}");
}

#[test]
fn fork_and_cancel_keep_the_checkpoint_consistent() {
    let repo = tempfile::tempdir().unwrap();
    fixture(repo.path());
    let (root, mut st) = task::create(repo.path(), "goal", settings(2)).unwrap();
    st.status = TaskStatus::Running;
    task::save(&root, &mut st).unwrap();
    let forked = task::fork(&root, &st.id).unwrap();
    assert_eq!(forked.id, "2");
    assert_eq!(forked.forked_from.as_deref(), Some("1"));
    assert_eq!(forked.branch, "kor/task/2/merged");
    assert_eq!(forked.status, TaskStatus::Failed, "a fork is resumable, not running");
    let cancelled = task::cancel(&root, "1").unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);
    assert!(task::load(&root, "../1").is_err());
    let excluded = std::fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert!(excluded.contains("/.kor/worktrees/") && excluded.contains("/.kor/tasks/"));
    assert!(git(&root, &["status", "--porcelain"]).trim().is_empty());
}

#[test]
fn checkpoints_live_in_sqlite_and_old_json_ones_are_imported() {
    let repo = tempfile::tempdir().unwrap();
    fixture(repo.path());
    let (root, mut st) = task::create(repo.path(), "goal", settings(2)).unwrap();
    let dir = task::tasks_dir(&root);
    assert!(dir.join("checkpoints.sqlite").is_file());
    assert!(!dir.join("1.json").exists(), "no JSON checkpoint any more");
    // A checkpoint an older version left as `<id>.json`, and a `TaskCreate` tool file beside it.
    st.id = "7".into();
    st.goal = "from an older version".into();
    std::fs::write(dir.join("7.json"), serde_json::to_string(&st).unwrap()).unwrap();
    std::fs::write(dir.join("8.json"), r#"{"description":"tool task","status":"open"}"#).unwrap();
    assert_eq!(task::load(&root, "7").unwrap().goal, "from an older version");
    assert!(dir.join("7.json.imported").is_file() && !dir.join("7.json").exists());
    assert!(dir.join("8.json").is_file(), "the tool's own files are left alone");
    let ids: Vec<String> = task::list(&root).into_iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"1".to_owned()) && ids.contains(&"7".to_owned()));
    // Saving updates the row in place.
    let mut one = task::load(&root, "1").unwrap();
    one.goal = "changed".into();
    task::save(&root, &mut one).unwrap();
    assert_eq!(task::load(&root, "1").unwrap().goal, "changed");
    assert_eq!(task::list(&root).len(), 2);
}

