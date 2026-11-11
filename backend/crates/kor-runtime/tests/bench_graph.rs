//! `kor bench --graph`: the same task with the graph tools off and on, compared by tokens, cost
//! and passes (spec §7.5). The scripted model reads files one by one without the graph and asks
//! the graph once with it, so the token cut is visible without a real model.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{Agent, Permissions};
use kor_core::{FinishReason, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::model_catalog::ModelData;
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::bench::{self, Arm, Prices, Spec};
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};

struct Scripted {
    manifest: ProviderManifest,
    turn: Mutex<u32>,
    saw_graph: Arc<Mutex<Vec<bool>>>,
}

fn call(name: &str, json: &str, tokens: u64) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: "c".into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: json.into(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Usage {
            usage: Usage {
                input: tokens,
                output: tokens / 10,
                ..Usage::default()
            },
        },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
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
        let graph = req.tools.iter().any(|t| t.name == "graph_query");
        let mut turn = self.turn.lock().unwrap();
        *turn += 1;
        if *turn == 1 {
            self.saw_graph.lock().unwrap().push(graph);
        }
        let evs = match (graph, *turn) {
            (true, 1) => call("graph_query", r#"{"query": "login"}"#, 1_000),
            (false, 1..=3) => call(
                "Read",
                &format!(r#"{{"file_path": "src/f{}.rs"}}"#, *turn),
                4_000,
            ),
            _ => call(
                "Write",
                r#"{"file_path": "fixed.txt", "content": "ok"}"#,
                1_000,
            ),
        };
        let evs = if (graph && *turn > 2) || (!graph && *turn > 4) {
            vec![
                StreamEvent::TextDelta {
                    text: "done".into(),
                },
                StreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]
        } else {
            evs
        };
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

#[tokio::test]
async fn graph_arms_compare_tokens_cost_and_passes() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    for i in 1..=3 {
        std::fs::write(
            project.path().join(format!("src/f{i}.rs")),
            format!("pub fn f{i}() -> u32 {{ {i} }}\n"),
        )
        .unwrap();
    }
    std::fs::write(project.path().join("src/auth.rs"), "pub fn login() {}\n").unwrap();
    let spec = Spec::parse(
        "test = \"cat fixed.txt\"\n[[task]]\nname = \"fix\"\nprompt = \"create fixed.txt\"\n",
    )
    .unwrap();
    let prices = Prices {
        models: vec![ModelData {
            model_id: "m/x".into(),
            input_price_per_m: Some(1.0),
            output_price_per_m: Some(2.0),
            ..ModelData::default()
        }],
        local_providers: BTreeSet::new(),
    };
    let saw_graph = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&saw_graph);
    let make = move |root: &Path, model: &str| -> Result<Agent, String> {
        Ok(Agent {
            provider: Arc::new(Scripted {
                manifest: Registry::builtin().get("local").unwrap().clone(),
                turn: Mutex::new(0),
                saw_graph: Arc::clone(&seen),
            }),
            model: model.into(),
            tools: default_tools(),
            permissions: Permissions::new(PermissionMode::AcceptEdits, &[], &[], &[]).unwrap(),
            ctx: ToolCtx::new(root).map_err(|e| e.to_string())?,
            system: "sys".into(),
            messages: Vec::new(),
            usage: Usage::default(),
            max_turns: 8,
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
        })
    };
    let jobs = bench::graph_jobs(&spec, &["m/x".to_owned()], true, true);
    assert_eq!(
        jobs.iter().map(|j| j.arm).collect::<Vec<_>>(),
        vec![Arm::GraphOff, Arm::GraphOn]
    );
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let rows = bench::run(
        &l,
        project.path(),
        &spec,
        &jobs,
        &make,
        &prices,
        &|_| {},
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(*saw_graph.lock().unwrap(), vec![false, true]);
    assert!(rows.iter().all(|r| r.passed), "{rows:?}");
    assert!(rows[1].tool_calls_ok >= 1, "{:?}", rows[1]);

    let r = bench::report(rows);
    let g = r.graph.clone().expect("graph comparison");
    let cut = g.token_delta_pct.expect("token delta");
    assert!(cut < -50.0, "{cut}");
    assert!(g.cost_delta_pct.is_some_and(|c| c < 0.0));
    assert!(g.pass_delta_pp.abs() < 1e-9);
    assert_eq!(g.meets_acceptance, Some(true));
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["rows"][1]["arm"], "graph-on");

    // Only one arm: no comparison.
    assert!(
        bench::graph_jobs(&spec, &["m/x".to_owned()], false, true)
            .iter()
            .all(|j| j.arm == Arm::GraphOn)
    );
}
