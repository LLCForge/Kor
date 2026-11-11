//! CLI surfaces of features the desktop already had (parity.toml): agents, memory, connections,
//! cost, routines and MCP editing. All logic lives in the core crates; this only prints.

use kor_i18n::args;
use serde_json::json;

use crate::Ctx;

fn project_root() -> std::path::PathBuf {
    kor_runtime::project_root().unwrap_or_default()
}

/// `kor agents`: the same list the `Agent` tool offers.
pub fn agents(_ctx: &Ctx) -> bool {
    let compat = kor_runtime::load_compat(&project_root());
    for name in ["general-purpose", "Explore", "Plan"] {
        println!("{name}  [built-in]");
    }
    for a in kor_agent::builtin::with_builtins(compat.agents) {
        let short: String = a.description.chars().take(100).collect();
        let origin = if a.source.path.starts_with("<built-in>") {
            "built-in"
        } else {
            "custom"
        };
        println!("{}  [{origin}]  {short}", a.name);
    }
    true
}

/// `kor memory`: this project's `MEMORY.md` (or its path when empty).
pub fn memory(ctx: &Ctx) -> bool {
    let Some(lay) = kor_runtime::memory_layout(&project_root()) else {
        println!("{}", ctx.l.t("memory-none"));
        return true;
    };
    match kor_agent::memory::read_entrypoint(&lay) {
        Some(text) => println!("{text}"),
        None => println!("{}", kor_tools::display_path(&lay.entry)),
    }
    true
}

/// `kor connections`: SSH hosts and databases from the user's `connections.toml`.
pub fn connections(ctx: &Ctx) -> bool {
    let l = &ctx.l;
    let Some(file) = kor_config::user_dir().map(|d| d.join("connections.toml")) else {
        println!("{}", l.t("connections-none"));
        return true;
    };
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    match kor_agent::remote::Connections::parse(&text) {
        Ok(c) if c.ssh.is_empty() && c.db.is_empty() => {
            println!(
                "{}",
                l.t_args(
                    "connections-empty",
                    &args!("path" => file.display().to_string())
                )
            );
            true
        }
        Ok(c) => {
            for (name, h) in &c.ssh {
                let user = h
                    .user
                    .as_deref()
                    .map(|u| format!("{u}@"))
                    .unwrap_or_default();
                let port = h.port.map(|p| format!(":{p}")).unwrap_or_default();
                println!("ssh  {name}  {user}{}{port}", h.host);
            }
            for (name, d) in &c.db {
                let target = d
                    .file
                    .clone()
                    .or_else(|| d.host.clone())
                    .unwrap_or_default();
                let via = d
                    .ssh
                    .as_deref()
                    .map(|s| format!("  (ssh {s})"))
                    .unwrap_or_default();
                println!("db   {name}  {:?}  {target}{via}", d.kind);
            }
            true
        }
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            false
        }
    }
}

/// The `kor cost` report: per model cost, requests and tokens from the shared ledger (CLI and
/// desktop), today, this month against `[budget]`, and a projection.
pub fn cost_report(ctx: &Ctx, days: u32) -> kor_runtime::cost::Report {
    let budget = ctx
        .config
        .as_ref()
        .map(|c| c.config.budget.clone())
        .unwrap_or_default();
    let entries = kor_runtime::usage::file()
        .map(|p| kor_runtime::usage::read(&p))
        .unwrap_or_default();
    kor_runtime::cost::report(
        &entries,
        &kor_runtime::cost::Prices::load(),
        kor_runtime::cost::now_ms(),
        days,
        &budget,
    )
}

/// `kor cost`: the cost dashboard (§5.2).
pub fn cost(ctx: &Ctx, days: u32) -> bool {
    for line in kor_runtime::cost::report_lines(&ctx.l, &cost_report(ctx, days)) {
        println!("{line}");
    }
    true
}

