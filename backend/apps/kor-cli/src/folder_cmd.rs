//! `kor folder init|status|sync|remove` and `kor mount [--at] [--unmount]` (§27.5, ADR 0052).
//! Logic in `kor_runtime::korfolder`; the drive in `kor_platform::mount`.

use std::sync::Arc;

use kor_i18n::{Localizer, args};
use kor_runtime::korfolder::{self, Layout, dav, watch};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum FolderCmd {
    /// Create `~/Kor/` and fill it.
    Init {
        #[arg(long)]
        yes: bool,
    },
    /// What is in `~/Kor/`.
    Status,
    /// Update views, apply edited front matter, run dropped tasks.
    Sync,
    /// Remove what Kor created in `~/Kor/` (your files stay).
    Remove {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, clap::Args)]
pub struct MountArgs {
    /// Drive letter (Windows, `*` = next free) or folder (macOS) to mount at.
    #[arg(long)]
    pub at: Option<String>,
    /// Unmount a drive left mounted.
    #[arg(long)]
    pub unmount: bool,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let yes = |s: clap::Command| s.mut_arg("yes", |a| a.help(l.t("arg-folder-yes")));
    c.about(l.t("cmd-folder"))
        .mut_subcommand("init", |s| yes(s.about(l.t("cmd-folder-init"))))
        .mut_subcommand("status", |s| s.about(l.t("cmd-folder-status")))
        .mut_subcommand("sync", |s| s.about(l.t("cmd-folder-sync")))
        .mut_subcommand("remove", |s| yes(s.about(l.t("cmd-folder-remove"))))
}

pub fn localize_mount(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-mount"))
        .mut_arg("at", |a| a.help(l.t("arg-mount-at")))
        .mut_arg("unmount", |a| a.help(l.t("arg-mount-unmount")))
}

fn layout() -> anyhow::Result<Layout> {
    Layout::current().ok_or_else(|| anyhow::anyhow!("no home directory"))
}

fn lang(ctx: &Ctx) -> kor_i18n::Lang {
    ctx.l.lang()
}

/// The project of the working folder, when it is one (`.git`, `AGENTS.md`, `.kor/`).
fn cwd_project() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let root = kor_runtime::osint::project_root(&cwd);
    (root.join(".git").exists() || root.join("AGENTS.md").is_file() || root.join(".kor").is_dir())
        .then_some(root)
}

fn print_sync(l: &Localizer, r: &korfolder::SyncReport) {
    println!(
        "{}",
        l.t_args(
            "folder-synced",
            &args!("written" => r.written.len(), "linked" => r.linked.len(), "applied" => r.applied.len())
        )
    );
    for p in &r.kept {
        println!(
            "  {}",
            l.t_args("folder-kept", &args!("path" => p.display().to_string()))
        );
    }
    for n in &r.notes {
        eprintln!("  ! {n}");
    }
}

#[allow(clippy::needless_pass_by_value)] // owned value from the parsed arguments
pub fn run(ctx: &Ctx, cmd: FolderCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let lay = layout()?;
    match cmd {
        FolderCmd::Init { yes } => {
            let q = l.t_args(
                "folder-init-ask",
                &args!("path" => lay.root.display().to_string()),
            );
            if !lay.enabled() && !crate::shell_cmd::confirm(l, &q, yes) {
                return Ok(false);
            }
            let r = korfolder::enable(&lay, cwd_project().as_deref())?;
            println!(
                "{}",
                l.t_args(
                    "folder-ready",
                    &args!("path" => lay.root.display().to_string())
                )
            );
            print_sync(l, &r);
            Ok(true)
        }
        FolderCmd::Status => {
            let s = korfolder::status(&lay);
            if !s.enabled {
                println!("{}", l.t("folder-off"));
                return Ok(true);
            }
            println!(
                "{}",
                l.t_args("folder-on", &args!("path" => s.path.display().to_string()))
            );
            for i in &s.items {
                match &i.target {
                    Some(t) => println!("  {}  → {}", i.name, t.display()),
                    None => println!("  {}/", i.name),
                }
            }
            println!(
                "{}",
                l.t_args(
                    "folder-waiting",
                    &args!("inbox" => s.inbox, "tasks" => s.tasks)
                )
            );
            Ok(true)
        }
        FolderCmd::Sync => {
            if !lay.enabled() {
                eprintln!("{}", l.t("folder-off"));
                return Ok(false);
            }
            if let Some(p) = cwd_project() {
                korfolder::add(&lay, &p)?;
            }
            let runner = watch::AgentRunner { lang: lang(ctx) };
            let pass = crate::providers::runtime()?.block_on(watch::pass(&lay, &runner));
            print_sync(l, &pass.sync);
            for t in &pass.tasks {
                match t {
                    korfolder::TaskOutcome::Done { result, .. } => {
                        println!("  ✓ {}", result.display());
                    }
                    korfolder::TaskOutcome::Failed { error, file } => {
                        eprintln!("  ✗ {}: {error}", file.display());
                    }
                }
            }
            Ok(true)
        }
        FolderCmd::Remove { yes } => {
            if !lay.enabled() {
                println!("{}", l.t("folder-off"));
                return Ok(true);
            }
            if !crate::shell_cmd::confirm(l, &l.t("folder-remove-ask"), yes) {
                return Ok(false);
            }
            let r = korfolder::disable(&lay)?;
            println!(
                "{}",
                l.t_args("folder-removed", &args!("count" => r.removed.len()))
            );
            for p in &r.kept {
                println!(
                    "  {}",
                    l.t_args("folder-kept", &args!("path" => p.display().to_string()))
                );
            }
            Ok(true)
        }
    }
}

