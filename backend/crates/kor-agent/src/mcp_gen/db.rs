//! `postgres://…`, `mysql://…`, `sqlite:…` → `list_tables`, `describe_table`, `query` (one
//! read-only statement, row limit, timeout) and, only with `--allow-writes`, `execute` (always
//! asks). Reuses `remote::db`: read-only sessions on the server itself (`psql` with
//! `default_transaction_read_only`, `mysql` with a read-only session, `SQLite` opened read-only),
//! the password from the keychain (`db-<id>`) or `KOR_DB_<ID>_PASSWORD`, never from the config.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use kor_secrets::SecretString;
use serde_json::{Value, json};

use super::{AddRequest, DbTarget, Definition, GenTool, Kind, Plan, serve::Backend};
use crate::remote::db as rdb;
use crate::remote::{Connections, DbConn, DbKind};

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Names and hosts only: nothing `psql`/`mysql` could read as an option.
fn plain(s: &str, extra: &[u8], max: usize) -> bool {
    (1..=max).contains(&s.len())
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

/// A database URL → target (without the password) and the password it carried.
pub fn parse_url(source: &str, root: &Path) -> Result<(DbTarget, Option<SecretString>), String> {
    if let Some(rest) = source.strip_prefix("sqlite:") {
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        // `sqlite:///C:/x.db` → `C:/x.db`; `sqlite:///home/x.db` stays absolute.
        let rest = match rest.as_bytes() {
            [b'/', d, b':', ..] if d.is_ascii_alphabetic() => &rest[1..],
            _ => rest,
        };
        let p = PathBuf::from(rest);
        let p = if p.is_absolute() { p } else { root.join(p) };
        if !p.is_file() {
            return Err(format!("{} does not exist", p.display()));
        }
        return Ok((
            DbTarget {
                engine: "sqlite".into(),
                host: None,
                port: None,
                database: None,
                user: None,
                file: Some(p.to_string_lossy().into_owned()),
                sslmode: None,
            },
            None,
        ));
    }
    let url = reqwest::Url::parse(source).map_err(|e| format!("database URL: {e}"))?;
    let engine = match url.scheme() {
        "postgres" | "postgresql" => "postgres",
        "mysql" => "mysql",
        other => {
            return Err(format!(
                "unsupported database `{other}` (postgres, mysql, sqlite)"
            ));
        }
    };
    let host = url.host_str().ok_or("database URL needs a host")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if !plain(host, b".:_-", 253) {
        return Err("database URL: invalid host".into());
    }
    let user = Some(decode(url.username())).filter(|u| !u.is_empty());
    if user.as_deref().is_some_and(|u| !plain(u, b"._-@", 64)) {
        return Err("database URL: invalid user".into());
    }
    let database = Some(decode(url.path().trim_start_matches('/'))).filter(|d| !d.is_empty());
    if database.as_deref().is_some_and(|d| !plain(d, b"._-$", 64)) {
        return Err("database URL: invalid database name".into());
    }
    let sslmode = url
        .query_pairs()
        .find(|(k, _)| k == "sslmode")
        .map(|(_, v)| v.into_owned());
    if sslmode.as_deref().is_some_and(|m| {
        ![
            "disable",
            "allow",
            "prefer",
            "require",
            "verify-ca",
            "verify-full",
        ]
        .contains(&m)
    }) {
        return Err("database URL: invalid sslmode".into());
    }
    let password = url.password().map(|p| SecretString::from(decode(p)));
    Ok((
        DbTarget {
            engine: engine.into(),
            host: Some(host.to_owned()),
            port: url.port(),
            database,
            user,
            file: None,
            sslmode,
        },
        password,
    ))
}

/// The database name, or the `SQLite` file name.
pub fn default_id(t: &DbTarget) -> String {
    let name = t.database.clone().or_else(|| {
        t.file
            .as_deref()
            .and_then(|f| Path::new(f).file_stem())
            .map(|s| s.to_string_lossy().into_owned())
    });
    super::slug(&name.unwrap_or_else(|| t.engine.clone()))
}

fn conn(t: &DbTarget) -> DbConn {
    DbConn {
        kind: match t.engine.as_str() {
            "postgres" => DbKind::Postgres,
            "mysql" => DbKind::Mysql,
            _ => DbKind::Sqlite,
        },
        host: t.host.clone(),
        port: t.port,
        database: t.database.clone(),
        user: t.user.clone(),
        file: t.file.clone(),
        sslmode: t.sslmode.clone(),
        ssh: None,
    }
}

fn tool(name: &str, description: String, schema: Value, write: bool) -> GenTool {
    GenTool {
        name: name.into(),
        description,
        input_schema: schema,
        write,
        plan: Plan::Sql { op: name.into() },
    }
}

/// The server's tools. For `SQLite` the table names are read now and listed in the tool
/// descriptions; servers are not contacted before the password is in the keychain (`kor mcp
/// test` checks the connection).
pub async fn definition(
    id: &str,
    target: DbTarget,
    req: &AddRequest,
) -> Result<Definition, String> {
    let mut def = Definition {
        id: id.to_owned(),
        kind: Kind::Database,
        source: match &target.file {
            Some(f) => format!("sqlite:{f}"),
            None => format!(
                "{}://{}{}{}/{}",
                target.engine,
                target
                    .user
                    .as_deref()
                    .map(|u| format!("{u}@"))
                    .unwrap_or_default(),
                target.host.as_deref().unwrap_or(""),
                target.port.map(|p| format!(":{p}")).unwrap_or_default(),
                target.database.as_deref().unwrap_or("")
            ),
        },
        endpoint: None,
        allow_writes: req.allow_writes,
        auth: None,
        db: Some(target),
        row_limit: req.row_limit.unwrap_or_else(super::default_rows),
        timeout_secs: req.timeout_secs.unwrap_or_else(super::default_timeout),
        tools: Vec::new(),
        skipped: 0,
    };
    let names = if def.db.as_ref().is_some_and(|t| t.file.is_some()) {
        Server::new(def.clone()).table_names().await?
    } else {
        Vec::new()
    };
    let engine = def
        .db
        .as_ref()
        .map_or("sql", |t| t.engine.as_str())
        .to_owned();
    let listed = if names.is_empty() {
        String::new()
    } else {
        let shown: Vec<&str> = names.iter().take(100).map(String::as_str).collect();
        format!(" Tables: {}.", shown.join(", "))
    };
    let mut table = json!({"type": "string", "maxLength": 200, "description": "Table name (schema.table on Postgres)"});
    if !names.is_empty() && names.len() <= 500 {
        table["enum"] = json!(names);
    }
    def.tools = vec![
        tool(
            "list_tables",
            format!("Tables and views of the {engine} database `{id}`.{listed}"),
            json!({"type": "object", "properties": {}}),
            false,
        ),
        tool(
            "describe_table",
            format!("Columns of one table of `{id}` (name, type, not null, primary key)."),
            json!({"type": "object", "properties": {"table": table}, "required": ["table"]}),
            false,
        ),
        tool(
            "query",
            format!(
                "Run one read-only SQL statement (SELECT, WITH, EXPLAIN, SHOW…) on `{id}`; the database refuses writes. Returns at most {} rows.",
                def.rows()
            ),
            json!({"type": "object", "properties": {
                "sql": {"type": "string", "maxLength": 100_000},
                "limit": {"type": "integer", "minimum": 1, "maximum": def.rows(), "description": "Rows to return"}
            }, "required": ["sql"]}),
            false,
        ),
    ];
    if req.allow_writes {
        def.tools.push(tool(
            "execute",
            format!("Run one statement that changes `{id}` (INSERT, UPDATE, DELETE, DDL). Always asks the user first."),
            json!({"type": "object", "properties": {"sql": {"type": "string", "maxLength": 100_000}}, "required": ["sql"]}),
            true,
        ));
    } else {
        def.skipped = 1;
    }
    Ok(def)
}

/// One statement, nothing after a `;`.
fn single(sql: &str) -> Result<String, String> {
    let s = sql.trim().trim_end_matches(';').trim();
    if s.is_empty() {
        return Err("the SQL is empty".into());
    }
    if s.contains(';') {
        return Err("only one statement is allowed (no `;` inside the SQL)".into());
    }
    Ok(s.to_owned())
}

/// Interrupts `SQLite` work on `c` when `timeout` passes before the returned sender is dropped.
fn watchdog(c: &rusqlite::Connection, timeout: Duration) -> std::sync::mpsc::Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let h = c.get_interrupt_handle();
    std::thread::spawn(move || {
        if rx.recv_timeout(timeout) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            h.interrupt();
        }
    });
    tx
}

