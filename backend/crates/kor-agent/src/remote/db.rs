//! `db_schema` and `db_query`. Every query runs read-only on the database itself, not just by a
//! keyword check here:
//! - `SQLite` opens the file read-only (`SQLITE_OPEN_READ_ONLY`, `query_only`) and refuses any
//!   statement `SQLite` does not mark read-only.
//! - Postgres runs through `psql` with `default_transaction_read_only=on` and a statement timeout.
//! - `MySQL` runs through `mysql` with `SET SESSION TRANSACTION READ ONLY`.
//!
//! Only one statement is accepted, so a query cannot switch the session back to read-write first.

use kor_platform::NoWindow as _;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use kor_secrets::ExposeSecret;
use kor_tools::ToolCtx;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Connections, DbConn, DbKind, Remote, UNTRUSTED, ssh_base_args};

const MAX_ROWS: usize = 1000;
const MAX_CELL: usize = 200;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

/// One statement whose first keyword reads. Semicolons are allowed only at the end.
pub(crate) fn check_read_only(sql: &str, sqlite: bool) -> Result<String, String> {
    let mut s = sql.trim();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    let s = s.trim_end().trim_end_matches(';').trim_end();
    if s.is_empty() {
        return Err("the SQL is empty".into());
    }
    if s.len() > 100_000 {
        return Err("the SQL is longer than 100000 characters".into());
    }
    if s.contains(';') {
        return Err("only one statement is allowed (no `;` inside the SQL)".into());
    }
    let first: String = s
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_lowercase();
    let mut allowed = vec![
        "select", "with", "values", "explain", "show", "table", "describe", "desc",
    ];
    if sqlite {
        allowed.push("pragma");
    }
    if !allowed.contains(&first.as_str()) {
        return Err(format!(
            "db_query only reads ({}); `{first}` is not allowed. Propose data changes to the user instead.",
            allowed.join(", ")
        ));
    }
    Ok(s.to_owned())
}

fn cell(v: &str) -> String {
    let one: String = v
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .take(MAX_CELL)
        .collect();
    if v.chars().count() > MAX_CELL {
        format!("{one}…")
    } else {
        one
    }
}

