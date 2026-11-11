//! Database-schema layer (spec §7.3 "DB şeması"): `Table` nodes from schema files and live
//! snapshots (`kor db connect`), `References` edges for foreign keys, and `ReadsTable` /
//! `WritesTable` edges from SQL string literals and common ORM calls in code.
//!
//! Offline sources: SQL DDL (`CREATE TABLE`, `ALTER TABLE ... ADD / REFERENCES`, in `*.sql` files
//! or code strings), Prisma `schema.prisma`, Rails `create_table` / `add_foreign_key`, Django
//! `models.Model` classes, EF Core `DbSet<T>`, Knex `createTable` and Laravel `Schema::create`
//! (names only for the last three). Live snapshots are JSON files under
//! `<root>/.kor/graph/db/<connection>.json` holding table, column and foreign-key names only.

use crate::graph::Graph;
use crate::lang::Lang;
use crate::model::{Confidence, EdgeKind, NodeId, NodeKind, NodeRef};
use crate::parse::FileFacts;
use crate::query::Link;
use crate::resolve::{Derived, dir_of};
use crate::{Error, Result};
use petgraph::Direction;
use petgraph::visit::EdgeRef;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Node id prefix of tables (`table:<lowercase name>`).
pub const TABLE_PREFIX: &str = "table:";
const MAX_COLUMNS: usize = 200;
const MAX_TABLES: usize = 2000;
const MAX_USES: usize = 500;
const MAX_LITERAL: usize = 20_000;

/// A table defined by a schema source in one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableDef {
    /// Table name as written; empty for a Django model without `db_table` (named after its app).
    pub name: String,
    /// ORM model / class name (`User`), when the source has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `name type` per column.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// Referenced tables or models (foreign keys), as written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    pub start: u32,
    pub end: u32,
}

/// A table read or written by code (or by a `.sql` file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableUse {
    pub name: String,
    pub write: bool,
    /// Guessed from an ORM call rather than literal SQL.
    #[serde(default)]
    pub orm: bool,
    /// Enclosing definition index; `None` = file level.
    pub from: Option<u32>,
    pub line: u32,
}

/// Schema of a live database, read with catalog queries only. Never holds data or credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbSnapshot {
    pub connection: String,
    /// `sqlite`, `postgres` or `mysql`.
    pub kind: String,
    /// Unix seconds.
    pub taken_at: u64,
    pub tables: Vec<LiveTable>,
    /// Slowest statements (`kor db connect --slow`, opt-in): `pg_stat_statements`,
    /// `performance_schema` or `sys.dm_exec_query_stats`, string literals replaced by `'?'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slow_queries: Vec<SlowQuery>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTable {
    pub name: String,
    pub columns: Vec<String>,
    /// Tables this one has foreign keys to.
    #[serde(default)]
    pub references: Vec<String>,
    /// `name (col1,col2)` with ` unique` appended for unique indexes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<String>,
    /// Columns that hold a foreign key (index suggestions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fk_columns: Vec<String>,
}

/// One normalized statement from the database's own statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlowQuery {
    pub query: String,
    pub calls: u64,
    /// Mean execution time in microseconds.
    pub mean_us: u64,
    pub total_us: u64,
}

/// One table with its schema and the code touching it (`kor db show`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableInfo {
    pub node: NodeRef,
    pub columns: Vec<String>,
    pub references: Vec<NodeRef>,
    pub referenced_by: Vec<NodeRef>,
    pub readers: Vec<Link>,
    pub writers: Vec<Link>,
    /// Live connections whose snapshot has this table.
    pub live: Vec<String>,
    /// Indexes from live snapshots.
    #[serde(default)]
    pub indexes: Vec<String>,
    /// Slow statements (opt-in snapshots) that mention this table.
    #[serde(default)]
    pub slow_queries: Vec<SlowQuery>,
}

// ---------------------------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------------------------

fn unquote(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']' | '\''))
}

/// `public."Users"` / `` `app`.`users` `` → `users`. Schemas are dropped: two schemas with the same
/// table name share one node.
pub fn table_key(raw: &str) -> String {
    let last = raw.trim().rsplit('.').next().unwrap_or(raw);
    unquote(last).to_lowercase()
}

/// `query` names table `key` as a whole word (optionally schema- or quote-qualified).
pub fn mentions_table(query: &str, key: &str) -> bool {
    let q = query.to_lowercase();
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    q.match_indices(key).any(|(i, _)| {
        !q[..i].chars().next_back().is_some_and(word)
            && !q[i + key.len()..].chars().next().is_some_and(word)
    })
}

/// Loose key for model ↔ table matching: no underscores, singular (`user_profiles` → `userprofile`).
fn alias_key(key: &str) -> String {
    let k = key.replace('_', "");
    if let Some(s) = k.strip_suffix("ies") {
        format!("{s}y")
    } else if k.ends_with("ses") || k.ends_with("xes") {
        k[..k.len() - 2].to_owned()
    } else if k.ends_with('s') && !k.ends_with("ss") {
        k[..k.len() - 1].to_owned()
    } else {
        k
    }
}

/// Django's default table name: `<app>_<model>`, the app being the folder of `models.py`
/// (or the parent of a `models/` package).
fn django_table(path: &str, model: &str) -> String {
    let mut dir = dir_of(path);
    if dir == "models" || dir.ends_with("/models") {
        dir = dir_of(dir);
    }
    let app = dir.rsplit('/').next().unwrap_or("");
    if app.is_empty() {
        model.to_lowercase()
    } else {
        format!("{app}_{}", model.to_lowercase())
    }
}

/// Byte offset → 1-based line.
struct Lines(Vec<usize>);

impl Lines {
    fn new(text: &str) -> Self {
        let mut v = vec![0];
        v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self(v)
    }

    fn at(&self, byte: usize) -> u32 {
        let i = self.0.partition_point(|&s| s <= byte);
        u32::try_from(i.max(1)).unwrap_or(u32::MAX)
    }
}

fn re(s: &str) -> Option<Regex> {
    Regex::new(s).ok()
}

// ---------------------------------------------------------------------------------------------
// SQL DDL
// ---------------------------------------------------------------------------------------------