fn timed_out(e: &rusqlite::Error) -> String {
    let s = e.to_string();
    if s.contains("interrupt") {
        "the query timed out".into()
    } else {
        s
    }
}

fn sqlite_read(
    path: &Path,
    sql: &str,
    max_rows: usize,
    timeout: Duration,
) -> Result<String, String> {
    let c = rdb::sqlite_open(path)?;
    let _guard = watchdog(&c, timeout);
    let mut stmt = c.prepare(sql).map_err(|e| timed_out(&e))?;
    if !stmt.readonly() {
        return Err("SQLite reports this statement would write; `query` only reads".into());
    }
    let columns: Vec<String> = stmt.column_names().map(str::to_owned).collect();
    let mut rows = Vec::new();
    let mut more = false;
    let mut it = stmt.query([]).map_err(|e| timed_out(&e))?;
    while let Some(row) = it.next().map_err(|e| timed_out(&e))? {
        if rows.len() >= max_rows {
            more = true;
            break;
        }
        rows.push(
            (0..columns.len())
                .map(|i| row.get_ref(i).map(rdb::sqlite_value).unwrap_or_default())
                .collect(),
        );
    }
    Ok(rdb::table(&columns, &rows, more))
}

fn sqlite_write(path: &Path, sql: &str, timeout: Duration) -> Result<String, String> {
    let c =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| e.to_string())?;
    c.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let _guard = watchdog(&c, timeout);
    let n = c.execute(sql, []).map_err(|e| timed_out(&e))?;
    Ok(format!("{n} row(s) changed"))
}

