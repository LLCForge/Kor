//! `kor packages [affected [--since <ref>] | deps <name> | cost [--days N]]` (spec §15.15).
//! Detection, the package graph and narrowed commands live in `kor_graph::workspace`; this only
//! prints.

use std::path::Path;

use kor_graph::workspace::{self, Package, Workspace};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, Clone, clap::Subcommand)]
pub enum PackagesCmd {
    /// Packages touched by the changes (plus their dependents) and narrowed build/test commands.
    Affected {
        /// Compare against this git ref instead of `HEAD` (uncommitted changes).
        #[arg(long)]
        since: Option<String>,
    },
    /// Direct dependencies and (transitive) dependents of a package.
    Deps { name: String },
    /// Cost per package, from the files each model call's session touched.
    Cost {
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=365))]
        days: u32,
    },
}

/// `kor packages`: the workspace's packages, or one of the subcommands.
pub fn run(ctx: &Ctx, cmd: Option<&PackagesCmd>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = kor_runtime::project_root()?;
    let ws = workspace::detect(&root);
    if !ws.is_monorepo() {
        println!("{}", l.t("packages-none"));
        return Ok(true);
    }
    Ok(match cmd {
        None => {
            list(l, &ws);
            true
        }
        Some(PackagesCmd::Affected { since }) => affected(l, &ws, since.as_deref()),
        Some(PackagesCmd::Deps { name }) => deps(l, &ws, name),
        Some(PackagesCmd::Cost { days }) => {
            cost(l, &ws, *days);
            true
        }
    })
}

fn line(ws: &Workspace, p: &Package) -> String {
    let rel = ws.rel(&p.root);
    format!(
        "  {:<32} {:<7} {}",
        p.name,
        p.tool.key(),
        if rel.is_empty() { "." } else { &rel }
    )
}

fn list(l: &Localizer, ws: &Workspace) {
    let kinds: Vec<&str> = ws.kinds.iter().map(|k| k.key()).collect();
    println!(
        "{}",
        l.t_args(
            "packages-head",
            &args!("count" => ws.packages.len(), "kinds" => kinds.join(", "))
        )
    );
    for p in &ws.packages {
        if p.deps.is_empty() {
            println!("{}", line(ws, p));
        } else {
            println!("{}  → {}", line(ws, p), p.deps.join(", "));
        }
    }
}

fn affected(l: &Localizer, ws: &Workspace, since: Option<&str>) -> bool {
    let changed = match workspace::changed_files(&ws.root, since) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", l.t_args("packages-git-error", &args!("error" => e)));
            return false;
        }
    };
    let hit = ws.affected(&changed);
    if hit.is_empty() {
        println!("{}", l.t("packages-affected-none"));
        return true;
    }
    println!(
        "{}",
        l.t_args("packages-affected-head", &args!("count" => hit.len()))
    );
    for p in &hit {
        println!("{}", line(ws, p));
    }
    println!("{}", l.t("packages-commands-head"));
    for c in ws.narrowed_commands(&hit) {
        println!("  {c}");
    }
    true
}

fn deps(l: &Localizer, ws: &Workspace, name: &str) -> bool {
    let Some(p) = ws.get(name) else {
        eprintln!(
            "{}",
            l.t_args("packages-not-found", &args!("name" => name.to_owned()))
        );
        return false;
    };
    let users: Vec<&str> = ws
        .dependents(name)
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let or_none = |v: String| if v.is_empty() { "-".to_owned() } else { v };
    println!("{}", line(ws, p));
    println!(
        "{}",
        l.t_args(
            "packages-deps",
            &args!("list" => or_none(p.deps.join(", ")))
        )
    );
    println!(
        "{}",
        l.t_args(
            "packages-dependents",
            &args!("list" => or_none(users.join(", ")))
        )
    );
    true
}

fn cost(l: &Localizer, ws: &Workspace, days: u32) {
    use kor_runtime::cost;
    let entries: Vec<_> = kor_runtime::usage::file()
        .map(|p| kor_runtime::usage::read(&p))
        .unwrap_or_default()
        .into_iter()
        .filter(|e| {
            e.path
                .as_deref()
                .is_some_and(|p| Path::new(p).starts_with(&ws.root))
        })
        .collect();
    let outside = l.t("packages-cost-outside");
    let rows = cost::rows_by(&entries, &cost::Prices::load(), cost::now_ms(), days, |e| {
        e.path
            .as_deref()
            .and_then(|p| ws.package_of(Path::new(p)))
            .map_or_else(|| outside.clone(), |p| p.name.clone())
    });
    println!("{}", l.t_args("packages-cost-head", &args!("days" => days)));
    if rows.is_empty() {
        println!("{}", l.t("packages-cost-none"));
        return;
    }
    for r in &rows {
        println!("{}", cost::row_line(l, &r.model, r));
    }
    println!("{}", l.t("cost-estimate-note"));
}
