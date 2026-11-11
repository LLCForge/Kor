//! `kor mcp list|add|remove|enable|disable|test|run|serve` (spec §8.3, ADR 0038). Generators and
//! the stdio server loop live in `kor_agent::mcp_gen`; configuration editing in
//! `kor_agent::mcp_config`. This file only parses, prints and wires the runtime pieces.

use std::path::PathBuf;
use std::sync::Arc;

use kor_agent::mcp_config::{self, Entry, Locations};
use kor_agent::mcp_gen::{self, AddRequest, Auth, Kind};
use kor_i18n::{Localizer, args};
use serde_json::{Value, json};

use crate::Ctx;

/// `kor mcp dev` (§28.15).
#[path = "mcpdev_cmd.rs"]
pub mod dev;

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per process
pub enum McpCmd {
    /// Every configured server (Kor, Claude Code, the project, other tools), on/off, tool counts.
    List {
        /// Start enabled stdio servers to count their tools.
        #[arg(long)]
        probe: bool,
    },
    /// `kor mcp add <source>` (postgres://…, mysql://…, sqlite:FILE, OpenAPI/GraphQL URL or file,
    /// ./folder), or `kor mcp add <id> -- <command…>` / `--url URL`.
    Add {
        source: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long = "env")]
        env: Vec<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        disabled: bool,
        /// Server id for a generated server (default: from the source).
        #[arg(long)]
        id: Option<String>,
        /// openapi, graphql, database or local (default: detected).
        #[arg(long)]
        kind: Option<String>,
        /// `OpenAPI` base URL or `GraphQL` endpoint (for schema files).
        #[arg(long = "base-url")]
        base_url: Option<String>,
        /// Also generate tools that change data (they always ask before running).
        #[arg(long = "allow-writes")]
        allow_writes: bool,
        /// bearer:SECRET, header:NAME:SECRET or query:NAME:SECRET (SECRET from `kor vault`).
        #[arg(long)]
        auth: Option<String>,
        #[arg(long = "row-limit")]
        row_limit: Option<usize>,
        /// Seconds per call.
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(last = true)]
        command: Vec<String>,
    },
    Remove {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Connect, list tools, call one read-only tool, with timings.
    Test {
        id: String,
    },
    /// Serve a generated server on stdin/stdout (what its `mcp.json` entry runs).
    #[command(hide = true)]
    Run {
        id: String,
    },
    /// Develop an MCP server: hot reload, inspect, call tools, JSON-RPC log (§28.15).
    Dev(dev::McpDevArgs),
    /// Offer Kor's graph, memory and cost tools to other MCP clients on stdin/stdout.
    Serve {
        /// Comma-separated tool groups (graph, memory, cost, tasks); default from mcp.json `serve.groups`.
        #[arg(long)]
        groups: Option<String>,
    },
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-mcp"))
        .mut_subcommand("list", |s| s.about(l.t("cmd-mcp-list")))
        .mut_subcommand("add", |s| s.about(l.t("cmd-mcp-add")))
        .mut_subcommand("remove", |s| s.about(l.t("cmd-mcp-remove")))
        .mut_subcommand("enable", |s| s.about(l.t("cmd-mcp-enable")))
        .mut_subcommand("disable", |s| s.about(l.t("cmd-mcp-disable")))
        .mut_subcommand("test", |s| s.about(l.t("cmd-mcp-test")))
        .mut_subcommand("serve", |s| s.about(l.t("cmd-mcp-serve")))
        .mut_subcommand("dev", |s| dev::localize(s, l))
}

fn root() -> PathBuf {
    kor_runtime::project_root().unwrap_or_default()
}

