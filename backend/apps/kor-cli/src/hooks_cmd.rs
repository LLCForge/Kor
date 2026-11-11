//! `kor hooks list|add|edit|rm|test|log` (§15.1, §28.12, ADR 0063): every hook this project runs,
//! from Kor's `hooks.toml` files and Claude Code `settings.json`, with the Kor event each Claude
//! event covers; and the same editing the desktop's visual hook editor does (logic:
//! `kor_runtime::hooks_edit`).

use clap::{Args, Subcommand};
use kor_i18n::{Localizer, args};
use kor_runtime::hooks_edit::{self as he, Format, HookFile, Item, Sample};
use serde_json::Value;

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum HooksCmd {
    List,
    /// Add a hook.
    Add(HookArgs),
    /// Replace hook <id> of <event> (ids from `kor hooks list --ids`).
    Edit {
        /// Current event of the hook.
        old_event: String,
        id: String,
        #[command(flatten)]
        hook: HookArgs,
    },
    /// Remove hook <id> of <event>.
    Rm {
        event: String,
        id: String,
        #[arg(long = "in", default_value = "kor-project")]
        target: String,
    },
    /// Dry-run: would the hook run for this sample, and (with --run) what it returns.
    Test {
        event: String,
        id: String,
        #[arg(long = "in", default_value = "kor-project")]
        target: String,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// Really run the command / MCP call with a sample payload.
        #[arg(long)]
        run: bool,
    },
    /// Recent hook runs.
    Log {
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
    },
}

#[derive(Debug, Args)]
pub struct HookArgs {
    /// pre-edit, post-edit, pre-command, post-command, pre-commit, on-task-start, on-task-done,
    /// on-approval-needed, on-budget-alert, on-error (Claude files: `PreToolUse`, `PostToolUse`, Stop…).
    pub event: String,
    #[arg(long, conflicts_with_all = ["mcp", "notify"])]
    pub command: Option<String>,
    /// `server/tool`.
    #[arg(long, conflicts_with = "notify")]
    pub mcp: Option<String>,
    /// JSON object for the MCP tool.
    #[arg(long, requires = "mcp")]
    pub args: Option<String>,
    #[arg(long)]
    pub notify: Option<String>,
    #[arg(long)]
    pub matcher: Option<String>,
    #[arg(long = "files")]
    pub files: Vec<String>,
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub timeout: Option<u64>,
    #[arg(long)]
    pub disabled: bool,
    /// kor-project (default), kor-user, claude-project, claude-local, claude-user.
    #[arg(long = "in", default_value = "kor-project")]
    pub target: String,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-hooks"))
        .mut_subcommand("list", |s| s.about(l.t("cmd-hooks-list")))
        .mut_subcommand("add", |s| s.about(l.t("cmd-hooks-add")))
        .mut_subcommand("edit", |s| s.about(l.t("cmd-hooks-edit")))
        .mut_subcommand("rm", |s| s.about(l.t("cmd-hooks-rm")))
        .mut_subcommand("test", |s| {
            s.about(l.t("cmd-hooks-test"))
                .mut_arg("run", |a| a.help(l.t("arg-hooks-run")))
        })
        .mut_subcommand("log", |s| s.about(l.t("cmd-hooks-log")))
}

fn all_files(root: &std::path::Path) -> Vec<HookFile> {
    he::files(
        root,
        dirs::home_dir().as_deref(),
        kor_config::user_dir().as_deref(),
    )
}

/// The file behind `--in`.
fn target(files: &[HookFile], name: &str) -> Option<HookFile> {
    let (format, scope) = name.split_once('-')?;
    let format = match format {
        "kor" => Format::Kor,
        "claude" => Format::Claude,
        _ => return None,
    };
    files
        .iter()
        .find(|f| f.format == format && f.scope == scope)
        .cloned()
}

fn item(h: &HookArgs) -> Item {
    Item {
        event: h.event.clone(),
        command: h.command.clone(),
        mcp: h.mcp.clone(),
        args: h.args.clone(),
        notify: h.notify.clone(),
        matcher: h.matcher.clone(),
        files: h.files.clone(),
        branch: h.branch.clone(),
        model: h.model.clone(),
        timeout: h.timeout,
        disabled: h.disabled,
        ..Item::default()
    }
}

