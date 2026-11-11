//! `kor mcp dev ./server [-- command…]` (Master Build Prompt §28.15, ADR 0063): run an MCP server
//! under development, reload it when its files change (with a tool-schema diff), inspect its
//! tools/resources/prompts, call tools by hand, and watch the JSON-RPC traffic live (also written
//! to `.kor/mcp-dev/<time>.jsonl`). Logic: `kor_runtime::mcp_dev`.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::Args;
use kor_i18n::{Localizer, args};
use kor_runtime::mcp_dev::{self, Dir, Inventory, Session, Sink, Target, Traffic};
use serde_json::{Value, json};

use crate::Ctx;

#[derive(Debug, Args)]
pub struct McpDevArgs {
    /// Server file or folder (a .js/.ts/.py file, or a project with package.json, Cargo.toml, …).
    pub path: Option<PathBuf>,
    /// Do not reload on file changes.
    #[arg(long)]
    pub no_watch: bool,
    /// Print every JSON-RPC message in full (default: one line each).
    #[arg(long)]
    pub raw: bool,
    /// The server command, when it cannot be detected: `kor mcp dev ./srv -- node build/index.js`.
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-mcp-dev"))
        .mut_arg("path", |a| a.help(l.t("arg-mcp-dev-path")))
        .mut_arg("no_watch", |a| a.help(l.t("arg-mcp-dev-no-watch")))
        .mut_arg("raw", |a| a.help(l.t("arg-mcp-dev-raw")))
        .mut_arg("command", |a| a.help(l.t("arg-mcp-dev-command")))
}

/// One line for a message: `→ tools/call #3`, `← #3 ok`, `← #4 error -32602 …`.
pub fn summary(t: &Traffic) -> String {
    let v: Option<Value> = matches!(t.dir, Dir::In | Dir::Out)
        .then(|| serde_json::from_str(&t.text).ok())
        .flatten();
    match (t.dir, v) {
        (Dir::Out, Some(v)) => format!(
            "→ {}{}",
            v["method"].as_str().unwrap_or("?"),
            v["id"]
                .as_u64()
                .map(|i| format!(" #{i}"))
                .unwrap_or_default()
        ),
        (Dir::In, Some(v)) if v.get("error").is_some() => format!(
            "← #{} error {} {} ({})",
            v["id"],
            v["error"]["code"],
            v["error"]["message"].as_str().unwrap_or(""),
            mcp_dev::explain(v["error"]["code"].as_i64().unwrap_or(0))
        ),
        (Dir::In, Some(v)) if v.get("method").is_some() => {
            format!("← {}", v["method"].as_str().unwrap_or(""))
        }
        (Dir::In, Some(v)) => format!("← #{} ok", v["id"]),
        (Dir::Stderr, _) => format!("  stderr: {}", t.text),
        _ => format!("  ! {}", t.text),
    }
}

