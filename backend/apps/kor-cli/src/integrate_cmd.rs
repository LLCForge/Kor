//! `kor integrate install|uninstall|status` — the "Kor ile Aç" file-manager entries (§25,
//! ADR 0052) — and the two commands those entries run on installs without the desktop app:
//! `kor integrate open-terminal [--task explain|bugs|test] <path…>` ("Kor CLI ile aç") and
//! `kor integrate edit <path…>` ("Kor ile düzenle", §27.6). OS work is in
//! `kor_platform::shell_integration` / `kor_platform::terminal`; nothing is installed without the
//! user's confirmation (`--yes` for installers).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use kor_i18n::{Localizer, args};
use kor_platform::shell_integration::{self as si, Apps, Options, Status};
use kor_runtime::osint::{self, QuickTask};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct IntegrateArgs {
    #[command(subcommand)]
    pub command: IntegrateCmd,
}

#[derive(Debug, clap::Subcommand)]
pub enum IntegrateCmd {
    /// Add the entries for this user (no administrator rights).
    Install {
        /// Also "Projeyi açıkla / Hata bul / Bu dosyayı test et" (default: `shell.context_menu.extended`).
        #[arg(long, conflicts_with = "no_extended")]
        extended: bool,
        #[arg(long)]
        no_extended: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Remove everything Kor added.
    Uninstall {
        #[arg(long)]
        yes: bool,
    },
    /// What is installed.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Open `kor` in the terminal at the project of the paths.
    OpenTerminal {
        #[arg(long, value_parser = ["explain", "bugs", "test"])]
        task: Option<String>,
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Batch-edit the files (`kor files`) in a terminal; parallel calls (one per selected file) merge.
    Edit {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-integrate"))
        .mut_subcommand("install", |s| {
            s.about(l.t("cmd-integrate-install"))
                .mut_arg("extended", |a| a.help(l.t("arg-integrate-extended")))
                .mut_arg("no_extended", |a| a.help(l.t("arg-integrate-no-extended")))
                .mut_arg("yes", |a| a.help(l.t("arg-integrate-yes")))
        })
        .mut_subcommand("uninstall", |s| {
            s.about(l.t("cmd-integrate-uninstall"))
                .mut_arg("yes", |a| a.help(l.t("arg-integrate-yes")))
        })
        .mut_subcommand("status", |s| {
            s.about(l.t("cmd-integrate-status"))
                .mut_arg("json", |a| a.help(l.t("arg-integrate-json")))
        })
        .mut_subcommand("open-terminal", |s| {
            s.about(l.t("cmd-integrate-open-terminal"))
                .mut_arg("task", |a| a.help(l.t("arg-integrate-task")))
                .mut_arg("paths", |a| a.help(l.t("arg-integrate-paths")))
        })
        .mut_subcommand("edit", |s| {
            s.about(l.t("cmd-integrate-edit"))
                .mut_arg("paths", |a| a.help(l.t("arg-integrate-paths")))
        })
}

fn apps() -> Apps {
    si::detect_apps(std::env::current_exe().ok())
}

/// First interactive start of the CLI (ADR 0052): the right-click menu and the default shell's
/// block, each once and only if never attempted on this machine (a removed item stays removed).
/// The desktop adds its own OS items the same way. Release builds only; isolated and cargo runs skip.
pub fn first_run(ctx: &Ctx) {
    use kor_runtime::autointegrate::{self as auto, Item, Outcome};
    if cfg!(debug_assertions) {
        return;
    }
    let blocked = auto::guard_here(false);
    let (Some(data), Some(cfg)) = (kor_platform::data_dir(), config(ctx)) else {
        return;
    };
    let shell =
        Some(kor_runtime::shell::default_kind()).filter(|k| kor_runtime::shell::present(*k));
    let items = auto::plan(&cfg.integrations, shell, &[]);
    let l = &ctx.l;
    let out = auto::run_once(&data, blocked, &items, |it| match it {
        Item::ContextMenu => {
            let opts = Options {
                extended: cfg.shell.context_menu.extended,
                lang: l.lang().code().to_owned(),
            };
            si::install(&apps(), &opts)
                .map(drop)
                .map_err(|e| e.to_string())
        }
        Item::Shell(k) => kor_runtime::shell::plan_install(*k)
            .map_err(|e| format!("{e:?}"))
            .and_then(|p| kor_runtime::shell::apply(&p).map_err(|e| e.to_string())),
        // The plan gets no OS items here: those need the desktop's bundle.
        Item::Os(_) => Err("desktop-only".into()),
    });
    if let Ok(Outcome::Ran(m)) = out
        && !m.installed.is_empty()
    {
        eprintln!(
            "{}",
            l.t_args(
                "autoint-notice-body",
                &args!("list" => auto::names(l, &m.installed))
            )
        );
    }
}

fn config(ctx: &Ctx) -> Option<&kor_config::Config> {
    ctx.config.as_ref().ok().map(|c| &c.config)
}

fn print_status(l: &Localizer, s: &Status) {
    let yes_no = |b: bool| l.t(if b { "integrate-yes" } else { "integrate-no" });
    println!("{}: {}", l.t("integrate-installed"), yes_no(s.installed));
    if s.installed {
        println!("{}: {}", l.t("integrate-extended"), yes_no(s.extended));
    }
    println!("{}: {}", l.t("integrate-scheme"), yes_no(s.scheme));
    if let Some(m) = s.modern_menu {
        println!("{}: {}", l.t("integrate-win11-menu"), yes_no(m));
    }
    if let Some(f) = s.finder_sync {
        println!("{}: {}", l.t("integrate-finder-sync"), yes_no(f));
    }
    if !s.file_managers.is_empty() {
        println!(
            "{}: {}",
            l.t("integrate-file-managers"),
            s.file_managers.join(", ")
        );
    }
    for e in &s.entries {
        println!("  {e}");
    }
    if let Some(e) = &s.last_error {
        println!("{}: {e}", l.t("integrate-last-error"));
    }
}

pub fn run(ctx: &Ctx, a: &IntegrateArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    match &a.command {
        IntegrateCmd::Status { json } => {
            let s = si::status(&apps());
            if *json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                print_status(l, &s);
                println!(
                    "{}",
                    kor_runtime::autointegrate::status_line(
                        l,
                        &config(ctx)
                            .map(|c| c.integrations.clone())
                            .unwrap_or_default()
                    )
                );
            }
            Ok(true)
        }
        IntegrateCmd::Install {
            extended,
            no_extended,
            yes,
        } => {
            let apps = apps();
            let extended = *extended
                || (!*no_extended && config(ctx).is_some_and(|c| c.shell.context_menu.extended));
            let what = match (&apps.desktop, &apps.cli) {
                (Some(d), _) => d.display().to_string(),
                (None, Some(c)) => c.display().to_string(),
                (None, None) => String::new(),
            };
            println!(
                "{}",
                l.t_args("integrate-install-plan", &args!("app" => what))
            );
            if !crate::shell_cmd::confirm(l, &l.t("integrate-install-ask"), *yes) {
                return Ok(true);
            }
            let opts = Options {
                extended,
                lang: l.lang().code().to_owned(),
            };
            match si::install(&apps, &opts) {
                Ok(s) => {
                    println!("{}", l.t("integrate-installed-ok"));
                    print_status(l, &s);
                    Ok(true)
                }
                Err(e) => {
                    eprintln!(
                        "{}",
                        l.t_args("integrate-failed", &args!("error" => e.to_string()))
                    );
                    Ok(false)
                }
            }
        }
        IntegrateCmd::Uninstall { yes } => {
            if !crate::shell_cmd::confirm(l, &l.t("integrate-uninstall-ask"), *yes) {
                return Ok(true);
            }
            match si::uninstall() {
                Ok(_) => {
                    println!("{}", l.t("integrate-removed"));
                    Ok(true)
                }
                Err(e) => {
                    eprintln!(
                        "{}",
                        l.t_args("integrate-failed", &args!("error" => e.to_string()))
                    );
                    Ok(false)
                }
            }
        }
        IntegrateCmd::OpenTerminal { task, paths } => {
            open_terminal(ctx, task.as_deref(), paths, &[])
        }
        IntegrateCmd::Edit { paths } => edit(ctx, paths),
    }
}

fn absolute(paths: &[PathBuf]) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    paths
        .iter()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                cwd.join(p)
            }
        })
        .collect()
}

