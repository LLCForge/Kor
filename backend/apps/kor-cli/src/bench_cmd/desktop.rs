//! `kor bench --ui`, desktop half (ADR 0062): start `kor-desktop` with `KOR_BENCH_PORT`; it
//! connects back over loopback, says `ready` when its window is painted and interactive, and in
//! `full` mode runs the renderer's own measurements (keypress, token render, panel and thread
//! open, scroll frames, file search, idle memory/CPU, soak) and sends them as JSON lines.

use std::io::{BufRead as _, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use kor_i18n::{Localizer, args};
use kor_runtime::budgets::Stat;
use serde::Deserialize;

use super::perf::Results;

/// Launches: the first is the cold start, the rest are warm; the last one runs the full set.
const LAUNCHES: usize = 5;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const FULL_TIMEOUT: Duration = Duration::from_secs(600);

/// The desktop binary: `--desktop`, `KOR_DESKTOP`, or `kor-desktop` next to this `kor`.
pub fn locate(given: Option<&Path>) -> Option<PathBuf> {
    given
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("KOR_DESKTOP").map(PathBuf::from))
        .or_else(|| {
            let me = std::env::current_exe().ok()?;
            Some(me.with_file_name(kor_platform::exe("kor-desktop")))
        })
        .filter(|p| p.is_file())
}

/// One line the desktop sends.
#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Line {
    Ready,
    Results {
        results: std::collections::BTreeMap<String, Stat>,
    },
    Error {
        message: String,
    },
}

/// Parsed lines of one connection, as they arrive.
fn read_lines(listener: &TcpListener) -> mpsc::Receiver<(Instant, Result<Line, String>)> {
    let (tx, rx) = mpsc::sync_channel(64);
    let listener = listener.try_clone();
    std::thread::spawn(move || {
        let Ok(listener) = listener else { return };
        let Ok((sock, _)) = listener.accept() else {
            return;
        };
        for line in BufReader::new(sock).lines() {
            let Ok(line) = line else { break };
            let parsed = serde_json::from_str::<Line>(&line).map_err(|e| e.to_string());
            if tx.send((Instant::now(), parsed)).is_err() {
                break;
            }
        }
    });
    rx
}

fn stop(mut child: Child) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// One launch: startup time, and the results in `full` mode.
fn launch(exe: &Path, home: &Path, full: bool) -> Result<(Duration, Option<Results>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let rx = read_lines(&listener);
    let t0 = Instant::now();
    let child = Command::new(exe)
        .env("KOR_BENCH_PORT", port.to_string())
        .env("KOR_BENCH_MODE", if full { "full" } else { "startup" })
        .env("KOR_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let mut startup = None;
    let mut results = None;
    let deadline = |t: Duration| t0 + t;
    let mut error = None;
    loop {
        let limit = deadline(if full { FULL_TIMEOUT } else { STARTUP_TIMEOUT });
        let wait = limit.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok((at, Ok(Line::Ready))) => {
                startup = Some(at - t0);
                if !full {
                    break;
                }
            }
            Ok((_, Ok(Line::Results { results: r }))) => {
                results = Some(r);
                break;
            }
            Ok((_, Ok(Line::Error { message }))) => {
                error = Some(message);
                break;
            }
            Ok((_, Err(e))) => {
                error = Some(e);
                break;
            }
            Err(_) => break,
        }
    }
    stop(child);
    if let Some(e) = error {
        return Err(e);
    }
    let startup = startup.ok_or("the desktop never reported ready (timeout)")?;
    if full && results.is_none() {
        return Err("the desktop sent no results (timeout)".into());
    }
    Ok((startup, results))
}

/// Measure the desktop budgets into `r`; anything that could not be measured goes to `notes`.
pub fn run(l: &Localizer, given: Option<&Path>, r: &mut Results, notes: &mut Vec<String>) {
    let Some(exe) = locate(given) else {
        notes.push(l.t("pbench-no-desktop"));
        return;
    };
    eprintln!(
        "{}",
        l.t_args(
            "pbench-phase",
            &args!("what" => format!("desktop: {}", exe.display()))
        )
    );
    if let Ok(m) = std::fs::metadata(&exe) {
        #[allow(clippy::cast_precision_loss)]
        r.insert(
            "ui.binary".into(),
            Stat::single(m.len() as f64 / 1_048_576.0),
        );
    }
    let home = std::env::temp_dir().join(format!("kor-bench-desktop-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&home);
    let mut warm = Vec::new();
    for i in 0..LAUNCHES {
        let full = i + 1 == LAUNCHES;
        match launch(&exe, &home, full) {
            Ok((startup, results)) => {
                if i == 0 {
                    r.insert(
                        "ui.startup_cold".into(),
                        Stat::single(startup.as_secs_f64() * 1000.0),
                    );
                } else {
                    warm.push(startup);
                }
                if let Some(results) = results {
                    r.extend(results.into_iter().filter(|(k, _)| k.starts_with("ui.")));
                }
            }
            Err(e) => {
                notes.push(l.t_args("pbench-desktop-failed", &args!("error" => e)));
                break;
            }
        }
    }
    if let Some(s) = Stat::of_durations(&warm) {
        r.insert("ui.startup_warm".into(), s);
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)] // exact values parsed from a fixture
    fn lines_parse() {
        let l: Line = serde_json::from_str(r#"{"event":"ready"}"#).unwrap();
        assert!(matches!(l, Line::Ready));
        let l: Line = serde_json::from_str(
            r#"{"event":"results","results":{"ui.scroll":{"p50":4.0,"p95":9.5,"n":120}}}"#,
        )
        .unwrap();
        let Line::Results { results } = l else {
            panic!()
        };
        assert_eq!(results["ui.scroll"].p95, 9.5);
        assert!(serde_json::from_str::<Line>(r#"{"event":"nope"}"#).is_err());
    }

    #[test]
    #[allow(clippy::items_after_statements)] // helper kept next to its only use
    fn a_fake_desktop_reports_over_loopback() {
        // The protocol end to end with this test process playing the desktop.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let rx = read_lines(&listener);
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        use std::io::Write as _;
        writeln!(s, r#"{{"event":"ready"}}"#).unwrap();
        writeln!(
            s,
            r#"{{"event":"results","results":{{"ui.keypress":{{"p50":1.0,"p95":2.0,"n":50}}}}}}"#
        )
        .unwrap();
        drop(s);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().1,
            Ok(Line::Ready)
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().1,
            Ok(Line::Results { .. })
        ));
    }

    #[test]
    fn missing_desktop_is_a_note_not_a_pass() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let mut r = Results::new();
        let mut notes = Vec::new();
        run(
            &l,
            Some(Path::new("/definitely/not/kor-desktop")),
            &mut r,
            &mut notes,
        );
        assert!(r.is_empty());
        assert_eq!(notes, [l.t("pbench-no-desktop")]);
    }
}
