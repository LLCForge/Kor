//! The agent front end: interactive REPL, one-shot prompt and print (`-p`) mode.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use async_trait::async_trait;
use kor_agent::{
    Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, StopReason, memory, session,
};
use kor_compat::template::Part;
use kor_core::{FinishReason, PermissionMode};
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::Ctx;
use crate::providers::provider_error;
use crate::tui::{self, Ev, Out};

#[derive(Debug, Default)]
pub struct RunArgs {
    pub prompt: Option<String>,
    pub print: bool,
    pub model: Option<String>,
    pub mode: Option<PermissionMode>,
    pub resume: Option<Resume>,
}

pub use kor_runtime::Resume;

use kor_runtime::{Built, memory_layout, new_session_path, refresh_memory};

struct Style {
    color: bool,
}

impl Style {
    fn detect() -> Self {
        Self {
            // §28.11: NO_COLOR, TERM=dumb and pipes get plain text.
            color: tui::term::color(),
        }
    }
    fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    }
    fn ember(&self, s: &str) -> String {
        let (r, g, b) = kor_core::theme::ember::MID;
        self.paint(&format!("38;2;{r};{g};{b}"), s)
    }
    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
}


/// §17.2 (ADR 0061): this surface's core event stream into the IPC trace (`KOR_IPC_TRACE`), in the
/// desktop's `kor:event` wire shape, so CLI, TUI and desktop runs can be compared item by item.
fn trace_ev(surface: kor_core::ipc_trace::Surface, ev: &AgentEvent) {
    if kor_core::ipc_trace::enabled()
        && let Some(v) = kor_agent::remote::wire::event_json(ev.clone())
    {
        trace_item(surface, v);
    }
}

fn trace_item(surface: kor_core::ipc_trace::Surface, mut v: serde_json::Value) {
    if kor_core::ipc_trace::enabled() {
        v["sessionId"] = json!(surface.as_str());
        kor_core::ipc_trace::event(surface, "kor:event", &v);
    }
}

/// What the full screen shows for a core event (`None`: nothing to draw).
pub(crate) fn tui_ev(ev: AgentEvent) -> Option<Ev> {
    Some(match ev {
        AgentEvent::Text(t) | AgentEvent::Thinking(t) => Ev::Text(t),
        AgentEvent::ToolProgress { message: Some(m), .. } => Ev::Progress(m),
        AgentEvent::ToolProgress { .. } => return None,
        AgentEvent::Usage(u) => Ev::Usage(u.input, u.output),
        AgentEvent::Notice(n) => Ev::Notice(n),
        AgentEvent::Context(b) => Ev::Context(b),
        AgentEvent::ToolStart { name, summary, .. } => Ev::Tool(format!("{name} {summary}")),
        AgentEvent::ToolEnd { name, is_error, .. } => Ev::ToolEnd { name, is_error },
        AgentEvent::Status(st) => Ev::Status(st),
        AgentEvent::Estimate(p) => Ev::Notice(format!("{} ({})", p.line, p.method)),
    })
}

/// Blocking line read off the async runtime. `None` on EOF or error.
async fn read_line() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        match std::io::stdin().lock().read_line(&mut s) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(s),
        }
    })
    .await
    .ok()
    .flatten()
}

struct TerminalApprover<'a> {
    l: &'a Localizer,
    style: &'a Style,
    /// §28.4: `BEL` + title while waiting (ADR 0051).
    ctx: &'a Ctx,
}

#[async_trait]
impl Approver for TerminalApprover<'_> {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.review(req).await.reply
    }

    /// §12.3: the diff of every file, then all at once (`y`/`a`/`n`), `e` to edit a single
    /// file, or `f` to answer file by file (`a`/`e`/`r`, `e` opens `$EDITOR`).
    async fn review(&self, req: &PermissionRequest) -> kor_agent::Review {
        use kor_agent::{FileDecision, Review};
        // Let the renderer flush events queued before this request.
        tokio::task::yield_now().await;
        crate::brand::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
        trace_item(kor_core::ipc_trace::Surface::Cli, kor_agent::remote::wire::confirm_json(req));
        crate::status_cmd::call_me(self.ctx, &req.tool, &req.summary);
        eprint!("\r\x1b[2K");
        for f in &req.files {
            for line in tui::review::diff(f).lines() {
                let shown = match line.as_bytes().first() {
                    Some(b'+') if !line.starts_with("+++") => self.style.paint("32", line),
                    Some(b'-') if !line.starts_with("---") => self.style.red(line),
                    _ => self.style.dim(line),
                };
                eprintln!("{shown}");
            }
        }
        if let Some(key) = &req.destructive {
            eprintln!("\n{}", self.style.red(&tui::review::destructive(self.l, key)));
        }
        let key = if req.files.is_empty() { "perm-prompt" } else { "perm-review" };
        eprint!(
            "\n{} ",
            self.style.ember(&self.l.t_args(
                key,
                &args!("tool" => req.tool.clone(), "summary" => req.summary.clone()),
            ))
        );
        let _ = std::io::stderr().flush();
        let answer = read_line().await;
        let reply = |r: PermissionReply| Review::from(r);
        let review = match answer.as_deref().map(str::trim) {
            Some("y" | "Y") => reply(PermissionReply::Allow),
            Some("a" | "A") => reply(PermissionReply::AllowAlways),
            Some("e" | "E") if req.files.len() == 1 => {
                let mut r = reply(PermissionReply::Allow);
                r.files.insert(req.files[0].path.clone(), self.edit(&req.files[0]).await);
                r
            }
            Some("f" | "F") if !req.files.is_empty() => {
                let mut r = reply(PermissionReply::Allow);
                for f in &req.files {
                    eprint!(
                        "{} ",
                        self.style.ember(&self.l.t_args("perm-file", &args!("path" => f.path.clone())))
                    );
                    let _ = std::io::stderr().flush();
                    let d = match read_line().await.as_deref().map(str::trim) {
                        Some("a" | "A") => FileDecision::Accept,
                        Some("e" | "E") => self.edit(f).await,
                        _ => FileDecision::Reject,
                    };
                    r.files.insert(f.path.clone(), d);
                }
                r
            }
            _ => reply(PermissionReply::Deny),
        };
        crate::brand::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
        crate::status_cmd::call_me_done();
        review
    }

    /// §28.13 the compact choice card: numbered options, the recommended one marked; Enter takes
    /// it, `2!` picks 2 and saves it as a project rule.
    async fn choose(&self, q: &kor_agent::ask::Question) -> Option<kor_agent::ask::Choice> {
        tokio::task::yield_now().await;
        crate::brand::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
        eprint!("\r\x1b[2K");
        eprintln!("\n{}", self.style.ember(&format!("? {}", q.question)));
        for (i, o) in q.options.iter().enumerate() {
            let mark = if i == q.default { "*" } else { " " };
            eprintln!("  {mark}{} {o}", i + 1);
        }
        eprint!("{} ", self.style.ember(&self.l.t_args("ask-choose", &args!("default" => q.default + 1))));
        let _ = std::io::stderr().flush();
        let answer = read_line().await;
        crate::brand::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
        let a = answer?.trim().to_owned();
        let remember = a.ends_with('!');
        let index = a
            .trim_end_matches('!')
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=q.options.len()).contains(n))
            .map_or(q.default, |n| n - 1);
        Some(kor_agent::ask::Choice { index, remember })
    }

    /// §28.1 what an automatic summary replaces; with `compact_confirm` the user keeps items by
    /// number or cancels.
    async fn compact(&self, items: &[kor_agent::ask::CompactItem], confirm: bool) -> kor_agent::ask::CompactChoice {
        use kor_agent::ask::CompactChoice;
        let (indices, labels) = compact_labels(items);
        eprintln!("\n{}", self.style.ember(&self.l.t_args("compact-preview", &args!("count" => items.len()))));
        for (n, label) in labels.iter().enumerate() {
            eprintln!("  {} {label}", n + 1);
        }
        if !confirm {
            return CompactChoice::default();
        }
        eprint!("{} ", self.style.ember(&self.l.t("compact-preview-ask")));
        let _ = std::io::stderr().flush();
        crate::brand::PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
        let answer = read_line().await.unwrap_or_default();
        crate::brand::PAUSED.store(false, std::sync::atomic::Ordering::Relaxed);
        let a = answer.trim();
        if matches!(a, "c" | "C" | "i" | "İ" | "q") {
            return CompactChoice::Cancel;
        }
        let keep = a
            .split([',', ' '])
            .filter_map(|n| n.trim().parse::<usize>().ok())
            .filter_map(|n| n.checked_sub(1).and_then(|i| indices.get(i).copied()))
            .collect();
        CompactChoice::Proceed { keep }
    }
}

