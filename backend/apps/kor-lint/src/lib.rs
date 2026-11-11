//! `kor_lint` (§32.2, ADR 0062): a source scanner for the performance rules clippy cannot see.
//!
//! - [`Rule::BlockingInAsync`]: blocking file, process, network or sleep calls inside `async`
//!   code (they stall a tokio worker). Work handed to `spawn_blocking` / `block_in_place` /
//!   a thread is fine.
//! - [`Rule::BlockingInUi`]: the render path is sacred: no I/O or JSON parsing in a function that
//!   draws a ratatui `Frame`, and no synchronous `#[tauri::command]` (those run on the main thread).
//! - [`Rule::UnboundedChannel`]: `unbounded_channel()`, `mpsc::unbounded()`, std `mpsc::channel()`.
//! - [`Rule::UncachedRegex`]: `Regex::new("literal")` outside a `static` / `LazyLock` / `OnceLock`.
//!
//! Test code (`#[cfg(test)]`, `#[test]`, `tests/`, `benches/`, `examples/`) is not scanned. A
//! finding is silenced by `// kor-lint: allow(<rule>) <why>` on its line or the line above. Existing
//! findings are recorded in a baseline; CI fails only when a file gains findings (a ratchet).

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    BlockingInAsync,
    BlockingInUi,
    UnboundedChannel,
    UncachedRegex,
}

pub const RULES: [Rule; 4] = [
    Rule::BlockingInAsync,
    Rule::BlockingInUi,
    Rule::UnboundedChannel,
    Rule::UncachedRegex,
];

impl Rule {
    pub fn id(self) -> &'static str {
        match self {
            Self::BlockingInAsync => "blocking_in_async",
            Self::BlockingInUi => "blocking_in_ui",
            Self::UnboundedChannel => "unbounded_channel",
            Self::UncachedRegex => "uncached_regex",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        RULES.into_iter().find(|r| r.id() == s)
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    /// As given to the scanner, `/`-separated.
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: kor_lint::{}: {}",
            self.file, self.line, self.rule, self.message
        )
    }
}

/// Calls that block the calling thread (resolved paths, prefix match).
const BLOCKING: [&str; 9] = [
    "std::fs::",
    "std::thread::sleep",
    "std::process::Command",
    "std::net::TcpStream::connect",
    "std::net::TcpListener::bind",
    "std::net::UdpSocket::bind",
    "std::io::stdin",
    "reqwest::blocking::",
    "tree_sitter::",
];

/// Extra calls forbidden on a render path: parsing is work for a worker thread.
const UI_ONLY: [&str; 4] = [
    "serde_json::from_str",
    "serde_json::from_slice",
    "serde_json::from_reader",
    "toml::from_str",
];

/// Methods that block whatever their receiver is.
const BLOCKING_METHODS: [&str; 5] = [
    "blocking_recv",
    "blocking_send",
    "blocking_lock",
    "blocking_read",
    "blocking_write",
];

/// Calls whose closure argument runs off the async worker.
const OFFLOAD: [&str; 4] = ["spawn_blocking", "block_in_place", "thread::spawn", "scope"];

/// Calls whose argument runs once and is cached.
const CACHE_CTORS: [&str; 3] = ["LazyLock::new", "Lazy::new", "lazy_static"];
const CACHE_METHODS: [&str; 4] = [
    "get_or_init",
    "get_or_try_init",
    "call_once",
    "get_or_insert_with",
];

#[derive(Clone, Copy, Default)]
struct Ctx {
    is_async: bool,
    ui: bool,
    cached: bool,
}

struct Scanner<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    /// Local name → full path, from every `use` in the file.
    imports: HashMap<String, String>,
    ctx: Ctx,
    out: Vec<Finding>,
}

fn has_attr(attrs: &[syn::Attribute], pred: impl Fn(&str) -> bool) -> bool {
    attrs.iter().any(|a| {
        let text = quote_attr(a);
        pred(&text)
    })
}

/// The attribute as compact text, `cfg(test)`, `tauri::command`, …
fn quote_attr(a: &syn::Attribute) -> String {
    use quote_lite::ToCompact;
    a.meta.compact()
}

/// `ToTokens` → text without spaces (enough to match attribute and type names).
mod quote_lite {
    use quote::ToTokens;

    pub trait ToCompact {
        fn compact(&self) -> String;
    }

