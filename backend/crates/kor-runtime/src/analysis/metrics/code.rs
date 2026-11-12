//! Per-file code statistics from tree-sitter (the grammars `kor-graph` already ships): functions
//! with cyclomatic complexity and line ranges, size, comments, documented public items, naming
//! conventions, error-handling patterns, preferred constructs, and line windows for duplicate
//! detection. One pass over the syntax tree per file; nothing is cached (it is fast enough, and
//! the inputs change with every edit).
//!
//! Cyclomatic complexity = 1 + decision points (`if`/`elif`, loops, match arms / cases, `catch`,
//! ternaries, `&&`/`||`/`and`/`or`/`??`), the usual `McCabe` approximation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use kor_graph::lang::Lang;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

/// Files larger than this are skipped (generated or vendored code).
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Lines per duplicate-detection window.
pub const DUP_WINDOW: usize = 6;
/// Directories never scanned, even when not git-ignored.
const SKIP_DIRS: [&str; 9] = [
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".git",
    ".kor",
    "__pycache__",
    ".venv",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnStat {
    pub name: String,
    /// 1-based first and last line.
    pub line: u32,
    pub end: u32,
    pub complexity: u32,
    pub public: bool,
    pub documented: bool,
    /// `return`s inside an `if` (guard clauses).
    pub early_returns: u32,
}

impl FnStat {
    pub fn lines(&self) -> u32 {
        self.end.saturating_sub(self.line) + 1
    }
}

/// A name declared in the file, for naming-convention statistics and `kor style check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameUse {
    /// `function`, `type`, `variable`, `constant`.
    pub category: String,
    pub name: String,
    pub line: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStats {
    /// Project-relative, `/`-separated.
    pub path: String,
    pub lang: String,
    pub lines: u32,
    pub code_lines: u32,
    pub comment_lines: u32,
    pub blank_lines: u32,
    pub functions: Vec<FnStat>,
    pub types: u32,
    pub public_items: u32,
    pub documented_public: u32,
    /// Public items without a doc comment: (name, line), at most 200 per file.
    pub undocumented: Vec<(String, u32)>,
    pub names: Vec<NameUse>,
    /// Error-handling pattern → lines where it occurs.
    pub errors: BTreeMap<String, Vec<u32>>,
    /// Construct → count (`loop`, `iterator chain`, `match/switch`, `ternary`, `const`, …).
    pub constructs: BTreeMap<String, u32>,
    /// A test file by name, or a file with test markers in it.
    pub is_test: bool,
    pub has_tests: bool,
    /// (hash, first line, last line) of every window of [`DUP_WINDOW`] significant lines.
    #[serde(skip)]
    pub windows: Vec<(u64, u32, u32)>,
}

impl FileStats {
    pub fn complexity(&self) -> u32 {
        self.functions.iter().map(|f| f.complexity).sum()
    }

    pub fn max_complexity(&self) -> u32 {
        self.functions
            .iter()
            .map(|f| f.complexity)
            .max()
            .unwrap_or(0)
    }
}

const FN_KINDS: [&str; 10] = [
    "function_item",
    "function_definition",
    "function_declaration",
    "generator_function_declaration",
    "method_definition",
    "method_declaration",
    "constructor_declaration",
    "local_function_statement",
    "method",
    "singleton_method",
];
const ANON_FN_KINDS: [&str; 9] = [
    "arrow_function",
    "function_expression",
    "function",
    "func_literal",
    "closure_expression",
    "lambda",
    "lambda_expression",
    "anonymous_function",
    "anonymous_function_creation_expression",
];
const TYPE_KINDS: [&str; 16] = [
    "struct_item",
    "enum_item",
    "trait_item",
    "union_item",
    "type_item",
    "class_declaration",
    "class_definition",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
    "struct_declaration",
    "type_spec",
    "class",
    "class_specifier",
    "trait_declaration",
    "type_alias_declaration",
];
const DECISION_KINDS: [&str; 34] = [
    "if_statement",
    "if_expression",
    "if",
    "elif_clause",
    "else_if_clause",
    "unless",
    "if_modifier",
    "unless_modifier",
    "while_statement",
    "while_expression",
    "while",
    "while_modifier",
    "until",
    "for_statement",
    "for_expression",
    "for_in_statement",
    "foreach_statement",
    "enhanced_for_statement",
    "for",
    "loop_expression",
    "do_statement",
    "match_arm",
    "case_clause",
    "switch_case",
    "switch_section",
    "switch_block_statement_group",
    "when",
    "expression_case",
    "type_case",
    "case_statement",
    "catch_clause",
    "except_clause",
    "rescue",
    "conditional_expression",
];
const IF_KINDS: [&str; 6] = [
    "if_statement",
    "if_expression",
    "if",
    "unless",
    "if_modifier",
    "unless_modifier",
];
const LOOP_KINDS: [&str; 13] = [
    "for_statement",
    "for_expression",
    "for_in_statement",
    "foreach_statement",
    "enhanced_for_statement",
    "for",
    "loop_expression",
    "do_statement",
    "while_statement",
    "while_expression",
    "while",
    "until",
    "range_clause",
];
const MATCH_KINDS: [&str; 8] = [
    "match_expression",
    "match_statement",
    "switch_statement",
    "switch_expression",
    "expression_switch_statement",
    "type_switch_statement",
    "case",
    "match_block",
];
const ITER_METHODS: [&str; 14] = [
    "map",
    "filter",
    "fold",
    "reduce",
    "forEach",
    "flat_map",
    "filter_map",
    "flatMap",
    "any",
    "all",
    "find",
    "collect",
    "Select",
    "Where",
];
const CALL_KINDS: [&str; 7] = [
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "member_call_expression",
    "method_call_expression",
];

static TEST_MARKERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#\[(?:tokio::)?test\]|#\[cfg\(test\)\]|\bdef test_|@Test\b|\bdescribe\(|\bit\(|\btest\(|\bfunc Test[A-Z_]|\[Fact\]|\[Test\]|\[TestMethod\]|\bRSpec\.|< Minitest::Test",
    )
    .expect("valid regex")
});