impl TerminalApprover<'_> {
    /// The proposal opened in `$EDITOR`; what the user saves is written. If the editor fails
    /// the file is rejected (nothing the user did not see is written).
    async fn edit(&self, f: &kor_agent::Proposal) -> kor_agent::FileDecision {
        let (path, text) = (f.path.clone(), f.after.clone().unwrap_or_default());
        let r = tokio::task::spawn_blocking(move || tui::review::edit_in_editor(&path, &text)).await;
        match r.unwrap_or_else(|e| Err(e.to_string())) {
            Ok(content) => kor_agent::FileDecision::Edit(content),
            Err(error) => {
                eprintln!("{}", self.style.red(&self.l.t_args("perm-editor-failed", &args!("error" => error))));
                kor_agent::FileDecision::Reject
            }
        }
    }
}

/// Print mode cannot ask, so anything needing approval is refused.
struct DenyAll;

#[async_trait]
impl Approver for DenyAll {
    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
}

#[allow(clippy::too_many_lines)] // one arm per event kind and display mode
async fn render(
    mut rx: UnboundedReceiver<AgentEvent>,
    style: &Style,
    print_mode: bool,
    lang: kor_i18n::Lang,
) {
    let mut out = std::io::stdout();
    // The ember spinner runs while Kor waits for the model, and stops at its first output.
    let animate = style.color && !print_mode && std::io::stderr().is_terminal() && !tui::term::still();
    // §28.9 `ui.wrap` / `ui.wrap_width`: model text wraps at word boundaries.
    let mut wrap = tui::term::Wrapper::new(if print_mode { 0 } else { tui::term::wrap_width() });
    let mut spin = crate::brand::Spinner::new();
    let mut waiting = true;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(80));
    loop {
        let ev = tokio::select! {
            ev = rx.recv() => ev,
            _ = tick.tick(), if animate && waiting => {
                spin.draw();
                continue;
            }
        };
        let Some(ev) = ev else { break };
        trace_ev(kor_core::ipc_trace::Surface::Cli, &ev);
        spin.clear();
        // §28.1 `--json`: one JSON object per line; the context breakdown is a `context.usage` event.
        if tui::term::json() {
            if let Some(line) = tui::term::json_event(&ev) {
                println!("{line}");
            }
            continue;
        }
        if let AgentEvent::Context(b) = &ev {
            spin.set_context(b.percent(b.total()));
        }
        match &ev {
            AgentEvent::Text(_) | AgentEvent::Thinking(_) => {
                waiting = false;
            }
            // §16.2: a running tool (build, tests) keeps the spinner moving under its line.
            AgentEvent::ToolStart { .. } | AgentEvent::ToolEnd { .. } => {
                waiting = true;
            }
            AgentEvent::Status(s) => {
                // §16.2: verb, elapsed and tokens come from the core; draw at once.
                spin.set_status(s.clone());
                if animate && waiting {
                    spin.draw();
                }
                continue;
            }
            _ => {}
        }
        match ev {
            AgentEvent::Text(t) => {
                let _ = out.write_all(wrap.push(&t).as_bytes());
                let _ = out.flush();
            }
            _ if print_mode => {}
            AgentEvent::Thinking(t) => eprint!("{}", style.dim(&t)),
            AgentEvent::ToolProgress {
                message: Some(m), ..
            } => eprintln!("{}", style.dim(&format!("  … {m}"))),
            AgentEvent::ToolProgress { .. } | AgentEvent::Status(_) => {}
            AgentEvent::Usage(u) => spin.add_usage(u.input, u.output),
            // §27.16: the estimate before the first model call; the spinner then shows spend vs it.
            AgentEvent::Estimate(p) => {
                eprintln!("{}", style.dim(&format!("  {} ({})", p.line, p.method)));
                spin.set_estimate(p);
            }
            AgentEvent::Context(b) => {
                // §5.3 status line per request, plus the one-time "summarize?" offer.
                let l = kor_i18n::Localizer::new(lang);
                eprintln!("{}", style.dim(&format!("  · {}", kor_runtime::context::status(&l, &b))));
                if let Some(w) = kor_runtime::context::warning(&l, &b) {
                    eprintln!("{}", style.ember(&w));
                }
            }
            AgentEvent::ToolStart { name, summary, .. } => {
                let _ = out.write_all(wrap.flush().as_bytes());
                // §28.9 `ui.tool_output = quiet`: only failures show.
                if tui::term::verbosity() != kor_config::Verbosity::Quiet {
                    println!("\n{} {name} {}", style.ember("⏺"), style.dim(&summary));
                }
            }
            AgentEvent::ToolEnd {
                output, is_error, ..
            } if tui::term::verbosity() != kor_config::Verbosity::Normal || tui::term::images().is_some() => {
                let v = tui::term::verbosity();
                if v == kor_config::Verbosity::Verbose {
                    for line in output.lines().take(200) {
                        let line = format!("  │ {line}");
                        println!("{}", if is_error { style.red(&line) } else { style.dim(&line) });
                    }
                } else if is_error || v == kor_config::Verbosity::Normal {
                    let first: String = output.lines().next().unwrap_or_default().chars().take(120).collect();
                    let line = format!("  ⎿ {first}");
                    println!("{}", if is_error { style.red(&line) } else { style.dim(&line) });
                }
                // §28.11: generated images inline in kitty / sixel terminals; the path otherwise.
                for p in tui::term::image_paths(&output) {
                    tui::term::show_image(&p, tui::term::images());
                }
            }
            AgentEvent::ToolEnd {
                output, is_error, ..
            } => {
                let lines = output.lines().count();
                let first = output.lines().next().unwrap_or_default();
                let first: String = first.chars().take(120).collect();
                let more = if lines > 1 {
                    format!(" (+{} lines)", lines - 1)
                } else {
                    String::new()
                };
                let line = format!("  ⎿ {first}{more}");
                println!(
                    "{}",
                    if is_error {
                        style.red(&line)
                    } else {
                        style.dim(&line)
                    }
                );
            }
            AgentEvent::Notice(n) => eprintln!("{}", style.dim(&format!("  · {n}"))),
        }
    }
    spin.clear();
    let _ = out.write_all(wrap.flush().as_bytes());
    let _ = out.flush();
}

struct Session {
    agent: Agent,
    path: Option<PathBuf>,
    mode_note: String,
    compat: kor_compat::Compat,
    resumed: bool,
    fast: bool,
    effort: String,
    output_style: String,
    vim: bool,
    /// Cost router and ledger; its summary is printed when the session ends (§5.2).
    cost: Arc<kor_runtime::cost::Control>,
    lang: kor_i18n::Lang,
}

impl Drop for Session {
    /// Session cost summary on stderr, so piped output (`-p`) stays clean.
    fn drop(&mut self) {
        let l = Localizer::new(self.lang);
        for line in self.cost.summary_lines(&l) {
            eprintln!("{line}");
        }
    }
}

fn build_provider(ctx: &Ctx, model: &str) -> Option<Built> {
    let mut notes = Vec::new();
    let built = kor_runtime::build_provider(&ctx.l, model, &mut notes);
    for n in &notes {
        eprintln!("{n}");
    }
    match built {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("{e}");
            None
        }
    }
}

