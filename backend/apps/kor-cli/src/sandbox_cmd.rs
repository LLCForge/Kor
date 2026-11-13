//! `kor sandbox status|test|pull` (§6.4, ADR 0033): what confines agent commands here, a safe
//! self-check, and the explicit Docker image download. Logic: `kor_runtime::sandbox`, `kor_sandbox`.

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_sandbox::{Filter, SandboxKind};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum SandboxCmd {
    /// Backend, mode, limits and network allowlist.
    Status,
    /// Try a write outside a throwaway project and report whether it was blocked.
    Test,
    /// Download the Docker sandbox image (Kor never pulls it on its own).
    Pull,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-sandbox"))
        .mut_subcommand("status", |s| s.about(l.t("cmd-sandbox-status")))
        .mut_subcommand("test", |s| s.about(l.t("cmd-sandbox-test")))
        .mut_subcommand("pull", |s| s.about(l.t("cmd-sandbox-pull")))
}

pub fn run(ctx: &Ctx, cmd: &SandboxCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let cfg = ctx
        .config
        .as_ref()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .config
        .sandbox
        .clone();
    let st = kor_runtime::sandbox::status(l, &cfg);
    match cmd {
        SandboxCmd::Status => {
            status(l, &st);
            Ok(true)
        }
        SandboxCmd::Test => test(l, &st),
        SandboxCmd::Pull => {
            let image = &st.setup.options.image;
            println!(
                "{}",
                l.t_args("sandbox-pull-start", &args!("image" => image.clone()))
            );
            let out = kor_sandbox::docker_pull(image, true)?;
            let key = if out.status.success() {
                "sandbox-pull-done"
            } else {
                "sandbox-pull-failed"
            };
            println!("{}", l.t_args(key, &args!("image" => image.clone())));
            Ok(out.status.success())
        }
    }
}

fn status(l: &Localizer, st: &kor_runtime::sandbox::Status) {
    let s = &st.setup;
    let e = &st.enforcement;
    let backend = s.kind.as_str();
    println!(
        "{}",
        l.t_args(
            "sandbox-status-head",
            &args!(
                "mode" => st.mode.as_str(),
                "backend" => backend,
                "pref" => kor_runtime::sandbox::preference_str(st.preference)
            )
        )
    );
    let state = |on: bool| {
        if on {
            l.t("sandbox-state-on")
        } else {
            l.t_args("sandbox-state-off", &args!("backend" => backend))
        }
    };
    let files = if e.files {
        "sandbox-status-files"
    } else {
        "sandbox-status-files-none"
    };
    println!("  {}", l.t(files));
    let net = if !s.network {
        l.t("sandbox-status-net-off")
    } else if s.options.allow.is_empty() {
        l.t("sandbox-status-net-all")
    } else {
        let hosts = s.options.allow.join(", ");
        let how = match e.filter {
            Filter::Domains => l.t("sandbox-status-net-domains"),
            Filter::Proxy => l.t_args("sandbox-status-net-proxy", &args!("backend" => backend)),
            Filter::None => l.t_args("sandbox-net-wide", &args!("backend" => backend)),
        };
        format!(
            "{} {how}",
            l.t_args("sandbox-status-net-list", &args!("hosts" => hosts))
        )
    };
    println!("  {net}");
    let o = &s.options;
    let none = l.t("sandbox-state-none");
    let mem = if o.memory_mb == 0 {
        none.clone()
    } else {
        state(e.memory)
    };
    let cpu = if o.cpu_percent == 0 || o.cpu_percent >= 100 {
        none
    } else {
        state(e.cpu)
    };
    println!(
        "  {}",
        l.t_args(
            "sandbox-status-memory",
            &args!("mb" => o.memory_mb, "state" => mem)
        )
    );
    println!(
        "  {}",
        l.t_args(
            "sandbox-status-cpu",
            &args!("pct" => o.cpu_percent, "state" => cpu)
        )
    );
    let overlay = if o.overlay {
        "sandbox-status-overlay-on"
    } else {
        "sandbox-status-overlay-off"
    };
    println!("  {}", l.t(overlay));
    if let Some(d) = st.docker {
        let key = if !d.daemon {
            "sandbox-docker-nodaemon"
        } else if d.image_present {
            "sandbox-docker-present"
        } else {
            "sandbox-docker-missing"
        };
        println!(
            "  {}",
            l.t_args(
                "sandbox-status-docker",
                &args!("image" => o.image.clone(), "state" => l.t(key))
            )
        );
    }
    println!("  {}", l.t("sandbox-status-covered"));
    for n in &s.notes {
        println!("  ! {n}");
    }
}

fn test(l: &Localizer, st: &kor_runtime::sandbox::Status) -> anyhow::Result<bool> {
    let kind = st.setup.kind;
    if kind == SandboxKind::None {
        println!("{}", l.t("sandbox-test-none"));
        return Ok(false);
    }
    let p = crate::providers::runtime()?.block_on(kor_sandbox::probe(kind, &st.setup.options))?;
    let path = kor_tools::display_path(&p.target);
    if !p.blocked {
        eprintln!(
            "{}",
            l.t_args(
                "sandbox-test-allowed",
                &args!("backend" => kind.as_str(), "path" => path)
            )
        );
        return Ok(false);
    }
    println!(
        "{}",
        l.t_args(
            "sandbox-test-blocked",
            &args!("backend" => kind.as_str(), "path" => path)
        )
    );
    match p.detected.first() {
        Some(d) => println!(
            "  {}",
            l.t_args("sandbox-test-detected", &args!("line" => d.line.clone()))
        ),
        None => println!("  ! {}", l.t("sandbox-test-undetected")),
    }
    Ok(true)
}
