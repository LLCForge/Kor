//! Storage behind one small interface: `SQLite` (embedded, the default and the tests) or Postgres
//! (`db = "postgres://…"`). Every statement is written once with `$1, $2, …` placeholders, which
//! both engines bind positionally (in `SQLite` `$n` is a named parameter numbered by first use, so
//! each statement uses its placeholders in ascending order). Columns are `BIGINT` or `TEXT` only.
//! Values are always bound, never formatted into SQL.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// One bound value or one column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    /// An integer.
    Int(i64),
    /// Text.
    Text(String),
    /// SQL NULL.
    Null,
}

impl Val {
    /// The text, if this is text.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The integer, if this is one.
    pub fn int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The text, or `""`.
    pub fn s(&self) -> String {
        self.text().unwrap_or_default().to_owned()
    }

    /// The integer, or 0.
    pub fn i(&self) -> i64 {
        self.int().unwrap_or(0)
    }
}

impl From<&str> for Val {
    fn from(s: &str) -> Self {
        Self::Text(s.to_owned())
    }
}
impl From<String> for Val {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}
impl From<&String> for Val {
    fn from(s: &String) -> Self {
        Self::Text(s.clone())
    }
}
impl From<i64> for Val {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}
impl From<Option<String>> for Val {
    fn from(s: Option<String>) -> Self {
        s.map_or(Self::Null, Self::Text)
    }
}

impl rusqlite::ToSql for Val {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, Value, ValueRef};
        Ok(match self {
            Self::Int(i) => ToSqlOutput::Owned(Value::Integer(*i)),
            Self::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
            Self::Null => ToSqlOutput::Owned(Value::Null),
        })
    }
}

/// One result row.
pub type Row = Vec<Val>;

/// The schema; idempotent, run at every start.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS members (email TEXT PRIMARY KEY, role TEXT NOT NULL, created_ms BIGINT NOT NULL, added_by TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS certs (fingerprint TEXT PRIMARY KEY, email TEXT NOT NULL, expires_ms BIGINT NOT NULL, revoked BIGINT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS tokens (hash TEXT PRIMARY KEY, email TEXT NOT NULL, kind TEXT NOT NULL, expires_ms BIGINT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS docs (ns TEXT NOT NULL, key TEXT NOT NULL, body TEXT NOT NULL, version BIGINT NOT NULL, updated_by TEXT NOT NULL, updated_ms BIGINT NOT NULL, PRIMARY KEY (ns, key))",
    "CREATE TABLE IF NOT EXISTS doc_history (ns TEXT NOT NULL, key TEXT NOT NULL, version BIGINT NOT NULL, body TEXT NOT NULL, updated_by TEXT NOT NULL, updated_ms BIGINT NOT NULL, PRIMARY KEY (ns, key, version))",
    "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, state TEXT NOT NULL, prompt TEXT NOT NULL, repo TEXT NOT NULL, callback TEXT NOT NULL, secret TEXT NOT NULL, created_by TEXT NOT NULL, worker TEXT NOT NULL, result TEXT NOT NULL, created_ms BIGINT NOT NULL, updated_ms BIGINT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS audit (seq BIGINT PRIMARY KEY, ts_ms BIGINT NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL, prev TEXT NOT NULL, hash TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS vault (name TEXT PRIMARY KEY, nonce TEXT NOT NULL, sealed TEXT NOT NULL, updated_by TEXT NOT NULL, updated_ms BIGINT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS shares (hash TEXT PRIMARY KEY, ns TEXT NOT NULL, key TEXT NOT NULL, created_by TEXT NOT NULL, expires_ms BIGINT NOT NULL)",
];

enum Engine {
    Sqlite(Mutex<rusqlite::Connection>),
    Postgres {
        url: String,
        client: tokio::sync::Mutex<Option<Arc<tokio_postgres::Client>>>,
    },
}

/// A database handle; cheap to clone.
#[derive(Clone)]
pub struct Db(Arc<Engine>);

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.engine())
    }
}

impl Db {
    /// `sqlite:<path>`, `sqlite::memory:` or `postgres://…`.
    ///
    /// # Errors
    /// An unknown scheme, an insecure Postgres URL, or the database cannot be opened.
    pub async fn open(url: &str, base: &Path) -> Result<Self, String> {
        let db = if let Some(rest) = url.strip_prefix("sqlite:") {
            let conn = if rest == ":memory:" {
                rusqlite::Connection::open_in_memory()
            } else {
                let p = base.join(rest);
                if let Some(d) = p.parent() {
                    // kor-lint: allow(blocking_in_async) once at server start, before any request
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                rusqlite::Connection::open(p)
            }
            .map_err(|e| e.to_string())?;
            conn.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
            )
            .map_err(|e| e.to_string())?;
            Self(Arc::new(Engine::Sqlite(Mutex::new(conn))))
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            crate::pg_tls::check_url(url)?;
            let client = crate::pg_tls::connect(url).await?;
            Self(Arc::new(Engine::Postgres {
                url: url.to_owned(),
                client: tokio::sync::Mutex::new(Some(Arc::new(client))),
            }))
        } else {
            return Err("db must be `sqlite:<file>` or `postgres://…`".into());
        };
        for s in SCHEMA {
            db.exec(s, &[]).await?;
        }
        Ok(db)
    }