fn setup(ctx: &Ctx, a: &RunArgs) -> anyhow::Result<Option<Session>> {
    let l = &ctx.l;
    let root = crate::compat::project_root().context("current directory")?;
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", crate::config_error(l, e));
            return Ok(None);
        }
    };
    let opts = kor_runtime::Options {
        root,
        model: a.model.clone(),
        mode: a.mode,
        resume: a.resume.clone(),
        new_id: None,
        surface: kor_agent::prompt::Surface::Terminal,
    };
    let mut notes = Vec::new();
    let prepared = kor_runtime::prepare(l, loaded, &opts, &mut notes);
    for n in &notes {
        eprintln!("{n}");
    }
    let p = match prepared {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return Ok(None);
        }
    };
    let mode_note = l.t_args(
        "repl-welcome",
        &args!("model" => p.model.to_string(), "mode" => p.mode.as_str(), "sandbox" => p.sandbox.as_str()),
    );
    Ok(Some(Session {
        agent: p.agent,
        path: p.path,
        mode_note,
        compat: p.compat,
        resumed: a.resume.is_some(),
        fast: false,
        effort: "high".into(),
        output_style: "default".into(),
        vim: false,
        cost: p.cost,
        lang: l.lang(),
    }))
}

type Busy = Arc<Mutex<Option<CancellationToken>>>;

/// First Ctrl+C interrupts the running turn; Ctrl+C while idle exits.
fn install_ctrl_c(busy: Busy) {
    tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            let current = busy.lock().map(|b| b.clone()).unwrap_or(None);
            match current {
                Some(t) if !t.is_cancelled() => t.cancel(),
                _ => std::process::exit(130),
            }
        }
    });
}

/// Run one user message. Returns whether it finished normally.
async fn turn(
    s: &mut Session,
    text: String,
    ctx: &Ctx,
    style: &Style,
    print_mode: bool,
    busy: &Busy,
) -> bool {
    refresh_memory(&mut s.agent);
    let l = &ctx.l;
    let cancel = CancellationToken::new();
    if let Ok(mut b) = busy.lock() {
        *b = Some(cancel.clone());
    }
    let (tx, rx) = unbounded_channel();
    let approver_t = TerminalApprover { l, style, ctx };
    let approver: &dyn Approver = if print_mode { &DenyAll } else { &approver_t };
    let agent = &mut s.agent;
    let send = async move {
        let r = agent.send(text, &tx, approver, cancel).await;
        drop(tx);
        r
    };
    let (result, ()) = tokio::join!(send, render(rx, style, print_mode, l.lang()));
    if let Ok(mut b) = busy.lock() {
        *b = None;
    }
    // `--json` output is one object per line; a blank line would not parse.
    if !tui::term::json() {
        println!();
    }
    if let Some(p) = &s.path
        && let Err(e) = session::save(p, &s.agent.messages)
    {
        eprintln!(
            "{}",
            l.t_args("error-generic", &args!("error" => e.to_string()))
        );
    }
    let note = |key: &str| {
        eprintln!(
            "{}",
            style.dim(&l.t_args(key, &args!("n" => s.agent.max_turns)))
        );
    };
    match result {
        Ok(StopReason::Done(FinishReason::Stop | FinishReason::ToolUse)) => true,
        Ok(StopReason::Done(FinishReason::MaxTokens)) => {
            note("stop-max-tokens");
            true
        }
        Ok(StopReason::Done(FinishReason::ContentFiltered)) => {
            note("stop-filtered");
            false
        }
        Ok(StopReason::Done(_) | StopReason::Cancelled) => {
            note("stop-cancelled");
            false
        }
        Ok(StopReason::Budget | StopReason::MaxTurns) => {
            note("stop-max-turns");
            false
        }
        Ok(StopReason::Blocked(reason)) => {
            eprintln!(
                "{}",
                style.red(&l.t_args("prompt-blocked", &args!("reason" => reason)))
            );
            false
        }
        Err(e) => {
            let msg = provider_error(l, &e);
            trace_item(kor_core::ipc_trace::Surface::Cli, json!({ "type": "error", "message": msg }));
            eprintln!("{}", style.red(&msg));
            false
        }
    }
}

enum Slash {
    Quit,
    Done,
    /// A custom command or skill: name and arguments.
    Custom(String, String),
    /// Text to send as the next message (`/paste`, `/voice`; spec §10).
    Prompt(String),
}

