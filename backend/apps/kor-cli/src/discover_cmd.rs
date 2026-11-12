//! `kor discover [user@host[:port] | connection]` (spec §15.16, ADR 0044): read-only scan of a
//! server over SSH, saved as `<kor home>/infra/<host>.md` (+ `.json`) and linked into the current
//! project's graph (`.kor/graph/infra/<host>.json`, `RunsOn` edges). Steps that lack permissions
//! are offered under `sudo -n` one by one (`--sudo docker,ports` or `--sudo all` answers ahead,
//! for scheduled runs; without a terminal and without `--sudo` they are skipped).

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use kor_agent::remote::SshHost;
use kor_agent::remote::discover::{self, SudoOffer};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, Clone, clap::Args)]
pub struct DiscoverArgs {
    /// `user@host[:port]`, an `~/.ssh/config` alias or a connection id.
    pub target: Option<String>,
    /// Steps allowed under `sudo -n` without asking (`docker,ports,...` or `all`).
    #[arg(long, value_delimiter = ',')]
    pub sudo: Vec<String>,
    /// Do not link the result into the current project's graph.
    #[arg(long)]
    pub no_link: bool,
    /// Print the snapshot as JSON.
    #[arg(long)]
    pub json: bool,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-discover"))
        .mut_arg("target", |a| a.help(l.t("arg-discover-target")))
        .mut_arg("sudo", |a| a.help(l.t("arg-discover-sudo")))
        .mut_arg("no_link", |a| a.help(l.t("arg-discover-no-link")))
        .mut_arg("json", |a| a.help(l.t("arg-discover-json")))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Ask on the terminal, or take the `--sudo` list.
fn sudo_answer(l: &Localizer, allowed: &[String], interactive: bool, o: &SudoOffer) -> bool {
    if allowed.iter().any(|s| s == "all" || s == &o.step) {
        return true;
    }
    if !interactive {
        return false;
    }
    eprint!(
        "{} ",
        l.t_args(
            "discover-sudo-ask",
            &args!("step" => o.step.clone(), "reason" => o.reason.clone(), "command" => o.command.clone())
        )
    );
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(
        line.trim().to_lowercase().as_str(),
        "y" | "yes" | "e" | "evet" | "o" | "oui"
    )
}

/// Scan `ssh`, save the report in Kor's home; returns the discovery.
pub async fn scan(
    l: &Localizer,
    ssh: &SshHost,
    address: &str,
    sudo: &[String],
    interactive: bool,
) -> Option<discover::Discovery> {
    let Some(host) = kor_graph::infra::host_id(&ssh.host) else {
        eprintln!(
            "{}",
            l.t_args(
                "error-generic",
                &args!("error" => format!("invalid host {}", ssh.host))
            )
        );
        return None;
    };
    eprintln!(
        "{}",
        l.t_args("discover-scanning", &args!("host" => address.to_owned()))
    );
    let sh = kor_agent::remote::SshShell::new(ssh);
    // The callback must be Send + Sync (the desktop runs it on its runtime): carry the language.
    let lang = l.lang();
    let ask = move |offer: &SudoOffer| sudo_answer(&Localizer::new(lang), sudo, interactive, offer);
    let found = match discover::discover(&sh, address, &host, now(), &ask).await {
        Ok(found) => found,
        Err(e) => {
            eprintln!("{}", l.t_args("discover-failed", &args!("error" => e)));
            return None;
        }
    };
    let snap = &found.snapshot;
    println!(
        "{}",
        l.t_args(
            "discover-summary",
            &args!("host" => address.to_owned(), "os" => snap.os.clone(), "services" => snap.services.len(),
                   "ports" => snap.ports.len(), "vhosts" => snap.vhosts.len())
        )
    );
    match kor_runtime::kor_home().map(|home| home.join("infra")) {
        Some(dir) => match discover::save(&dir, &found) {
            Ok((md, _)) => println!(
                "{}",
                l.t_args("discover-saved", &args!("path" => md.display().to_string()))
            ),
            Err(e) => eprintln!("{}", l.t_args("error-generic", &args!("error" => e))),
        },
        None => eprintln!(
            "{}",
            l.t_args("error-generic", &args!("error" => "no Kor data directory"))
        ),
    }
    for offer in &found.sudo_offered {
        println!(
            "{}",
            l.t_args(
                "discover-sudo-skipped",
                &args!("step" => offer.step.clone())
            )
        );
    }
    Some(found)
}

/// Link into the project at `root`: copy the snapshot and refresh the graph.
fn link(l: &Localizer, root: &Path, d: &discover::Discovery) -> bool {
    let r =
        kor_graph::infra::save(&kor_graph::infra::infra_dir(root), &d.snapshot).and_then(|_| {
            let mut g = kor_graph::Graph::open(root)?;
            g.update()?;
            g.refresh_infra()?;
            Ok(g.nodes()
                .filter(|n| n.kind == kor_graph::NodeKind::Service)
                .count())
        });
    match r {
        Ok(n) => {
            println!(
                "{}",
                l.t_args(
                    "discover-linked",
                    &args!("count" => n, "path" => root.display().to_string())
                )
            );
            true
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("graph-error", &args!("error" => e.to_string()))
            );
            false
        }
    }
}

