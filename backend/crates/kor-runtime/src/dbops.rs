//! Database-aware agent helpers (spec §8.5, ADR 0041): the project's migration style for
//! `kor db migrate new`, and index suggestions from live snapshots (foreign keys without an index,
//! columns filtered by slow queries). Live schema, slow queries and the copy-first rule for
//! destructive SQL are in `kor_agent::remote::db`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use kor_graph::DbSnapshot;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationStyle {
    /// `prisma`, `rails`, `django`, `alembic`, `laravel`, `knex`, `ef-core`, `diesel`, `sql`.
    pub framework: String,
    /// Folder holding the migrations, relative to the root.
    pub dir: String,
    /// The framework's own generator, when it has one.
    pub generator: Option<String>,
    /// Newest existing migrations (style examples), relative paths.
    pub examples: Vec<String>,
}

fn files_in(root: &Path, dir: &str) -> Vec<String> {
    let mut out: Vec<String> = ignore::WalkBuilder::new(root.join(dir))
        .max_depth(Some(3))
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            e.path()
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .filter(|p| {
            let ext = Path::new(p)
                .extension()
                .map(std::ffi::OsStr::to_ascii_lowercase);
            !matches!(
                ext.as_deref().and_then(std::ffi::OsStr::to_str),
                Some("md" | "lock")
            ) && !p.contains("__pycache__")
                && !p.ends_with("__init__.py")
                && !p.ends_with("migration_lock.toml")
        })
        .collect();
    out.sort();
    out
}

/// `add users email` → `add_users_email`.
pub fn slug(desc: &str) -> String {
    let s: String = desc
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let s = s
        .split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    s.chars().take(60).collect()
}

fn camel(desc: &str) -> String {
    slug(desc)
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn exists(root: &Path, p: &str) -> bool {
    root.join(p).exists()
}

fn glob_root(root: &Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == ext))
}