fn list(ctx: &Ctx, root: &std::path::Path) {
    let l = &ctx.l;
    let c = kor_runtime::load_compat(root);
    let user = kor_config::user_dir().map_or_else(String::new, |d| {
        kor_tools::display_path(&d.join("hooks.toml"))
    });
    let project = kor_tools::display_path(&root.join(".kor").join("hooks.toml"));
    println!(
        "{}",
        l.t_args("hooks-files", &args!("user" => user, "project" => project))
    );
    println!(
        "{}",
        l.t_args("hooks-count", &args!("count" => c.hooks.len()))
    );
    for h in &c.hooks {
        let matcher = h
            .matcher
            .as_deref()
            .map(|m| format!(" [{m}]"))
            .unwrap_or_default();
        let kor = kor_agent::hooks::kor_events(&h.event, None, &Value::Null);
        let alias = if kor.is_empty() {
            String::new()
        } else {
            format!(" → {}", kor.join(", "))
        };
        let cond = h
            .kor
            .as_ref()
            .map(|k| {
                let mut v = Vec::new();
                if !k.files.is_empty() {
                    v.push(format!("files={}", k.files.join(",")));
                }
                if let Some(b) = &k.branch {
                    v.push(format!("branch={b}"));
                }
                if let Some(m) = &k.model {
                    v.push(format!("model={m}"));
                }
                if k.disabled {
                    v.push("disabled".into());
                }
                if v.is_empty() {
                    String::new()
                } else {
                    format!(" {{{}}}", v.join(" "))
                }
            })
            .unwrap_or_default();
        println!(
            "  {}{matcher}{alias}{cond}: {}  ({:?}/{:?}, {})",
            h.event,
            h.command,
            h.source.eco,
            h.source.scope,
            kor_tools::display_path(&h.source.path)
        );
    }
    if c.hooks.is_empty() {
        println!("  {}", l.t("hooks-none"));
    }
    for w in c.warnings.iter().filter(|w| w.contains("hooks")) {
        println!(
            "  {}",
            l.t_args("hooks-warning", &args!("text" => w.clone()))
        );
    }
    // Editable ids per file (`kor hooks edit|rm|test <event> <id> --in …`).
    let (items, _) = he::list(&all_files(root));
    if !items.is_empty() {
        println!("{}", l.t("hooks-ids"));
        for i in &items {
            let f = match i.format {
                Some(Format::Claude) => "claude",
                _ => "kor",
            };
            println!("  --in {f}-{}  {} {}", i.scope, i.event, i.id);
        }
    }
}

