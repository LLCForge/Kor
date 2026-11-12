//! Per-file fact extraction: tree-sitter for code, small line scanners for env/config/ADR files.
//! Facts are what the resolver needs; they are cached by content hash (see `store`).

use crate::lang::Lang;
use crate::model::NodeKind;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, QueryCursor};

/// Bump when extraction changes so cached facts are re-parsed.
pub const FACTS_VERSION: u32 = 4;
/// Files larger than this are skipped (generated code, bundles, data).
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TEXT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Code,
    Env,
    Config,
    Adr,
    /// `*.sql`: DDL and statements (database-schema layer).
    Sql,
    /// `*.prisma` schema.
    Prisma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Call,
    Extends,
    Implements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportForm {
    /// `use`, `import`, `#include`, `require`.
    Normal,
    /// Path relative to the importing file (`require_relative`, PHP `require`).
    Relative,
    /// Rust `mod x;`.
    ModDecl,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Def {
    pub name: String,
    pub kind: NodeKind,
    /// Index of the enclosing definition in `defs`.
    pub parent: Option<u32>,
    /// Owner type for methods outside a class body (Rust `impl X { .. }`).
    pub container: Option<String>,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ref {
    pub name: String,
    pub kind: RefKind,
    /// Enclosing definition index; `None` = file level.
    pub from: Option<u32>,
    /// Explicit source type name (Rust `impl Trait for Type`).
    pub source: Option<String>,
    pub line: u32,
    /// Called through a receiver (`x.foo()`), so the target type is unknown.
    #[serde(default)]
    pub method: bool,
    /// Path qualifier of `Type::f()` / `module::f()`.
    #[serde(default)]
    pub qualifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Import {
    pub spec: String,
    pub form: ImportForm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Located {
    pub text: String,
    pub from: Option<u32>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileFacts {
    pub version: u32,
    pub kind: FileKind,
    /// Language name for code files.
    pub lang: String,
    pub lines: u32,
    pub defs: Vec<Def>,
    pub refs: Vec<Ref>,
    pub imports: Vec<Import>,
    /// Environment variable reads (`text` = variable name).
    pub env_reads: Vec<Located>,
    /// Keys defined by a `.env*` file. Values are never read into the graph.
    pub env_keys: Vec<String>,
    /// `WHY:` / `NOTE:` comments.
    pub notes: Vec<Located>,
    /// ADR title.
    pub title: String,
    /// ADR backtick mentions (identifiers or paths).
    pub mentions: Vec<String>,
    /// Tables defined here (DDL, Prisma, Rails, Django, EF, Knex, Laravel).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<crate::db::TableDef>,
    /// Tables read or written here (SQL literals, ORM calls).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_uses: Vec<crate::db::TableUse>,
}

impl FileFacts {
    fn empty(kind: FileKind, lang: &str, text: &str) -> Self {
        Self {
            version: FACTS_VERSION,
            kind,
            lang: lang.to_owned(),
            lines: u32::try_from(text.lines().count()).unwrap_or(u32::MAX),
            defs: Vec::new(),
            refs: Vec::new(),
            imports: Vec::new(),
            env_reads: Vec::new(),
            env_keys: Vec::new(),
            notes: Vec::new(),
            title: String::new(),
            mentions: Vec::new(),
            tables: Vec::new(),
            table_uses: Vec::new(),
        }
    }
}

/// What a project-relative path is to the graph, if anything.
pub fn classify(rel: &str) -> Option<(FileKind, Option<Lang>)> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if name.starts_with(".env") {
        return Some((FileKind::Env, None));
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    match ext.as_str() {
        "toml" | "yaml" | "yml" => Some((FileKind::Config, None)),
        "sql" => Some((FileKind::Sql, None)),
        "prisma" => Some((FileKind::Prisma, None)),
        "md" => {
            let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
            (dir.ends_with("docs/decisions") || dir.ends_with("docs/adr"))
                .then_some((FileKind::Adr, None))
        }
        _ => Lang::from_ext(&ext).map(|l| (FileKind::Code, Some(l))),
    }
}

/// Extract facts from one file's text.
pub fn parse(kind: FileKind, lang: Option<Lang>, text: &str) -> FileFacts {
    match (kind, lang) {
        (FileKind::Code, Some(l)) => parse_code(l, text),
        (FileKind::Env, _) => parse_env(text),
        (FileKind::Adr, _) => parse_adr(text),
        (FileKind::Sql, _) => {
            let mut f = FileFacts::empty(FileKind::Sql, "sql", text);
            (f.tables, f.table_uses) = crate::db::sql_file(text);
            f
        }
        (FileKind::Prisma, _) => {
            let mut f = FileFacts::empty(FileKind::Prisma, "prisma", text);
            f.tables = crate::db::prisma_file(text);
            f
        }
        _ => parse_config(text),
    }
}

fn line_of(node: Node<'_>) -> u32 {
    u32::try_from(node.start_position().row + 1).unwrap_or(u32::MAX)
}

fn node_text<'a>(node: Node<'_>, src: &'a str) -> &'a str {
    src.get(node.byte_range()).unwrap_or("")
}

/// `foo::Bar<T>` / `a.b.C` / `\App\X` → `Bar` / `C` / `X`.
fn clean_type_name(s: &str) -> String {
    let s = s.split(['<', '(', '[', '{']).next().unwrap_or(s).trim();
    let s = s.rsplit(['.', ':', '\\']).next().unwrap_or(s);
    s.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_'))
        .to_owned()
}

fn clean_import(s: &str) -> String {
    let s = s.trim().trim_end_matches(';').trim();
    let s = s.strip_prefix("import").map_or(s, str::trim_start);
    let s = s.strip_prefix("static ").unwrap_or(s);
    let s = s.strip_prefix("using").map_or(s, str::trim_start);
    let s = s.strip_prefix("static ").unwrap_or(s);
    s.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '<' | '>' | ' '))
        .trim()
        .to_owned()
}

/// For C/C++ the tags query marks the `function_declarator`; the body lives on its parent.
fn widen_def(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() != "function_declarator" {
        return Some(node);
    }
    let mut cur = node.parent();
    while let Some(p) = cur {
        match p.kind() {
            "function_definition" => return Some(p),
            // Prototypes and field declarations are not definitions.
            "declaration" | "field_declaration" | "parameter_declaration" => return None,
            _ => cur = p.parent(),
        }
    }
    None
}

fn def_kind(capture: &str, node: Node<'_>) -> Option<NodeKind> {
    let kind = capture.strip_prefix("definition.")?;
    Some(match kind {
        "function" | "method" | "macro" => NodeKind::Function,
        "class" => NodeKind::Struct,
        "interface" => NodeKind::Trait,
        "type" => {
            let is_iface = node
                .child_by_field_name("type")
                .is_some_and(|t| t.kind() == "interface_type");
            if is_iface {
                NodeKind::Trait
            } else {
                NodeKind::Struct
            }
        }
        _ => return None,
    })
}

fn impl_owner(node: Node<'_>, src: &str) -> Option<String> {
    let mut cur = node.parent();
    for _ in 0..4 {
        let p = cur?;
        if p.kind() == "impl_item" {
            return p
                .child_by_field_name("type")
                .map(|t| clean_type_name(node_text(t, src)));
        }
        cur = p.parent();
    }
    None
}

struct RawDef {
    name: String,
    kind: NodeKind,
    start_byte: usize,
    end_byte: usize,
    start: u32,
    end: u32,
    container: Option<String>,
}

#[allow(clippy::too_many_lines)]
fn parse_code(lang: Lang, text: &str) -> FileFacts {
    let mut facts = FileFacts::empty(FileKind::Code, lang.name(), text);
    let mut parser = Parser::new();
    let (Some(query), Ok(())) = (lang.query(), parser.set_language(&lang.language())) else {
        return facts;
    };
    let Some(tree) = parser.parse(text, None) else {
        return facts;
    };
    let capture_names = query.capture_names();
    let mut defs_found: Vec<RawDef> = Vec::new();
    let mut refs_found: Vec<(Ref, usize)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), text.as_bytes());
    while let Some(m) = matches.next() {
        let mut name_node = None;
        let mut role: Option<(&str, Node<'_>)> = None;
        let mut impl_type = None;
        let mut qual = None;
        for c in m.captures {
            let cname = capture_names.get(c.index as usize).copied().unwrap_or("");
            match cname {
                "name" => name_node = Some(c.node),
                "impl_type" => impl_type = Some(clean_type_name(node_text(c.node, text))),
                "qual" => qual = Some(clean_type_name(node_text(c.node, text))),
                n if n.starts_with("definition.") || n.starts_with("reference.") => {
                    role = Some((n, c.node));
                }
                n @ ("import" | "import_rel" | "mod_decl" | "extends" | "implements" | "call") => {
                    role = Some((n, c.node));
                }
                _ => {}
            }
        }
        let Some((cname, node)) = role else { continue };
        match cname {
            "import" | "import_rel" | "mod_decl" => {
                let spec = clean_import(node_text(node, text));
                if !spec.is_empty() && spec.len() <= 300 {
                    let form = match cname {
                        "import_rel" => ImportForm::Relative,
                        "mod_decl" => ImportForm::ModDecl,
                        _ => ImportForm::Normal,
                    };
                    facts.imports.push(Import { spec, form });
                }
            }
            "extends" | "implements" | "call" => {
                let name = clean_type_name(node_text(node, text));
                let kind = match cname {
                    "extends" if lang == Lang::CSharp && is_csharp_interface(&name) => {
                        RefKind::Implements
                    }
                    "extends" => RefKind::Extends,
                    "implements" => RefKind::Implements,
                    _ => RefKind::Call,
                };
                let method = kind == RefKind::Call && is_method_call(node);
                let r = Ref {
                    name,
                    kind,
                    from: None,
                    source: impl_type.clone(),
                    line: line_of(node),
                    method,
                    qualifier: qual.clone(),
                };
                refs_found.push((r, node.start_byte()));
            }
            c if c.starts_with("definition.") => {
                let (Some(nn), Some(kind)) = (name_node, def_kind(c, node)) else {
                    continue;
                };
                let Some(def_node) = widen_def(node) else {
                    continue;
                };
                let name = node_text(nn, text).to_owned();
                if name.is_empty() || name.len() > 200 {
                    continue;
                }
                defs_found.push(RawDef {
                    container: if lang == Lang::Rust {
                        impl_owner(def_node, text)
                    } else {
                        None
                    },
                    name,
                    kind,
                    start_byte: def_node.start_byte(),
                    end_byte: def_node.end_byte(),
                    start: line_of(def_node),
                    end: u32::try_from(def_node.end_position().row + 1).unwrap_or(u32::MAX),
                });
            }
            "reference.call" | "reference.send" | "reference.class" => {
                // Macro invocations (`format!`, `vec!`) are not calls into project code.
                let Some(nn) = name_node.filter(|_| node.kind() != "macro_invocation") else {
                    continue;
                };
                let name = clean_type_name(node_text(nn, text));
                let r = Ref {
                    name,
                    kind: RefKind::Call,
                    from: None,
                    source: None,
                    line: line_of(nn),
                    method: is_method_call(nn),
                    qualifier: None,
                };
                refs_found.push((r, nn.start_byte()));
            }
            _ => {}
        }
    }
    // Dedupe (tags queries overlap, e.g. Rust methods match both function patterns) and order by position.
    defs_found.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then(b.end_byte.cmp(&a.end_byte))
    });
    defs_found.dedup_by(|a, b| a.start_byte == b.start_byte && a.name == b.name);
    let ranges: Vec<(usize, usize)> = defs_found
        .iter()
        .map(|d| (d.start_byte, d.end_byte))
        .collect();
    let innermost = |pos: usize, exclude: Option<usize>| -> Option<u32> {
        // Defs are sorted by start; the last one containing `pos` is the innermost.
        let mut best = None;
        for (i, (s, e)) in ranges.iter().enumerate() {
            if *s > pos {
                break;
            }
            if pos < *e && Some(i) != exclude {
                best = Some(i);
            }
        }
        best.and_then(|i| u32::try_from(i).ok())
    };
    for (i, d) in defs_found.iter().enumerate() {
        let parent =
            innermost(d.start_byte, Some(i)).filter(|p| ranges[*p as usize].1 >= d.end_byte);
        facts.defs.push(Def {
            name: d.name.clone(),
            kind: d.kind,
            parent,
            container: d.container.clone(),
            start: d.start,
            end: d.end,
        });
    }
    let mut seen = std::collections::HashSet::new();
    for (mut r, pos) in refs_found {
        if r.name.is_empty() || r.name.len() > 200 {
            continue;
        }
        r.from = innermost(pos, None);
        if seen.insert((r.name.clone(), r.kind, r.from, r.line, r.qualifier.clone())) {
            facts.refs.push(r);
        }
    }
    let line_from = |line: u32| -> Option<u32> {
        let mut best: Option<usize> = None;
        for (i, d) in facts.defs.iter().enumerate() {
            if d.start <= line
                && line <= d.end
                && best.is_none_or(|b| facts.defs[b].start <= d.start)
            {
                best = Some(i);
            }
        }
        best.and_then(|i| u32::try_from(i).ok())
    };
    let env_reads: Vec<Located> = scan_env_reads(text)
        .into_iter()
        .map(|(text, line)| Located {
            from: line_from(line),
            text,
            line,
        })
        .collect();
    let notes: Vec<Located> = scan_notes(text)
        .into_iter()
        .map(|(text, line)| Located {
            from: line_from(line),
            text,
            line,
        })
        .collect();
    let table_uses: Vec<crate::db::TableUse> = crate::db::code_uses(lang, text)
        .into_iter()
        .map(|mut u| {
            u.from = line_from(u.line);
            u
        })
        .collect();
    facts.env_reads = env_reads;
    facts.notes = notes;
    facts.table_uses = table_uses;
    facts.tables = crate::db::code_tables(lang, text);
    facts
}