/// Handle a built-in `/command`, or hand back a custom one.
#[allow(clippy::too_many_lines)]
async fn slash(s: &mut Session, line: &str, ctx: &Ctx) -> anyhow::Result<Slash> {
    let l = &ctx.l;
    let (cmd, arg) = line
        .split_once(' ')
        .map_or((line, ""), |(c, a)| (c, a.trim()));
    match cmd {
        "/exit" | "/quit" => return Ok(Slash::Quit),
        "/help" => {
            println!("{}", l.t("repl-help"));
            crate::compat::custom_help(l, &s.compat);
        }
        "/clear" => {
            s.agent.messages.clear();
            s.path = new_session_path();
            println!("{}", l.t("history-cleared"));
        }
        "/cost" if arg.is_empty() => {
            let u = s.agent.usage;
            println!(
                "{}",
                l.t_args(
                    "cost-line",
                    &args!("input" => u.input, "output" => u.output, "cached" => u.cache_read)
                )
            );
        }
        "/model" if !arg.is_empty() => {
            if let Some((p, model, key)) = build_provider(ctx, arg) {
                s.agent.provider = p;
                model.model().clone_into(&mut s.agent.model);
                if let Some(k) = key {
                    s.agent.redactor.register(k);
                }
                println!(
                    "{}",
                    l.t_args("model-set", &args!("model" => model.to_string()))
                );
            }
        }
        // §28.2: `/plan` takes the write tools away; `/go` approves the session's newest plan.
        "/plan" if arg.is_empty() => {
            s.agent.permissions.mode = PermissionMode::Plan;
            s.agent.ctx.plan_gate.enter();
            println!("{}", l.t("plan-mode-on"));
        }
        "/go" => {
            let root = s.agent.ctx.root.clone();
            let session = s.agent.ctx.plan_gate.session.clone();
            match kor_tools::plans::latest_draft(&root, session.as_deref()) {
                Some(p) => {
                    let p = kor_tools::plans::approve(&root, &p.id)?;
                    if s.agent.permissions.mode == PermissionMode::Plan && s.agent.ctx.plan_gate.refresh(&root) {
                        s.agent.permissions.mode = PermissionMode::Default;
                    }
                    println!("{}", l.t_args("plan-approved", &args!("id" => p.id, "title" => p.title)));
                }
                None => eprintln!("{}", l.t("plan-none-draft")),
            }
        }
        // §28.9: tool output detail, also while a task runs (the TUI applies it at once).
        "/verbosity" => match tui::term::parse_verbosity(arg) {
            Some(v) => {
                tui::term::set_verbosity(v);
                println!("{}", l.t_args("verbosity-set", &args!("level" => arg.to_owned())));
            }
            None => eprintln!("{}", l.t("verbosity-usage")),
        },
        "/mode" if !arg.is_empty() => match arg.parse::<PermissionMode>() {
            Ok(PermissionMode::Auto) if !s.agent.ctx.sandbox.is_enforcing() => {
                eprintln!("{}", l.t("mode-auto-needs-sandbox"));
            }
            Ok(m) => {
                s.agent.permissions.mode = m;
                println!("{}", l.t_args("mode-set", &args!("mode" => m.as_str())));
            }
            Err(_) => eprintln!("{}", l.t_args("bad-mode", &args!("mode" => arg.to_owned()))),
        },
        "/compact" => {
            let cancel = CancellationToken::new();
            // Asked for explicitly (also the answer to the context warning): summarize at any size.
            match kor_agent::compact::compact_now(&mut s.agent, cancel, "manual").await {
                Ok(true) => println!("{}", l.t("history-cleared")),
                Ok(false) => println!("compact: under the threshold"),
                Err(e) => eprintln!("{}", provider_error(l, &e)),
            }
        }
        "/memory" if arg.is_empty() => {
            if let Some(lay) = memory_layout(&s.agent.ctx.root) {
                match memory::read_entrypoint(&lay) {
                    Some(text) => println!("{text}"),
                    None => println!("{}", kor_tools::display_path(&lay.entry)),
                }
            }
        }
        "/agents" if arg.is_empty() => {
            println!("general-purpose");
            println!("Explore");
            println!("Plan");
            for a in kor_agent::builtin::with_builtins(s.compat.agents.clone()) {
                let short: String = a.description.chars().take(100).collect();
                let origin = if a.source.path.starts_with("<built-in>") {
                    "built-in"
                } else {
                    "custom"
                };
                println!("{}  [{origin}]  {short}", a.name);
            }
        }
        "/mcp" if arg.is_empty() => {
            crate::print_mcp_servers(&s.agent.ctx.root, s.compat.trusted);
            let mcp_tools: Vec<&str> = s
                .agent
                .tools
                .iter()
                .map(|t| t.name())
                .filter(|n| n.starts_with("mcp__"))
                .collect();
            if !mcp_tools.is_empty() {
                println!("connected tools: {}", mcp_tools.join(", "));
            }
        }
        "/config" if arg.is_empty() => {
            if let Some(p) = &ctx.paths.user {
                println!("{}", p.display());
            }
            if let Some(p) = &ctx.paths.project {
                println!("{}", p.display());
            }
        }
        "/dream" => {
            s.agent.dream = true;
            println!("dream armed for the next finished turn");
        }
        "/theme" | "/color" => {
            if crate::tui::theme_color(arg).is_some() {
                println!("{arg}");
            } else {
                println!("{}", crate::tui::THEMES.join(" "));
            }
        }
        "/stickers" => println!("pause play running done fast"),
        "/vim" => {
            s.vim = !s.vim;
            let key = kor_agent::port::parse_keystroke("ctrl+k");
            println!("vim {} ctrl={}", s.vim, key.ctrl);
        }
        // §28.1: what is in the context, item by item; `drop <n>` removes one, `pin <n>` keeps it
        // through the next summary.
        "/context" if arg == "items" || arg.starts_with("drop ") || arg.starts_with("pin ") => {
            let items = kor_agent::ask::compact_items(&s.agent.messages);
            let n = arg.split_whitespace().nth(1).and_then(|n| n.parse::<usize>().ok());
            match (arg.split_whitespace().next(), n) {
                (Some("drop"), Some(i)) if kor_agent::ask::drop_item(&mut s.agent.messages, i) => {
                    if let Some(p) = &s.path {
                        let _ = session::save(p, &s.agent.messages);
                    }
                    println!("{}", l.t_args("context-dropped", &args!("n" => i)));
                }
                (Some("pin"), Some(i)) if i < items.len() => {
                    if !s.agent.context.pinned.contains(&i) {
                        s.agent.context.pinned.push(i);
                    }
                    println!("{}", l.t_args("context-pinned", &args!("n" => i)));
                }
                (Some("drop" | "pin"), _) => eprintln!("{}", l.t("context-item-usage")),
                _ => {
                    for it in &items {
                        let pin = if s.agent.context.pinned.contains(&it.index) { "*" } else { " " };
                        println!("{pin}{:>4}  {:<9} ≈{:>6}  {}", it.index, it.kind, it.tokens, it.label);
                    }
                    println!("{}", l.t("context-item-usage"));
                }
            }
        }
        "/context" => {
            if let Some(b) = &s.agent.context.last {
                print!("{}", kor_runtime::context::render(l, b));
            } else {
                let cfg = ctx.config.as_ref().map(|c| c.config.context.clone()).unwrap_or_default();
                print!("{}", kor_runtime::context::report(l, &cfg, 1));
            }
        }
        "/rewind" => {
            if let Some(path) = &s.path
                && let Ok(mut messages) = kor_agent::session::load(path)
            {
                let flags: Vec<bool> = messages
                    .iter()
                    .map(|m| m.role == kor_core::Role::User)
                    .collect();
                let keep = kor_agent::port::rewind_to_prior_user(&flags);
                messages.truncate(keep);
                let _ = kor_agent::session::save(path, &messages);
                s.agent.messages = messages;
                println!("rewound");
            }
        }
        "/diff" => println!("{}", kor_agent::port::unified_diff("session", "", arg)),
        "/export" if arg.starts_with("--all") => {
            crate::run_slash(ctx, line);
        }
        "/export" | "/copy" => {
            if let Some(path) = &s.path {
                println!("{}", path.display());
            }
        }
        "/tasks" => {
            let jobs = s.agent.ctx.jobs.lock().map(|g| g.len()).unwrap_or(0);
            println!("{jobs} background tasks");
        }
        "/sandbox-toggle" => {
            s.agent.ctx.sandbox = if s.agent.ctx.sandbox == kor_sandbox::SandboxKind::None {
                kor_sandbox::detect()
            } else {
                kor_sandbox::SandboxKind::None
            };
            println!("{}", s.agent.ctx.sandbox.as_str());
        }
        "/fast" => {
            s.fast = !s.fast;
            println!("{}", s.fast);
        }
        "/effort" => {
            if matches!(arg, "low" | "medium" | "high" | "max") {
                s.effort = arg.to_owned();
            }
            println!("{}", s.effort);
        }
        "/output-style" => {
            if matches!(arg, "default" | "Explanatory" | "Learning") {
                s.output_style = arg.to_owned();
            }
            println!("{}", s.output_style);
        }
        // `/review` is `kor review` (§8.8): it falls through to `run_slash`.
        "/chrome" | "/mobile" | "/install-github-app" => {
            if let Some(url) = kor_agent::port::command_url(match cmd {
                "/mobile" => "mobile-ios",
                other => other.trim_start_matches('/'),
            }) {
                println!("{url}");
            }
        }
        "/bridge" | "/remote-setup" | "/teleport" | "/usage" | "/desktop" => {
            println!("{cmd} has no URL constant in its command file");
        }
        "/doctor" => {
            let _ = crate::doctor::run(ctx);
        }
        "/resume" => {
            if let Some(p) = &s.path {
                println!("{}", kor_tools::display_path(p));
            }
        }
        // Spec §10: a clipboard image (error screenshot) becomes local OCR text in the message.
        "/paste" => {
            if let Some(text) = crate::screenshot_cmd::paste_prompt(l, arg).await {
                return Ok(Slash::Prompt(text));
            }
        }
        // Spec §10: speech → local whisper.cpp → the next message. Runs off this runtime.
        "/voice" => {
            let a = crate::voice_cmd::VoiceArgs {
                seconds: arg.parse::<u32>().unwrap_or(10).clamp(1, 120),
                language: None,
                yes: false,
            };
            let heard = std::thread::scope(|t| t.spawn(|| crate::voice_cmd::listen(ctx, &a)).join());
            if let Ok(Some(text)) = heard {
                println!("{text}");
                return Ok(Slash::Prompt(text));
            }
        }
        "/permissions" => {
            println!(
                "{}",
                l.t_args(
                    "mode-set",
                    &args!("mode" => s.agent.permissions.mode.as_str())
                )
            );
            for (kind, rules) in s.agent.permissions.rules() {
                for r in rules {
                    let spec = r
                        .specifier
                        .as_deref()
                        .map(|x| format!("({x})"))
                        .unwrap_or_default();
                    println!("  {kind}: {}{spec}", r.tool);
                }
            }
        }
        _ => {
            // Every `kor <command>` also works here as `/<command>` (parity.toml): same parser, same code.
            if crate::run_slash(ctx, line).is_some() {
                return Ok(Slash::Done);
            }
            let name = cmd.trim_start_matches('/');
            if crate::compat::expand(&s.compat, name, "").is_some() {
                return Ok(Slash::Custom(name.to_owned(), arg.to_owned()));
            }
            eprintln!(
                "{}",
                l.t_args("unknown-command", &args!("cmd" => cmd.to_owned()))
            );
        }
    }
    Ok(Slash::Done)
}