fn loc() -> Result<Locations, String> {
    let root = root();
    Ok(Locations {
        kor_file: kor_config::user_dir()
            .ok_or("user configuration folder not found")?
            .join("mcp.json"),
        home: dirs::home_dir(),
        trusted: kor_runtime::is_trusted(&root),
        root,
    })
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

/// `kor mcp` without a subcommand lists.
pub fn run(ctx: &Ctx, cmd: Option<McpCmd>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    Ok(match cmd {
        None => list(l, false)?,
        Some(McpCmd::List { probe }) => list(l, probe)?,
        Some(McpCmd::Add {
            source,
            url,
            env,
            label,
            disabled,
            id,
            kind,
            base_url,
            allow_writes,
            auth,
            row_limit,
            timeout,
            command,
        }) => {
            if url.is_some() || !command.is_empty() {
                crate::extra::mcp_add(
                    ctx,
                    &source,
                    url.as_deref(),
                    &env,
                    label,
                    disabled,
                    &command,
                )
            } else {
                let kind = match kind.as_deref().map(|k| Kind::parse(k).ok_or(k)) {
                    Some(Err(k)) => {
                        return Ok(fail(
                            l,
                            format!("--kind {k}: openapi, graphql, database, local"),
                        ));
                    }
                    Some(Ok(k)) => Some(k),
                    None => None,
                };
                let auth = match auth.as_deref().map(Auth::parse).transpose() {
                    Ok(a) => a,
                    Err(e) => return Ok(fail(l, e)),
                };
                let req = AddRequest {
                    source,
                    id,
                    kind,
                    endpoint: base_url,
                    allow_writes,
                    auth,
                    label,
                    row_limit,
                    timeout_secs: timeout,
                };
                add(l, &req, disabled)?
            }
        }
        Some(McpCmd::Remove { id }) => crate::extra::mcp_remove(ctx, &id),
        Some(McpCmd::Enable { id }) => crate::extra::mcp_set_enabled(ctx, &id, true),
        Some(McpCmd::Disable { id }) => crate::extra::mcp_set_enabled(ctx, &id, false),
        Some(McpCmd::Test { id }) => test(l, &id)?,
        Some(McpCmd::Dev(a)) => dev::run(ctx, &a)?,
        Some(McpCmd::Run { id }) => run_generated(ctx, &id)?,
        Some(McpCmd::Serve { groups }) => serve(ctx, groups.as_deref())?,
    })
}

fn add(l: &Localizer, req: &AddRequest, disabled: bool) -> anyhow::Result<bool> {
    let loc = match loc() {
        Ok(x) => x,
        Err(e) => return Ok(fail(l, e)),
    };
    let exe = std::env::current_exe()?;
    let rt = crate::providers::runtime()?;
    let mut added = match rt.block_on(mcp_gen::generate(req, &loc.root, &exe)) {
        Ok(a) => a,
        Err(e) => return Ok(fail(l, e)),
    };
    if disabled {
        added.entry["disabled"] = json!(true);
    }
    if let Err(e) = mcp_gen::install(&loc, &added, &kor_secrets::KeychainStore) {
        return Ok(fail(l, e));
    }
    println!(
        "{}",
        l.t_args(
            "mcp-gen-added",
            &args!("id" => added.id.clone(), "kind" => added.kind.key(), "tools" => added.tools)
        )
    );
    if added.password.is_some() {
        println!(
            "{}",
            l.t_args("mcp-gen-password", &args!("id" => added.id.clone()))
        );
    }
    if let Some(n) = added.note {
        println!("  {n}");
    }
    Ok(true)
}

/// A stdio spec from a configuration entry, `${VAR}` expanded.
pub fn spec_of(e: &Entry) -> kor_agent::mcp_tools::ServerSpec {
    let env = |k: &str| std::env::var(k).ok();
    let x = |s: &str| kor_agent::mcp_tools::expand(s, &env);
    kor_agent::mcp_tools::ServerSpec {
        name: e.id.clone(),
        command: x(e.raw.get("command").and_then(Value::as_str).unwrap_or("")),
        args: mcp_config::strings(e.raw.get("args"))
            .iter()
            .map(|a| x(a))
            .collect(),
        env: mcp_config::string_map(e.raw.get("env"))
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), x(v))))
            .collect(),
        origin: e.external.unwrap_or("mcp.json").to_owned(),
    }
}

