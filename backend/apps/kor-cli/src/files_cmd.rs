//! `kor files [--task "<talimat>"] <glob|path…> [--dry-run] [--yes] [--model]` and the spec's
//! short form `kor files "<talimat>" *.jpg` (§27.6, ADR 0052). Without an instruction it asks
//! for one (the "Kor ile düzenle" menu entry of CLI-only installs). Logic in `kor_runtime::batch`:
//! the model proposes, Kor validates, shows the preview table, applies after confirmation with a
//! `kor undo` journal.

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use kor_i18n::{Localizer, args};
use kor_runtime::batch::{self, ProviderPlanner, Row};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct FilesArgs {
    /// Instruction ("tarihi başa al, boşlukları tire yap").
    #[arg(long)]
    pub task: Option<String>,
    /// Files or globs; without `--task` the first one may be the instruction.
    #[arg(required = true)]
    pub paths: Vec<String>,
    /// Show the plan only.
    #[arg(long)]
    pub dry_run: bool,
    /// Apply without asking.
    #[arg(long)]
    pub yes: bool,
    #[arg(short, long)]
    pub model: Option<String>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-files"))
        .mut_arg("task", |a| a.help(l.t("arg-files-task")))
        .mut_arg("paths", |a| a.help(l.t("arg-files-paths")))
        .mut_arg("dry_run", |a| a.help(l.t("arg-files-dry-run")))
        .mut_arg("yes", |a| a.help(l.t("arg-files-yes")))
        .mut_arg("model", |a| a.help(l.t("arg-model")))
}

/// Instruction and path patterns: `--task`, else a first argument that is not a file or a glob.
fn split(a: &FilesArgs, cwd: &Path) -> (Option<String>, Vec<String>) {
    if let Some(t) = &a.task {
        return (Some(t.clone()), a.paths.clone());
    }
    match a.paths.split_first() {
        Some((first, rest))
            if !rest.is_empty()
                && !cwd.join(first).exists()
                && !first.contains(['*', '?', '[']) =>
        {
            (Some(first.clone()), rest.to_vec())
        }
        _ => (None, a.paths.clone()),
    }
}

fn ask_instruction(l: &Localizer) -> Option<String> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    print!("{} ", l.t("files-ask"));
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok()?;
    Some(line.trim().to_owned()).filter(|s| !s.is_empty())
}

pub(crate) fn print_rows(l: &Localizer, rows: &[Row], cwd: &Path) {
    let short = |s: &str| {
        Path::new(s)
            .strip_prefix(cwd)
            .map_or_else(|_| s.to_owned(), |p| p.display().to_string())
    };
    for r in rows {
        let detail = if r.detail.is_empty() {
            String::new()
        } else {
            format!("  ({})", r.detail)
        };
        println!(
            "  {:<16} {}  →  {}{detail}",
            l.t(&format!("files-op-{}", r.op.replace('_', "-"))),
            r.from
                .split(", ")
                .map(&short)
                .collect::<Vec<_>>()
                .join(", "),
            short(&r.to)
        );
    }
}

pub fn run(ctx: &Ctx, a: &FilesArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let cwd = std::env::current_dir()?;
    let (instruction, patterns) = split(a, &cwd);
    let files: Vec<PathBuf> = batch::expand(&patterns, &cwd).map_err(|e| anyhow::anyhow!(e))?;
    if files.is_empty() {
        eprintln!("{}", l.t("files-none"));
        return Ok(false);
    }
    let Some(instruction) = instruction.or_else(|| ask_instruction(l)) else {
        eprintln!("{}", l.t("files-no-task"));
        return Ok(false);
    };
    let loaded = ctx.config.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
    let (provider, model) = kor_runtime::oneshot_provider(l, loaded, a.model.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let planner = ProviderPlanner { provider, model };
    println!(
        "{}",
        l.t_args("files-planning", &args!("count" => files.len()))
    );
    let plan = crate::providers::runtime()?
        .block_on(batch::plan(&planner, &files, &instruction))
        .map_err(|e| anyhow::anyhow!(e))?;
    for w in &plan.warnings {
        eprintln!("  ! {w}");
    }
    if plan.steps.is_empty() {
        println!("{}", l.t("files-nothing"));
        return Ok(true);
    }
    println!("\n{}", l.t("files-preview"));
    print_rows(l, &plan.rows, &cwd);
    if a.dry_run {
        println!("\n{}", l.t("files-dry-run"));
        return Ok(true);
    }
    if !crate::shell_cmd::confirm(
        l,
        &l.t_args("files-confirm", &args!("count" => plan.steps.len())),
        a.yes,
    ) {
        return Ok(false);
    }
    let out = batch::apply(&plan, kor_runtime::undo_dir().as_deref());
    println!(
        "{}",
        l.t_args("files-applied", &args!("count" => out.done.len()))
    );
    for (r, e) in &out.failed {
        eprintln!("  ✗ {}: {e}", r.from);
    }
    if let Some(id) = &out.undo {
        println!(
            "{}",
            l.t_args("files-undo", &args!("session" => id.clone()))
        );
    }
    Ok(out.failed.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: FilesArgs,
    }

    #[test]
    fn parses_both_forms_and_localizes() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.jpg"), "").unwrap();
        let t = T::try_parse_from(["files", "--task", "webp yap", "*.jpg", "--dry-run"]).unwrap();
        assert_eq!(
            split(&t.a, d.path()),
            (Some("webp yap".into()), vec!["*.jpg".into()])
        );
        assert!(t.a.dry_run);
        let t = T::try_parse_from(["files", "boşlukları tire yap", "*.jpg", "a.jpg"]).unwrap();
        assert_eq!(
            split(&t.a, d.path()).0.as_deref(),
            Some("boşlukları tire yap")
        );
        // Only paths: the instruction is asked for.
        let t = T::try_parse_from(["files", "a.jpg"]).unwrap();
        assert_eq!(split(&t.a, d.path()), (None, vec!["a.jpg".into()]));
        let t = T::try_parse_from(["files", "a.jpg", "*.png"]).unwrap();
        assert_eq!(split(&t.a, d.path()).0, None);
        assert!(T::try_parse_from(["files"]).is_err());
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            localize(T::command(), &Localizer::new(lang)).debug_assert();
        }
    }
}
