//! Natural-language command routing — the intent layer (§30, ADR 0056).
//!
//! Free text typed in the CLI (`kor "maliyetimi göster"`), the REPL, the TUI or the desktop
//! composer passes through [`Router`] before it reaches the agent:
//!
//! 1. **Deterministic match** against [`catalog::Catalog`] (built-in `assets/intents/intent.toml`
//!    generated from `parity.toml`, plugin `intent.toml` files, the user's learned patterns):
//!    normalized tokens, Turkish/French folding, suffix stems and edit distance ([`matcher`]).
//! 2. **Local embedding similarity** (ADR 0023) when the `embed-e5-small` component is installed
//!    and stage 1 is unsure. Nothing leaves the machine.
//! 3. **Model fallback** only when still ambiguous and the caller allows it (config
//!    `intent.model_fallback`, never in privacy/offline mode unless the model is local); the model
//!    may only pick one of the candidates, it cannot invent a command or change its safety class.
//! 4. Still unsure → one compact question ([`Decision::Choose`]); nothing plausible → the agent.
//!
//! Read-only local commands run at once ([`Decision::Run`]); anything that writes, costs money or
//! goes to the network is shown with the exact command and confirmed ([`Decision::Confirm`]).
//! Routed commands run through their normal code path, so every approval (sandbox, per-file
//! review, destructive-operation prompts) still applies: natural language bypasses nothing.

pub mod catalog;
pub mod matcher;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{Datelike, Local, TimeZone, Timelike};
use kor_agent::semantic::Embedder;
use kor_core::{Message, Role};
use kor_providers::{CancellationToken, ChatRequest, Provider};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use catalog::{Catalog, Intent, Kind, Param, Run, Source};

/// Stage 1 score (√(recall·precision) of the best phrase) from which a command is taken.
/// Tuned on `tests/intents` (ADR 0056).
pub const ACCEPT: f32 = 0.5;
/// Below `ACCEPT` but at least this: plausible, a candidate for the embedding/model stages.
pub const PLAUSIBLE: f32 = 0.3;
/// Two commands whose ranking keys are this close are a tie (one question).
const TIE: f32 = 1e-4;
/// The second-best phrase breaks ties between commands with the same best phrase score.
const SECOND: f32 = 0.02;
/// A command that takes the kind of value the text contains (a path, a number) ranks higher.
const VALUE_BONUS: f32 = 0.03;
/// Embedding cosine needed on its own, and its lead over the next command.
const EMB_ALONE: f32 = 0.9;
const EMB_AGREE: f32 = 0.85;
const EMB_MARGIN: f32 = 0.015;
/// Longer text is a task, not a command.
const MAX_CHARS: usize = 400;
/// Candidates offered to the model or in a question.
const MAX_CANDIDATES: usize = 6;
const MAX_LEARNED: usize = 2000;
/// Words that join steps of a pipeline (folded).
const SEPARATORS: &[&str] = &[
    " ve sonra ",
    " sonra da ",
    " sonra ",
    " ardindan ",
    " daha sonra ",
    " and then ",
    " then ",
    " afterwards ",
    " puis ",
    " ensuite ",
    " et ensuite ",
    "; ",
    ", ",
];

/// How a plan was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Keyword,
    Learned,
    Embedding,
    Model,
}

/// A command ready to run, with the exact text shown to the user.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// Catalog key: `kor explain`.
    pub command: String,
    /// Arguments after `kor` (`["explain", "src/main.rs"]`); for a plugin `/cmd`, its arguments.
    pub argv: Vec<String>,
    /// `kor explain src/main.rs`
    pub line: String,
    /// `/explain src/main.rs` (REPL/TUI), from parity.toml's TUI path when it has one.
    pub slash: String,
    pub desktop: Option<String>,
    /// parity.toml description (Turkish).
    pub name: String,
    pub run: Run,
    pub via: Via,
    pub score: f32,
    pub params: BTreeMap<String, String>,
    /// Required arguments neither the text nor the context gave.
    pub missing: Vec<String>,
    /// Rough model cost from catalog prices (always an estimate, shown with "≈").
    pub usd: Option<f64>,
    pub eta_s: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Step {
    Command(Plan),
    Agent(String),
}

/// What to do with the text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    /// Not a command: the normal agent task.
    Agent,
    /// Local read-only command: run it now.
    Run { plan: Plan },
    /// Show `plan.line` and run on confirmation.
    Confirm { plan: Plan },
    /// A required argument is missing: ask for it (`plan.missing`).
    Missing { plan: Plan },
    /// Several commands fit: one compact question. The agent stays an option.
    Choose { options: Vec<Plan> },
    /// `a, then b`: steps in order, confirmed once.
    Pipeline { steps: Vec<Step> },
    /// `her pazartesi …`: a routine (`kor routines`), confirmed first.
    Schedule {
        prompt: String,
        interval_minutes: u64,
        /// Unix ms of the first run.
        first_run: u64,
        plan: Option<Plan>,
    },
}

/// What the surface knows besides the text (§30.1 "parameters from context").
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Context {
    /// File selected in the desktop file tree / editor.
    pub selection: Option<String>,
    /// Clipboard text; `read_clipboard` reads it lazily when an argument is missing.
    pub clipboard: Option<String>,
    pub read_clipboard: bool,
    /// Open browser tab (side panel).
    pub tab_url: Option<String>,
    /// Session model, for the "≈ $" estimate.
    pub model: Option<String>,
    /// `intent.auto_run = false` turns every `Run` into `Confirm`.
    pub confirm_all: bool,
}

