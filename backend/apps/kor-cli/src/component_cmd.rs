//! `kor component list|add|remove|install --all|update|verify`: Kor's downloadable helper programs (§24.2, `kor-components`).

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;

use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum ComponentCmd {
    /// Show every component, its version and whether it is installed.
    List,
    /// Download, verify and install a component (or install it from a local archive).
    Add {
        name: String,
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long, short)]
        yes: bool,
    },
    /// Delete an installed component.
    Remove { name: String },
    /// Download every component that has a build for this machine (one question for the total).
    Install {
        #[arg(long, required = true)]
        all: bool,
        #[arg(long, short)]
        yes: bool,
    },
    /// Install the pinned version of every component that has an older one installed.
    Update {
        #[arg(long, short)]
        yes: bool,
    },
    /// Re-hash installed files; a broken copy is deleted and downloaded again on next use.
    Verify { name: Option<String> },
}

/// `45.9 MB`
fn mb(bytes: u64) -> String {
    // Display only; f64 is exact far beyond any download size.
    #[allow(clippy::cast_precision_loss)]
    let v = bytes as f64 / 1_048_576.0;
    format!("{v:.1} MB")
}

fn known(l: &Localizer, name: &str) -> Option<kor_components::Status> {
    let all = kor_components::list();
    let found = all.iter().find(|s| s.name == name).cloned();
    if found.is_none() {
        let names: Vec<_> = all.iter().map(|s| s.name.as_str()).collect();
        eprintln!(
            "{}",
            l.t_args(
                "component-unknown",
                &args!("name" => name.to_owned(), "names" => names.join(", "))
            )
        );
    }
    found
}

fn list(l: &Localizer) -> bool {
    let mut rows = vec![vec![
        l.t("component-col-name"),
        l.t("component-col-version"),
        l.t("component-col-size"),
        l.t("component-col-state"),
    ]];
    for s in kor_components::list() {
        let state = match (&s.installed, s.size) {
            (Some(p), _) => p.display().to_string(),
            (None, Some(_)) => l.t("component-missing"),
            (None, None) => l.t("component-unavailable"),
        };
        rows.push(vec![
            s.name,
            s.version,
            s.size.map(mb).unwrap_or_default(),
            state,
        ]);
    }
    crate::providers::table(&rows);
    true
}

fn ask(l: &Localizer, s: &kor_components::Status, size: u64) -> bool {
    if !std::io::stdin().is_terminal() {
        return true;
    }
    print!(
        "{} ",
        l.t_args(
            "component-ask",
            &args!("name" => s.name.clone(), "version" => s.version.clone(), "size" => mb(size))
        )
    );
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    let a = line.trim().to_lowercase();
    a.is_empty() || ["y", "yes", "e", "evet", "o", "oui"].contains(&a.as_str())
}

fn add(ctx: &Ctx, name: &str, from: Option<&std::path::Path>, yes: bool) -> bool {
    let l = &ctx.l;
    let fail = |e: String| {
        eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
        false
    };
    let Some(s) = known(l, name) else {
        return false;
    };
    let done = |p: PathBuf| {
        println!(
            "{}",
            l.t_args(
                "component-installed-at",
                &args!("name" => name.to_owned(), "path" => p.display().to_string())
            )
        );
        true
    };
    if let Some(file) = from {
        return kor_components::add_from(name, file).map_or_else(fail, done);
    }
    if let Some(p) = s.installed {
        return done(p);
    }
    let Some(size) = s.size else {
        return fail(l.t_args("component-no-build", &args!("name" => name.to_owned())));
    };
    let privacy = ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
    kor_components::set_privacy(privacy);
    if privacy {
        return fail(l.t_args("component-privacy", &args!("name" => name.to_owned())));
    }
    if !yes && !ask(l, &s, size) {
        println!("{}", l.t("component-cancelled"));
        return false;
    }
    let rt = match crate::providers::runtime() {
        Ok(rt) => rt,
        Err(e) => return fail(e.to_string()),
    };
    let show = |p: kor_components::Progress| {
        let pct = p.total.filter(|t| *t > 0).map_or(0, |t| p.done * 100 / t);
        eprint!("\r{} {pct:>3}% ({})", p.name, mb(p.done));
        let _ = std::io::stderr().flush();
    };
    let r = rt.block_on(kor_components::ensure(name, &show));
    eprintln!();
    r.map_or_else(fail, done)
}

