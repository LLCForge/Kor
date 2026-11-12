//! §16.4 / §17.2 E2E matrix, CLI and TUI surfaces (ADR 0061).
//!
//! Every scenario in `tests/e2e/scenarios/*.yaml` runs against the real `kor` binary and the real core,
//! with the deterministic mock provider (`kor_runtime::e2e::MockServer`) in an isolated Kor home:
//! - CLI: the line REPL in a pseudo-terminal (`expectrl`; `ConPTY` on Windows), answering approvals like a
//!   user, and the parity CLI command for `command` steps;
//! - TUI: `kor tui` in a pseudo-terminal, keys for prompt / approval, the parity slash command for
//!   `command` steps.
//!
//! Both surfaces write the IPC trace; `kor_runtime::e2e::check` compares the core event stream, files,
//! cost ledger and output with the scenario, and `compare` requires CLI and TUI to agree. The desktop
//! runs the same scenarios through `frontend/e2e/desktop.e2e.ts` and the same checker.
//!

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use expectrl::{Eof, Regex, Session};
use kor_runtime::e2e::{self, IsolatedEnv, MockServer, Outcome, Scenario, Step};
use serde_json::Value;

const KOR: &str = env!("CARGO_BIN_EXE_kor");
const TIMEOUT: Duration = Duration::from_secs(120);

fn scenarios() -> Vec<Scenario> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/e2e/scenarios");
    Scenario::load_dir(&dir).expect("scenarios load")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

#[allow(clippy::struct_field_names)] // the scenario's own field names
struct Env {
    env: IsolatedEnv,
    mock: MockServer,
    _dir: tempfile::TempDir,
}

fn setup(rt: &tokio::runtime::Runtime, s: &Scenario, surface: &str) -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let mock = rt
        .block_on(MockServer::start(s.mock.clone()))
        .expect("mock provider");
    let env = e2e::isolated_env_with(dir.path(), s, &mock.url, surface, Some(Path::new(KOR)))
        .expect("isolated env");
    Env {
        env,
        mock,
        _dir: dir,
    }
}

fn command(env: &IsolatedEnv) -> Command {
    let mut c = Command::new(KOR);
    c.current_dir(&env.project)
        .envs(&env.vars)
        .env("NO_COLOR", "1")
        .env("KOR_LANG", "en")
        .env("KOR_UI", "line")
        .env_remove("COLORTERM");
    c
}

fn kor_args(cli: &str, args: &[String]) -> Vec<String> {
    let mut v: Vec<String> = cli.split_whitespace().skip(1).map(str::to_owned).collect();
    v.extend(args.iter().map(|a| a.replace(e2e::KOR_PLACEHOLDER, KOR)));
    v
}

/// A parity command run as its own process; its output is part of the surface's output.
fn run_command(env: &IsolatedEnv, st: &Step) -> String {
    let cli = st.command.as_deref().unwrap_or_default();
    let out = command(env)
        .arg("--lang")
        .arg("en")
        .args(kor_args(cli, &st.args))
        .output()
        .expect("run kor");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Some Windows builds' `ConPTY` never hands a child's output to `expectrl` (not even
/// `cmd /c echo`); the PTY tests cannot say anything there, so they are skipped with the reason.
fn pty_works() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        let windows = std::env::consts::OS == "windows";
        let mut c = if windows {
            Command::new("cmd")
        } else {
            Command::new("sh")
        };
        if windows {
            c.args(["/c", "echo kor-pty-probe"]);
        } else {
            c.args(["-c", "echo kor-pty-probe"]);
        }
        let Ok(mut p) = Session::spawn(c) else {
            return false;
        };
        p.set_expect_timeout(Some(Duration::from_secs(10)));
        p.expect("kor-pty-probe").is_ok()
    })
}

fn skip_without_pty(test: &str) -> bool {
    if pty_works() {
        return false;
    }
    eprintln!(
        "{test}: skipped, this machine's pseudo-terminal does not pass a child's output through"
    );
    true
}

fn pty(cmd: Command) -> Session {
    let mut p = Session::spawn(cmd).expect("pseudo-terminal (ConPTY on Windows)");
    p.set_expect_timeout(Some(TIMEOUT));
    p
}

fn text(captures: &expectrl::Captures) -> String {
    String::from_utf8_lossy(captures.before()).into_owned()
        + &String::from_utf8_lossy(captures.as_bytes())
}