/// A test file by its path (`tests/`, `__tests__/`, `*_test.*`, `test_*.py`, `*.spec.*`, …).
pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let java = Path::new(&p).extension().is_some_and(|e| e == "java");
    let cs = Path::new(&p).extension().is_some_and(|e| e == "cs");
    let name = p.rsplit('/').next().unwrap_or(&p);
    let stem = name.split('.').next().unwrap_or(name);
    p.starts_with("tests/")
        || p.starts_with("test/")
        || p.contains("/tests/")
        || p.contains("/test/")
        || p.contains("__tests__/")
        || p.contains("/spec/")
        || stem.ends_with("_test")
        || stem.ends_with("_spec")
        || stem.starts_with("test_")
        || stem.ends_with("test") && stem.len() > 4 && java
        || stem.ends_with("tests") && cs
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// Language of a path by extension.
pub fn lang_of(path: &str) -> Option<Lang> {
    let ext = path.rsplit_once('.')?.1;
    Lang::from_ext(ext)
}

/// Naming convention of an identifier; `None` for single lowercase words (fit every convention).
pub fn convention(name: &str) -> Option<&'static str> {
    let name = name.trim_start_matches(['_', '$']);
    let letters: Vec<char> = name.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return None;
    }
    let has_lower = letters.iter().any(|c| c.is_lowercase());
    let has_upper = letters.iter().any(|c| c.is_uppercase());
    let first_upper = name.chars().next().is_some_and(char::is_uppercase);
    if name.contains('_') {
        return Some(match (has_lower, has_upper) {
            (false, true) => "SCREAMING_SNAKE_CASE",
            (true, false) => "snake_case",
            _ => "mixed",
        });
    }
    match (has_lower, has_upper, first_upper) {
        (true, true, true) => Some("PascalCase"),
        (false, true, _) => Some("SCREAMING_SNAKE_CASE"),
        (true, true, false) => Some("camelCase"),
        _ => None,
    }
}