fn open_terminal(
    ctx: &Ctx,
    task: Option<&str>,
    paths: &[PathBuf],
    extra: &[String],
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let paths = absolute(paths);
    let Some((root, files)) = osint::resolve(&paths) else {
        return Ok(false);
    };
    let mut args: Vec<String> = extra.to_vec();
    if let Some(q) = task.and_then(QuickTask::parse) {
        args.push(q.prompt(l, &root, &files));
    }
    let kor = std::env::current_exe()?;
    let preferred = config(ctx).and_then(|c| c.shell.terminal.clone());
    match kor_platform::terminal::open(&root, preferred.as_deref(), &kor, &args) {
        Ok(()) => Ok(true),
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args(
                    "integrate-terminal-failed",
                    &args!("error" => e.to_string())
                )
            );
            Ok(false)
        }
    }
}

/// Spool directory where parallel `kor integrate edit` calls leave their paths.
fn spool() -> anyhow::Result<PathBuf> {
    let d = kor_platform::data_dir()
        .ok_or_else(|| anyhow::anyhow!("no data directory"))?
        .join("edit-spool");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Collect every spooled path once no new spool file arrived for `quiet`, deleting the files.
pub(crate) fn drain(dir: &Path, quiet: Duration, max_wait: Duration) -> Vec<PathBuf> {
    let start = SystemTime::now();
    let count = || std::fs::read_dir(dir).map(Iterator::count).unwrap_or(0);
    let mut seen = count();
    loop {
        std::thread::sleep(quiet);
        let now = count();
        if now == seen || start.elapsed().unwrap_or_default() > max_wait {
            break;
        }
        seen = now;
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(&f) {
            for line in text.lines().filter(|l| !l.is_empty()) {
                let p = PathBuf::from(line);
                if !out.contains(&p) && out.len() < osint::MAX_PATHS {
                    out.push(p);
                }
            }
        }
        let _ = std::fs::remove_file(f);
    }
    out
}

fn edit(ctx: &Ctx, paths: &[PathBuf]) -> anyhow::Result<bool> {
    let dir = spool()?;
    let mine = dir.join(format!(
        "{}-{}.txt",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos()
    ));
    let body = absolute(paths).iter().fold(String::new(), |mut s, p| {
        s.push_str(&p.to_string_lossy());
        s.push('\n');
        s
    });
    std::fs::write(&mine, body)?;
    let lock_path = dir.with_extension("lock");
    // Explorer starts one process per selected file: the first one collects, the others leave.
    // A latecomer whose file is still there after the collector finished collects it itself.
    let guard = loop {
        if let Some(g) = kor_platform::try_lock_file(&lock_path)? {
            break g;
        }
        std::thread::sleep(Duration::from_millis(250));
        if !mine.exists() {
            return Ok(true);
        }
    };
    if !mine.exists() {
        return Ok(true);
    }
    let files = drain(&dir, Duration::from_millis(500), Duration::from_secs(5));
    drop(guard);
    let mut extra = vec!["files".to_owned()];
    extra.extend(files.iter().map(|p| p.display().to_string()));
    open_terminal(ctx, None, &files, &extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: IntegrateArgs,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["integrate", "install", "--extended", "--yes"]).unwrap();
        assert!(matches!(
            t.a.command,
            IntegrateCmd::Install {
                extended: true,
                yes: true,
                ..
            }
        ));
        assert!(
            T::try_parse_from(["integrate", "install", "--extended", "--no-extended"]).is_err()
        );
        let t = T::try_parse_from(["integrate", "open-terminal", "--task", "bugs", "/a", "/b"])
            .unwrap();
        assert!(
            matches!(t.a.command, IntegrateCmd::OpenTerminal { task: Some(ref x), ref paths } if x == "bugs" && paths.len() == 2)
        );
        assert!(T::try_parse_from(["integrate", "open-terminal", "--task", "rm", "/a"]).is_err());
        assert!(T::try_parse_from(["integrate", "edit"]).is_err());
        assert!(matches!(
            T::try_parse_from(["integrate", "status", "--json"])
                .unwrap()
                .a
                .command,
            IntegrateCmd::Status { json: true }
        ));
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            localize(T::command(), &Localizer::new(lang)).debug_assert();
        }
    }

    #[test]
    fn spooled_paths_merge_once() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("1.txt"), "/a\n/b\n").unwrap();
        std::fs::write(d.path().join("2.txt"), "/b\n/c\n").unwrap();
        let got = drain(d.path(), Duration::from_millis(10), Duration::from_secs(1));
        assert_eq!(
            got,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }
}
