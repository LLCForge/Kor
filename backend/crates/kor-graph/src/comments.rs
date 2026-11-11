//! Comments and undocumented functions of one source file (tree-sitter), for `kor comment`
//! (Master Build Prompt §12.2): every grammar names its comment nodes `*comment*`, so one walk
//! covers all supported languages without per-language queries.

use tree_sitter::{Node, Parser};

use crate::lang::Lang;

/// One comment as it appears in the file (markers included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Byte range in the source.
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// A function or method with no comment right above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undocumented {
    /// Byte offset of the start of the function's first line (where a comment line goes).
    pub line_start: usize,
    /// Leading whitespace of that line.
    pub indent: String,
    /// The function's first line, trimmed (signature for the model).
    pub signature: String,
}

const FUNCTION_KINDS: &[&str] = &[
    "function_item",
    "function_definition",
    "function_declaration",
    "method_definition",
    "method_declaration",
    "method",
    "singleton_method",
    "constructor_declaration",
    "generator_function_declaration",
];

fn parse(lang: Lang, src: &str) -> Option<tree_sitter::Tree> {
    let mut p = Parser::new();
    p.set_language(&lang.language()).ok()?;
    p.parse(src, None)
}

fn walk<'a>(n: Node<'a>, f: &mut impl FnMut(Node<'a>) -> bool) {
    if !f(n) {
        return;
    }
    let mut c = n.walk();
    for child in n.children(&mut c) {
        walk(child, f);
    }
}

/// Every comment in `src`, in file order. Empty when the file does not parse.
pub fn comments(lang: Lang, src: &str) -> Vec<Comment> {
    let Some(tree) = parse(lang, src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(tree.root_node(), &mut |n| {
        if n.kind().contains("comment") {
            let (start, end) = (n.start_byte(), n.end_byte());
            // Rust line comments include the newline; keep it out of the span.
            let text = src[start..end].trim_end_matches(['\r', '\n']);
            out.push(Comment {
                start,
                end: start + text.len(),
                text: text.to_owned(),
            });
            return false;
        }
        true
    });
    out
}

/// Functions and methods without a comment on the line(s) right above them.
pub fn undocumented(lang: Lang, src: &str) -> Vec<Undocumented> {
    let Some(tree) = parse(lang, src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(tree.root_node(), &mut |n| {
        if !FUNCTION_KINDS.contains(&n.kind()) {
            return true;
        }
        // Attributes/decorators/`export` sit between a doc comment and the function.
        let mut first = n;
        while let Some(p) = first
            .parent()
            .filter(|p| matches!(p.kind(), "decorated_definition" | "export_statement"))
        {
            first = p;
        }
        let mut prev = first.prev_sibling();
        while let Some(p) = prev.filter(|p| {
            matches!(
                p.kind(),
                "attribute_item"
                    | "decorator"
                    | "attribute_list"
                    | "annotation"
                    | "marker_annotation"
            )
        }) {
            prev = p.prev_sibling();
        }
        let documented = prev.is_some_and(|p| {
            p.kind().contains("comment") && first.start_position().row - p.end_position().row <= 1
        });
        if !documented {
            let line_start = src[..first.start_byte()].rfind('\n').map_or(0, |i| i + 1);
            let line = src[line_start..].lines().next().unwrap_or_default();
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push(Undocumented {
                line_start,
                indent,
                signature: line.trim().chars().take(200).collect(),
            });
        }
        true
    });
    out
}

/// The line-comment marker a new doc comment uses in `lang`.
pub fn doc_marker(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust | Lang::CSharp => "///",
        Lang::Python | Lang::Ruby => "#",
        _ => "//",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_comments_but_not_comment_like_strings() {
        let src = "// head\nfn a() {\n    let s = \"// not a comment\"; /* block */\n}\n";
        let c = comments(Lang::Rust, src);
        let texts: Vec<_> = c.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["// head", "/* block */"]);
        assert_eq!(&src[c[1].start..c[1].end], "/* block */");
        let py = comments(Lang::Python, "x = '# no'  # yes\n");
        assert_eq!(py.len(), 1);
        assert_eq!(py[0].text, "# yes");
    }

    #[test]
    fn lists_functions_without_a_comment_above() {
        let src =
            "/// documented\n#[inline]\nfn a() {}\n\nfn b() {}\nimpl X {\n    fn c(&self) {}\n}\n";
        let u = undocumented(Lang::Rust, src);
        let sigs: Vec<_> = u.iter().map(|u| u.signature.as_str()).collect();
        assert_eq!(sigs, ["fn b() {}", "fn c(&self) {}"]);
        assert_eq!(u[1].indent, "    ");
        assert_eq!(&src[u[1].line_start..u[1].line_start + 4], "    ");
        let ts = undocumented(
            Lang::TypeScript,
            "// ok\nexport function f() {}\nexport function g() {}\n",
        );
        assert_eq!(ts.len(), 1);
        assert!(ts[0].signature.contains("g()"));
    }
}