struct Walk<'s> {
    src: &'s [u8],
    lines: Vec<&'s str>,
    lang: Lang,
    s: FileStats,
    /// Rows that hold nothing but a comment.
    comment_rows: BTreeSet<usize>,
}

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    n.utf8_text(src).unwrap_or_default()
}

fn row(n: Node) -> u32 {
    u32::try_from(n.start_position().row + 1).unwrap_or(u32::MAX)
}

fn end_row(n: Node) -> u32 {
    u32::try_from(n.end_position().row + 1).unwrap_or(u32::MAX)
}

/// The declared name of a definition: field `name`, else the identifier inside `declarator`.
fn def_name<'a>(n: Node, src: &'a [u8]) -> Option<&'a str> {
    if let Some(name) = n.child_by_field_name("name") {
        return Some(text(name, src));
    }
    let mut d = n.child_by_field_name("declarator")?;
    for _ in 0..6 {
        match d.kind() {
            "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
                return Some(text(d, src));
            }
            "qualified_identifier" => {
                return d.child_by_field_name("name").map(|x| text(x, src));
            }
            _ => d = d.child_by_field_name("declarator")?,
        }
    }
    None
}

impl Walk<'_> {
    fn has_doc(&self, node: Node) -> bool {
        let src = self.src;
        if self.lang == Lang::Python
            && let Some(body) = node.child_by_field_name("body")
            && let Some(first) = body.named_child(0)
        {
            return first.kind() == "expression_statement"
                && first.named_child(0).is_some_and(|s| s.kind() == "string");
        }
        let mut target = node;
        while let Some(p) = target.parent() {
            if matches!(
                p.kind(),
                "export_statement"
                    | "decorated_definition"
                    | "template_declaration"
                    | "lexical_declaration"
                    | "variable_declaration"
                    | "variable_declarator"
            ) {
                target = p;
            } else {
                break;
            }
        }
        let mut expect = target.start_position().row;
        let mut prev = target.prev_sibling();
        while let Some(p) = prev {
            let k = p.kind();
            if k.contains("attribute") || k.contains("decorator") || k.contains("annotation") {
                expect = p.start_position().row;
                prev = p.prev_sibling();
                continue;
            }
            if !k.contains("comment") || p.end_position().row + 1 < expect {
                return false;
            }
            let t = text(p, src).trim_start();
            return match self.lang {
                Lang::Go | Lang::Ruby | Lang::Python => true,
                Lang::Rust => t.starts_with("///") || t.starts_with("//!") || t.starts_with("/**"),
                _ => t.starts_with("/**") || t.starts_with("///"),
            };
        }
        false
    }

    fn is_public(&self, node: Node, name: &str) -> bool {
        let src = self.src;
        match self.lang {
            Lang::Rust => (0..node.child_count())
                .filter_map(|i| node.child(i))
                .any(|c| c.kind() == "visibility_modifier" && text(c, src) == "pub"),
            Lang::Go => name.chars().next().is_some_and(char::is_uppercase),
            Lang::Python => !name.starts_with('_'),
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
                let mut p = node.parent();
                for _ in 0..3 {
                    match p {
                        Some(x) if x.kind() == "export_statement" => return true,
                        Some(x) => p = x.parent(),
                        None => break,
                    }
                }
                false
            }
            Lang::C | Lang::Cpp => !(0..node.child_count())
                .filter_map(|i| node.child(i))
                .any(|c| c.kind() == "storage_class_specifier" && text(c, src) == "static"),
            Lang::Java | Lang::CSharp | Lang::Php => {
                let mods = (0..node.child_count())
                    .filter_map(|i| node.child(i))
                    .filter(|c| c.kind().contains("modifier"))
                    .any(|c| text(c, src).contains("public"));
                mods || (self.lang == Lang::Php && node.kind() == "function_definition")
            }
            // Ruby and the languages without a visibility rule here: every item counts as public.
            _ => true,
        }
    }

    fn public_item(&mut self, node: Node, name: &str) -> (bool, bool) {
        let public = self.is_public(node, name);
        let documented = self.has_doc(node);
        if public {
            self.s.public_items += 1;
            if documented {
                self.s.documented_public += 1;
            } else if self.s.undocumented.len() < 200 {
                self.s.undocumented.push((name.to_owned(), row(node)));
            }
        }
        (public, documented)
    }

    fn name_use(&mut self, category: &str, name: &str, line: u32) {
        if name.is_empty() || name == "_" || name.len() > 120 {
            return;
        }
        let category = if category == "variable" && convention(name) == Some("SCREAMING_SNAKE_CASE")
        {
            "constant"
        } else {
            category
        };
        self.s.names.push(NameUse {
            category: category.to_owned(),
            name: name.to_owned(),
            line,
        });
    }

    fn error(&mut self, pattern: &str, line: u32) {
        self.s
            .errors
            .entry(pattern.to_owned())
            .or_default()
            .push(line);
    }

    fn construct(&mut self, name: &str) {
        *self.s.constructs.entry(name.to_owned()).or_default() += 1;
    }

    /// The name an anonymous function gets from what it is assigned to.
    fn anon_name(&self, node: Node) -> Option<String> {
        let p = node.parent()?;
        let field = match p.kind() {
            "variable_declarator" => "name",
            "pair" => "key",
            "assignment_expression" | "assignment" => "left",
            "public_field_definition" | "field_definition" => "property",
            _ => return None,
        };
        let n = p.child_by_field_name(field)?;
        matches!(
            n.kind(),
            "identifier" | "property_identifier" | "private_property_identifier"
        )
        .then(|| text(n, self.src).to_owned())
    }

    fn variables(&mut self, node: Node) {
        let src = self.src;
        let field = match node.kind() {
            "let_declaration" => "pattern",
            "variable_declarator" | "var_spec" | "const_spec" => "name",
            "assignment" | "assignment_expression" | "short_var_declaration" => "left",
            "init_declarator" => "declarator",
            _ => return,
        };
        let Some(target) = node.child_by_field_name(field) else {
            return;
        };
        if node.kind() == "variable_declarator"
            && node
                .child_by_field_name("value")
                .is_some_and(|v| ANON_FN_KINDS.contains(&v.kind()))
        {
            return; // counted as a function name
        }
        let mut ids = Vec::new();
        match target.kind() {
            "identifier" | "variable_name" | "name" => ids.push(target),
            "expression_list" => ids.extend(
                (0..target.named_child_count())
                    .filter_map(|i| target.named_child(i))
                    .filter(|c| c.kind() == "identifier"),
            ),
            _ => {}
        }
        for id in ids {
            let name = text(id, src).trim_start_matches('$').to_owned();
            self.name_use("variable", &name, row(id));
        }
    }

    fn called_name(&self, node: Node) -> Option<String> {
        let f = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("method"))
            .or_else(|| node.child_by_field_name("name"))?;
        let last = match f.kind() {
            "field_expression" | "selector_expression" => f.child_by_field_name("field"),
            "member_expression" => f.child_by_field_name("property"),
            "attribute" => f.child_by_field_name("attribute"),
            "member_access_expression" | "scoped_identifier" => f.child_by_field_name("name"),
            _ => Some(f),
        }?;
        Some(text(last, self.src).to_owned())
    }

    #[allow(clippy::too_many_lines)] // one dispatch over node kinds; splitting it hides the table
    fn run(&mut self, root: Node) {
        // (node, enclosing function index, `if` depth inside that function)
        let mut stack: Vec<(Node, Option<usize>, u32)> = vec![(root, None, 0)];
        while let Some((node, mut func, mut ifs)) = stack.pop() {
            let kind = node.kind();
            let src = self.src;
            if kind.contains("comment") {
                let start = node.start_position();
                let line = self.line_text(start.row);
                if line.trim_start().len() + start.column == line.len() {
                    self.comment_rows.insert(start.row);
                }
                let end = node.end_position();
                // A line comment's range ends at column 0 of the next row.
                let last = if end.column == 0 && end.row > start.row {
                    end.row - 1
                } else {
                    end.row
                };
                for r in start.row + 1..=last {
                    self.comment_rows.insert(r);
                }
                continue;
            }
            let named_fn = FN_KINDS.contains(&kind)
                && (self.lang == Lang::Ruby
                    || node.child_by_field_name("body").is_some()
                    || !matches!(kind, "method_declaration" | "function_declaration"));
            let fn_name = if named_fn {
                def_name(node, src).map(str::to_owned)
            } else if ANON_FN_KINDS.contains(&kind) {
                self.anon_name(node)
            } else {
                None
            };
            if let Some(name) = fn_name {
                let decl = if ANON_FN_KINDS.contains(&kind) {
                    node.parent().unwrap_or(node)
                } else {
                    node
                };
                let (public, documented) = self.public_item(decl, &name);
                self.name_use("function", &name, row(node));
                self.s.functions.push(FnStat {
                    name,
                    line: row(node),
                    end: end_row(node),
                    complexity: 1,
                    public,
                    documented,
                    early_returns: 0,
                });
                func = Some(self.s.functions.len() - 1);
                ifs = 0;
            }
            if TYPE_KINDS.contains(&kind)
                && let Some(name) = def_name(node, src).map(str::to_owned)
            {
                self.s.types += 1;
                let decl = if kind == "type_spec" {
                    node.parent().unwrap_or(node)
                } else {
                    node
                };
                self.public_item(decl, &name);
                self.name_use("type", &name, row(node));
            }
            let decision = DECISION_KINDS.contains(&kind)
                || kind == "ternary_expression"
                || (matches!(kind, "binary_expression" | "boolean_operator" | "binary")
                    && node.child_by_field_name("operator").is_some_and(|o| {
                        matches!(text(o, src), "&&" | "||" | "and" | "or" | "??")
                    }));
            if decision && let Some(f) = func {
                self.s.functions[f].complexity += 1;
            }
            if matches!(kind, "return_statement" | "return_expression" | "return")
                && ifs > 0
                && let Some(f) = func
            {
                self.s.functions[f].early_returns += 1;
            }
            if IF_KINDS.contains(&kind) {
                ifs += 1;
                self.construct("if");
                if self.lang == Lang::Go
                    && node
                        .child_by_field_name("condition")
                        .is_some_and(|c| text(c, src).replace(' ', "").contains("err!=nil"))
                {
                    self.error("if err != nil", row(node));
                }
            }
            self.variables(node);
            self.patterns(node, kind);
            for i in (0..node.named_child_count()).rev() {
                if let Some(c) = node.named_child(i) {
                    stack.push((c, func, ifs));
                }
            }
        }
    }

    fn patterns(&mut self, node: Node, kind: &str) {
        let src = self.src;
        let line = row(node);
        match kind {
            "try_expression" => self.error("?", line),
            "try_statement" | "begin" => self.error("try/catch", line),
            "except_clause" => {
                let bare = node.named_child_count() <= 1;
                self.error(if bare { "bare except" } else { "except" }, line);
            }
            "rescue" => self.error("rescue", line),
            "throw_statement" | "throw_expression" | "raise_statement" => self.error("throw", line),
            "macro_invocation" => {
                let m = node
                    .child_by_field_name("macro")
                    .map_or("", |m| text(m, src));
                match m {
                    "panic" | "todo" | "unimplemented" | "unreachable" => {
                        self.error("panic!", line);
                    }
                    "format" => self.construct("format!"),
                    _ => {}
                }
            }
            "lexical_declaration" => {
                let first = node.child(0).map_or("", |c| text(c, src));
                if first == "const" || first == "let" {
                    self.construct(first);
                }
            }
            "variable_declaration"
                if matches!(self.lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx) =>
            {
                self.construct("var");
            }
            "arrow_function" => self.construct("arrow function"),
            "function_declaration"
                if matches!(self.lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx) =>
            {
                self.construct("function declaration");
            }
            "conditional_expression" | "ternary_expression" => self.construct("ternary"),
            "list_comprehension"
            | "dictionary_comprehension"
            | "set_comprehension"
            | "generator_expression" => self.construct("comprehension"),
            "template_string" | "interpolated_string_expression" | "interpolation" => {
                self.construct("interpolation");
            }
            "string" if self.lang == Lang::Python && text(node, src).starts_with(['f', 'F']) => {
                self.construct("interpolation");
            }
            "await_expression" | "await" => self.construct("await"),
            k if LOOP_KINDS.contains(&k) => self.construct("loop"),
            k if MATCH_KINDS.contains(&k) => self.construct("match/switch"),
            k if CALL_KINDS.contains(&k) => {
                if let Some(name) = self.called_name(node) {
                    match name.as_str() {
                        "unwrap" if self.lang == Lang::Rust => self.error("unwrap", line),
                        "expect" if self.lang == Lang::Rust => self.error("expect", line),
                        "catch" => self.error(".catch()", line),
                        n if ITER_METHODS.contains(&n) => self.construct("iterator chain"),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn line_text(&self, row: usize) -> &str {
        self.lines.get(row).copied().unwrap_or_default()
    }
}

/// A line counts for duplicate detection when it has some substance (not just braces).
fn significant(line: &str) -> Option<&str> {
    let t = line.trim();
    (t.len() >= 4 && t.chars().any(char::is_alphanumeric)).then_some(t)
}

fn windows(text: &str) -> Vec<(u64, u32, u32)> {
    let lines: Vec<(u32, &str)> = text
        .lines()
        .enumerate()
        .filter_map(|(i, l)| significant(l).map(|t| (u32::try_from(i + 1).unwrap_or(u32::MAX), t)))
        .collect();
    lines
        .windows(DUP_WINDOW)
        .map(|w| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for (_, t) in w {
                t.hash(&mut h);
            }
            (h.finish(), w[0].0, w[DUP_WINDOW - 1].0)
        })
        .collect()
}

/// Statistics of one file's text. `parser` is reused across files of the same thread.
pub fn analyze_with(parser: &mut Parser, lang: Lang, path: &str, text: &str) -> FileStats {
    let mut s = FileStats {
        path: path.to_owned(),
        lang: lang.name().to_owned(),
        is_test: is_test_path(path),
        has_tests: TEST_MARKERS.is_match(text),
        windows: windows(text),
        ..FileStats::default()
    };
    let total = text.lines().count();
    let blank = text.lines().filter(|l| l.trim().is_empty()).count();
    s.lines = u32::try_from(total).unwrap_or(u32::MAX);
    s.blank_lines = u32::try_from(blank).unwrap_or(u32::MAX);
    if parser.set_language(&lang.language()).is_err() {
        s.code_lines = s.lines - s.blank_lines;
        return s;
    }
    let Some(tree) = parser.parse(text, None) else {
        s.code_lines = s.lines - s.blank_lines;
        return s;
    };
    let mut w = Walk {
        src: text.as_bytes(),
        lines: text.lines().collect(),
        lang,
        s,
        comment_rows: BTreeSet::new(),
    };
    w.run(tree.root_node());
    let comment_only = w
        .comment_rows
        .iter()
        .filter(|r| w.lines.get(**r).is_some_and(|l| !l.trim().is_empty()))
        .count();
    let mut s = w.s;
    s.comment_lines = u32::try_from(comment_only).unwrap_or(u32::MAX);
    s.code_lines = s.lines.saturating_sub(s.blank_lines + s.comment_lines);
    s
}

/// Statistics of one file's text.
pub fn analyze(lang: Lang, path: &str, text: &str) -> FileStats {
    analyze_with(&mut Parser::new(), lang, path, text)
}

/// Source files of the project: git-ignore aware, hidden and vendored directories skipped,
/// files over [`MAX_FILE_BYTES`] and languages without a grammar left out.
pub fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ignore::WalkBuilder::new(root)
        .hidden(true)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES))
        .map(ignore::DirEntry::into_path)
        .filter(|p| lang_of(&p.to_string_lossy()).is_some())
        .collect();
    out.sort();
    out
}

