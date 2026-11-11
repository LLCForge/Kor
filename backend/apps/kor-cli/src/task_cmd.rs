//! `kor task "<goal>" --agents N` and `kor task list|status|resume|fork|cancel|resolve`
//! (spec §8.2, §6.2, ADR 0037). The orchestration lives in `kor_runtime::task`; this file
//! parses, prints progress and asks what to do with a conflict.

use std::collections::BTreeMap;
use std::io::{IsTerminal as _, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex};

use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use kor_runtime::cost::{fmt_time, fmt_usd};
use kor_runtime::task::{
    self, Engine, Resolution, RuntimeSpawner, Settings, SubStatus, TaskState, TaskStatus,
};

use crate::Ctx;

#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct TaskArgs {
    #[command(subcommand)]
    command: Option<TaskCmd>,
    /// What the team of agents should do.
    goal: Vec<String>,
    /// Number of sub-agents (1–16; default `[orchestrator] agents`).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=16))]
    agents: Option<u8>,
    /// Pinned models: `backend=openai/gpt-4.1,docs=local/qwen3`.
    #[arg(long = "model-per-role", value_delimiter = ',')]
    model_per_role: Vec<String>,
    /// Whole-task budget in USD.
    #[arg(long)]
    budget_usd: Option<f64>,
    /// Model that plans and re-plans.
    #[arg(long)]
    orchestrator_model: Option<String>,
    /// Test command run before review and on the merged branch.
    #[arg(long)]
    test: Option<String>,
    /// Sub-agents running at the same time.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=16))]
    parallel: Option<u8>,
    /// Run on a paired machine (auto = least busy).
    #[arg(long)]
    on: Option<String>,
}

#[derive(Debug, clap::Subcommand)]
pub enum TaskCmd {
    /// Tasks of this repository.
    List,
    /// Sub-agents with role, model, status, cost and branch.
    Status { id: String },
    /// Continue a stopped or crashed task from its checkpoint.
    Resume { id: String },
    /// A copy of a task from the same point, on new branches.
    Fork { id: String },
    /// Stop a task and remove its worktrees.
    Cancel { id: String },
    /// Settle a merge conflict: re-plan that part or drop it.
    Resolve {
        id: String,
        /// Sub-task number as `kor task status` shows it.
        part: usize,
        #[arg(value_enum)]
        action: ResolveAction,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ResolveAction {
    Replan,
    Drop,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-task"))
        .mut_arg("goal", |a| a.help(l.t("arg-task-goal")))
        .mut_arg("agents", |a| a.help(l.t("arg-task-agents")))
        .mut_arg("model_per_role", |a| a.help(l.t("arg-task-model-per-role")))
        .mut_arg("budget_usd", |a| a.help(l.t("arg-task-budget")))
        .mut_arg("orchestrator_model", |a| a.help(l.t("arg-task-orchestrator")))
        .mut_arg("test", |a| a.help(l.t("arg-task-test")))
        .mut_arg("parallel", |a| a.help(l.t("arg-task-parallel")))
        .mut_arg("on", |a| a.help(l.t("arg-task-on")))
        .mut_subcommand("list", |s| s.about(l.t("cmd-task-list")))
        .mut_subcommand("status", |s| s.about(l.t("cmd-task-status")))
        .mut_subcommand("resume", |s| s.about(l.t("cmd-task-resume")))
        .mut_subcommand("fork", |s| s.about(l.t("cmd-task-fork")))
        .mut_subcommand("cancel", |s| s.about(l.t("cmd-task-cancel")))
        .mut_subcommand("resolve", |s| {
            s.about(l.t("cmd-task-resolve"))
                .mut_arg("part", |a| a.help(l.t("arg-task-part")))
        })
}

pub fn run(ctx: &Ctx, a: TaskArgs) -> anyhow::Result<bool> {
    if let Some(peer) = &a.on {
        return crate::peer_cmd::run_on(ctx, peer, &a.goal.join(" "));
    }
    let l = &ctx.l;
    let cwd = std::env::current_dir()?;
    let root = task::git::repo_root(&cwd).map_err(anyhow::Error::msg)?;
    match a.command {
        Some(TaskCmd::List) => Ok(list(l, &root)),
        Some(TaskCmd::Status { id }) => {
            let st = task::load(&root, &id).map_err(anyhow::Error::msg)?;
            print_status(l, &st);
            Ok(true)
        }
        Some(TaskCmd::Fork { id }) => {
            let st = task::fork(&root, &id).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args("task-forked", &args!("from" => id, "id" => st.id))
            );
            Ok(true)
        }
        Some(TaskCmd::Cancel { id }) => {
            let st = task::cancel(&root, &id).map_err(anyhow::Error::msg)?;
            println!("{}", l.t_args("task-cancelled", &args!("id" => st.id)));
            Ok(true)
        }
        Some(TaskCmd::Resume { id }) => {
            let mut st = task::load(&root, &id).map_err(anyhow::Error::msg)?;
            drive(ctx, &root, &mut st, None, Step::Run)
        }
        Some(TaskCmd::Resolve { id, part, action }) => {
            let mut st = task::load(&root, &id).map_err(anyhow::Error::msg)?;
            let how = match action {
                ResolveAction::Replan => Resolution::Replan,
                ResolveAction::Drop => Resolution::Drop,
            };
            drive(ctx, &root, &mut st, None, Step::Resolve(part.saturating_sub(1), how))
        }
        None => {
            let goal = a.goal.join(" ");
            if goal.trim().is_empty() {
                anyhow::bail!(l.t("task-no-goal"));
            }
            let loaded = loaded(ctx)?;
            let mut s = Settings::from_config(&loaded.config.orchestrator);
            if let Some(n) = a.agents {
                s.agents = usize::from(n);
            }
            if let Some(n) = a.parallel {
                s.max_parallel = usize::from(n);
            }
            if let Some(b) = a.budget_usd {
                s.budget_usd = b.max(0.0);
            }
            if let Some(t) = a.test {
                t.trim().clone_into(&mut s.test_command);
            }
            s.models.extend(role_models(l, &a.model_per_role)?);
            let (root, mut st) = task::create(&cwd, &goal, s).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args(
                    "task-created",
                    &args!("id" => st.id.clone(), "branch" => st.branch.clone())
                )
            );
            drive(
                ctx,
                &root,
                &mut st,
                a.orchestrator_model.as_deref(),
                Step::Run,
            )
        }
    }
}

