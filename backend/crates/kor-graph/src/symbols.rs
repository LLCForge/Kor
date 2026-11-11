//! Find a named function, method or type in one source file (tree-sitter), for structural edits
//! (`EditSymbol`, Master Build Prompt §8.1: "replace the body of this function"). Names may be
//! qualified with their container (`Cart::total`, `Cart.total`).

use tree_sitter::{Node, Parser};

use crate::lang::Lang;

/// Where a symbol sits in the source (byte ranges).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// `function`, `method`, `class`…  (the grammar's node kind).
    pub kind: String,
    /// `Container::name` when it has a container, else `name`.
    pub qualified: String,
    pub start: usize,
    pub end: usize,
    /// The body (`{ … }` or an indented block); `None` for a declaration without one.
    pub body: Option<(usize, usize)>,
    /// 1-based first line.
    pub line: usize,
}

const DEFINITION_KINDS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "method_definition",
    "method_declaration",
    "method",
    "singleton_method",
    "constructor_declaration",
    "generator_function_declaration",
    "class_declaration",
    "class_definition",
    "class",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "interface_declaration",
    "struct_specifier",
    "enum_declaration",
    "module",
];

fn name_of<'a>(n: Node<'_>, src: &'a str) -> Option<&'a str> {
    // `impl Cart { … }` is named by its type; elsewhere `type` is a return type (`int add(…)`).
    let field = n
        .child_by_field_name("name")
        .or_else(|| if n.kind() == "impl_item" { n.child_by_field_name("type") } else { None });
    if let Some(f) = field {
        return f.utf8_text(src.as_bytes()).ok();
    }
    // C/C++: `function_definition > declarator: function_declarator > declarator: identifier`.
    let mut d = n.child_by_field_name("declarator");
    while let Some(x) = d {
        match x.child_by_field_name("declarator") {
            Some(inner) => d = Some(inner),
            None => return x.utf8_text(src.as_bytes()).ok(),
        }
    }
    None
}

/// Every definition in `src`, outermost first.
pub fn definitions(lang: Lang, src: &str) -> Vec<Symbol> {
    let mut p = Parser::new();
    if p.set_language(&lang.language()).is_err() {
        return Vec::new();
    }
    let Some(tree) = p.parse(src, None) else { return Vec::new() };
    let mut out = Vec::new();
    fn walk(n: Node<'_>, src: &str, container: &[String], out: &mut Vec<Symbol>) {
        let mut inner = container.to_vec();
        if DEFINITION_KINDS.contains(&n.kind())
            && let Some(name) = name_of(n, src)
        {
            let name = name.trim().to_owned();
            let qualified = if container.is_empty() { name.clone() } else { format!("{}::{name}", container.join("::")) };
            out.push(Symbol {
                kind: n.kind().to_owned(),
                qualified,
                start: n.start_byte(),
                end: n.end_byte(),
                body: n.child_by_field_name("body").map(|b| (b.start_byte(), b.end_byte())),
                line: n.start_position().row + 1,
            });
            inner.push(name);
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            walk(child, src, &inner, out);
        }
    }
    walk(tree.root_node(), src, &[], &mut out);
    out
}

/// Definitions whose name is `name`: a bare name matches at any depth, `A::b` / `A.b` must match
/// the end of the qualified name.
pub fn find(lang: Lang, src: &str, name: &str) -> Vec<Symbol> {
    let want: Vec<&str> = name.split(|c| c == ':' || c == '.').filter(|s| !s.is_empty()).collect();
    if want.is_empty() {
        return Vec::new();
    }
    definitions(lang, src)
        .into_iter()
        .filter(|s| {
            let have: Vec<&str> = s.qualified.split("::").collect();
            have.len() >= want.len() && have[have.len() - want.len()..] == want[..]
        })
        .collect()
}

/// The first syntax error in `src` as (1-based line, 1-based column); `None` when it parses.
pub fn first_error(lang: Lang, src: &str) -> Option<(usize, usize)> {
    let mut p = Parser::new();
    p.set_language(&lang.language()).ok()?;
    let tree = p.parse(src, None)?;
    if !tree.root_node().has_error() {
        return None;
    }
    let mut found = None;
    walk_err(tree.root_node(), &mut found);
    found.or(Some((1, 1)))
}

fn walk_err(n: Node<'_>, found: &mut Option<(usize, usize)>) {
    if found.is_some() || !n.has_error() {
        return;
    }
    if n.is_error() || n.is_missing() {
        let p = n.start_position();
        *found = Some((p.row + 1, p.column + 1));
        return;
    }
    let mut c = n.walk();
    for child in n.children(&mut c) {
        walk_err(child, found);
    }
}

/// `true` when `src` parses without syntax errors.
pub fn parses(lang: Lang, src: &str) -> bool {
    let mut p = Parser::new();
    p.set_language(&lang.language()).is_ok() && p.parse(src, None).is_some_and(|t| !t.root_node().has_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_functions_methods_and_qualified_names() {
        let src = "fn total() -> u32 { 1 }\nstruct Cart;\nimpl Cart {\n    fn total(&self) -> u32 {\n        2\n    }\n}\n";
        let all = find(Lang::Rust, src, "total");
        assert_eq!(all.len(), 2);
        let m = find(Lang::Rust, src, "Cart::total");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].line, 4);
        let (b0, b1) = m[0].body.unwrap();
        assert!(src[b0..b1].starts_with('{') && src[b0..b1].contains('2'));
        assert_eq!(find(Lang::Rust, src, "Cart.total").len(), 1);
        assert!(find(Lang::Rust, src, "Nope").is_empty());
        let py = "class Cart:\n    def total(self):\n        return 2\n";
        let p = find(Lang::Python, py, "Cart.total");
        assert_eq!(p.len(), 1);
        let (b0, b1) = p[0].body.unwrap();
        assert_eq!(&py[b0..b1], "return 2");
        let c = "int add(int a, int b) { return a + b; }\n";
        assert_eq!(find(Lang::C, c, "add").len(), 1);
        assert!(parses(Lang::Rust, src));
        assert!(!parses(Lang::Rust, "fn x( {"));
        assert!(first_error(Lang::Rust, "fn ok() {}\nfn bad( {\n").is_some_and(|(l, _)| l == 2));
        assert_eq!(first_error(Lang::Python, "def f():\n    return 1\n"), None);
    }
}
