//! Consensus and shadow modes of the cost router (Master Build Prompt §5.1).
//!
//! **Consensus** (`kor consensus "<prompt>"`, `[routing] consensus = [...]`): the same prompt goes
//! to 2–3 cheap models in parallel; their answers are compared line by line. Lines most of them
//! give are taken as agreed, the rest are shown per model so the user picks.
//!
//! **Shadow** (`[routing] shadow = "<cheap model>"`): after every finished task, the cheap model
//! answers the same prompt in the background (text only, no tools). How much its answer agrees
//! with the main model's is appended to `<kor home>/shadow.jsonl`; once the recent average stays
//! above `shadow_threshold` over at least `shadow_min_samples` tasks, the user is told once a day
//! that the cheap model would do ("ucuza geç").

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use kor_i18n::{Localizer, args};
use kor_providers::Provider;
use serde::{Deserialize, Serialize};

/// One model's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Answer {
    pub model: String,
    pub text: String,
}

/// Lines only some models gave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    pub models: Vec<String>,
    pub lines: Vec<String>,
}

/// The merged result of a consensus run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Consensus {
    pub answers: Vec<Answer>,
    /// Lines a majority gave, in the order of the first answer that has them.
    pub agreed: Vec<String>,
    /// What each model said beyond the agreed lines (identical extras are grouped).
    pub disputed: Vec<Variant>,
    /// Share of all distinct lines that are agreed, 0–1.
    pub agreement: f64,
    /// Models that failed, with the error.
    pub failed: Vec<(String, String)>,
}

fn norm(line: &str) -> String {
    line.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// Line-level merge of `answers` (see the module docs).
pub fn merge(answers: Vec<Answer>, failed: Vec<(String, String)>) -> Consensus {
    let n = answers.len();
    let sets: Vec<BTreeSet<String>> = answers
        .iter()
        .map(|a| lines(&a.text).into_iter().map(norm).collect())
        .collect();
    let count = |k: &str| sets.iter().filter(|s| s.contains(k)).count();
    // Majority: more than half (two of two, two of three).
    let agreed_key = |k: &str| n >= 2 && count(k) * 2 > n;
    let mut agreed = Vec::new();
    let mut seen = BTreeSet::new();
    for a in &answers {
        for l in lines(&a.text) {
            let k = norm(l);
            if agreed_key(&k) && seen.insert(k) {
                agreed.push(l.trim().to_owned());
            }
        }
    }
    let mut groups: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    for a in &answers {
        let extra: Vec<String> = lines(&a.text)
            .into_iter()
            .filter(|l| !agreed_key(&norm(l)))
            .map(|l| l.trim().to_owned())
            .collect();
        if !extra.is_empty() {
            groups.entry(extra).or_default().push(a.model.clone());
        }
    }
    let disputed = groups
        .into_iter()
        .map(|(lines, models)| Variant { models, lines })
        .collect();
    let all: BTreeSet<&String> = sets.iter().flatten().collect();
    #[allow(clippy::cast_precision_loss)] // line counts
    let agreement = if all.is_empty() {
        0.0
    } else {
        seen.len() as f64 / all.len() as f64
    };
    Consensus {
        answers,
        agreed,
        disputed,
        agreement,
        failed,
    }
}

/// Ask every `(provider, model)` the same prompt in parallel, then [`merge`].
pub async fn run(targets: &[(Arc<dyn Provider>, String)], prompt: &str) -> Consensus {
    let calls = targets.iter().map(|(p, m)| async move {
        let r = kor_agent::gitflow::ask(p, m, prompt.to_owned(), 2000).await;
        (format!("{}/{m}", p.id()), r.map_err(|e| e.to_string()))
    });
    let mut answers = Vec::new();
    let mut failed = Vec::new();
    for (model, r) in futures::future::join_all(calls).await {
        match r {
            Ok(text) => answers.push(Answer { model, text }),
            Err(e) => failed.push((model, e)),
        }
    }
    merge(answers, failed)
}

// ---------------------------------------------------------------------------------------------
// Shadow
// ---------------------------------------------------------------------------------------------

/// Word-level F1 of two answers, 0–1 (1 = the same words).
#[allow(clippy::many_single_char_names)] // short names of the math or the protocol
pub fn similarity(a: &str, b: &str) -> f64 {
    let words = |t: &str| -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for w in t
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            *m.entry(w.to_lowercase()).or_insert(0) += 1;
        }
        m
    };
    let (x, y) = (words(a), words(b));
    let common: usize = x
        .iter()
        .map(|(w, n)| (*n).min(*y.get(w).unwrap_or(&0)))
        .sum();
    let (nx, ny) = (x.values().sum::<usize>(), y.values().sum::<usize>());
    if nx == 0 || ny == 0 {
        return if nx == ny { 1.0 } else { 0.0 };
    }
    #[allow(clippy::cast_precision_loss)] // word counts
    let (p, r) = (common as f64 / ny as f64, common as f64 / nx as f64);
    if p + r == 0.0 {
        0.0
    } else {
        2.0 * p * r / (p + r)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowEntry {
    pub at: u64,
    pub main: String,
    pub shadow: String,
    pub agreement: f64,
    /// The "switch to the cheap model" suggestion was shown with this entry.
    #[serde(default)]
    pub suggested: bool,
}

/// Recent shadow history (newest last).
pub fn read_log(path: &Path) -> Vec<ShadowEntry> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append(path: &Path, e: &ShadowEntry) {
    use std::io::Write as _;
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        && let Ok(line) = serde_json::to_string(e)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Average agreement of the last 20 `main`→`shadow` pairs, with the sample count.
pub fn average(log: &[ShadowEntry], main: &str, shadow: &str) -> Option<(f64, usize)> {
    let recent: Vec<f64> = log
        .iter()
        .rev()
        .filter(|e| e.main == main && e.shadow == shadow)
        .take(20)
        .map(|e| e.agreement)
        .collect();
    #[allow(clippy::cast_precision_loss)]
    (!recent.is_empty()).then(|| {
        (
            recent.iter().sum::<f64>() / recent.len() as f64,
            recent.len(),
        )
    })
}

/// Shadow runner the agent calls after every finished task.
pub struct ShadowRunner {
    provider: Arc<dyn Provider>,
    model: String,
    log: PathBuf,
    /// 0–100.
    threshold: u8,
    min_samples: usize,
    l: Localizer,
    pending: Mutex<Option<String>>,
}

impl std::fmt::Debug for ShadowRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShadowRunner")
            .field("model", &self.shadow_ref())
            .field("threshold", &self.threshold)
            .finish_non_exhaustive()
    }
}