fn list(l: &Localizer, probe: bool) -> anyhow::Result<bool> {
    let loc = match loc() {
        Ok(x) => x,
        Err(e) => return Ok(fail(l, e)),
    };
    let entries = match mcp_config::entries(&loc) {
        Ok(e) => e,
        Err(e) => return Ok(fail(l, e)),
    };
    let probed: Vec<kor_agent::mcp_tools::Connected> = if probe {
        let specs: Vec<_> = entries
            .iter()
            .filter(|e| {
                !mcp_config::disabled(&e.raw)
                    && !mcp_config::is_http(&e.raw)
                    && e.raw.get("kor").is_none()
            })
            .map(spec_of)
            .collect();
        let rt = crate::providers::runtime()?;
        rt.block_on(kor_agent::mcp_tools::connect_all(&specs, &loc.root))
            .1
    } else {
        Vec::new()
    };
    let compat = kor_compat::load(&kor_compat::LoadOptions {
        root: loc.root.clone(),
        home: loc.home.clone(),
        codex_home: None,
        trusted: loc.trusted,
    });
    let inherited: Vec<String> = compat
        .mcp_servers
        .iter()
        .map(|s| s.name.clone())
        .filter(|n| !entries.iter().any(|e| &e.id == n))
        .collect();
    if entries.is_empty() && inherited.is_empty() {
        println!("{}", l.t("mcp-list-empty"));
    }
    for e in &entries {
        let state = if mcp_config::disabled(&e.raw) {
            "off"
        } else {
            "on "
        };
        let tools = mcp_gen::tool_count(&loc.kor_file, &e.raw)
            .map(|n| n.to_string())
            .or_else(|| {
                probed
                    .iter()
                    .find(|c| c.server == e.id)
                    .map(|c| match &c.result {
                        Ok(n) => n.to_string(),
                        Err(err) => format!("! {err}"),
                    })
            })
            .unwrap_or_else(|| "?".into());
        let kind = e
            .raw
            .get("kor")
            .and_then(|k| k.get("generated"))
            .and_then(Value::as_str)
            .unwrap_or(if mcp_config::is_http(&e.raw) {
                "http"
            } else {
                "stdio"
            });
        let origin = e.external.unwrap_or("kor");
        println!(
            "{:<24} {state}  {kind:<8} tools: {tools:<6} ({origin})",
            e.id
        );
    }
    if !inherited.is_empty() {
        println!(
            "{}",
            l.t_args(
                "mcp-list-inherited",
                &args!("names" => inherited.join(", "))
            )
        );
    }
    Ok(true)
}

fn test(l: &Localizer, id: &str) -> anyhow::Result<bool> {
    let loc = match loc() {
        Ok(x) => x,
        Err(e) => return Ok(fail(l, e)),
    };
    let Some(e) = mcp_config::entries(&loc)
        .ok()
        .and_then(|all| all.into_iter().find(|e| e.id == id))
    else {
        eprintln!(
            "{}",
            l.t_args("mcp-not-found", &args!("id" => id.to_owned()))
        );
        return Ok(false);
    };
    if mcp_config::is_http(&e.raw) {
        eprintln!(
            "{}",
            l.t_args("mcp-remote-test", &args!("id" => id.to_owned()))
        );
        return Ok(false);
    }
    let rt = crate::providers::runtime()?;
    let r = rt.block_on(mcp_gen::test(&spec_of(&e), &loc.root));
    if let Some(err) = &r.error {
        eprintln!(
            "{}",
            l.t_args(
                "mcp-test-failed",
                &args!("id" => id.to_owned(), "error" => err.clone())
            )
        );
        return Ok(false);
    }
    println!(
        "{}",
        l.t_args(
            "mcp-test-ok",
            &args!("id" => id.to_owned(), "connect" => r.connect_ms, "count" => r.tools.len(), "list" => r.list_ms)
        )
    );
    println!("  {}", r.tools.join(", "));
    match &r.probe {
        Some(tool) => {
            let result = if r.probe_ok { "ok" } else { "error" };
            println!(
                "{}",
                l.t_args(
                    "mcp-test-probe",
                    &args!("tool" => tool.clone(), "result" => result, "ms" => r.probe_ms)
                )
            );
            for line in r.probe_output.lines().take(6) {
                println!("    {line}");
            }
        }
        None => println!("{}", l.t("mcp-test-no-probe")),
    }
    Ok(r.probe.is_none() || r.probe_ok)
}

fn secrets(ctx: &Ctx) -> mcp_gen::Secrets {
    let cfg = ctx
        .config
        .as_ref()
        .map(|c| c.config.vault.clone())
        .unwrap_or_default();
    // stdin is the protocol here: a file vault needs KOR_VAULT_PASSPHRASE, else env only.
    match kor_runtime::secure::vault_store(&cfg, None) {
        Ok(store) => mcp_gen::secrets_from(Arc::from(store)),
        Err(_) => Arc::new(|n: &str| std::env::var(n).ok().filter(|v| !v.is_empty())),
    }
}