const NAME: &str = r#"([A-Za-z_"`\[][\w."`\[\]$]*)"#;

static CREATE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(&format!(
        r"(?i)\bcreate\s+(?:or\s+replace\s+)?(?:(?:global|local)\s+)?(?:temporary\s+|temp\s+|unlogged\s+)?table\s+(?:if\s+not\s+exists\s+)?{NAME}"
    ))
});
static ALTER_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(&format!(
        r"(?i)\balter\s+table\s+(?:only\s+)?(?:if\s+exists\s+)?{NAME}"
    ))
});
static REF_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(&format!(r"(?i)\breferences\s+{NAME}")));
static ADD_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r#"(?i)\badd\s+(?:column\s+)?(?:if\s+not\s+exists\s+)?([A-Za-z_"`\[][\w"`\]]*)\s+([A-Za-z]\w*(?:\s*\([^)]*\))?)"#,
    )
});
static TYPE_STOP_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r"(?i)\s(?:not|null|default|primary|references|unique|check|constraint|generated|collate|auto_increment|autoincrement|identity|comment|on)\b",
    )
});

const CONSTRAINT_WORDS: &[&str] = &[
    "constraint",
    "primary",
    "foreign",
    "unique",
    "check",
    "key",
    "index",
    "exclude",
    "fulltext",
    "spatial",
];

/// `--` and `/* */` comments → spaces (newlines kept so line numbers stay right).
fn strip_sql_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c == '-' && it.peek() == Some(&'-') {
            for c in it.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && it.peek() == Some(&'*') {
            it.next();
            let mut prev = ' ';
            for c in it.by_ref() {
                if c == '\n' {
                    out.push('\n');
                }
                if prev == '*' && c == '/' {
                    break;
                }
                prev = c;
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Text inside the parenthesis opening at byte `open`, and the byte after the closing one.
fn balanced(text: &str, open: usize) -> Option<(&str, usize)> {
    let mut depth = 0u32;
    let mut quote: Option<char> = None;
    for (i, c) in text.get(open..)?.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((&text[open + 1..open + i], open + i + 1));
                }
            }
            _ => {}
        }
        if i > 200_000 {
            return None;
        }
    }
    None
}

/// Split at top-level commas.
fn items(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0u32, 0usize);
    let mut quote: Option<char> = None;
    for (i, c) in body.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out
}

fn refs_in(s: &str) -> impl Iterator<Item = String> + '_ {
    REF_RE
        .as_ref()
        .into_iter()
        .flat_map(move |r| r.captures_iter(s).map(|c| c[1].to_owned()))
}

/// One `CREATE TABLE` item: a column (`name type`) and/or a foreign-key target.
fn column_item(item: &str) -> (Option<String>, Vec<String>) {
    let item = item.trim();
    let refs: Vec<String> = refs_in(item).collect();
    let Some((name, rest)) = item.split_once(char::is_whitespace) else {
        return (None, refs);
    };
    if CONSTRAINT_WORDS.contains(&name.to_ascii_lowercase().as_str()) {
        return (None, refs);
    }
    let name = unquote(name);
    if name.is_empty() {
        return (None, refs);
    }
    let rest = format!(" {}", rest.trim());
    let cut = TYPE_STOP_RE
        .as_ref()
        .and_then(|r| r.find(&rest))
        .map_or(rest.len(), |m| m.start());
    let ty: String = rest[..cut]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    (Some(format!("{name} {ty}").trim().to_owned()), refs)
}

