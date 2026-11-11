//! `kor intents list|test|add|forget` and the intent hook of the REPL / TUI / `kor "<text>"`
//! (§30, ADR 0056). Routing lives in `kor_runtime::intent`; this file shows decisions and asks.

use std::io::Write as _;
use std::sync::Arc;

use kor_i18n::{Localizer, args};
use kor_providers::Provider;
use kor_runtime::intent::{Context, Decision, Fallbacks, Plan, Router, Run, Step, Via};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum IntentsCmd {
    /// Every command free text can reach, with its phrases and your own patterns.
    List {
        /// Only this command (`kor cost`, `cost`).
        command: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show where a sentence goes, without running anything.
    Test {
        #[arg(required = true)]
        text: Vec<String>,
        #[arg(long)]
        json: bool,
        /// Stage 1 only: no local embeddings, no model.
        #[arg(long)]
        offline: bool,
    },
    /// Teach a sentence: `kor intents add "siteye bak" "kor doctor"`.
    Add { text: String, command: String },
    /// Forget a sentence you taught.
    Forget { text: String },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-intents"))
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-intents-list"))
                .mut_arg("command", |a| a.help(l.t("arg-intents-command")))
                .mut_arg("json", |a| a.help(l.t("arg-tool-json")))
        })
        .mut_subcommand("test", |s| {
            s.about(l.t("cmd-intents-test"))
                .mut_arg("text", |a| a.help(l.t("arg-intents-text")))
                .mut_arg("json", |a| a.help(l.t("arg-tool-json")))
                .mut_arg("offline", |a| a.help(l.t("arg-intents-offline")))
        })
        .mut_subcommand("add", |s| {
            s.about(l.t("cmd-intents-add"))
                .mut_arg("text", |a| a.help(l.t("arg-intents-text")))
                .mut_arg("command", |a| a.help(l.t("arg-intents-command")))
        })
        .mut_subcommand("forget", |s| {
            s.about(l.t("cmd-intents-forget"))
                .mut_arg("text", |a| a.help(l.t("arg-intents-text")))
        })
}

/// `cost` → `kor cost`; `/cost` stays a plugin command name only when the catalog has it.
fn full_command(router: &Router, c: &str) -> String {
    let c = c.trim();
    if router.catalog.get(c).is_some() {
        return c.to_owned();
    }
    let with = format!(
        "kor {}",
        c.trim_start_matches('/').trim_start_matches("kor ").trim()
    );
    if router.catalog.get(&with).is_some() {
        with
    } else {
        c.to_owned()
    }
}

fn root() -> Option<std::path::PathBuf> {
    crate::graph_cmd::root().ok()
}

