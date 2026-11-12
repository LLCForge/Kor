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
    Bash,
    Lua,
    Scala,
    Swift,
    Kotlin,
    Haskell,
    Elixir,
    Ocaml,
    Zig,
    Julia,
    R,
    Dart,
    Erlang,
    Elm,
    Nix,
    Hcl,
    Solidity,
    Fortran,
    Gleam,
    Groovy,
    ObjC,
    PowerShell,
    FSharp,
    D,
    Verilog,
    Cuda,
    Ada,
    Pascal,
    GdScript,
    Odin,
    Sql,
    Proto,
    Vhdl,
    CommonLisp,
}

pub const ALL: [Lang; 46] = [
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
    Lang::Bash,
    Lang::Lua,
    Lang::Scala,
    Lang::Swift,
    Lang::Kotlin,
    Lang::Haskell,
    Lang::Elixir,
    Lang::Ocaml,
    Lang::Zig,
    Lang::Julia,
    Lang::R,
    Lang::Dart,
    Lang::Erlang,
    Lang::Elm,
    Lang::Nix,
    Lang::Hcl,
    Lang::Solidity,
    Lang::Fortran,
    Lang::Gleam,
    Lang::Groovy,
    Lang::ObjC,
    Lang::PowerShell,
    Lang::FSharp,
    Lang::D,
    Lang::Verilog,
    Lang::Cuda,
    Lang::Ada,
    Lang::Pascal,
    Lang::GdScript,
    Lang::Odin,
    Lang::Sql,
    Lang::Proto,
    Lang::Vhdl,
    Lang::CommonLisp,
];