// ---------------------------------------------------------------------------------------------
// Learned patterns (§30.2): `~/.kor/intents/learned.json`, local only.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub text: String,
    pub command: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Learned {
    #[serde(default)]
    pub patterns: Vec<Pattern>,
    /// Last value per argument kind (`domain`, `path`, `url`): fills a missing argument.
    #[serde(default)]
    pub last: BTreeMap<String, String>,
}

/// Normalized key of a text (folded content tokens).
pub fn key(text: &str) -> String {
    let mut words: Vec<String> = matcher::words(text)
        .iter()
        .filter(|w| matcher::is_value_like(w))
        .map(|w| (*w).to_owned())
        .collect();
    words.splice(0..0, matcher::tokens(text));
    words.join(" ")
}

impl Learned {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// # Errors
    /// When the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("kor-tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    }

    /// Remember `text` → `command` (a correction or `kor intents add`); replaces an older one.
    pub fn learn(&mut self, text: &str, command: &str, now: u64) {
        let k = key(text);
        self.patterns.retain(|p| key(&p.text) != k);
        self.patterns.push(Pattern {
            text: text.trim().to_owned(),
            command: command.to_owned(),
            at: now,
        });
        if self.patterns.len() > MAX_LEARNED {
            let drop = self.patterns.len() - MAX_LEARNED;
            self.patterns.drain(..drop);
        }
    }

    /// `true` when a pattern for `text` was removed.
    pub fn forget(&mut self, text: &str) -> bool {
        let k = key(text);
        let before = self.patterns.len();
        self.patterns.retain(|p| key(&p.text) != k);
        before != self.patterns.len()
    }
}

/// `<kor home>/intents/learned.json`.
pub fn learned_file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("intents").join("learned.json"))
}

// ---------------------------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------------------------

/// One scored command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub command: String,
    pub score: f32,
    /// Ranking key: `score` plus a small share of the second-best phrase (tie-break).
    #[serde(skip)]
    pub key: f32,
    /// The phrase that matched best.
    pub phrase: String,
}

#[derive(Debug, Clone, Default)]
pub struct Router {
    pub catalog: Catalog,
    pub learned: Learned,
    /// Every content word of the catalog (out-of-vocabulary words weigh more).
    vocab: Vec<String>,
    /// Where phrase embeddings are cached (`<kor home>/intents`); `None` keeps them in memory.
    emb_cache: Option<PathBuf>,
}

/// Optional later stages.
#[derive(Default)]
pub struct Fallbacks<'a> {
    pub embedder: Option<&'a dyn Embedder>,
    /// Provider and `provider/model` for the model fallback; `None` = not allowed.
    pub model: Option<(Arc<dyn Provider>, String)>,
}

impl std::fmt::Debug for Fallbacks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fallbacks")
            .field("embedder", &self.embedder.map(Embedder::name))
            .field("model", &self.model.as_ref().map(|m| &m.1))
            .finish()
    }
}

enum Single {
    Found(Plan),
    Tie(Vec<Plan>),
    Unsure(Vec<Candidate>),
    Nothing,
}

impl Router {
    /// Built-in catalog, the project's enabled plugins and the user's learned patterns.
    pub fn load(root: Option<&Path>) -> Self {
        let mut catalog = Catalog::builtin();
        if let Some(home) = dirs::home_dir() {
            let plugins = kor_agent::plugins::plugins_root(&home);
            for (id, dir) in kor_agent::plugins::installed(&plugins) {
                if !kor_agent::plugins::is_enabled(&plugins, &id) {
                    continue;
                }
                for f in [
                    dir.join("intent.toml"),
                    kor_agent::plugins::meta_dir(&dir).join("intent.toml"),
                ] {
                    if f.is_file() {
                        catalog.add_plugin(&id, &f);
                    }
                }
            }
        }
        if let Some(r) = root {
            let f = r.join(".kor").join("intent.toml");
            if f.is_file() {
                catalog.add_plugin("project", &f);
            }
        }
        let learned = learned_file()
            .map(|p| Learned::load(&p))
            .unwrap_or_default();
        let mut r = Self::new(catalog, learned);
        r.emb_cache = crate::kor_home().map(|h| h.join("intents"));
        r
    }

    pub fn new(mut catalog: Catalog, learned: Learned) -> Self {
        for p in &learned.patterns {
            if let Some(i) = catalog.get_mut(&p.command) {
                i.add_phrase("user", &p.text);
            }
        }
        let mut vocab: Vec<String> = catalog
            .intents
            .iter()
            .flat_map(|i| i.vocab.iter().cloned())
            .collect();
        vocab.sort_unstable();
        vocab.dedup();
        Self {
            catalog,
            learned,
            vocab,
            emb_cache: None,
        }
    }

    /// Commands ranked by stage 1 (best phrase per command), best first.
    pub fn rank(&self, text: &str) -> Vec<Candidate> {
        let input = matcher::tokens(text);
        // Words Kor's catalog never uses (`parser`, `tooltip`) weigh more against a command.
        let weights: Vec<f32> = input
            .iter()
            .map(|t| {
                if matcher::best(t, &self.vocab) < 0.7 {
                    matcher::OOV_WEIGHT
                } else {
                    1.0
                }
            })
            .collect();
        let kinds = value_kinds(text);
        let mut out: Vec<Candidate> = self
            .catalog
            .intents
            .iter()
            .filter(|i| i.run != Run::Never)
            .filter_map(|i| {
                let mut scored: Vec<(f32, &String)> = i
                    .phrases
                    .iter()
                    .map(|p| (matcher::score(&input, &weights, &p.tokens), &p.text))
                    .collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0));
                let (best, phrase) = *scored.first()?;
                if best <= 0.0 {
                    return None;
                }
                let bonus = if i.params.iter().any(|p| kinds.contains(&p.kind)) {
                    VALUE_BONUS
                } else {
                    0.0
                };
                let second = scored.get(1).map_or(0.0, |x| x.0);
                Some(Candidate {
                    command: i.command.clone(),
                    score: best + bonus,
                    key: best + bonus + SECOND * second,
                    phrase: phrase.clone(),
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.key
                .total_cmp(&a.key)
                .then_with(|| a.command.cmp(&b.command))
        });
        out
    }

