//! `kor status [--watch]`, `kor approve|deny <id>`, `kor approvals` (§27.1, §27.8, §28.4, §28.5,
//! ADR 0051). What runs on this machine — desktop chats, daemon jobs, remote tasks — and one
//! approval queue over all of them. Logic in `kor_runtime::presence`; this file prints.

use std::io::{IsTerminal as _, Write as _};

use kor_i18n::{Localizer, args};
use kor_runtime::presence::attention::{self, Policy};
use kor_runtime::presence::desktop::Source;
use kor_runtime::presence::eta::{Eta, Scope};
use kor_runtime::presence::notifications::Category;
use kor_runtime::presence::overview::{self, Overview};
use kor_runtime::presence::queue::{self, Item, Risk};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct StatusArgs {
    /// Keep a compact view open, refreshed every second (Ctrl+C to leave).
    #[arg(long)]
    pub watch: bool,
    /// Machine-readable output.
    #[arg(long, conflicts_with = "watch")]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct AnswerArgs {
    /// Approval id (from `kor status`, `kor approvals` or a notification).
    pub id: String,
    /// Also answer every similar pending request (same tool and change).
    #[arg(long)]
    pub similar: bool,
}

#[derive(Debug, clap::Args)]
pub struct ApprovalsArgs {
    /// Print the queue and exit (no interactive view).
    #[arg(long)]
    pub list: bool,
    #[arg(long)]
    pub json: bool,
}

pub fn localize_status(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-status"))
}

pub fn localize_approve(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-approve"))
}

pub fn localize_deny(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-deny"))
}

pub fn localize_approvals(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-approvals"))
}

fn policy(ctx: &Ctx) -> Policy {
    ctx.config
        .as_ref()
        .map(|c| Policy::from(&c.config.notify))
        .unwrap_or_default()
}

fn hour() -> u8 {
    use chrono::Timelike as _;
    u8::try_from(chrono::Local::now().hour()).unwrap_or(0)
}

/// §28.4 "call me" for the terminal: `BEL` and the window title while the CLI/TUI waits for an
/// answer (`notify.bell`, quiet hours, muted categories respected); also listed in the
/// notification center so the desktop bell and `kor notifications` show it.
pub fn call_me(ctx: &Ctx, tool: &str, summary: &str) {
    let l = &ctx.l;
    let p = policy(ctx);
    let mut err = std::io::stderr();
    if err.is_terminal()
        && let Some(seq) = attention::terminal_call(
            &p,
            Category::Approval,
            hour(),
            &l.t("presence-waiting-title"),
        )
    {
        let _ = err.write_all(seq.as_bytes());
        let _ = err.flush();
    }
    // The same sound file as the desktop (§28.4).
    if attention::terminal_sound(&p, Category::Approval, hour()) {
        kor_platform::sound::play("soft");
    }
    if !p.muted.contains(&Category::Approval)
        && let Some(s) = kor_runtime::presence::notifications::Store::standard()
    {
        let _ = s.push(kor_runtime::presence::notifications::Notification {
            category: Category::Approval,
            title: l.t("presence-waiting-title"),
            body: format!("{tool}: {summary}"),
            ..Default::default()
        });
    }
}

/// The terminal title back to plain "Kor" once answered.
pub fn call_me_done() {
    let mut err = std::io::stderr();
    if err.is_terminal() {
        let _ = err.write_all(b"\x1b]0;Kor\x07");
        let _ = err.flush();
    }
}

fn clock(ms: u64) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

fn elapsed(since: u64, now: u64) -> String {
    let s = now.saturating_sub(since) / 1000;
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

/// `≈ 14:32 (p50 of 12 turns in this project)` / `taking longer than usual`.
pub fn eta_line(l: &Localizer, e: &Eta) -> String {
    if e.overdue {
        return l.t("status-eta-overdue");
    }
    let method = l.t_args(
        "status-eta-method",
        &args!(
            "p" => i64::from(e.percentile),
            "n" => i64::try_from(e.samples).unwrap_or(i64::MAX),
            "scope" => match e.scope { Scope::Project => "project", Scope::All => "all" }
        ),
    );
    l.t_args(
        "status-eta",
        &args!("time" => clock(e.finish_ms.unwrap_or_default()), "method" => method),
    )
}

fn risk_label(l: &Localizer, r: Risk) -> String {
    l.t(match r {
        Risk::Low => "risk-low",
        Risk::Medium => "risk-medium",
        Risk::High => "risk-high",
    })
}

fn source_label(s: Source) -> &'static str {
    s.key()
}