/// True when the called name hangs off a receiver expression (`x.foo()`, `$x->foo()`, `x.foo`).
fn is_method_call(name: Node<'_>) -> bool {
    let Some(p) = name.parent() else { return false };
    // Go `pkg.Func()` (`selector_expression`) is package-qualified, not a method call; imports resolve it.
    match p.kind() {
        "field_expression"
        | "member_expression"
        | "attribute"
        | "member_access_expression"
        | "member_call_expression"
        | "nullsafe_member_call_expression" => true,
        "method_invocation" | "call" => p
            .child_by_field_name("object")
            .or_else(|| p.child_by_field_name("receiver"))
            .is_some(),
        _ => false,
    }
}

/// C# convention: `IFoo` names an interface.
fn is_csharp_interface(name: &str) -> bool {
    let mut c = name.chars();
    c.next() == Some('I') && c.next().is_some_and(char::is_uppercase)
}

static ENV_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"process\.env\.([A-Za-z_][A-Za-z0-9_]*)"#,
        r#"|process\.env\[\s*['"]([^'"]+)['"]"#,
        r#"|import\.meta\.env\.([A-Za-z_][A-Za-z0-9_]*)"#,
        r#"|env::var(?:_os)?\(\s*"([^"]+)""#,
        r#"|(?:option_)?env!\(\s*"([^"]+)""#,
        r#"|os\.environ(?:\.get)?\s*[\[\(]\s*['"]([^'"]+)['"]"#,
        r#"|os\.(?:getenv|Getenv|LookupEnv)\(\s*['"]([^'"]+)['"]"#,
        r#"|System\.getenv\(\s*"([^"]+)""#,
        r#"|Environment\.GetEnvironmentVariable\(\s*"([^"]+)""#,
        r#"|\bgetenv\(\s*['"]([^'"]+)['"]"#,
        r#"|\$_ENV\[\s*['"]([^'"]+)['"]"#,
        r#"|\bENV(?:\.fetch\(|\[)\s*['"]([^'"]+)['"]"#,
    ))
    .ok()
});

static NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?://+|#+|/\*+|\*|--|;+)\s*(WHY|NOTE)\s*:\s*(.+)").ok());

static CONFIG_ENV_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)").ok());

fn valid_env_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

fn truncate(s: &str) -> String {
    let s = s.trim().trim_end_matches("*/").trim();
    match s.char_indices().nth(MAX_TEXT) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn lines_u32(text: &str) -> impl Iterator<Item = (u32, &str)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (u32::try_from(i + 1).unwrap_or(u32::MAX), l))
}

fn scan_env_reads(text: &str) -> Vec<(String, u32)> {
    let Some(re) = ENV_RE.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (line, l) in lines_u32(text) {
        for c in re.captures_iter(l) {
            if let Some(name) = c.iter().skip(1).flatten().next().map(|m| m.as_str())
                && valid_env_name(name)
            {
                out.push((name.to_owned(), line));
            }
        }
    }
    out
}

fn scan_notes(text: &str) -> Vec<(String, u32)> {
    let Some(re) = NOTE_RE.as_ref() else {
        return Vec::new();
    };
    lines_u32(text)
        .filter_map(|(line, l)| {
            let c = re.captures(l)?;
            Some((format!("{}: {}", &c[1], truncate(&c[2])), line))
        })
        .collect()
}

fn parse_env(text: &str) -> FileFacts {
    let mut f = FileFacts::empty(FileKind::Env, "", text);
    for l in text.lines() {
        let l = l.trim();
        let l = l.strip_prefix("export ").unwrap_or(l);
        if l.starts_with('#') {
            continue;
        }
        // Only the key: the value after `=` is never kept.
        if let Some((k, _)) = l.split_once('=') {
            let k = k.trim();
            if valid_env_name(k) && !f.env_keys.iter().any(|x| x == k) {
                f.env_keys.push(k.to_owned());
            }
        }
    }
    f
}

