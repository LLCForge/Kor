//! `kor db connect|refresh|tables|show` (spec §7.3, database-schema layer, ADR 0024). Schema
//! files are part of the project graph already; `connect` adds a live connection's schema from the
//! user's `connections.toml` (read-only catalog queries in `kor_agent::remote::db`), saved as
//! `.kor/graph/db/<name>.json`. Everything else reads the graph.

use std::path::Path;

use kor_graph::{Graph, TableInfo};
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, Clone, clap::Subcommand)]
pub enum DbCmd {
    /// Read a connection's schema (read-only) into the project graph.
    Connect {
        /// A `connections.toml` id, or a URL (`postgres://`, `mysql://`, `mssql://`, `sqlite:`).
        name: String,
        /// Id to save a URL under (default: from the database name).
        #[arg(long)]
        id: Option<String>,
        /// Also read the slowest statements (`pg_stat_statements`, `performance_schema`, `dm_exec_query_stats`).
        #[arg(long)]
        slow: bool,
    },
    /// Re-read the schema of one connected database, or of all of them.
    Refresh { name: Option<String> },
    /// Tables in the project graph.
    Tables,
    /// Columns, foreign keys, readers and writers of a table.
    Show { table: String },
    /// Indexes of connected databases and suggested new ones.
    Indexes,
    /// Migrations (`migrate new "<description>"`).
    Migrate {
        #[command(subcommand)]
        command: MigrateCmd,
    },
    /// Run an SQL file on a connection; destructive SQL is tried on a copy first.
    Apply {
        name: String,
        file: std::path::PathBuf,
        /// Approve non-destructive SQL without asking (destructive SQL always asks).
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum MigrateCmd {
    /// The agent writes a new migration in the project's framework style.
    New { description: String },
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-db"))
        .mut_subcommand("connect", |s| {
            s.about(l.t("cmd-db-connect"))
                .mut_arg("name", |a| a.help(l.t("arg-db-name")))
                .mut_arg("slow", |a| a.help(l.t("arg-db-slow")))
                .mut_arg("id", |a| a.help(l.t("arg-db-id")))
        })
        .mut_subcommand("refresh", |s| {
            s.about(l.t("cmd-db-refresh"))
                .mut_arg("name", |a| a.help(l.t("arg-db-name")))
        })
        .mut_subcommand("tables", |s| s.about(l.t("cmd-db-tables")))
        .mut_subcommand("show", |s| {
            s.about(l.t("cmd-db-show"))
                .mut_arg("table", |a| a.help(l.t("arg-db-table")))
        })
        .mut_subcommand("indexes", |s| s.about(l.t("cmd-db-indexes")))
        .mut_subcommand("migrate", |s| {
            s.about(l.t("cmd-db-migrate")).mut_subcommand("new", |n| {
                n.about(l.t("cmd-db-migrate-new"))
                    .mut_arg("description", |a| a.help(l.t("arg-db-migration")))
            })
        })
        .mut_subcommand("apply", |s| {
            s.about(l.t("cmd-db-apply"))
                .mut_arg("name", |a| a.help(l.t("arg-db-name")))
                .mut_arg("file", |a| a.help(l.t("arg-db-sql-file")))
                .mut_arg("yes", |a| a.help(l.t("arg-db-yes")))
        })
}

/// `kor db` without a subcommand lists the tables.
#[allow(clippy::too_many_lines)] // one dispatch arm per subcommand
pub fn run(ctx: &Ctx, cmd: Option<&DbCmd>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = crate::graph_cmd::root()?;
    let privacy =
        kor_runtime::privacy_mode() || ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
    match cmd {
        Some(DbCmd::Migrate {
            command: MigrateCmd::New { description },
        }) => {
            let style = kor_runtime::dbops::migration_style(&root, description);
            println!(
                "{}",
                l.t_args(
                    "db-migration-style",
                    &args!("framework" => style.framework.clone(), "dir" => style.dir.clone())
                )
            );
            return crate::repl::run(
                ctx,
                &crate::repl::RunArgs {
                    prompt: Some(kor_runtime::dbops::migration_prompt(&style, description)),
                    ..crate::repl::RunArgs::default()
                },
            );
        }
        Some(DbCmd::Apply { name, file, yes }) => {
            return apply(l, &root, name, file, *yes, !privacy);
        }
        Some(DbCmd::Indexes) => return Ok(indexes(l, &root)),
        _ => {}
    }
    let saved;
    let cmd = match cmd {
        Some(DbCmd::Connect { name, id, slow }) if is_url(name) => {
            let Some(name) = save_url(l, name, id.as_deref()) else {
                return Ok(false);
            };
            saved = DbCmd::Connect {
                name,
                id: None,
                slow: *slow,
            };
            Some(&saved)
        }
        c => c,
    };
    // Slow queries are opt-in: `connect --slow`; `refresh` re-reads them where they were read.
    let snaps = kor_graph::db::load_snapshots(&root);
    let had_slow = |n: &str| {
        snaps
            .iter()
            .any(|s| s.connection == n && !s.slow_queries.is_empty())
    };
    let names: Vec<(String, bool)> = match cmd {
        Some(DbCmd::Connect { name, slow, .. }) => vec![(name.clone(), *slow)],
        Some(DbCmd::Refresh { name: Some(name) }) => vec![(name.clone(), had_slow(name))],
        Some(DbCmd::Refresh { name: None }) => snaps
            .iter()
            .map(|s| (s.connection.clone(), !s.slow_queries.is_empty()))
            .collect(),
        _ => Vec::new(),
    };
    if matches!(cmd, Some(DbCmd::Refresh { name: None })) && names.is_empty() {
        println!("{}", l.t("db-none-connected"));
        return Ok(true);
    }
    let mut ok = true;
    if !names.is_empty() {
        let rt = crate::providers::runtime()?;
        for (name, slow) in &names {
            ok &= connect(l, &rt, &root, name, !privacy, *slow);
        }
    }
    let g = match crate::graph_cmd::open(&root) {
        Ok(g) => g,
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("graph-error", &args!("error" => e.to_string()))
            );
            return Ok(false);
        }
    };
    Ok(match cmd {
        Some(DbCmd::Show { table }) => show(l, &g, table),
        Some(DbCmd::Tables) | None => {
            tables(l, &g);
            true
        }
        Some(
            DbCmd::Connect { .. }
            | DbCmd::Refresh { .. }
            | DbCmd::Indexes
            | DbCmd::Migrate { .. }
            | DbCmd::Apply { .. },
        ) => {
            println!(
                "{}",
                l.t_args("db-graph-tables", &args!("count" => g.tables().len()))
            );
            ok
        }
    })
}

