//! Project memory and the knowledge base (§6.1, §31.3): `kor memory [notes push|pull]`,
//! `kor context add|remove|projects`, `kor recall`, `kor forget`, `kor knowledge export`,
//! `kor knowledge reindex`, `kor workspace add|list|remove`. Logic lives in `kor-memory`; this
//! only prints.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use kor_i18n::args;
use kor_memory::{ExportFormat, Knowledge, ProjectMemory};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum MemoryCmd {
    /// Decisions as git notes (`refs/notes/kor`).
    Notes {
        #[command(subcommand)]
        command: NotesCmd,
    },
}

#[derive(Debug, Subcommand)]
pub enum NotesCmd {
    /// Store `decisions.md` as the repository's `kor` git note.
    Push,
    /// Add decisions from the repository's `kor` git note.
    Pull,
}

#[derive(Debug, Subcommand)]
pub enum ContextCmd {
    /// Link another project, so tasks can use both.
    Add { path: PathBuf },
    /// Unlink a project.
    Remove { path: PathBuf },
    /// Linked projects.
    Projects,
}

#[derive(Debug, clap::Args)]
pub struct RecallArgs {
    #[arg(required = true, num_args = 1..)]
    pub query: Vec<String>,
    /// Only this project (a folder; `.` = the current one).
    #[arg(long)]
    pub project: Option<PathBuf>,
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
}

#[derive(Debug, Subcommand)]
pub enum KnowledgeCmd {
    /// Write every record as Markdown (default, Obsidian-friendly) or JSON.
    Export {
        #[arg(long, conflicts_with = "json")]
        md: bool,
        #[arg(long)]
        json: bool,
        dir: Option<PathBuf>,
    },
    /// Embed every record that has no vector yet (local model, `embed-e5-small` component).
    Reindex,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCmd {
    /// Register a folder, or every folder a pattern matches (`~/projects/*`).
    Add {
        pattern: String,
    },
    List,
    Remove {
        path: String,
    },
}

/// Localize the clap texts of these commands (called from `localized_command`).
pub fn localize(c: clap::Command, l: &kor_i18n::Localizer) -> clap::Command {
    match c.get_name() {
        "memory" => c.about(l.t("cmd-memory")).mut_subcommand("notes", |s| {
            s.about(l.t("cmd-memory-notes"))
                .mut_subcommand("push", |p| p.about(l.t("cmd-memory-notes-push")))
                .mut_subcommand("pull", |p| p.about(l.t("cmd-memory-notes-pull")))
        }),
        "context" => c
            .about(l.t("cmd-context"))
            .mut_arg("history", |a| a.help(l.t("arg-context-history")))
            .mut_subcommand("add", |s| {
                s.about(l.t("cmd-context-add"))
                    .mut_arg("path", |a| a.help(l.t("arg-context-path")))
            })
            .mut_subcommand("remove", |s| {
                s.about(l.t("cmd-context-remove"))
                    .mut_arg("path", |a| a.help(l.t("arg-context-path")))
            })
            .mut_subcommand("projects", |s| s.about(l.t("cmd-context-projects"))),
        "recall" => c
            .about(l.t("cmd-recall"))
            .mut_arg("query", |a| a.help(l.t("arg-recall-query")))
            .mut_arg("project", |a| a.help(l.t("arg-recall-project")))
            .mut_arg("limit", |a| a.help(l.t("arg-recall-limit"))),
        "forget" => c
            .about(l.t("cmd-forget"))
            .mut_arg("id", |a| a.help(l.t("arg-forget-id"))),
        "knowledge" => c
            .about(l.t("cmd-knowledge"))
            .mut_subcommand("export", |s| {
                s.about(l.t("cmd-knowledge-export"))
                    .mut_arg("md", |a| a.help(l.t("arg-knowledge-md")))
                    .mut_arg("json", |a| a.help(l.t("arg-knowledge-json")))
                    .mut_arg("dir", |a| a.help(l.t("arg-knowledge-dir")))
            })
            .mut_subcommand("reindex", |s| s.about(l.t("cmd-knowledge-reindex"))),
        "workspace" => c
            .about(l.t("cmd-workspace"))
            .mut_subcommand("add", |s| {
                s.about(l.t("cmd-workspace-add"))
                    .mut_arg("pattern", |a| a.help(l.t("arg-workspace-pattern")))
            })
            .mut_subcommand("list", |s| s.about(l.t("cmd-workspace-list")))
            .mut_subcommand("remove", |s| {
                s.about(l.t("cmd-workspace-remove"))
                    .mut_arg("path", |a| a.help(l.t("arg-workspace-path")))
            }),
        _ => c,
    }
}

fn root() -> PathBuf {
    kor_runtime::project_root().unwrap_or_default()
}

fn home(ctx: &Ctx) -> Option<PathBuf> {
    let h = kor_runtime::kor_home();
    if h.is_none() {
        eprintln!("{}", ctx.l.t("memory-no-home"));
    }
    h
}

fn fail(ctx: &Ctx, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        ctx.l
            .t_args("memory-error", &args!("error" => e.to_string()))
    );
    false
}