/// `CREATE TABLE` and `ALTER TABLE` statements anywhere in `text` (SQL file or code).
fn ddl(text: &str, lines: &Lines) -> Vec<TableDef> {
    let mut out = Vec::new();
    if let Some(create) = CREATE_RE.as_ref() {
        for c in create.captures_iter(text) {
            let (Some(m), Some(name)) = (c.get(0), c.get(1)) else {
                continue;
            };
            let mut def = TableDef {
                name: name.as_str().to_owned(),
                start: lines.at(m.start()),
                end: lines.at(m.end()),
                ..TableDef::default()
            };
            let after = &text[m.end()..];
            let skip = after.len() - after.trim_start().len();
            if after.trim_start().starts_with('(')
                && let Some((body, end)) = balanced(text, m.end() + skip)
            {
                for it in items(body) {
                    let (col, refs) = column_item(it);
                    if let Some(col) = col
                        && def.columns.len() < MAX_COLUMNS
                    {
                        def.columns.push(col);
                    }
                    def.refs.extend(refs);
                }
                def.end = lines.at(end.saturating_sub(1));
            }
            out.push(def);
            if out.len() >= MAX_TABLES {
                return out;
            }
        }
    }
    if let Some(alter) = ALTER_RE.as_ref() {
        for c in alter.captures_iter(text) {
            let (Some(m), Some(name)) = (c.get(0), c.get(1)) else {
                continue;
            };
            let rest = &text[m.end()..];
            let stmt = &rest[..rest.find(';').unwrap_or(rest.len())];
            let stmt = stmt.get(..4000).unwrap_or(stmt);
            let mut def = TableDef {
                name: name.as_str().to_owned(),
                start: lines.at(m.start()),
                end: lines.at(m.end()),
                refs: refs_in(stmt).collect(),
                ..TableDef::default()
            };
            if let Some(add) = ADD_RE.as_ref() {
                for a in add.captures_iter(stmt) {
                    let col = unquote(&a[1]);
                    if !CONSTRAINT_WORDS.contains(&col.to_ascii_lowercase().as_str()) {
                        def.columns.push(format!("{col} {}", &a[2]));
                    }
                }
            }
            out.push(def);
            if out.len() >= MAX_TABLES {
                break;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SQL use: statements in `.sql` files and SQL string literals in code
// ---------------------------------------------------------------------------------------------

static USE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(&format!(
        r"(?i)\b(insert\s+(?:or\s+\w+\s+)?into|replace\s+into|merge\s+into|delete\s+from|update(?:\s+only)?|from|join)\s+{NAME}"
    ))
});
/// A quote followed by an SQL verb: the start of an SQL string literal.
static SQL_LITERAL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"["'`]\s*(?i:select|insert|update|delete|with|merge|replace)\b"#));

fn sql_uses(sql: &str, offset: usize, lines: &Lines, out: &mut Vec<TableUse>) {
    let Some(r) = USE_RE.as_ref() else { return };
    for c in r.captures_iter(sql) {
        let (Some(kw), Some(name)) = (c.get(1), c.get(2)) else {
            continue;
        };
        let k = kw.as_str().to_ascii_lowercase();
        out.push(TableUse {
            name: name.as_str().to_owned(),
            write: !k.starts_with("from") && !k.starts_with("join"),
            orm: false,
            from: None,
            line: lines.at(offset + kw.start()),
        });
    }
}

fn literal_uses(text: &str, lines: &Lines, out: &mut Vec<TableUse>) {
    let Some(r) = SQL_LITERAL_RE.as_ref() else {
        return;
    };
    let mut next = 0;
    for m in r.find_iter(text) {
        if m.start() < next {
            continue;
        }
        let quote = text[m.start()..].chars().next().unwrap_or('"');
        let body_start = m.start() + 1;
        let mut end = text.len();
        let mut escaped = false;
        for (i, c) in text[body_start..].char_indices() {
            if i > MAX_LITERAL {
                end = body_start + i;
                break;
            }
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                end = body_start + i;
                break;
            }
        }
        sql_uses(&text[body_start..end], body_start, lines, out);
        next = end;
    }
}

// ---------------------------------------------------------------------------------------------
// Prisma, Rails, Django, EF, Knex, Laravel
// ---------------------------------------------------------------------------------------------

static PRISMA_MODEL_RE: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"^model\s+(\w+)\s*\{"));
static PRISMA_MAP_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"^@@map\(\s*(?:name:\s*)?"([^"]+)""#));

/// `schema.prisma`: `model X { .. }` blocks; relation fields with `fields:` are foreign keys.
fn prisma(text: &str) -> Vec<TableDef> {
    struct Field {
        name: String,
        ty: String,
        fk: bool,
    }
    let (Some(model_re), Some(map_re)) = (PRISMA_MODEL_RE.as_ref(), PRISMA_MAP_RE.as_ref()) else {
        return Vec::new();
    };
    let mut models: Vec<(TableDef, Vec<Field>)> = Vec::new();
    let mut cur: Option<(TableDef, Vec<Field>)> = None;
    for (i, l) in text.lines().enumerate() {
        let line = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let t = l.trim();
        let Some((def, fields)) = cur.as_mut() else {
            if let Some(c) = model_re.captures(t) {
                let def = TableDef {
                    name: c[1].to_owned(),
                    model: Some(c[1].to_owned()),
                    start: line,
                    end: line,
                    ..TableDef::default()
                };
                cur = Some((def, Vec::new()));
            }
            continue;
        };
        if t.starts_with('}') {
            def.end = line;
            if let Some(done) = cur.take() {
                models.push(done);
            }
            continue;
        }
        if let Some(c) = map_re.captures(t) {
            c[1].clone_into(&mut def.name);
            continue;
        }
        if t.is_empty() || t.starts_with("//") || t.starts_with('@') {
            continue;
        }
        let mut w = t.split_whitespace();
        if let (Some(name), Some(ty)) = (w.next(), w.next()) {
            fields.push(Field {
                name: name.to_owned(),
                ty: ty.trim_end_matches('?').trim_end_matches("[]").to_owned(),
                fk: t.contains("@relation(") && t.contains("fields:"),
            });
        }
    }
    let names: Vec<String> = models.iter().filter_map(|(d, _)| d.model.clone()).collect();
    models
        .into_iter()
        .map(|(mut def, fields)| {
            for f in fields {
                if names.contains(&f.ty) {
                    if f.fk {
                        def.refs.push(f.ty);
                    }
                } else if def.columns.len() < MAX_COLUMNS {
                    def.columns.push(format!("{} {}", f.name, f.ty));
                }
            }
            def
        })
        .collect()
}

static RB_CREATE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"^\s*create_table\s*\(?\s*[:"']?(\w+)"#));
static RB_COL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"^\s*t\.(\w+)\s*\(?\s*[:"'](\w+)"#));
static RB_FK_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"^\s*add_foreign_key\s*\(?\s*[:"']?(\w+)["']?\s*,\s*[:"']?(\w+)"#));

/// Rails `db/schema.rb` and migrations.
fn rails(text: &str) -> Vec<TableDef> {
    let (Some(create), Some(col), Some(fk)) =
        (RB_CREATE_RE.as_ref(), RB_COL_RE.as_ref(), RB_FK_RE.as_ref())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur: Option<TableDef> = None;
    for (i, l) in text.lines().enumerate() {
        let line = u32::try_from(i + 1).unwrap_or(u32::MAX);
        if let Some(def) = cur.as_mut() {
            if l.trim() == "end" {
                def.end = line;
                out.extend(cur.take());
            } else if let Some(c) = col.captures(l) {
                let (method, name) = (&c[1], &c[2]);
                match method {
                    "references" | "belongs_to" => {
                        def.columns.push(format!("{name}_id references"));
                        if l.contains("foreign_key") {
                            def.refs.push(name.to_owned());
                        }
                    }
                    "index" | "timestamps" | "check_constraint" => {}
                    _ if def.columns.len() < MAX_COLUMNS => {
                        def.columns.push(format!("{name} {method}"));
                    }
                    _ => {}
                }
            }
            continue;
        }
        if let Some(c) = create.captures(l) {
            cur = Some(TableDef {
                name: c[1].to_owned(),
                start: line,
                end: line,
                ..TableDef::default()
            });
        } else if let Some(c) = fk.captures(l) {
            out.push(TableDef {
                name: c[1].to_owned(),
                refs: vec![c[2].to_owned()],
                start: line,
                end: line,
                ..TableDef::default()
            });
        }
    }
    out.extend(cur);
    out
}

static PY_CLASS_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"^(\s*)class\s+(\w+)\s*\(([^)]*)\)\s*:"));
static PY_FIELD_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"^\s+(\w+)\s*=\s*models\.(\w+)\(\s*['"]?([\w.]*)"#));
static PY_TABLE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"db_table\s*=\s*['"]([\w.]+)['"]"#));

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

/// Django `class X(models.Model)` with its fields; `ForeignKey` / `OneToOneField` /
/// `ManyToManyField` targets are references. Abstract models are skipped.
fn django(text: &str) -> Vec<TableDef> {
    let (Some(class), Some(field), Some(table)) = (
        PY_CLASS_RE.as_ref(),
        PY_FIELD_RE.as_ref(),
        PY_TABLE_RE.as_ref(),
    ) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(c) = class.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let bases = &c[3];
        if !(bases.contains("models.Model") || bases.split(',').any(|b| b.trim() == "Model")) {
            i += 1;
            continue;
        }
        let base_indent = c[1].len();
        let start = i;
        let mut def = TableDef {
            model: Some(c[2].to_owned()),
            start: u32::try_from(i + 1).unwrap_or(u32::MAX),
            ..TableDef::default()
        };
        let mut abstract_model = false;
        i += 1;
        while i < lines.len() && (lines[i].trim().is_empty() || indent(lines[i]) > base_indent) {
            let l = lines[i];
            if let Some(f) = field.captures(l) {
                def.columns.push(format!("{} {}", &f[1], &f[2]));
                if matches!(&f[2], "ForeignKey" | "OneToOneField" | "ManyToManyField")
                    && !f[3].is_empty()
                {
                    def.refs.push(f[3].to_owned());
                }
            } else if let Some(t) = table.captures(l) {
                t[1].clone_into(&mut def.name);
            } else if l.contains("abstract") && l.contains("True") {
                abstract_model = true;
            }
            i += 1;
        }
        let last = (start + 1..i).rev().find(|j| !lines[*j].trim().is_empty());
        def.end = u32::try_from(last.unwrap_or(start) + 1).unwrap_or(u32::MAX);
        def.columns.truncate(MAX_COLUMNS);
        if !abstract_model {
            out.push(def);
        }
    }
    out
}

