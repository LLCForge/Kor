//! `kor tool list|describe|search|add|share|pull` (§26.2, ADR 0056): the PATH tool catalog and
//! typed tool definitions. Logic in `kor_agent::clitools` and `kor_runtime::clitools`.

use std::path::PathBuf;

use kor_agent::clitools::{self, Catalog, defs, help};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum ToolCmd {
    /// Programs found on PATH (with versions) and your typed tool definitions.
    List {
        /// Scan PATH again now.
        #[arg(long)]
        refresh: bool,
        /// Also programs without a description.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// A program's help, summarized (cached).
    Describe {
        name: String,
        #[arg(long)]
        refresh: bool,
        /// Add a summary written by the model (`agent.model`, never in privacy mode unless local).
        #[arg(long)]
        model: bool,
        /// Save a typed definition generated from the help to `tools/generated/`.
        #[arg(long)]
        generate: bool,
        #[arg(long)]
        json: bool,
    },
    /// Find tools for a task (`kor tool search convert video`).
    Search {
        #[arg(required = true)]
        query: Vec<String>,
    },
    /// Define a typed tool from a template: `kor tool add thumbs --command "ffmpeg -i {input} {output}"`.
    Add {
        name: String,
        #[arg(long)]
        command: String,
        #[arg(long, default_value = "write", value_parser = ["read", "write", "network", "destructive"])]
        safety: String,
        #[arg(long, default_value = "")]
        description: String,
        /// Save for every project (Kor home) instead of this project's `.kor/tools/`.
        #[arg(long)]
        user: bool,
    },
    /// Send your tool definitions to the team server (`[team] server`).
    Share,
    /// Fetch the team's tool definitions.
    Pull,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-tool"))
        .mut_subcommand("list", |s| {
            s.about(l.t("cmd-tool-list"))
                .mut_arg("refresh", |a| a.help(l.t("arg-tool-refresh")))
                .mut_arg("all", |a| a.help(l.t("arg-tool-all")))
                .mut_arg("json", |a| a.help(l.t("arg-tool-json")))
        })
        .mut_subcommand("describe", |s| {
            s.about(l.t("cmd-tool-describe"))
                .mut_arg("name", |a| a.help(l.t("arg-tool-name")))
                .mut_arg("refresh", |a| a.help(l.t("arg-tool-refresh")))
                .mut_arg("model", |a| a.help(l.t("arg-tool-model")))
                .mut_arg("generate", |a| a.help(l.t("arg-tool-generate")))
                .mut_arg("json", |a| a.help(l.t("arg-tool-json")))
        })
        .mut_subcommand("search", |s| {
            s.about(l.t("cmd-tool-search"))
                .mut_arg("query", |a| a.help(l.t("arg-tool-query")))
        })
        .mut_subcommand("add", |s| {
            s.about(l.t("cmd-tool-add"))
                .mut_arg("name", |a| a.help(l.t("arg-tool-name")))
                .mut_arg("command", |a| a.help(l.t("arg-tool-command")))
                .mut_arg("safety", |a| a.help(l.t("arg-tool-safety")))
                .mut_arg("description", |a| a.help(l.t("arg-tool-description")))
                .mut_arg("user", |a| a.help(l.t("arg-tool-user")))
        })
        .mut_subcommand("share", |s| s.about(l.t("cmd-tool-share")))
        .mut_subcommand("pull", |s| s.about(l.t("cmd-tool-pull")))
}

fn home(l: &Localizer) -> anyhow::Result<PathBuf> {
    kor_runtime::clitools::tools_home().ok_or_else(|| anyhow::anyhow!(l.t("tool-no-home")))
}

fn permits(ctx: &Ctx) -> kor_config::ToolsConfig {
    ctx.config
        .as_ref()
        .map(|c| c.config.tools.clone())
        .unwrap_or_default()
}

pub fn run(ctx: &Ctx, cmd: &ToolCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let rt = crate::providers::runtime()?;
    let root = crate::graph_cmd::root()?;
    let trusted = kor_runtime::is_trusted(&root);
    let dirs = kor_runtime::clitools::dirs(&root, trusted);
    match cmd {
        ToolCmd::List { refresh, all, json } => {
            let home = home(l)?;
            let mut cat = rt.block_on(Catalog::load(&home, *refresh));
            let cfg = permits(ctx);
            cat.filter(|n| cfg.permits(n));
            let (loaded, errors) = defs::load(&dirs);
            if *json {
                let typed: Vec<_> = loaded.iter().map(|x| serde_json::json!({"name": x.def.name, "origin": x.origin, "safety": x.def.safety, "command": x.def.command, "description": x.def.description})).collect();
                println!(
                    "{}",
                    serde_json::json!({"catalog": cat, "typed": typed, "errors": errors})
                );
                return Ok(true);
            }
            let shown: Vec<_> = cat
                .tools
                .iter()
                .filter(|t| *all || t.known || !t.description.is_empty())
                .collect();
            println!("{}", l.t_args("tool-list-head", &args!("shown" => shown.len(), "total" => cat.tools.len(), "when" => kor_runtime::cost::fmt_time(cat.generated_at))));
            for t in shown {
                let v = t.version.as_deref().unwrap_or("");
                let net = if t.network { " ⇄" } else { "" };
                println!("  {:<14} {:<12} {}{net}", t.name, v, t.description);
            }
            if !loaded.is_empty() {
                println!("\n{}", l.t("tool-typed-head"));
                for x in &loaded {
                    println!(
                        "  {:<18} [{:?}/{:?}] {}",
                        x.def.name,
                        x.origin,
                        x.def.safety,
                        defs::display(&x.def.command)
                    );
                }
            }
            for e in errors {
                eprintln!("{}", l.t_args("tool-def-error", &args!("error" => e)));
            }
            if !trusted && root.join(".kor").join("tools").is_dir() {
                eprintln!("{}", l.t("tool-untrusted"));
            }
            Ok(true)
        }
        ToolCmd::Describe {
            name,
            refresh,
            model,
            generate,
            json,
        } => {
            if !permits(ctx).permits(name) {
                eprintln!(
                    "{}",
                    l.t_args("tool-denied", &args!("name" => name.clone()))
                );
                return Ok(false);
            }
            let home = home(l)?;
            let tctx = kor_tools::ToolCtx::new(&root)?;
            let mut h = match rt.block_on(help::describe(&home.join("help"), name, *refresh, &tctx))
            {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                    return Ok(false);
                }
            };
            if *model {
                let loaded = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
                match kor_runtime::oneshot_provider(l, loaded, None) {
                    Ok((p, m)) => match rt.block_on(help::model_summary(&p, &m, &h)) {
                        Ok(s) => {
                            h.model_summary = Some(s);
                            let _ = help::store(&home.join("help"), &h);
                        }
                        Err(e) => eprintln!("{}", l.t_args("error-generic", &args!("error" => e))),
                    },
                    Err(e) => eprintln!("{e}"),
                }
            }
            if *generate {
                let d = help::generate(&h);
                let dir = home.join("generated");
                std::fs::create_dir_all(&dir)?;
                let f = dir.join(format!("{}.toml", d.name));
                std::fs::write(&f, toml::to_string_pretty(&d)?)?;
                eprintln!(
                    "{}",
                    l.t_args(
                        "tool-generated",
                        &args!("name" => d.name.clone(), "path" => f.display().to_string())
                    )
                );
            }
            if *json {
                println!("{}", serde_json::to_string_pretty(&h)?);
            } else {
                print!("{}", help::render(&h));
            }
            Ok(true)
        }
        ToolCmd::Search { query } => {
            let home = home(l)?;
            let cfg = permits(ctx);
            let mut cat = rt.block_on(Catalog::load(&home, false));
            cat.filter(|n| cfg.permits(n));
            let (loaded, _) = defs::load(&dirs);
            let privacy = kor_runtime::privacy_mode();
            let hits = clitools::search::keyword_hits(&cat, &loaded, &query.join(" "), privacy);
            if hits.is_empty() {
                println!("{}", l.t("tool-search-none"));
                return Ok(false);
            }
            for h in hits {
                println!("  {}", h.line);
            }
            Ok(true)
        }
        ToolCmd::Add {
            name,
            command,
            safety,
            description,
            user,
        } => {
            let safety: defs::Safety =
                serde_json::from_value(serde_json::Value::String(safety.clone()))?;
            let f = match kor_runtime::clitools::add(
                &root,
                name,
                command,
                safety,
                description,
                *user,
            ) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                    return Ok(false);
                }
            };
            println!(
                "{}",
                l.t_args(
                    "tool-added",
                    &args!("name" => name.clone(), "path" => f.display().to_string())
                )
            );
            if !*user && !trusted {
                eprintln!("{}", l.t("tool-untrusted"));
            }
            Ok(true)
        }
        ToolCmd::Share => {
            let loaded = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            match rt.block_on(kor_runtime::clitools::share(&loaded.config, &root, trusted)) {
                Ok(n) => {
                    println!("{}", l.t_args("tool-shared", &args!("count" => n)));
                    Ok(true)
                }
                Err(e) => {
                    eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                    Ok(false)
                }
            }
        }
        ToolCmd::Pull => {
            let loaded = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            match rt.block_on(kor_runtime::clitools::pull(&loaded.config)) {
                Ok((ok, rejected)) => {
                    println!(
                        "{}",
                        l.t_args(
                            "tool-pulled",
                            &args!("count" => ok.len(), "names" => ok.join(", "))
                        )
                    );
                    for r in rejected {
                        eprintln!("{}", l.t_args("tool-def-error", &args!("error" => r)));
                    }
                    Ok(true)
                }
                Err(e) => {
                    eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                    Ok(false)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory as _, Parser};

    use super::*;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: ToolCmd,
    }

    #[test]
    fn parses_and_localizes() {
        let c = Cli::parse_from([
            "x",
            "add",
            "thumbs",
            "--command",
            "ffmpeg -i {input} {output}",
            "--user",
        ]);
        assert!(matches!(c.cmd, ToolCmd::Add { ref safety, user: true, .. } if safety == "write"));
        assert!(
            Cli::try_parse_from(["x", "add", "t", "--command", "x", "--safety", "root"]).is_err()
        );
        assert!(matches!(
            Cli::parse_from(["x", "describe", "git", "--generate"]).cmd,
            ToolCmd::Describe { generate: true, .. }
        ));
        assert!(Cli::try_parse_from(["x", "search"]).is_err());
        let l = Localizer::new(kor_i18n::Lang::Fr);
        let cmd = localize(
            clap::Command::new("tool").subcommands(Cli::command().get_subcommands().cloned()),
            &l,
        );
        assert!(cmd.get_subcommands().any(|s| s.get_name() == "pull"));
        for key in [
            "cmd-tool",
            "cmd-tool-describe",
            "tool-list-head",
            "tool-untrusted",
            "arg-tool-safety",
        ] {
            assert_ne!(l.t(key), key, "missing fr string {key}");
        }
    }
}