/// Expand a custom slash command: run its `!` commands through the normal permission
/// checks (with its `allowed-tools` granted for the duration) and inline `@` files.
#[allow(clippy::format_push_string)]
async fn expand_custom(
    s: &mut Session,
    name: &str,
    args: &str,
    ctx: &Ctx,
    style: &Style,
    print_mode: bool,
) -> Option<String> {
    let exp = crate::compat::expand(&s.compat, name, args)?;
    if let Some(e) = &exp.error {
        eprintln!("{}", crate::compat::input_error(&ctx.l, name, e));
        return None;
    }
    let saved = s.agent.permissions.clone();
    if let Err(e) = s.agent.permissions.add_allow(&exp.allowed_tools) {
        eprintln!(
            "{}",
            ctx.l
                .t_args("error-generic", &args!("error" => e.to_string()))
        );
    }
    s.agent.ctx.cancel = CancellationToken::new();
    let (tx, rx) = unbounded_channel();
    let approver_t = TerminalApprover { l: &ctx.l, style, ctx };
    let approver: &dyn Approver = if print_mode { &DenyAll } else { &approver_t };
    let mut out = String::new();
    for part in exp.parts {
        match part {
            Part::Text(t) => out.push_str(&t),
            Part::Shell(cmd) => {
                let o = s
                    .agent
                    .run_tool_now("Bash", json!({"command": cmd}), &tx, approver)
                    .await;
                out.push_str(o.content.trim_end());
            }
            Part::File(f) => {
                let text = s
                    .agent
                    .ctx
                    .resolve(&f)
                    .ok()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .filter(|t| t.len() <= 256 * 1024);
                match text {
                    Some(t) => out.push_str(&format!("\n<file path=\"{f}\">\n{t}\n</file>\n")),
                    None => out.push_str(&format!("@{f}")),
                }
            }
        }
    }
    s.agent.permissions = saved;
    drop(tx);
    render(rx, style, print_mode, ctx.l.lang()).await;
    Some(out)
}

/// Run `SessionStart` hooks; their output joins the system prompt.
async fn session_start(s: &mut Session, style: &Style) {
    let Some(h) = s.agent.hooks.clone() else {
        return;
    };
    let source = if s.resumed { "resume" } else { "startup" };
    let o = h.run("SessionStart", None, json!({"source": source})).await;
    for n in &o.notices {
        eprintln!("{}", style.dim(&format!("  · {n}")));
    }
    for c in o.context {
        s.agent
            .system
            .push_str("\n\n# Context from a SessionStart hook\n\n");
        s.agent.system.push_str(&c);
    }
}

/// §28.6 `[auto] session_suggest`: a title and tags for an untagged session, applied with
/// `kor sessions suggest <id> --apply`.
fn suggest_at_end(ctx: &Ctx, s: &Session, style: &Style) {
    use kor_runtime::session_tree as tree;
    if !ctx.config.as_ref().is_ok_and(|c| c.config.auto.session_suggest) || s.agent.messages.len() < 2 {
        return;
    }
    let (Some(path), Some(meta)) = (&s.path, tree::meta_path()) else { return };
    let (Some(id), Some(dir)) = (path.file_stem().and_then(|x| x.to_str()), path.parent()) else { return };
    let tagged = tree::list(dir, &meta, &tree::Filter { archived: true, ..tree::Filter::default() })
        .ok()
        .and_then(|all| all.into_iter().find(|e| e.id == id))
        .is_none_or(|e| !e.tags.is_empty());
    let sug = tree::suggest(&s.agent.messages);
    if tagged || sug.tags.is_empty() {
        return;
    }
    let tags = sug.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ");
    eprintln!(
        "{}",
        style.dim(&ctx.l.t_args("sessions-suggest-end", &args!("id" => id.to_owned(), "title" => sug.title, "tags" => tags)))
    );
}

async fn session_end(s: &Session, reason: &str) {
    if let Some(h) = &s.agent.hooks {
        let _ = h.run("SessionEnd", None, json!({"reason": reason})).await;
    }
}

/// One-shot prompt: `/name args` expands a custom command first.
async fn one_shot(
    s: &mut Session,
    p: &str,
    ctx: &Ctx,
    style: &Style,
    print_mode: bool,
    busy: &Busy,
) -> bool {
    let text = match p.strip_prefix('/') {
        Some(rest) => {
            let (name, args) = rest.split_once(' ').unwrap_or((rest, ""));
            if let Some(e) = crate::compat::expand(&s.compat, name, args.trim()).and_then(|x| x.error) {
                eprintln!("{}", crate::compat::input_error(&ctx.l, name, &e));
                return false;
            }
            match expand_custom(s, name, args.trim(), ctx, style, print_mode).await {
                Some(t) => t,
                None => p.to_owned(),
            }
        }
        // §30: `kor "maliyetimi göster"` runs `kor cost`; `-p` (scripts) always goes to the agent.
        None if !print_mode && crate::intent_cmd::enabled(ctx) => {
            let model = Some((s.agent.provider.clone(), s.agent.model.clone()));
            let interactive = std::io::stdin().is_terminal();
            let actions = crate::intent_cmd::route_line(ctx, p, model, interactive).await;
            return run_intent(s, actions, ctx, style, busy).await.is_ok_and(|(_, ok)| ok);
        }
        None => p.to_owned(),
    };
    turn(s, text, ctx, style, print_mode, busy).await
}

pub fn run(ctx: &Ctx, a: &RunArgs) -> anyhow::Result<bool> {
    start(ctx, a, false)
}

/// `kor tui`: the full-screen interface whatever `ui.default_tui` says.
pub fn run_tui(ctx: &Ctx, model: Option<String>, mode: Option<PermissionMode>) -> anyhow::Result<bool> {
    let a = RunArgs {
        model,
        mode,
        ..RunArgs::default()
    };
    start(ctx, &a, true)
}

fn start(ctx: &Ctx, a: &RunArgs, force_tui: bool) -> anyhow::Result<bool> {
    kor_core::ipc_trace::init_from_env();
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if force_tui && !terminal {
        eprintln!("{}", ctx.l.t("tui-needs-terminal"));
        return Ok(false);
    }
    // `kor` alone: the TUI unless `ui.default_tui = false` or `KOR_UI=line`.
    let full_screen = terminal
        && (force_tui
            || (std::env::var_os("KOR_UI").is_none_or(|v| v != "line")
                && ctx.config.as_ref().is_ok_and(|c| c.config.ui.default_tui)));
    let Some(mut s) = setup(ctx, a)? else {
        return Ok(false);
    };
    if let Ok(c) = &ctx.config {
        tui::term::init(&c.config);
    }
    let style = Style::detect();
    let l = &ctx.l;
    crate::providers::runtime()?.block_on(async {
        let busy: Busy = Arc::default();
        install_ctrl_c(busy.clone());
        // §32.2: warm the provider connection while MCP servers attach and the user types.
        s.agent.provider.preconnect();

        let mcp = kor_runtime::attach_mcp(l, &mut s.agent, &s.compat).await;
        for n in &mcp.notes {
            eprintln!("{n}");
        }
        if let Some(line) = &mcp.connected
            && !a.print
        {
            eprintln!("{}", style.dim(line));
        }

        session_start(&mut s, &style).await;
        if let Some(p) = &a.prompt {
            let ok = one_shot(&mut s, p, ctx, &style, a.print, &busy).await;
            session_end(&s, "exit").await;
            return Ok(ok);
        }

        // §16.1: small banner below 60 columns, colour depth from the terminal (truecolor / 256 / none).
        println!("{}\n", crate::brand::banner_here());
        println!("{}", style.ember(&s.mode_note));
        println!(
            "{}",
            style.dim(&l.t_args(
                "repl-root",
                &args!("root" => kor_tools::display_path(&s.agent.ctx.root))
            ))
        );
        println!("{}", style.dim(&l.t("repl-hint")));
        if full_screen {
            let ok = tui_session(&mut s, ctx, &style, &busy).await?;
            session_end(&s, "exit").await;
            suggest_at_end(ctx, &s, &style);
            return Ok(ok);
        }
        loop {
            print!("\n{} ", style.ember("✶"));
            let _ = std::io::stdout().flush();
            let Some(line) = read_line().await else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if dispatch_line(&mut s, line, ctx, &style, &busy).await? {
                break;
            }
        }
        session_end(&s, "exit").await;
        suggest_at_end(ctx, &s, &style);
        Ok(true)
    })
}

struct Ask {
    tool: String,
    summary: String,
    files: Vec<String>,
    destructive: Option<String>,
    /// The proposed content of each file in `files` (a reviewed write), for `e<n>` on the full screen.
    texts: Vec<String>,
    /// A question or the compaction preview instead of a permission (ADR 0050).
    kind: AskKind,
    reply: tokio::sync::oneshot::Sender<kor_agent::Review>,
}

#[derive(Debug, Clone, Default)]
enum AskKind {
    #[default]
    Permission,
    /// §28.13: options and the recommended one.
    Choice { options: Vec<String>, default: usize },
    /// §28.1: labels shown (largest items) and their conversation indices.
    Compact { indices: Vec<usize> },
}