    fn learned_command(&self, text: &str) -> Option<&str> {
        let k = key(text);
        self.learned
            .patterns
            .iter()
            .rev()
            .find(|p| key(&p.text) == k)
            .map(|p| p.command.as_str())
            .filter(|c| self.catalog.get(c).is_some_and(|i| i.run != Run::Never))
    }

    fn single(&self, text: &str, ctx: &Context) -> Single {
        if let Some(cmd) = self.learned_command(text) {
            return self
                .plan(cmd, text, ctx, Via::Learned, 1.0)
                .map_or(Single::Nothing, Single::Found);
        }
        let ranked = self.rank(text);
        if matcher::tokens(text).is_empty() {
            return Single::Nothing;
        }
        // Below ACCEPT the later stages decide; the embedding stage also sees text stage 1 missed.
        if let Some(top) = ranked.first().filter(|c| c.score >= ACCEPT) {
            let tied: Vec<&Candidate> = ranked
                .iter()
                .take_while(|c| top.key - c.key < TIE)
                .collect();
            if tied.len() > 1 {
                let plans: Vec<Plan> = tied
                    .iter()
                    .filter_map(|c| self.plan(&c.command, text, ctx, Via::Keyword, c.score))
                    .collect();
                return Single::Tie(plans);
            }
            return self
                .plan(&top.command, text, ctx, Via::Keyword, top.score)
                .map_or(Single::Nothing, Single::Found);
        }
        Single::Unsure(
            ranked
                .into_iter()
                .take_while(|c| c.score >= PLAUSIBLE)
                .take(MAX_CANDIDATES)
                .collect(),
        )
    }

    /// Stage 1 only (no embedding, no model): what the accuracy fixtures measure.
    pub fn route(&self, text: &str, ctx: &Context) -> Decision {
        self.route_with(text, ctx, &mut |_, _| Resolved::Agent)
    }

    /// All stages: deterministic, local embedding, then the model when allowed.
    pub async fn route_full(&self, text: &str, ctx: &Context, fb: &Fallbacks<'_>) -> Decision {
        // Async stages run first on the unsure single text; the sync walker then uses the answer.
        let pre = self.pre_resolve(text, ctx, fb).await;
        self.route_with(text, ctx, &mut |t, _| {
            pre.iter()
                .find(|(seg, _)| seg == t)
                .map_or(Resolved::Agent, |(_, r)| r.clone())
        })
    }

    /// Embedding and model answers for every unsure segment of `text`.
    async fn pre_resolve(
        &self,
        text: &str,
        ctx: &Context,
        fb: &Fallbacks<'_>,
    ) -> Vec<(String, Resolved)> {
        let mut segs =
            vec![strip_schedule(text).map_or_else(|| text.trim().to_owned(), |s| s.rest)];
        if let Some(parts) = split_steps(&segs[0]) {
            segs.extend(parts);
        }
        let mut out = Vec::new();
        for seg in segs {
            if let Single::Unsure(c) = self.single(&seg, ctx) {
                let r = self.resolve_unsure(&seg, &c, fb).await;
                out.push((seg, r));
            }
        }
        out
    }

    async fn resolve_unsure(
        &self,
        text: &str,
        unsure: &[Candidate],
        fb: &Fallbacks<'_>,
    ) -> Resolved {
        let det_top = unsure.first();
        let mut unsure = unsure.to_vec();
        if let Some(e) = fb.embedder
            && let Ok(Some((cmd, cos, margin))) = self.embed_best(e, text).await
        {
            let agrees = det_top.is_some_and(|c| c.command == cmd);
            if (cos >= EMB_ALONE && margin >= EMB_MARGIN) || (agrees && cos >= EMB_AGREE) {
                return Resolved::Pick(cmd, Via::Embedding, cos, BTreeMap::new());
            }
            // A semantic near-miss is still a candidate for the model.
            if cos >= EMB_AGREE && !unsure.iter().any(|c| c.command == cmd) {
                unsure.push(Candidate {
                    command: cmd,
                    score: 0.0,
                    key: 0.0,
                    phrase: String::new(),
                });
            }
        }
        // The model is asked only about plausible text: an ordinary task costs no extra call.
        if let Some((provider, model)) = fb.model.as_ref().filter(|_| !unsure.is_empty()) {
            let names: Vec<&str> = unsure.iter().map(|c| c.command.as_str()).collect();
            match self.ask_model(provider.as_ref(), model, text, &names).await {
                Ok(ModelAnswer::Command {
                    command,
                    params,
                    sure: true,
                }) => {
                    return Resolved::Pick(command, Via::Model, 0.0, params);
                }
                Ok(ModelAnswer::Command { command, .. }) => {
                    if let Some(i) = unsure.iter().position(|x| x.command == command) {
                        let first = unsure.remove(i);
                        unsure.insert(0, first);
                    }
                    return Resolved::Ask(unsure);
                }
                Ok(ModelAnswer::None) => return Resolved::Agent,
                Err(_) => {}
            }
        }
        Resolved::Agent
    }