static EF_DBSET_RE: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"DbSet<\s*(\w+)\s*>\s+(\w+)"));
static CREATE_CALL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"(?:\.createTable|Schema::create)\(\s*['"]([\w.]+)['"]"#));

/// Names-only table definitions from EF Core `DbSet<T> Name`, Knex `createTable('x')` and Laravel
/// `Schema::create('x')`.
fn named_tables(lang: Lang, text: &str, lines: &Lines) -> Vec<TableDef> {
    let (r, csharp) = match lang {
        Lang::CSharp => (EF_DBSET_RE.as_ref(), true),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Php => {
            (CREATE_CALL_RE.as_ref(), false)
        }
        _ => return Vec::new(),
    };
    let Some(r) = r else { return Vec::new() };
    r.captures_iter(text)
        .filter_map(|c| {
            let line = lines.at(c.get(0)?.start());
            let (name, model) = if csharp {
                (c[2].to_owned(), Some(c[1].to_owned()))
            } else {
                (c[1].to_owned(), None)
            };
            Some(TableDef {
                name,
                model,
                start: line,
                end: line,
                ..TableDef::default()
            })
        })
        .collect()
}

static PRISMA_CALL_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r"\.(\w+)\.(findMany|findUniqueOrThrow|findUnique|findFirstOrThrow|findFirst|count|aggregate|groupBy|createManyAndReturn|createMany|create|updateMany|update|upsert|deleteMany|delete)\s*\(",
    )
});
static QUERY_BUILDER_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(r#"(\bknex|\bDB::table|->table|\.table|->from|\.from|\.into)\(\s*['"]([\w.]+)['"]\s*\)"#)
});
static DJANGO_CALL_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"\b([A-Z]\w*)\.objects\.(\w+)"));
static SQLA_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"(\.query|\bselect|\binsert|\bupdate|\bdelete)\(\s*([A-Z]\w*)\s*[),]"));
static AR_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r"\b([A-Z]\w*)\.(where|find_by!?|find_each|find_or_create_by!?|find|all|first|last|pluck|count|exists\?|order|includes|joins|create!?|insert_all!?|upsert_all|update_all|delete_all|destroy_all)(?:\W|$)",
    )
});
static EF_USE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r"\b\w*(?:[Cc]ontext|[Dd]b)\.([A-Z]\w*)(\.(?:Add|AddRange|AddAsync|AddRangeAsync|Remove|RemoveRange|Update|UpdateRange|Attach|ExecuteDelete|ExecuteDeleteAsync|ExecuteUpdate|ExecuteUpdateAsync)\b)?",
    )
});

fn orm_uses(lang: Lang, text: &str, lines: &Lines, out: &mut Vec<TableUse>) {
    let mut push = |name: &str, write: bool, at: usize| {
        out.push(TableUse {
            name: name.to_owned(),
            write,
            orm: true,
            from: None,
            line: lines.at(at),
        });
    };
    let at = |c: &regex::Captures<'_>| c.get(0).map_or(0, |m| m.start());
    let js = matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx);
    if js && let Some(r) = PRISMA_CALL_RE.as_ref() {
        for c in r.captures_iter(text) {
            let m = &c[2];
            let write = !(m.starts_with("find") || matches!(m, "count" | "aggregate" | "groupBy"));
            push(&c[1], write, at(&c));
        }
    }
    if (js || lang == Lang::Php)
        && let Some(r) = QUERY_BUILDER_RE.as_ref()
    {
        for c in r.captures_iter(text) {
            push(&c[2], c[1].ends_with("into"), at(&c));
        }
    }
    if lang == Lang::Python {
        if let Some(r) = DJANGO_CALL_RE.as_ref() {
            for c in r.captures_iter(text) {
                let write = matches!(
                    &c[2],
                    "create"
                        | "bulk_create"
                        | "get_or_create"
                        | "update_or_create"
                        | "update"
                        | "delete"
                        | "bulk_update"
                );
                push(&c[1], write, at(&c));
            }
        }
        if let Some(r) = SQLA_RE.as_ref() {
            for c in r.captures_iter(text) {
                let write = !matches!(&c[1], ".query" | "select");
                push(&c[2], write, at(&c));
            }
        }
    }
    if lang == Lang::Ruby
        && let Some(r) = AR_RE.as_ref()
    {
        for c in r.captures_iter(text) {
            let m = &c[2];
            let write = m.starts_with("create")
                || m.starts_with("insert")
                || m.starts_with("upsert")
                || m.starts_with("update")
                || m.starts_with("delete")
                || m.starts_with("destroy")
                || m.starts_with("find_or_create");
            push(&c[1], write, at(&c));
        }
    }
    if lang == Lang::CSharp
        && let Some(r) = EF_USE_RE.as_ref()
    {
        for c in r.captures_iter(text) {
            push(&c[1], c.get(2).is_some(), at(&c));
        }
    }
}