fn remove(l: &Localizer, name: &str) -> bool {
    if known(l, name).is_none() {
        return false;
    }
    match kor_components::remove(name) {
        Ok(true) => {
            println!(
                "{}",
                l.t_args("component-removed", &args!("name" => name.to_owned()))
            );
            true
        }
        Ok(false) => {
            println!(
                "{}",
                l.t_args("component-not-installed", &args!("name" => name.to_owned()))
            );
            true
        }
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            false
        }
    }
}

/// Download `names` after one question for their total size.
fn install_many(ctx: &Ctx, names: &[String], yes: bool) -> bool {
    let l = &ctx.l;
    let all = kor_components::list();
    let todo: Vec<&kor_components::Status> = all
        .iter()
        .filter(|s| names.contains(&s.name) && s.size.is_some())
        .collect();
    if todo.is_empty() {
        println!("{}", l.t("component-nothing"));
        return true;
    }
    let total: u64 = todo.iter().filter_map(|s| s.size).sum();
    if !yes && std::io::stdin().is_terminal() {
        print!(
            "{} ",
            l.t_args(
                "component-ask-many",
                &args!("count" => todo.len(), "size" => mb(total))
            )
        );
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let a = std::io::stdin()
            .read_line(&mut line)
            .map(|_| line.trim().to_lowercase())
            .unwrap_or_default();
        if !(a.is_empty() || ["y", "yes", "e", "evet", "o", "oui"].contains(&a.as_str())) {
            println!("{}", l.t("component-cancelled"));
            return false;
        }
    }
    // Already asked once for the total; each `add` below does not ask again.
    todo.iter()
        .fold(true, |ok, s| add(ctx, &s.name, None, true) && ok)
}

fn verify(l: &Localizer, name: Option<&str>) -> bool {
    let names: Vec<String> = match name {
        Some(n) => match known(l, kor_components::canonical(n)) {
            Some(s) => vec![s.name],
            None => return false,
        },
        None => kor_components::list()
            .into_iter()
            .filter(|s| s.installed.is_some())
            .map(|s| s.name)
            .collect(),
    };
    let mut ok = true;
    for n in names {
        let line = match kor_components::verify(&n) {
            Ok(kor_components::Health::Ok) => {
                l.t_args("component-verify-ok", &args!("name" => n.clone()))
            }
            Ok(kor_components::Health::Missing) => {
                l.t_args("component-not-installed", &args!("name" => n.clone()))
            }
            Ok(kor_components::Health::Outdated { installed }) => l.t_args(
                "component-verify-outdated",
                &args!("name" => n.clone(), "version" => installed),
            ),
            Ok(kor_components::Health::Unrecorded) => {
                l.t_args("component-verify-unrecorded", &args!("name" => n.clone()))
            }
            Ok(kor_components::Health::Broken(files)) => {
                ok = false;
                l.t_args(
                    "component-verify-broken",
                    &args!("name" => n.clone(), "files" => files.join(", ")),
                )
            }
            Err(e) => {
                ok = false;
                l.t_args("error-generic", &args!("error" => e))
            }
        };
        println!("{line}");
    }
    ok
}

pub fn run(ctx: &Ctx, cmd: ComponentCmd) -> bool {
    match cmd {
        ComponentCmd::List => list(&ctx.l),
        ComponentCmd::Add { name, from, yes } => {
            add(ctx, kor_components::canonical(&name), from.as_deref(), yes)
        }
        ComponentCmd::Remove { name } => remove(&ctx.l, kor_components::canonical(&name)),
        ComponentCmd::Install { yes, .. } => {
            let names: Vec<String> = kor_components::list()
                .into_iter()
                .filter(|s| s.installed.is_none())
                .map(|s| s.name)
                .collect();
            install_many(ctx, &names, yes)
        }
        ComponentCmd::Update { yes } => install_many(ctx, &kor_components::outdated(), yes),
        ComponentCmd::Verify { name } => verify(&ctx.l, name.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn install_needs_all() {
        use clap::Parser as _;
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: super::ComponentCmd,
        }
        assert!(T::try_parse_from(["kor", "install"]).is_err());
        assert!(matches!(
            T::try_parse_from(["kor", "install", "--all", "-y"])
                .unwrap()
                .c,
            super::ComponentCmd::Install {
                all: true,
                yes: true
            }
        ));
        assert!(matches!(
            T::try_parse_from(["kor", "verify"]).unwrap().c,
            super::ComponentCmd::Verify { name: None }
        ));
    }

    #[test]
    fn sizes_read_as_megabytes() {
        assert_eq!(super::mb(48_077_770), "45.9 MB");
        assert_eq!(super::mb(0), "0.0 MB");
    }
}
