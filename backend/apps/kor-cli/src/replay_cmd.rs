//! `kor replay [<id>] | list | export <id> [--out FILE]` (§15.9, ADR 0043): play a recorded
//! session in the terminal with a timeline (pause, step forward/back, the diff at that point),
//! list recordings, write one shareable file. Logic: `kor_runtime::replay`.

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::{cursor, execute, terminal};
use kor_i18n::{Localizer, args};
use kor_runtime::replay::{self, Recording, Timeline};

use crate::Ctx;

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ReplayArgs {
    #[command(subcommand)]
    pub command: Option<ReplaySub>,
    /// Recording id, or an exported file.
    pub id: Option<String>,
    /// Playback speed (2 = twice as fast).
    #[arg(long, default_value_t = 1.0)]
    pub speed: f64,
    /// Print the timeline instead of the interactive player.
    #[arg(long)]
    pub print: bool,
}

#[derive(Debug, Subcommand)]
pub enum ReplaySub {
    /// List the recorded sessions of this project.
    List,
    /// Write one shareable file (cast and events together).
    Export {
        id: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-replay"))
        .mut_arg("id", |a| a.help(l.t("arg-replay-id")))
        .mut_arg("speed", |a| a.help(l.t("arg-replay-speed")))
        .mut_arg("print", |a| a.help(l.t("arg-replay-print")))
        .mut_subcommand("list", |s| s.about(l.t("cmd-replay-list")))
        .mut_subcommand("export", |s| {
            s.about(l.t("cmd-replay-export"))
                .mut_arg("id", |a| a.help(l.t("arg-replay-id")))
                .mut_arg("out", |a| a.help(l.t("arg-replay-out")))
        })
}

pub fn run(ctx: &Ctx, a: &ReplayArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    match (&a.command, &a.id) {
        (Some(ReplaySub::Export { id, out }), _) => {
            let path = replay::export(&root, id, out.as_deref()).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                l.t_args(
                    "replay-exported",
                    &args!("path" => path.display().to_string())
                )
            );
            Ok(true)
        }
        (Some(ReplaySub::List), _) | (None, None) => {
            list(l, &root);
            Ok(true)
        }
        (None, Some(id)) => {
            let rec = replay::load(&root, id).map_err(anyhow::Error::msg)?;
            if rec.events.is_empty() {
                println!("{}", l.t("replay-empty"));
                return Ok(false);
            }
            if a.print || !std::io::stdout().is_terminal() {
                for s in &Timeline::new(&rec.events).steps {
                    println!("{:>8.1}  {}", s.t, s.title);
                }
                return Ok(true);
            }
            play(l, &rec, a.speed)?;
            Ok(true)
        }
    }
}

fn list(l: &Localizer, root: &std::path::Path) {
    let all = replay::list(root);
    if all.is_empty() {
        println!("{}", l.t("replay-list-empty"));
        return;
    }
    println!("{}", l.t("replay-list-head"));
    for s in all {
        let cost = if s.cost_estimated { "≈$" } else { "$" };
        println!(
            "{} | {:.0}s | {} | {} | {cost}{:.4} | {}",
            s.id, s.duration, s.events, s.tools, s.cost_usd, s.title
        );
    }
}

/// Restores the terminal however the player ends.
struct Raw;

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            terminal::LeaveAlternateScreen,
            cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// The frame: header, the steps around the current one, the diff (when shown), the keys.