/// [`analyze`] every source file of `root` on all cores, sorted by path.
pub fn scan(root: &Path) -> Vec<FileStats> {
    let files = source_files(root);
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(16);
    let chunk = files.len().div_ceil(threads).max(1);
    let mut out: Vec<FileStats> = std::thread::scope(|sc| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                sc.spawn(move || {
                    let mut parser = Parser::new();
                    part.iter()
                        .filter_map(|p| {
                            let rel = super::rel(root, p);
                            let lang = lang_of(&rel)?;
                            let text = std::fs::read_to_string(p).ok()?;
                            Some(analyze_with(&mut parser, lang, &rel, &text))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A block of lines that also appears elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DupBlock {
    pub file: String,
    pub line: u32,
    pub end: u32,
    pub other: String,
    pub other_line: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Duplication {
    /// Lines inside a window of [`DUP_WINDOW`] significant lines that occurs at least twice.
    pub duplicated_lines: u64,
    pub code_lines: u64,
    /// First copy of each duplicated block, at most 50.
    pub blocks: Vec<DupBlock>,
    /// Duplicated lines per file.
    pub per_file: BTreeMap<String, u32>,
}

impl Duplication {
    pub fn ratio(&self) -> f64 {
        if self.code_lines == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)] // line counts are far below 2^52
            let r = self.duplicated_lines as f64 / self.code_lines as f64;
            r.min(1.0)
        }
    }
}

/// Repeated windows across (and within) `files`; test files are left out.
pub fn duplication(files: &[FileStats]) -> Duplication {
    let mut by_hash: HashMap<u64, Vec<(usize, u32, u32)>> = HashMap::new();
    for (i, f) in files.iter().enumerate().filter(|(_, f)| !f.is_test) {
        for &(h, a, b) in &f.windows {
            by_hash.entry(h).or_default().push((i, a, b));
        }
    }
    let mut marked: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); files.len()];
    let mut groups: Vec<&Vec<(usize, u32, u32)>> =
        by_hash.values().filter(|v| v.len() > 1).collect();
    groups.sort_by_key(|v| (v[0].0, v[0].1));
    let mut blocks: Vec<DupBlock> = Vec::new();
    for g in groups {
        for &(i, a, b) in g {
            marked[i].extend(a..=b);
        }
        let (fa, la, ea) = g[0];
        let (fb, lb, _) = g[1];
        let (file, other) = (&files[fa].path, &files[fb].path);
        // Consecutive windows of one long copy extend the previous block instead of a new one.
        if let Some(last) = blocks
            .iter_mut()
            .rev()
            .find(|x| &x.file == file && &x.other == other && x.end + 1 >= la && x.line <= la)
        {
            last.end = last.end.max(ea);
            continue;
        }
        if blocks.len() < 50 {
            blocks.push(DupBlock {
                file: file.clone(),
                line: la,
                end: ea,
                other: other.clone(),
                other_line: lb,
            });
        }
    }
    let mut per_file = BTreeMap::new();
    let mut dup = 0u64;
    for (i, m) in marked.iter().enumerate() {
        if !m.is_empty() {
            per_file.insert(
                files[i].path.clone(),
                u32::try_from(m.len()).unwrap_or(u32::MAX),
            );
            dup += m.len() as u64;
        }
    }
    Duplication {
        duplicated_lines: dup,
        code_lines: files
            .iter()
            .filter(|f| !f.is_test)
            .map(|f| u64::from(f.code_lines))
            .sum(),
        blocks,
        per_file,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_functions_complexity_docs_and_naming() {
        let src = r#"
/// Adds.
pub fn add_one(x: i32) -> i32 {
    if x > 0 && x < 10 {
        return x + 1;
    }
    match x {
        0 => 1,
        _ => x,
    }
}

fn helper() -> Result<(), String> {
    let some_value = std::fs::read("a").map_err(|e| e.to_string())?;
    let n = some_value.len();
    let _ = n.checked_add(1).unwrap();
    Ok(())
}

pub struct Thing;
// plain comment
"#;
        let s = analyze(Lang::Rust, "src/lib.rs", src);
        assert_eq!(s.functions.len(), 2);
        let add = &s.functions[0];
        assert_eq!(add.name, "add_one");
        // 1 + if + && + 2 match arms
        assert_eq!(add.complexity, 5);
        assert_eq!(add.early_returns, 1);
        assert!(add.public && add.documented);
        assert!(!s.functions[1].public);
        assert_eq!(s.public_items, 2);
        assert_eq!(s.documented_public, 1);
        assert_eq!(s.undocumented, vec![("Thing".to_owned(), 20)]);
        assert_eq!(s.errors["?"], vec![14]);
        assert_eq!(s.errors["unwrap"], vec![16]);
        assert!(
            s.names
                .iter()
                .any(|n| n.name == "some_value" && n.category == "variable")
        );
        assert_eq!(s.comment_lines, 2);
    }

    #[test]
    fn js_and_python_constructs() {
        let js = "/** doc */\nexport const loadUser = async (id) => {\n  const userName = id ? 'a' : 'b'\n  try { return [1].map(x => x) } catch (e) { throw e }\n}\nfunction other() { let total = 1; return total }\n";
        let s = analyze(Lang::JavaScript, "src/user.js", js);
        assert_eq!(
            s.functions
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["loadUser", "other"]
        );
        assert!(s.functions[0].public && s.functions[0].documented);
        assert_eq!(s.constructs["const"], 2);
        assert_eq!(s.constructs["let"], 1);
        assert_eq!(s.constructs["ternary"], 1);
        assert!(s.errors.contains_key("try/catch") && s.errors.contains_key("throw"));
        let py = "def load_user(uid):\n    \"\"\"Doc.\"\"\"\n    try:\n        return [x for x in uid]\n    except:\n        pass\n\ndef _private():\n    pass\n";
        let p = analyze(Lang::Python, "app/users.py", py);
        assert_eq!(p.functions.len(), 2);
        assert!(p.functions[0].documented && p.functions[0].public);
        assert!(!p.functions[1].public);
        assert_eq!(p.errors["bare except"].len(), 1);
        assert_eq!(p.constructs["comprehension"], 1);
    }

    #[test]
    fn conventions() {
        assert_eq!(convention("load_user"), Some("snake_case"));
        assert_eq!(convention("loadUser"), Some("camelCase"));
        assert_eq!(convention("LoadUser"), Some("PascalCase"));
        assert_eq!(convention("MAX_SIZE"), Some("SCREAMING_SNAKE_CASE"));
        assert_eq!(convention("run"), None);
        assert!(is_test_path("src/foo_test.go"));
        assert!(is_test_path("tests/a.rs"));
        assert!(is_test_path("web/a.spec.ts"));
        assert!(!is_test_path("src/contest.rs"));
    }

    #[test]
    fn duplicate_blocks_are_found_once() {
        let body = "let alpha = compute(1);\nlet beta = compute(2);\nlet gamma = alpha + beta;\nprintln!(\"{gamma}\");\nlet delta = gamma * 2;\nlet eps = delta - 1;\nlet zeta = eps / 3;\n";
        let a = analyze(Lang::Rust, "a.rs", &format!("fn a() {{\n{body}}}\n"));
        let b = analyze(Lang::Rust, "b.rs", &format!("fn b() {{\n{body}}}\n"));
        let d = duplication(&[a, b]);
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.blocks[0].file, "a.rs");
        assert_eq!((d.blocks[0].line, d.blocks[0].end), (2, 8));
        assert_eq!(d.duplicated_lines, 14);
    }
}
