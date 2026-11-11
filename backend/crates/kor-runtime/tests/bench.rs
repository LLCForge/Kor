//! `kor bench` end to end with scripted models on a tiny fixture project: each job runs in a
//! copy, the test command decides pass/fail, usage turns into cost, repaired calls are counted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
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
        let evs = self.turns.lock().unwrap().pop_front().unwrap_or_default();
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

fn say(text: &str, usage: Option<Usage>) -> Vec<StreamEvent> {
    let mut v = vec![StreamEvent::TextDelta { text: text.into() }];
    v.extend(usage.map(|usage| StreamEvent::Usage { usage }));
    v.push(StreamEvent::Finish {
        reason: FinishReason::Stop,
    });
    v
}

/// `good/m` writes `fixed.txt` with Python-style quotes (repaired locally); `bad/m` only talks.
fn script(model: &str) -> Vec<Vec<StreamEvent>> {
    let usage = Usage {
        input: 1_000_000,
        output: 100_000,
        ..Usage::default()
    };
    if model == "good/m" {
        vec![
            vec![
                StreamEvent::ToolCallStart {
                    index: 0,
                    id: "c1".into(),
                    name: "Write".into(),
                },
                StreamEvent::ToolCallDelta {
                    index: 0,
                    json: "{'file_path': 'fixed.txt', 'content': 'ok'}".into(),
                },
                StreamEvent::ToolCallEnd { index: 0 },
                StreamEvent::Usage { usage },
                StreamEvent::Finish {
                    reason: FinishReason::ToolUse,
                },
            ],
            say("done", None),
        ]
    } else {
        vec![say("I did nothing.", None)]
    }
}

fn agent(root: &Path, model: &str) -> Result<Agent, String> {
    let provider = Arc::new(Scripted {
        manifest: Registry::builtin().get("local").unwrap().clone(),
        turns: Mutex::new(script(model).into()),
    });
    Ok(Agent {
        provider,
        model: model.into(),
        tools: default_tools(),
        permissions: Permissions::new(PermissionMode::AcceptEdits, &[], &[], &[]).unwrap(),
        ctx: ToolCtx::new(root).map_err(|e| e.to_string())?,
        system: "sys".into(),
        messages: Vec::new(),
        usage: Usage::default(),
        max_turns: 5,
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
    })
}

#[tokio::test]
async fn races_models_in_copies_and_suggests_routing() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("readme.txt"), "fixture").unwrap();
    // `cat` exists in sh and PowerShell; a missing file fails in both.
    let spec = Spec::parse(
        "test = \"cat fixed.txt\"\n[[task]]\nname = \"fix\"\nprompt = \"create fixed.txt\"\n",
    )
    .unwrap();
    let prices = Prices {
        models: vec![ModelData {
            model_id: "good/m".into(),
            input_price_per_m: Some(1.0),
            output_price_per_m: Some(2.0),
            ..ModelData::default()
        }],
        local_providers: BTreeSet::from(["bad".to_owned()]),
    };
    let models = vec!["good/m".to_owned(), "bad/m".to_owned()];
    let jobs = bench::race_jobs(&spec, &models);
    assert!(bench::within_budget(&bench::estimate(&jobs, &prices), 1.0));
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let seen = Mutex::new(Vec::new());
    let rows = bench::run(
        &l,
        project.path(),
        &spec,
        &jobs,
        &agent,
        &prices,
        &|p| seen.lock().unwrap().push(p),
        &CancellationToken::new(),
    )
    .await;

    assert!(
        !project.path().join("fixed.txt").exists(),
        "working tree untouched"
    );
    assert_eq!(seen.lock().unwrap().len(), 4, "start + done per job");
    let good = &rows[0];
    assert!(good.passed, "{good:?}");
    assert_eq!(
        (good.input_tokens, good.output_tokens),
        (1_000_000, 100_000)
    );
    assert_eq!(good.cost_usd, Some(1.2));
    assert!(!good.cost_estimated);
    assert_eq!((good.tool_calls_ok, good.tool_calls_repaired), (0, 1));
    assert_eq!(good.tool_mode, "native");
    let bad = &rows[1];
    assert!(!bad.passed && bad.error.is_none(), "{bad:?}");
    assert!(bad.tokens_estimated && bad.cost_usd == Some(0.0));

    let r = bench::report(rows);
    assert_eq!(r.routing["simple_edit"], "good/m");
    assert!(r.routing_toml.starts_with("[routing]\n"));
    assert!(r.comparison.is_none());
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["rows"][0]["arm"], "race");

    // Router comparison: everything on good/m vs simple_edit routed to the free model.
    let routes = BTreeMap::from([("simple_edit".to_owned(), "bad/m".to_owned())]);
    let jobs = bench::router_jobs(&spec, "good/m", &routes);
    let rows = bench::run(
        &l,
        project.path(),
        &spec,
        &jobs,
        &agent,
        &prices,
        &|_| {},
        &CancellationToken::new(),
    )
    .await;
    let c = bench::compare(&rows).unwrap();
    assert_eq!(c.cost_delta_pct, Some(-100.0));
    assert!((c.pass_delta_pp + 100.0).abs() < 1e-9);
    assert_eq!(c.meets_acceptance, Some(false));
    assert_eq!(rows[1].arm, Arm::Routed);
}

#[tokio::test]
async fn cancelled_run_returns_no_rows() {
    let project = tempfile::tempdir().unwrap();
    let spec = Spec::parse("test = \"exit 0\"\n[[task]]\nname = \"a\"\nprompt = \"p\"\n").unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let rows = bench::run(
        &l,
        project.path(),
        &spec,
        &bench::race_jobs(&spec, &["bad/m".to_owned()]),
        &agent,
        &Prices::default(),
        &|_| {},
        &cancel,
    )
    .await;
    assert!(rows.is_empty());
}