fn shown(p: &Path) -> String {
    kor_tools::display_path(p)
}

fn open_kb(ctx: &Ctx) -> Option<Knowledge> {
    let home = home(ctx)?;
    // With the local embedding model when installed (hybrid recall), FTS only otherwise.
    match kor_memory::open_knowledge(&home) {
        Ok(k) => Some(k),
        Err(e) => {
            fail(ctx, e);
            None
        }
    }
}

/// `kor memory [notes push|pull]`.
pub fn memory(ctx: &Ctx, command: Option<&MemoryCmd>) -> bool {
    let Some(MemoryCmd::Notes { command }) = command else {
        let ok = crate::extra::memory(ctx);
        if let Some(home) = home(ctx) {
            let mem = ProjectMemory::new(&home, &root());
            if mem.ensure().is_ok() {
                println!(
                    "{}",
                    ctx.l
                        .t_args("memory-dir", &args!("path" => shown(&mem.dir)))
                );
            }
        }
        return ok;
    };
    let enabled = ctx.config.as_ref().is_ok_and(|c| c.config.memory.git_notes);
    if !enabled {
        println!("{}", ctx.l.t("memory-notes-off"));
        return false;
    }
    let Some(home) = home(ctx) else {
        return false;
    };
    let root = root();
    let mem = ProjectMemory::new(&home, &root);
    match command {
        NotesCmd::Push => match kor_memory::notes::push(&root, &mem) {
            Ok(commit) => {
                println!(
                    "{}",
                    ctx.l
                        .t_args("memory-notes-pushed", &args!("commit" => commit))
                );
                true
            }
            Err(e) => fail(ctx, e),
        },
        NotesCmd::Pull => match kor_memory::notes::pull(&root, &mem) {
            Ok(n) => {
                println!(
                    "{}",
                    ctx.l.t_args("memory-notes-pulled", &args!("count" => n))
                );
                true
            }
            Err(e) => fail(ctx, e),
        },
    }
}

/// `kor context [--history N]` and `kor context add|remove|projects`.
pub fn context(ctx: &Ctx, history: usize, command: Option<&ContextCmd>) -> bool {
    let Some(command) = command else {
        return crate::context_cmd::run(ctx, history);
    };
    let Some(home) = home(ctx) else {
        return false;
    };
    let root = root();
    let mem = ProjectMemory::new(&home, &root);
    let l = &ctx.l;
    match command {
        ContextCmd::Add { path } => match mem.link(&root, path) {
            Ok(Some(p)) => {
                println!(
                    "{}",
                    l.t_args("context-linked", &args!("path" => shown(&p)))
                );
                true
            }
            Ok(None) => {
                println!(
                    "{}",
                    l.t_args("context-already-linked", &args!("path" => shown(path)))
                );
                true
            }
            Err(e) => fail(ctx, format!("{}: {e}", shown(path))),
        },
        ContextCmd::Remove { path } => match mem.unlink(path) {
            Ok(true) => {
                println!(
                    "{}",
                    l.t_args("context-unlinked", &args!("path" => shown(path)))
                );
                true
            }
            Ok(false) => {
                println!(
                    "{}",
                    l.t_args("context-not-linked", &args!("path" => shown(path)))
                );
                false
            }
            Err(e) => fail(ctx, e),
        },
        ContextCmd::Projects => {
            let links = mem.linked_projects();
            if links.is_empty() {
                println!("{}", l.t("context-projects-none"));
            }
            for p in links {
                println!("{}", shown(&p));
            }
            true
        }
    }
}

/// The `--project` filter as a stored project key (`.` = the current project).
fn project_filter(p: &Path) -> String {
    let p = if p == Path::new(".") {
        root()
    } else {
        p.to_owned()
    };
    kor_memory::project_key(&p.canonicalize().unwrap_or(p))
}