    fn route_with(
        &self,
        text: &str,
        ctx: &Context,
        unsure: &mut dyn FnMut(&str, &[Candidate]) -> Resolved,
    ) -> Decision {
        let text = text.trim();
        if text.is_empty() || text.starts_with('/') || text.chars().count() > MAX_CHARS {
            return Decision::Agent;
        }
        if let Some(s) = strip_schedule(text) {
            let plan = match self.decide_single(&s.rest, ctx, unsure) {
                Decision::Run { plan } | Decision::Confirm { plan } => Some(plan),
                _ => None,
            };
            return Decision::Schedule {
                prompt: plan
                    .as_ref()
                    .map_or_else(|| s.rest.clone(), |p| format!("{} (`{}`)", s.rest, p.line)),
                interval_minutes: s.minutes,
                first_run: s.first_run,
                plan,
            };
        }
        if let Some(parts) = split_steps(text) {
            let steps: Vec<Step> = parts
                .iter()
                .filter_map(|p| match self.decide_single(p, ctx, unsure) {
                    Decision::Run { plan } | Decision::Confirm { plan } => {
                        Some(Step::Command(plan))
                    }
                    // "lütfen, …" or "ok, …" is not a step of its own.
                    _ if matcher::tokens(p).len() < 2 => None,
                    _ => Some(Step::Agent(p.clone())),
                })
                .collect();
            if steps.len() >= 2 && steps.iter().any(|s| matches!(s, Step::Command(_))) {
                return Decision::Pipeline { steps };
            }
        }
        self.decide_single(text, ctx, unsure)
    }

    fn decide_single(
        &self,
        text: &str,
        ctx: &Context,
        unsure: &mut dyn FnMut(&str, &[Candidate]) -> Resolved,
    ) -> Decision {
        let plan = match self.single(text, ctx) {
            Single::Found(p) => p,
            Single::Tie(options) => return Decision::Choose { options },
            Single::Nothing => return Decision::Agent,
            Single::Unsure(c) => match unsure(text, &c) {
                Resolved::Agent => return Decision::Agent,
                Resolved::Ask(c) => {
                    let options: Vec<Plan> = c
                        .iter()
                        .take(3)
                        .filter_map(|c| self.plan(&c.command, text, ctx, Via::Keyword, c.score))
                        .collect();
                    return if options.is_empty() {
                        Decision::Agent
                    } else {
                        Decision::Choose { options }
                    };
                }
                Resolved::Pick(cmd, via, score, params) => {
                    let Some(mut p) = self.plan(&cmd, text, ctx, via, score) else {
                        return Decision::Agent;
                    };
                    // Model-extracted values fill only declared, still-empty arguments.
                    if let Some(i) = self.catalog.get(&cmd) {
                        for (k, v) in params {
                            if i.params.iter().any(|x| x.name == k) && !p.params.contains_key(&k) {
                                p.params.insert(k, v);
                            }
                        }
                        p = finish(i, p);
                    }
                    p
                }
            },
        };
        if !plan.missing.is_empty() {
            Decision::Missing { plan }
        } else if plan.run == Run::Auto && !ctx.confirm_all {
            Decision::Run { plan }
        } else {
            Decision::Confirm { plan }
        }
    }

    /// A plan for `command`, its arguments taken from the text, then the context.
    pub fn plan(
        &self,
        command: &str,
        text: &str,
        ctx: &Context,
        via: Via,
        score: f32,
    ) -> Option<Plan> {
        let i = self.catalog.get(command).filter(|i| i.run != Run::Never)?;
        let params = fill(i, text, ctx, &self.learned);
        let p = Plan {
            command: i.command.clone(),
            argv: Vec::new(),
            line: String::new(),
            slash: String::new(),
            desktop: i.desktop.clone(),
            name: i.name.clone(),
            run: i.run,
            via,
            score,
            params,
            missing: Vec::new(),
            usd: estimate_usd(i.tokens, ctx.model.as_deref()),
            eta_s: i.eta_s,
        };
        Some(finish(i, p))
    }

    /// Learn `text` → `command` and save (a correction, a chosen option, `kor intents add`).
    ///
    /// # Errors
    /// Unknown command or the file cannot be written.
    pub fn learn(&mut self, text: &str, command: &str) -> Result<(), String> {
        let Some(i) = self
            .catalog
            .get_mut(command)
            .filter(|i| i.run != Run::Never)
        else {
            return Err(format!("unknown command: {command}"));
        };
        i.add_phrase("user", text);
        for t in matcher::tokens(text) {
            if let Err(k) = self.vocab.binary_search(&t) {
                self.vocab.insert(k, t);
            }
        }
        self.learned.learn(text, command, crate::cost::now_ms());
        match learned_file() {
            Some(f) => self.learned.save(&f),
            None => Ok(()),
        }
    }

    /// Remember the arguments of a plan that ran (fills "last used domain" next time).
    pub fn remember(&mut self, plan: &Plan) {
        let Some(i) = self.catalog.get(&plan.command) else {
            return;
        };
        let mut changed = false;
        for p in &i.params {
            if matches!(p.kind, Kind::Domain | Kind::Url | Kind::Path)
                && let Some(v) = plan.params.get(&p.name)
            {
                let k = kind_key(p.kind).to_owned();
                changed |= self.learned.last.insert(k, v.clone()).as_ref() != Some(v);
            }
        }
        if changed && let Some(f) = learned_file() {
            let _ = self.learned.save(&f);
        }
    }

    // ---- stage 2: local embeddings ---------------------------------------------------------

    /// Best command by cosine similarity: (command, cosine, lead over the next command).
    async fn embed_best(
        &self,
        e: &dyn Embedder,
        text: &str,
    ) -> Result<Option<(String, f32, f32)>, String> {
        let index = self.embedding_index(e).await?;
        let q = e
            .embed_query(e.default_model(), &[text.to_owned()])
            .await
            .map_err(|x| x.message)?
            .pop()
            .ok_or("no vector")?;
        let mut best: BTreeMap<&str, f32> = BTreeMap::new();
        for (cmd, v) in &index.vectors {
            let c = cosine(&q, v);
            let e = best.entry(cmd.as_str()).or_insert(f32::MIN);
            *e = e.max(c);
        }
        let mut ranked: Vec<(&str, f32)> = best.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(ranked.first().map(|(c, s)| {
            let next = ranked.get(1).map_or(0.0, |x| x.1);
            ((*c).to_owned(), *s, s - next)
        }))
    }