fn dedupe(mut uses: Vec<TableUse>) -> Vec<TableUse> {
    let mut seen = std::collections::HashSet::new();
    uses.retain(|u| seen.insert((table_key(&u.name), u.write, u.line)));
    uses.truncate(MAX_USES);
    uses
}

// ---------------------------------------------------------------------------------------------
// Facts entry points (called from `parse`)
// ---------------------------------------------------------------------------------------------

/// A `.sql` file: DDL plus the statements that read or write tables.
pub(crate) fn sql_file(text: &str) -> (Vec<TableDef>, Vec<TableUse>) {
    let clean = strip_sql_comments(text);
    let lines = Lines::new(&clean);
    let tables = ddl(&clean, &lines);
    let mut uses = Vec::new();
    sql_uses(&clean, 0, &lines, &mut uses);
    (tables, dedupe(uses))
}

/// A `.prisma` file.
pub(crate) fn prisma_file(text: &str) -> Vec<TableDef> {
    prisma(text)
}

/// Tables defined in a code file (DDL in strings, Rails, Django, EF, Knex, Laravel).
pub(crate) fn code_tables(lang: Lang, text: &str) -> Vec<TableDef> {
    let lines = Lines::new(text);
    let mut out = Vec::new();
    if text.contains("TABLE") || text.contains("table") || text.contains("Table") {
        out.extend(ddl(text, &lines));
    }
    match lang {
        Lang::Ruby if text.contains("create_table") || text.contains("add_foreign_key") => {
            out.extend(rails(text));
        }
        Lang::Python if text.contains("Model") => out.extend(django(text)),
        _ => {}
    }
    out.extend(named_tables(lang, text, &lines));
    out.truncate(MAX_TABLES);
    out
}

/// Tables read or written by a code file (`from` is filled in by the caller).
pub(crate) fn code_uses(lang: Lang, text: &str) -> Vec<TableUse> {
    let lines = Lines::new(text);
    let mut out = Vec::new();
    literal_uses(text, &lines, &mut out);
    orm_uses(lang, text, &lines, &mut out);
    dedupe(out)
}

// ---------------------------------------------------------------------------------------------
// Live snapshots
// ---------------------------------------------------------------------------------------------

/// `<root>/.kor/graph/db/`.
pub fn snapshot_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("graph").join("db")
}

