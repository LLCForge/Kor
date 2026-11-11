//! `kor acp` (spec §28.8, ADR 0057): Kor as an Agent Client Protocol agent for editors (Zed,
//! `JetBrains` IDEs, Neovim, Emacs …) on stdin/stdout, and as an ACP client that runs another ACP agent
//! as a sub-agent (`kor acp run`). The protocol lives in `kor_runtime::acp`.

use std::io::{IsTerminal as _, Write as _};
use std::sync::Arc;

use async_trait::async_trait;
use kor_i18n::{Localizer, args};
use kor_runtime::acp::client::{AcpClient, AgentSpec, ClientUi};
use serde_json::Value;

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct AcpArgs {
    #[command(subcommand)]
    command: Option<AcpCmd>,
}

#[derive(Debug, clap::Subcommand)]
pub enum AcpCmd {
    /// Serve an editor on stdin/stdout (what `kor acp` alone does).
    Serve,
    /// Run an ACP agent as a sub-agent with a prompt.
    Run {
        /// A name from `[acp.agents]`, or the program to start.
        agent: String,
        prompt: Vec<String>,
        /// Arguments for an ad-hoc program (repeatable).
        #[arg(long = "arg", allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// The ACP agents in the configuration.
    Agents,
    /// How to add Kor to an editor (Zed settings).
    Setup,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-acp"))
        .mut_subcommand("serve", |s| s.about(l.t("cmd-acp-serve")))
        .mut_subcommand("run", |s| {
            s.about(l.t("cmd-acp-run"))
                .mut_arg("agent", |a| a.help(l.t("arg-acp-agent")))
                .mut_arg("prompt", |a| a.help(l.t("arg-acp-prompt")))
                .mut_arg("args", |a| a.help(l.t("arg-acp-arg")))
        })
        .mut_subcommand("agents", |s| s.about(l.t("cmd-acp-agents")))
        .mut_subcommand("setup", |s| s.about(l.t("cmd-acp-setup")))
}

/// `kor acp` / `kor acp serve` owns stdin/stdout: never a slash command.
pub fn serves_stdio(a: &AcpArgs) -> bool {
    matches!(a.command, None | Some(AcpCmd::Serve))
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

/// The terminal side of a sub-agent: its text streams to stdout, its permission requests are
/// asked here (no terminal: rejected).
struct TerminalUi {
    l: Localizer,
}

#[async_trait]
impl ClientUi for TerminalUi {
    fn update(&self, u: &Value) {
        let s = |k: &str| u[k].as_str().unwrap_or_default().to_owned();
        match u["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                print!("{}", u["content"]["text"].as_str().unwrap_or_default());
                let _ = std::io::stdout().flush();
            }
            Some("tool_call") => println!("\n> {} ({})", s("title"), s("kind")),
            Some("tool_call_update") if !s("status").is_empty() => println!("  {}", s("status")),
            _ => {}
        }
    }

    async fn permission(&self, tool_call: &Value, options: &[Value]) -> Option<String> {
        let reject = options
            .iter()
            .find(|o| o["kind"] == "reject_once")
            .and_then(|o| o["optionId"].as_str().map(str::to_owned));
        if !std::io::stdin().is_terminal() {
            return reject;
        }
        let title = tool_call["title"].as_str().unwrap_or_default().to_owned();
        eprintln!("\n{}", self.l.t_args("acp-ask", &args!("title" => title)));
        for (i, o) in options.iter().enumerate() {
            eprintln!("  {}) {}", i + 1, o["name"].as_str().unwrap_or_default());
        }
        let line = tokio::task::spawn_blocking(|| {
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
            s
        })
        .await
        .unwrap_or_default();
        line.trim()
            .parse::<usize>()
            .ok()
            .and_then(|n| options.get(n.checked_sub(1)?))
            .and_then(|o| o["optionId"].as_str().map(str::to_owned))
            .or(reject)
    }
}

pub fn run(ctx: &Ctx, a: AcpArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let loaded = match ctx.config.as_ref() {
        Ok(c) => c.clone(),
        Err(e) => return Ok(fail(l, e)),
    };
    kor_runtime::apply_network_policy(l, &loaded.config);
    let rt = crate::providers::runtime()?;
    match a.command.unwrap_or(AcpCmd::Serve) {
        AcpCmd::Serve => {
            rt.block_on(kor_runtime::acp::serve_stdio(
                Localizer::new(l.lang()),
                loaded,
            ));
            Ok(true)
        }
        AcpCmd::Agents => {
            if loaded.config.acp.agents.is_empty() {
                println!("{}", l.t("acp-no-agents"));
            }
            for (name, s) in &loaded.config.acp.agents {
                println!("{name:<16} {} {}", s.command, s.args.join(" "));
            }
            Ok(true)
        }
        AcpCmd::Setup => {
            let exe = std::env::current_exe()?;
            let v = kor_runtime::acp::editor_setup(&exe);
            println!("{}", l.t("acp-setup-intro"));
            println!(
                "{}",
                serde_json::to_string_pretty(&v["zed"]).unwrap_or_default()
            );
            Ok(true)
        }
        AcpCmd::Run {
            agent,
            prompt,
            args,
        } => {
            let spec = loaded
                .config
                .acp
                .agents
                .get(&agent)
                .cloned()
                .unwrap_or_else(|| AgentSpec {
                    command: agent.clone(),
                    args,
                    env: std::collections::BTreeMap::new(),
                });
            let text = prompt.join(" ");
            if text.trim().is_empty() {
                return Ok(fail(l, l.t("acp-empty-prompt")));
            }
            let root = std::env::current_dir()?;
            let ui = Arc::new(TerminalUi {
                l: Localizer::new(l.lang()),
            });
            rt.block_on(async {
                let c = match AcpClient::spawn(&agent, &spec, &root, ui).await {
                    Ok(c) => c,
                    Err(e) => return Ok(fail(l, e)),
                };
                eprintln!(
                    "{}",
                    l.t_args("acp-started", &args!("agent" => c.agent_info["name"].as_str().unwrap_or(&agent).to_owned()))
                );
                match c.prompt(&text).await {
                    Ok(reason) => {
                        println!("\n[{reason}]");
                        Ok(true)
                    }
                    Err(e) => Ok(fail(l, e)),
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: AcpArgs,
    }

    #[test]
    fn args_parse() {
        assert!(T::try_parse_from(["acp"]).unwrap().a.command.is_none());
        assert!(matches!(
            T::try_parse_from(["acp", "serve"]).unwrap().a.command,
            Some(AcpCmd::Serve)
        ));
        let t = T::try_parse_from([
            "acp",
            "run",
            "gemini",
            "explain",
            "this",
            "--arg",
            "--experimental-acp",
        ])
        .unwrap();
        assert!(
            matches!(t.a.command, Some(AcpCmd::Run { ref agent, ref args, .. }) if agent == "gemini" && args.len() == 1)
        );
    }
}