/// Last mount, for `--unmount` from another process.
fn mount_record(lay: &Layout) -> std::path::PathBuf {
    lay.home.join("korfolder-mount.json")
}

fn default_at(lay: &Layout) -> String {
    match kor_platform::os() {
        kor_platform::Os::Windows => "*".into(),
        kor_platform::Os::Mac => lay.root.with_file_name("KorDrive").display().to_string(),
        _ => String::new(),
    }
}

pub fn mount(ctx: &Ctx, a: &MountArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let lay = layout()?;
    if a.unmount {
        let rec: serde_json::Value = std::fs::read(mount_record(&lay))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| anyhow::anyhow!(l.t("mount-none")))?;
        let (url, at) = (
            rec["url"].as_str().unwrap_or(""),
            rec["at"].as_str().unwrap_or(""),
        );
        kor_platform::mount::unmount(url, at)?;
        let _ = std::fs::remove_file(mount_record(&lay));
        println!("{}", l.t("mount-unmounted"));
        return Ok(true);
    }
    if !lay.enabled() {
        eprintln!("{}", l.t("folder-off"));
        return Ok(false);
    }
    let at = a.at.clone().unwrap_or_else(|| default_at(&lay));
    crate::providers::runtime()?.block_on(async {
        let server = dav::serve(lay.root.clone()).await?;
        // Keep views and drop boxes live while mounted.
        let _watch = watch::start(
            lay.clone(),
            Arc::new(watch::AgentRunner { lang: lang(ctx) }),
            Arc::new(|_: &watch::Pass| {}),
        );
        let shown = match kor_platform::mount::mount(&server.url, &at) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "{}",
                    l.t_args(
                        "mount-failed",
                        &args!("error" => e.to_string(), "hint" => kor_platform::mount::client_hint(kor_platform::os()), "path" => lay.root.display().to_string())
                    )
                );
                return Ok(false);
            }
        };
        let rec = serde_json::json!({ "url": server.url, "at": if shown.ends_with(':') { shown.clone() } else { at.clone() } });
        let _ = std::fs::write(mount_record(&lay), rec.to_string());
        println!("{}", l.t_args("mount-ready", &args!("at" => shown.clone(), "url" => server.url.clone())));
        let _ = tokio::signal::ctrl_c().await;
        let unmount_at = if shown.ends_with(':') { shown } else { at };
        let _ = kor_platform::mount::unmount(&server.url, &unmount_at);
        let _ = std::fs::remove_file(mount_record(&lay));
        println!("{}", l.t("mount-unmounted"));
        Ok::<_, anyhow::Error>(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct F {
        #[command(subcommand)]
        c: FolderCmd,
    }

    #[derive(Debug, clap::Parser)]
    struct M {
        #[command(flatten)]
        a: MountArgs,
    }

    #[test]
    fn parses_and_localizes() {
        assert!(matches!(
            F::try_parse_from(["folder", "init", "--yes"]).unwrap().c,
            FolderCmd::Init { yes: true }
        ));
        assert!(matches!(
            F::try_parse_from(["folder", "sync"]).unwrap().c,
            FolderCmd::Sync
        ));
        assert!(matches!(
            F::try_parse_from(["folder", "remove"]).unwrap().c,
            FolderCmd::Remove { yes: false }
        ));
        assert!(F::try_parse_from(["folder", "sil"]).is_err());
        let m = M::try_parse_from(["mount", "--at", "K:"]).unwrap();
        assert_eq!(m.a.at.as_deref(), Some("K:"));
        assert!(M::try_parse_from(["mount", "--unmount"]).unwrap().a.unmount);
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            let l = Localizer::new(lang);
            localize(F::command(), &l).debug_assert();
            localize_mount(M::command(), &l).debug_assert();
        }
    }
}