    async fn embedding_index(&self, e: &dyn Embedder) -> Result<Arc<EmbIndex>, String> {
        let mut texts = Vec::new();
        let mut cmds = Vec::new();
        for i in self.catalog.intents.iter().filter(|i| i.run != Run::Never) {
            for p in &i.phrases {
                texts.push(p.text.clone());
                cmds.push(i.command.clone());
            }
        }
        let mut h = Sha256::new();
        h.update(e.name().as_bytes());
        h.update(e.default_model().as_bytes());
        for (t, c) in texts.iter().zip(&cmds) {
            h.update(c.as_bytes());
            h.update([0]);
            h.update(t.as_bytes());
            h.update([1]);
        }
        let hash: [u8; 32] = h.finalize().into();
        if let Some(i) = lock(&EMB).as_ref().filter(|i| i.hash == hash) {
            return Ok(i.clone());
        }
        let file = self.emb_cache.as_ref().map(|d| d.join("embeddings.bin"));
        if let Some(i) = file
            .as_deref()
            .and_then(|f| EmbIndex::read(f, &hash, &cmds))
        {
            let i = Arc::new(i);
            *lock(&EMB) = Some(i.clone());
            return Ok(i);
        }
        let mut vectors = Vec::with_capacity(texts.len());
        for (chunk, cs) in texts.chunks(64).zip(cmds.chunks(64)) {
            let vs = e
                .embed(e.default_model(), chunk)
                .await
                .map_err(|x| x.message)?;
            vectors.extend(cs.iter().cloned().zip(vs));
        }
        let i = Arc::new(EmbIndex { hash, vectors });
        if let Some(f) = &file {
            let _ = i.write(f);
        }
        *lock(&EMB) = Some(i.clone());
        Ok(i)
    }

    // ---- stage 3: model --------------------------------------------------------------------

    async fn ask_model(
        &self,
        provider: &dyn Provider,
        model: &str,
        text: &str,
        candidates: &[&str],
    ) -> Result<ModelAnswer, String> {
        let mut list = String::new();
        for c in candidates {
            let Some(i) = self.catalog.get(c) else {
                continue;
            };
            let params: Vec<String> = i
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{} ({:?}{})",
                        p.name,
                        p.kind,
                        if p.required { ", required" } else { "" }
                    )
                })
                .collect();
            let _ = writeln!(
                list,
                "- {}: {} | args: {}",
                i.command,
                i.name,
                params.join(", ")
            );
        }
        let user = format!(
            "User request (any language): {text}\n\nCandidate Kor commands:\n{list}\nIf the request asks for exactly one of these commands, answer {{\"command\": \"<command>\", \"params\": {{\"<arg>\": \"<value from the request>\"}}, \"sure\": true|false}}. If it is an ordinary task for a coding agent instead, answer {{\"command\": null}}. JSON only."
        );
        let req = ChatRequest {
            model: model.to_owned(),
            system: Some("You route a user's request to a command of the Kor CLI. Never invent commands or values. Reply with one JSON object only.".into()),
            messages: vec![Message::text(Role::User, &user)],
            tools: Vec::new(),
            max_tokens: Some(200),
            temperature: Some(0.0),
            reasoning: None,
        };
        let stream = provider
            .complete(req, CancellationToken::new())
            .await
            .map_err(|e| e.message)?;
        let reply = kor_agent::subagent::collect_text(stream)
            .await
            .map_err(|e| e.message)?;
        parse_model_answer(&reply, candidates)
    }
}

#[derive(Debug, Clone)]
enum Resolved {
    Agent,
    Ask(Vec<Candidate>),
    Pick(String, Via, f32, BTreeMap<String, String>),
}

#[derive(Debug, PartialEq, Eq)]
enum ModelAnswer {
    None,
    Command {
        command: String,
        params: BTreeMap<String, String>,
        sure: bool,
    },
}