impl Lang {
    /// File extensions (lower case) of this language. `h` is C (also read by C++).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["rs"],
            Self::Python => &["py", "pyi"],
            Self::TypeScript => &["ts", "mts", "cts"],
            Self::Tsx => &["tsx"],
            Self::JavaScript => &["js", "mjs", "cjs", "jsx"],
            Self::Go => &["go"],
            Self::Java => &["java"],
            Self::CSharp => &["cs"],
            Self::C => &["c", "h"],
            Self::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
            Self::Php => &["php"],
            Self::Ruby => &["rb"],
            Self::Bash => &["sh", "bash"],
            Self::Lua => &["lua"],
            Self::Scala => &["scala", "sc"],
            Self::Swift => &["swift"],
            Self::Kotlin => &["kt", "kts"],
            Self::Haskell => &["hs"],
            Self::Elixir => &["ex", "exs"],
            Self::Ocaml => &["ml"],
            Self::Zig => &["zig"],
            Self::Julia => &["jl"],
            Self::R => &["r"],
            Self::Dart => &["dart"],
            Self::Erlang => &["erl", "hrl"],
            Self::Elm => &["elm"],
            Self::Nix => &["nix"],
            Self::Hcl => &["hcl", "tf", "tfvars"],
            Self::Solidity => &["sol"],
            Self::Fortran => &["f", "for", "f90", "f95", "f03", "f08"],
            Self::Gleam => &["gleam"],
            Self::Groovy => &["groovy", "gradle"],
            Self::ObjC => &["m", "mm"],
            Self::PowerShell => &["ps1", "psm1"],
            Self::FSharp => &["fs", "fsx"],
            Self::D => &["d", "di"],
            Self::Verilog => &["v", "vh", "sv", "svh"],
            Self::Cuda => &["cu", "cuh"],
            Self::Ada => &["adb", "ads"],
            Self::Pascal => &["pas", "pp", "dpr"],
            Self::GdScript => &["gd"],
            Self::Odin => &["odin"],
            Self::Sql => &["sql"],
            Self::Proto => &["proto"],
            Self::Vhdl => &["vhd", "vhdl"],
            Self::CommonLisp => &["lisp", "lsp", "cl"],
        }
    }

    pub fn from_ext(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        ALL.into_iter()
            .find(|l| l.extensions().contains(&ext.as_str()))
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
            Self::Bash => "bash",
            Self::Lua => "lua",
            Self::Scala => "scala",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Haskell => "haskell",
            Self::Elixir => "elixir",
            Self::Ocaml => "ocaml",
            Self::Zig => "zig",
            Self::Julia => "julia",
            Self::R => "r",
            Self::Dart => "dart",
            Self::Erlang => "erlang",
            Self::Elm => "elm",
            Self::Nix => "nix",
            Self::Hcl => "hcl",
            Self::Solidity => "solidity",
            Self::Fortran => "fortran",
            Self::Gleam => "gleam",
            Self::Groovy => "groovy",
            Self::ObjC => "objc",
            Self::PowerShell => "powershell",
            Self::FSharp => "fsharp",
            Self::D => "d",
            Self::Verilog => "verilog",
            Self::Cuda => "cuda",
            Self::Ada => "ada",
            Self::Pascal => "pascal",
            Self::GdScript => "gdscript",
            Self::Odin => "odin",
            Self::Sql => "sql",
            Self::Proto => "proto",
            Self::Vhdl => "vhdl",
            Self::CommonLisp => "commonlisp",
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
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
            Self::Lua => tree_sitter_lua::LANGUAGE.into(),
            Self::Scala => tree_sitter_scala::LANGUAGE.into(),
            Self::Swift => tree_sitter_swift::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Self::Haskell => tree_sitter_haskell::LANGUAGE.into(),
            Self::Elixir => tree_sitter_elixir::LANGUAGE.into(),
            Self::Ocaml => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            Self::Zig => tree_sitter_zig::LANGUAGE.into(),
            Self::Julia => tree_sitter_julia::LANGUAGE.into(),
            Self::R => tree_sitter_r::LANGUAGE.into(),
            Self::Dart => tree_sitter_dart::LANGUAGE.into(),
            Self::Erlang => tree_sitter_erlang::LANGUAGE.into(),
            Self::Elm => tree_sitter_elm::LANGUAGE.into(),
            Self::Nix => tree_sitter_nix::LANGUAGE.into(),
            Self::Hcl => tree_sitter_hcl::LANGUAGE.into(),
            Self::Solidity => tree_sitter_solidity::LANGUAGE.into(),
            Self::Fortran => tree_sitter_fortran::LANGUAGE.into(),
            Self::Gleam => tree_sitter_gleam::LANGUAGE.into(),
            Self::Groovy => tree_sitter_groovy::LANGUAGE.into(),
            Self::ObjC => tree_sitter_objc::LANGUAGE.into(),
            Self::PowerShell => tree_sitter_powershell::LANGUAGE.into(),
            Self::FSharp => tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
            Self::D => tree_sitter_d::LANGUAGE.into(),
            Self::Verilog => tree_sitter_verilog::LANGUAGE.into(),
            Self::Cuda => tree_sitter_cuda::LANGUAGE.into(),
            Self::Ada => tree_sitter_ada::LANGUAGE.into(),
            Self::Pascal => tree_sitter_pascal::LANGUAGE.into(),
            Self::GdScript => tree_sitter_gdscript::LANGUAGE.into(),
            Self::Odin => tree_sitter_odin::LANGUAGE.into(),
            Self::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Self::Proto => tree_sitter_proto::LANGUAGE.into(),
            Self::Vhdl => tree_sitter_vhdl::LANGUAGE.into(),
            Self::CommonLisp => tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into(),
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
            // CUDA is C++ with kernels: the same tags and patterns.
            Self::Cpp | Self::Cuda => (
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
            Self::Bash => (
                "",
                r#"
(function_definition name: (word) @name) @definition.function
(command name: (command_name (word) @name)) @reference.call
(command name: (command_name (word) @_c) argument: (word) @import_rel (#any-of? @_c "source" "."))
"#,
            ),
            Self::Lua => (tree_sitter_lua::TAGS_QUERY, ""),
            Self::Scala => (
                "",
                r"
(class_definition name: (_) @name) @definition.class
(object_definition name: (_) @name) @definition.class
(trait_definition name: (_) @name) @definition.interface
(enum_definition name: (_) @name) @definition.class
(function_definition name: (_) @name) @definition.function
(function_declaration name: (_) @name) @definition.function
(import_declaration) @import
(extends_clause type: (_) @extends)
(call_expression function: (identifier) @name) @reference.call
",
            ),
            Self::Swift => (
                tree_sitter_swift::TAGS_QUERY,
                "(call_expression . (simple_identifier) @name) @reference.call",
            ),
            Self::Kotlin => (
                "",
                r"
(class_declaration name: (_) @name) @definition.class
(object_declaration name: (_) @name) @definition.class
(function_declaration name: (_) @name) @definition.function
(import (qualified_identifier) @import)
(delegation_specifier (constructor_invocation (user_type (identifier) @extends)))
(call_expression (identifier) @name) @reference.call
",
            ),
            Self::Haskell => (
                "",
                r"
(function name: (variable) @name) @definition.function
(data_type name: (name) @name) @definition.class
(class name: (name) @name) @definition.interface
(import module: (module) @import)
(apply function: (variable) @name) @reference.call
",
            ),
            Self::Elixir => (tree_sitter_elixir::TAGS_QUERY, ""),
            Self::Ocaml => (tree_sitter_ocaml::TAGS_QUERY, ""),
            Self::Zig => (
                "",
                r#"
(function_declaration name: (identifier) @name) @definition.function
(variable_declaration (identifier) @name (struct_declaration)) @definition.class
(variable_declaration (identifier) @name (enum_declaration)) @definition.class
(variable_declaration (identifier) @name (union_declaration)) @definition.class
(builtin_function (builtin_identifier) @_b (arguments (string (string_content) @import_rel)) (#eq? @_b "@import") (#match? @import_rel "\\.zig$"))
(builtin_function (builtin_identifier) @_b (arguments (string (string_content) @import)) (#eq? @_b "@import") (#not-match? @import "\\.zig$"))
(call_expression function: (identifier) @name) @reference.call
"#,
            ),
            Self::Julia => (
                "",
                r"
(struct_definition (type_head (identifier) @name)) @definition.class
(abstract_definition (type_head (identifier) @name)) @definition.class
(function_definition (signature (call_expression . (identifier) @name))) @definition.function
(macro_definition (signature (call_expression . (identifier) @name))) @definition.macro
(using_statement (identifier) @import)
",
            ),
            Self::R => (tree_sitter_r::TAGS_QUERY, ""),
            Self::Dart => (tree_sitter_dart::TAGS_QUERY, ""),
            Self::Erlang => (
                "",
                r"
(fun_decl . (function_clause name: (atom) @name)) @definition.function
(call expr: (atom) @name) @reference.call
(pp_include file: (string) @import_rel)
",
            ),
            Self::Elm => (tree_sitter_elm::TAGS_QUERY, ""),
            Self::Nix => (
                "",
                r#"
(binding attrpath: (attrpath . (identifier) @name) expression: (function_expression)) @definition.function
(apply_expression function: (variable_expression name: (identifier) @_i) argument: (path_expression) @import_rel (#eq? @_i "import"))
(apply_expression function: (variable_expression name: (identifier) @name)) @reference.call
"#,
            ),
            Self::Hcl => (
                "",
                r#"
(block (identifier) @_k (string_lit) (string_lit (template_literal) @name) (#any-of? @_k "resource" "data")) @definition.class
(block (identifier) @_k . (string_lit (template_literal) @name) . (block_start) (#any-of? @_k "module" "variable" "output")) @definition.class
(attribute (identifier) @_s (expression (literal_value (string_lit (template_literal) @import_rel))) (#eq? @_s "source") (#match? @import_rel "^\\."))
"#,
            ),
            Self::Solidity => (tree_sitter_solidity::TAGS_QUERY, ""),
            Self::Fortran => (
                "",
                r"
(subroutine (subroutine_statement name: (name) @name)) @definition.function
(function (function_statement name: (name) @name)) @definition.function
(subroutine_call subroutine: (identifier) @name) @reference.call
(call_expression . (identifier) @name) @reference.call
",
            ),
            Self::Gleam => (
                tree_sitter_gleam::TAGS_QUERY,
                "(function_call function: (identifier) @name) @reference.call",
            ),
            Self::Groovy => (
                "",
                r"
(class_declaration name: (identifier) @name) @definition.class
(method_declaration name: (identifier) @name) @definition.method
(function_definition name: (identifier) @name) @definition.function
(import_declaration) @import
(superclass (type_identifier) @extends)
(method_invocation name: (identifier) @name) @reference.call
",
            ),
            Self::ObjC => (
                "",
                r"
(class_interface . (identifier) @name) @definition.class
(class_implementation . (identifier) @name) @definition.class
(protocol_declaration . (identifier) @name) @definition.interface
(method_definition (method_type) . (identifier) @name) @definition.method
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(class_interface superclass: (identifier) @extends)
(preproc_include path: (_) @import)
(module_import path: (_) @import)
(message_expression method: (identifier) @name) @reference.call
(call_expression function: (identifier) @name) @reference.call
",
            ),
            Self::PowerShell => (
                "",
                r#"
(function_statement (function_name) @name) @definition.function
(class_statement . (simple_name) @name) @definition.class
(class_method_definition (simple_name) @name) @definition.method
(command command_name: (command_name) @name) @reference.call
(command (command_invokation_operator) command_name: (command_name_expr (command_name) @import_rel) (#match? @import_rel "\\.ps(m)?1$"))
"#,
            ),
            Self::FSharp => (tree_sitter_fsharp::TAGS_QUERY, ""),
            Self::D => (
                "",
                r"
(class_declaration (class) . (identifier) @name) @definition.class
(struct_declaration (struct) . (identifier) @name) @definition.class
(function_declaration (identifier) @name) @definition.function
(import_declaration (imported (module_fqn) @import))
(base_class (identifier) @extends)
(call_expression . (identifier) @name) @reference.call
",
            ),
            Self::Verilog => (
                "",
                r"
(module_header (simple_identifier) @name) @definition.class
(class_declaration (class_identifier) @name) @definition.class
(function_body_declaration (function_identifier) @name) @definition.function
(task_body_declaration (task_identifier) @name) @definition.function
(module_instantiation . (simple_identifier) @name) @reference.class
",
            ),
            Self::Ada => (
                "",
                r"
(subprogram_body (procedure_specification name: (identifier) @name)) @definition.function
(subprogram_body (function_specification name: (identifier) @name)) @definition.function
(procedure_call_statement name: (identifier) @name) @reference.call
(with_clause (selected_component) @import)
(with_clause (identifier) @import)
",
            ),
            Self::Pascal => (
                "",
                r"
(declType name: (identifier) @name type: (declClass)) @definition.class
(defProc header: (declProc name: (identifier) @name)) @definition.function
(defProc header: (declProc name: (genericDot rhs: (identifier) @name))) @definition.method
(declClass parent: (typeref (identifier) @extends))
(declUses (moduleName) @import)
",
            ),
            Self::GdScript => (
                "",
                r#"
(function_definition name: (name) @name) @definition.function
(class_definition name: (name) @name) @definition.class
(class_name_statement name: (name) @name) @definition.class
(extends_statement (type (identifier) @extends))
(call (identifier) @_p arguments: (arguments (string) @import) (#any-of? @_p "preload" "load"))
(call (identifier) @name) @reference.call
"#,
            ),
            Self::Odin => (
                "",
                r"
(procedure_declaration . (identifier) @name) @definition.function
(struct_declaration . (identifier) @name) @definition.class
(enum_declaration . (identifier) @name) @definition.class
(union_declaration . (identifier) @name) @definition.class
(import_declaration (string (string_content) @import))
(call_expression function: (identifier) @name) @reference.call
",
            ),
            Self::Sql => (
                "",
                r"
(create_table (object_reference name: (identifier) @name)) @definition.class
(create_view (object_reference name: (identifier) @name)) @definition.class
(create_function (object_reference name: (identifier) @name)) @definition.function
",
            ),
            Self::Proto => (
                "",
                r"
(message (message_name (identifier) @name)) @definition.class
(enum (enum_name (identifier) @name)) @definition.class
(service (service_name (identifier) @name)) @definition.interface
(rpc (rpc_name (identifier) @name)) @definition.method
(import path: (string) @import)
",
            ),
            Self::Vhdl => (
                "",
                r"
(entity_declaration entity: (identifier) @name) @definition.class
(use_clause (selected_name_list (selected_name) @import))
",
            ),
            Self::CommonLisp => (
                "",
                r#"
(defun (defun_header function_name: (sym_lit) @name)) @definition.function
(list_lit . (sym_lit) @_d . (sym_lit) @name (#eq? @_d "defclass")) @definition.class
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
        static CACHE: [OnceLock<Option<Query>>; ALL.len()] = [const { OnceLock::new() }; ALL.len()];
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