#[derive(Debug, clap::Subcommand)]
pub enum RoutineCmd {
    List,
    /// `kor routines add --every 1440 [--name N] [--dir D] <prompt…>`
    Add {
        #[arg(long)]
        every: u32,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        #[arg(required = true)]
        prompt: Vec<String>,
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
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `kor routines …`. The desktop app runs due routines while it is open.
pub fn routines(ctx: &Ctx, cmd: RoutineCmd) -> bool {
    use kor_runtime::routines as r;
    let l = &ctx.l;
    let Some(path) = r::file() else {
        eprintln!("{}", l.t("routines-none"));
        return false;
    };
    let result = match cmd {
        RoutineCmd::List => {
            let all = r::list(&path);
            if all.is_empty() {
                println!("{}", l.t("routines-none"));
            }
            for t in all {
                println!(
                    "{}  {}  {} min  {}  {}",
                    t["id"].as_str().unwrap_or_default(),
                    if t["enabled"] == true { "on " } else { "off" },
                    t["intervalMinutes"],
                    t["name"].as_str().unwrap_or_default(),
                    t["root"].as_str().unwrap_or_default(),
                );
            }
            Ok(true)
        }
        RoutineCmd::Add {
            every,
            name,
            dir,
            prompt,
        } => {
            let root = dir
                .or_else(|| std::env::current_dir().ok())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            let input = json!({
                "name": name.unwrap_or_default(),
                "prompt": prompt.join(" "),
                "intervalMinutes": every,
                "root": root,
            });
            let map = input.as_object().cloned().unwrap_or_default();
            r::add(&path, &map, now_ms()).map(|t| {
                println!(
                    "{}",
                    l.t_args(
                        "routine-added",
                        &args!("id" => t["id"].as_str().unwrap_or_default().to_owned())
                    )
                );
                true
            })
        }
        RoutineCmd::Remove { id } => r::remove(&path, &id),
        RoutineCmd::Enable { id } => r::set_enabled(&path, &id, true, now_ms()),
        RoutineCmd::Disable { id } => r::set_enabled(&path, &id, false, now_ms()),
    };
    match result {
        Ok(true) => true,
        Ok(false) => {
            eprintln!("{}", l.t("routine-not-found"));
            false
        }
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            false
        }
    }
}

/// Kor's MCP locations for the current project.
fn mcp_loc() -> Option<kor_agent::mcp_config::Locations> {
    let root = project_root();
    Some(kor_agent::mcp_config::Locations {
        kor_file: kor_config::user_dir()?.join("mcp.json"),
        home: dirs::home_dir(),
        trusted: kor_runtime::is_trusted(&root),
        root,
    })
}

/// `kor mcp add <id> [--url U] [--env K=V]… [--label L] [--disabled] -- <command> [args…]`.
pub fn mcp_add(
    ctx: &Ctx,
    id: &str,
    url: Option<&str>,
    env: &[String],
    label: Option<String>,
    disabled: bool,
    command: &[String],
) -> bool {
    let mut vars = serde_json::Map::new();
    for kv in env {
        let Some((k, v)) = kv.split_once('=') else {
            eprintln!(
                "{}",
                ctx.l.t_args("mcp-bad-env", &args!("value" => kv.clone()))
            );
            return false;
        };
        vars.insert(k.to_owned(), json!(v));
    }
    let cfg = match (url, command.split_first()) {
        (Some(u), _) => json!({"transport": "http", "url": u, "headers": vars}),
        (None, Some((cmd, rest))) => {
            json!({"transport": "stdio", "command": cmd, "args": rest, "env": vars})
        }
        (None, None) => {
            eprintln!("{}", ctx.l.t("mcp-need-command"));
            return false;
        }
    };
    let mut cfg = cfg;
    cfg["id"] = json!(id);
    cfg["enabled"] = json!(!disabled);
    if let Some(l) = label {
        cfg["label"] = json!(l);
    }
    mcp_result(
        ctx,
        mcp_loc()
            .ok_or_else(no_dir)
            .and_then(|loc| kor_agent::mcp_config::save(&loc, &cfg)),
    )
}

fn no_dir() -> String {
    "user configuration folder not found".into()
}

pub fn mcp_remove(ctx: &Ctx, id: &str) -> bool {
    mcp_result(
        ctx,
        mcp_loc()
            .ok_or_else(no_dir)
            .and_then(|loc| kor_agent::mcp_config::remove(&loc, id)),
    )
}

pub fn mcp_set_enabled(ctx: &Ctx, id: &str, enabled: bool) -> bool {
    mcp_result(
        ctx,
        mcp_loc()
            .ok_or_else(no_dir)
            .and_then(|loc| kor_agent::mcp_config::set_enabled(&loc, id, enabled)),
    )
}

fn mcp_result(ctx: &Ctx, r: Result<(), String>) -> bool {
    match r {
        Ok(()) => {
            println!("{}", ctx.l.t("mcp-saved"));
            true
        }
        Err(e) => {
            eprintln!("{}", ctx.l.t_args("error-generic", &args!("error" => e)));
            false
        }
    }
}