/// The model's JSON, accepted only when it names one of the candidates.
fn parse_model_answer(reply: &str, candidates: &[&str]) -> Result<ModelAnswer, String> {
    let (Some(open), Some(close)) = (reply.find('{'), reply.rfind('}')) else {
        return Err("no JSON in the reply".into());
    };
    let v: serde_json::Value =
        serde_json::from_str(reply.get(open..=close).unwrap_or("")).map_err(|e| e.to_string())?;
    match v.get("command") {
        Some(serde_json::Value::Null) | None => Ok(ModelAnswer::None),
        Some(serde_json::Value::String(c)) if candidates.contains(&c.as_str()) => {
            let params = v
                .get("params")
                .and_then(serde_json::Value::as_object)
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| {
                            let s = match v {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => return None,
                            };
                            (!s.is_empty() && s.len() <= 512 && !s.contains(['\n', '\r', '\0']))
                                .then(|| (k.clone(), s))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(ModelAnswer::Command {
                command: c.clone(),
                params,
                sure: v
                    .get("sure")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        }
        Some(_) => Err("the model named a command outside the candidates".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Embedding index cache: `<kor home>/intents/embeddings.bin`
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct EmbIndex {
    hash: [u8; 32],
    vectors: Vec<(String, Vec<f32>)>,
}

static EMB: Mutex<Option<Arc<EmbIndex>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

const EMB_MAGIC: &[u8; 8] = b"KORINT01";

impl EmbIndex {
    /// Layout: magic, sha256 of the phrase list, u32 count, u32 dims, then `count × dims` f32 LE.
    fn write(&self, path: &Path) -> std::io::Result<()> {
        let dims = self.vectors.first().map_or(0, |v| v.1.len());
        let mut buf = Vec::with_capacity(48 + self.vectors.len() * dims * 4);
        buf.extend_from_slice(EMB_MAGIC);
        buf.extend_from_slice(&self.hash);
        buf.extend_from_slice(&u32::try_from(self.vectors.len()).unwrap_or(0).to_le_bytes());
        buf.extend_from_slice(&u32::try_from(dims).unwrap_or(0).to_le_bytes());
        for (_, v) in &self.vectors {
            for x in v {
                buf.extend_from_slice(&x.to_le_bytes());
            }
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("kor-tmp");
        std::fs::write(&tmp, buf)?;
        std::fs::rename(&tmp, path)
    }

    fn read(path: &Path, hash: &[u8; 32], cmds: &[String]) -> Option<Self> {
        let b = std::fs::read(path).ok()?;
        let u32_at = |i: usize| {
            b.get(i..i + 4)
                .and_then(|s| s.try_into().ok())
                .map(u32::from_le_bytes)
        };
        if b.get(..8)? != EMB_MAGIC || b.get(8..40)? != hash {
            return None;
        }
        let (n, dims) = (u32_at(40)? as usize, u32_at(44)? as usize);
        if n != cmds.len() || dims == 0 || b.len() != 48 + n * dims * 4 {
            return None;
        }
        let vectors = cmds
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let start = 48 + k * dims * 4;
                let v = b[start..start + dims * 4]
                    .chunks_exact(4)
                    .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                    .collect();
                (c.clone(), v)
            })
            .collect();
        Some(Self {
            hash: *hash,
            vectors,
        })
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

// ---------------------------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------------------------

/// Argument names that take every leftover word, not just one.
const REST: &[&str] = &[
    "query",
    "text",
    "prompt",
    "question",
    "message",
    "description",
    "task",
];

fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::Path => "path",
        Kind::Text => "text",
        Kind::Number => "number",
        Kind::Url => "url",
        Kind::Domain => "domain",
    }
}

/// Kinds of values the text names (`src/a.rs` → path, `42` → number, `parseConfig` → text).
fn value_kinds(text: &str) -> Vec<Kind> {
    let mut out = Vec::new();
    for w in matcher::words(text)
        .into_iter()
        .filter(|w| matcher::is_value_like(w))
    {
        let k = if is_url(w) {
            Kind::Url
        } else if is_path(w) {
            Kind::Path
        } else if w.chars().all(|c| c.is_ascii_digit()) {
            Kind::Number
        } else {
            Kind::Text
        };
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

fn is_url(w: &str) -> bool {
    (w.starts_with("https://") || w.starts_with("http://")) && w.len() > 8
}

fn is_path(w: &str) -> bool {
    if is_url(w) {
        return false;
    }
    w.contains(['/', '\\'])
        || w.starts_with('~')
        || w.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        })
}

fn is_domain(w: &str) -> bool {
    let w = w.trim_end_matches('/');
    let labels: Vec<&str> = w.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels.last().is_some_and(|t| {
            (2..=24).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphabetic())
        })
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#', ':']).next()?;
    is_domain(host).then(|| host.to_owned())
}

/// Words of the text that may be values for `i`, in order, each with a preference rank:
/// 0 value-like (`src/a.rs`, `parseConfig`), 1 a word the command never uses, 2 a word only one of
/// its phrases uses (an example value or a rare synonym). Stopwords and the command's common
/// words are never values.
fn leftovers(i: &Intent, text: &str) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = Vec::new();
    for w in matcher::words(text) {
        if matcher::is_value_like(w) {
            out.push((w.to_owned(), 0));
            continue;
        }
        let folded = matcher::fold(w);
        let rank = folded
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.chars().count() >= 2 && !matcher::is_stopword(t))
            .filter_map(|t| {
                if matcher::best(t, &i.strong) >= 0.8 {
                    None
                } else if matcher::best(t, &i.vocab) >= 0.7 {
                    Some(2)
                } else {
                    Some(1)
                }
            })
            .min();
        if let Some(r) = rank {
            out.push((w.to_owned(), r));
        }
    }
    out
}

fn fill(i: &Intent, text: &str, ctx: &Context, learned: &Learned) -> BTreeMap<String, String> {
    let mut words = leftovers(i, text);
    let mut out = BTreeMap::new();
    let take = |pred: &dyn Fn(&str) -> bool, words: &mut Vec<(String, u8)>| {
        words
            .iter()
            .position(|(w, _)| pred(w))
            .map(|k| words.remove(k).0)
    };
    // Typed values first, so a free-text argument does not swallow a path or a number.
    let mut order: Vec<&Param> = i.params.iter().collect();
    order.sort_by_key(|p| matches!(p.kind, Kind::Text));
    let mut clipboard: Option<Option<String>> = ctx.clipboard.clone().map(Some);
    let mut clip = || -> Option<String> {
        if clipboard.is_none() {
            clipboard = Some(ctx.read_clipboard.then(read_clipboard).flatten());
        }
        clipboard
            .clone()
            .flatten()
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty() && c.len() <= 2048 && !c.contains('\n'))
    };
    for p in order {
        let from_text = match p.kind {
            Kind::Url => take(&is_url, &mut words),
            Kind::Domain => {
                take(&|w| is_domain(w) || is_url(w), &mut words).map(|w| host_of(&w).unwrap_or(w))
            }
            Kind::Path => take(&is_path, &mut words),
            Kind::Number => take(
                &|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()),
                &mut words,
            ),
            Kind::Text if REST.contains(&p.name.as_str()) => {
                let all: Vec<String> = words
                    .drain(..)
                    .filter(|(_, r)| *r < 2)
                    .map(|(w, _)| w)
                    .collect();
                (!all.is_empty()).then(|| all.join(" "))
            }
            // The most value-like word first (`parseConfig`, `STRIPE_KEY`), then in order.
            Kind::Text => (0..=2u8)
                .find_map(|r| words.iter().position(|(_, x)| *x == r))
                .map(|k| words.remove(k).0),
        };
        let v = from_text.or_else(|| match p.kind {
            Kind::Path => ctx
                .selection
                .clone()
                .or_else(|| clip().filter(|c| is_path(c))),
            Kind::Url => ctx.tab_url.clone().or_else(|| clip().filter(|c| is_url(c))),
            Kind::Domain => ctx
                .tab_url
                .as_deref()
                .and_then(host_of)
                .or_else(|| learned.last.get("domain").cloned())
                .or_else(|| clip().filter(|c| is_domain(c))),
            Kind::Text | Kind::Number => None,
        });
        if let Some(v) = v.or_else(|| p.default.clone()) {
            out.insert(p.name.clone(), v);
        }
    }
    out
}