fn print_item(l: &Localizer, it: &Item, now: u64) {
    let a = &it.approval;
    println!(
        "  {}  [{}] {}  {}  {}",
        a.id,
        source_label(a.source),
        risk_label(l, it.risk),
        a.tool,
        elapsed(a.since, now)
    );
    if !a.title.is_empty() {
        println!("      {}", a.title);
    }
    println!("      {}", a.summary.lines().next().unwrap_or_default());
}

fn print_overview(l: &Localizer, o: &Overview) {
    println!("{}", l.t("status-desktop"));
    if !o.desktop {
        println!("  {}", l.t("status-desktop-off"));
    } else if o.sessions.is_empty() {
        println!("  {}", l.t("status-no-tasks"));
    }
    for s in &o.sessions {
        let cost = kor_runtime::cost::fmt_usd(s.cost_usd, s.cost_estimated);
        let state = if s.waiting {
            l.t("status-waiting")
        } else {
            s.verb.clone()
        };
        println!(
            "  {}  {}  {}  {}  {}",
            s.id,
            if s.title.is_empty() {
                &s.root
            } else {
                &s.title
            },
            state,
            elapsed(s.since, o.now),
            cost
        );
        if let Some([d, t]) = s.progress {
            println!("      {d}/{t}");
        }
        if let Some(e) = &s.eta {
            println!("      {}", eta_line(l, e));
        }
    }
    println!();
    let state = l.t(if o.daemon.running {
        "status-daemon-on"
    } else {
        "status-daemon-off"
    });
    println!(
        "{}",
        l.t_args(
            "status-daemon",
            &args!("state" => state, "queued" => i64::try_from(o.daemon.queued).unwrap_or(0))
        )
    );
    for j in &o.daemon.jobs {
        println!(
            "  {}  {}  {}  {}  {}",
            j.id,
            j.trigger,
            j.status,
            j.started.map(|s| elapsed(s, o.now)).unwrap_or_default(),
            kor_runtime::cost::fmt_usd(j.cost_usd, j.cost_estimated)
        );
        if let Some(e) = &j.eta {
            println!("      {}", eta_line(l, e));
        }
    }
    if !o.remote.is_empty() {
        println!();
        println!("{}", l.t("status-remote"));
        for t in &o.remote {
            println!(
                "  {}  {}  {}  {}",
                t.id,
                t.state,
                elapsed(t.started, o.now),
                t.prompt.lines().next().unwrap_or_default()
            );
        }
    }
    println!();
    println!(
        "{}",
        l.t_args(
            "status-approvals",
            &args!("n" => i64::try_from(o.approvals.len()).unwrap_or(0))
        )
    );
    for it in &o.approvals {
        print_item(l, it, o.now);
    }
    for g in &o.groups {
        println!(
            "  {}",
            l.t_args(
                "approve-group",
                &args!("n" => i64::try_from(g.ids.len()).unwrap_or(0), "summary" => g.summary.clone())
            )
        );
    }
}

/// One-line-per-task compact view (`--watch`, the TUI counterpart of the status window).
fn compact(l: &Localizer, o: &Overview) -> Vec<String> {
    let mut out = Vec::new();
    for s in &o.sessions {
        let mut line = format!(
            "{} {} · {} · {}",
            if s.waiting { "!" } else { "*" },
            if s.waiting {
                l.t("status-waiting")
            } else {
                s.verb.clone()
            },
            elapsed(s.since, o.now),
            kor_runtime::cost::fmt_usd(s.cost_usd, s.cost_estimated)
        );
        if let Some(e) = s.eta.as_ref().filter(|e| !e.overdue) {
            line.push_str(" · ≈");
            line.push_str(&clock(e.finish_ms.unwrap_or_default()));
        }
        line.push_str("  ");
        line.push_str(if s.title.is_empty() {
            &s.root
        } else {
            &s.title
        });
        out.push(line);
    }
    for j in &o.daemon.jobs {
        out.push(format!(
            "* {} · {} · {}",
            j.trigger,
            j.status,
            j.started.map(|s| elapsed(s, o.now)).unwrap_or_default()
        ));
    }
    for t in &o.remote {
        out.push(format!(
            "* {} · {} · {}",
            t.id,
            t.state,
            elapsed(t.started, o.now)
        ));
    }
    if out.is_empty() {
        out.push(l.t("status-no-tasks"));
    }
    if !o.approvals.is_empty() {
        out.push(l.t_args(
            "status-approvals",
            &args!("n" => i64::try_from(o.approvals.len()).unwrap_or(0)),
        ));
    }
    out
}