/// Connection names as `connections.toml` allows them (`a-z`, `0-9`, `-`): safe as file names.
pub fn valid_connection(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Write `<root>/.kor/graph/db/<connection>.json` (atomically). Returns the path.
pub fn save_snapshot(root: &Path, snap: &DbSnapshot) -> Result<PathBuf> {
    if !valid_connection(&snap.connection) {
        return Err(Error::Connection(snap.connection.clone()));
    }
    let dir = snapshot_dir(root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", snap.connection));
    let tmp = dir.join(format!("{}.json.tmp", snap.connection));
    std::fs::write(&tmp, serde_json::to_vec_pretty(snap)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Every readable snapshot of the project, by connection name.
pub fn load_snapshots(root: &Path) -> Vec<DbSnapshot> {
    let Ok(rd) = std::fs::read_dir(snapshot_dir(root)) else {
        return Vec::new();
    };
    let mut out: Vec<DbSnapshot> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<DbSnapshot>(&b).ok())
        .filter(|s| valid_connection(&s.connection))
        .collect();
    out.sort_by(|a, b| a.connection.cmp(&b.connection));
    out
}

// ---------------------------------------------------------------------------------------------
// Graph layer
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Info {
    name: String,
    file: Option<String>,
    span: Option<(u32, u32)>,
    columns: Vec<String>,
    live: bool,
    refs: Vec<String>,
    models: Vec<String>,
}

fn push_columns(into: &mut Vec<String>, cols: &[String]) {
    for c in cols {
        let name = c.split_whitespace().next().unwrap_or(c);
        if into.len() < MAX_COLUMNS
            && !into
                .iter()
                .any(|x| x.split_whitespace().next() == Some(name))
        {
            into.push(c.clone());
        }
    }
}

/// Table nodes, `References` edges, and `ReadsTable` / `WritesTable` edges from code.
/// `ids[path]` are the file's definition ids (to attribute a use to its enclosing symbol).
/// Uses only link to known tables: literal SQL naming one exactly is `Extracted`, a loose
/// (singular / model-name) match `Inferred{0.6}`, an ORM guess `Inferred{0.7}`.
#[allow(clippy::too_many_lines)]
pub(crate) fn layer(
    g: &mut Derived,
    files: &BTreeMap<String, FileFacts>,
    ids: &HashMap<&str, Vec<NodeId>>,
    live: &[DbSnapshot],
) {
    let mut tables: BTreeMap<String, Info> = BTreeMap::new();
    for snap in live {
        for t in &snap.tables {
            let key = table_key(&t.name);
            if key.is_empty() {
                continue;
            }
            let info = tables.entry(key).or_default();
            if info.name.is_empty() {
                unquote(t.name.rsplit('.').next().unwrap_or(&t.name)).clone_into(&mut info.name);
            }
            push_columns(&mut info.columns, &t.columns);
            info.live = true;
            info.refs.extend(t.references.iter().cloned());
        }
    }
    for (path, f) in files {
        for t in &f.tables {
            let name = match (&t.model, t.name.is_empty()) {
                (Some(m), true) => django_table(path, m),
                _ => t.name.clone(),
            };
            let key = table_key(&name);
            if key.is_empty() {
                continue;
            }
            let info = tables.entry(key).or_default();
            if info.name.is_empty() {
                unquote(name.rsplit('.').next().unwrap_or(&name)).clone_into(&mut info.name);
            }
            // The first source with columns (else the first at all) is the table's location.
            if info.file.is_none()
                || (info.span.is_some_and(|s| s.0 == s.1) && !t.columns.is_empty())
            {
                info.file = Some(path.clone());
                info.span = Some((t.start, t.end.max(t.start)));
            }
            if !info.live {
                push_columns(&mut info.columns, &t.columns);
            }
            info.refs.extend(t.refs.iter().cloned());
            info.models.extend(t.model.iter().cloned());
        }
    }
    if tables.is_empty() {
        return;
    }
    let mut exact: HashMap<String, NodeId> = HashMap::new();
    let mut loose: HashMap<String, NodeId> = HashMap::new();
    for (key, info) in &tables {
        let id = format!("{TABLE_PREFIX}{key}");
        g.node(
            &id,
            NodeKind::Table,
            &info.name,
            info.file.as_deref(),
            info.span,
            &info.columns.join("\n"),
        );
        if let Some(f) = &info.file {
            g.edge(f, &id, EdgeKind::Contains, Confidence::Extracted);
        }
        loose.entry(alias_key(key)).or_insert_with(|| id.clone());
        for m in &info.models {
            loose
                .entry(alias_key(&m.to_lowercase()))
                .or_insert_with(|| id.clone());
        }
        exact.insert(key.clone(), id);
    }
    let lookup = |raw: &str| -> Option<(&NodeId, bool)> {
        let k = table_key(raw);
        if let Some(id) = exact.get(&k) {
            return Some((id, true));
        }
        loose.get(&alias_key(&k)).map(|id| (id, false))
    };
    for (key, info) in &tables {
        let id = format!("{TABLE_PREFIX}{key}");
        for r in &info.refs {
            if let Some((target, is_exact)) = lookup(r) {
                let conf = if is_exact {
                    Confidence::Extracted
                } else {
                    Confidence::Inferred { score: 0.8 }
                };
                g.edge(&id, target, EdgeKind::References, conf);
            }
        }
    }
    for (path, f) in files {
        let fids = ids.get(path.as_str());
        for u in &f.table_uses {
            let Some((target, is_exact)) = lookup(&u.name) else {
                continue;
            };
            let conf = match (u.orm, is_exact) {
                (false, true) => Confidence::Extracted,
                (false, false) => Confidence::Inferred { score: 0.6 },
                (true, _) => Confidence::Inferred { score: 0.7 },
            };
            let src = u
                .from
                .and_then(|i| fids.and_then(|v| v.get(i as usize)))
                .map_or(path.as_str(), String::as_str);
            let kind = if u.write {
                EdgeKind::WritesTable
            } else {
                EdgeKind::ReadsTable
            };
            g.edge(src, target, kind, conf);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------------------

impl Graph {
    /// Live database snapshots merged into the graph.
    pub fn db_snapshots(&self) -> &[DbSnapshot] {
        &self.db
    }

    /// Re-read `.kor/graph/db/*.json`; recomputes only when a snapshot changed.
    pub fn refresh_db(&mut self) -> Result<crate::UpdateStats> {
        let snaps = load_snapshots(&self.root);
        if snaps == self.db {
            return Ok(crate::UpdateStats::default());
        }
        self.db = snaps;
        self.recompute()
    }

    /// Every table node, by name.
    pub fn tables(&self) -> Vec<TableInfo> {
        let mut out: Vec<TableInfo> = self
            .nodes
            .values()
            .filter(|n| n.kind == NodeKind::Table)
            .filter_map(|n| self.table_info(&n.id))
            .collect();
        out.sort_by(|a, b| a.node.name.cmp(&b.node.name));
        out
    }

    /// One table by name (`users`, `public.users`) or id (`table:users`).
    pub fn table(&self, name: &str) -> Option<TableInfo> {
        let id = if name.starts_with(TABLE_PREFIX) {
            name.to_owned()
        } else {
            format!("{TABLE_PREFIX}{}", table_key(name))
        };
        if self.nodes.contains_key(&id) {
            return self.table_info(&id);
        }
        let want = alias_key(&table_key(name));
        let n = self
            .nodes
            .values()
            .find(|n| n.kind == NodeKind::Table && alias_key(&table_key(&n.name)) == want)?;
        self.table_info(&n.id)
    }

    fn table_info(&self, id: &str) -> Option<TableInfo> {
        let node = self.nodes.get(id)?;
        let ix = *self.ix.get(id)?;
        let mut info = TableInfo {
            node: NodeRef::from(node),
            columns: node
                .text
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect(),
            references: Vec::new(),
            referenced_by: Vec::new(),
            readers: Vec::new(),
            writers: Vec::new(),
            live: self
                .db
                .iter()
                .filter(|s| {
                    s.tables
                        .iter()
                        .any(|t| format!("{TABLE_PREFIX}{}", table_key(&t.name)) == id)
                })
                .map(|s| s.connection.clone())
                .collect(),
            indexes: Vec::new(),
            slow_queries: Vec::new(),
        };
        let key = id.trim_start_matches(TABLE_PREFIX);
        for s in &self.db {
            for t in s.tables.iter().filter(|t| table_key(&t.name) == key) {
                info.indexes.extend(t.indexes.iter().cloned());
            }
            info.slow_queries.extend(
                s.slow_queries
                    .iter()
                    .filter(|q| mentions_table(&q.query, key))
                    .cloned(),
            );
        }
        for e in self.pg.edges_directed(ix, Direction::Outgoing) {
            if let Some(edge) = self.edge_list.get(*e.weight())
                && edge.kind == EdgeKind::References
                && let Some(n) = self.nodes.get(&self.pg[e.target()])
            {
                info.references.push(NodeRef::from(n));
            }
        }
        for e in self.pg.edges_directed(ix, Direction::Incoming) {
            let (Some(edge), Some(n)) = (
                self.edge_list.get(*e.weight()),
                self.nodes.get(&self.pg[e.source()]),
            ) else {
                continue;
            };
            let link = Link {
                node: NodeRef::from(n),
                kind: edge.kind,
                confidence: edge.confidence,
            };
            match edge.kind {
                EdgeKind::References => info.referenced_by.push(link.node),
                EdgeKind::ReadsTable => info.readers.push(link),
                EdgeKind::WritesTable => info.writers.push(link),
                _ => {}
            }
        }
        for v in [&mut info.readers, &mut info.writers] {
            v.sort_by(|a, b| {
                b.confidence
                    .score()
                    .total_cmp(&a.confidence.score())
                    .then(a.node.id.cmp(&b.node.id))
            });
        }
        info.references.sort_by(|a, b| a.id.cmp(&b.id));
        info.referenced_by.sort_by(|a, b| a.id.cmp(&b.id));
        Some(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{FileKind, classify, parse};

    fn facts(rel: &str, text: &str) -> FileFacts {
        let (kind, lang) = classify(rel).expect("graph input");
        parse(kind, lang, text)
    }

    #[test]
    fn sql_ddl_and_statements() {
        let sql = "-- users\nCREATE TABLE IF NOT EXISTS public.\"users\" (\n  id serial PRIMARY KEY,\n  email varchar(255) NOT NULL UNIQUE,\n  price numeric(10, 2) DEFAULT 0\n);\nCREATE TABLE posts (\n  id int,\n  user_id int REFERENCES users(id) ON DELETE CASCADE,\n  CONSTRAINT fk FOREIGN KEY (id) REFERENCES `authors`(id)\n);\nALTER TABLE posts ADD COLUMN title text NOT NULL;\nALTER TABLE ONLY comments ADD CONSTRAINT c FOREIGN KEY (post_id) REFERENCES posts (id);\nINSERT INTO posts (id) SELECT id FROM users;\n";
        let f = facts("db/schema.sql", sql);
        assert_eq!(f.kind, FileKind::Sql);
        let users = &f.tables[0];
        assert_eq!(users.name, "public.\"users\"");
        assert_eq!(
            users.columns,
            vec!["id serial", "email varchar(255)", "price numeric(10, 2)"]
        );
        assert_eq!((users.start, users.end), (2, 6));
        let posts = &f.tables[1];
        assert_eq!(posts.columns, vec!["id int", "user_id int"]);
        assert_eq!(posts.refs, vec!["users", "`authors`"]);
        assert!(
            f.tables
                .iter()
                .any(|t| t.name == "posts" && t.columns == vec!["title text"])
        );
        assert!(
            f.tables
                .iter()
                .any(|t| t.name == "comments" && t.refs == vec!["posts"])
        );
        assert!(
            f.table_uses
                .iter()
                .any(|u| u.name == "posts" && u.write && u.line == 14)
        );
        assert!(f.table_uses.iter().any(|u| u.name == "users" && !u.write));
        assert_eq!(table_key("public.\"Users\""), "users");
        assert_eq!(alias_key("user_profiles"), "userprofile");
        assert_eq!(alias_key("categories"), "category");
        assert_eq!(django_table("shop/models.py", "Order"), "shop_order");
        assert_eq!(django_table("shop/models/order.py", "Order"), "shop_order");
    }

    #[test]
    fn prisma_rails_django_ef() {
        let p = facts(
            "prisma/schema.prisma",
            "model User {\n  id    Int    @id\n  email String @unique\n  posts Post[]\n}\n\nmodel Post {\n  id       Int  @id\n  author   User @relation(fields: [authorId], references: [id])\n  authorId Int\n  @@map(\"blog_posts\")\n}\n",
        );
        assert_eq!(p.tables[0].columns, vec!["id Int", "email String"]);
        assert_eq!(p.tables[1].name, "blog_posts");
        assert_eq!(p.tables[1].refs, vec!["User"]);
        assert_eq!(p.tables[1].columns, vec!["id Int", "authorId Int"]);

        let rb = facts(
            "db/schema.rb",
            "ActiveRecord::Schema.define do\n  create_table \"orders\", force: :cascade do |t|\n    t.string \"code\"\n    t.references :customer, foreign_key: true\n    t.timestamps\n  end\n  add_foreign_key \"orders\", \"shops\"\nend\n",
        );
        let o = &rb.tables[0];
        assert_eq!(o.columns, vec!["code string", "customer_id references"]);
        assert_eq!(o.refs, vec!["customer"]);
        assert_eq!((o.start, o.end), (2, 6));
        assert!(
            rb.tables
                .iter()
                .any(|t| t.name == "orders" && t.refs == vec!["shops"])
        );

        let py = facts(
            "shop/models.py",
            "from django.db import models\n\nclass Base(models.Model):\n    class Meta:\n        abstract = True\n\nclass Order(models.Model):\n    code = models.CharField(max_length=9)\n    customer = models.ForeignKey('auth.User', on_delete=models.CASCADE)\n\n    class Meta:\n        db_table = \"orders\"\n\ndef total():\n    return Order.objects.filter(code='x').count()\n",
        );
        assert_eq!(py.tables.len(), 1, "{:?}", py.tables);
        assert_eq!(py.tables[0].name, "orders");
        assert_eq!(py.tables[0].refs, vec!["auth.User"]);
        assert!(
            py.table_uses
                .iter()
                .any(|u| u.name == "Order" && u.orm && !u.write && u.from.is_some())
        );

        let cs = facts(
            "Data/AppDb.cs",
            "class AppDb : DbContext { public DbSet<Customer> Customers { get; set; } }\nclass S { void Go(AppDb _db) { _db.Customers.Add(new Customer()); var n = _db.Customers.Count(); } }\n",
        );
        assert_eq!(cs.tables[0].name, "Customers");
        assert!(
            cs.table_uses
                .iter()
                .any(|u| u.name == "Customers" && u.write)
        );
        assert!(
            cs.table_uses
                .iter()
                .any(|u| u.name == "Customers" && !u.write)
        );
    }

    #[test]
    fn sql_literals_and_orm_calls_in_code() {
        let ts = facts(
            "src/repo.ts",
            "export async function list(db) {\n  return db.query(`SELECT u.id FROM users u\n    JOIN orders o ON o.user_id = u.id`);\n}\nexport function save(db) {\n  db.run(\"INSERT INTO audit_log (x) VALUES (1)\");\n  return prisma.order.findMany({});\n}\nconst msg = 'select a color';\n",
        );
        let names: Vec<(&str, bool, bool)> = ts
            .table_uses
            .iter()
            .map(|u| (u.name.as_str(), u.write, u.orm))
            .collect();
        assert!(names.contains(&("users", false, false)), "{names:?}");
        assert!(names.contains(&("orders", false, false)));
        assert!(names.contains(&("audit_log", true, false)));
        assert!(names.contains(&("order", false, true)));
        assert!(ts.table_uses.iter().all(|u| u.from.is_some()));
        let py = facts(
            "app/q.py",
            "def f(session):\n    session.query(Invoice).all()\n    cur.execute(\"\"\"\n        UPDATE invoices SET paid = 1\n    \"\"\")\n",
        );
        assert!(py.table_uses.iter().any(|u| u.name == "Invoice" && u.orm));
        assert!(
            py.table_uses
                .iter()
                .any(|u| u.name == "invoices" && u.write && u.line == 4)
        );
        let rb = facts(
            "app/x.rb",
            "def go\n  User.where(active: true)\n  User.create!(name: 'a')\nend\n",
        );
        assert!(rb.table_uses.iter().any(|u| u.name == "User" && !u.write));
        assert!(rb.table_uses.iter().any(|u| u.name == "User" && u.write));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn graph_layer_tables_fks_and_code_edges() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let w = |p: &str, s: &str| -> Result<()> {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap_or(dir.path()))?;
            std::fs::write(path, s)?;
            Ok(())
        };
        w(
            "migrations/001.sql",
            "CREATE TABLE users (id int PRIMARY KEY, email text);\nCREATE TABLE orders (id int, user_id int REFERENCES users(id));\n",
        )?;
        w(
            "src/repo.py",
            "def load(cur):\n    cur.execute(\"SELECT id FROM orders WHERE user_id = 1\")\n\ndef add(cur):\n    cur.execute('INSERT INTO users (email) VALUES (1)')\n\ndef guess():\n    return User.objects.get(id=1)\n",
        )?;
        let mut g = Graph::open(dir.path())?;
        g.update()?;
        let users = g.node("table:users").expect("users table");
        assert_eq!(users.kind, NodeKind::Table);
        assert_eq!(users.text, "id int\nemail text");
        let edge = |g: &Graph, s: &str, t: &str, k: EdgeKind| {
            g.edges()
                .find(|e| e.source == s && e.target == t && e.kind == k)
                .map(|e| e.confidence)
        };
        assert_eq!(
            edge(&g, "table:orders", "table:users", EdgeKind::References),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            edge(&g, "src/repo.py#load", "table:orders", EdgeKind::ReadsTable),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            edge(&g, "src/repo.py#add", "table:users", EdgeKind::WritesTable),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            edge(&g, "src/repo.py#guess", "table:users", EdgeKind::ReadsTable),
            Some(Confidence::Inferred { score: 0.7 })
        );
        // Queries: show, explain, impact.
        let t = g.table("users").expect("table info");
        assert_eq!(t.columns, vec!["id int", "email text"]);
        assert_eq!(t.referenced_by[0].id, "table:orders");
        assert!(t.writers.iter().any(|l| l.node.id == "src/repo.py#add"));
        assert!(t.readers.iter().any(|l| l.node.id == "src/repo.py#guess"));
        assert_eq!(g.tables().len(), 2);
        let ex = g.explain("users").expect("explain");
        assert_eq!(ex.node.id, "table:users");
        assert!(ex.source.is_some_and(|s| s.contains("CREATE TABLE users")));
        let imp = g.impact("table:users", 2);
        assert!(imp.affected.iter().any(|a| a.node.id == "src/repo.py#add"));
        assert!(imp.affected.iter().any(|a| a.node.id == "src/repo.py#load"));

        // A live snapshot adds a table and replaces columns; it persists across reopen.
        let snap = DbSnapshot {
            connection: "main".into(),
            kind: "sqlite".into(),
            taken_at: 1,
            tables: vec![
                LiveTable {
                    name: "users".into(),
                    columns: vec!["id INTEGER".into(), "email TEXT".into(), "name TEXT".into()],
                    indexes: vec!["users_email (email) unique".into()],
                    ..LiveTable::default()
                },
                LiveTable {
                    name: "sessions".into(),
                    columns: vec!["id INTEGER".into()],
                    references: vec!["users".into()],
                    ..LiveTable::default()
                },
            ],
            slow_queries: vec![SlowQuery {
                query: "SELECT * FROM users WHERE name = $1".into(),
                calls: 3,
                mean_us: 9000,
                total_us: 27000,
            }],
        };
        save_snapshot(dir.path(), &snap)?;
        let s = g.refresh_db()?;
        assert!(s.changed);
        assert_eq!(g.table("users").map(|t| t.columns.len()), Some(3));
        assert_eq!(
            g.table("users").map(|t| t.live),
            Some(vec!["main".to_owned()])
        );
        let users = g.table("users").expect("users");
        assert_eq!(users.indexes, vec!["users_email (email) unique"]);
        assert_eq!(users.slow_queries.len(), 1);
        assert!(
            g.table("sessions")
                .is_some_and(|t| t.slow_queries.is_empty())
        );
        assert!(mentions_table("select 1 from public.users u", "users"));
        assert!(!mentions_table("select 1 from users_old", "users"));
        assert_eq!(
            edge(&g, "table:sessions", "table:users", EdgeKind::References),
            Some(Confidence::Extracted)
        );
        assert!(!g.refresh_db()?.changed);
        drop(g);
        let g = Graph::open(dir.path())?;
        assert!(g.node("table:sessions").is_some());
        assert_eq!(g.db_snapshots().len(), 1);
        // Bad connection names never become paths.
        let bad = DbSnapshot {
            connection: "../x".into(),
            ..DbSnapshot::default()
        };
        assert!(save_snapshot(dir.path(), &bad).is_err());
        Ok(())
    }
}