pub fn run(ctx: &Ctx, cmd: &IntentsCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let mut router = Router::load(root().as_deref());
    match cmd {
        IntentsCmd::List { command, json } => {
            let pick = command.as_deref().map(|c| full_command(&router, c));
            let list: Vec<_> = router
                .catalog
                .intents
                .iter()
                .filter(|i| i.run != Run::Never)
                .filter(|i| pick.as_ref().is_none_or(|p| &i.command == p))
                .collect();
            if *json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(true);
            }
            if list.is_empty() {
                eprintln!(
                    "{}",
                    l.t_args(
                        "intents-unknown",
                        &args!("command" => pick.unwrap_or_default())
                    )
                );
                return Ok(false);
            }
            let lang = l.lang().code();
            for i in &list {
                let class = l.t(if i.run == Run::Auto {
                    "intents-class-auto"
                } else {
                    "intents-class-confirm"
                });
                println!(
                    "{}  [{class}]  {}",
                    i.command,
                    i.tui.as_deref().unwrap_or("")
                );
                let mine: Vec<&str> = i
                    .phrases
                    .iter()
                    .filter(|p| p.lang == lang || p.lang == "user")
                    .map(|p| p.text.as_str())
                    .collect();
                let shown: Vec<&str> = if mine.is_empty() || pick.is_some() {
                    i.phrases.iter().map(|p| p.text.as_str()).collect()
                } else {
                    mine
                };
                let limit = if pick.is_some() { usize::MAX } else { 4 };
                println!(
                    "    {}",
                    shown
                        .iter()
                        .take(limit)
                        .map(|t| format!("\"{t}\""))
                        .collect::<Vec<_>>()
                        .join(" · ")
                );
            }
            if !router.learned.patterns.is_empty() && pick.is_none() {
                println!("\n{}", l.t("intents-learned"));
                for p in &router.learned.patterns {
                    println!("  \"{}\" → {}", p.text, p.command);
                }
            }
            for e in &router.catalog.errors {
                eprintln!(
                    "{}",
                    l.t_args("intents-plugin-error", &args!("error" => e.clone()))
                );
            }
            Ok(true)
        }
        IntentsCmd::Test {
            text,
            json,
            offline,
        } => {
            let text = text.join(" ");
            let context = Context {
                model: model_ref(ctx),
                ..Context::default()
            };
            let embedder = (!offline)
                .then(kor_agent::semantic::local_embedder)
                .flatten();
            let model = if *offline {
                None
            } else {
                ctx.config
                    .as_ref()
                    .ok()
                    .and_then(|c| kor_runtime::intent::fallback_model(l, c))
            };
            let fb = Fallbacks {
                embedder: embedder
                    .as_ref()
                    .map(|e| e as &dyn kor_agent::semantic::Embedder),
                model,
            };
            let d = crate::providers::runtime()?.block_on(router.route_full(&text, &context, &fb));
            if *json {
                println!(
                    "{}",
                    serde_json::json!({"decision": d, "ranked": router.rank(&text).into_iter().take(5).collect::<Vec<_>>()})
                );
                return Ok(true);
            }
            for line in describe(l, &d) {
                println!("{line}");
            }
            println!("\n{}", l.t("intents-ranked"));
            for c in router.rank(&text).into_iter().take(5) {
                println!("  {:.2}  {}  \"{}\"", c.score, c.command, c.phrase);
            }
            Ok(true)
        }
        IntentsCmd::Add { text, command } => {
            let c = full_command(&router, command);
            match router.learn(text, &c) {
                Ok(()) => {
                    println!(
                        "{}",
                        l.t_args(
                            "intents-added",
                            &args!("text" => text.clone(), "command" => c)
                        )
                    );
                    Ok(true)
                }
                Err(_) => {
                    eprintln!("{}", l.t_args("intents-unknown", &args!("command" => c)));
                    Ok(false)
                }
            }
        }
        IntentsCmd::Forget { text } => {
            let removed = router.learned.forget(text);
            if removed && let Some(f) = kor_runtime::intent::learned_file() {
                router.learned.save(&f).map_err(anyhow::Error::msg)?;
            }
            println!(
                "{}",
                l.t(if removed {
                    "intents-forgotten"
                } else {
                    "intents-not-learned"
                })
            );
            Ok(removed)
        }
    }
}

fn model_ref(ctx: &Ctx) -> Option<String> {
    ctx.config
        .as_ref()
        .ok()
        .map(|c| c.config.agent.model.trim().to_owned())
        .filter(|m| !m.is_empty())
}

fn suffix(l: &Localizer, p: &Plan) -> String {
    let mut s = String::new();
    if let Some(e) = p.eta_s {
        s.push_str(&l.t_args("intent-eta", &args!("seconds" => e)));
    }
    if let Some(u) = p.usd {
        s.push_str(&format!(", {}", kor_runtime::cost::fmt_usd(u, true)));
    }
    if s.is_empty() { s } else { format!(" ({s})") }
}

fn via(l: &Localizer, p: &Plan) -> String {
    let key = match p.via {
        Via::Keyword => "intent-via-keyword",
        Via::Learned => "intent-via-learned",
        Via::Embedding => "intent-via-embedding",
        Via::Model => "intent-via-model",
    };
    l.t(key)
}

fn fmt_ms(ms: u64) -> String {
    kor_runtime::cost::fmt_time(ms)
}

/// What the decision means, one line per item, in the user's language.
pub fn describe(l: &Localizer, d: &Decision) -> Vec<String> {
    match d {
        Decision::Agent => vec![l.t("intent-agent")],
        Decision::Run { plan } => vec![l.t_args("intent-run", &args!("line" => plan.line.clone(), "via" => via(l, plan)))],
        Decision::Confirm { plan } => vec![
            l.t_args("intent-confirm", &args!("line" => plan.line.clone(), "extra" => suffix(l, plan), "name" => plan.name.clone())),
            l.t("intent-confirm-hint"),
        ],
        Decision::Missing { plan } => vec![l.t_args(
            "intent-missing",
            &args!("line" => plan.line.clone(), "names" => plan.missing.join(", ")),
        )],
        Decision::Choose { options } => {
            let mut v = vec![l.t("intent-choose")];
            for (i, p) in options.iter().enumerate() {
                v.push(format!("  {}) {} — {}", i + 1, p.line, p.name));
            }
            v
        }
        Decision::Pipeline { steps } => {
            let mut v = vec![l.t_args("intent-pipeline", &args!("count" => steps.len()))];
            for (i, s) in steps.iter().enumerate() {
                v.push(match s {
                    Step::Command(p) => format!("  {}) {}{}", i + 1, p.line, suffix(l, p)),
                    Step::Agent(t) => format!("  {}) {}", i + 1, l.t_args("intent-step-agent", &args!("text" => t.clone()))),
                });
            }
            v.push(l.t("intent-confirm-hint"));
            v
        }
        Decision::Schedule { prompt, interval_minutes, first_run, .. } => vec![
            l.t_args(
                "intent-schedule",
                &args!("prompt" => prompt.clone(), "minutes" => *interval_minutes, "first" => fmt_ms(*first_run)),
            ),
            l.t("intent-schedule-hint"),
        ],
    }
}