fn frame(l: &Localizer, rec: &Recording, tl: &Timeline, playing: bool, diff: bool) -> String {
    let (cols, rows_total) = terminal::size().unwrap_or((100, 30));
    let (width, height) = (usize::from(cols).max(20), usize::from(rows_total).max(10));
    let fit = |s: &str| s.chars().take(width - 1).collect::<String>();
    let state = l.t(if playing {
        "replay-playing"
    } else {
        "replay-paused"
    });
    let now = tl.current().map_or(0.0, |s| s.t);
    let mut out = vec![
        fit(&l.t_args(
            "replay-head",
            &args!("id" => rec.id.clone(), "pos" => tl.pos() + 1, "len" => tl.len(), "time" => format!("{now:.1}"), "state" => state),
        )),
        String::new(),
    ];
    let mut body = Vec::new();
    if diff {
        match tl.diff_at().map(|i| &rec.events[i]) {
            None => body.push(l.t("replay-no-diff")),
            Some(e) => {
                body.push(l.t_args(
                    "replay-diff-head",
                    &args!("tool" => e["tool"].as_str().unwrap_or_default().to_owned()),
                ));
                for f in e["files"].as_array().into_iter().flatten() {
                    let path = f["path"].as_str().unwrap_or_default();
                    let text = kor_agent::port::unified_diff(
                        path,
                        f["before"].as_str().unwrap_or_default(),
                        f["after"].as_str().unwrap_or_default(),
                    );
                    body.extend(text.lines().map(fit));
                }
            }
        }
    } else {
        let rows = height.saturating_sub(4);
        let start = (tl.pos() + 1).saturating_sub(rows);
        for (i, s) in tl.steps.iter().enumerate().skip(start).take(rows) {
            let mark = if i == tl.pos() { '▶' } else { ' ' };
            body.push(fit(&format!("{mark}{:>7.1}  {}", s.t, s.title)));
        }
    }
    body.truncate(height.saturating_sub(4));
    out.extend(body);
    while out.len() < height - 1 {
        out.push(String::new());
    }
    out.push(fit(&l.t("replay-keys")));
    out.join("\r\n")
}

fn play(l: &Localizer, rec: &Recording, speed: f64) -> anyhow::Result<()> {
    let mut tl = Timeline::new(&rec.events);
    let mut out = std::io::stdout();
    terminal::enable_raw_mode()?;
    let _raw = Raw;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
    let (mut playing, mut diff) = (true, false);
    let mut due = Instant::now() + Duration::from_secs_f64(tl.delay(speed));
    loop {
        execute!(
            out,
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        write!(out, "{}", frame(l, rec, &tl, playing, diff))?;
        out.flush()?;
        let wait = if playing {
            due.saturating_duration_since(Instant::now())
        } else {
            Duration::from_millis(500)
        };
        if event::poll(wait)? {
            let Event::Key(k) = event::read()? else {
                continue;
            };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char(' ') => playing = !playing && !tl.at_end(),
                KeyCode::Right | KeyCode::Char('n' | 'l') => {
                    playing = false;
                    tl.forward();
                }
                KeyCode::Left | KeyCode::Char('p' | 'h') => {
                    playing = false;
                    tl.back();
                }
                KeyCode::Char('d') => diff = !diff,
                KeyCode::Char('g') | KeyCode::Home => tl.go(0),
                KeyCode::Char('G') | KeyCode::End => tl.go(usize::MAX),
                _ => {}
            }
            due = Instant::now() + Duration::from_secs_f64(tl.delay(speed));
        } else if playing {
            if !tl.forward() || tl.at_end() {
                playing = false;
            }
            due = Instant::now() + Duration::from_secs_f64(tl.delay(speed));
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Subcommand)]
    enum Top {
        Replay(ReplayArgs),
    }

    fn parse(v: &[&str]) -> ReplayArgs {
        let Top::Replay(a) = Cli::try_parse_from(std::iter::once("kor").chain(v.iter().copied()))
            .unwrap()
            .cmd;
        a
    }

    #[test]
    fn parses_play_list_and_export() {
        let a = parse(&["replay", "abc", "--speed", "2", "--print"]);
        assert_eq!(a.id.as_deref(), Some("abc"));
        assert!(a.print && (a.speed - 2.0).abs() < f64::EPSILON);
        assert!(matches!(
            parse(&["replay", "list"]).command,
            Some(ReplaySub::List)
        ));
        assert!(parse(&["replay"]).command.is_none());
        let a = parse(&["replay", "export", "abc", "--out", "x.json"]);
        assert!(
            matches!(a.command, Some(ReplaySub::Export { ref id, ref out }) if id == "abc" && out.as_deref() == Some(std::path::Path::new("x.json")))
        );
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let _ = localize(
            <ReplayArgs as Args>::augment_args(clap::Command::new("replay")),
            &l,
        );
    }
}