    /// `sqlite` or `postgres`.
    pub fn engine(&self) -> &'static str {
        match &*self.0 {
            Engine::Sqlite(_) => "sqlite",
            Engine::Postgres { .. } => "postgres",
        }
    }

    /// Run a statement; the number of rows it changed.
    ///
    /// # Errors
    /// A database error.
    pub async fn exec(&self, sql: &str, params: &[Val]) -> Result<u64, String> {
        match &*self.0 {
            Engine::Sqlite(c) => {
                let c = c.lock().unwrap_or_else(PoisonError::into_inner);
                let mut st = c.prepare_cached(sql).map_err(|e| e.to_string())?;
                st.execute(rusqlite::params_from_iter(params.iter()))
                    .map(|n| n as u64)
                    .map_err(|e| e.to_string())
            }
            Engine::Postgres { .. } => {
                let client = self.pg().await?;
                let p = pg_params(params);
                let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                    p.iter().map(|b| &**b as _).collect();
                client.execute(sql, &refs).await.map_err(|e| e.to_string())
            }
        }
    }

    /// Run a query; every row with its columns in order.
    ///
    /// # Errors
    /// A database error.
    pub async fn query(&self, sql: &str, params: &[Val]) -> Result<Vec<Row>, String> {
        match &*self.0 {
            Engine::Sqlite(c) => {
                use rusqlite::types::ValueRef;
                let c = c.lock().unwrap_or_else(PoisonError::into_inner);
                let mut st = c.prepare_cached(sql).map_err(|e| e.to_string())?;
                let n = st.column_count();
                let mut rows = st
                    .query(rusqlite::params_from_iter(params.iter()))
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                while let Some(r) = rows.next().map_err(|e| e.to_string())? {
                    let mut row = Vec::with_capacity(n);
                    for i in 0..n {
                        row.push(match r.get_ref(i).map_err(|e| e.to_string())? {
                            ValueRef::Integer(v) => Val::Int(v),
                            ValueRef::Text(t) => Val::Text(String::from_utf8_lossy(t).into_owned()),
                            ValueRef::Null => Val::Null,
                            ValueRef::Real(_) | ValueRef::Blob(_) => {
                                return Err("unexpected column type".into());
                            }
                        });
                    }
                    out.push(row);
                }
                Ok(out)
            }
            Engine::Postgres { .. } => {
                let client = self.pg().await?;
                let p = pg_params(params);
                let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                    p.iter().map(|b| &**b as _).collect();
                let rows = client.query(sql, &refs).await.map_err(|e| e.to_string())?;
                rows.iter()
                    .map(|r| {
                        (0..r.len())
                            .map(|i| {
                                if *r.columns()[i].type_() == tokio_postgres::types::Type::INT8 {
                                    r.try_get::<_, Option<i64>>(i)
                                        .map(|v| v.map_or(Val::Null, Val::Int))
                                } else {
                                    r.try_get::<_, Option<String>>(i)
                                        .map(|v| v.map_or(Val::Null, Val::Text))
                                }
                                .map_err(|e| e.to_string())
                            })
                            .collect()
                    })
                    .collect()
            }
        }
    }

    /// The live Postgres client, reconnecting once when the connection was lost.
    async fn pg(&self) -> Result<Arc<tokio_postgres::Client>, String> {
        let Engine::Postgres { url, client } = &*self.0 else {
            return Err("not postgres".into());
        };
        let mut c = client.lock().await;
        if c.as_ref().is_none_or(|c| c.is_closed()) {
            *c = Some(Arc::new(crate::pg_tls::connect(url).await?));
        }
        c.clone().ok_or_else(|| "no database connection".into())
    }
}

fn pg_params(params: &[Val]) -> Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> {
    params
        .iter()
        .map(|v| -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
            match v {
                Val::Int(i) => Box::new(*i),
                Val::Text(s) => Box::new(s.clone()),
                Val::Null => Box::new(None::<String>),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_binds_dollar_placeholders_in_order() {
        let db = Db::open("sqlite::memory:", Path::new(".")).await.unwrap();
        db.exec(
            "INSERT INTO members (email, role, created_ms, added_by) VALUES ($1, $2, $3, $4)",
            &["a@x".into(), "admin".into(), 5i64.into(), "init".into()],
        )
        .await
        .unwrap();
        let rows = db
            .query(
                "SELECT email, created_ms FROM members WHERE role = $1",
                &["admin".into()],
            )
            .await
            .unwrap();
        assert_eq!(rows, vec![vec![Val::from("a@x"), Val::Int(5)]]);
        // A value with SQL in it stays a value.
        let rows = db
            .query(
                "SELECT email FROM members WHERE email = $1",
                &["x' OR '1'='1".into()],
            )
            .await
            .unwrap();
        assert!(rows.is_empty());
    }
}