/// What the REPL does after routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run `/command args` through the REPL's slash path (same code as `kor <command>`).
    Slash(String),
    /// A normal agent turn.
    Agent(String),
}

async fn ask_line(prompt: &str) -> Option<String> {
    print!("{prompt} ");
    let _ = std::io::stdout().flush();
    tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        std::io::stdin()
            .read_line(&mut s)
            .ok()
            .filter(|n| *n > 0)
            .map(|_| s.trim().to_owned())
    })
    .await
    .ok()
    .flatten()
}

fn yes(answer: &str) -> bool {
    matches!(
        answer.to_lowercase().as_str(),
        "" | "y" | "yes" | "e" | "evet" | "o" | "oui"
    )
}

/// Whether routing is on (`intent.enabled`, default on).
pub fn enabled(ctx: &Ctx) -> bool {
    ctx.config.as_ref().is_ok_and(|c| c.config.intent.enabled)
}

/// Route one line of free text typed in the line REPL (or `kor "<text>"`): print the decision,
/// ask where needed, learn corrections, return what to do. `interactive = false` (no terminal)
/// never runs anything that needs a confirmation.
pub async fn route_line(
    ctx: &Ctx,
    text: &str,
    provider: Option<(Arc<dyn Provider>, String)>,
    interactive: bool,
) -> Vec<Action> {
    let l = &ctx.l;
    let mut router = Router::load(root().as_deref());
    let cfg = ctx.config.as_ref().ok();
    let context = Context {
        model: model_ref(ctx),
        confirm_all: cfg.is_some_and(|c| !c.config.intent.auto_run),
        read_clipboard: interactive,
        ..Context::default()
    };
    let embedder = kor_agent::semantic::local_embedder();
    let model = cfg
        .and_then(|c| kor_runtime::intent::fallback_model(l, c))
        .or_else(|| {
            provider.filter(|(_, m)| {
                cfg.is_some_and(|c| {
                    c.config.intent.model_fallback
                        && kor_runtime::intent::model_allowed(&c.config, m)
                })
            })
        });
    let fb = Fallbacks {
        embedder: embedder
            .as_ref()
            .map(|e| e as &dyn kor_agent::semantic::Embedder),
        model,
    };
    let d = router.route_full(text, &context, &fb).await;
    let agent = || vec![Action::Agent(text.to_owned())];
    if d == Decision::Agent {
        return agent();
    }
    for line in describe(l, &d) {
        eprintln!("{line}");
    }
    let run = |router: &mut Router, p: &Plan| {
        router.remember(p);
        Action::Slash(p.slash.clone())
    };
    match d {
        Decision::Agent => agent(),
        Decision::Run { plan } => vec![run(&mut router, &plan)],
        _ if !interactive => {
            eprintln!("{}", l.t("intent-needs-terminal"));
            agent()
        }
        Decision::Confirm { plan } => {
            let Some(a) = ask_line("›").await else {
                return Vec::new();
            };
            if yes(&a) {
                return vec![run(&mut router, &plan)];
            }
            correction(l, &mut router, text, &a).unwrap_or_else(agent)
        }
        Decision::Missing { mut plan } => {
            for name in plan.missing.clone() {
                let Some(v) = ask_line(&format!("{name}:"))
                    .await
                    .filter(|v| !v.is_empty())
                else {
                    return agent();
                };
                plan.params.insert(name, v);
            }
            // Rebuild the exact line with the values the user typed.
            match router.plan(
                &plan.command,
                &format!(
                    "{text} {}",
                    plan.params.values().cloned().collect::<Vec<_>>().join(" ")
                ),
                &context,
                plan.via,
                plan.score,
            ) {
                Some(p) if p.missing.is_empty() => {
                    eprintln!("{}", l.t_args("intent-confirm", &args!("line" => p.line.clone(), "extra" => suffix(l, &p), "name" => p.name.clone())));
                    let Some(a) = ask_line("›").await else {
                        return Vec::new();
                    };
                    if yes(&a) {
                        vec![run(&mut router, &p)]
                    } else {
                        agent()
                    }
                }
                _ => agent(),
            }
        }
        Decision::Choose { options } => {
            let Some(a) = ask_line("#").await else {
                return Vec::new();
            };
            match a
                .parse::<usize>()
                .ok()
                .and_then(|n| options.get(n.wrapping_sub(1)))
            {
                Some(p) => {
                    // §30.2: the choice is learned locally.
                    let _ = router.learn(text, &p.command);
                    if p.run == Run::Auto
                        || yes(&ask_line(&format!("{} ›", p.line)).await.unwrap_or_default())
                    {
                        vec![run(&mut router, p)]
                    } else {
                        agent()
                    }
                }
                None if a.is_empty() => agent(),
                None => correction(l, &mut router, text, &a).unwrap_or_else(agent),
            }
        }
        Decision::Pipeline { steps } => {
            let Some(a) = ask_line("›").await else {
                return Vec::new();
            };
            if !yes(&a) {
                return agent();
            }
            steps
                .iter()
                .map(|s| match s {
                    Step::Command(p) => run(&mut router, p),
                    Step::Agent(t) => Action::Agent(t.clone()),
                })
                .collect()
        }
        Decision::Schedule {
            prompt,
            interval_minutes,
            first_run,
            ..
        } => {
            let Some(a) = ask_line("›").await else {
                return Vec::new();
            };
            if !yes(&a) {
                return agent();
            }
            match schedule(&prompt, interval_minutes, first_run) {
                Ok(id) => eprintln!(
                    "{}",
                    l.t_args(
                        "intent-scheduled",
                        &args!("id" => id, "first" => fmt_ms(first_run))
                    )
                ),
                Err(e) => eprintln!("{}", l.t_args("error-generic", &args!("error" => e))),
            }
            Vec::new()
        }
    }
}