pub fn status(ctx: &Ctx, a: &StatusArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let rt = crate::providers::runtime()?;
    if a.json {
        let o = rt.block_on(overview::collect());
        println!("{}", serde_json::to_string_pretty(&o)?);
        return Ok(true);
    }
    if !a.watch {
        print_overview(l, &rt.block_on(overview::collect()));
        return Ok(true);
    }
    let tty = std::io::stdout().is_terminal();
    loop {
        let lines = compact(l, &rt.block_on(overview::collect()));
        let mut out = std::io::stdout().lock();
        if tty {
            let _ = out.write_all(b"\x1b[2J\x1b[H");
        }
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
        let _ = out.flush();
        drop(out);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn answer_one(ctx: &Ctx, rt: &tokio::runtime::Runtime, id: &str, approve: bool) -> bool {
    let l = &ctx.l;
    match rt.block_on(queue::answer(id, approve)) {
        Ok(src) => {
            println!(
                "{}",
                l.t_args(
                    if approve { "approve-sent" } else { "deny-sent" },
                    &args!("id" => id.to_owned(), "source" => src.key())
                )
            );
            true
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args(
                    "approve-failed",
                    &args!("id" => id.to_owned(), "error" => e)
                )
            );
            false
        }
    }
}

/// `kor approve <id>` / `kor deny <id>`.
pub fn answer(ctx: &Ctx, a: &AnswerArgs, approve: bool) -> anyhow::Result<bool> {
    let rt = crate::providers::runtime()?;
    let mut ids = vec![a.id.clone()];
    if a.similar {
        let items = rt.block_on(queue::collect());
        if let Some(group) = items
            .iter()
            .find(|i| i.approval.id == a.id)
            .map(|i| i.group.clone())
        {
            ids = items
                .iter()
                .filter(|i| i.group == group)
                .map(|i| i.approval.id.clone())
                .collect();
        }
    }
    let mut ok = true;
    for id in &ids {
        ok &= answer_one(ctx, &rt, id, approve);
    }
    Ok(ok)
}

/// Key in the interactive queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Approve,
    Deny,
    Next,
    Prev,
    Details,
    ApproveSimilar,
    Quit,
}

fn key_of(code: crossterm::event::KeyCode) -> Option<Key> {
    use crossterm::event::KeyCode as K;
    Some(match code {
        K::Enter | K::Right | K::Char('y' | 'Y') => Key::Approve,
        K::Left | K::Char('n' | 'N') => Key::Deny,
        K::Down | K::Char('j' | 'J') => Key::Next,
        K::Up | K::Char('k' | 'K') => Key::Prev,
        K::Char('d' | 'D') => Key::Details,
        K::Char('a' | 'A') => Key::ApproveSimilar,
        K::Esc | K::Char('q' | 'Q') => Key::Quit,
        _ => return None,
    })
}

fn read_key() -> anyhow::Result<Key> {
    crossterm::terminal::enable_raw_mode()?;
    let k = loop {
        match crossterm::event::read() {
            Ok(crossterm::event::Event::Key(e))
                if e.kind == crossterm::event::KeyEventKind::Press =>
            {
                if let Some(k) = key_of(e.code) {
                    break Ok(k);
                }
            }
            Ok(_) => {}
            Err(e) => break Err(e),
        }
    };
    // Raw mode is left even when reading failed.
    crossterm::terminal::disable_raw_mode()?;
    Ok(k?)
}