impl ShadowRunner {
    pub fn new(
        provider: Arc<dyn Provider>,
        model: String,
        log: PathBuf,
        threshold: u8,
        min_samples: usize,
        l: Localizer,
    ) -> Self {
        Self {
            provider,
            model,
            log,
            threshold,
            min_samples: min_samples.max(1),
            l,
            pending: Mutex::new(None),
        }
    }

    fn shadow_ref(&self) -> String {
        format!("{}/{}", self.provider.id(), self.model)
    }
}

#[async_trait::async_trait]
impl kor_agent::shadow::Shadow for ShadowRunner {
    async fn observe(&self, prompt: &str, answer: &str, main: &str) -> Option<String> {
        let shadow = self.shadow_ref();
        if main == shadow || answer.trim().is_empty() {
            return None;
        }
        let text = kor_agent::gitflow::ask(&self.provider, &self.model, prompt.to_owned(), 2000)
            .await
            .ok()?;
        let agreement = similarity(answer, &text);
        let now = kor_health::system::now_secs();
        let mut log = read_log(&self.log);
        let mut entry = ShadowEntry {
            at: now,
            main: main.to_owned(),
            shadow: shadow.clone(),
            agreement,
            suggested: false,
        };
        log.push(entry.clone());
        let (avg, n) = average(&log, main, &shadow)?;
        let shown_today = log
            .iter()
            .any(|e| e.suggested && e.main == main && now.saturating_sub(e.at) < 86_400);
        let notice = (n >= self.min_samples && avg * 100.0 >= f64::from(self.threshold) && !shown_today).then(|| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let pct = (avg * 100.0).round() as u64;
            self.l.t_args("shadow-suggest", &args!("cheap" => shadow.clone(), "main" => main.to_owned(), "pct" => pct, "count" => n))
        });
        entry.suggested = notice.is_some();
        append(&self.log, &entry);
        if let Some(n) = &notice {
            *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(n.clone());
        }
        notice
    }

    fn take_pending(&self) -> Option<String> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(model: &str, text: &str) -> Answer {
        Answer {
            model: model.into(),
            text: text.into(),
        }
    }

    #[test]
    fn majority_lines_are_agreed_the_rest_disputed() {
        let c = merge(
            vec![
                a("x/a", "Use a mutex.\nLock before write.\nUse Arc."),
                a("x/b", "use a   MUTEX.\nLock before write.\nUse Rc."),
                a("x/c", "Use a mutex.\nUse Rc."),
            ],
            Vec::new(),
        );
        assert_eq!(c.agreed, ["Use a mutex.", "Lock before write.", "Use Rc."]);
        assert_eq!(
            c.disputed,
            [Variant {
                models: vec!["x/a".into()],
                lines: vec!["Use Arc.".into()]
            }]
        );
        assert!((c.agreement - 0.75).abs() < 1e-9, "{}", c.agreement);
        // Two models must both say a line.
        let two = merge(
            vec![a("p", "same\nonly p"), a("q", "same\nonly q")],
            vec![("r".into(), "timeout".into())],
        );
        assert_eq!(two.agreed, ["same"]);
        assert_eq!(two.disputed.len(), 2);
        assert_eq!(two.failed.len(), 1);
        // A single answer agrees with nobody.
        assert!(merge(vec![a("p", "x")], Vec::new()).agreed.is_empty());
    }

    #[test]
    fn similarity_and_running_average() {
        assert!((similarity("the cat sat", "the cat sat") - 1.0).abs() < 1e-9);
        assert!(similarity("the cat sat", "a dog ran") < 0.01);
        let s = similarity("add a null check before the call", "add a null check");
        assert!(s > 0.6 && s < 1.0, "{s}");
        let e = |m: &str, v: f64| ShadowEntry {
            at: 0,
            main: m.into(),
            shadow: "c".into(),
            agreement: v,
            suggested: false,
        };
        let log = vec![e("m", 0.8), e("other", 0.1), e("m", 1.0)];
        let (avg, n) = average(&log, "m", "c").unwrap();
        assert_eq!(n, 2);
        assert!((avg - 0.9).abs() < 1e-9);
        assert!(average(&log, "m", "zzz").is_none());
    }

    struct Echo(kor_providers::ProviderManifest);

    #[async_trait::async_trait]
    impl Provider for Echo {
        fn manifest(&self) -> &kor_providers::ProviderManifest {
            &self.0
        }
        async fn list_models(
            &self,
        ) -> Result<Vec<kor_providers::ModelInfo>, kor_core::ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            _r: kor_providers::ChatRequest,
            _c: kor_providers::CancellationToken,
        ) -> Result<kor_providers::EventStream, kor_core::ProviderError> {
            let text = "Add a null check before the call.".to_owned();
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                kor_core::StreamEvent::TextDelta { text },
            )])))
        }
    }

    #[tokio::test]
    async fn shadow_suggests_switching_once_agreement_holds() {
        use kor_agent::shadow::Shadow as _;
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("shadow.jsonl");
        let p: Arc<dyn Provider> = Arc::new(Echo(
            kor_providers::Registry::builtin()
                .get("local")
                .unwrap()
                .clone(),
        ));
        let r = ShadowRunner::new(
            p,
            "small".into(),
            log.clone(),
            80,
            2,
            Localizer::new(kor_i18n::Lang::En),
        );
        let answer = "Add a null check before the call.";
        assert!(
            r.observe("fix it", answer, "big/model").await.is_none(),
            "one sample is not enough"
        );
        let n = r
            .observe("fix it", answer, "big/model")
            .await
            .expect("two agreeing samples");
        assert!(
            n.contains("local/small") && n.contains("big/model") && n.contains("100"),
            "{n}"
        );
        assert_eq!(r.take_pending().as_deref(), Some(n.as_str()));
        assert!(r.take_pending().is_none());
        assert!(
            r.observe("fix it", answer, "big/model").await.is_none(),
            "once a day"
        );
        assert_eq!(read_log(&log).len(), 3);
        // A very different main answer is recorded as low agreement.
        r.observe("fix it", "Rewrite the whole module in Go.", "other/model")
            .await;
        let (avg, n) = average(&read_log(&log), "other/model", "local/small").unwrap();
        assert!(avg < 0.2 && n == 1, "{avg}");
        // The main model being the shadow model itself is skipped.
        assert!(r.observe("q", answer, "local/small").await.is_none());
        assert_eq!(read_log(&log).len(), 4);
    }
}