/// Text of the clipboard (only read when an argument is missing and the surface allows it).
fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// argv, display line, slash form and missing arguments of a plan.
fn finish(i: &Intent, mut p: Plan) -> Plan {
    let mut argv: Vec<String> = i
        .command
        .split_whitespace()
        .skip(usize::from(i.command.starts_with("kor")))
        .map(str::to_owned)
        .collect();
    // `kor cost --export` style catalog keys carry the flag; its value follows it.
    let mut missing = Vec::new();
    for prm in &i.params {
        match p.params.get(&prm.name) {
            Some(v) => {
                if let Some(f) = &prm.flag
                    && argv.last() != Some(f)
                {
                    argv.push(f.clone());
                }
                argv.push(v.clone());
            }
            None if prm.required => missing.push(prm.name.clone()),
            None => {}
        }
    }
    let quoted: Vec<String> = argv.iter().map(|a| quote(a)).collect();
    let head = if i.command.starts_with("kor") {
        "kor "
    } else {
        ""
    };
    format!("{head}{}", quoted.join(" "))
        .trim()
        .clone_into(&mut p.line);
    p.slash = if i.command.starts_with('/') {
        format!("{} {}", i.command, quoted.join(" "))
            .trim()
            .to_owned()
    } else {
        format!("/{}", quoted.join(" "))
    };
    p.argv = argv;
    p.missing = missing;
    p
}

fn quote(a: &str) -> String {
    if !a.is_empty()
        && a.chars()
            .all(|c| c.is_alphanumeric() || "-_./:@=,+~\\".contains(c))
    {
        a.to_owned()
    } else {
        format!("\"{}\"", a.replace('"', "\\\""))
    }
}

/// Catalog price of `tokens` on `model` (3:1 input:output blend, like the cost router).
fn estimate_usd(tokens: Option<u64>, model: Option<&str>) -> Option<f64> {
    let (tokens, model) = (tokens?, model?);
    let (i, o) = crate::cost::Prices::load().get(model)?;
    #[allow(clippy::cast_precision_loss)] // small numbers
    Some((3.0 * i + o) / 4.0 * tokens as f64 / 1_000_000.0)
}

/// Provider and model id for the model fallback: `[routing] simple_edit` (the router's cheap
/// slot) or the default model; `None` when not allowed or not configured.
pub fn fallback_model(
    l: &kor_i18n::Localizer,
    loaded: &kor_config::Loaded,
) -> Option<(Arc<dyn Provider>, String)> {
    let c = &loaded.config;
    let cheap = c.routing.simple_edit.trim();
    let model = if cheap.is_empty() {
        c.agent.model.trim()
    } else {
        cheap
    };
    if model.is_empty() || !model_allowed(c, model) {
        return None;
    }
    crate::oneshot_provider(l, loaded, Some(model)).ok()
}

/// Whether the model fallback may run: config switch, and in privacy/offline mode only a local model.
pub fn model_allowed(config: &kor_config::Config, model: &str) -> bool {
    let local = {
        let p = model.split('/').next().unwrap_or_default();
        p == "local" || p.starts_with("local-") || p == "ollama"
    };
    config.intent.model_fallback && (local || !(config.privacy.mode || config.offline.enabled))
}

// ---------------------------------------------------------------------------------------------
// Pipelines and schedules
// ---------------------------------------------------------------------------------------------