/// The TUI answers questions and the compaction preview through the same reply channel as a
/// permission; these keys of `Review::files` carry the answer.
const CHOICE_KEY: &str = "#choice";
const REMEMBER_KEY: &str = "#remember";
const KEEP_KEY: &str = "#keep";

fn answer_review(reply: PermissionReply, fields: &[(&str, String)]) -> kor_agent::Review {
    let mut r = kor_agent::Review::from(reply);
    for (k, v) in fields {
        r.files.insert((*k).to_owned(), kor_agent::FileDecision::Edit(v.clone()));
    }
    r
}

fn review_field(r: &kor_agent::Review, key: &str) -> Option<String> {
    match r.files.get(key) {
        Some(kor_agent::FileDecision::Edit(v)) => Some(v.clone()),
        _ => None,
    }
}

/// The largest parts of the conversation, at most 9 (keys 1-9 mark them), with labels.
fn compact_labels(items: &[kor_agent::ask::CompactItem]) -> (Vec<usize>, Vec<String>) {
    let mut big: Vec<&kor_agent::ask::CompactItem> = items.iter().collect();
    big.sort_by(|a, b| b.tokens.cmp(&a.tokens));
    big.truncate(9);
    big.sort_by_key(|i| i.index);
    (
        big.iter().map(|i| i.index).collect(),
        big.iter().map(|i| format!("{} {} ≈{}", i.kind, i.label, i.tokens)).collect(),
    )
}

struct ChanApprover {
    tx: tokio::sync::mpsc::UnboundedSender<Ask>,
}

#[async_trait]
impl Approver for ChanApprover {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.review(req).await.reply
    }

    async fn choose(&self, q: &kor_agent::ask::Question) -> Option<kor_agent::ask::Choice> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let ask = Ask {
            tool: "AskUserQuestion".into(),
            summary: q.question.clone(),
            files: Vec::new(),
            destructive: None,
            texts: Vec::new(),
            kind: AskKind::Choice { options: q.options.clone(), default: q.default },
            reply,
        };
        self.tx.send(ask).ok()?;
        let r = rx.await.ok()?;
        Some(kor_agent::ask::Choice {
            index: review_field(&r, CHOICE_KEY).and_then(|v| v.parse().ok()).unwrap_or(q.default),
            remember: review_field(&r, REMEMBER_KEY).is_some(),
        })
    }

    async fn compact(&self, items: &[kor_agent::ask::CompactItem], confirm: bool) -> kor_agent::ask::CompactChoice {
        use kor_agent::ask::CompactChoice;
        if !confirm {
            return CompactChoice::default();
        }
        let (indices, labels) = compact_labels(items);
        let tokens: u64 = items.iter().map(|i| i.tokens).sum();
        let (reply, rx) = tokio::sync::oneshot::channel();
        let ask = Ask {
            tool: "compact".into(),
            summary: format!("{} · ≈{tokens}", items.len()),
            files: labels,
            destructive: None,
            texts: Vec::new(),
            kind: AskKind::Compact { indices },
            reply,
        };
        if self.tx.send(ask).is_err() {
            return CompactChoice::default();
        }
        match rx.await {
            Ok(r) if r.reply == PermissionReply::Deny => CompactChoice::Cancel,
            Ok(r) => CompactChoice::Proceed {
                keep: review_field(&r, KEEP_KEY)
                    .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
                    .unwrap_or_default(),
            },
            Err(_) => CompactChoice::default(),
        }
    }

    async fn review(&self, req: &PermissionRequest) -> kor_agent::Review {
        trace_item(kor_core::ipc_trace::Surface::Tui, kor_agent::remote::wire::confirm_json(req));
        let (reply, rx) = tokio::sync::oneshot::channel();
        if self
            .tx
            .send(Ask {
                tool: req.tool.clone(),
                summary: req.summary.clone(),
                files: req.files.iter().map(|f| f.path.clone()).collect(),
                destructive: req.destructive.clone(),
                texts: req.files.iter().map(|f| f.after.clone().unwrap_or_default()).collect(),
                kind: AskKind::Permission,
                reply,
            })
            .is_err()
        {
            return PermissionReply::Deny.into();
        }
        rx.await.unwrap_or_else(|_| PermissionReply::Deny.into())
    }
}

type Term = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>;

/// Next terminal event (key, mouse, resize), or `None` after a short wait.
fn read_event() -> std::io::Result<Option<crossterm::event::Event>> {
    if crossterm::event::poll(std::time::Duration::from_millis(40))? {
        return Ok(Some(crossterm::event::read()?));
    }
    Ok(None)
}

async fn poll_event() -> anyhow::Result<Option<crossterm::event::Event>> {
    Ok(tokio::task::spawn_blocking(read_event).await??)
}

fn term_area(terminal: &Term) -> anyhow::Result<ratatui::layout::Rect> {
    let size = terminal.size()?;
    Ok(ratatui::layout::Rect::new(0, 0, size.width, size.height))
}

/// Core state the side panels read; cloned handles, so they stay readable while a turn runs.
struct Sources {
    path: Option<PathBuf>,
    jobs: Arc<Mutex<std::collections::HashMap<String, kor_tools::Job>>>,
    cost: Arc<kor_runtime::cost::Control>,
    /// The session's project graph (`[graph] enabled`), for the graph panel.
    graph: Option<Arc<kor_agent::graph::Session>>,
}

fn sources(s: &Session) -> Sources {
    Sources {
        path: s.path.clone(),
        jobs: s.agent.ctx.jobs.clone(),
        cost: s.cost.clone(),
        graph: kor_agent::graph::find(&s.agent.ctx.root),
    }
}

/// Re-read the side panels; `quota` also builds the `kor quota` report (network only for
/// `OpenRouter` credits, never in privacy mode).
async fn tui_sync(src: &Sources, ctx: &Ctx, screen: &mut tui::Screen, quota: bool) {
    let l = &ctx.l;
    if let Some(dir) = src.path.as_deref().and_then(tui::changes::journal_dir) {
        tui::set_changes(screen, l, tui::changes::load(&dir));
    }
    screen.jobs = tui::data::jobs(&src.jobs);
    screen.cost = tui::data::cost_lines(l, &src.cost);
    screen.health = tui::data::health_lines(l);
    tui::graph::fill(screen, l, src.graph.as_deref());
    if quota && let Ok(c) = &ctx.config {
        screen.quota = tui::data::quota_lines(l, &c.config).await;
    }
}

/// What a loop still has to do after the reducer and its I/O ran.
enum Step {
    None,
    Submit(String),
    Decision(PermissionReply),
    Quit,
    Interrupt,
    /// Open file `n` of a reviewed write in `$EDITOR`.
    Edit(usize),
}

async fn tui_apply(out: Out, src: &Sources, ctx: &Ctx, screen: &mut tui::Screen) -> Step {
    let l = &ctx.l;
    match out {
        Out::None => {}
        Out::Submit(line) => return Step::Submit(line),
        Out::Decision(d) => return Step::Decision(d),
        Out::Quit => return Step::Quit,
        Out::Interrupt => return Step::Interrupt,
        Out::Refresh => tui_sync(src, ctx, screen, true).await,
        Out::Sync => tui_sync(src, ctx, screen, false).await,
        Out::Diff => tui::fill_diff(screen, l),
        Out::Keep(c) => tui::keep(screen, l, &c),
        Out::Revert(c) => tui::revert(screen, l, &c),
        Out::Graph => tui::graph::fill(screen, l, src.graph.as_deref()),
        Out::EditFile(i) => return Step::Edit(i),
    }
    Step::None
}