/// Detect how this project writes migrations.
pub fn migration_style(root: &Path, desc: &str) -> MigrationStyle {
    let (s, c) = (slug(desc), camel(desc));
    let pick = |framework: &str, dir: &str, generator: Option<String>| MigrationStyle {
        framework: framework.into(),
        dir: dir.into(),
        generator,
        examples: files_in(root, dir).into_iter().rev().take(2).collect(),
    };
    if exists(root, "prisma/schema.prisma") {
        return pick(
            "prisma",
            "prisma/migrations",
            Some(format!("npx prisma migrate dev --create-only --name {s}")),
        );
    }
    if exists(root, "db/migrate") && exists(root, "Gemfile") {
        return pick(
            "rails",
            "db/migrate",
            Some(format!("bin/rails generate migration {c}")),
        );
    }
    if exists(root, "manage.py") {
        let dir = ignore::WalkBuilder::new(root)
            .max_depth(Some(3))
            .build()
            .flatten()
            .find(|e| e.file_name() == "migrations" && e.path().is_dir())
            .and_then(|e| {
                e.path()
                    .strip_prefix(root)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .unwrap_or_else(|| "migrations".into());
        return pick(
            "django",
            &dir,
            Some(format!("python manage.py makemigrations --name {s}")),
        );
    }
    if exists(root, "alembic.ini") {
        let dir = if exists(root, "alembic/versions") {
            "alembic/versions"
        } else {
            "migrations/versions"
        };
        return pick(
            "alembic",
            dir,
            Some(format!(
                "alembic revision -m \"{}\"",
                desc.replace('"', "'")
            )),
        );
    }
    if exists(root, "artisan") {
        return pick(
            "laravel",
            "database/migrations",
            Some(format!("php artisan make:migration {s}")),
        );
    }
    if ["knexfile.js", "knexfile.ts", "knexfile.cjs", "knexfile.mjs"]
        .iter()
        .any(|f| exists(root, f))
    {
        return pick(
            "knex",
            "migrations",
            Some(format!("npx knex migrate:make {s}")),
        );
    }
    if glob_root(root, "csproj").is_some() && exists(root, "Migrations") {
        return pick(
            "ef-core",
            "Migrations",
            Some(format!("dotnet ef migrations add {c}")),
        );
    }
    if exists(root, "diesel.toml") {
        return pick(
            "diesel",
            "migrations",
            Some(format!("diesel migration generate {s}")),
        );
    }
    let dir = [
        "migrations",
        "db/migrations",
        "sql/migrations",
        "database/migrations",
    ]
    .into_iter()
    .find(|d| exists(root, d))
    .unwrap_or("migrations");
    pick("sql", dir, None)
}

/// The agent's task for `kor db migrate new "<desc>"`.
pub fn migration_prompt(style: &MigrationStyle, desc: &str) -> String {
    let examples = if style.examples.is_empty() {
        "none yet".to_owned()
    } else {
        style.examples.join(", ")
    };
    let how = match &style.generator {
        Some(g) => format!(
            "Create it with the framework's generator (`{g}`) when that tool is installed; otherwise write the file by hand with the same name pattern and content shape."
        ),
        None => format!(
            "Write plain SQL in `{}/`, named like the existing files (next number or a UTC timestamp, then `_{}`); add a matching down/rollback file if the existing ones have one.",
            style.dir,
            slug(desc)
        ),
    };
    format!(
        "Write a database migration: {desc}\n\n\
         Project style: {fw} migrations in `{dir}/`. Newest examples: {examples} — read them first and copy their conventions.\n\
         {how}\n\
         Check the current schema first: graph_explain on the affected tables (`table:<name>`) shows columns, foreign keys, indexes and the code that reads or writes them; update that code too if the migration changes what it uses.\n\
         Make it reversible when the framework supports it. Never apply it to a database: the user reviews it and runs it (destructive SQL goes through `kor db apply`, which tries it on a copy first).",
        fw = style.framework,
        dir = style.dir,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexSuggestion {
    pub connection: String,
    pub table: String,
    pub columns: Vec<String>,
    pub reason: String,
    pub sql: String,
}

static FILTER_COL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:where|and|on)\s+(?:[\w"`]+\.)?["`]?(\w+)["`]?\s*(?:=|<|>|\bin\b|\blike\b|\bis\b)"#).expect("filter column regex")
});

/// First column of each index.
fn leading(indexes: &[String]) -> Vec<String> {
    indexes
        .iter()
        .filter_map(|i| {
            i.split_once('(')
                .and_then(|(_, r)| r.split([',', ')']).next())
        })
        .map(|c| c.trim().to_ascii_lowercase())
        .collect()
}

/// Indexes worth adding: a foreign-key column that leads no index, and a column a slow query
/// filters on that leads no index.
pub fn suggest_indexes(snaps: &[DbSnapshot]) -> Vec<IndexSuggestion> {
    let mut out: Vec<IndexSuggestion> = Vec::new();
    for s in snaps {
        for t in &s.tables {
            let lead = leading(&t.indexes);
            let cols: Vec<String> = t
                .columns
                .iter()
                .filter_map(|c| c.split_whitespace().next())
                .map(str::to_ascii_lowercase)
                .collect();
            let short = t.name.rsplit('.').next().unwrap_or(&t.name);
            let mut add = |col: &str, reason: String| {
                let col = col.to_ascii_lowercase();
                if lead.contains(&col)
                    || out.iter().any(|o| {
                        o.connection == s.connection
                            && o.table == t.name
                            && o.columns == [col.clone()]
                    })
                {
                    return;
                }
                out.push(IndexSuggestion {
                    connection: s.connection.clone(),
                    table: t.name.clone(),
                    sql: format!("CREATE INDEX idx_{short}_{col} ON {} ({col});", t.name),
                    columns: vec![col],
                    reason,
                });
            };
            for fk in &t.fk_columns {
                add(
                    fk,
                    "foreign key without an index (joins and ON DELETE scan the table)".into(),
                );
            }
            let key = kor_graph::db::table_key(&t.name);
            for q in s
                .slow_queries
                .iter()
                .filter(|q| kor_graph::db::mentions_table(&q.query, &key))
            {
                for c in FILTER_COL.captures_iter(&q.query).filter_map(|c| c.get(1)) {
                    let col = c.as_str().to_ascii_lowercase();
                    if cols.contains(&col) {
                        #[allow(clippy::cast_precision_loss)]
                        let ms = q.mean_us as f64 / 1000.0;
                        add(
                            &col,
                            format!(
                                "filtered by a slow query ({ms:.1} ms average, {} calls)",
                                q.calls
                            ),
                        );
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_graph::{LiveTable, SlowQuery};

    #[test]
    fn styles_and_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        assert_eq!(migration_style(r, "x").framework, "sql");
        std::fs::create_dir_all(r.join("db/migrate")).unwrap();
        std::fs::write(r.join("Gemfile"), "").unwrap();
        std::fs::write(r.join("db/migrate/20240101_create_users.rb"), "").unwrap();
        let s = migration_style(r, "Add email to users!");
        assert_eq!(s.framework, "rails");
        assert_eq!(
            s.generator.as_deref(),
            Some("bin/rails generate migration AddEmailToUsers")
        );
        assert_eq!(s.examples, vec!["db/migrate/20240101_create_users.rb"]);
        let p = migration_prompt(&s, "Add email to users!");
        assert!(p.contains("Never apply") && p.contains("db/migrate"));
        std::fs::create_dir_all(r.join("prisma")).unwrap();
        std::fs::write(r.join("prisma/schema.prisma"), "").unwrap();
        assert_eq!(
            migration_style(r, "a b").generator.as_deref(),
            Some("npx prisma migrate dev --create-only --name a_b")
        );
    }

    #[test]
    fn index_suggestions() {
        let snap = DbSnapshot {
            connection: "main".into(),
            tables: vec![LiveTable {
                name: "public.orders".into(),
                columns: vec![
                    "id int".into(),
                    "user_id int".into(),
                    "status text".into(),
                    "shop_id int".into(),
                ],
                indexes: vec![
                    "orders_pkey (id) unique".into(),
                    "orders_shop (shop_id,id)".into(),
                ],
                fk_columns: vec!["user_id".into(), "shop_id".into()],
                ..LiveTable::default()
            }],
            slow_queries: vec![SlowQuery {
                query: "SELECT * FROM orders o WHERE o.status = $1 AND id > $2".into(),
                calls: 9,
                mean_us: 120_000,
                total_us: 0,
            }],
            ..DbSnapshot::default()
        };
        let s = suggest_indexes(&[snap]);
        let cols: Vec<&str> = s.iter().map(|x| x.columns[0].as_str()).collect();
        assert_eq!(cols, vec!["user_id", "status"]);
        assert_eq!(
            s[0].sql,
            "CREATE INDEX idx_orders_user_id ON public.orders (user_id);"
        );
        assert!(s[1].reason.contains("120.0 ms"));
    }
}
