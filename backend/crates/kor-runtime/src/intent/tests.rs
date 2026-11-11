use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_core::{ProviderError, StreamEvent};
use kor_providers::{ChatRequest, EventStream, ModelInfo, ProviderManifest};

use super::*;

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    text: String,
    expect: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, serde_yaml::Value>,
}

fn fixtures() -> Vec<(String, Fixture)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/intents");
    let mut out = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/intents")
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    for f in files
        .into_iter()
        .filter(|f| f.extension().is_some_and(|e| e == "yaml"))
    {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let list: Vec<Fixture> =
            serde_yaml::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        out.extend(list.into_iter().map(|x| (name.clone(), x)));
    }
    out
}

fn value(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_owned(),
    }
}

fn router() -> Router {
    Router::new(Catalog::builtin(), Learned::default())
}

fn chosen(d: &Decision) -> Option<&Plan> {
    match d {
        Decision::Run { plan } | Decision::Confirm { plan } | Decision::Missing { plan } => {
            Some(plan)
        }
        _ => None,
    }
}

/// §30.2: the fixture set (tests/intents/*.yaml) must route at ≥ 95% — command and arguments.
#[test]
fn fixture_accuracy() {
    let r = router();
    let ctx = Context::default();
    let all = fixtures();
    assert!(
        all.len() >= 500,
        "need 500+ fixture sentences, have {}",
        all.len()
    );
    let mut per_file: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut ok, mut cmd_ok, mut asked) = (0usize, 0usize, 0usize);
    let mut fails = Vec::new();
    for (file, f) in &all {
        let d = r.route(&f.text, &ctx);
        if matches!(d, Decision::Choose { .. }) {
            asked += 1;
        }
        let got = chosen(&d);
        let cmd_right = got.map(|p| p.command.as_str()) == f.expect.as_deref();
        let params_right = f
            .params
            .iter()
            .all(|(k, v)| got.and_then(|p| p.params.get(k)) == Some(&value(v)));
        let e = per_file.entry(file.clone()).or_default();
        e.1 += 1;
        if cmd_right {
            cmd_ok += 1;
        }
        if cmd_right && params_right {
            ok += 1;
            e.0 += 1;
        } else {
            fails.push(format!(
                "{file}: {:?} → {:?} {:?} (want {:?} {:?})",
                f.text,
                got.map(|p| &p.command),
                got.map(|p| &p.params),
                f.expect,
                f.params
            ));
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b as f64;
    for (file, (a, b)) in &per_file {
        eprintln!("intent accuracy {file}: {a}/{b} = {:.1}%", pct(*a, *b));
    }
    eprintln!(
        "intent accuracy total: {ok}/{} = {:.1}% (command only {:.1}%, questions {asked})",
        all.len(),
        pct(ok, all.len()),
        pct(cmd_ok, all.len())
    );
    for f in &fails {
        eprintln!("  miss {f}");
    }
    assert!(pct(ok, all.len()) >= 95.0, "intent accuracy below 95%");
}

#[test]
fn read_only_runs_and_writes_are_confirmed() {
    let r = router();
    let ctx = Context::default();
    let d = r.route("maliyetimi göster", &ctx);
    assert!(
        matches!(&d, Decision::Run { plan } if plan.command == "kor cost" && plan.line == "kor cost" && plan.slash == "/cost"),
        "{d:?}"
    );
    let d = r.route("son hatayı düzelt", &ctx);
    assert!(
        matches!(&d, Decision::Confirm { plan } if plan.command == "kor fix"),
        "{d:?}"
    );
    let all = Context {
        confirm_all: true,
        ..Context::default()
    };
    assert!(matches!(
        r.route("maliyetimi göster", &all),
        Decision::Confirm { .. }
    ));
    // A coding task is not a command.
    assert_eq!(
        r.route("parse fonksiyonundaki null hatasını düzelt", &ctx),
        Decision::Agent
    );
    assert_eq!(r.route("/cost", &ctx), Decision::Agent);
    assert_eq!(r.route("", &ctx), Decision::Agent);
    // Nothing reaches a command routing must never pick.
    for i in r.catalog.intents.iter().filter(|i| i.run == Run::Never) {
        assert!(r.plan(&i.command, "x", &ctx, Via::Keyword, 1.0).is_none());
    }
}

#[test]
fn arguments_come_from_text_then_context() {
    let r = router();
    let d = r.route("explain src/main.rs", &Context::default());
    let p = chosen(&d).unwrap();
    assert_eq!(p.command, "kor explain");
    assert_eq!(p.argv, ["explain", "src/main.rs"]);
    assert_eq!(p.line, "kor explain src/main.rs");
    let ctx = Context {
        selection: Some("lib/auth.ts".into()),
        ..Context::default()
    };
    let d = r.route("bu dosyayı açıkla", &ctx);
    if let Some(p) = chosen(&d).filter(|p| p.command == "kor explain") {
        assert_eq!(
            p.params.get("file").map(String::as_str),
            Some("lib/auth.ts")
        );
    }
    let d = r.route("dosyayı açıkla", &Context::default());
    if let Decision::Missing { plan } = &d {
        assert_eq!(plan.missing, ["file"]);
    }
    assert_eq!(quote("a b"), "\"a b\"");
    assert!(
        is_path("src/x.rs") && is_path("main.rs") && !is_path("v1") && !is_path("https://x.io/a")
    );
    assert!(
        is_domain("example.net")
            && !is_domain("x")
            && host_of("https://a.example.com/p").as_deref() == Some("a.example.com")
    );
}

#[test]
fn learned_patterns_win_and_can_be_forgotten() {
    let mut l = Learned::default();
    l.learn("siteye bir bak", "kor doctor", 1);
    l.learn("Siteye bir bak!", "kor cost", 2);
    assert_eq!(l.patterns.len(), 1);
    let r = Router::new(Catalog::builtin(), l.clone());
    let d = r.route("siteye bir bak", &Context::default());
    assert!(
        matches!(&d, Decision::Run { plan } if plan.command == "kor cost" && plan.via == Via::Learned),
        "{d:?}"
    );
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("learned.json");
    l.last.insert("domain".into(), "example.net".into());
    l.save(&f).unwrap();
    let mut back = Learned::load(&f);
    assert_eq!(
        back.last.get("domain").map(String::as_str),
        Some("example.net")
    );
    assert!(back.forget("siteye bir bak"));
    assert!(!back.forget("siteye bir bak"));
}

#[test]
fn pipelines_and_schedules() {
    let r = router();
    let ctx = Context::default();
    let d = r.route("maliyetimi göster, sonra son hatayı düzelt", &ctx);
    let Decision::Pipeline { steps } = &d else {
        panic!("{d:?}")
    };
    assert_eq!(steps.len(), 2);
    assert!(matches!(&steps[0], Step::Command(p) if p.command == "kor cost"));
    assert!(matches!(&steps[1], Step::Command(p) if p.command == "kor fix"));
    let d = r.route("maliyetimi göster, then add a retry to fetch()", &ctx);
    assert!(
        matches!(&d, Decision::Pipeline { steps } if matches!(&steps[1], Step::Agent(t) if t == "add a retry to fetch()")),
        "{d:?}"
    );

    // Monday 2026-11-09 10:00 local → next Monday 09:00.
    let now = Local
        .with_ymd_and_hms(2026, 11, 9, 10, 0, 0)
        .single()
        .unwrap();
    let s = strip_schedule_at("her pazartesi maliyetimi göster", now).unwrap();
    assert_eq!((s.rest.as_str(), s.minutes), ("maliyetimi göster", 10_080));
    let first = Local
        .timestamp_millis_opt(i64::try_from(s.first_run).unwrap())
        .single()
        .unwrap();
    assert_eq!(
        (first.weekday(), first.hour(), first.day()),
        (chrono::Weekday::Mon, 9, 16)
    );
    let s = strip_schedule_at("show my cost every day at 18:30", now).unwrap();
    assert_eq!((s.rest.as_str(), s.minutes), ("show my cost", 1440));
    let first = Local
        .timestamp_millis_opt(i64::try_from(s.first_run).unwrap())
        .single()
        .unwrap();
    assert_eq!((first.hour(), first.minute(), first.day()), (18, 30, 9));
    let s = strip_schedule_at("toutes les 2 heures affiche mes coûts", now).unwrap();
    assert_eq!(s.minutes, 120);
    let s = strip_schedule_at("her gün saat 9'da bağımlılıkları kontrol et", now).unwrap();
    assert_eq!(s.minutes, 1440);
    assert!(strip_schedule_at("every day I see this error in parse()", now).is_none());
    let d = r.route("her pazartesi maliyetimi göster", &ctx);
    assert!(
        matches!(&d, Decision::Schedule { plan: Some(p), interval_minutes: 10_080, .. } if p.command == "kor cost"),
        "{d:?}"
    );
}

#[test]
fn model_answers_are_validated() {
    let c = ["kor cost", "kor quota"];
    assert_eq!(
        parse_model_answer("{\"command\": null}", &c).unwrap(),
        ModelAnswer::None
    );
    assert!(parse_model_answer("{\"command\": \"kor deploy\"}", &c).is_err());
    assert!(parse_model_answer("no json", &c).is_err());
    let a = parse_model_answer("sure: ```json\n{\"command\":\"kor quota\",\"params\":{\"x\":\"1\",\"bad\":\"a\\nb\"},\"sure\":true}\n```", &c).unwrap();
    assert_eq!(
        a,
        ModelAnswer::Command {
            command: "kor quota".into(),
            params: [("x".to_owned(), "1".to_owned())].into(),
            sure: true
        }
    );
}

struct Mock {
    manifest: ProviderManifest,
    reply: String,
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl Provider for Mock {
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
        self.seen.lock().unwrap().push(req.messages[0].plain_text());
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            StreamEvent::TextDelta {
                text: self.reply.clone(),
            },
        )])))
    }
}