/// One agent turn on the full screen. Returns whether the user quit, and a message typed
/// meanwhile (sent next).
#[allow(clippy::too_many_lines)]
async fn tui_turn(
    s: &mut Session,
    ctx: &Ctx,
    text: String,
    screen: &mut tui::Screen,
    terminal: &mut Term,
    busy: &Busy,
) -> anyhow::Result<(bool, Option<String>)> {
    let l = &ctx.l;
    refresh_memory(&mut s.agent);
    let src = sources(s);
    let cancel = CancellationToken::new();
    if let Ok(mut b) = busy.lock() {
        *b = Some(cancel.clone());
    }
    let task: String = text.lines().next().unwrap_or_default().chars().take(80).collect();
    tui::reduce(screen, l, Ev::Busy(Some(task)));
    let (ev_tx, mut ev_rx) = unbounded_channel();
    let (ask_tx, mut ask_rx) = tokio::sync::mpsc::unbounded_channel();
    let approver = ChanApprover { tx: ask_tx };
    let mut pending: Option<tokio::sync::oneshot::Sender<kor_agent::Review>> = None;
    let mut pending_kind = AskKind::Permission;
    let mut quit = false;
    let mut queued = None;
    {
        let work = s.agent.send(text, &ev_tx, &approver, cancel.clone());
        tokio::pin!(work);
        let mut reader = tokio::task::spawn_blocking(read_event);
        // §16.2: the status frame turns while the agent works, between core events too.
        let mut anim = tokio::time::interval(std::time::Duration::from_millis(100));
        anim.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            // §32.3: frames over 50 ms go to perf.log when `[perf] log` is on.
            crate::bench_cmd::draw_logged(terminal, screen, l, ctx.config.as_ref().ok().map(|c| &c.config))?;
            let out = tokio::select! {
                biased;
                _ = anim.tick(), if !crate::tui::term::still() => continue,
                ev = ev_rx.recv() => {
                    let Some(ev) = ev else { continue };
                    trace_ev(kor_core::ipc_trace::Surface::Tui, &ev);
                    let Some(shown) = tui_ev(ev) else { continue };
                    tui::reduce(screen, l, shown)
                }
                ask = ask_rx.recv() => {
                    let Some(ask) = ask else { continue };
                    crate::status_cmd::call_me(ctx, &ask.tool, &ask.summary);
                    let kind = ask.kind.clone();
                    let ev = if let AskKind::Choice { options, default } = &ask.kind {
                        screen.perm_choices.clone_from(options);
                        screen.perm_choice = *default;
                        screen.perm_remember = false;
                        screen.perm_files.clear();
                        screen.perm_rejected.clear();
                        screen.perm_danger = None;
                        Ev::Ask(ask.summary)
                    } else if let AskKind::Compact { .. } = &ask.kind {
                        screen.perm_compact = true;
                        screen.perm_rejected = vec![false; ask.files.len()];
                        screen.perm_files = ask.files;
                        screen.perm_danger = None;
                        Ev::Ask(ask.summary)
                    } else if ask.tool == "EnterPlanMode" || ask.tool == "ExitPlanMode" {
                        Ev::AskPlan(ask.summary)
                    } else {
                        screen.perm_rejected = vec![false; ask.files.len()];
                        screen.perm_edited = vec![None; ask.files.len()];
                        screen.perm_texts = ask.texts;
                        screen.perm_files = ask.files;
                        screen.perm_danger =
                            ask.destructive.map(|k| tui::review::destructive(l, &k));
                        Ev::Ask(ask.summary)
                    };
                    pending_kind = kind;
                    pending = Some(ask.reply);
                    tui::reduce(screen, l, ev)
                }
                r = &mut reader => {
                    // The reader is kept across iterations: dropping it could lose a key it read.
                    reader = tokio::task::spawn_blocking(read_event);
                    let Some(e) = r?? else { continue };
                    let area = term_area(terminal)?;
                    let Some(ev) = tui::input_ev(screen, l, area, &e) else { continue };
                    tui::reduce(screen, l, ev)
                }
                result = &mut work => {
                    if let Err(e) = result {
                        let msg = provider_error(l, &e);
                        trace_item(kor_core::ipc_trace::Surface::Tui, json!({ "type": "error", "message": msg }));
                        tui::reduce(screen, l, Ev::Notice(msg.clone()));
                        tui::reduce(screen, l, Ev::Progress(msg));
                    }
                    break;
                }
            };
            match tui_apply(out, &src, ctx, screen).await {
                Step::Decision(d) => {
                    if let Some(tx) = pending.take() {
                        let review = match std::mem::take(&mut pending_kind) {
                            AskKind::Choice { .. } => {
                                let mut f = vec![(CHOICE_KEY, screen.perm_choice.to_string())];
                                if screen.perm_remember && d == PermissionReply::Allow {
                                    f.push((REMEMBER_KEY, String::new()));
                                }
                                answer_review(d, &f)
                            }
                            AskKind::Compact { indices } => {
                                let keep: Vec<String> = indices
                                    .iter()
                                    .zip(&screen.perm_rejected)
                                    .filter(|(_, k)| **k)
                                    .map(|(i, _)| i.to_string())
                                    .collect();
                                answer_review(d, &[(KEEP_KEY, keep.join(","))])
                            }
                            AskKind::Permission => {
                                let mut r = tui::review::from_marks(d, &screen.perm_files, &screen.perm_rejected);
                                tui::review::apply_edits(&mut r, &screen.perm_files, &screen.perm_rejected, &screen.perm_edited);
                                r
                            }
                        };
                        screen.perm_choices.clear();
                        screen.perm_compact = false;
                        let _ = tx.send(review);
                    }
                }
                step @ (Step::Interrupt | Step::Quit) => {
                    quit |= matches!(step, Step::Quit);
                    // A question still open is answered "deny", so the agent can stop.
                    if let Some(tx) = pending.take() {
                        let _ = tx.send(PermissionReply::Deny.into());
                    }
                    screen.perm = None;
                    screen.perm_choices.clear();
                    screen.perm_compact = false;
                    cancel.cancel();
                }
                Step::None => {}
                Step::Edit(i) => {
                    // Wait out the key reader (it polls 40 ms at a time) so it does not take the
                    // editor's keys, leave the full screen, edit, come back.
                    let _ = (&mut reader).await;
                    let t = tui_config(ctx);
                    leave_screen(&t);
                    let path = screen.perm_files.get(i).cloned().unwrap_or_default();
                    let text = screen.perm_texts.get(i).cloned().unwrap_or_default();
                    let edited = tokio::task::spawn_blocking(move || tui::review::edit_in_editor(&path, &text)).await;
                    enter_screen(&t)?;
                    terminal.clear()?;
                    match edited {
                        Ok(Ok(content)) => {
                            if let Some(slot) = screen.perm_edited.get_mut(i) {
                                *slot = Some(content);
                            }
                            if let Some(off) = screen.perm_rejected.get_mut(i) {
                                *off = false;
                            }
                        }
                        Ok(Err(e)) => screen.notice(l.t_args("error-generic", &args!("error" => e))),
                        Err(e) => screen.notice(l.t_args("error-generic", &args!("error" => e.to_string()))),
                    }
                    reader = tokio::task::spawn_blocking(read_event);
                }
                Step::Submit(line) => {
                    if let Some(v) = line.strip_prefix("/verbosity ").and_then(tui::term::parse_verbosity) {
                        tui::term::set_verbosity(v);
                        screen.notice(l.t_args("verbosity-set", &args!("level" => line[11..].trim().to_owned())));
                    } else {
                        queued = Some(line);
                    }
                }
            }
        }
        // A key read as the turn ended still counts.
        if let Ok(Ok(Some(e))) = reader.await
            && let Some(ev) = tui::input_ev(screen, l, term_area(terminal)?, &e)
        {
            let out = tui::reduce(screen, l, ev);
            match tui_apply(out, &src, ctx, screen).await {
                Step::Submit(line) => queued = Some(line),
                Step::Quit => quit = true,
                Step::Decision(_) | Step::Interrupt | Step::None | Step::Edit(_) => {}
            }
        }
    }
    if let Ok(mut b) = busy.lock() {
        *b = None;
    }
    tui::reduce(screen, l, Ev::Busy(None));
    if let Some(p) = &s.path
        && let Err(e) = session::save(p, &s.agent.messages)
    {
        screen.notice(l.t_args("error-generic", &args!("error" => e.to_string())));
    }
    tui_sync(&src, ctx, screen, false).await;
    screen.model.clone_from(&s.agent.model);
    Ok((quit, queued))
}

