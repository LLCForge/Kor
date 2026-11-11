//! `kor doctor --ipc-trace [FILE]` (spec §17.2, ADR 0061): runs a real task through the core against
//! the built-in mock provider, in an isolated Kor home, with every call and event written to an IPC
//! trace (JSON lines with timing, schema-checked against `kor-core/schema/ipc.json`). It then reports
//! the record count, schema violations, the slowest steps and whether the core event stream matches
//! the scenario — the same check the CLI, TUI and desktop E2E runs use.
//!
//! To trace your own sessions instead, start any surface with `KOR_IPC_TRACE=<file>`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use kor_i18n::args;
use kor_runtime::e2e::{self, Scenario};

use crate::Ctx;

/// The scenario `kor doctor --ipc-trace` runs: start a task → status events → diff preview →
/// approve → file written → cost ledger updated (§16.4).
const SCENARIO: &str = include_str!("../../../../../tests/e2e/scenarios/task-approve-write.yaml");

/// How long the traced run may take before it is stopped.
const TIMEOUT: Duration = Duration::from_secs(180);

/// `out`: trace file (default `<kor home>/logs/ipc-trace-<unix ms>.ndjson`). Returns whether the run,
/// the schema check and the event comparison all passed.
pub fn run(ctx: &Ctx, out: Option<&Path>) -> bool {
    let l = &ctx.l;
    match run_inner(out) {
        Ok(r) => {
            println!("{}", l.t_args("doctor-ipc-trace-file", &args!("path" => r.trace.display().to_string())));
            println!("  {}", l.t_args("doctor-ipc-trace-records", &args!("count" => r.records, "calls" => r.slowest.len())));
            for (name, ms) in &r.slowest {
                println!("    {ms:>9.1} ms  {name}");
            }
            if r.violations.is_empty() {
                println!("  ✓ {}", l.t("doctor-ipc-trace-schema-ok"));
            } else {
                for v in &r.violations {
                    println!("  ✗ {}", l.t_args("doctor-ipc-trace-schema-bad", &args!("detail" => v.clone())));
                }
            }
            if r.problems.is_empty() {
                println!("  ✓ {}", l.t("doctor-ipc-trace-scenario-ok"));
            } else {
                for p in &r.problems {
                    println!("  ✗ {}", l.t_args("doctor-ipc-trace-scenario-bad", &args!("detail" => p.clone())));
                }
            }
            println!("  {}", l.t("doctor-ipc-trace-hint"));
            r.violations.is_empty() && r.problems.is_empty()
        }
        Err(e) => {
            println!("  ✗ {}", l.t_args("doctor-ipc-trace-failed", &args!("error" => format!("{e:#}"))));
            false
        }
    }
}

struct Report {
    trace: PathBuf,
    records: usize,
    /// Slowest timed records (calls / steps), at most five.
    slowest: Vec<(String, f64)>,
    violations: Vec<String>,
    problems: Vec<String>,
}

fn default_file() -> anyhow::Result<PathBuf> {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    Ok(kor_runtime::kor_home()
        .ok_or_else(|| anyhow::anyhow!("no Kor data folder"))?
        .join("logs")
        .join(format!("ipc-trace-{ms}.ndjson")))
}

fn run_inner(out: Option<&Path>) -> anyhow::Result<Report> {
    let scenario = Scenario::parse(SCENARIO)?;
    let target = match out {
        Some(p) => p.to_owned(),
        None => default_file()?,
    };
    let dir = std::env::temp_dir().join(format!("kor-ipc-trace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rt = crate::providers::runtime()?;
    let (env, output) = rt.block_on(async {
        let mock = e2e::MockServer::start(scenario.mock.clone()).await?;
        let env = e2e::isolated_env(&dir, &scenario, &mock.url, "cli")?;
        let exe = std::env::current_exe()?;
        let env2 = env.clone();
        let input = cli_input(&scenario);
        // The surface is the real `kor` REPL (line mode), answering from stdin like a user would.
        let output = tokio::task::spawn_blocking(move || drive(&exe, &env2, &input)).await??;
        anyhow::Ok((env, output))
    })?;
    let outcome = e2e::check(&scenario, &env, "cli", &output);
    let records = kor_core::ipc_trace::read(&env.trace).unwrap_or_default();
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&env.trace, &target)?;
    let _ = std::fs::remove_dir_all(&dir);
    let mut timed: Vec<(String, f64)> = records
        .iter()
        .filter_map(|r| Some((r["name"].as_str()?.to_owned(), r["ms"].as_f64()?)))
        .collect();
    timed.sort_by(|a, b| b.1.total_cmp(&a.1));
    timed.truncate(5);
    let violations = kor_core::ipc_trace::violations(&records)
        .into_iter()
        .map(|(n, e)| format!("{n}: {}", e.join("; ")))
        .collect();
    Ok(Report { trace: target, records: records.len(), slowest: timed, violations, problems: outcome.problems })
}

/// What a user types for the scenario's steps: each prompt, each approval (`y`/`a`/`n`).
fn cli_input(s: &Scenario) -> String {
    let mut text = String::new();
    for st in &s.steps {
        if let Some(p) = &st.prompt {
            text.push_str(p);
        } else if let Some(a) = &st.approve {
            text.push_str(match a.as_str() {
                "always" => "a",
                "no" => "n",
                _ => "y",
            });
        } else {
            continue;
        }
        text.push('\n');
    }
    text
}

fn drive(exe: &Path, env: &e2e::IsolatedEnv, input: &str) -> anyhow::Result<String> {
    let mut child = Command::new(exe)
        .arg("--lang")
        .arg("en")
        .current_dir(&env.project)
        .envs(&env.vars)
        .env("KOR_UI", "line")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // End of input closes the REPL after the last step.
        stdin.write_all(input.as_bytes())?;
    }
    let started = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            anyhow::bail!("the traced run took longer than {}s", TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    Ok(format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_scenario_and_input() {
        let s = Scenario::parse(SCENARIO).unwrap();
        assert_eq!(s.name, "task-approve-write");
        let input = cli_input(&s);
        assert!(input.ends_with("y\n"), "{input:?}");
        assert!(input.lines().count() >= 2);
    }
}