pub fn run(ctx: &Ctx, cmd: &HooksCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let files = all_files(&root);
    let fail = |e: String| {
        eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
        false
    };
    let pick = |name: &str| {
        target(&files, name)
            .ok_or_else(|| l.t_args("hooks-bad-target", &args!("target" => name.to_owned())))
    };
    match cmd {
        HooksCmd::List => {
            list(ctx, &root);
            Ok(true)
        }
        HooksCmd::Add(h) => {
            let f = match pick(&h.target) {
                Ok(f) => f,
                Err(e) => return Ok(fail(e)),
            };
            match he::save(&f, &item(h), None) {
                Ok(()) => {
                    println!(
                        "{}",
                        l.t_args(
                            "hooks-saved",
                            &args!("file" => kor_tools::display_path(&f.path))
                        )
                    );
                    Ok(true)
                }
                Err(e) => Ok(fail(e)),
            }
        }
        HooksCmd::Edit {
            old_event,
            id,
            hook,
        } => {
            let f = match pick(&hook.target) {
                Ok(f) => f,
                Err(e) => return Ok(fail(e)),
            };
            match he::save(&f, &item(hook), Some((old_event, id))) {
                Ok(()) => {
                    println!(
                        "{}",
                        l.t_args(
                            "hooks-saved",
                            &args!("file" => kor_tools::display_path(&f.path))
                        )
                    );
                    Ok(true)
                }
                Err(e) => Ok(fail(e)),
            }
        }
        HooksCmd::Rm {
            event,
            id,
            target: t,
        } => {
            let f = match pick(t) {
                Ok(f) => f,
                Err(e) => return Ok(fail(e)),
            };
            match he::remove(&f, event, id) {
                Ok(()) => {
                    println!("{}", l.t("hooks-removed"));
                    Ok(true)
                }
                Err(e) => Ok(fail(e)),
            }
        }
        HooksCmd::Test {
            event,
            id,
            target: t,
            tool,
            file,
            model,
            branch,
            run,
        } => {
            let f = match pick(t) {
                Ok(f) => f,
                Err(e) => return Ok(fail(e)),
            };
            let (items, _) = he::list(std::slice::from_ref(&f));
            let Some(it) = items.into_iter().find(|i| i.event == *event && i.id == *id) else {
                return Ok(fail(l.t_args(
                    "hooks-unknown",
                    &args!("event" => event.clone(), "id" => id.clone()),
                )));
            };
            let sample = Sample {
                tool: tool.clone(),
                file: file.clone(),
                model: model.clone(),
                branch: branch.clone(),
                prompt: None,
            };
            let shell = kor_tools::Shell::detect();
            let d = crate::providers::runtime()?
                .block_on(he::dry_run(&root, shell, &it, &sample, *run));
            if d.matches {
                println!("{}", l.t("hooks-would-run"));
            } else {
                println!(
                    "{}",
                    l.t_args("hooks-would-skip", &args!("why" => d.skipped.join("; ")))
                );
            }
            if !d.kor_events.is_empty() {
                println!("  → {}", d.kor_events.join(", "));
            }
            if d.executed {
                println!("{}", l.t_args("hooks-ran", &args!("ms" => d.ms)));
                if let Some(b) = &d.blocked {
                    println!("  ✗ {b}");
                }
                for c in &d.context {
                    println!("  + {c}");
                }
                for n in &d.notices {
                    println!("  ! {n}");
                }
            }
            Ok(true)
        }
        HooksCmd::Log { limit } => {
            let rows = he::log(*limit);
            if rows.is_empty() {
                println!("{}", l.t("hooks-log-none"));
            }
            for r in rows {
                let at = r["at"]
                    .as_i64()
                    .map(kor_memory::kb::date)
                    .unwrap_or_default();
                println!(
                    "{at}  {:<8} {} {}  {}ms  {}",
                    r["result"].as_str().unwrap_or(""),
                    r["event"].as_str().unwrap_or(""),
                    r["command"].as_str().unwrap_or(""),
                    r["ms"].as_u64().unwrap_or(0),
                    r["message"].as_str().unwrap_or("")
                );
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: HooksCmd,
    }

    #[test]
    fn parses() {
        let c = Cli::try_parse_from([
            "h",
            "add",
            "post-edit",
            "--command",
            "cargo fmt",
            "--files",
            "src/**",
            "--branch",
            "main",
        ])
        .unwrap();
        let HooksCmd::Add(a) = c.cmd else { panic!() };
        assert_eq!(
            (a.event.as_str(), a.files.len(), a.target.as_str()),
            ("post-edit", 1, "kor-project")
        );
        assert!(Cli::try_parse_from(["h", "add", "x", "--command", "a", "--notify", "b"]).is_err());
        assert!(
            Cli::try_parse_from(["h", "add", "x", "--args", "{}"]).is_err(),
            "args needs mcp"
        );
        assert!(matches!(
            Cli::try_parse_from(["h", "test", "pre-edit", "0", "--run"])
                .unwrap()
                .cmd,
            HooksCmd::Test { run: true, .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["h", "edit", "pre-edit", "0", "post-edit", "--notify", "x"])
                .unwrap()
                .cmd,
            HooksCmd::Edit { .. }
        ));
        let files = he::files(std::path::Path::new("/p"), None, None);
        assert!(
            target(&files, "kor-project").is_some() && target(&files, "claude-local").is_some()
        );
        assert!(target(&files, "nope").is_none());
        let l = Localizer::new(kor_i18n::Lang::En);
        let _ = localize(<Cli as clap::CommandFactory>::command(), &l);
    }
}