fn loaded(ctx: &Ctx) -> anyhow::Result<&kor_config::Loaded> {
    ctx.config
        .as_ref()
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

/// `backend=openai/gpt-4.1` → (`backend`, `openai/gpt-4.1`).
fn role_models(l: &Localizer, items: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for it in items.iter().filter(|s| !s.trim().is_empty()) {
        let Some((role, model)) = it.split_once('=') else {
            anyhow::bail!(l.t_args("task-bad-role-model", &args!("value" => it.clone())));
        };
        let (role, model) = (role.trim(), model.trim());
        if role.is_empty() || model.parse::<kor_core::ModelRef>().is_err() {
            anyhow::bail!(l.t_args("task-bad-role-model", &args!("value" => it.clone())));
        }
        out.insert(task::slug(role), model.to_owned());
    }
    Ok(out)
}

enum Step {
    Run,
    Resolve(usize, Resolution),
}

/// Run (or resolve) with progress lines; at a conflict, show it and ask what to do.
fn drive(
    ctx: &Ctx,
    root: &Path,
    st: &mut TaskState,
    orchestrator_model: Option<&str>,
    mut step: Step,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let loaded = loaded(ctx)?;
    let (orchestrator, reviewer) =
        task::models(l, loaded, orchestrator_model).map_err(anyhow::Error::msg)?;
    let engine = Engine {
        root: root.to_path_buf(),
        lang: l.lang(),
        orchestrator,
        reviewer,
        spawner: Arc::new(RuntimeSpawner::new(l.lang(), loaded)),
        progress: printer(l.lang()),
    };
    let rt = crate::providers::runtime()?;
    let cancel = CancellationToken::new();
    loop {
        let r = rt.block_on(async {
            let on_signal = cancel.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    on_signal.cancel();
                }
            });
            match step {
                Step::Run => engine.run(st, &cancel).await,
                Step::Resolve(i, how) => engine.resolve(st, i, how, &cancel).await,
            }
        });
        if let Err(e) = r {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            return Ok(false);
        }
        print_status(l, st);
        if st.status != TaskStatus::Conflict {
            return Ok(st.status == TaskStatus::Done);
        }
        let Some(c) = st.conflicts.first().cloned() else {
            return Ok(false);
        };
        print_conflict(l, &c);
        match ask_resolution(l, &st.id, c.index) {
            Some(how) => step = Step::Resolve(c.index, how),
            None => return Ok(false),
        }
    }
}