fn is_url(s: &str) -> bool {
    s.contains("://") || s.starts_with("sqlite:")
}

/// `kor db connect <url>`: save the entry to `connections.toml` and the password to the keychain.
fn save_url(l: &Localizer, url: &str, id: Option<&str>) -> Option<String> {
    let fail = |e: String| {
        eprintln!("{}", l.t_args("db-url-failed", &args!("error" => e)));
        None
    };
    let Some(file) = kor_config::user_dir().map(|d| d.join("connections.toml")) else {
        return fail("no user configuration directory".into());
    };
    let entry = match kor_agent::remote::url::parse(url, id) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    if let Some(pw) = &entry.password {
        let secret = kor_secrets::SecretString::from(pw.clone());
        if let Err(e) = kor_secrets::store_key(
            &format!("db-{}", entry.id),
            &secret,
            &kor_secrets::KeychainStore,
        ) {
            return fail(format!(
                "the password could not be stored in the keychain: {e}"
            ));
        }
    }
    if let Err(e) = kor_agent::remote::url::save(&file, &entry) {
        return fail(e);
    }
    println!(
        "{}",
        l.t_args(
            "db-url-saved",
            &args!("id" => entry.id.clone(), "path" => file.display().to_string(),
                   "keychain" => if entry.password.is_some() { "yes" } else { "no" })
        )
    );
    Some(entry.id)
}