/// `a, then b` → `["a", "b"]` (on the original text, so values keep their case).
fn split_steps(text: &str) -> Option<Vec<String>> {
    // Byte offsets of the folded text do not map back when folding changes lengths, so match
    // separators on a lowercase copy whose ASCII separators keep their positions.
    let lower: String = text
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for sep in SEPARATORS {
        let mut from = 0;
        while let Some(k) = lower[from..].find(sep) {
            let at = from + k;
            if !cuts.iter().any(|(a, b)| at < *b && at + sep.len() > *a) {
                cuts.push((at, at + sep.len()));
            }
            from = at + sep.len();
        }
    }
    if cuts.is_empty() {
        return None;
    }
    cuts.sort_unstable();
    let mut parts = Vec::new();
    let mut start = 0;
    for (a, b) in cuts {
        parts.push(text[start..a].trim().to_owned());
        start = b;
    }
    parts.push(text[start..].trim().to_owned());
    parts.retain(|p| !p.is_empty());
    (parts.len() >= 2).then_some(parts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduleExpr {
    rest: String,
    minutes: u64,
    first_run: u64,
}

const WEEKDAYS: &[(&[&str], u32)] = &[
    (&["pazartesi", "monday", "mondays", "lundi", "lundis"], 0),
    (&["sali", "tuesday", "tuesdays", "mardi", "mardis"], 1),
    (
        &[
            "carsamba",
            "wednesday",
            "wednesdays",
            "mercredi",
            "mercredis",
        ],
        2,
    ),
    (&["persembe", "thursday", "thursdays", "jeudi", "jeudis"], 3),
    (&["cuma", "friday", "fridays", "vendredi", "vendredis"], 4),
    (
        &["cumartesi", "saturday", "saturdays", "samedi", "samedis"],
        5,
    ),
    (&["pazar", "sunday", "sundays", "dimanche", "dimanches"], 6),
];

/// A leading or trailing time expression (`her pazartesi`, `every day at 9`, `toutes les 2
/// heures`) and the text without it.
fn strip_schedule(text: &str) -> Option<ScheduleExpr> {
    strip_schedule_at(text, Local::now())
}

#[allow(clippy::too_many_lines)] // one tokenizer for every tr/en/fr time phrase
fn strip_schedule_at(text: &str, now: chrono::DateTime<Local>) -> Option<ScheduleExpr> {
    let raw: Vec<&str> = text.split_whitespace().collect();
    let folded: Vec<String> = raw
        .iter()
        .map(|w| {
            matcher::fold(w)
                .trim_matches(|c: char| !c.is_alphanumeric() && c != ':')
                .to_owned()
        })
        .collect();
    let every = ["her", "every", "each", "chaque", "tous", "toutes"];
    let start = folded.iter().position(|w| every.contains(&w.as_str()))?;
    let mut pos = start + 1;
    // `tous les`, `toutes les`
    if folded.get(pos).is_some_and(|w| w == "les") {
        pos += 1;
    }
    let count: u64 = match folded.get(pos).and_then(|w| w.parse().ok()) {
        Some(c) if c > 0 => {
            pos += 1;
            c
        }
        _ => 1,
    };
    let unit = folded.get(pos)?.as_str();
    let mut weekday = None;
    let minutes = if unit.starts_with("dakika") || unit == "dk" || unit.starts_with("minute") {
        count
    } else if unit.starts_with("saat") || unit.starts_with("hour") || unit.starts_with("heure") {
        count * 60
    } else if ["gun", "day", "jour"].iter().any(|x| unit.starts_with(x))
        || ["sabah", "morning", "matin", "daily"].contains(&unit)
    {
        count * 1440
    } else if ["hafta", "week", "semaine"]
        .iter()
        .any(|x| unit.starts_with(x))
    {
        count * 10_080
    } else if let Some((_, day)) = WEEKDAYS.iter().find(|(names, _)| {
        names
            .iter()
            .any(|x| unit.starts_with(x) && unit.len() <= x.len() + 4)
    }) {
        weekday = Some(*day);
        10_080
    } else {
        return None;
    };
    pos += 1;
    // Optional time of day: `saat 9`, `9'da`, `at 9`, `at 09:30`, `à 9h`.
    let mut hm: Option<(u32, u32)> = None;
    let mut scan = pos;
    while scan < folded.len() && scan <= pos + 2 {
        // `9'da` → `9`, `9h` → `9`, `09:30` stays.
        let word = folded[scan]
            .split('\'')
            .next()
            .unwrap_or_default()
            .trim_end_matches(char::is_alphabetic);
        let parsed = match word.split_once(':') {
            Some((hh, mm)) => hh.parse().ok().zip(mm.parse().ok()),
            None => word.parse().ok().map(|hh| (hh, 0)),
        };
        if let Some(time) = parsed.filter(|(hh, mm): &(u32, u32)| *hh < 24 && *mm < 60) {
            hm = Some(time);
            scan += 1;
            pos = scan;
            break;
        }
        if !["saat", "at", "a", "de", "around"].contains(&folded[scan].as_str()) {
            break;
        }
        scan += 1;
    }
    let end = pos;
    // Only at the start or the end of the text: "every day I see this error" is not a schedule.
    let rest = if start == 0 {
        raw[end..].join(" ")
    } else if end >= raw.len() {
        raw[..start].join(" ")
    } else {
        return None;
    };
    let rest = rest
        .trim_matches(|c: char| c == ',' || c.is_whitespace())
        .to_owned();
    if rest.is_empty() {
        return None;
    }
    // "every day I see this error…" describes, it does not schedule.
    let subject = rest
        .split_whitespace()
        .next()
        .map(matcher::fold)
        .unwrap_or_default();
    if start == 0
        && [
            "i", "we", "it", "this", "that", "there", "he", "she", "they", "je", "on", "il",
            "elle", "nous", "ca", "cela",
        ]
        .contains(&subject.trim_matches(|c: char| !c.is_alphanumeric()))
    {
        return None;
    }
    let (hour, minute) = hm.unwrap_or(if weekday.is_some() || minutes >= 1440 {
        (9, 0)
    } else {
        (now.hour(), now.minute())
    });
    let mut first = now
        .date_naive()
        .and_hms_opt(hour, minute, 0)
        .and_then(|date| Local.from_local_datetime(&date).single())
        .unwrap_or(now);
    if let Some(day) = weekday {
        let today = now.weekday().num_days_from_monday();
        let ahead = (day + 7 - today) % 7;
        first += chrono::Duration::days(i64::from(ahead));
    }
    if minutes < 1440 && hm.is_none() {
        first = now + chrono::Duration::minutes(i64::try_from(minutes).unwrap_or(60));
    }
    while first <= now {
        first += chrono::Duration::minutes(i64::try_from(minutes.min(10_080)).unwrap_or(1440));
    }
    Some(ScheduleExpr {
        rest,
        minutes,
        first_run: u64::try_from(first.timestamp_millis()).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests;