    impl<T: ToTokens> ToCompact for T {
        fn compact(&self) -> String {
            self.to_token_stream()
                .to_string()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect()
        }
    }
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    has_attr(attrs, |a| {
        a == "test"
            || a.ends_with("::test")
            || (a.starts_with("cfg(") && a.contains("test") && !a.contains("not(test"))
    })
}

fn collect_use(tree: &syn::UseTree, prefix: &str, out: &mut HashMap<String, String>) {
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}::{name}")
        }
    };
    match tree {
        syn::UseTree::Path(p) => collect_use(&p.tree, &join(&p.ident.to_string()), out),
        syn::UseTree::Name(n) => {
            let name = n.ident.to_string();
            if name == "self" {
                if let Some(last) = prefix.rsplit("::").next() {
                    out.insert(last.to_owned(), prefix.to_owned());
                }
            } else {
                out.insert(name.clone(), join(&name));
            }
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string(), join(&r.ident.to_string()));
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

struct Uses(HashMap<String, String>);

impl<'ast> Visit<'ast> for Uses {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        collect_use(&u.tree, "", &mut self.0);
    }
}

fn path_text(p: &syn::Path) -> String {
    p.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

impl Scanner<'_> {
    fn resolve(&self, p: &syn::Path) -> String {
        let text = path_text(p);
        let (first, rest) = text.split_once("::").unwrap_or((&text, ""));
        match self.imports.get(first) {
            Some(full) if rest.is_empty() => full.clone(),
            Some(full) => format!("{full}::{rest}"),
            None => text,
        }
    }

    fn allowed(&self, line: usize, rule: Rule) -> bool {
        let tag = format!("kor-lint: allow({})", rule.id());
        let text = |l: usize| l.checked_sub(1).and_then(|i| self.lines.get(i)).copied();
        // On the line itself, or alone on the line above (a trailing comment covers only its line).
        text(line).is_some_and(|s| s.contains(&tag))
            || text(line - 1).is_some_and(|s| s.trim_start().starts_with("//") && s.contains(&tag))
    }

    fn report(&mut self, rule: Rule, span: proc_macro2::Span, message: String) {
        let line = span.start().line;
        if !self.allowed(line, rule) {
            self.out.push(Finding {
                rule,
                file: self.file.to_owned(),
                line,
                message,
            });
        }
    }

    fn with<R>(&mut self, ctx: Ctx, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.ctx;
        self.ctx = ctx;
        let r = f(self);
        self.ctx = saved;
        r
    }

    fn check_call(
        &mut self,
        path: &str,
        args: &syn::punctuated::Punctuated<syn::Expr, syn::Token![,]>,
        span: proc_macro2::Span,
    ) {
        if (self.ctx.is_async || self.ctx.ui)
            && let Some(b) = BLOCKING.iter().find(|b| path.starts_with(*b))
        {
            let rule = if self.ctx.ui {
                Rule::BlockingInUi
            } else {
                Rule::BlockingInAsync
            };
            let place = if self.ctx.ui {
                "a render path"
            } else {
                "async code"
            };
            self.report(
                rule,
                span,
                format!("`{path}` blocks ({b}) in {place}; use tokio or spawn_blocking"),
            );
        }
        if self.ctx.ui && UI_ONLY.iter().any(|u| path.starts_with(u)) {
            self.report(
                Rule::BlockingInUi,
                span,
                format!("`{path}` parses on a render path; parse on a worker and send the result"),
            );
        }
        let last = path.rsplit("::").next().unwrap_or(path);
        let unbounded = matches!(last, "unbounded_channel" | "unbounded")
            || (last == "channel"
                && args.is_empty()
                && (path.contains("mpsc") || path.contains("crossbeam")));
        if unbounded {
            self.report(Rule::UnboundedChannel, span, format!("`{path}()` is unbounded; use a bounded channel (backpressure) or say why with an allow comment"));
        }
        let regex = ["Regex::new", "RegexBuilder::new", "RegexSet::new"]
            .iter()
            .any(|r| path.ends_with(r));
        if regex && !self.ctx.cached && args.first().is_some_and(literal) {
            self.report(
                Rule::UncachedRegex,
                span,
                format!("`{path}` compiles a literal pattern on every call; keep it in a LazyLock"),
            );
        }
    }
}

/// A string literal, or an array of them (`RegexSet::new([..])`).
fn literal(e: &syn::Expr) -> bool {
    match e {
        syn::Expr::Lit(l) => matches!(l.lit, syn::Lit::Str(_)),
        syn::Expr::Array(a) => !a.elems.is_empty() && a.elems.iter().all(literal),
        syn::Expr::Reference(r) => literal(&r.expr),
        syn::Expr::Group(g) => literal(&g.expr),
        syn::Expr::Paren(p) => literal(&p.expr),
        _ => false,
    }
}