fn connect(
    l: &Localizer,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    name: &str,
    allow_network: bool,
    slow: bool,
) -> bool {
    let file = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let result = rt
        .block_on(async {
            let mut snap =
                kor_agent::remote::db::introspect(file.clone(), name, root, allow_network).await?;
            if slow {
                match kor_agent::remote::db::slow_queries(file, name, allow_network).await {
                    Ok(q) => snap.slow_queries = q,
                    Err(e) => eprintln!("{}", l.t_args("db-slow-failed", &args!("error" => e))),
                }
            }
            Ok::<_, String>(snap)
        })
        .and_then(|snap| {
            kor_graph::db::save_snapshot(root, &snap)
                .map(|path| (snap, path))
                .map_err(|e| e.to_string())
        });
    match result {
        Ok((snap, path)) => {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            println!(
                "{}",
                l.t_args(
                    "db-connected",
                    &args!("name" => name, "kind" => snap.kind.clone(),
                           "count" => snap.tables.len(), "path" => rel.display().to_string())
                )
            );
            true
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("db-connect-failed", &args!("name" => name, "error" => e))
            );
            false
        }
    }
}

fn tables(l: &Localizer, g: &Graph) {
    let all = g.tables();
    if all.is_empty() {
        println!("{}", l.t("db-no-tables"));
        return;
    }
    println!(
        "{}",
        l.t_args("db-tables-head", &args!("count" => all.len()))
    );
    for t in &all {
        println!("  {}", summary(t));
    }
}

fn summary(t: &TableInfo) -> String {
    let source = match (&t.node.file, t.live.is_empty()) {
        (Some(f), true) => f.clone(),
        (Some(f), false) => format!("{f}, db:{}", t.live.join(",")),
        (None, _) => format!("db:{}", t.live.join(",")),
    };
    format!(
        "{:<32} {:>3} col  {:>3} r  {:>3} w  {source}",
        t.node.name,
        t.columns.len(),
        t.readers.len(),
        t.writers.len()
    )
}

fn show(l: &Localizer, g: &Graph, name: &str) -> bool {
    let Some(t) = g.table(name) else {
        println!("{}", l.t_args("db-table-not-found", &args!("name" => name)));
        return false;
    };
    println!("{}", summary(&t));
    println!(
        "{}",
        l.t_args("db-columns", &args!("count" => t.columns.len()))
    );
    for c in &t.columns {
        println!("    {c}");
    }
    for (key, refs) in [
        ("db-references", &t.references),
        ("db-referenced-by", &t.referenced_by),
    ] {
        if !refs.is_empty() {
            let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
            println!("{}", l.t_args(key, &args!("list" => names.join(", "))));
        }
    }
    if !t.indexes.is_empty() {
        println!(
            "{}",
            l.t_args("db-indexes", &args!("list" => t.indexes.join("; ")))
        );
    }
    for q in t.slow_queries.iter().take(10) {
        #[allow(clippy::cast_precision_loss)]
        let ms = q.mean_us as f64 / 1000.0;
        let query: String = q.query.chars().take(200).collect();
        println!(
            "{}",
            l.t_args(
                "db-slow-query",
                &args!("ms" => format!("{ms:.1}"), "calls" => q.calls, "query" => query)
            )
        );
    }
    for (key, links) in [("db-readers", &t.readers), ("db-writers", &t.writers)] {
        if links.is_empty() {
            continue;
        }
        println!("{}", l.t_args(key, &args!("count" => links.len())));
        for k in links.iter().take(50) {
            let loc = match (&k.node.file, k.node.line) {
                (Some(f), Some(line)) if f != &k.node.name => format!("  {f}:{line}"),
                _ => String::new(),
            };
            println!(
                "    {} {}{loc}  [{}]",
                k.node.kind.as_str(),
                k.node.name,
                k.confidence.as_str()
            );
        }
    }
    true
}

fn indexes(l: &Localizer, root: &Path) -> bool {
    let snaps = kor_graph::db::load_snapshots(root);
    if snaps.is_empty() {
        println!("{}", l.t("db-none-connected"));
        return true;
    }
    for s in &snaps {
        for t in s.tables.iter().filter(|t| !t.indexes.is_empty()) {
            println!("{}.{}: {}", s.connection, t.name, t.indexes.join("; "));
        }
    }
    let sug = kor_runtime::dbops::suggest_indexes(&snaps);
    if sug.is_empty() {
        println!("{}", l.t("db-no-index-suggestions"));
    } else {
        println!(
            "{}",
            l.t_args("db-index-suggestions", &args!("count" => sug.len()))
        );
        for s in sug {
            println!("  {}  -- {} ({})", s.sql, s.reason, s.connection);
        }
    }
    true
}