/// Consecutive prompt / approve steps: one REPL session in a PTY, answered like a user.
fn cli_session(env: &IsolatedEnv, steps: &[&Step], out: &mut String) {
    let mut c = command(env);
    c.arg("--lang").arg("en");
    let mut p = pty(c);
    // The hint line ends the banner; the `✶ ` prompt follows it.
    out.push_str(&text(&p.expect("/exit quits").expect("REPL start")));
    out.push_str(&text(&p.expect("✶ ").expect("first prompt")));
    for st in steps {
        if let Some(prompt) = &st.prompt {
            p.send_line(prompt).expect("send prompt");
            // The turn ends at the next prompt, or stops at an approval card first.
            let next = steps.iter().skip_while(|s| !std::ptr::eq(**s, *st)).nth(1);
            if next.is_some_and(|n| n.approve.is_some()) {
                out.push_str(&text(
                    &p.expect(Regex(r"\[n\] (no|reject all)"))
                        .expect("approval card"),
                ));
            } else {
                out.push_str(&text(&p.expect("✶ ").expect("turn end")));
            }
        } else if let Some(a) = &st.approve {
            let key = match a.as_str() {
                "always" => "a",
                "no" => "n",
                _ => "y",
            };
            p.send_line(key).expect("send approval");
            out.push_str(&text(&p.expect("✶ ").expect("turn end after approval")));
        }
    }
    p.send_line("/exit").expect("exit");
    out.push_str(&text(&p.expect(Eof).expect("REPL exits")));
}

fn run_cli(s: &Scenario, rt: &tokio::runtime::Runtime) -> Outcome {
    let e = setup(rt, s, "cli");
    let mut out = String::new();
    let mut group: Vec<&Step> = Vec::new();
    for st in &s.steps {
        if st.command.is_some() {
            if !group.is_empty() {
                cli_session(&e.env, &std::mem::take(&mut group), &mut out);
            }
            out.push_str(&run_command(&e.env, st));
        } else {
            group.push(st);
        }
    }
    if !group.is_empty() {
        cli_session(&e.env, &group, &mut out);
    }
    e2e::check_with(s, &e.env, "cli", &out, &e.mock.requests())
}

/// The parity manifest's TUI slash command for a CLI command (`kor undo` → `/undo`).
fn tui_command(cli: &str) -> String {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../parity.toml"))
            .expect("parity.toml");
    let doc: toml::Table = text.parse().expect("parity.toml parses");
    doc["feature"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f.get("cli").and_then(toml::Value::as_str) == Some(cli))
        .and_then(|f| f.get("tui").and_then(toml::Value::as_str))
        .filter(|t| t.starts_with('/'))
        .unwrap_or_else(|| panic!("parity.toml has no TUI slash command for `{cli}`"))
        .to_owned()
}

fn run_tui(s: &Scenario, rt: &tokio::runtime::Runtime) -> Outcome {
    let e = setup(rt, s, "tui");
    let mut c = command(&e.env);
    c.arg("--lang").arg("en").arg("tui").env_remove("KOR_UI");
    let mut p = pty(c);
    // A TUI that exits before it is ready leaves its last screen (an error, a panic) in the
    // failure message instead of a bare `Eof`.
    let first = p
        .expect(expectrl::Any::boxed(vec![
            Box::new("Idle"),
            Box::new(expectrl::Eof),
        ]))
        .expect("TUI start");
    let mut out = text(&first);
    assert!(
        out.contains("Idle"),
        "the TUI exited before it was ready:
{out}"
    );
    for (i, st) in s.steps.iter().enumerate() {
        let next_is_approval = s.steps.get(i + 1).is_some_and(|n| n.approve.is_some());
        if let Some(prompt) = &st.prompt {
            p.send(prompt).expect("type prompt");
            p.send("\r").expect("submit");
        } else if let Some(a) = &st.approve {
            p.send(match a.as_str() {
                "always" => "A",
                "no" => "d",
                _ => "a",
            })
            .expect("answer");
        } else if let Some(cli) = &st.command {
            let mut line = tui_command(cli);
            for a in &st.args {
                line.push(' ');
                line.push_str(&a.replace(e2e::KOR_PLACEHOLDER, KOR));
            }
            p.send(&line).expect("type command");
            p.send("\r").expect("submit");
        }
        let until = if next_is_approval {
            Regex(r"\[d\] deny")
        } else {
            Regex("Idle")
        };
        match p.expect(until) {
            Ok(found) => out.push_str(&text(&found)),
            // The screen so far, so a CI failure shows where the TUI stopped.
            Err(e) => panic!(
                "TUI step {i} ({st:?}): {e:?}\n--- screen so far ---\n{}",
                strip_ansi(&out)
            ),
        }
    }
    p.send("\x1b").expect("Esc quits");
    out.push_str(&text(&p.expect(Eof).expect("TUI exits")));
    e2e::check_with(s, &e.env, "tui", &strip_ansi(&out), &e.mock.requests())
}