/// Terminal: one braille spinner row per sub-agent, redrawn in place (§16.1, §16.2 rule 5).
/// Otherwise one line per sub-task status change.
fn printer(lang: kor_i18n::Lang) -> task::Progress {
    let seen: Mutex<BTreeMap<usize, SubStatus>> = Mutex::new(BTreeMap::new());
    let live = crate::brand::LiveMulti::start();
    Arc::new(move |st: &TaskState| {
        let l = Localizer::new(lang);
        if let Some(live) = &live {
            use crate::brand::RowState;
            for s in &st.subtasks {
                let state = match s.status {
                    SubStatus::Pending => RowState::Waiting,
                    SubStatus::Running => RowState::Running,
                    SubStatus::Approved | SubStatus::Merged => RowState::Done,
                    SubStatus::Rejected | SubStatus::Failed | SubStatus::Conflict | SubStatus::Dropped | SubStatus::Cancelled => RowState::Failed,
                };
                live.set(s.index, &s.role, &l.t(&format!("task-sub-{}", sub_key(s.status))), state);
            }
            return;
        }
        let mut seen = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for s in &st.subtasks {
            if seen.insert(s.index, s.status) != Some(s.status) {
                eprintln!(
                    "{}",
                    l.t_args(
                        "task-sub-line",
                        &args!(
                            "n" => s.index + 1,
                            "total" => st.subtasks.len(),
                            "role" => s.role.clone(),
                            "status" => l.t(&format!("task-sub-{}", sub_key(s.status)))
                        )
                    )
                );
            }
        }
    })
}

fn sub_key(s: SubStatus) -> &'static str {
    match s {
        SubStatus::Pending => "pending",
        SubStatus::Running => "running",
        SubStatus::Approved => "approved",
        SubStatus::Rejected => "rejected",
        SubStatus::Failed => "failed",
        SubStatus::Merged => "merged",
        SubStatus::Conflict => "conflict",
        SubStatus::Dropped => "dropped",
        SubStatus::Cancelled => "cancelled",
    }
}