fn red(s: &str) -> String {
    use std::io::IsTerminal as _;
    if std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal() {
        format!("\x1b[31m{s}\x1b[0m")
    } else {
        s.to_owned()
    }
}

/// `kor db apply`: destructive SQL asks, runs on a copy, shows the schema change and asks again
/// (`--yes` never covers it); other SQL asks once.
fn apply(
    l: &Localizer,
    root: &Path,
    name: &str,
    file: &Path,
    yes: bool,
    allow_network: bool,
) -> anyhow::Result<bool> {
    let fail = |e: String| {
        eprintln!(
            "{}",
            l.t_args("db-connect-failed", &args!("name" => name, "error" => e))
        );
        Ok(false)
    };
    let sql = match std::fs::read_to_string(file) {
        Ok(s) if s.len() <= 1024 * 1024 => s,
        Ok(_) => return fail("the SQL file is larger than 1 MB".into()),
        Err(e) => return fail(format!("{}: {e}", file.display())),
    };
    let conns = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let destructive = kor_agent::remote::db::destructive_statements(&sql);
    let rt = crate::providers::runtime()?;
    let ask_apply = l.t_args("db-apply-ask", &args!("name" => name));
    if destructive.is_empty() {
        if !crate::shell_cmd::confirm(l, &ask_apply, yes) {
            return Ok(false);
        }
    } else {
        println!(
            "{}",
            red(&l.t_args("db-destructive", &args!("count" => destructive.len())))
        );
        for d in &destructive {
            println!("  {}", red(d));
        }
        if !crate::shell_cmd::confirm(l, &l.t("db-trial-ask"), false) {
            return Ok(false);
        }
        let trial = kor_agent::remote::db::trial(conns.clone(), name, root, &sql, allow_network);
        match rt.block_on(trial) {
            Ok(change) => {
                println!("{}", l.t("db-trial-ok"));
                for c in &change {
                    println!("  {c}");
                }
            }
            Err(e) => return fail(e),
        }
        if !crate::shell_cmd::confirm(l, &ask_apply, false) {
            return Ok(false);
        }
    }
    let run = kor_agent::remote::db::apply(conns, name, root, &sql, allow_network);
    match rt.block_on(run) {
        Ok(()) => {
            println!("{}", l.t_args("db-applied", &args!("name" => name)));
            Ok(true)
        }
        Err(e) => fail(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: Option<DbCmd>,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["x", "show", "users"]).unwrap();
        assert!(matches!(t.c, Some(DbCmd::Show { ref table }) if table == "users"));
        let t = T::try_parse_from(["x", "refresh"]).unwrap();
        assert!(matches!(t.c, Some(DbCmd::Refresh { name: None })));
        let t = T::try_parse_from(["x", "migrate", "new", "add email"]).unwrap();
        assert!(matches!(
            t.c,
            Some(DbCmd::Migrate { command: MigrateCmd::New { ref description } }) if description == "add email"
        ));
        let t = T::try_parse_from(["x", "connect", "main", "--slow"]).unwrap();
        assert!(matches!(t.c, Some(DbCmd::Connect { slow: true, .. })));
        let t = T::try_parse_from(["x", "connect", "postgres://u@h/db", "--id", "main"]).unwrap();
        assert!(matches!(t.c, Some(DbCmd::Connect { id: Some(ref i), .. }) if i == "main"));
        assert!(is_url("sqlite:a.db") && is_url("mysql://h/d") && !is_url("main"));
        assert!(T::try_parse_from(["x", "apply", "main", "m.sql", "--yes"]).is_ok());
        let l = Localizer::new(kor_i18n::Lang::En);
        let c = localize(T::command(), &l);
        c.debug_assert();
    }

    #[test]
    fn shows_tables_from_schema_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("schema.sql"),
            "CREATE TABLE users (id int, email text);\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("repo.py"),
            "def add(c):\n    c.execute('INSERT INTO users (email) VALUES (1)')\n",
        )
        .unwrap();
        let g = crate::graph_cmd::open(dir.path()).unwrap();
        let l = Localizer::new(kor_i18n::Lang::En);
        assert!(show(&l, &g, "users"));
        assert!(!show(&l, &g, "nope"));
        let t = g.table("users").unwrap();
        assert!(summary(&t).contains("schema.sql"));
        assert_eq!(t.writers.len(), 1);
    }
}
