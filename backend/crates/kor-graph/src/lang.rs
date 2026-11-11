//! Supported languages: grammar, tags query and the extra import/heritage patterns.

use std::sync::OnceLock;
use tree_sitter::{Language, Query};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Go,
    Java,
    CSharp,
    C,
    Cpp,
    Php,
    Ruby,
}

pub const ALL: [Lang; 12] = [
    Lang::Rust,
    Lang::Python,
    Lang::TypeScript,
    Lang::Tsx,
    Lang::JavaScript,
    Lang::Go,
    Lang::Java,
    Lang::CSharp,
    Lang::C,
    Lang::Cpp,
    Lang::Php,
    Lang::Ruby,
];

impl Lang {
    pub fn from_ext(ext: &str) -> Option<Self> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "go" => Self::Go,
            "java" => Self::Java,
            "cs" => Self::CSharp,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "php" => Self::Php,
            "rb" => Self::Ruby,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Php => "php",
            Self::Ruby => "ruby",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        ALL.into_iter().find(|l| l.name() == s)
    }

    pub fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        }
    }

    /// The grammar's own `tags.scm` plus our patterns. Our capture names: `import`, `import_rel`,
    /// `mod_decl`, `extends`, `implements`, `impl_type`, `call`, `qual` (the `Type` of `Type::f()`).
    #[allow(clippy::too_many_lines)]
    fn query_source(self) -> String {
        const JS_EXTRA: &str = r#"
(import_statement source: (string) @import)
(export_statement source: (string) @import)
(call_expression function: (identifier) @_req arguments: (arguments (string) @import) (#eq? @_req "require"))
"#;
        let (tags, extra): (&str, &str) = match self {
            Self::Rust => (
                tree_sitter_rust::TAGS_QUERY,
                r"
(use_declaration argument: (_) @import)
(mod_item name: (identifier) @mod_decl !body)
(call_expression function: (scoped_identifier path: (_) @qual name: (identifier) @call))
(impl_item trait: (_) @implements type: (_) @impl_type)
",
            ),
            Self::Python => (
                tree_sitter_python::TAGS_QUERY,
                r"
(import_statement name: (dotted_name) @import)
(import_statement name: (aliased_import name: (dotted_name) @import))
(import_from_statement module_name: (_) @import)
(class_definition superclasses: (argument_list (identifier) @extends))
(class_definition superclasses: (argument_list (attribute) @extends))
",
            ),
            Self::JavaScript => (
                tree_sitter_javascript::TAGS_QUERY,
                "(class_heritage (_) @extends)",
            ),
            Self::TypeScript | Self::Tsx => (
                tree_sitter_typescript::TAGS_QUERY,
                r"
(extends_clause value: (_) @extends)
(implements_clause (_) @implements)
(extends_type_clause type: (_) @extends)
",
            ),
            Self::Go => (
                tree_sitter_go::TAGS_QUERY,
                "(import_spec path: (_) @import)",
            ),
            Self::Java => (
                tree_sitter_java::TAGS_QUERY,
                r"
(import_declaration) @import
(superclass (type_identifier) @extends)
(super_interfaces (type_list (_) @implements))
(extends_interfaces (type_list (_) @extends))
",
            ),
            Self::CSharp => (
                tree_sitter_c_sharp::TAGS_QUERY,
                r"
(using_directive) @import
(base_list (_) @extends)
(invocation_expression function: (identifier) @call)
",
            ),
            Self::C => (
                tree_sitter_c::TAGS_QUERY,
                r"
(preproc_include path: (_) @import)
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
",
            ),
            Self::Cpp => (
                tree_sitter_cpp::TAGS_QUERY,
                r"
(preproc_include path: (_) @import)
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
(call_expression function: (qualified_identifier name: (identifier) @call))
(base_class_clause (_) @extends)
",
            ),
            Self::Php => (
                tree_sitter_php::TAGS_QUERY,
                r"
(namespace_use_clause) @import
(function_call_expression function: (name) @call)
(require_expression (_) @import_rel)
(require_once_expression (_) @import_rel)
(include_expression (_) @import_rel)
(include_once_expression (_) @import_rel)
(base_clause (_) @extends)
(class_interface_clause (_) @implements)
",
            ),
            Self::Ruby => (
                tree_sitter_ruby::TAGS_QUERY,
                r#"
(call method: (identifier) @_m arguments: (argument_list (string) @import) (#eq? @_m "require"))
(call method: (identifier) @_m arguments: (argument_list (string) @import_rel) (#eq? @_m "require_relative"))
(superclass (_) @extends)
"#,
            ),
        };
        let mut src = String::new();
        // The TypeScript grammar is a superset of JavaScript; its tags.scm only adds TS-specific patterns.
        if matches!(self, Self::TypeScript | Self::Tsx) {
            src.push_str(tree_sitter_javascript::TAGS_QUERY);
            src.push('\n');
        }
        src.push_str(tags);
        src.push('\n');
        if matches!(self, Self::JavaScript | Self::TypeScript | Self::Tsx) {
            src.push_str(JS_EXTRA);
        }
        src.push_str(extra);
        src
    }

    /// Compiled query, built once per language. `None` if the grammar rejected it (a bug; tested).
    pub fn query(self) -> Option<&'static Query> {
        static CACHE: [OnceLock<Option<Query>>; 12] = [const { OnceLock::new() }; 12];
        let idx = ALL.iter().position(|l| *l == self)?;
        CACHE[idx]
            .get_or_init(|| Query::new(&self.language(), &self.query_source()).ok())
            .as_ref()
    }

    #[cfg(test)]
    pub fn query_error(self) -> Option<String> {
        Query::new(&self.language(), &self.query_source())
            .err()
            .map(|e| format!("{e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_compiles() {
        for l in ALL {
            assert_eq!(l.query_error(), None, "{}", l.name());
            assert!(l.query().is_some());
            assert_eq!(Lang::from_name(l.name()), Some(l));
        }
        assert_eq!(Lang::from_ext("TSX"), Some(Lang::Tsx));
        assert_eq!(Lang::from_ext("md"), None);
    }
}