fn task_key(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Planning => "planning",
        TaskStatus::Running => "running",
        TaskStatus::Merging => "merging",
        TaskStatus::Conflict => "conflict",
        TaskStatus::Done => "done",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn list(l: &Localizer, root: &Path) -> bool {
    let all = task::list(root);
    if all.is_empty() {
        println!("{}", l.t("task-empty"));
        return true;
    }
    let mut rows = vec![vec![
        "#".to_owned(),
        l.t("task-col-goal"),
        "%".to_owned(),
        l.t("task-col-status"),
        l.t("task-col-cost"),
        l.t("task-col-updated"),
    ]];
    for t in &all {
        rows.push(vec![
            format!("#{}", t.id),
            t.goal.chars().take(48).collect(),
            format!("{}%", t.percent()),
            l.t(&format!("task-status-{}", task_key(t.status))),
            fmt_usd(t.total_usd(), t.estimated()),
            fmt_time(t.updated_ms),
        ]);
    }
    crate::providers::table(&rows);
    true
}

fn print_status(l: &Localizer, st: &TaskState) {
    println!(
        "{}",
        l.t_args(
            "task-head",
            &args!(
                "id" => st.id.clone(),
                "goal" => st.goal.clone(),
                "status" => l.t(&format!("task-status-{}", task_key(st.status))),
                "percent" => st.percent(),
                "branch" => st.branch.clone()
            )
        )
    );
    let mut rows = vec![vec![
        "#".to_owned(),
        l.t("task-col-role"),
        l.t("task-col-model"),
        l.t("task-col-status"),
        l.t("task-col-cost"),
        l.t("task-col-branch"),
    ]];
    for s in &st.subtasks {
        let model = s
            .model
            .clone()
            .or_else(|| (!s.models_used.is_empty()).then(|| s.models_used.join(", ")))
            .unwrap_or_else(|| l.t("task-model-router"));
        rows.push(vec![
            (s.index + 1).to_string(),
            s.role.clone(),
            model,
            l.t(&format!("task-sub-{}", sub_key(s.status))),
            fmt_usd(s.cost_usd, s.estimated),
            s.branch.clone(),
        ]);
    }
    rows.push(vec![
        String::new(),
        l.t("task-orchestrator"),
        String::new(),
        String::new(),
        fmt_usd(st.orchestrator_usd, st.estimated),
        String::new(),
    ]);
    rows.push(vec![
        String::new(),
        l.t("task-reviewer"),
        String::new(),
        String::new(),
        fmt_usd(st.reviewer_usd, st.estimated),
        String::new(),
    ]);
    rows.push(vec![
        String::new(),
        l.t("cost-total"),
        String::new(),
        String::new(),
        fmt_usd(st.total_usd(), st.estimated()),
        String::new(),
    ]);
    crate::providers::table(&rows);
    for s in &st.subtasks {
        if let Some(e) = &s.error {
            println!("  {}. {}: {e}", s.index + 1, s.role);
        } else if s.status == SubStatus::Rejected
            && let Some(r) = &s.review
        {
            println!("  {}. {}: {r}", s.index + 1, s.role);
        }
    }
    if let Some(t) = &st.test {
        let key = if t.ok { "task-test-ok" } else { "task-test-failed" };
        println!("{}", l.t(key));
        if !t.ok {
            println!("{}", t.output);
        }
    }
    if let Some(e) = &st.error {
        eprintln!("{}", l.t_args("error-generic", &args!("error" => e.clone())));
    }
    for c in &st.conflicts {
        println!(
            "{}",
            l.t_args(
                "task-conflict-head",
                &args!(
                    "n" => c.index + 1,
                    "role" => c.role.clone(),
                    "files" => c.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join(", ")
                )
            )
        );
    }
}

fn print_conflict(l: &Localizer, c: &task::Conflict) {
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    for line in c.diff.lines() {
        let paint = color
            && (line.starts_with("++<<<<<<<")
                || line.starts_with("++=======")
                || line.starts_with("++>>>>>>>")
                || line.starts_with("<<<<<<<")
                || line.starts_with(">>>>>>>"));
        if paint {
            println!("\x1b[31m{line}\x1b[0m");
        } else {
            println!("{line}");
        }
    }
    println!(
        "{}",
        l.t_args("task-conflict-safe", &args!("role" => c.role.clone()))
    );
}

/// `r` re-plan, `d` drop, anything else keeps it for later (`kor task resolve`).
fn ask_resolution(l: &Localizer, id: &str, index: usize) -> Option<Resolution> {
    let hint = l.t_args("task-conflict-hint", &args!("id" => id.to_owned(), "n" => index + 1));
    if !std::io::stdin().is_terminal() {
        println!("{hint}");
        return None;
    }
    print!("{} ", l.t("task-conflict-ask"));
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return None;
    }
    match line.trim().to_lowercase().as_str() {
        "r" | "replan" | "y" | "yeniden" => Some(Resolution::Replan),
        "d" | "drop" | "b" | "bırak" | "a" | "abandonner" => Some(Resolution::Drop),
        _ => {
            println!("{hint}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Debug, clap::Subcommand)]
    enum Top {
        Task(TaskArgs),
    }

    fn parse(args: &[&str]) -> TaskArgs {
        let Top::Task(a) = Cli::try_parse_from([&["kor", "task"], args].concat())
            .unwrap()
            .cmd;
        a
    }

    #[test]
    fn goal_with_flags_and_subcommands_parse() {
        let a = parse(&[
            "auth sistemini sıfırdan yaz",
            "--agents",
            "5",
            "--model-per-role",
            "backend=openai/gpt-4.1,docs=local/qwen3",
            "--budget-usd",
            "2.5",
        ]);
        assert!(a.command.is_none());
        assert_eq!(a.goal, ["auth sistemini sıfırdan yaz"]);
        assert_eq!(a.agents, Some(5));
        assert_eq!(a.model_per_role.len(), 2);
        assert_eq!(a.budget_usd, Some(2.5));
        assert!(matches!(parse(&["list"]).command, Some(TaskCmd::List)));
        assert!(matches!(
            parse(&["resolve", "3", "2", "replan"]).command,
            Some(TaskCmd::Resolve { part: 2, action: ResolveAction::Replan, .. })
        ));
        assert!(matches!(parse(&["resume", "7"]).command, Some(TaskCmd::Resume { .. })));
        assert!(Cli::try_parse_from(["kor", "task", "x", "--agents", "40"]).is_err());
    }

    #[test]
    fn role_models_are_checked() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let m = role_models(&l, &["Back End=openai/gpt-4.1".into()]).unwrap();
        assert_eq!(m["back-end"], "openai/gpt-4.1");
        assert!(role_models(&l, &["backend".into()]).is_err());
        assert!(role_models(&l, &["=openai/x".into()]).is_err());
    }
}