fn draws_frame(sig: &syn::Signature) -> bool {
    use quote_lite::ToCompact;
    sig.inputs.iter().any(|i| match i {
        syn::FnArg::Typed(t) => {
            let ty = t.ty.compact();
            ty.contains("Frame<") || ty.ends_with("Frame")
        }
        syn::FnArg::Receiver(_) => false,
    })
}

impl<'ast> Visit<'ast> for Scanner<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !is_test(&m.attrs) {
            visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if is_test(&f.attrs) {
            return;
        }
        let sync_command = f.sig.asyncness.is_none()
            && has_attr(&f.attrs, |a| a == "tauri::command" || a == "command");
        if sync_command {
            self.report(Rule::BlockingInUi, f.sig.ident.span(), format!("`#[tauri::command] fn {}` is synchronous and runs on the main thread; make it async", f.sig.ident));
        }
        let ctx = Ctx {
            is_async: f.sig.asyncness.is_some(),
            ui: draws_frame(&f.sig),
            cached: false,
        };
        self.with(ctx, |s| visit::visit_item_fn(s, f));
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if is_test(&f.attrs) {
            return;
        }
        let ctx = Ctx {
            is_async: f.sig.asyncness.is_some(),
            ui: draws_frame(&f.sig),
            cached: false,
        };
        self.with(ctx, |s| visit::visit_impl_item_fn(s, f));
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        let ctx = Ctx {
            is_async: f.sig.asyncness.is_some(),
            ui: draws_frame(&f.sig),
            cached: false,
        };
        self.with(ctx, |s| visit::visit_trait_item_fn(s, f));
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        let ctx = Ctx {
            cached: true,
            ..self.ctx
        };
        self.with(ctx, |me| visit::visit_item_static(me, s));
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        let ctx = Ctx {
            cached: true,
            ..self.ctx
        };
        self.with(ctx, |me| visit::visit_item_const(me, c));
    }

    fn visit_expr_async(&mut self, a: &'ast syn::ExprAsync) {
        let ctx = Ctx {
            is_async: true,
            ..self.ctx
        };
        self.with(ctx, |s| visit::visit_expr_async(s, a));
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        let ctx = if c.asyncness.is_some() {
            Ctx {
                is_async: true,
                ..self.ctx
            }
        } else {
            self.ctx
        };
        self.with(ctx, |s| visit::visit_expr_closure(s, c));
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let path = match &*call.func {
            syn::Expr::Path(p) => Some(self.resolve(&p.path)),
            _ => None,
        };
        if let Some(path) = &path {
            self.check_call(path, &call.args, call.func.span());
        }
        self.visit_expr(&call.func);
        let path = path.unwrap_or_default();
        let ctx = if OFFLOAD.iter().any(|o| path.ends_with(o)) {
            Ctx {
                is_async: false,
                ui: false,
                ..self.ctx
            }
        } else if CACHE_CTORS.iter().any(|c| path.ends_with(c)) {
            Ctx {
                cached: true,
                ..self.ctx
            }
        } else {
            self.ctx
        };
        self.with(ctx, |s| {
            for a in &call.args {
                s.visit_expr(a);
            }
        });
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let name = m.method.to_string();
        if (self.ctx.is_async || self.ctx.ui) && BLOCKING_METHODS.contains(&name.as_str()) {
            let rule = if self.ctx.ui {
                Rule::BlockingInUi
            } else {
                Rule::BlockingInAsync
            };
            self.report(
                rule,
                m.method.span(),
                format!("`.{name}()` blocks the thread"),
            );
        }
        self.visit_expr(&m.receiver);
        // `std::thread::Builder::new().spawn(|| ..)` / `scope.spawn(|| ..)`: a sync closure handed to
        // `spawn` runs on its own thread.
        let sync_spawn = name == "spawn"
            && m.args
                .first()
                .is_some_and(|a| matches!(a, syn::Expr::Closure(c) if c.asyncness.is_none()));
        let ctx = if sync_spawn || name == "spawn_blocking" {
            Ctx {
                is_async: false,
                ui: false,
                ..self.ctx
            }
        } else if CACHE_METHODS.contains(&name.as_str()) {
            Ctx {
                cached: true,
                ..self.ctx
            }
        } else {
            self.ctx
        };
        self.with(ctx, |s| {
            for a in &m.args {
                s.visit_expr(a);
            }
        });
    }
}