/// `kor approvals`: the queue; on a terminal one card at a time with the same keys as the desktop.
pub fn approvals(ctx: &Ctx, a: &ApprovalsArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let rt = crate::providers::runtime()?;
    let mut items = rt.block_on(queue::collect());
    if a.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "items": items, "groups": queue::groups(&items) })
            )?
        );
        return Ok(true);
    }
    if items.is_empty() {
        println!("{}", l.t("approve-none"));
        return Ok(true);
    }
    let now = kor_runtime::cost::now_ms();
    if a.list || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        for it in &items {
            print_item(l, it, now);
        }
        for g in queue::groups(&items) {
            println!(
                "  {}",
                l.t_args(
                    "approve-group",
                    &args!("n" => i64::try_from(g.ids.len()).unwrap_or(0), "summary" => g.summary)
                )
            );
        }
        return Ok(true);
    }
    let mut i = 0usize;
    let mut ok = true;
    while !items.is_empty() {
        i = i.min(items.len() - 1);
        let it = items[i].clone();
        println!();
        println!("({}/{})", i + 1, items.len());
        print_item(l, &it, kor_runtime::cost::now_ms());
        let similar: Vec<String> = items
            .iter()
            .filter(|x| x.group == it.group)
            .map(|x| x.approval.id.clone())
            .collect();
        if similar.len() > 1 {
            println!("  {}", l.t_args("approve-group", &args!("n" => i64::try_from(similar.len()).unwrap_or(0), "summary" => it.approval.summary.clone())));
        }
        println!("{}", l.t("approvals-keys"));
        match read_key()? {
            k @ (Key::Approve | Key::Deny) => {
                ok &= answer_one(ctx, &rt, &it.approval.id, k == Key::Approve);
                items.remove(i);
            }
            Key::ApproveSimilar => {
                for id in &similar {
                    ok &= answer_one(ctx, &rt, id, true);
                }
                items.retain(|x| x.group != it.group);
            }
            Key::Next => i = (i + 1) % items.len(),
            Key::Prev => i = i.checked_sub(1).unwrap_or(items.len() - 1),
            Key::Details => {
                for f in &it.approval.files {
                    println!("      {f}");
                }
                if let Some(d) = &it.approval.destructive {
                    println!("      ! {d}");
                }
                println!("{}", it.approval.summary);
            }
            Key::Quit => break,
        }
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, clap::Parser)]
    enum T {
        Status(StatusArgs),
        Approve(AnswerArgs),
        Deny(AnswerArgs),
        Approvals(ApprovalsArgs),
    }

    #[test]
    fn parses() {
        assert!(matches!(
            T::try_parse_from(["kor", "status", "--watch"]).unwrap(),
            T::Status(StatusArgs {
                watch: true,
                json: false
            })
        ));
        assert!(T::try_parse_from(["kor", "status", "--watch", "--json"]).is_err());
        assert!(matches!(
            T::try_parse_from(["kor", "approve", "toolu_1", "--similar"]).unwrap(),
            T::Approve(AnswerArgs { similar: true, .. })
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "deny", "ab12"]).unwrap(),
            T::Deny(AnswerArgs { similar: false, .. })
        ));
        assert!(T::try_parse_from(["kor", "deny"]).is_err());
        assert!(matches!(
            T::try_parse_from(["kor", "approvals", "--list"]).unwrap(),
            T::Approvals(ApprovalsArgs {
                list: true,
                json: false
            })
        ));
        let l = Localizer::new(kor_i18n::Lang::En);
        let c = <T as clap::CommandFactory>::command();
        let _ = localize_status(c.clone(), &l);
        let _ = localize_approve(c.clone(), &l);
        let _ = localize_deny(c.clone(), &l);
        let _ = localize_approvals(c, &l);
    }

    #[test]
    fn keys_match_the_desktop_queue() {
        use crossterm::event::KeyCode as K;
        assert_eq!(key_of(K::Enter), Some(Key::Approve));
        assert_eq!(key_of(K::Right), Some(Key::Approve));
        assert_eq!(key_of(K::Left), Some(Key::Deny));
        assert_eq!(key_of(K::Char('j')), Some(Key::Next));
        assert_eq!(key_of(K::Char('K')), Some(Key::Prev));
        assert_eq!(key_of(K::Esc), Some(Key::Quit));
        assert_eq!(key_of(K::Char('x')), None);
    }

    #[test]
    fn eta_lines_are_marked_as_estimates() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let e = Eta {
            finish_ms: Some(0),
            remaining_secs: Some(10),
            percentile: 50,
            scope: Scope::Project,
            samples: 12,
            overdue: false,
        };
        assert!(eta_line(&l, &e).contains('≈'));
        let late = Eta { overdue: true, ..e };
        assert!(!eta_line(&l, &late).contains('≈'));
        assert_eq!(elapsed(0, 61_000), "1m01s");
        assert_eq!(elapsed(0, 3_660_000), "1h01m");
    }
}
