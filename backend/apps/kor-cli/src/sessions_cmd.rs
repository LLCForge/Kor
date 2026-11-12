//! `kor sessions` session tree (Master Build Prompt §28.6, ADR 0050): folders, tags, pin, archive,
//! colour, full-text + semantic search, title/tag suggestions, Markdown export/import. The records
//! are the desktop sidebar's own (`desktop.json`), so both surfaces show the same tree.
//! Logic: `kor_runtime::session_tree`.

use std::fmt::Write as _;

use clap::{Args, Subcommand};
use kor_i18n::{Localizer, args};
use kor_runtime::session_tree::{self as tree, Entry, Filter};

use crate::Ctx;

/// `kor sessions fork|tree|compare|merge|unfork` (§27.3).
#[path = "fork_cmd.rs"]
pub mod fork;

#[derive(Debug, Args, Default, Clone)]
#[allow(clippy::struct_excessive_bools)] // one CLI flag each
pub struct FilterArgs {
    /// Only sessions with this tag (`auth` or `#auth`).
    #[arg(long)]
    pub tag: Option<String>,
    /// Only sessions in this folder.
    #[arg(long)]
    pub folder: Option<String>,
    /// Only pinned sessions.
    #[arg(long)]
    pub pinned: bool,
    /// Include archived sessions.
    #[arg(long)]
    pub archived: bool,
    /// Every session, not only the newest 30.
    #[arg(long)]
    pub all: bool,
    /// Machine-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum HistoryTool {
    ClaudeCode,
    Cursor,
}

#[derive(Debug, Args)]
pub struct SessionsArgs {
    #[command(flatten)]
    pub filter: FilterArgs,
    #[command(subcommand)]
    pub command: Option<SessionsCmd>,
}

#[derive(Debug, Subcommand)]
pub enum SessionsCmd {
    /// List sessions (same filters as `kor sessions`).
    Ls(FilterArgs),
    /// Full-text search (and by meaning, with the local embedding model).
    Search {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Move a session into a folder (created when missing); without a folder, out of it.
    Mv { id: String, folder: Option<String> },
    /// Pin (or `--off` unpin) a session.
    Pin {
        id: String,
        #[arg(long)]
        off: bool,
    },
    /// Archive (or `--off` restore) a session.
    Archive {
        id: String,
        #[arg(long)]
        off: bool,
    },
    /// Add tags (`--remove` to take them off).
    Tag {
        id: String,
        #[arg(required = true, num_args = 1..)]
        tags: Vec<String>,
        #[arg(long)]
        remove: bool,
    },
    /// Colour dot: red, orange, yellow, green, blue, purple, gray, or none.
    Color { id: String, color: String },
    /// Set the title (empty to go back to the first message).
    Rename {
        #[arg(index = 1)]
        id: String,
        #[arg(index = 2, num_args = 0..)]
        title: Vec<String>,
    },
    /// Suggested title and tags; `--apply` saves them.
    Suggest {
        id: String,
        #[arg(long)]
        apply: bool,
    },
    /// Write every session as Markdown into `<dir>/<folder>/`.
    Export { dir: std::path::PathBuf },
    /// Read a Markdown export back as a session.
    Import { file: std::path::PathBuf },
    /// Import chats from Claude Code (this project, or every project with --all) or Cursor.
    ImportFrom {
        #[arg(value_enum)]
        tool: HistoryTool,
        /// Claude Code: every project, not only the current folder.
        #[arg(long)]
        all: bool,
    },
    /// Conversation branches (§27.3): fork, tree, compare, merge, unfork.
    #[command(flatten)]
    Fork(fork::ForkCmd),
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let f = |c: clap::Command| {
        c.mut_arg("tag", |a| a.help(l.t("arg-sessions-tag")))
            .mut_arg("folder", |a| a.help(l.t("arg-sessions-folder")))
            .mut_arg("pinned", |a| a.help(l.t("arg-sessions-pinned")))
            .mut_arg("archived", |a| a.help(l.t("arg-sessions-archived")))
            .mut_arg("all", |a| a.help(l.t("arg-sessions-all")))
            .mut_arg("json", |a| a.help(l.t("arg-json")))
    };
    let id = |c: clap::Command| c.mut_arg("id", |a| a.help(l.t("arg-session-id")));
    fork::localize(f(c.about(l.t("cmd-sessions"))), l)
        .mut_subcommand("ls", |s| f(s.about(l.t("cmd-sessions-ls"))))
        .mut_subcommand("search", |s| {
            s.about(l.t("cmd-sessions-search"))
                .mut_arg("text", |a| a.help(l.t("arg-sessions-text")))
        })
        .mut_subcommand("mv", |s| {
            id(s.about(l.t("cmd-sessions-mv")))
                .mut_arg("folder", |a| a.help(l.t("arg-sessions-folder")))
        })
        .mut_subcommand("pin", |s| {
            id(s.about(l.t("cmd-sessions-pin"))).mut_arg("off", |a| a.help(l.t("arg-sessions-off")))
        })
        .mut_subcommand("archive", |s| {
            id(s.about(l.t("cmd-sessions-archive")))
                .mut_arg("off", |a| a.help(l.t("arg-sessions-off")))
        })
        .mut_subcommand("tag", |s| {
            id(s.about(l.t("cmd-sessions-tag")))
                .mut_arg("remove", |a| a.help(l.t("arg-sessions-remove")))
        })
        .mut_subcommand("color", |s| id(s.about(l.t("cmd-sessions-color"))))
        .mut_subcommand("rename", |s| id(s.about(l.t("cmd-sessions-rename"))))
        .mut_subcommand("suggest", |s| {
            id(s.about(l.t("cmd-sessions-suggest")))
                .mut_arg("apply", |a| a.help(l.t("arg-sessions-apply")))
        })
        .mut_subcommand("export", |s| s.about(l.t("cmd-sessions-export")))
        .mut_subcommand("import", |s| s.about(l.t("cmd-sessions-import")))
        .mut_subcommand("import-from", |s| {
            s.about(l.t("cmd-sessions-import-from"))
                .mut_arg("all", |a| a.help(l.t("arg-sessions-import-all")))
        })
}

fn dirs() -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let s = kor_runtime::sessions_dir().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    let m = tree::meta_path().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    Ok((s, m))
}