fn mock(reply: &str) -> Arc<Mock> {
    Arc::new(Mock {
        manifest: kor_providers::Registry::builtin()
            .get("local")
            .unwrap()
            .clone(),
        reply: reply.into(),
        seen: Mutex::new(Vec::new()),
    })
}

/// An unsure text for the fixtures' catalog: stage 1 scores it between PLAUSIBLE and ACCEPT.
fn unsure_text(r: &Router) -> String {
    let probes = [
        "bu ay modele giden para ne kadar oldu acaba",
        "sandbox ortamı ne durumda acaba söyler misin",
        "maliyet raporu",
        "harcama durumu nedir",
        "sandbox ne alemde",
        "güncelleme var mı bakar mısın şu an",
        "kota",
        "doctor çalıştır bakalım ortam",
    ];
    probes
        .iter()
        .find(|t| {
            r.rank(t)
                .first()
                .is_some_and(|c| c.score >= PLAUSIBLE && c.score < ACCEPT)
        })
        .map(|t| (*t).to_owned())
        .expect("one probe in the unsure band")
}

#[tokio::test]
async fn the_model_picks_only_among_candidates() {
    let r = router();
    let text = unsure_text(&r);
    let top = r.rank(&text)[0].command.clone();
    let m = mock(&format!(
        "{{\"command\": \"{top}\", \"params\": {{}}, \"sure\": true}}"
    ));
    let fb = Fallbacks {
        embedder: None,
        model: Some((m.clone() as Arc<dyn Provider>, "local/m".into())),
    };
    let d = r.route_full(&text, &Context::default(), &fb).await;
    let p = chosen(&d).unwrap_or_else(|| panic!("{d:?}"));
    assert_eq!((p.command.as_str(), p.via), (top.as_str(), Via::Model));
    assert!(m.seen.lock().unwrap()[0].contains(&top));
    // `null` → the agent; an unknown command → ignored (no model decision).
    let m = mock("{\"command\": null}");
    let fb = Fallbacks {
        embedder: None,
        model: Some((m as Arc<dyn Provider>, "local/m".into())),
    };
    assert_eq!(
        r.route_full(&text, &Context::default(), &fb).await,
        Decision::Agent
    );
    let m = mock("{\"command\": \"kor deploy\", \"sure\": true}");
    let fb = Fallbacks {
        embedder: None,
        model: Some((m as Arc<dyn Provider>, "local/m".into())),
    };
    let d = r.route_full(&text, &Context::default(), &fb).await;
    assert!(chosen(&d).is_none_or(|p| p.via != Via::Model), "{d:?}");
    // Not sure → one question with the model's pick first.
    let m = mock(&format!("{{\"command\": \"{top}\", \"sure\": false}}"));
    let fb = Fallbacks {
        embedder: None,
        model: Some((m as Arc<dyn Provider>, "local/m".into())),
    };
    let d = r.route_full(&text, &Context::default(), &fb).await;
    assert!(
        matches!(&d, Decision::Choose { options } if options[0].command == top),
        "{d:?}"
    );
}

