//! E2E helper for surfaces outside the Rust tests (the desktop's `Playwright` / `WebDriver` runs, ADR 0061).
//!
//! `mock_provider serve --scenario <yaml> --root <dir> [--surface desktop]`
//!   starts the scenario's mock provider, prepares the isolated Kor home, prints one JSON line
//!   `{ "url", "env", "project", "trace", "scenario" }` and serves until stdin closes.
//! `mock_provider check --scenario <yaml> --root <dir> --surface <s> [--output <file>] [--requests <file>]`
//!   compares the finished run with the scenario (`kor_runtime::e2e::check_with`), prints the outcome as
//!   JSON and exits 1 when anything differs.
//! `mock_provider compare <outcome.json>…` exits 1 when the surfaces' event streams differ.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use kor_runtime::e2e::{self, Outcome, Scenario};
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("mock_provider: {e}");
            ExitCode::from(2)
        }
    }
}

fn scenario(args: &[String]) -> Result<Scenario, String> {
    let path = arg(args, "--scenario").ok_or("--scenario <yaml> is required")?;
    Scenario::load(&PathBuf::from(path)).map_err(|e| e.to_string())
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let root = || {
        arg(args, "--root")
            .map(PathBuf::from)
            .ok_or("--root <dir> is required")
    };
    match args.first().map(String::as_str) {
        Some("serve") => {
            let s = scenario(args)?;
            let root = root()?;
            let surface = arg(args, "--surface").unwrap_or_else(|| "desktop".into());
            let kor = arg(args, "--kor").map(PathBuf::from);
            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            let mock = rt
                .block_on(e2e::MockServer::start(s.mock.clone()))
                .map_err(|e| e.to_string())?;
            let env = e2e::isolated_env_with(&root, &s, &mock.url, &surface, kor.as_deref())
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                json!({ "url": mock.url, "env": env.vars, "project": env.project, "trace": env.trace, "home": env.home, "scenario": s })
            );
            // Serve until the driver closes stdin, then leave the requests next to the trace.
            let mut sink = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut sink);
            let requests = serde_json::to_string(&mock.requests()).map_err(|e| e.to_string())?;
            std::fs::write(root.join(format!("requests-{surface}.json")), requests)
                .map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
        Some("check") => {
            let s = scenario(args)?;
            let root = root()?;
            let surface = arg(args, "--surface").ok_or("--surface is required")?;
            let output = arg(args, "--output")
                .map(|p| std::fs::read_to_string(p).unwrap_or_default())
                .unwrap_or_default();
            let requests: Vec<Value> =
                std::fs::read_to_string(root.join(format!("requests-{surface}.json")))
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default();
            let env = e2e::IsolatedEnv {
                project: root
                    .join("project")
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
                home: root.join("home"),
                trace: root.join(format!("trace-{surface}.ndjson")),
                root,
                vars: std::collections::BTreeMap::new(),
            };
            let o = e2e::check_with(&s, &env, &surface, &output, &requests);
            println!("{}", serde_json::to_string(&o).map_err(|e| e.to_string())?);
            Ok(if o.problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Some("compare") => {
            let outs: Vec<Outcome> = args[1..]
                .iter()
                .map(|p| {
                    let v: Value = serde_json::from_str(
                        &std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(Outcome {
                        surface: v["surface"].as_str().unwrap_or_default().to_owned(),
                        events: v["events"].as_array().cloned().unwrap_or_default(),
                        problems: Vec::new(),
                    })
                })
                .collect::<Result<_, String>>()?;
            let diffs = e2e::compare(&outs);
            for d in &diffs {
                println!("{d}");
            }
            Ok(if diffs.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        _ => Err("usage: mock_provider serve|check|compare …".into()),
    }
}