fn line(e: &Entry) -> String {
    let mut s = format!("{}  {}", e.id, e.title);
    if e.pinned {
        s.insert_str(0, "* ");
    } else {
        s.insert_str(0, "  ");
    }
    if let Some(f) = &e.folder {
        let _ = write!(s, "  [{f}]");
    }
    for t in &e.tags {
        let _ = write!(s, " #{t}");
    }
    if let Some(c) = &e.color {
        let _ = write!(s, "  ({c})");
    }
    if e.archived {
        s.push_str("  ~");
    }
    s
}

fn list(l: &Localizer, f: &FilterArgs) -> anyhow::Result<bool> {
    let (s, m) = dirs()?;
    let entries = tree::list(
        &s,
        &m,
        &Filter {
            tag: f.tag.clone(),
            folder: f.folder.clone(),
            pinned: f.pinned,
            archived: f.archived,
        },
    )?;
    let shown: Vec<&Entry> = entries
        .iter()
        .take(if f.all { usize::MAX } else { 30 })
        .collect();
    if f.json {
        println!("{}", serde_json::to_string_pretty(&shown)?);
        return Ok(true);
    }
    if shown.is_empty() {
        println!("{}", l.t("session-none"));
    }
    for e in shown {
        println!("{}", line(e));
    }
    Ok(true)
}

fn done(l: &Localizer, id: &str) -> bool {
    println!(
        "{}",
        l.t_args("sessions-updated", &args!("id" => id.to_owned()))
    );
    true
}

/// The local embedding model, when installed (never a remote one: transcripts stay local).
fn embedder() -> Option<std::sync::Arc<kor_memory::LocalEmbedder>> {
    kor_memory::LocalEmbedder::installed()
}