/// Removes CSI / OSC escape sequences (the TUI draws with cursor moves between runs of text).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for d in chars.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
                // A cursor move separates words on screen.
                out.push(' ');
            }
            Some(']') => {
                while let Some(d) = chars.next() {
                    if d == '\x07' || (d == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn report(all: &[(String, Vec<Outcome>)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, outs) in all {
        for o in outs {
            for p in &o.problems {
                problems.push(format!("{name} [{}]: {p}", o.surface));
            }
        }
        for d in e2e::compare(outs) {
            problems.push(format!("{name}: surfaces differ: {d}"));
        }
    }
    problems
}

#[test]
fn scenarios_on_cli_and_tui_match_the_catalog_and_each_other() {
    if skip_without_pty("scenarios_on_cli_and_tui_match_the_catalog_and_each_other") {
        return;
    }
    let rt = runtime();
    let mut all = Vec::new();
    for s in scenarios() {
        let mut outs = Vec::new();
        if s.on("cli") {
            outs.push(run_cli(&s, &rt));
        }
        if s.on("tui") {
            outs.push(run_tui(&s, &rt));
        }
        all.push((s.name.clone(), outs));
    }
    let problems = report(&all);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn pty_repl_shows_banner_and_quits() {
    if skip_without_pty("pty_repl_shows_banner_and_quits") {
        return;
    }
    let s = Scenario::parse("name: banner\ntitle: banner\n").expect("scenario");
    let rt = runtime();
    let e = setup(&rt, &s, "cli");
    let mut c = command(&e.env);
    c.arg("--lang").arg("en");
    let mut p = pty(c);
    // The banner is drawn from assets/banner*.txt; the narrow one below 60 columns.
    let start = text(&p.expect("/exit quits").expect("banner and hint"));
    assert!(start.contains("██") || start.contains("▄"), "{start}");
    p.send_line("/exit").expect("exit");
    p.expect(Eof).expect("exits");
}

/// `kor -p --json`: the desktop wire shape, one JSON object per line (§28.1), snapshot (§16.4).
#[test]
fn print_json_output_snapshot() {
    let s = scenarios()
        .into_iter()
        .find(|s| s.name == "cost-update")
        .expect("cost-update scenario");
    let rt = runtime();
    let e = setup(&rt, &s, "cli");
    let out = command(&e.env)
        .args(["--lang", "en", "-p", "--json", "merhaba"])
        .output()
        .expect("kor -p --json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect();
    // Status ticks depend on timing: the first one stays (it is sent at the start).
    let mut seen = false;
    lines.retain(|v| v["type"] != "status" || !std::mem::replace(&mut seen, true));
    // The turn summary's wall time ("Took 0s" or "Took 1s") depends on the machine.
    for v in &mut lines {
        if let Some(t) = v["text"].as_str().filter(|t| t.starts_with("Took ")) {
            let rest = t.split_once(" · ").map_or("", |(_, r)| r);
            v["text"] = Value::from(format!("Took [time] · {rest}"));
        }
    }
    insta::assert_json_snapshot!("print_json_lines", lines, {
        "[].elapsedMs" => "[ms]",
        "[].tokens" => "[tokens]",
        "[].verb" => "[verb]",
        "[].verbIndex" => "[index]",
        "[].breakdown" => "[breakdown]",
        "[].total" => "[total]",
        "[].percent" => "[percent]",
    });
    // Every line is valid against the IPC schema (`kor:event` items) where it is one.
    for v in lines
        .iter()
        .filter(|v| !matches!(v["type"].as_str(), Some("usage" | "context.usage")))
    {
        let mut item = v.clone();
        item["sessionId"] = Value::from("cli");
        let errors = kor_core::ipc::check_event("kor:event", &item);
        assert!(errors.is_empty(), "{v}: {errors:?}");
    }
}

/// `--json` of a listing command, in an isolated home with one finished session.
#[test]
fn sessions_json_snapshot() {
    let s = scenarios()
        .into_iter()
        .find(|s| s.name == "cost-update")
        .expect("cost-update scenario");
    let rt = runtime();
    let e = setup(&rt, &s, "cli");
    let run = command(&e.env)
        .args(["--lang", "en", "-p", "merhaba"])
        .output()
        .expect("kor -p");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let out = command(&e.env)
        .args(["--lang", "en", "sessions", "--json"])
        .output()
        .expect("kor sessions --json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    insta::assert_json_snapshot!("sessions_json", v, {
        "[].id" => "[id]",
        "[].path" => "[path]",
        "[].root" => "[root]",
        "[].modified" => "[time]",
        "[].created" => "[time]",
        "[].updated" => "[time]",
        "[].mtime" => "[time]",
        "[].size" => "[size]",
    });
}