/// A generated database server.
#[derive(Debug)]
pub struct Server {
    def: Definition,
}

impl Server {
    pub fn new(def: Definition) -> Self {
        Self { def }
    }

    fn target(&self) -> Result<&DbTarget, String> {
        self.def
            .db
            .as_ref()
            .ok_or_else(|| "no database in the definition".into())
    }

    /// `(columns, rows, more)` from a server, under the definition's timeout.
    async fn server(
        &self,
        sql: &str,
        rows: usize,
        exec: rdb::Exec,
    ) -> Result<(Vec<String>, Vec<Vec<String>>, bool), String> {
        rdb::check_backslashes(sql)?;
        let d = conn(self.target()?);
        // psql/mysql reach the network themselves: the guard's host rule applies (ADR 0035).
        kor_core::net::check_host(d.host.as_deref().unwrap_or("127.0.0.1"))
            .map_err(|b| b.message)?;
        let cancel = tokio_util::sync::CancellationToken::new();
        tokio::time::timeout(
            self.def.timeout(),
            rdb::server_query(
                &Connections::default(),
                &self.def.id,
                &d,
                sql,
                rows,
                &cancel,
                exec,
            ),
        )
        .await
        .map_err(|_| "the query timed out".to_owned())?
    }

    /// `name: columns` per table.
    async fn schema(&self) -> Result<String, String> {
        let t = self.target()?;
        if let Some(f) = &t.file {
            let p = PathBuf::from(f);
            return tokio::task::spawn_blocking(move || rdb::sqlite_schema(&p))
                .await
                .map_err(|e| e.to_string())?;
        }
        let kind = conn(t).kind;
        let (_, rows, _) = self
            .server(rdb::schema_sql(kind), 200_000, rdb::Exec::ReadOnly)
            .await?;
        Ok(rdb::group_schema(&rows))
    }

    /// `(table, line)` of every schema line (`users (table): id INTEGER pk, …` or `public.a: …`).
    async fn schema_lines(&self) -> Result<Vec<(String, String)>, String> {
        Ok(self
            .schema()
            .await?
            .lines()
            .filter(|l| !l.starts_with('['))
            .filter_map(|l| {
                let (name, _) = l.split_once(':')?;
                let name = name.split(" (").next().unwrap_or(name).trim();
                Some((name.to_owned(), l.to_owned()))
            })
            .collect())
    }