/// Findings in one file's source. `file` is only the label printed.
///
/// # Errors
/// When the source does not parse.
pub fn scan_source(file: &str, src: &str) -> Result<Vec<Finding>, String> {
    let ast = syn::parse_file(src).map_err(|e| format!("{file}: {e}"))?;
    let mut uses = Uses(HashMap::new());
    uses.visit_file(&ast);
    let mut s = Scanner {
        file,
        lines: src.lines().collect(),
        imports: uses.0,
        ctx: Ctx::default(),
        out: Vec::new(),
    };
    s.visit_file(&ast);
    s.out
        .sort_by(|a, b| a.line.cmp(&b.line).then(a.rule.cmp(&b.rule)));
    Ok(s.out)
}

const SKIP_DIRS: [&str; 7] = [
    "target",
    "node_modules",
    "tests",
    "benches",
    "examples",
    "gen",
    "fixtures",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if p.is_dir() {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Every finding under `roots` (files or directories), plus files that did not parse.
pub fn scan_paths(roots: &[PathBuf]) -> (Vec<Finding>, Vec<String>) {
    let mut files = Vec::new();
    for r in roots {
        if r.is_dir() {
            rust_files(r, &mut files);
        } else {
            files.push(r.clone());
        }
    }
    let (mut found, mut errors) = (Vec::new(), Vec::new());
    for f in files {
        let label = f.to_string_lossy().replace('\\', "/");
        match std::fs::read_to_string(&f) {
            Ok(src) => match scan_source(&label, &src) {
                Ok(v) => found.extend(v),
                Err(e) => errors.push(e),
            },
            Err(e) => errors.push(format!("{label}: {e}")),
        }
    }
    (found, errors)
}

/// Allowed findings per rule and file.
pub type Baseline = BTreeMap<String, BTreeMap<String, usize>>;

pub fn tally(findings: &[Finding]) -> Baseline {
    let mut b = Baseline::new();
    for f in findings {
        *b.entry(f.rule.id().to_owned())
            .or_default()
            .entry(f.file.clone())
            .or_default() += 1;
    }
    b
}

/// `(rule, file, now, allowed)`.
pub type Change = (String, String, usize, usize);

/// Every file that gained findings, and every file that lost some (the baseline can shrink).
pub fn compare(baseline: &Baseline, findings: &[Finding]) -> (Vec<Change>, Vec<Change>) {
    let now = tally(findings);
    let (mut worse, mut better) = (Vec::new(), Vec::new());
    for (rule, files) in &now {
        for (file, n) in files {
            let allowed = baseline
                .get(rule)
                .and_then(|f| f.get(file))
                .copied()
                .unwrap_or(0);
            if *n > allowed {
                worse.push((rule.clone(), file.clone(), *n, allowed));
            }
        }
    }
    for (rule, files) in baseline {
        for (file, allowed) in files {
            let n = now
                .get(rule)
                .and_then(|f| f.get(file))
                .copied()
                .unwrap_or(0);
            if n < *allowed {
                better.push((rule.clone(), file.clone(), n, *allowed));
            }
        }
    }
    (worse, better)
}

/// # Errors
/// When the JSON is not a `{ rule: { file: count } }` object.
pub fn parse_baseline(text: &str) -> Result<Baseline, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

pub fn render_baseline(b: &Baseline) -> String {
    serde_json::to_string_pretty(b).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(src: &str) -> Vec<(Rule, usize)> {
        scan_source("t.rs", src)
            .unwrap()
            .into_iter()
            .map(|f| (f.rule, f.line))
            .collect()
    }

    #[test]
    fn blocking_calls_in_async_code_are_found() {
        let src = "use std::fs;\nuse std::thread;\nasync fn a() {\n    let _ = fs::read(\"x\");\n    thread::sleep(d);\n}\nfn sync() { let _ = std::fs::read(\"x\"); }\n";
        assert_eq!(
            rules(src),
            [(Rule::BlockingInAsync, 4), (Rule::BlockingInAsync, 5)]
        );
    }

    #[test]
    fn async_blocks_and_closures_count_and_offloaded_work_does_not() {
        let src = "fn f() {\n    tokio::spawn(async move { std::fs::read(\"x\") });\n}\nasync fn g() {\n    tokio::task::spawn_blocking(move || std::fs::read(\"x\")).await;\n    std::thread::Builder::new().spawn(move || std::fs::read(\"x\"));\n    let _ = rx.blocking_recv();\n}\n";
        assert_eq!(
            rules(src),
            [(Rule::BlockingInAsync, 2), (Rule::BlockingInAsync, 7)]
        );
    }

    #[test]
    fn std_process_is_blocking_but_tokio_process_is_not() {
        let src = "use std::process::Command;\nasync fn a() { Command::new(\"git\").output(); }\nmod m { use tokio::process::Command as C; async fn b() { C::new(\"git\").output().await; } }\n";
        assert_eq!(rules(src), [(Rule::BlockingInAsync, 2)]);
    }

    #[test]
    fn render_paths_and_sync_tauri_commands_are_ui_findings() {
        let src = "fn draw(frame: &mut Frame<'_>, s: &S) {\n    let v: V = serde_json::from_str(s).unwrap();\n    std::fs::read(\"x\");\n}\n#[tauri::command]\nfn sync_cmd() {}\n#[tauri::command]\nasync fn ok_cmd() {}\n";
        assert_eq!(
            rules(src),
            [
                (Rule::BlockingInUi, 2),
                (Rule::BlockingInUi, 3),
                (Rule::BlockingInUi, 6)
            ]
        );
    }

    #[test]
    fn unbounded_channels_are_found_bounded_are_not() {
        let src = "use std::sync::mpsc;\nfn f() {\n    let (a, b) = mpsc::channel();\n    let (c, d) = tokio::sync::mpsc::unbounded_channel::<u8>();\n    let (e, g) = tokio::sync::mpsc::channel(16);\n    let (h, i) = mpsc::sync_channel(4);\n    let (j, k) = futures::channel::mpsc::unbounded();\n}\n";
        assert_eq!(
            rules(src),
            [
                (Rule::UnboundedChannel, 3),
                (Rule::UnboundedChannel, 4),
                (Rule::UnboundedChannel, 7)
            ]
        );
    }

    #[test]
    fn literal_regexes_must_be_cached() {
        let src = "use regex::Regex;\nstatic A: LazyLock<Regex> = LazyLock::new(|| Regex::new(\"a+\").unwrap());\nfn f(user: &str) {\n    let b = Regex::new(\"b+\").unwrap();\n    let c = Regex::new(user).unwrap();\n    let d = CELL.get_or_init(|| Regex::new(\"d\").unwrap());\n    let e = regex::RegexSet::new([\"x\", \"y\"]);\n}\n";
        assert_eq!(
            rules(src),
            [(Rule::UncachedRegex, 4), (Rule::UncachedRegex, 7)]
        );
    }

    #[test]
    fn tests_and_allow_comments_are_skipped() {
        let src = "#[cfg(test)]\nmod tests { async fn a() { std::fs::read(\"x\"); } }\n#[test]\nfn t() { let _ = std::sync::mpsc::channel::<u8>(); }\nasync fn b() {\n    // kor-lint: allow(blocking_in_async) tiny config file, read once at start\n    std::fs::read(\"x\");\n    std::fs::read(\"y\"); // kor-lint: allow(blocking_in_async) same\n    std::fs::read(\"z\");\n}\n";
        assert_eq!(rules(src), [(Rule::BlockingInAsync, 9)]);
    }

    #[test]
    fn baseline_is_a_ratchet() {
        let f = |file: &str, line| Finding {
            rule: Rule::UnboundedChannel,
            file: file.into(),
            line,
            message: String::new(),
        };
        let base = tally(&[f("a.rs", 1), f("a.rs", 2), f("b.rs", 1)]);
        let text = render_baseline(&base);
        assert_eq!(parse_baseline(&text).unwrap(), base);
        // Same counts: clean. One more in a.rs and a new file: both worse. b.rs fixed: better.
        assert_eq!(
            compare(&base, &[f("a.rs", 1), f("a.rs", 2), f("b.rs", 1)]),
            (vec![], vec![])
        );
        let (worse, better) = compare(
            &base,
            &[f("a.rs", 1), f("a.rs", 2), f("a.rs", 3), f("c.rs", 1)],
        );
        assert_eq!(
            worse,
            [
                ("unbounded_channel".into(), "a.rs".into(), 3, 2),
                ("unbounded_channel".into(), "c.rs".into(), 1, 0)
            ]
        );
        assert_eq!(better, [("unbounded_channel".into(), "b.rs".into(), 0, 1)]);
    }

    #[test]
    fn rule_ids_round_trip() {
        for r in RULES {
            assert_eq!(Rule::parse(r.id()), Some(r));
        }
        assert_eq!(Rule::parse("nope"), None);
    }
}