#[allow(clippy::too_many_lines)] // one arm per `kor sessions` subcommand
pub fn run(ctx: &Ctx, args: &SessionsArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(cmd) = &args.command else {
        return list(l, &args.filter);
    };
    let (sd, md) = dirs()?;
    let result = match cmd {
        SessionsCmd::Fork(f) => return fork::run(ctx, f),
        SessionsCmd::Ls(f) => return list(l, f),
        SessionsCmd::Search { text, json } => {
            let query = text.join(" ");
            let model = embedder();
            let hits = tree::search(
                &sd,
                &md,
                &query,
                20,
                model.as_deref().map(|e| e as &dyn kor_memory::Embed),
            )?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else if hits.is_empty() {
                println!(
                    "{}",
                    l.t_args("sessions-search-none", &args!("text" => query))
                );
            } else {
                for h in &hits {
                    println!(
                        "{}  {}{}\n    {}",
                        h.id,
                        h.title,
                        if h.semantic { "  ≈" } else { "" },
                        h.snippet
                    );
                }
            }
            return Ok(true);
        }
        SessionsCmd::Mv { id, folder } => {
            tree::move_to(&sd, &md, id, folder.as_deref()).map(|()| done(l, id))
        }
        SessionsCmd::Pin { id, off } => tree::pin(&sd, &md, id, !off).map(|()| done(l, id)),
        SessionsCmd::Archive { id, off } => tree::archive(&sd, &md, id, !off).map(|()| done(l, id)),
        SessionsCmd::Tag { id, tags, remove } => {
            let (add, rm) = if *remove {
                (&[][..], &tags[..])
            } else {
                (&tags[..], &[][..])
            };
            tree::tag(&sd, &md, id, add, rm).map(|t| {
                println!(
                    "{}  {}",
                    id,
                    t.iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                true
            })
        }
        SessionsCmd::Color { id, color } => {
            let c = (color != "none").then_some(color.as_str());
            tree::color(&sd, &md, id, c).map(|()| done(l, id))
        }
        SessionsCmd::Rename { id, title } => {
            let t = title.join(" ");
            tree::rename(&sd, &md, id, Some(&t)).map(|()| done(l, id))
        }
        SessionsCmd::Suggest { id, apply } => {
            if !tree::valid_id(id) {
                return Err(tree::TreeError::UnknownSession(id.clone()).into());
            }
            let msgs = kor_agent::session::load(&sd.join(format!("{id}.jsonl")))
                .map_err(|_| tree::TreeError::UnknownSession(id.clone()))?;
            let sug = tree::suggest(&msgs);
            println!(
                "{}",
                l.t_args(
                    "sessions-suggest",
                    &args!("title" => sug.title.clone(), "tags" => sug.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "))
                )
            );
            if *apply {
                tree::rename(&sd, &md, id, Some(&sug.title))?;
                tree::tag(&sd, &md, id, &sug.tags, &[])?;
                return Ok(done(l, id));
            }
            Ok(true)
        }
        SessionsCmd::Export { dir } => tree::export_markdown(&sd, &md, dir).map(|n| {
            println!(
                "{}",
                l.t_args(
                    "sessions-exported",
                    &args!("count" => n, "dir" => dir.display().to_string())
                )
            );
            true
        }),
        SessionsCmd::Import { file } => tree::import_markdown(&sd, &md, file).map(|id| {
            println!("{}", l.t_args("sessions-imported", &args!("id" => id)));
            true
        }),
        SessionsCmd::ImportFrom { tool, all } => {
            use kor_runtime::session_import as si;
            let chats = match tool {
                HistoryTool::ClaudeCode => {
                    let home =
                        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
                    let root = std::env::current_dir()?;
                    Ok(si::claude_code(&home, &root, *all))
                }
                HistoryTool::Cursor => {
                    let db = dirs::config_dir()
                        .map(|c| si::cursor_db(&c))
                        .filter(|p| p.is_file());
                    db.ok_or_else(|| "Cursor".to_owned())
                        .and_then(|db| si::cursor(&db))
                }
            };
            let chats = chats
                .map_err(|e| anyhow::anyhow!(l.t_args("sessions-error", &args!("error" => e))))?;
            // §3.3: this project's Claude Code chats also leave a summary in its memory (not
            // `--all`, whose chats belong to other projects; not Cursor, which is not per project).
            if matches!(tool, HistoryTool::ClaudeCode)
                && !*all
                && let (Some(home), Ok(root)) = (kor_runtime::kor_home(), std::env::current_dir())
                && let Err(e) = si::remember(&home, &root, &si::fresh(&sd, &chats), "Claude Code")
            {
                eprintln!(
                    "{}",
                    l.t_args("sessions-error", &args!("error" => e.to_string()))
                );
            }
            si::save_new(&sd, &chats)
                .map_err(tree::TreeError::from)
                .map(|n| {
                    println!(
                        "{}",
                        l.t_args(
                            "sessions-imported-from",
                            &args!("count" => n, "found" => chats.len())
                        )
                    );
                    true
                })
        }
    };
    result
        .map_err(|e| anyhow::anyhow!(l.t_args("sessions-error", &args!("error" => e.to_string()))))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(clap::Subcommand)]
    enum Top {
        Sessions(SessionsArgs),
    }

    fn parse(argv: &[&str]) -> SessionsArgs {
        let Top::Sessions(a) = Cli::try_parse_from(argv).unwrap().cmd;
        a
    }

    #[test]
    fn parses_tree_commands() {
        let a = parse(&["kor", "sessions", "--tag", "auth"]);
        assert_eq!(a.filter.tag.as_deref(), Some("auth"));
        assert!(a.command.is_none());
        assert!(
            matches!(parse(&["kor", "sessions", "ls", "--tag", "bug"]).command, Some(SessionsCmd::Ls(ref f)) if f.tag.as_deref() == Some("bug"))
        );
        assert!(
            matches!(parse(&["kor", "sessions", "search", "redis", "timeout"]).command, Some(SessionsCmd::Search { ref text, .. }) if text.len() == 2)
        );
        assert!(
            matches!(parse(&["kor", "sessions", "mv", "abc", "Backend"]).command, Some(SessionsCmd::Mv { ref folder, .. }) if folder.as_deref() == Some("Backend"))
        );
        assert!(matches!(
            parse(&["kor", "sessions", "pin", "abc", "--off"]).command,
            Some(SessionsCmd::Pin { off: true, .. })
        ));
        assert!(matches!(
            parse(&["kor", "sessions", "tag", "abc", "a", "b", "--remove"]).command,
            Some(SessionsCmd::Tag { remove: true, .. })
        ));
        assert!(Cli::try_parse_from(["kor", "sessions", "search"]).is_err());
        let l = Localizer::new(kor_i18n::Lang::Fr);
        let _ = localize(
            <Cli as clap::CommandFactory>::command()
                .find_subcommand("sessions")
                .unwrap()
                .clone(),
            &l,
        );
    }
}