pub(crate) fn table(columns: &[String], rows: &[Vec<String>], more: bool) -> String {
    let mut out = format!("{UNTRUSTED}\n{}\n", columns.join(" | "));
    for r in rows {
        out.push_str(&r.join(" | "));
        out.push('\n');
    }
    if more {
        let _ = writeln!(
            out,
            "[first {MAX_ROWS} rows; narrow the query with WHERE or LIMIT]"
        );
    } else {
        let _ = writeln!(out, "({} rows)", rows.len());
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------------------------

pub(crate) fn sqlite_open(path: &Path) -> Result<rusqlite::Connection, String> {
    if !path.is_file() {
        return Err(format!("{} does not exist", path.display()));
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let c = rusqlite::Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    c.pragma_update(None, "query_only", true)
        .map_err(|e| e.to_string())?;
    c.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    Ok(c)
}

pub(crate) fn sqlite_value(v: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => cell(&String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => format!("<blob {} bytes>", b.len()),
    }
}

pub(crate) fn sqlite_query(path: &Path, sql: &str) -> Result<String, String> {
    let c = sqlite_open(path)?;
    let mut stmt = c.prepare(sql).map_err(|e| e.to_string())?;
    if !stmt.readonly() {
        return Err("SQLite reports this statement would write; db_query only reads".into());
    }
    let columns: Vec<String> = stmt.column_names().map(str::to_owned).collect();
    let n = columns.len();
    let mut rows = Vec::new();
    let mut more = false;
    let mut it = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = it.next().map_err(|e| e.to_string())? {
        if rows.len() >= MAX_ROWS {
            more = true;
            break;
        }
        rows.push(
            (0..n)
                .map(|i| row.get_ref(i).map(sqlite_value).unwrap_or_default())
                .collect(),
        );
    }
    Ok(table(&columns, &rows, more))
}

pub(crate) fn sqlite_schema(path: &Path) -> Result<String, String> {
    let c = sqlite_open(path)?;
    let mut tables = c
        .prepare("SELECT name, type FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| e.to_string())?;
    let names: Vec<(String, String)> = tables
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if names.is_empty() {
        return Ok("No tables.".into());
    }
    let mut out = format!("{UNTRUSTED}\n");
    for (name, kind) in names {
        let mut cols = c
            .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?1)")
            .map_err(|e| e.to_string())?;
        let list: Vec<String> = cols
            .query_map([&name], |r| {
                let (n, t, notnull, pk): (String, String, i64, i64) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                Ok(format!(
                    "{n} {t}{}{}",
                    if notnull == 1 { " not null" } else { "" },
                    if pk > 0 { " pk" } else { "" }
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        let _ = writeln!(out, "{name} ({kind}): {}", list.join(", "));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Servers (psql / mysql) and the SSH tunnel
// ---------------------------------------------------------------------------------------------

/// `psql` / `mysql` on PATH, or in the default Windows install folders (newest version).
fn client(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join(&exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if cfg!(windows) {
        let base = PathBuf::from(
            std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into()),
        );
        let parent = if name == "psql" {
            base.join("PostgreSQL")
        } else {
            base.join("MySQL")
        };
        let mut found: Vec<PathBuf> = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path().join("bin").join(&exe))
            .filter(|p| p.is_file())
            .collect();
        found.sort();
        return found.pop();
    }
    None
}

fn password(id: &str) -> Option<String> {
    let env = format!(
        "KOR_DB_{}_PASSWORD",
        id.to_ascii_uppercase().replace('-', "_")
    );
    kor_secrets::resolve_key(
        &format!("db-{id}"),
        &env,
        |k| std::env::var(k).ok(),
        &kor_secrets::KeychainStore,
    )
    .ok()
    .flatten()
    .map(|k| k.secret.expose_secret().to_owned())
}

struct Tunnel {
    _child: tokio::process::Child,
    port: u16,
}

async fn tunnel(
    conns: &Connections,
    ssh_id: &str,
    host: &str,
    port: u16,
) -> Result<Tunnel, String> {
    let h = conns.ssh_host(ssh_id)?;
    let local = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let mut cmd = tokio::process::Command::new(super::ssh_exe());
    cmd.arg("-N")
        .args(ssh_base_args(h))
        .args(["-o", "ExitOnForwardFailure=yes", "-L"])
        .arg(format!("127.0.0.1:{local}:{host}:{port}"))
        .arg("--")
        .arg(&h.host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.no_window();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start ssh: {e}"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", local))
            .await
            .is_ok()
        {
            return Ok(Tunnel {
                _child: child,
                port: local,
            });
        }
        if let Ok(Some(_)) = child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err).await;
            }
            return Err(format!("SSH tunnel failed: {}", super::friendly(&err)));
        }
        if tokio::time::Instant::now() > deadline {
            return Err("the SSH tunnel did not open in time".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// How a statement runs on a server: the agent's tools only ever read; `kor db apply` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exec {
    ReadOnly,
    /// Autocommit (needed for `CREATE/DROP DATABASE`).
    Write,
    /// One transaction (`psql -1`).
    WriteTx,
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn server_query(
    conns: &Connections,
    id: &str,
    d: &DbConn,
    sql: &str,
    max_rows: usize,
    cancel: &tokio_util::sync::CancellationToken,
    exec: Exec,
) -> Result<(Vec<String>, Vec<Vec<String>>, bool), String> {
    let (name, default_port) = match d.kind {
        DbKind::Postgres => ("psql", 5432),
        DbKind::Mysql => ("mysql", 3306),
        DbKind::Mssql if exec == Exec::ReadOnly => ("sqlcmd", 1433),
        DbKind::Mssql => return Err("Kor only reads the schema of SQL Server databases".into()),
        DbKind::Sqlite => unreachable!("sqlite is local"),
    };
    let exe = client(name).ok_or_else(|| {
        let product = match name {
            "psql" => "PostgreSQL",
            "mysql" => "MySQL",
            _ => "SQL Server (sqlcmd)",
        };
        format!("`{name}` is not installed. Install the {product} client (Kor's desktop app can install it under Settings > Runtimes) or add it to PATH.")
    })?;
    let mut host = d.host.clone().unwrap_or_else(|| "127.0.0.1".into());
    let mut port = d.port.unwrap_or(default_port);
    let _tunnel = match &d.ssh {
        Some(ssh) => {
            let t = tunnel(conns, ssh, &host, port).await?;
            host = "127.0.0.1".into();
            port = t.port;
            Some(t)
        }
        None => None,
    };
    let mut cmd = tokio::process::Command::new(exe);
    let pw = password(id);
    match d.kind {
        DbKind::Postgres => {
            cmd.args([
                "-X",
                "-q",
                "-v",
                "ON_ERROR_STOP=1",
                "--csv",
                "-h",
                &host,
                "-p",
                &port.to_string(),
            ]);
            if let Some(u) = &d.user {
                cmd.args(["-U", u]);
            }
            if let Some(db) = &d.database {
                cmd.args(["-d", db]);
            }
            if exec == Exec::WriteTx {
                cmd.arg("-1");
            }
            // SQL goes in on stdin as UTF-8: Windows would turn a `-c` argument into the ANSI
            // code page and break non-ASCII text.
            cmd.args(["-f", "-"])
                .env("PGCLIENTENCODING", "UTF8")
                .env(
                    "PGOPTIONS",
                    if exec == Exec::ReadOnly {
                        "-c default_transaction_read_only=on -c statement_timeout=30000"
                    } else {
                        "-c statement_timeout=30000"
                    },
                )
                .env("PGCONNECT_TIMEOUT", "15")
                .env("PGAPPNAME", "kor")
                .env("PGSSLMODE", d.sslmode.as_deref().unwrap_or("prefer"));
            match &pw {
                Some(p) => cmd.env("PGPASSWORD", p),
                None => cmd.env_remove("PGPASSWORD"),
            };
        }
        DbKind::Mysql => {
            cmd.args([
                "--batch",
                "--default-character-set=utf8mb4",
                "--connect-timeout=15",
                "-h",
                &host,
                "-P",
                &port.to_string(),
            ]);
            if exec == Exec::ReadOnly {
                cmd.arg("--init-command=SET SESSION TRANSACTION READ ONLY");
            }
            if let Some(u) = &d.user {
                cmd.args(["-u", u]);
            }
            if let Some(db) = &d.database {
                cmd.args(["-D", db]);
            }
            match &pw {
                Some(p) => cmd.env("MYSQL_PWD", p),
                None => cmd.env_remove("MYSQL_PWD"),
            };
        }
        DbKind::Mssql => {
            // -h -1: no header row, -W: trimmed, -s TAB, -b: non-zero exit on errors.
            cmd.args(["-S", &format!("{host},{port}"), "-b", "-W", "-h", "-1", "-s", "\t"]);
            match &d.user {
                Some(u) => cmd.args(["-U", u]),
                None => cmd.arg("-E"),
            };
            if let Some(db) = &d.database {
                cmd.args(["-d", db]);
            }
            match &pw {
                Some(p) => cmd.env("SQLCMDPASSWORD", p),
                None => cmd.env_remove("SQLCMDPASSWORD"),
            };
        }
        DbKind::Sqlite => {}
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.no_window();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {name}: {e}"))?;
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    let mut input = child.stdin.take().ok_or("no stdin")?;
    let script = if d.kind == DbKind::Mssql {
        format!("SET NOCOUNT ON;\n{sql};\nGO\n")
    } else {
        format!("{sql};\n")
    };
    let work = async {
        input
            .write_all(script.as_bytes())
            .await
            .map_err(|x| x.to_string())?;
        drop(input);
        let mut o = Vec::new();
        let mut e = Vec::new();
        let mut out_limited = (&mut out).take(MAX_BYTES as u64 + 1);
        let mut err_limited = (&mut err).take(64 * 1024);
        let (r1, _) = tokio::join!(
            out_limited.read_to_end(&mut o),
            err_limited.read_to_end(&mut e)
        );
        r1.map_err(|x| x.to_string())?;
        let truncated = o.len() > MAX_BYTES;
        let _ = child.start_kill();
        let status = child.wait().await.map_err(|x| x.to_string())?;
        Ok::<_, String>((o, e, status, truncated))
    };
    let (o, e, status, truncated) = tokio::select! {
        r = tokio::time::timeout(TIMEOUT, work) => r.map_err(|_| "the query timed out".to_owned())??,
        () = cancel.cancelled() => return Err("cancelled".into()),
    };
    if !status.success() && !truncated {
        let msg = String::from_utf8_lossy(&e).trim().to_owned();
        return Err(if msg.is_empty() {
            format!("{name} failed")
        } else {
            msg
        });
    }
    let text = String::from_utf8_lossy(&o);
    Ok(match d.kind {
        DbKind::Postgres => parse_csv(&text, truncated, max_rows),
        // `sqlcmd -h -1` prints no header row.
        DbKind::Mssql => parse_tsv(&format!("-\n{text}"), truncated, max_rows),
        _ => parse_tsv(&text, truncated, max_rows),
    })
}

/// RFC 4180 CSV as written by `psql --csv`.
fn parse_csv(text: &str, truncated: bool, max_rows: usize) -> (Vec<String>, Vec<Vec<String>>, bool) {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut field = String::new();
    let mut record = Vec::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (_, c) if c != '\r' => field.push(c),
            _ => {}
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    split_rows(records, truncated, max_rows)
}

/// `mysql --batch`: tab-separated, `\t` `\n` `\\` escaped, NULL as `NULL`.
fn parse_tsv(text: &str, truncated: bool, max_rows: usize) -> (Vec<String>, Vec<Vec<String>>, bool) {
    let records = text
        .lines()
        .map(|l| {
            l.split('\t')
                .map(|f| {
                    f.replace("\\t", " ")
                        .replace("\\n", " ")
                        .replace("\\\\", "\\")
                })
                .collect()
        })
        .collect();
    split_rows(records, truncated, max_rows)
}

fn split_rows(
    mut records: Vec<Vec<String>>,
    truncated: bool,
    max_rows: usize,
) -> (Vec<String>, Vec<Vec<String>>, bool) {
    if records.is_empty() {
        return (Vec::new(), Vec::new(), false);
    }
    let header = records.remove(0);
    let more = truncated || records.len() > max_rows;
    records.truncate(max_rows);
    let rows = records
        .into_iter()
        .map(|r| r.iter().map(|v| cell(v)).collect())
        .collect();
    (header, rows, more)
}

pub(crate) fn schema_sql(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => {
            "SELECT table_schema || '.' || table_name AS t, column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema, table_name, ordinal_position"
        }
        DbKind::Mssql => {
            "SELECT TABLE_SCHEMA + '.' + TABLE_NAME, COLUMN_NAME, DATA_TYPE, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION"
        }
        _ => {
            "SELECT table_name AS t, column_name, column_type, is_nullable FROM information_schema.columns WHERE table_schema = DATABASE() ORDER BY table_name, ordinal_position"
        }
    }
}

pub(crate) fn group_schema(rows: &[Vec<String>]) -> String {
    let mut out = format!("{UNTRUSTED}\n");
    let mut current = String::new();
    let mut cols: Vec<String> = Vec::new();
    let flush = |out: &mut String, t: &str, cols: &mut Vec<String>| {
        if !t.is_empty() {
            let _ = writeln!(out, "{t}: {}", cols.join(", "));
        }
        cols.clear();
    };
    for r in rows {
        let [t, c, ty, nullable] = [0, 1, 2, 3].map(|i| r.get(i).cloned().unwrap_or_default());
        if t != current {
            flush(&mut out, &current, &mut cols);
            current = t;
        }
        cols.push(format!(
            "{c} {ty}{}",
            if nullable == "NO" { " not null" } else { "" }
        ));
    }
    flush(&mut out, &current, &mut cols);
    if rows.is_empty() {
        out.push_str("No tables.\n");
    }
    out
}

pub(crate) async fn run(
    query: bool,
    remote: &Remote,
    connection: Option<String>,
    file: Option<String>,
    sql: Option<String>,
    ctx: &ToolCtx,
) -> Result<String, String> {
    let sql = if query {
        Some(
            sql.filter(|s| !s.trim().is_empty())
                .ok_or("sql is required")?,
        )
    } else {
        None
    };
    let (conns, target) = match (connection, file) {
        (Some(_), Some(_)) => return Err("give connection or file, not both".into()),
        (None, None) => return Err("connection (from remote_connections) or file (an SQLite file in the project) is required".into()),
        (None, Some(f)) => (Connections::default(), Err(ctx.resolve(&f)?)),
        (Some(id), None) => {
            let conns = remote.load()?;
            let d = conns
                .db
                .get(&id)
                .cloned()
                .ok_or_else(|| format!("no database connection `{id}`; call remote_connections"))?;
            (conns, Ok((id, d)))
        }
    };
    let sqlite_path = match &target {
        Err(path) => Some(path.clone()),
        Ok((_, d)) if d.kind == DbKind::Sqlite => {
            Some(ctx.resolve(d.file.as_deref().unwrap_or_default())?)
        }
        Ok(_) => None,
    };
    if let Some(path) = sqlite_path {
        let sql = sql.map(|s| check_read_only(&s, true)).transpose()?;
        return tokio::task::spawn_blocking(move || match sql {
            Some(s) => sqlite_query(&path, &s),
            None => sqlite_schema(&path),
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let Ok((id, d)) = target else {
        unreachable!("file targets are SQLite")
    };
    if query && d.kind == DbKind::Mssql {
        return Err("SQL Server connections are schema-only (db_schema): sqlcmd has no read-only session to run queries in".into());
    }
    let statement = match sql {
        Some(s) => {
            let s = check_read_only(&s, false)?;
            // psql and mysql read backslash commands from their input (`\!` runs a local shell).
            if s.contains('\\') {
                return Err("backslashes are not allowed in SQL sent to a database server".into());
            }
            s
        }
        None => schema_sql(d.kind).to_owned(),
    };
    let (columns, rows, more) =
        server_query(&conns, &id, &d, &statement, MAX_ROWS, &ctx.cancel, Exec::ReadOnly).await?;
    if query {
        Ok(table(&columns, &rows, more))
    } else {
        Ok(group_schema(&rows))
    }
}

// ---------------------------------------------------------------------------------------------
// Live schema for the project graph (`kor db connect`, ADR 0024)
// ---------------------------------------------------------------------------------------------

/// Catalog rows `kind ('c' column | 'f' foreign key), table, column, type | referenced table, ordinal`.
fn catalog_sql(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => {
            "SELECT 'c' AS k, (table_schema || '.' || table_name)::text AS t, column_name::text AS a, data_type::text AS b, ordinal_position::int AS o FROM information_schema.columns WHERE table_schema NOT IN ('pg_catalog','information_schema') \
             UNION ALL SELECT 'f', (n.nspname || '.' || c.relname)::text, a.attname::text, (fns.nspname || '.' || fc.relname)::text, 0 FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace JOIN pg_class fc ON fc.oid = k.confrelid JOIN pg_namespace fns ON fns.oid = fc.relnamespace JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = ANY (k.conkey) WHERE k.contype = 'f' \
             UNION ALL SELECT 'i', (n.nspname || '.' || t.relname)::text, ic.relname::text, coalesce((SELECT string_agg(a.attname, ',' ORDER BY k.ord) FROM unnest(x.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = k.attnum), 'expression') || CASE WHEN x.indisunique THEN ' unique' ELSE '' END, 0 FROM pg_index x JOIN pg_class t ON t.oid = x.indrelid JOIN pg_class ic ON ic.oid = x.indexrelid JOIN pg_namespace n ON n.oid = t.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema','pg_toast') \
             ORDER BY 2, 1, 5"
        }
        DbKind::Mssql => {
            "SELECT 'c', s.name + '.' + t.name, c.name, ty.name, c.column_id FROM sys.columns c JOIN sys.tables t ON t.object_id = c.object_id JOIN sys.schemas s ON s.schema_id = t.schema_id JOIN sys.types ty ON ty.user_type_id = c.user_type_id \
             UNION ALL SELECT 'f', s.name + '.' + t.name, c.name, rs.name + '.' + rt.name, 0 FROM sys.foreign_key_columns f JOIN sys.tables t ON t.object_id = f.parent_object_id JOIN sys.schemas s ON s.schema_id = t.schema_id JOIN sys.columns c ON c.object_id = f.parent_object_id AND c.column_id = f.parent_column_id JOIN sys.tables rt ON rt.object_id = f.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id \
             UNION ALL SELECT 'i', s.name + '.' + t.name, i.name, STRING_AGG(c.name, ',') WITHIN GROUP (ORDER BY ic.key_ordinal) + CASE WHEN i.is_unique = 1 THEN ' unique' ELSE '' END, 0 FROM sys.indexes i JOIN sys.tables t ON t.object_id = i.object_id JOIN sys.schemas s ON s.schema_id = t.schema_id JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0 JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE i.name IS NOT NULL GROUP BY s.name, t.name, i.name, i.is_unique \
             ORDER BY 2, 1, 5"
        }
        _ => {
            "SELECT 'c' AS k, table_name AS t, column_name AS a, column_type AS b, ordinal_position AS o FROM information_schema.columns WHERE table_schema = DATABASE() \
             UNION ALL SELECT 'f', table_name, column_name, referenced_table_name, 0 FROM information_schema.key_column_usage WHERE table_schema = DATABASE() AND referenced_table_name IS NOT NULL \
             UNION ALL SELECT 'i', table_name, index_name, CONCAT(GROUP_CONCAT(column_name ORDER BY seq_in_index), IF(MIN(non_unique) = 0, ' unique', '')), 0 FROM information_schema.statistics WHERE table_schema = DATABASE() GROUP BY table_name, index_name \
             ORDER BY 2, 1, 5"
        }
    }
}

/// `name (cols)` plus ` unique`, from a catalog `i` row (`b` = `cols[ unique]`).
fn index_label(name: &str, cols: &str) -> String {
    match cols.strip_suffix(" unique") {
        Some(c) => format!("{name} ({c}) unique"),
        None => format!("{name} ({cols})"),
    }
}

fn group_catalog(rows: &[Vec<String>]) -> Vec<kor_graph::LiveTable> {
    let mut out: Vec<kor_graph::LiveTable> = Vec::new();
    for r in rows {
        let [k, t, a, b] = [0, 1, 2, 3].map(|i| r.get(i).cloned().unwrap_or_default());
        if t.is_empty() {
            continue;
        }
        if out.last().is_none_or(|l| l.name != t) {
            out.push(kor_graph::LiveTable {
                name: t,
                ..kor_graph::LiveTable::default()
            });
        }
        let Some(cur) = out.last_mut() else { continue };
        match k.as_str() {
            "f" => {
                if !cur.references.contains(&b) {
                    cur.references.push(b);
                }
                if !cur.fk_columns.contains(&a) {
                    cur.fk_columns.push(a);
                }
            }
            "i" => cur.indexes.push(index_label(&a, &b)),
            _ => cur.columns.push(format!("{a} {b}")),
        }
    }
    out
}

fn sqlite_tables(path: &Path) -> Result<Vec<kor_graph::LiveTable>, String> {
    let c = sqlite_open(path)?;
    let names: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for name in names {
        let columns = c
            .prepare("SELECT name, type FROM pragma_table_info(?1)")
            .and_then(|mut s| {
                s.query_map([&name], |r| {
                    Ok(format!("{} {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect()
            })
            .map_err(|e| e.to_string())?;
        let references = c
            .prepare("SELECT DISTINCT \"table\" FROM pragma_foreign_key_list(?1)")
            .and_then(|mut s| s.query_map([&name], |r| r.get(0))?.collect())
            .map_err(|e| e.to_string())?;
        let fk_columns = c
            .prepare("SELECT DISTINCT \"from\" FROM pragma_foreign_key_list(?1)")
            .and_then(|mut s| s.query_map([&name], |r| r.get(0))?.collect())
            .map_err(|e| e.to_string())?;
        let indexes = c
            .prepare("SELECT l.name, (SELECT group_concat(i.name, ',') FROM (SELECT name FROM pragma_index_info(l.name) ORDER BY seqno) i) || CASE WHEN l.\"unique\" THEN ' unique' ELSE '' END FROM pragma_index_list(?1) l ORDER BY l.name")
            .and_then(|mut s| {
                s.query_map([&name], |r| {
                    let (n, cols): (String, Option<String>) = (r.get(0)?, r.get(1)?);
                    Ok(index_label(&n, cols.as_deref().unwrap_or("expression")))
                })?
                .collect()
            })
            .map_err(|e| e.to_string())?;
        out.push(kor_graph::LiveTable {
            name,
            columns,
            references,
            indexes,
            fk_columns,
        });
    }
    Ok(out)
}

/// Read the schema (tables, columns, foreign keys) of connection `id` from the user's
/// `connections.toml` for the project graph. Only catalog `SELECT`s run, on the same read-only
/// sessions as `db_schema` (`SQLite` opened read-only; Postgres with
/// `default_transaction_read_only` and a 30 s statement timeout; `MySQL` with a read-only session),
/// all under a 60 s limit. Passwords come from the keychain or `KOR_DB_<ID>_PASSWORD` and never
/// reach the snapshot. Database servers are only contacted when `allow_network` (privacy mode
/// off); a relative `SQLite` file is resolved against `root`.
pub async fn introspect(
    connections: Option<PathBuf>,
    id: &str,
    root: &Path,
    allow_network: bool,
) -> Result<kor_graph::DbSnapshot, String> {
    let file = connections.ok_or("no user configuration directory")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let conns = Connections::parse(&text)?;
    let d = conns
        .db
        .get(id)
        .cloned()
        .ok_or_else(|| format!("no database connection `{id}` in {}", file.display()))?;
    let (kind, tables) = match d.kind {
        DbKind::Sqlite => {
            let p = PathBuf::from(d.file.as_deref().unwrap_or_default());
            let path = if p.is_absolute() { p } else { root.join(p) };
            let tables = tokio::task::spawn_blocking(move || sqlite_tables(&path))
                .await
                .map_err(|e| e.to_string())??;
            ("sqlite", tables)
        }
        kind => {
            if !allow_network {
                return Err("privacy mode is on: Kor does not connect to database servers".into());
            }
            let cancel = tokio_util::sync::CancellationToken::new();
            let (_, rows, _) = server_query(
                &conns,
                id,
                &d,
                catalog_sql(kind),
                200_000,
                &cancel,
                Exec::ReadOnly,
            )
            .await?;
            let name = match kind {
                DbKind::Postgres => "postgres",
                DbKind::Mssql => "mssql",
                _ => "mysql",
            };
            (name, group_catalog(&rows))
        }
    };
    Ok(kor_graph::DbSnapshot {
        connection: id.to_owned(),
        kind: kind.to_owned(),
        taken_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        tables,
        slow_queries: Vec::new(),
    })
}

fn load_conn(connections: Option<PathBuf>, id: &str) -> Result<(Connections, DbConn), String> {
    let file = connections.ok_or("no user configuration directory")?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let conns = Connections::parse(&text)?;
    let d = conns
        .db
        .get(id)
        .cloned()
        .ok_or_else(|| format!("no database connection `{id}` in {}", file.display()))?;
    Ok((conns, d))
}

fn sqlite_path(d: &DbConn, root: &Path) -> PathBuf {
    let p = PathBuf::from(d.file.as_deref().unwrap_or_default());
    if p.is_absolute() { p } else { root.join(p) }
}

/// String literals → `'?'`: statistics views can hold the values a statement ran with.
fn mask_literals(q: &str) -> String {
    static LIT: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"'(?:[^']|'')*'").expect("literal regex"));
    let one: String = q.split_whitespace().collect::<Vec<_>>().join(" ");
    let masked = LIT.replace_all(&one, "'?'");
    masked.chars().take(2000).collect()
}

/// Slowest statements by mean time from the database's own statistics (opt-in, `kor db connect
/// --slow`): Postgres `pg_stat_statements` (the extension must be installed), `MySQL`
/// `performance_schema` digests, SQL Server `sys.dm_exec_query_stats`. Read-only sessions,
/// literals masked. `SQLite` has no such log.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub async fn slow_queries(
    connections: Option<PathBuf>,
    id: &str,
    allow_network: bool,
) -> Result<Vec<kor_graph::SlowQuery>, String> {
    let (conns, d) = load_conn(connections, id)?;
    let sql = match d.kind {
        DbKind::Sqlite => return Err("SQLite keeps no slow query statistics".into()),
        DbKind::Postgres => {
            "SELECT query, calls, mean_exec_time * 1000, total_exec_time * 1000 FROM pg_stat_statements WHERE query NOT ILIKE '%pg_stat_statements%' ORDER BY mean_exec_time DESC LIMIT 20"
        }
        DbKind::Mysql => {
            "SELECT DIGEST_TEXT, COUNT_STAR, AVG_TIMER_WAIT / 1000000, SUM_TIMER_WAIT / 1000000 FROM performance_schema.events_statements_summary_by_digest WHERE DIGEST_TEXT IS NOT NULL AND SCHEMA_NAME = DATABASE() ORDER BY AVG_TIMER_WAIT DESC LIMIT 20"
        }
        DbKind::Mssql => {
            "SELECT TOP 20 REPLACE(REPLACE(SUBSTRING(t.text, 1, 2000), CHAR(10), ' '), CHAR(9), ' '), s.execution_count, s.total_elapsed_time / s.execution_count, s.total_elapsed_time FROM sys.dm_exec_query_stats s CROSS APPLY sys.dm_exec_sql_text(s.sql_handle) t ORDER BY 3 DESC"
        }
    };
    if !allow_network {
        return Err("privacy mode is on: Kor does not connect to database servers".into());
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    let (_, rows, _) = server_query(&conns, id, &d, sql, 20, &cancel, Exec::ReadOnly)
        .await
        .map_err(|e| {
            if e.contains("pg_stat_statements") {
                format!("{e} (install it: CREATE EXTENSION pg_stat_statements; and add it to shared_preload_libraries)")
            } else {
                e
            }
        })?;
    let num = |v: Option<&String>| {
        v.and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|f| f.is_finite() && *f >= 0.0)
            // Microseconds fit in u64 for any real duration.
            .map_or(0, |f| f.round() as u64)
    };
    Ok(rows
        .iter()
        .filter_map(|r| {
            let q = r.first().filter(|q| !q.trim().is_empty())?;
            Some(kor_graph::SlowQuery {
                query: mask_literals(q),
                calls: num(r.get(1)),
                mean_us: num(r.get(2)),
                total_us: num(r.get(3)),
            })
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Writes (`kor db apply`): destructive DDL is tried on a copy first
// ---------------------------------------------------------------------------------------------

/// Statements outside quotes, comments dropped.
fn statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = sql.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '-' if chars.peek() == Some(&'-') => {
                    for n in chars.by_ref() {
                        if n == '\n' {
                            break;
                        }
                    }
                    cur.push(' ');
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    let mut prev = ' ';
                    for n in chars.by_ref() {
                        if prev == '*' && n == '/' {
                            break;
                        }
                        prev = n;
                    }
                    cur.push(' ');
                }
                ';' => out.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            },
        }
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Statements that destroy schema or data (§8.5): `DROP`, `TRUNCATE`, `ALTER … DROP|RENAME`,
/// `DELETE`/`UPDATE` without `WHERE`. These always ask and run on a copy first.
pub fn destructive_statements(sql: &str) -> Vec<String> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^(drop\s|truncate\s|alter\s+table\s.*\s(drop|rename)\s|delete\s+from\s|update\s)").expect("ddl regex")
    });
    statements(sql)
        .into_iter()
        .filter(|s| {
            let l = s.to_ascii_lowercase();
            RE.is_match(s)
                && !((l.starts_with("delete") || l.starts_with("update")) && l.contains(" where "))
        })
        .collect()
}

/// psql / mysql read backslash commands from their input (`\!` runs a shell): refuse any
/// backslash outside a quoted literal.
pub(crate) fn check_backslashes(sql: &str) -> Result<(), String> {
    let mut quote: Option<char> = None;
    let mut chars = sql.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            // `'it\'s'` (MySQL): the escaped character stays inside the literal.
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (None, '\\') => {
                return Err("backslashes outside string literals are not allowed (psql/mysql meta-commands)".into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// What changed between two schemas: `- table x`, `+ table y`, `- column t.c`, `+ column t.c`.
pub fn schema_change(before: &[kor_graph::LiveTable], after: &[kor_graph::LiveTable]) -> Vec<String> {
    let cols = |t: &kor_graph::LiveTable| -> Vec<String> {
        t.columns
            .iter()
            .map(|c| c.split_whitespace().next().unwrap_or_default().to_ascii_lowercase())
            .collect()
    };
    let find = |list: &'_ [kor_graph::LiveTable], n: &str| {
        list.iter().find(|t| t.name.eq_ignore_ascii_case(n)).map(cols)
    };
    let mut out = Vec::new();
    for t in before {
        match find(after, &t.name) {
            None => out.push(format!("- table {}", t.name)),
            Some(now) => {
                for c in cols(t).iter().filter(|c| !now.contains(c)) {
                    out.push(format!("- column {}.{c}", t.name));
                }
                for c in now.iter().filter(|c| !cols(t).contains(c)) {
                    out.push(format!("+ column {}.{c}", t.name));
                }
            }
        }
    }
    for t in after.iter().filter(|t| find(before, &t.name).is_none()) {
        out.push(format!("+ table {}", t.name));
    }
    out
}

static TRIALS: std::sync::Mutex<Vec<(String, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());

fn trial_key(id: &str, sql: &str) -> String {
    use sha2::Digest as _;
    let h = sha2::Sha256::digest(format!("{id}\0{}", sql.trim()).as_bytes());
    h.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn trial_passed(key: &str) -> bool {
    let list = TRIALS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    list.iter()
        .any(|(k, at)| k == key && at.elapsed() < Duration::from_secs(30 * 60))
}

fn sqlite_exec(path: &Path, sql: &str) -> Result<(), String> {
    let mut c = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    c.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
    let tx = c.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(sql).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Run `sql` on a throwaway copy of connection `id` and report the schema change, so a
/// destructive change is seen before it touches the real database. `SQLite`: the file (and its
/// `-wal`) copied into a private temp folder. Postgres: `CREATE DATABASE … TEMPLATE <db>` (needs
/// `CREATEDB` and no other session on the database), dropped afterwards. `MySQL` and SQL Server
/// have no cheap copy: refused with the reason. A passed trial lets [`apply`] run the same SQL
/// for 30 minutes.
pub async fn trial(
    connections: Option<PathBuf>,
    id: &str,
    root: &Path,
    sql: &str,
    allow_network: bool,
) -> Result<Vec<String>, String> {
    let (conns, d) = load_conn(connections, id)?;
    let key = trial_key(id, sql);
    let change = match d.kind {
        DbKind::Sqlite => {
            let src = sqlite_path(&d, root);
            let sql = sql.to_owned();
            tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
                let before = sqlite_tables(&src)?;
                let dir = std::env::temp_dir().join(format!(
                    "kor-db-trial-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos())
                ));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let copy = dir.join("copy.db");
                let run = || -> Result<Vec<String>, String> {
                    std::fs::copy(&src, &copy).map_err(|e| e.to_string())?;
                    let wal = PathBuf::from(format!("{}-wal", src.display()));
                    if wal.is_file() {
                        std::fs::copy(&wal, dir.join("copy.db-wal")).map_err(|e| e.to_string())?;
                    }
                    sqlite_exec(&copy, &sql)?;
                    Ok(schema_change(&before, &sqlite_tables(&copy)?))
                };
                let out = run();
                let _ = std::fs::remove_dir_all(&dir);
                out
            })
            .await
            .map_err(|e| e.to_string())??
        }
        DbKind::Postgres => {
            if !allow_network {
                return Err("privacy mode is on: Kor does not connect to database servers".into());
            }
            check_backslashes(sql)?;
            let db = d.database.clone().ok_or("the connection has no `database` to copy")?;
            let cancel = tokio_util::sync::CancellationToken::new();
            let (_, rows, _) = server_query(&conns, id, &d, catalog_sql(d.kind), 200_000, &cancel, Exec::ReadOnly).await?;
            let before = group_catalog(&rows);
            let copy = format!(
                "kor_trial_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
            );
            let admin = DbConn { database: Some("postgres".into()), ..d.clone() };
            let create = format!("CREATE DATABASE {} TEMPLATE {}", quote_ident(&copy), quote_ident(&db));
            server_query(&conns, id, &admin, &create, 0, &cancel, Exec::Write)
                .await
                .map_err(|e| format!("could not copy `{db}` for a trial ({e}). Needs the CREATEDB privilege and no other session on `{db}`; Kor does not run destructive SQL without a trial"))?;
            let on_copy = DbConn { database: Some(copy.clone()), ..d.clone() };
            let result = async {
                server_query(&conns, id, &on_copy, sql, 0, &cancel, Exec::WriteTx).await?;
                let (_, rows, _) = server_query(&conns, id, &on_copy, catalog_sql(d.kind), 200_000, &cancel, Exec::ReadOnly).await?;
                Ok::<_, String>(schema_change(&before, &group_catalog(&rows)))
            }
            .await;
            let drop = format!("DROP DATABASE IF EXISTS {}", quote_ident(&copy));
            if let Err(e) = server_query(&conns, id, &admin, &drop, 0, &cancel, Exec::Write).await {
                return Err(format!("{} (the trial copy `{copy}` could not be dropped: {e})", result.err().unwrap_or_default()));
            }
            result?
        }
        DbKind::Mysql => return Err("MySQL has no cheap database copy; Kor refuses destructive SQL here. Try it on a staging database".into()),
        DbKind::Mssql => return Err("Kor only reads the schema of SQL Server databases".into()),
    };
    TRIALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((key, std::time::Instant::now()));
    Ok(change)
}

/// Run `sql` on the real database of connection `id` (`SQLite`: one transaction; Postgres:
/// `psql -1`; `MySQL`: autocommit). Destructive SQL ([`destructive_statements`]) only runs after
/// a passed [`trial`] of the same SQL in this process; the caller asks the user before each.
pub async fn apply(
    connections: Option<PathBuf>,
    id: &str,
    root: &Path,
    sql: &str,
    allow_network: bool,
) -> Result<(), String> {
    if sql.trim().is_empty() {
        return Err("the SQL is empty".into());
    }
    if !destructive_statements(sql).is_empty() && !trial_passed(&trial_key(id, sql)) {
        return Err("destructive SQL: run it on a copy first (kor db apply runs the trial)".into());
    }
    let (conns, d) = load_conn(connections, id)?;
    match d.kind {
        DbKind::Sqlite => {
            let path = sqlite_path(&d, root);
            let sql = sql.to_owned();
            tokio::task::spawn_blocking(move || sqlite_exec(&path, &sql))
                .await
                .map_err(|e| e.to_string())?
        }
        DbKind::Mssql => Err("Kor only reads the schema of SQL Server databases".into()),
        kind => {
            if !allow_network {
                return Err("privacy mode is on: Kor does not connect to database servers".into());
            }
            check_backslashes(sql)?;
            let exec = if kind == DbKind::Postgres { Exec::WriteTx } else { Exec::Write };
            let cancel = tokio_util::sync::CancellationToken::new();
            server_query(&conns, id, &d, sql, 0, &cancel, exec).await.map(|_| ())
        }
    }
}

/// Connection ids defined in the user's `connections.toml` (`kor db connect` help).
pub fn connection_ids(connections: Option<&Path>) -> Result<Vec<String>, String> {
    let Some(file) = connections else {
        return Ok(Vec::new());
    };
    match std::fs::read_to_string(file) {
        Ok(t) => Ok(Connections::parse(&t)?.db.into_keys().collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn introspects_sqlite_for_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        {
            let c = rusqlite::Connection::open(dir.path().join("app.db")).unwrap();
            c.execute_batch("CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL); CREATE TABLE orders(id INTEGER, user_id INTEGER REFERENCES users(id)); INSERT INTO users(email) VALUES ('secret@x');").unwrap();
        }
        let conf = dir.path().join("connections.toml");
        std::fs::write(&conf, "[db.app]\nkind = \"sqlite\"\nfile = \"app.db\"\n\n[db.pg]\nkind = \"postgres\"\nhost = \"db.example\"\n").unwrap();
        let snap = introspect(Some(conf.clone()), "app", dir.path(), false)
            .await
            .unwrap();
        assert_eq!(snap.kind, "sqlite");
        assert_eq!(snap.tables.len(), 2);
        assert_eq!(snap.tables[0].name, "orders");
        assert_eq!(snap.tables[0].references, vec!["users"]);
        assert_eq!(snap.tables[1].columns, vec!["id INTEGER", "email TEXT"]);
        // Schema only: no row data.
        assert!(!serde_json::to_string(&snap).unwrap().contains("secret@x"));
        // Servers are refused in privacy mode, unknown ids are named.
        assert!(
            introspect(Some(conf.clone()), "pg", dir.path(), false)
                .await
                .unwrap_err()
                .contains("privacy")
        );
        assert!(introspect(Some(conf.clone()), "nope", dir.path(), true).await.is_err());
        assert_eq!(connection_ids(Some(&conf)).unwrap(), vec!["app", "pg"]);
        // The snapshot becomes a graph layer.
        kor_graph::db::save_snapshot(dir.path(), &snap).unwrap();
        let mut g = kor_graph::Graph::open(dir.path()).unwrap();
        g.update().unwrap();
        let t = g.table("orders").unwrap();
        assert_eq!(t.references[0].id, "table:users");
        assert_eq!(t.live, vec!["app"]);
    }

    #[test]
    fn catalog_rows_group_into_tables() {
        let rows: Vec<Vec<String>> = [
            ["c", "public.a", "id", "integer", "1"],
            ["c", "public.a", "b_id", "integer", "2"],
            ["f", "public.a", "b_id", "public.b", "0"],
            ["c", "public.b", "id", "integer", "1"],
        ]
        .iter()
        .map(|r| r.iter().map(|s| (*s).to_owned()).collect())
        .collect();
        let t = group_catalog(&rows);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].columns, vec!["id integer", "b_id integer"]);
        assert_eq!(t[0].references, vec!["public.b"]);
        assert_eq!(t[0].fk_columns, vec!["b_id"]);
        let rows = vec![
            ["i", "a", "a_pkey", "id unique", "0"].map(str::to_owned).to_vec(),
            ["i", "a", "a_x", "x,y", "0"].map(str::to_owned).to_vec(),
        ];
        assert_eq!(
            group_catalog(&rows)[0].indexes,
            vec!["a_pkey (id) unique", "a_x (x,y)"]
        );
        for k in [DbKind::Postgres, DbKind::Mysql, DbKind::Mssql] {
            assert!(!catalog_sql(k).contains('\\'));
        }
    }

    #[test]
    fn destructive_sql_is_found_outside_quotes_and_comments() {
        let sql = "-- DROP TABLE x;\nCREATE TABLE a(id int); INSERT INTO a VALUES ('drop table b; x');\n/* truncate c; */ ALTER TABLE a DROP COLUMN id;\nDELETE FROM a WHERE id = 1; delete from a; UPDATE a SET id = 2";
        assert_eq!(
            destructive_statements(sql),
            vec!["ALTER TABLE a DROP COLUMN id", "delete from a", "UPDATE a SET id = 2"]
        );
        assert!(destructive_statements("CREATE INDEX i ON a(id)").is_empty());
        assert!(check_backslashes("SELECT 'a\\nb', E'\\x'").is_ok());
        assert!(check_backslashes("SELECT 'it\\'s' \\! ls").is_err());
        assert!(check_backslashes("select 1 \\! rm -rf /").is_err());
        assert_eq!(
            mask_literals("SELECT *\n FROM u WHERE email = 'a@b.c' AND n = 'it''s'"),
            "SELECT * FROM u WHERE email = '?' AND n = '?'"
        );
    }

    #[tokio::test]
    async fn sqlite_destructive_sql_runs_on_a_copy_first() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT); CREATE TABLE old(id INTEGER); CREATE INDEX users_email ON users(email); CREATE TABLE orders(id INTEGER, user_id INTEGER REFERENCES users(id));").unwrap();
        }
        let conf = dir.path().join("connections.toml");
        std::fs::write(&conf, "[db.app]\nkind = \"sqlite\"\nfile = \"app.db\"\n[db.my]\nkind = \"mysql\"\nhost = \"h\"\n").unwrap();
        let snap = introspect(Some(conf.clone()), "app", dir.path(), false).await.unwrap();
        let users = snap.tables.iter().find(|t| t.name == "users").unwrap();
        assert_eq!(users.indexes, vec!["users_email (email)"]);
        let orders = snap.tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(orders.fk_columns, vec!["user_id"]);

        let sql = "DROP TABLE old; ALTER TABLE users ADD COLUMN name TEXT;";
        // No trial yet: refused.
        assert!(apply(Some(conf.clone()), "app", dir.path(), sql, false).await.unwrap_err().contains("copy"));
        let change = trial(Some(conf.clone()), "app", dir.path(), sql, false).await.unwrap();
        assert_eq!(change, vec!["- table old", "+ column users.name"]);
        // The real file is untouched by the trial.
        let names = |p: &Path| sqlite_tables(p).unwrap().into_iter().map(|t| t.name).collect::<Vec<_>>();
        assert!(names(&db).contains(&"old".to_owned()));
        apply(Some(conf.clone()), "app", dir.path(), sql, false).await.unwrap();
        assert!(!names(&db).contains(&"old".to_owned()));
        // A failing trial reports the error and leaves nothing behind.
        assert!(trial(Some(conf.clone()), "app", dir.path(), "DROP TABLE nope", false).await.is_err());
        // Non-destructive SQL needs no trial; MySQL destructive is refused with the reason.
        apply(Some(conf.clone()), "app", dir.path(), "CREATE TABLE t(id int)", false).await.unwrap();
        assert!(trial(Some(conf.clone()), "my", dir.path(), "DROP TABLE x", true).await.unwrap_err().contains("MySQL"));
        assert!(slow_queries(Some(conf), "app", true).await.is_err());
    }

    /// `cargo test -p kor-agent -- --ignored real_postgres_graph` with a running server at
    /// `KOR_TEST_PG` (`host:port:user:database`).
    #[tokio::test]
    #[ignore = "needs a PostgreSQL server"]
    async fn real_postgres_graph_snapshot() {
        let spec = std::env::var("KOR_TEST_PG").expect("KOR_TEST_PG");
        let p: Vec<&str> = spec.split(':').collect();
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("connections.toml");
        std::fs::write(&conf, format!("[db.pg]\nkind = \"postgres\"\nhost = \"{}\"\nport = {}\nuser = \"{}\"\ndatabase = \"{}\"\nsslmode = \"disable\"\n", p[0], p[1], p[2], p[3])).unwrap();
        let snap = introspect(Some(conf), "pg", dir.path(), true).await.unwrap();
        assert_eq!(snap.kind, "postgres");
        assert!(!snap.tables.is_empty());
    }

    #[test]
    fn only_single_reading_statements_pass() {
        assert_eq!(check_read_only("  SELECT 1;  ", false).unwrap(), "SELECT 1");
        assert!(
            check_read_only(
                "-- note\n/* c */ with x as (select 1) select * from x",
                false
            )
            .is_ok()
        );
        assert!(check_read_only("PRAGMA table_info(t)", true).is_ok());
        for bad in [
            "DELETE FROM t",
            "select 1; delete from t",
            "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
            "PRAGMA writable_schema=1",
            "",
            "-- only a comment",
        ] {
            assert!(check_read_only(bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn sqlite_reads_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        {
            let c = rusqlite::Connection::open(&p).unwrap();
            c.execute_batch("CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT); INSERT INTO users(name, note) VALUES ('Ada', 'line1\nline2'), ('Linus', NULL);").unwrap();
        }
        let out = sqlite_query(&p, "SELECT id, name, note FROM users ORDER BY id").unwrap();
        assert!(
            out.contains("id | name | note\n1 | Ada | line1 line2\n2 | Linus | NULL\n(2 rows)"),
            "{out}"
        );
        let schema = sqlite_schema(&p).unwrap();
        assert!(
            schema.contains("users (table): id INTEGER pk, name TEXT not null, note TEXT"),
            "{schema}"
        );
        // Even past the keyword check, the connection itself cannot write.
        assert!(sqlite_query(&p, "INSERT INTO users(name) VALUES ('x')").is_err());
        let c = sqlite_open(&p).unwrap();
        assert!(c.execute("DELETE FROM users", []).is_err());
    }

    #[test]
    fn csv_and_tsv_parse() {
        let (h, rows, more) = parse_csv("a,b\n1,\"x, \"\"y\"\"\nz\"\n2,\n", false, MAX_ROWS);
        assert_eq!(h, vec!["a", "b"]);
        assert_eq!(
            rows,
            vec![
                vec!["1".to_owned(), "x, \"y\" z".to_owned()],
                vec!["2".to_owned(), String::new()]
            ]
        );
        assert!(!more);
        let (h, rows, _) = parse_tsv("t\tc\nusers\tid\\tx\n", false, MAX_ROWS);
        assert_eq!(h, vec!["t", "c"]);
        assert_eq!(rows[0], vec!["users".to_owned(), "id x".to_owned()]);
    }

    #[test]
    fn schema_rows_group_by_table() {
        let rows = vec![
            vec![
                "public.a".into(),
                "id".into(),
                "integer".into(),
                "NO".into(),
            ],
            vec![
                "public.a".into(),
                "name".into(),
                "text".into(),
                "YES".into(),
            ],
            vec!["public.b".into(), "x".into(), "date".into(), "YES".into()],
        ];
        let s = group_schema(&rows);
        assert!(
            s.contains("public.a: id integer not null, name text\npublic.b: x date\n"),
            "{s}"
        );
    }

    /// A throwaway cluster from the installed `PostgreSQL`: schema, rows, and the server refusing
    /// a write hidden in a CTE. `cargo test -p kor-agent -- --ignored real_postgres`
    /// Stops the server even when an assertion fails.
    struct Stop<F: Fn()>(F);
    impl<F: Fn()> Drop for Stop<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a PostgreSQL installation"]
    #[allow(clippy::too_many_lines)]
    async fn real_postgres_is_read_only() {
        let bin = client("psql").expect("psql").parent().unwrap().to_owned();
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let run = |exe: &str, args: &[&str]| {
            let o = std::process::Command::new(bin.join(exe))
                .args(args)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{exe}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        let d = data.to_string_lossy().into_owned();
        run(
            "initdb",
            &[
                "-D",
                &d,
                "-U",
                "kor",
                "-A",
                "trust",
                "-E",
                "UTF8",
                "--locale=C",
                "--no-sync",
            ],
        );
        let opts = format!("-p {port} -c listen_addresses=127.0.0.1");
        let log = dir.path().join("log.txt").to_string_lossy().into_owned();
        // The server inherits pg_ctl's handles: no pipes, or waiting for them never ends.
        let pg_ctl = |args: &[&str]| {
            std::process::Command::new(bin.join("pg_ctl"))
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
        };
        assert!(pg_ctl(&["-D", &d, "-o", &opts, "-l", &log, "-w", "start"]).success());
        let stop = Stop(|| {
            pg_ctl(&["-D", &d, "-m", "immediate", "-w", "stop"]);
        });
        let p = port.to_string();
        run(
            "psql",
            &[
                "-X",
                "-h",
                "127.0.0.1",
                "-p",
                &p,
                "-U",
                "kor",
                "-d",
                "postgres",
                "-c",
                "CREATE TABLE items(id int PRIMARY KEY, name text NOT NULL); INSERT INTO items VALUES (1,U&'\\00E7ay'),(2,'a,\"b\"');",
            ],
        );

        let file = dir.path().join("connections.toml");
        std::fs::write(&file, format!("[db.pg]\nkind = \"postgres\"\nhost = \"127.0.0.1\"\nport = {port}\ndatabase = \"postgres\"\nuser = \"kor\"\nsslmode = \"disable\"\n")).unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let tools = super::super::tools(Some(file));
        let t = |n: &str| tools.iter().find(|t| t.name() == n).unwrap().clone();

        let schema = t("db_schema")
            .call(serde_json::json!({"connection": "pg"}), &ctx)
            .await;
        let rows = t("db_query").call(serde_json::json!({"connection": "pg", "sql": "SELECT id, name FROM items ORDER BY id"}), &ctx).await;
        let cte = t("db_query").call(serde_json::json!({"connection": "pg", "sql": "WITH x AS (INSERT INTO items VALUES (9,'z') RETURNING id) SELECT * FROM x"}), &ctx).await;
        let two = t("db_query")
            .call(
                serde_json::json!({"connection": "pg", "sql": "SELECT 1; DELETE FROM items"}),
                &ctx,
            )
            .await;
        let count = t("db_query")
            .call(
                serde_json::json!({"connection": "pg", "sql": "SELECT count(*) FROM items WHERE name <> 'çay'"}),
                &ctx,
            )
            .await;
        drop(stop);

        assert!(
            schema
                .content
                .contains("public.items: id integer not null, name text not null"),
            "{}",
            schema.content
        );
        assert!(
            rows.content
                .contains("id | name\n1 | çay\n2 | a,\"b\"\n(2 rows)"),
            "{}",
            rows.content
        );
        assert!(
            cte.is_error && cte.content.contains("read-only"),
            "{}",
            cte.content
        );
        assert!(
            two.is_error && two.content.contains("one statement"),
            "{}",
            two.content
        );
        // Turkish text travels in both directions; the CTE insert did not happen.
        assert!(count.content.contains("\n1\n"), "{}", count.content);
    }
}