fn parse_config(text: &str) -> FileFacts {
    let mut f = FileFacts::empty(FileKind::Config, "", text);
    if let Some(re) = CONFIG_ENV_RE.as_ref() {
        for (line, l) in lines_u32(text) {
            for c in re.captures_iter(l) {
                f.env_reads.push(Located {
                    text: c[1].to_owned(),
                    from: None,
                    line,
                });
            }
        }
    }
    f.notes = scan_notes(text)
        .into_iter()
        .map(|(text, line)| Located {
            text,
            from: None,
            line,
        })
        .collect();
    f
}

static MENTION_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"`([A-Za-z_][A-Za-z0-9_:./\-]{1,120})`").ok());

fn parse_adr(text: &str) -> FileFacts {
    let mut f = FileFacts::empty(FileKind::Adr, "", text);
    f.title = text
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(truncate)
        .unwrap_or_default();
    if let Some(re) = MENTION_RE.as_ref() {
        for c in re.captures_iter(text) {
            let m = c[1].trim_end_matches(['.', ':', '/']).to_owned();
            if !f.mentions.contains(&m) && f.mentions.len() < 100 {
                f.mentions.push(m);
            }
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(lang: Lang, src: &str) -> FileFacts {
        parse(FileKind::Code, Some(lang), src)
    }

    fn def_names(f: &FileFacts) -> Vec<&str> {
        f.defs.iter().map(|d| d.name.as_str()).collect()
    }

    fn has_ref(f: &FileFacts, name: &str, kind: RefKind) -> bool {
        f.refs.iter().any(|r| r.name == name && r.kind == kind)
    }

    #[test]
    fn rust_defs_calls_imports_impls() {
        let src = "use crate::store::Db;\nmod util;\n\npub trait Shape { fn area(&self) -> f64; }\npub struct Sq;\nimpl Shape for Sq {\n    fn area(&self) -> f64 { helper(2.0) }\n}\n// WHY: kept tiny\nfn helper(x: f64) -> f64 { let _k = std::env::var(\"API_URL\"); x }\n";
        let f = code(Lang::Rust, src);
        assert!(def_names(&f).contains(&"Shape"));
        assert!(def_names(&f).contains(&"Sq"));
        assert!(def_names(&f).contains(&"helper"));
        let area = f
            .defs
            .iter()
            .find(|d| d.name == "area" && d.container.is_some())
            .map(|d| d.container.clone());
        assert_eq!(area, Some(Some("Sq".into())));
        let call = f
            .refs
            .iter()
            .find(|r| r.name == "helper")
            .and_then(|r| r.from)
            .map(|i| f.defs[i as usize].name.clone());
        assert_eq!(call.as_deref(), Some("area"));
        let imp = f.refs.iter().find(|r| r.kind == RefKind::Implements);
        assert_eq!(
            imp.map(|r| (r.name.as_str(), r.source.as_deref())),
            Some(("Shape", Some("Sq")))
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.spec == "crate::store::Db" && i.form == ImportForm::Normal)
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.spec == "util" && i.form == ImportForm::ModDecl)
        );
        assert_eq!(f.env_reads[0].text, "API_URL");
        assert_eq!(f.notes[0].text, "WHY: kept tiny");
    }

    #[test]
    fn python_and_typescript() {
        let py = "import os\nfrom .models import User\n\nclass Admin(User):\n    def run(self):\n        return check(os.environ['DB_URL'])\n\ndef check(x):\n    return x\n";
        let f = code(Lang::Python, py);
        assert_eq!(def_names(&f), vec!["Admin", "run", "check"]);
        assert!(has_ref(&f, "User", RefKind::Extends));
        assert!(has_ref(&f, "check", RefKind::Call));
        assert!(f.imports.iter().any(|i| i.spec == ".models"));
        assert_eq!(f.env_reads[0].text, "DB_URL");
        let ts = "import { db } from './db';\ninterface Repo { find(): void }\nexport class UserRepo extends Base implements Repo {\n  find() { return db.query(process.env.TABLE); }\n}\n";
        let f = code(Lang::TypeScript, ts);
        assert!(def_names(&f).contains(&"UserRepo"));
        assert!(def_names(&f).contains(&"Repo"));
        assert!(has_ref(&f, "Base", RefKind::Extends));
        assert!(has_ref(&f, "Repo", RefKind::Implements));
        assert!(has_ref(&f, "query", RefKind::Call));
        assert!(f.imports.iter().any(|i| i.spec == "./db"));
        assert_eq!(f.env_reads[0].text, "TABLE");
    }

    #[test]
    fn other_languages_extract_something() {
        let cases: [(Lang, &str, &str, &str); 9] = [
            (
                Lang::Go,
                "package a\nimport \"x/y\"\nfunc Run() { Help() }\nfunc Help() {}\n",
                "Run",
                "Help",
            ),
            (
                Lang::Java,
                "import a.b.C;\nclass A extends B { void run() { help(); } void help() {} }\n",
                "run",
                "help",
            ),
            (
                Lang::CSharp,
                "using A.B;\nclass A : IFoo { void Run() { Help(); } void Help() {} }\n",
                "Run",
                "Help",
            ),
            (
                Lang::C,
                "#include \"x.h\"\nint help(void) { return 1; }\nint run(void) { return help(); }\n",
                "run",
                "help",
            ),
            (
                Lang::Cpp,
                "#include \"x.h\"\nclass A : public B { };\nint help() { return 1; }\nint run() { return help(); }\n",
                "run",
                "help",
            ),
            (
                Lang::Php,
                "<?php\nuse App\\Models\\User;\nclass A extends B { function run() { help(); } }\nfunction help() {}\n",
                "run",
                "help",
            ),
            (
                Lang::Ruby,
                "require 'json'\nclass A < B\n  def run\n    help()\n  end\nend\ndef help\nend\n",
                "run",
                "help",
            ),
            (
                Lang::JavaScript,
                "const x = require('./x');\nclass A extends B { run() { help(); } }\nfunction help() {}\n",
                "run",
                "help",
            ),
            (
                Lang::Tsx,
                "import x from './x';\nexport function App() { return help(); }\nfunction help() { return null; }\n",
                "App",
                "help",
            ),
        ];
        for (lang, src, caller, callee) in cases {
            let f = code(lang, src);
            assert!(
                def_names(&f).contains(&caller),
                "{lang:?} defs {:?}",
                def_names(&f)
            );
            let r = f
                .refs
                .iter()
                .find(|r| r.name == callee && r.kind == RefKind::Call);
            let from = r
                .and_then(|r| r.from)
                .map(|i| f.defs[i as usize].name.as_str());
            assert_eq!(from, Some(caller), "{lang:?} refs {:?}", f.refs);
            assert!(!f.imports.is_empty(), "{lang:?} imports");
        }
        let cs = code(Lang::CSharp, "class A : IFoo, Base { }");
        assert!(has_ref(&cs, "IFoo", RefKind::Implements));
        assert!(has_ref(&cs, "Base", RefKind::Extends));
    }

    /// Every language beyond the first twelve: its definitions, and calls and imports where the
    /// query captures them (`None`: the grammar's query has none).
    #[test]
    #[allow(clippy::too_many_lines)]
    fn added_languages_find_definitions_calls_and_imports() {
        type Case = (Lang, &'static str, &'static str, Option<&'static str>, bool);
        let cases: &[Case] = &[
            (
                Lang::Bash,
                "source ./lib.sh\nbuild() { helper all; }\n",
                "build",
                Some("helper"),
                true,
            ),
            (
                Lang::Lua,
                "local m = require('m')\nfunction run(x) return helper(x) end\n",
                "run",
                Some("helper"),
                false,
            ),
            (
                Lang::Scala,
                "import a.b.C\nclass Foo extends Bar { def run(x: Int): Int = helper(x) }\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Swift,
                "import Foundation\nclass Foo { func run() { helper() } }\n",
                "run",
                Some("helper"),
                false,
            ),
            (
                Lang::Kotlin,
                "import a.b.C\nclass Foo : Bar() { fun run() { helper() } }\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Haskell,
                "module M where\nimport Data.List\ndata Shape = Circle\nrun x = helper x\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Elixir,
                "defmodule M do\n  def run(x), do: helper(x)\nend\n",
                "run",
                Some("helper"),
                false,
            ),
            (Lang::Ocaml, "let run x = helper x\n", "run", None, false),
            (
                Lang::Zig,
                "const lib = @import(\"lib.zig\");\npub fn run() void { helper(); }\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Julia,
                "using LinearAlgebra\nfunction run(x)\n helper(x)\nend\n",
                "run",
                None,
                true,
            ),
            (
                Lang::R,
                "run <- function(x) { helper(x) }\n",
                "run",
                Some("helper"),
                false,
            ),
            (
                Lang::Dart,
                "import 'a.dart';\nclass Foo { void run() { helper(); } }\n",
                "run",
                None,
                false,
            ),
            (
                Lang::Erlang,
                "-include(\"a.hrl\").\nrun(X) -> helper(X).\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Elm,
                "module M exposing (run)\nrun x = helper x\n",
                "run",
                None,
                false,
            ),
            (
                Lang::Nix,
                "let\n  lib = import ./lib.nix;\n  build = x: helper x;\nin build\n",
                "build",
                Some("helper"),
                true,
            ),
            (
                Lang::Hcl,
                "resource \"aws_s3_bucket\" \"logs\" {\n}\nmodule \"vpc\" {\n  source = \"./vpc\"\n}\n",
                "logs",
                None,
                true,
            ),
            (
                Lang::Solidity,
                "contract Vault { function run() public { helper(); } }\n",
                "Vault",
                Some("helper"),
                false,
            ),
            (
                Lang::Fortran,
                "subroutine run(x)\n call helper(x)\nend subroutine\n",
                "run",
                Some("helper"),
                false,
            ),
            (
                Lang::Gleam,
                "pub fn run(x) { helper(x) }\n",
                "run",
                Some("helper"),
                false,
            ),
            (
                Lang::Groovy,
                "import a.b.C\nclass Foo extends Bar { def run() { } }\n",
                "run",
                None,
                true,
            ),
            (
                Lang::ObjC,
                "#import \"Foo.h\"\n@implementation Foo\n- (void)run { [self helper]; }\n@end\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::PowerShell,
                ". ./lib.ps1\nfunction Get-Thing { helper }\n",
                "Get-Thing",
                Some("helper"),
                true,
            ),
            (Lang::FSharp, "let run x = helper x\n", "run", None, false),
            (
                Lang::D,
                "import std.stdio;\nclass Foo : Bar { void run() { helper(); } }\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Verilog,
                "module counter(input clk);\nendmodule\n",
                "counter",
                None,
                false,
            ),
            (
                Lang::Cuda,
                "#include \"a.h\"\n__global__ void kern(int x) { helper(x); }\n",
                "kern",
                Some("helper"),
                true,
            ),
            (
                Lang::Ada,
                "with Ada.Text_IO;\npackage body P is\n procedure Run is\n begin\n  Helper;\n end Run;\nend P;\n",
                "Run",
                Some("Helper"),
                true,
            ),
            (
                Lang::Pascal,
                "unit U;\ninterface\nuses SysUtils;\nimplementation\nfunction F: Integer; begin Result := 1; end;\nend.\n",
                "F",
                None,
                true,
            ),
            (
                Lang::GdScript,
                "extends Node\nconst Lib = preload(\"res://lib.gd\")\nfunc run():\n\thelper()\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Odin,
                "package main\nimport \"core:fmt\"\nrun :: proc() { helper() }\n",
                "run",
                Some("helper"),
                true,
            ),
            (
                Lang::Sql,
                "CREATE TABLE users (id INT);\n",
                "users",
                None,
                false,
            ),
            (
                Lang::Proto,
                "import \"a.proto\";\nmessage User { string id = 1; }\n",
                "User",
                None,
                true,
            ),
            (
                Lang::Vhdl,
                "library ieee;\nuse ieee.std_logic_1164.all;\nentity counter is end entity;\n",
                "counter",
                None,
                true,
            ),
            (
                Lang::CommonLisp,
                "(defun run (x) (helper x))\n",
                "run",
                None,
                false,
            ),
        ];
        let mut missing = Vec::new();
        for (lang, src, def, call, import) in cases {
            let f = code(*lang, src);
            if !def_names(&f).contains(def) {
                missing.push(format!("{lang:?} def {def}: {:?}", def_names(&f)));
            }
            if let Some(c) = call.filter(|c| !has_ref(&f, c, RefKind::Call)) {
                let refs: Vec<&str> = f.refs.iter().map(|r| r.name.as_str()).collect();
                missing.push(format!("{lang:?} call {c}: {refs:?}"));
            }
            if *import && f.imports.is_empty() {
                missing.push(format!("{lang:?} import"));
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
        assert_eq!(cases.len() + 12, crate::lang::ALL.len());
    }

    #[test]
    fn env_config_adr() {
        let f = parse(
            FileKind::Env,
            None,
            "# c\nexport API_KEY=sk-secret\nDB_URL = postgres://x\n=bad\n",
        );
        assert_eq!(f.env_keys, vec!["API_KEY", "DB_URL"]);
        assert!(
            !serde_json::to_string(&f)
                .unwrap_or_default()
                .contains("secret")
        );
        let c = parse(
            FileKind::Config,
            None,
            "url = \"${DB_URL}\"\n# NOTE: shared with ci\n",
        );
        assert_eq!(c.env_reads[0].text, "DB_URL");
        assert_eq!(c.notes.len(), 1);
        let a = parse(
            FileKind::Adr,
            None,
            "# 0001 Use SQLite\nWe chose `Store` and `src/store.rs`.\n",
        );
        assert_eq!(a.title, "0001 Use SQLite");
        assert_eq!(a.mentions, vec!["Store", "src/store.rs"]);
        assert_eq!(
            classify("docs/decisions/0001-x.md").map(|c| c.0),
            Some(FileKind::Adr)
        );
        assert_eq!(classify("README.md"), None);
        assert_eq!(classify("a/.env.local").map(|c| c.0), Some(FileKind::Env));
        assert_eq!(classify("src/a.rs").map(|c| c.1), Some(Some(Lang::Rust)));
    }
}
