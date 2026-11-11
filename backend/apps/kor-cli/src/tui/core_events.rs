//! §16.4 "TUI: ratatui test backend, every panel drawn from real core events" (ADR 0061).
//!
//! The real agent (built by `kor_runtime::prepare`, the same assembly the TUI uses) talks to the
//! E2E mock provider over HTTP; its events go through the TUI's own mapping (`repl::tui_ev`) and
//! reducer, an approval card is answered like a key press, and every frame is drawn on a
//! `TestBackend`. No event is invented by the test.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use kor_agent::{AgentEvent, Approver, PermissionReply, PermissionRequest};
use kor_i18n::{Lang, Localizer};
use kor_providers::CancellationToken;
use kor_runtime::e2e::{MockServer, Scenario};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::repl::tui_ev;
use crate::tui::{Ev, PANELS, Screen, draw, reduce};

const SCENARIO: &str = include_str!("../../../../../tests/e2e/scenarios/task-approve-write.yaml");

/// Answers every card "allow", after showing it on the screen the way the TUI does.
struct Keys {
    asked: Arc<Mutex<Vec<PermissionRequest>>>,
}

#[async_trait]
impl Approver for Keys {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(req.clone());
        PermissionReply::Allow
    }
}

fn text(t: &Terminal<TestBackend>) -> String {
    let b = t.backend().buffer();
    let mut s = String::new();
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            s.push_str(b[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

#[test]
fn every_panel_draws_real_core_events() {
    let scenario = Scenario::parse(SCENARIO).unwrap();
    let l = Localizer::new(Lang::En);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mock = rt
        .block_on(MockServer::start(scenario.mock.clone()))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (rel, content) in &scenario.project.files {
        std::fs::write(root.join(rel), content).unwrap();
    }

    // The TUI's agent assembly, with the provider pointed at the mock (an OpenAI-compatible local server).
    let mut loaded = kor_config::Loaded {
        config: kor_config::Config::default(),
        settings: std::collections::BTreeMap::new(),
        files: Vec::new(),
    };
    loaded.config.agent.model = "local-vllm/kor-mock".into();
    loaded.config.agent.verify = kor_config::VerifyMode::Off;
    let opts = kor_runtime::Options {
        root: root.clone(),
        model: Some("local-vllm/kor-mock".into()),
        mode: None,
        resume: None,
        new_id: None,
        surface: kor_agent::prompt::Surface::Terminal,
    };
    let mut notes = Vec::new();
    let mut prepared =
        kor_runtime::prepare(&l, &loaded, &opts, &mut notes).expect("agent assembles");
    let mut manifest = kor_providers::Registry::builtin()
        .get("local-vllm")
        .expect("local-vllm manifest")
        .clone();
    manifest.base_url.clone_from(&mock.url);
    prepared.agent.provider = kor_providers::build(&manifest, None).expect("mock provider");
    prepared.agent.model = kor_runtime::e2e::MODEL.into();
    // The developer's own Claude Code settings (allow rules, bypass mode) must not decide this test.
    prepared.agent.permissions =
        kor_agent::Permissions::new(kor_core::PermissionMode::Default, &[], &[], &[]).unwrap();

    let mut screen = Screen::new("default", "local-vllm/kor-mock");
    let mut term = Terminal::new(TestBackend::new(160, 48)).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let keys = Keys {
        asked: Arc::clone(&asked),
    };
    let prompt = scenario.steps[0].prompt.clone().unwrap();
    reduce(&mut screen, &l, Ev::Busy(Some(prompt.clone())));

    let mut events: Vec<AgentEvent> = Vec::new();
    let mut status_frame = None;
    rt.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let agent = &mut prepared.agent;
        let work = async move {
            let r = agent
                .send(prompt, &tx, &keys, CancellationToken::new())
                .await;
            drop(tx);
            r
        };
        let pump = async {
            while let Some(ev) = rx.recv().await {
                events.push(ev.clone());
                let is_status = matches!(ev, AgentEvent::Status(_));
                if let Some(shown) = tui_ev(ev) {
                    reduce(&mut screen, &l, shown);
                }
                term.draw(|f| draw(f, &screen, &l)).unwrap();
                if is_status && status_frame.is_none() {
                    status_frame = Some(text(&term));
                }
            }
        };
        let (r, ()) = tokio::join!(work, pump);
        r.expect("turn finishes");
    });
    reduce(&mut screen, &l, Ev::Busy(None));
    term.draw(|f| draw(f, &screen, &l)).unwrap();
    let out = text(&term);

    // Core events reached the screen: the answer, the tool, the approval the core asked for.
    assert!(out.contains("notes.md dosyasını oluşturuyorum."), "{out}");
    assert!(out.contains("Tamamlandı: notes.md yazıldı."), "{out}");
    assert!(out.contains("Write"), "{out}");
    let asked = asked.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(asked.len(), 1, "one approval card");
    assert_eq!(
        asked[0]
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["notes.md"]
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.md")).unwrap(),
        "# Notlar\n\n- ilk not\n"
    );
    // `agent.status` (§16.2): the verb drawn is the core's, while the task runs.
    let verb = events
        .iter()
        .find_map(|e| {
            if let AgentEvent::Status(s) = e {
                Some(s.verb.clone())
            } else {
                None
            }
        })
        .expect("status events");
    assert!(
        status_frame.unwrap_or_default().contains(&verb),
        "the status verb is drawn"
    );
    // Cost panel: the provider's own token counts (1200 + 1300 in, 40 + 12 out).
    let usage: (u64, u64) = events
        .iter()
        .filter_map(|e| {
            if let AgentEvent::Usage(u) = e {
                Some((u.input, u.output))
            } else {
                None
            }
        })
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    assert_eq!(usage, (2500, 52));
    // Every panel is on the wide screen.
    for p in PANELS {
        assert!(out.contains(&l.t(p.key())), "{} missing:\n{out}", p.key());
    }
    assert_eq!(mock.unused(), 0, "the whole script was used");
}