/// A concept embedder: synonyms share a dimension, so meaning matches where words do not.
#[derive(Debug)]
struct Concepts;

const SYNONYMS: &[&[&str]] = &[
    &["spending", "expenses", "outlay", "harcama", "masraf"],
    &["show", "display", "reveal", "goster"],
    &["environment", "setup", "machine", "ortam"],
    &["health", "wellbeing", "saglik"],
];

#[async_trait]
impl Embedder for Concepts {
    fn name(&self) -> &'static str {
        "concepts"
    }
    fn default_model(&self) -> &'static str {
        "concepts-1"
    }
    fn local(&self) -> bool {
        true
    }
    async fn embed(&self, _m: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; 64];
                for w in matcher::tokens(t) {
                    let d = SYNONYMS
                        .iter()
                        .position(|s| s.contains(&w.as_str()))
                        .unwrap_or_else(|| {
                            8 + w
                                .bytes()
                                .fold(7usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize))
                                % 56
                        });
                    v[d] += 1.0;
                }
                v
            })
            .collect())
    }
}

#[tokio::test]
#[allow(clippy::many_single_char_names)] // r, d, p, f, i: router, decision, plan, file, index
async fn local_embeddings_match_meaning_stage_one_misses() {
    let cat = Catalog::parse(
        "[[intent]]
command = \"kor cost\"
run = \"auto\"
en = [\"show spending\"]

[[intent]]
command = \"kor doctor\"
run = \"auto\"
en = [\"check environment health\"]
",
        &Source::Builtin,
    )
    .unwrap();
    let r = Router::new(cat, Learned::default());
    // No shared word: stage 1 sends it to the agent.
    assert_eq!(
        r.route("display my expenses", &Context::default()),
        Decision::Agent
    );
    let fb = Fallbacks {
        embedder: Some(&Concepts),
        model: None,
    };
    let d = r
        .route_full("display my expenses", &Context::default(), &fb)
        .await;
    let p = chosen(&d).unwrap_or_else(|| panic!("{d:?}"));
    assert_eq!((p.command.as_str(), p.via), ("kor cost", Via::Embedding));
    let d = r
        .route_full("check machine wellbeing", &Context::default(), &fb)
        .await;
    assert!(
        matches!(&d, Decision::Run { plan } if plan.command == "kor doctor"),
        "{d:?}"
    );
    // Unrelated text stays with the agent.
    assert_eq!(
        r.route_full("refactor the parser module", &Context::default(), &fb)
            .await,
        Decision::Agent
    );

    // The index is cached on disk in the format it reads back.
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("e.bin");
    let i = EmbIndex {
        hash: [3; 32],
        vectors: vec![("a".into(), vec![1.0, 2.0]), ("b".into(), vec![0.5, -1.0])],
    };
    i.write(&f).unwrap();
    let back = EmbIndex::read(&f, &[3; 32], &["a".into(), "b".into()]).unwrap();
    assert_eq!(back.vectors, i.vectors);
    assert!(EmbIndex::read(&f, &[4; 32], &["a".into(), "b".into()]).is_none());
    assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
}

/// The real `embed-e5-small` component on the fixture set (run when it is installed:
/// `cargo test -p kor-runtime intent -- --ignored`).
#[tokio::test]
#[ignore = "needs the embed-e5-small component"]
async fn fixture_accuracy_with_local_embeddings() {
    let Some(e) = kor_agent::semantic::local_embedder() else {
        return;
    };
    let r = router();
    let fb = Fallbacks {
        embedder: Some(&e),
        model: None,
    };
    let all = fixtures();
    let mut ok = 0usize;
    for (_, f) in &all {
        let d = r.route_full(&f.text, &Context::default(), &fb).await;
        if chosen(&d).map(|p| p.command.as_str()) == f.expect.as_deref() {
            ok += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let pct = 100.0 * ok as f64 / all.len() as f64;
    eprintln!(
        "intent accuracy with local embeddings: {ok}/{} = {pct:.1}%",
        all.len()
    );
    assert!(pct >= 95.0);
}

#[test]
fn model_fallback_respects_privacy() {
    let mut c = kor_config::Config::default();
    assert!(model_allowed(&c, "openai/gpt-5-mini"));
    c.privacy.mode = true;
    assert!(!model_allowed(&c, "openai/gpt-5-mini"));
    assert!(model_allowed(&c, "local/qwen3"));
    c.intent.model_fallback = false;
    assert!(!model_allowed(&c, "local/qwen3"));
}

#[test]
fn routines_take_the_aligned_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("triggers.json");
    let mut m = serde_json::Map::new();
    m.insert("prompt".into(), "maliyetimi göster".into());
    m.insert("intervalMinutes".into(), 10_080.into());
    m.insert("firstRun".into(), 5_000_000u64.into());
    let t = crate::routines::add(&f, &m, 1_000).unwrap();
    assert_eq!(t["nextRun"], 5_000_000);
    m.insert("firstRun".into(), 10u64.into());
    let t = crate::routines::add(&f, &m, 1_000).unwrap();
    assert_eq!(t["nextRun"], 1_000 + 10_080 * 60_000);
}