    async fn table_names(&self) -> Result<Vec<String>, String> {
        Ok(self
            .schema_lines()
            .await?
            .into_iter()
            .map(|(n, _)| n)
            .collect())
    }
}

#[async_trait]
impl Backend for Server {
    fn name(&self) -> String {
        format!("kor-{}", self.def.id)
    }

    fn tools(&self) -> Vec<Value> {
        self.def.tools.iter().map(GenTool::descriptor).collect()
    }

    async fn call(&self, name: &str, args: Value) -> Result<String, String> {
        let tool = self
            .def
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or("unknown tool")?;
        super::validate(&tool.input_schema, &args)?;
        let t = self.target()?.clone();
        let timeout = self.def.timeout();
        match name {
            "list_tables" => {
                let names = self.table_names().await?;
                Ok(if names.is_empty() {
                    "No tables.".into()
                } else {
                    format!("{}\n{}", crate::remote::UNTRUSTED, names.join("\n"))
                })
            }
            "describe_table" => {
                let want = args["table"].as_str().unwrap_or_default();
                let lines = self.schema_lines().await?;
                lines
                    .iter()
                    .find(|(n, _)| n == want || n.rsplit('.').next() == Some(want))
                    .map(|(_, l)| format!("{}\n{l}", crate::remote::UNTRUSTED))
                    .ok_or_else(|| format!("no table `{want}`; call list_tables"))
            }
            "query" => {
                let limit = args["limit"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or_else(|| self.def.rows())
                    .clamp(1, self.def.rows());
                let sql = rdb::check_read_only(
                    args["sql"].as_str().unwrap_or_default(),
                    t.file.is_some(),
                )?;
                if let Some(f) = t.file {
                    return tokio::task::spawn_blocking(move || {
                        sqlite_read(Path::new(&f), &sql, limit, timeout)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                }
                let (c, r, more) = self.server(&sql, limit, rdb::Exec::ReadOnly).await?;
                Ok(rdb::table(&c, &r, more))
            }
            "execute" if self.def.allow_writes => {
                let sql = single(args["sql"].as_str().unwrap_or_default())?;
                if let Some(f) = t.file {
                    return tokio::task::spawn_blocking(move || {
                        sqlite_write(Path::new(&f), &sql, timeout)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                }
                let (c, r, more) = self.server(&sql, self.def.rows(), rdb::Exec::Write).await?;
                Ok(if c.is_empty() {
                    "done".into()
                } else {
                    rdb::table(&c, &r, more)
                })
            }
            _ => Err("unknown tool".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_tools::{McpClient, McpTool};
    use kor_tools::{Tool as _, ToolCtx};
    use std::sync::Arc;

    #[test]
    fn urls_parse_without_keeping_the_password() {
        use kor_secrets::ExposeSecret as _;
        let (t, pw) = parse_url(
            "postgres://ada:p%40ss@db.local:5433/shop?sslmode=require",
            Path::new("."),
        )
        .unwrap();
        assert_eq!(pw.unwrap().expose_secret(), "p@ss");
        assert_eq!(
            (
                t.host.as_deref(),
                t.port,
                t.database.as_deref(),
                t.user.as_deref()
            ),
            (Some("db.local"), Some(5433), Some("shop"), Some("ada"))
        );
        assert_eq!(t.sslmode.as_deref(), Some("require"));
        assert_eq!(default_id(&t), "shop");
        for bad in [
            "postgres://-oProxy@h/db",
            "postgres://h/d;b",
            "redis://h",
            "postgres://h/db?sslmode=x",
            "sqlite:nope.db",
        ] {
            assert!(parse_url(bad, Path::new(".")).is_err(), "{bad}");
        }
    }

    /// Generated `SQLite` server end to end: generate → install → serve over stdio (in process)
    /// → Kor's own MCP client lists and calls the tools; writes only when enabled.
    #[tokio::test]
    #[allow(clippy::many_single_char_names, clippy::too_many_lines)]
    async fn sqlite_server_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("shop.db");
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL); INSERT INTO items(name) VALUES ('çay'), ('kahve'), ('su');").unwrap();
        }
        let loc = crate::mcp_config::Locations {
            kor_file: dir.path().join("cfg").join("mcp.json"),
            home: None,
            root: dir.path().to_owned(),
            trusted: false,
        };
        let added = super::super::generate(
            &AddRequest {
                source: "sqlite:shop.db".into(),
                row_limit: Some(2),
                ..AddRequest::default()
            },
            dir.path(),
            Path::new("/usr/bin/kor"),
        )
        .await
        .unwrap();
        assert_eq!((added.id.as_str(), added.tools), ("shop", 3));
        super::super::install(&loc, &added, &kor_secrets::MemoryStore::default()).unwrap();
        let entry = &crate::mcp_config::entries(&loc).unwrap()[0];
        assert_eq!(entry.raw["args"], json!(["mcp", "run", "shop"]));
        assert_eq!(super::super::tool_count(&loc.kor_file, &entry.raw), Some(3));

        // `kor mcp run shop`, in process.
        let def = super::super::load(&loc.kor_file, "shop").unwrap();
        let backend = super::super::backend(def, Arc::new(|_: &str| None));
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            let (r, w) = tokio::io::split(b);
            super::super::serve::serve(backend.as_ref(), r, w).await
        });
        let (r, w) = tokio::io::split(a);
        let client = McpClient::from_streams("shop", r, w, None);
        client.handshake().await.unwrap();
        let tools = client.list_tools().await.unwrap();
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["list_tables", "describe_table", "query"]
        );
        assert_eq!(tools[0].annotations["readOnlyHint"], true);
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let call = |i: usize, args: Value| {
            let t = McpTool::new(client.clone(), tools[i].clone());
            let ctx = &ctx;
            async move { t.call(args, ctx).await }
        };
        assert!(call(0, json!({})).await.content.ends_with("items"));
        assert!(
            call(1, json!({"table": "items"}))
                .await
                .content
                .contains("id INTEGER pk, name TEXT not null")
        );
        let rows = call(2, json!({"sql": "SELECT name FROM items ORDER BY id"})).await;
        assert!(
            rows.content.contains("name\nçay\nkahve\n[first"),
            "{}",
            rows.content
        );
        let write = call(2, json!({"sql": "DELETE FROM items"})).await;
        assert!(write.is_error, "{}", write.content);
        assert!(call(1, json!({"table": "nope"})).await.is_error);
        assert!(
            call(2, json!({}))
                .await
                .content
                .contains("`sql` is required")
        );

        // With --allow-writes: `execute` is destructive, and Kor always asks before it.
        let added = super::super::generate(
            &AddRequest {
                source: format!("sqlite:{}", db.display()),
                id: Some("shop-rw".into()),
                allow_writes: true,
                ..AddRequest::default()
            },
            dir.path(),
            Path::new("kor"),
        )
        .await
        .unwrap();
        let server = super::super::backend(added.definition.unwrap(), Arc::new(|_: &str| None));
        let exec = server
            .tools()
            .into_iter()
            .find(|t| t["name"] == "execute")
            .unwrap();
        assert_eq!(exec["annotations"]["destructiveHint"], true);
        let remote = crate::mcp_tools::RemoteTool {
            name: "execute".into(),
            description: String::new(),
            input_schema: exec["inputSchema"].clone(),
            annotations: exec["annotations"].clone(),
        };
        let tool = McpTool::new(client.clone(), remote);
        // Even in auto mode with a rule allowing exactly this tool, it asks.
        let p = crate::Permissions::new(
            kor_core::PermissionMode::Auto,
            &["mcp__shop__execute".into()],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            p.decide(&tool, &json!({"sql": "DELETE FROM items"})),
            crate::Decision::Ask
        );
        assert_eq!(
            crate::permissions::destructive(&tool, &json!({})),
            Some("mcp-write")
        );
        assert_eq!(
            server
                .call("execute", json!({"sql": "DELETE FROM items WHERE id = 3"}))
                .await
                .unwrap(),
            "1 row(s) changed"
        );
        assert!(
            server
                .call(
                    "execute",
                    json!({"sql": "DELETE FROM items; DROP TABLE items"})
                )
                .await
                .is_err()
        );
    }
}