async fn tui_session(
    s: &mut Session,
    ctx: &Ctx,
    style: &Style,
    busy: &Busy,
) -> anyhow::Result<bool> {
    let t = tui_config(ctx);
    // §28.11: the title only when allowed, pushed on the title stack and restored on exit.
    if t.title {
        print!("{}", tui::term::push_title("Kor"));
    }
    if let Err(e) = enter_screen(&t) {
        leave_screen(&t);
        return Err(e.into());
    }
    let result = tui_main(s, ctx, style, busy).await;
    // Always give the terminal back, also after an error.
    leave_screen(&t);
    if t.title {
        print!("{}", tui::term::POP_TITLE);
        let _ = std::io::stdout().flush();
    }
    result
}

fn tui_config(ctx: &Ctx) -> kor_config::Tui {
    ctx.config.as_ref().map(|c| c.config.tui.clone()).unwrap_or_default()
}

/// §28.11: raw mode, plus the alternate screen and mouse capture only when configured; the
/// default runs inline on the main screen so scrollback stays intact.
fn enter_screen(t: &kor_config::Tui) -> std::io::Result<()> {
    use crossterm::event::EnableMouseCapture;
    use crossterm::execute;
    use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
    enable_raw_mode()?;
    if t.alt_screen {
        execute!(std::io::stdout(), EnterAlternateScreen)?;
    }
    if t.mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }
    Ok(())
}

fn leave_screen(t: &kor_config::Tui) {
    use crossterm::event::DisableMouseCapture;
    use crossterm::execute;
    use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
    let _ = disable_raw_mode();
    if t.mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    if t.alt_screen {
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

/// Full screen on the alternate screen, else an inline viewport as tall as the terminal (what was
/// on screen scrolls up into the scrollback instead of being overwritten).
fn new_terminal(t: &kor_config::Tui) -> std::io::Result<Term> {
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    if t.alt_screen {
        return ratatui::Terminal::new(backend);
    }
    let rows = crossterm::terminal::size().map_or(24, |(_, h)| h.max(10));
    ratatui::Terminal::with_options(
        backend,
        ratatui::TerminalOptions { viewport: ratatui::Viewport::Inline(rows) },
    )
}

async fn tui_main(s: &mut Session, ctx: &Ctx, style: &Style, busy: &Busy) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let t = tui_config(ctx);
    let mut terminal = new_terminal(&t)?;
    // §28.10: `[keys] mode` / `ui.tui_keys`, then `keymap.toml` (mode and per-key overrides).
    let keymap = kor_runtime::prefs::load_keymap(&s.agent.ctx.root);
    let keys = keymap.mode.unwrap_or_else(|| {
        ctx.config.as_ref().map(|c| c.config.key_mode()).unwrap_or_default()
    });
    let mut screen = tui::Screen::new(s.agent.permissions.mode.as_str(), s.agent.model.clone())
        .with_keys(keys)
        .with_keymap(&keymap.tui);
    for w in &keymap.warnings {
        screen.notice(w.clone());
    }
    screen.lines.push(s.mode_note.clone());
    tui_sync(&sources(s), ctx, &mut screen, true).await;
    let mut queued: Option<String> = None;
    // §30: the last intent proposal (text, offered commands, prefilled line, later pipeline steps).
    let mut offer: Option<(String, Vec<String>, String, Vec<String>)> = None;
    let mut steps: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    loop {
        let line = if let Some(q) = queued.take().or_else(|| steps.pop_front()) {
            q
        } else {
            crate::bench_cmd::draw_logged(&mut terminal, &screen, l, ctx.config.as_ref().ok().map(|c| &c.config))?;
            let Some(e) = poll_event().await? else { continue };
            let area = term_area(&terminal)?;
            let Some(ev) = tui::input_ev(&mut screen, l, area, &e) else { continue };
            let out = tui::reduce(&mut screen, l, ev);
            match tui_apply(out, &sources(s), ctx, &mut screen).await {
                Step::Submit(line) => line,
                Step::Quit | Step::Interrupt => break,
                Step::Decision(_) | Step::None | Step::Edit(_) => continue,
            }
        };
        if let Some((said, offered, prefill, then)) = offer.take() {
            // A proposal was open: a `/command` answers it (learned when it is a correction);
            // free text means "no, send it to the agent". Accepting a pipeline queues its rest.
            if line.starts_with('/') {
                crate::intent_cmd::learn_tui_choice(&said, &offered, &line);
            }
            if line.trim() == prefill.trim() {
                steps.extend(then);
            }
        } else if !line.starts_with('/') && crate::intent_cmd::enabled(ctx) {
            let model = Some((s.agent.provider.clone(), s.agent.model.clone()));
            let r = crate::intent_cmd::route_tui(ctx, &line, model).await;
            screen.lines.extend(r.lines);
            if let Some(cmd) = r.run {
                queued = Some(cmd);
                continue;
            }
            if let Some(p) = r.prefill {
                screen.input.clone_from(&p);
                offer = Some((line, r.offered, p, r.then));
                continue;
            }
        }
        let text = if line.starts_with('/') {
            // Command output and prompts (e.g. `/auth login`) need the normal terminal: leave the
            // full screen, run the command, wait for Enter, come back.
            leave_screen(&t);
            let out = slash(s, &line, ctx).await;
            let quit = matches!(out, Ok(Slash::Quit));
            let custom = match &out {
                Ok(Slash::Custom(name, args)) => expand_custom(s, name, args, ctx, style, false).await,
                Ok(Slash::Prompt(text)) => Some(text.clone()),
                _ => None,
            };
            if !quit && custom.is_none() {
                println!("\n{}", l.t("tui-press-enter"));
                let _ = std::io::stdin().read_line(&mut String::new());
            }
            enter_screen(&t)?;
            // Inline: a fresh viewport below the command's output; alternate screen: redraw.
            if t.alt_screen {
                terminal.clear()?;
            } else {
                terminal = new_terminal(&t)?;
            }
            if quit {
                break;
            }
            out?;
            screen.mode = s.agent.permissions.mode.as_str().into();
            screen.model.clone_from(&s.agent.model);
            tui_sync(&sources(s), ctx, &mut screen, false).await;
            match custom {
                Some(t) => t,
                None => continue,
            }
        } else {
            line
        };
        let (quit, next) = tui_turn(s, ctx, text, &mut screen, &mut terminal, busy).await?;
        if quit {
            break;
        }
        queued = next;
    }
    Ok(true)
}

async fn dispatch_line(
    s: &mut Session,
    line: &str,
    ctx: &Ctx,
    style: &Style,
    busy: &Busy,
) -> anyhow::Result<bool> {
    if line.starts_with('/') {
        match slash(s, line, ctx).await? {
            Slash::Quit => return Ok(true),
            Slash::Done => {}
            Slash::Custom(name, args) => {
                if let Some(text) = expand_custom(s, &name, &args, ctx, style, false).await {
                    turn(s, text, ctx, style, false, busy).await;
                }
            }
            Slash::Prompt(text) => {
                turn(s, text, ctx, style, false, busy).await;
            }
        }
    } else if crate::intent_cmd::enabled(ctx) {
        // §30: free text that names a Kor command goes there (read-only at once, the rest confirmed).
        let model = Some((s.agent.provider.clone(), s.agent.model.clone()));
        let actions = crate::intent_cmd::route_line(ctx, line, model, true).await;
        return run_intent(s, actions, ctx, style, busy).await.map(|(quit, _)| quit);
    } else {
        turn(s, line.to_owned(), ctx, style, false, busy).await;
    }
    Ok(false)
}

/// Run what the intent layer decided: `/command`s through the slash path, the rest as turns.
/// Returns (quit, every step succeeded).
async fn run_intent(
    s: &mut Session,
    actions: Vec<crate::intent_cmd::Action>,
    ctx: &Ctx,
    style: &Style,
    busy: &Busy,
) -> anyhow::Result<(bool, bool)> {
    let mut ok = true;
    for a in actions {
        let text = match a {
            crate::intent_cmd::Action::Agent(text) => Some(text),
            crate::intent_cmd::Action::Slash(cmd) => match slash(s, &cmd, ctx).await? {
                Slash::Quit => return Ok((true, ok)),
                Slash::Done => None,
                Slash::Custom(name, args) => expand_custom(s, &name, &args, ctx, style, false).await,
                Slash::Prompt(text) => Some(text),
            },
        };
        if let Some(t) = text {
            ok &= turn(s, t, ctx, style, false, busy).await;
        }
    }
    Ok((false, ok))
}