/// "no, I meant `/x`" (or `kor x`): learn it and run that instead.
fn correction(l: &Localizer, router: &mut Router, text: &str, answer: &str) -> Option<Vec<Action>> {
    let a = answer.trim();
    if !(a.starts_with('/') || a.starts_with("kor ")) {
        return None;
    }
    let command = full_command(
        router,
        a.split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
            .as_str(),
    );
    // The longest catalog command that prefixes the answer (`/graph status x` → `kor graph status`).
    let words: Vec<&str> = a
        .trim_start_matches('/')
        .trim_start_matches("kor ")
        .split_whitespace()
        .collect();
    let learned = (1..=words.len().min(3))
        .rev()
        .map(|n| format!("kor {}", words[..n].join(" ")))
        .find(|c| router.catalog.get(c).is_some())
        .unwrap_or(command);
    if router.learn(text, &learned).is_ok() {
        eprintln!(
            "{}",
            l.t_args(
                "intents-added",
                &args!("text" => text.to_owned(), "command" => learned)
            )
        );
    }
    Some(vec![Action::Slash(format!("/{}", words.join(" ")))])
}

/// Create the routine (`kor routines`) for a scheduled request.
fn schedule(prompt: &str, minutes: u64, first_run: u64) -> Result<String, String> {
    let file = kor_runtime::routines::file().ok_or("no Kor home")?;
    let root = root().map(|r| r.display().to_string()).unwrap_or_default();
    let mut m = serde_json::Map::new();
    m.insert("prompt".into(), prompt.into());
    m.insert("intervalMinutes".into(), minutes.into());
    m.insert("root".into(), root.into());
    m.insert("firstRun".into(), first_run.into());
    let t = kor_runtime::routines::add(&file, &m, kor_runtime::cost::now_ms())?;
    Ok(t.get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

/// TUI: what to show and what to do without leaving the full screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiRoute {
    /// Lines for the stream.
    pub lines: Vec<String>,
    /// Run now (read-only command).
    pub run: Option<String>,
    /// Put into the input line: Enter runs it, editing it changes it (the confirmation).
    pub prefill: Option<String>,
    /// Send to the agent unchanged.
    pub agent: bool,
    /// Commands offered for this text; a later `/command` among them is learned.
    pub offered: Vec<String>,
    /// Pipeline steps after the prefilled one (slash commands or agent text), queued on accept.
    pub then: Vec<String>,
}

/// Route text typed in the TUI. Confirmation is the prefilled input line: the exact command,
/// Enter to run, Esc/edit to change.
pub async fn route_tui(
    ctx: &Ctx,
    text: &str,
    provider: Option<(Arc<dyn Provider>, String)>,
) -> TuiRoute {
    let l = &ctx.l;
    let mut router = Router::load(root().as_deref());
    let cfg = ctx.config.as_ref().ok();
    let context = Context {
        model: model_ref(ctx),
        confirm_all: cfg.is_some_and(|c| !c.config.intent.auto_run),
        ..Context::default()
    };
    let embedder = kor_agent::semantic::local_embedder();
    let model = cfg
        .and_then(|c| kor_runtime::intent::fallback_model(l, c))
        .or_else(|| {
            provider.filter(|(_, m)| {
                cfg.is_some_and(|c| {
                    c.config.intent.model_fallback
                        && kor_runtime::intent::model_allowed(&c.config, m)
                })
            })
        });
    let fb = Fallbacks {
        embedder: embedder
            .as_ref()
            .map(|e| e as &dyn kor_agent::semantic::Embedder),
        model,
    };
    let d = router.route_full(text, &context, &fb).await;
    let mut out = TuiRoute {
        lines: Vec::new(),
        run: None,
        prefill: None,
        agent: false,
        offered: Vec::new(),
        then: Vec::new(),
    };
    match &d {
        Decision::Agent => {
            out.agent = true;
            return out;
        }
        Decision::Run { plan } => {
            router.remember(plan);
            out.run = Some(plan.slash.clone());
        }
        Decision::Confirm { plan } | Decision::Missing { plan } => {
            out.prefill = Some(plan.slash.clone());
            out.offered.push(plan.command.clone());
        }
        Decision::Choose { options } => {
            out.prefill = options.first().map(|p| p.slash.clone());
            out.offered = options.iter().map(|p| p.command.clone()).collect();
        }
        Decision::Pipeline { steps } => {
            // The first step is prefilled; accepting it queues the others in order.
            let mut all = steps.iter().map(|s| match s {
                Step::Command(p) => p.slash.clone(),
                Step::Agent(t) => t.clone(),
            });
            out.prefill = all.next();
            out.then = all.collect();
        }
        Decision::Schedule { .. } => {
            out.prefill = Some("/routines list".into());
        }
    }
    out.lines = describe(l, &d)
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    if out.prefill.is_some() {
        out.lines.push(l.t("intent-tui-hint"));
    }
    out
}

/// TUI: the user ran `/command` after a proposal for `text`; learn it when it was not the first offer.
pub fn learn_tui_choice(text: &str, offered: &[String], slash: &str) {
    let mut router = Router::load(root().as_deref());
    let words: Vec<&str> = slash.trim_start_matches('/').split_whitespace().collect();
    let Some(cmd) = (1..=words.len().min(3))
        .rev()
        .map(|n| format!("kor {}", words[..n].join(" ")))
        .find(|c| router.catalog.get(c).is_some())
    else {
        return;
    };
    if offered.first() != Some(&cmd) {
        let _ = router.learn(text, &cmd);
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory as _, Parser};

    use super::*;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: IntentsCmd,
    }

    #[test]
    fn parses_and_localizes() {
        let c = Cli::parse_from(["x", "test", "maliyetimi", "göster", "--offline"]);
        assert!(
            matches!(c.cmd, IntentsCmd::Test { ref text, offline: true, .. } if text.join(" ") == "maliyetimi göster")
        );
        let c = Cli::parse_from(["x", "add", "siteye bak", "kor doctor"]);
        assert!(matches!(c.cmd, IntentsCmd::Add { .. }));
        assert!(Cli::try_parse_from(["x", "test"]).is_err());
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let cmd = localize(
            clap::Command::new("intents").subcommands(Cli::command().get_subcommands().cloned()),
            &l,
        );
        assert!(cmd.get_subcommands().any(|s| s.get_name() == "forget"));
        for key in [
            "intent-agent",
            "intent-choose",
            "intent-confirm-hint",
            "intent-tui-hint",
            "cmd-intents-test",
            "intent-needs-terminal",
        ] {
            assert_ne!(l.t(key), key, "missing tr string {key}");
        }
    }

    #[test]
    fn corrections_and_answers() {
        assert!(yes("") && yes("E") && yes("oui") && !yes("n") && !yes("hayır"));
        let r = Router::new(
            kor_runtime::intent::Catalog::builtin(),
            kor_runtime::intent::Learned::default(),
        );
        assert_eq!(full_command(&r, "cost"), "kor cost");
        assert_eq!(full_command(&r, "/graph status"), "kor graph status");
        let l = Localizer::new(kor_i18n::Lang::En);
        let d = Decision::Pipeline {
            steps: vec![Step::Agent("x".into())],
        };
        assert!(describe(&l, &d).len() >= 2);
    }
}