/// `kor recall "<query>"`.
pub fn recall(ctx: &Ctx, a: &RecallArgs) -> bool {
    let Some(kb) = open_kb(ctx) else {
        return false;
    };
    let only = a.project.as_deref().map(project_filter);
    let here = kor_memory::project_key(&root());
    let query = a.query.join(" ");
    match kb.recall(&query, only.as_deref(), Some(&here), a.limit.clamp(1, 100)) {
        Ok(hits) if hits.is_empty() => {
            println!("{}", ctx.l.t("recall-none"));
            true
        }
        Ok(hits) => {
            if let Some(home) = kor_runtime::kor_home() {
                let ids: Vec<i64> = hits.iter().map(|h| h.record.id).collect();
                let _ = kor_memory::digest::record_uses(&home, &ids);
            }
            for h in hits {
                let r = h.record;
                let date = kor_memory::kb::date(r.ts);
                let project = r.project.rsplit('/').next().unwrap_or(&r.project);
                println!("#{}  {date}  {project}  {}", r.id, r.task);
                for line in r.summary.lines().take(3) {
                    println!("    {line}");
                }
                for e in r.errors.iter().take(2) {
                    println!("    ! {e}");
                }
            }
            true
        }
        Err(e) => fail(ctx, e),
    }
}

/// `kor forget <id>`.
pub fn forget(ctx: &Ctx, id: i64) -> bool {
    let Some(kb) = open_kb(ctx) else {
        return false;
    };
    match kb.forget(id) {
        Ok(true) => {
            println!("{}", ctx.l.t_args("forget-done", &args!("id" => id)));
            true
        }
        Ok(false) => {
            println!("{}", ctx.l.t_args("forget-missing", &args!("id" => id)));
            false
        }
        Err(e) => fail(ctx, e),
    }
}

/// `kor knowledge reindex`: embed the records that have no vector (with progress).
fn reindex(ctx: &Ctx, kb: &Knowledge) -> bool {
    if !kb.semantic() {
        println!(
            "{}",
            ctx.l.t_args(
                "knowledge-no-model",
                &args!("name" => kor_memory::embed::MODEL_COMPONENT)
            )
        );
        return false;
    }
    let show = |done: usize, total: usize| {
        eprint!("\r{done}/{total}");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    };
    let result = kb.reindex(&show);
    eprintln!();
    match result {
        Ok(n) => {
            println!(
                "{}",
                ctx.l.t_args(
                    "knowledge-reindexed",
                    &args!("count" => n, "total" => kb.embedded().unwrap_or(0))
                )
            );
            true
        }
        Err(e) => fail(ctx, e),
    }
}

/// `kor knowledge export [--md|--json] [dir]` and `kor knowledge reindex`.
pub fn knowledge(ctx: &Ctx, command: &KnowledgeCmd) -> bool {
    let Some(kb) = open_kb(ctx) else {
        return false;
    };
    let KnowledgeCmd::Export { md: _, json, dir } = command else {
        return reindex(ctx, &kb);
    };
    let dir = dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("kor-knowledge"));
    let format = if *json {
        ExportFormat::Json
    } else {
        ExportFormat::Markdown
    };
    match kb.export(&dir, format) {
        Ok(n) => {
            println!(
                "{}",
                ctx.l.t_args(
                    "knowledge-exported",
                    &args!("count" => n, "path" => shown(&dir))
                )
            );
            true
        }
        Err(e) => fail(ctx, e),
    }
}

/// `kor workspace add|list|remove`.
pub fn workspace(ctx: &Ctx, command: &WorkspaceCmd) -> bool {
    let Some(kb) = open_kb(ctx) else {
        return false;
    };
    let l = &ctx.l;
    match command {
        WorkspaceCmd::Add { pattern } => match kb.workspace_add(pattern) {
            Ok(added) => {
                for p in added {
                    println!(
                        "{}",
                        l.t_args("workspace-added", &args!("path" => shown(&p)))
                    );
                }
                true
            }
            Err(e) => fail(ctx, e),
        },
        WorkspaceCmd::List => match kb.workspaces() {
            Ok(list) if list.is_empty() => {
                println!("{}", l.t("workspace-none"));
                true
            }
            Ok(list) => {
                for p in list {
                    println!("{p}");
                }
                true
            }
            Err(e) => fail(ctx, e),
        },
        WorkspaceCmd::Remove { path } => match kb.workspace_remove(path) {
            Ok(true) => {
                println!(
                    "{}",
                    l.t_args("workspace-removed", &args!("path" => path.clone()))
                );
                true
            }
            Ok(false) => {
                println!(
                    "{}",
                    l.t_args("workspace-missing", &args!("path" => path.clone()))
                );
                false
            }
            Err(e) => fail(ctx, e),
        },
    }
}