fn sink(raw: bool, log: Option<PathBuf>) -> Sink {
    let file = log.and_then(|p| {
        let _ = std::fs::create_dir_all(p.parent()?);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    let file = Arc::new(Mutex::new(file));
    Arc::new(move |t: &Traffic| {
        let line = if raw {
            format!("{:?} {}", t.dir, t.text)
        } else {
            summary(t)
        };
        eprintln!("\x1b[2m{line}\x1b[0m");
        if let Some(f) = file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            let _ = writeln!(f, "{}", serde_json::to_string(t).unwrap_or_default());
        }
    })
}

fn print_inventory(l: &Localizer, inv: &Inventory) {
    let name = inv.server["serverInfo"]["name"].as_str().unwrap_or("?");
    let version = inv.server["serverInfo"]["version"].as_str().unwrap_or("");
    println!(
        "{}",
        l.t_args(
            "mcpdev-server",
            &args!("name" => name.to_owned(), "version" => version.to_owned())
        )
    );
    println!("{}", l.t_args("mcpdev-counts", &args!("tools" => inv.tools.len(), "resources" => inv.resources.len(), "prompts" => inv.prompts.len())));
    for t in &inv.tools {
        let props: Vec<String> = t["inputSchema"]["properties"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        println!(
            "  tool  {}({})  {}",
            t["name"].as_str().unwrap_or("?"),
            props.join(", "),
            t["description"]
                .as_str()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
        );
    }
    for r in &inv.resources {
        println!(
            "  res   {}  {}",
            r["uri"].as_str().unwrap_or("?"),
            r["name"].as_str().unwrap_or("")
        );
    }
    for p in &inv.prompts {
        println!("  prompt {}", p["name"].as_str().unwrap_or("?"));
    }
}

fn show(v: &Result<Value, String>) {
    match v {
        Ok(v) => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
        Err(e) => println!("\x1b[31m{e}\x1b[0m"),
    }
}

/// `call echo {"text":"hi"}` → ("echo", {"text":"hi"}).
pub fn split_call(rest: &str) -> Result<(String, Value), String> {
    let rest = rest.trim();
    let (name, json_text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if name.is_empty() {
        return Err("name missing".into());
    }
    let args = if json_text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(json_text.trim())
            .map_err(|e| format!("arguments are not JSON: {e}"))?
    };
    Ok((name.to_owned(), args))
}

/// Start the server and list what it offers.
async fn start(target: &Target, sink: &Sink) -> Result<(Session, Inventory), String> {
    let s = Session::start(target, sink.clone()).await?;
    let inv = s.inspect().await?;
    Ok((s, inv))
}

enum Input {
    Line(String),
    Reload,
}

pub fn run(ctx: &Ctx, a: &McpDevArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let target = match (&a.path, a.command.split_first()) {
        (path, Some((cmd, rest))) => {
            let cwd = path
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let dir = if cwd.is_file() {
                cwd.parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default()
            } else {
                cwd.clone()
            };
            Target {
                command: cmd.clone(),
                args: rest.to_vec(),
                cwd: dir,
                watch: cwd,
            }
        }
        (Some(p), None) => match mcp_dev::detect(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                return Ok(false);
            }
        },
        (None, None) => {
            eprintln!("{}", l.t("mcpdev-need-path"));
            return Ok(false);
        }
    };
    let root = crate::graph_cmd::root()?;
    let log = root.join(".kor").join("mcp-dev").join(format!(
        "{}.jsonl",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    println!(
        "{}",
        l.t_args(
            "mcpdev-log",
            &args!("path" => kor_tools::display_path(&log))
        )
    );
    let sink = sink(a.raw, Some(log));
    let rt = crate::providers::runtime()?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Input>();
    let _watcher = if a.no_watch {
        None
    } else {
        let tx = tx.clone();
        match mcp_dev::watch(&target.watch, move || {
            let _ = tx.send(Input::Reload);
        }) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("{}", l.t_args("mcpdev-watch-failed", &args!("error" => e)));
                None
            }
        }
    };
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut s = String::new();
        loop {
            s.clear();
            if std::io::BufRead::read_line(&mut stdin.lock(), &mut s).unwrap_or(0) == 0 {
                let _ = tx.send(Input::Line("quit".into()));
                break;
            }
            let _ = tx.send(Input::Line(s.trim().to_owned()));
        }
    });
    rt.block_on(async {
        let mut session: Option<Session> = None;
        let mut tools: Vec<Value> = Vec::new();
        match start(&target, &sink).await {
            Ok((s, inv)) => {
                print_inventory(l, &inv);
                tools = inv.tools;
                session = Some(s);
            }
            Err(e) => println!(
                "\x1b[31m{}\x1b[0m",
                l.t_args("mcpdev-start-failed", &args!("error" => e))
            ),
        }
        println!("{}", l.t("mcpdev-help"));
        loop {
            print!("mcp> ");
            let _ = std::io::stdout().flush();
            let Some(input) = rx.recv().await else { break };
            let line = match input {
                Input::Reload => "reload".to_owned(),
                Input::Line(s) => s,
            };
            let (cmd, rest) = line
                .split_once(char::is_whitespace)
                .unwrap_or((line.as_str(), ""));
            match cmd {
                "" => {}
                "quit" | "exit" | "q" => break,
                "help" | "?" => println!("{}", l.t("mcpdev-help")),
                "reload" | "r" => {
                    if let Some(s) = session.take() {
                        s.stop();
                    }
                    println!("{}", l.t("mcpdev-reloading"));
                    match start(&target, &sink).await {
                        Ok((s, inv)) => {
                            let d = mcp_dev::schema_diff(&tools, &inv.tools);
                            if d.is_empty() {
                                println!("{}", l.t("mcpdev-schema-same"));
                            } else {
                                for n in &d.added {
                                    println!("  \x1b[32m+ {n}\x1b[0m");
                                }
                                for n in &d.removed {
                                    println!("  \x1b[31m- {n}\x1b[0m");
                                }
                                for (n, w) in &d.changed {
                                    println!("  \x1b[33m~ {n}: {w}\x1b[0m");
                                }
                            }
                            tools = inv.tools;
                            session = Some(s);
                        }
                        Err(e) => println!(
                            "\x1b[31m{}\x1b[0m",
                            l.t_args("mcpdev-start-failed", &args!("error" => e))
                        ),
                    }
                }
                _ => {
                    let Some(s) = session.as_ref() else {
                        println!("{}", l.t("mcpdev-not-running"));
                        continue;
                    };
                    match cmd {
                        "tools" | "resources" | "prompts" | "ls" => match s.inspect().await {
                            Ok(inv) => print_inventory(l, &inv),
                            Err(e) => println!("\x1b[31m{e}\x1b[0m"),
                        },
                        "call" | "c" => match split_call(rest) {
                            Ok((n, a)) => show(&s.call_tool(&n, a).await),
                            Err(e) => println!("{e}"),
                        },
                        "read" => show(&s.read_resource(rest.trim()).await),
                        "prompt" => match split_call(rest) {
                            Ok((n, a)) => show(&s.get_prompt(&n, a).await),
                            Err(e) => println!("{e}"),
                        },
                        "log" => {
                            let n: usize = rest.trim().parse().unwrap_or(20);
                            let t = s.traffic();
                            for x in t.iter().skip(t.len().saturating_sub(n)) {
                                println!("{:>7}ms {:?} {}", x.ms, x.dir, x.text);
                            }
                        }
                        _ => println!("{}", l.t("mcpdev-help")),
                    }
                }
            }
        }
        if let Some(s) = session {
            s.stop();
        }
    });
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        a: McpDevArgs,
    }

    #[test]
    fn parses_and_formats() {
        let c = Cli::try_parse_from(["d", "./srv", "--", "node", "build/index.js"]).unwrap();
        assert_eq!(
            c.a.command,
            vec!["node".to_owned(), "build/index.js".to_owned()]
        );
        assert_eq!(
            split_call("echo {\"text\": \"hi\"}").unwrap(),
            ("echo".to_owned(), json!({"text": "hi"}))
        );
        assert_eq!(split_call("ping").unwrap().1, json!({}));
        assert!(split_call("echo {bad").is_err());
        let t = |dir, text: &str| Traffic {
            ms: 0,
            dir,
            text: text.into(),
        };
        assert_eq!(
            summary(&t(Dir::Out, r#"{"id":3,"method":"tools/call"}"#)),
            "→ tools/call #3"
        );
        assert!(
            summary(&t(
                Dir::In,
                r#"{"id":3,"error":{"code":-32601,"message":"x"}}"#
            ))
            .contains("method not found")
        );
        assert_eq!(summary(&t(Dir::In, r#"{"id":4,"result":{}}"#)), "← #4 ok");
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
