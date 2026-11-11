//! `kor undo`: revert every file change of a session (the most recent by default).

use kor_i18n::args;

use crate::Ctx;

pub fn run(ctx: &Ctx, list: bool, session: Option<&str>) -> bool {
    let l = &ctx.l;
    let Some(base) = kor_runtime::undo_dir() else {
        eprintln!("{}", l.t("undo-nothing"));
        return false;
    };
    let journals = kor_tools::undo::list(&base);
    if list {
        if journals.is_empty() {
            println!("{}", l.t("undo-nothing"));
        }
        for j in &journals {
            println!(
                "{}",
                l.t_args(
                    "undo-list-line",
                    &args!("session" => j.session.clone(), "count" => j.paths)
                )
            );
        }
        return true;
    }
    let pick = match session {
        Some(id) => journals.iter().find(|j| j.session == id),
        None => journals.first(),
    };
    let Some(j) = pick else {
        eprintln!("{}", l.t("undo-nothing"));
        return false;
    };
    match kor_tools::undo::undo(&j.dir) {
        Ok(r) => {
            for p in &r.restored {
                println!("  ↺ {}", p.display());
            }
            for p in &r.deleted {
                println!("  ✗ {}", p.display());
            }
            for p in &r.skipped {
                println!(
                    "  ! {}",
                    l.t_args("undo-skipped", &args!("path" => p.display().to_string()))
                );
            }
            for (p, e) in &r.failed {
                eprintln!(
                    "  ✗ {}",
                    l.t_args(
                        "undo-failed",
                        &args!("path" => p.display().to_string(), "error" => e.clone())
                    )
                );
            }
            println!(
                "{}",
                l.t_args(
                    "undo-done",
                    &args!(
                        "session" => j.session.clone(),
                        "restored" => r.restored.len(),
                        "deleted" => r.deleted.len()
                    )
                )
            );
            r.failed.is_empty()
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("error-generic", &args!("error" => e.to_string()))
            );
            false
        }
    }
}