/// A project to link into: the current folder when it has `.kor` or `.git`.
fn project_root() -> Option<PathBuf> {
    let root = crate::graph_cmd::root().ok()?;
    (root.join(".kor").is_dir() || root.join(".git").exists()).then_some(root)
}

/// `kor discover [target]`; `remote` is the session's `--remote` target.
pub fn run(ctx: &Ctx, a: &DiscoverArgs, remote: Option<&str>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if kor_runtime::privacy_mode() || ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode) {
        eprintln!(
            "{}",
            l.t_args("error-generic", &args!("error" => l.t("remote-privacy")))
        );
        return Ok(false);
    }
    let Some(target) = a.target.as_deref().or(remote) else {
        eprintln!("{}", l.t("discover-no-target"));
        return Ok(false);
    };
    let known = discover::step_ids();
    if let Some(bad) = a
        .sudo
        .iter()
        .find(|s| *s != "all" && !known.contains(s.as_str()))
    {
        eprintln!(
            "{}",
            l.t_args("discover-bad-step", &args!("step" => bad.clone(), "steps" => known.into_iter().collect::<Vec<_>>().join(", ")))
        );
        return Ok(false);
    }
    let file = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let h = match kor_agent::remote::resolve_target(target, file.as_deref()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            return Ok(false);
        }
    };
    let address = kor_agent::remote::display_target(&h);
    let rt = crate::providers::runtime()?;
    let interactive = std::io::stdin().is_terminal();
    let Some(d) = rt.block_on(scan(l, &h, &address, &a.sudo, interactive)) else {
        return Ok(false);
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&d.snapshot)?);
    }
    if !a.no_link
        && let Some(root) = project_root()
    {
        return Ok(link(l, &root, &d));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: DiscoverArgs,
    }

    #[test]
    fn args_parse() {
        let t = T::try_parse_from([
            "discover",
            "root@pve:2222",
            "--sudo",
            "docker,proxmox",
            "--no-link",
        ])
        .unwrap();
        assert_eq!(t.a.target.as_deref(), Some("root@pve:2222"));
        assert_eq!(t.a.sudo, ["docker", "proxmox"]);
        assert!(t.a.no_link && !t.a.json);
    }

    #[test]
    fn sudo_list_answers_without_a_terminal() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let o = SudoOffer {
            step: "docker".into(),
            command: "sudo -n sh -c 'x'".into(),
            reason: "r".into(),
        };
        assert!(sudo_answer(&l, &["docker".into()], false, &o));
        assert!(sudo_answer(&l, &["all".into()], false, &o));
        assert!(!sudo_answer(&l, &["ports".into()], false, &o));
        assert!(!sudo_answer(&l, &[], false, &o));
    }

    #[test]
    fn linking_puts_the_host_into_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("shop");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("package.json"), "{\"name\":\"shop\"}").unwrap();
        let d = discover::Discovery {
            snapshot: kor_graph::InfraSnapshot {
                host: "web1".into(),
                services: vec![kor_graph::infra::Service {
                    kind: "docker".into(),
                    name: "shop-1".into(),
                    image: Some("acme/shop:1".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            system: kor_health::system::Profile::default(),
            sudo_offered: Vec::new(),
            sudo_used: Vec::new(),
        };
        assert!(link(&Localizer::new(kor_i18n::Lang::En), &root, &d));
        let g = kor_graph::Graph::open(&root).unwrap();
        assert!(g.node("service:web1/docker/shop-1").is_some());
    }
}