async fn stdio(b: &dyn kor_agent::mcp_gen::serve::Backend) -> std::io::Result<()> {
    kor_agent::mcp_gen::serve::serve(b, tokio::io::stdin(), tokio::io::stdout()).await
}

fn run_generated(ctx: &Ctx, id: &str) -> anyhow::Result<bool> {
    let kor_file = match loc() {
        Ok(x) => x.kor_file,
        Err(e) => return Ok(fail(&ctx.l, e)),
    };
    let def = match mcp_gen::load(&kor_file, id) {
        Ok(d) => d,
        Err(e) => return Ok(fail(&ctx.l, e)),
    };
    let backend = mcp_gen::backend(def, secrets(ctx));
    crate::providers::runtime()?.block_on(stdio(backend.as_ref()))?;
    Ok(true)
}

fn serve(ctx: &Ctx, groups: Option<&str>) -> anyhow::Result<bool> {
    let loc = match loc() {
        Ok(x) => x,
        Err(e) => return Ok(fail(&ctx.l, e)),
    };
    let groups = match kor_agent::mcp_gen::kor_serve::configured_groups(&loc.kor_file, groups) {
        Ok(g) => g,
        Err(e) => return Ok(fail(&ctx.l, e)),
    };
    let budget = ctx
        .config
        .as_ref()
        .map(|c| c.config.budget.clone())
        .unwrap_or_default();
    let cost: kor_agent::mcp_gen::kor_serve::CostFn = Arc::new(move |days| {
        let entries = kor_runtime::usage::file()
            .map(|p| kor_runtime::usage::read(&p))
            .unwrap_or_default();
        let r = kor_runtime::cost::report(
            &entries,
            &kor_runtime::cost::Prices::load(),
            kor_runtime::cost::now_ms(),
            days,
            &budget,
        );
        serde_json::to_value(r).map_err(|e| e.to_string())
    });
    let task_root = loc.root.clone();
    let tasks: kor_agent::mcp_gen::kor_serve::TasksFn = Arc::new(move |id| {
        let v = match id {
            Some(id) => serde_json::to_value(kor_runtime::task::load(&task_root, id)?),
            None => serde_json::to_value(kor_runtime::task::list(&task_root)),
        };
        v.map_err(|e| e.to_string())
    });
    let rt = crate::providers::runtime()?;
    let server = match kor_agent::mcp_gen::kor_serve::KorServe::new(
        &loc.root,
        kor_runtime::kor_home(),
        groups,
        Some(cost),
        Some(tasks),
    ) {
        Ok(s) => s,
        Err(e) => return Ok(fail(&ctx.l, e)),
    };
    rt.block_on(stdio(&server))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: Option<McpCmd>,
    }

    #[test]
    fn parses_old_and_new_forms() {
        let p = |a: &[&str]| {
            T::try_parse_from(std::iter::once("mcp").chain(a.iter().copied())).map(|t| t.c)
        };
        assert!(p(&[]).unwrap().is_none());
        assert!(matches!(
            p(&["list", "--probe"]).unwrap(),
            Some(McpCmd::List { probe: true })
        ));
        let Some(McpCmd::Add {
            source, command, ..
        }) = p(&["add", "docs", "--", "npx", "-y", "d"]).unwrap()
        else {
            panic!()
        };
        assert_eq!((source.as_str(), command.len()), ("docs", 3));
        let Some(McpCmd::Add {
            source,
            allow_writes,
            auth,
            kind,
            ..
        }) = p(&[
            "add",
            "postgres://u@h/db",
            "--allow-writes",
            "--auth",
            "bearer:T",
            "--kind",
            "database",
        ])
        .unwrap()
        else {
            panic!()
        };
        assert!(source.starts_with("postgres") && allow_writes && auth.is_some() && kind.is_some());
        assert!(matches!(
            p(&["test", "shop"]).unwrap(),
            Some(McpCmd::Test { .. })
        ));
        assert!(matches!(
            p(&["run", "shop"]).unwrap(),
            Some(McpCmd::Run { .. })
        ));
        let Some(McpCmd::Serve { groups }) = p(&["serve", "--groups", "graph,cost"]).unwrap()
        else {
            panic!()
        };
        assert_eq!(groups.as_deref(), Some("graph,cost"));
    }
}
