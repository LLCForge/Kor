//! `kor dead-code [--prune]` (spec §15.14): symbols of the project graph that no entry point
//! reaches. Entry points are data (`deadcode-rules.toml`, extended by `.kor/dead-code.toml`):
//! `main`, tests, HTTP routes, cron/trigger handlers, exported public API, DI/reflection
//! registrations, framework callbacks — per framework (Next.js, Spring, Django, Flask/FastAPI,
//! Laravel, Rails, ASP.NET, Go, Rust library crates).
//!
//! A symbol the graph cannot reach whose name still appears somewhere the graph could not
//! resolve (outside the other candidates: a receiver call, a string, a template, a config
//! file), or that lives in a file using reflection, is `Ambiguous` and listed separately. The
//! rest is crossed with git: unreachable and unchanged for 180+ days = high confidence.
//!
//! `--prune` writes the deletion on a new branch in a separate worktree, runs the project's
//! compiler/type-checker and tests there (in the configured sandbox), restores files the
//! checker complains about, commits, and only after the user confirms pushes and opens a PR
//! through `forge::create_pull_request` with every removed item and its reason in the body.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use globset::{Glob, GlobSet, GlobSetBuilder};
use kor_graph::{Graph, NodeKind};
use kor_i18n::{Localizer, args};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use super::code;
use super::git;
use crate::analysis::{Finding, Report, Severity};
use crate::forge::{self, ForgeError, PrRef, PrRequest};

const BUILTIN: &str = include_str!("deadcode-rules.toml");
/// Unchanged for at least this many days + unreachable = high confidence.
pub const STALE_DAYS: i64 = 180;
/// Text files whose identifiers can reference code (templates, configs).
const REF_EXTS: [&str; 15] = [
    "html", "htm", "vue", "svelte", "jinja", "j2", "hbs", "ejs", "erb", "cshtml", "razor", "yaml",
    "yml", "json", "xml",
];

static REFLECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bgetattr\s*\(|__getattr__|\bimportlib\b|\bReflect\.|Class\.forName|\breflect\.|\.send\(:|method_missing|globals\(\)\[|\beval\s*\(|\bdlsym\b|\.GetMethod\(|Activator\.CreateInstance|Type\.GetType\(|\bcall_user_func")
        .expect("valid regex")
});
static IDENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*").expect("valid regex"));

#[derive(Debug, Clone, Deserialize)]
struct When {
    file: String,
    #[serde(default)]
    contains: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDef {
    name: String,
    #[serde(default)]
    framework: String,
    kind: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    whole_file: bool,
    names: Option<String>,
    lines: Option<String>,
    module: Option<String>,
    registrations: Option<String>,
    #[serde(default)]
    lib_crate: bool,
    #[serde(default)]
    trait_impls: bool,
    unless: Option<String>,
    #[serde(default)]
    when: Vec<When>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<RuleDef>,
    #[serde(default)]
    ignore: Vec<String>,
}

/// One compiled entry-point rule.
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub framework: String,
    /// `main`, `route`, `cron`, `test`, `public-api`, `di`, `callback`.
    pub kind: String,
    files: Option<GlobSet>,
    exclude: Option<GlobSet>,
    whole_file: bool,
    names: Option<Regex>,
    lines: Option<Regex>,
    module: Option<Regex>,
    registrations: Option<Regex>,
    lib_crate: bool,
    trait_impls: bool,
    unless: Option<Regex>,
    when: Vec<When>,
}

#[derive(Debug, Clone)]
pub struct Rules {
    pub rules: Vec<Rule>,
    ignore: Option<GlobSet>,
    /// Problems with `.kor/dead-code.toml` (invalid rules are skipped, never widened).
    pub notes: Vec<String>,
}

fn globs(list: &[String]) -> Result<Option<GlobSet>, String> {
    if list.is_empty() {
        return Ok(None);
    }
    let mut b = GlobSetBuilder::new();
    for g in list {
        b.add(Glob::new(g).map_err(|e| format!("{g}: {e}"))?);
    }
    b.build().map(Some).map_err(|e| e.to_string())
}

fn re(s: Option<&String>) -> Result<Option<Regex>, String> {
    s.map(|s| {
        RegexBuilder::new(s)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| format!("{s}: {e}"))
    })
    .transpose()
}

fn compile(d: &RuleDef) -> Result<Rule, String> {
    Ok(Rule {
        name: d.name.clone(),
        framework: d.framework.clone(),
        kind: d.kind.clone(),
        files: globs(&d.files)?,
        exclude: globs(&d.exclude)?,
        whole_file: d.whole_file,
        names: re(d.names.as_ref())?,
        lines: re(d.lines.as_ref())?,
        module: re(d.module.as_ref())?,
        registrations: re(d.registrations.as_ref())?,
        lib_crate: d.lib_crate,
        trait_impls: d.trait_impls,
        unless: re(d.unless.as_ref())?,
        when: d.when.clone(),
    })
}

impl Rules {
    /// Built-in rules plus `<root>/.kor/dead-code.toml`.
    ///
    /// # Panics
    /// Only if the built-in rule file does not parse (a unit test keeps it valid).
    pub fn load(root: &Path) -> Self {
        let mut notes = Vec::new();
        let builtin: RuleFile = toml::from_str(BUILTIN).expect("built-in dead-code rules parse");
        let user: RuleFile = match std::fs::read_to_string(root.join(".kor").join("dead-code.toml"))
        {
            Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
                notes.push(format!(".kor/dead-code.toml: {e}"));
                RuleFile::default()
            }),
            Err(_) => RuleFile::default(),
        };
        let mut rules = Vec::new();
        for d in builtin.rule.iter().chain(&user.rule) {
            match compile(d) {
                Ok(r) => rules.push(r),
                Err(e) => notes.push(format!("rule `{}`: {e}", d.name)),
            }
        }
        let ignore = globs(&user.ignore).unwrap_or_else(|e| {
            notes.push(format!(".kor/dead-code.toml ignore: {e}"));
            None
        });
        Self {
            rules,
            ignore,
            notes,
        }
    }

    fn ignored(&self, rel: &str) -> bool {
        self.ignore.as_ref().is_some_and(|g| g.is_match(rel))
    }
}

/// Per-file facts the rules look at, read once.
struct FileCtx {
    text: String,
    lines: Vec<String>,
    lib_crate: bool,
    trait_ranges: Vec<(u32, u32)>,
}

struct Ctx<'a> {
    root: &'a Path,
    files: HashMap<String, Option<FileCtx>>,
    when_cache: HashMap<(PathBuf, String), Option<String>>,
}

fn rust_trait_ranges(text: &str) -> Vec<(u32, u32)> {
    let mut p = tree_sitter::Parser::new();
    if p.set_language(&kor_graph::lang::Lang::Rust.language())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = p.parse(text, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.kind() == "impl_item" && n.child_by_field_name("trait").is_some() {
            let row = |r: usize| u32::try_from(r + 1).unwrap_or(u32::MAX);
            out.push((row(n.start_position().row), row(n.end_position().row)));
            continue;
        }
        for i in 0..n.named_child_count() {
            if let Some(c) = n.named_child(i) {
                stack.push(c);
            }
        }
    }
    out
}

fn is_lib_crate(root: &Path, rel: &str) -> bool {
    let mut dir = root.join(rel);
    while dir.pop() && dir.starts_with(root) {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            return dir.join("src/lib.rs").is_file()
                || std::fs::read_to_string(manifest).is_ok_and(|t| t.contains("[lib]"));
        }
    }
    false
}

impl Ctx<'_> {
    fn file(&mut self, rel: &str) -> Option<&FileCtx> {
        if !self.files.contains_key(rel) {
            let ctx = std::fs::read_to_string(self.root.join(rel))
                .ok()
                .map(|text| {
                    let rust = Path::new(rel).extension().is_some_and(|e| e == "rs");
                    FileCtx {
                        lines: text.lines().map(str::to_owned).collect(),
                        lib_crate: rust && is_lib_crate(self.root, rel),
                        trait_ranges: if rust {
                            rust_trait_ranges(&text)
                        } else {
                            Vec::new()
                        },
                        text,
                    }
                });
            self.files.insert(rel.to_owned(), ctx);
        }
        self.files.get(rel).and_then(Option::as_ref)
    }

    /// `when` conditions: the file exists in the directory of `rel` or an ancestor up to the root.
    fn when_ok(&mut self, rule: &Rule, rel: &str) -> bool {
        if rule.when.is_empty() {
            return true;
        }
        for w in &rule.when {
            let mut dir = self.root.join(rel);
            while dir.pop() && dir.starts_with(self.root) {
                let key = (dir.clone(), w.file.clone());
                let content = self
                    .when_cache
                    .entry(key)
                    .or_insert_with(|| {
                        std::fs::read_to_string(dir.join(&w.file))
                            .ok()
                            .map(|t| t.to_lowercase())
                    })
                    .clone();
                if let Some(t) = content {
                    if w.contains
                        .as_ref()
                        .is_none_or(|c| t.contains(&c.to_lowercase()))
                    {
                        return true;
                    }
                    break; // the nearest manifest decides
                }
            }
        }
        false
    }

    fn applies(&mut self, rule: &Rule, rel: &str) -> bool {
        if rule.files.as_ref().is_some_and(|g| !g.is_match(rel))
            || rule.exclude.as_ref().is_some_and(|g| g.is_match(rel))
        {
            return false;
        }
        if !self.when_ok(rule, rel) {
            return false;
        }
        let Some(f) = self.file(rel) else {
            return rule.unless.is_none() && !rule.lib_crate;
        };
        if rule.unless.as_ref().is_some_and(|u| u.is_match(&f.text)) {
            return false;
        }
        !rule.lib_crate || f.lib_crate
    }

    /// The first rule that makes symbol `name` at `span` in `rel` an entry point.
    fn symbol_entry(
        &mut self,
        rules: &Rules,
        rel: &str,
        name: &str,
        span: Option<(u32, u32)>,
    ) -> Option<usize> {
        for (i, r) in rules.rules.iter().enumerate() {
            let direct = r.whole_file || r.names.as_ref().is_some_and(|n| n.is_match(name));
            if !direct && r.lines.is_none() && !r.trait_impls {
                continue;
            }
            if !self.applies(r, rel) {
                continue;
            }
            if direct {
                return Some(i);
            }
            let Some(f) = self.file(rel) else { continue };
            let Some((start, _)) = span else { continue };
            if r.trait_impls
                && f.trait_ranges
                    .iter()
                    .any(|(a, b)| *a <= start && start <= *b)
            {
                return Some(i);
            }
            if let Some(lines) = &r.lines {
                let s = usize::try_from(start).unwrap_or(1).saturating_sub(1);
                // The definition line and the attributes/decorators/doc lines attached above it.
                let mut from = s.min(f.lines.len().saturating_sub(1));
                while from > 0 && s - from < 6 {
                    let above = f.lines[from - 1].trim_start();
                    if ["#[", "@", "[", "///", "//", "/**", "*", "#"]
                        .iter()
                        .any(|p| above.starts_with(p))
                    {
                        from -= 1;
                    } else {
                        break;
                    }
                }
                if f.lines
                    .get(from..=s.min(f.lines.len().saturating_sub(1)))
                    .is_some_and(|ls| ls.iter().any(|l| lines.is_match(l)))
                {
                    return Some(i);
                }
            }
        }
        None
    }

    fn module_entry(&mut self, rules: &Rules, rel: &str) -> Option<usize> {
        for (i, r) in rules.rules.iter().enumerate() {
            if !(r.whole_file || r.module.is_some()) || !self.applies(r, rel) {
                continue;
            }
            if r.whole_file {
                return Some(i);
            }
            let f = self.file(rel)?;
            if r.module.as_ref().is_some_and(|m| m.is_match(&f.text)) {
                return Some(i);
            }
        }
        None
    }
}

/// Files a `package.json` names as `main`, `module`, `bin` or `exports` (project-relative).
fn manifest_entries(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let walk = ignore::WalkBuilder::new(root)
        .max_depth(Some(5))
        .filter_entry(|e| e.file_name() != "node_modules")
        .build();
    for e in walk.flatten().filter(|e| e.file_name() == "package.json") {
        let Ok(v) = std::fs::read(e.path())
            .map(|b| serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default())
        else {
            continue;
        };
        let dir = e.path().parent().unwrap_or(root);
        let mut push = |s: &str| {
            let p = dir.join(s.trim_start_matches("./"));
            out.insert(super::rel(root, &p));
        };
        let mut strings = Vec::new();
        for key in ["main", "module", "bin", "exports", "types"] {
            collect_strings(&v[key], &mut strings);
        }
        for s in strings
            .iter()
            .filter(|s| s.starts_with('.') || s.contains('/') || s.contains('.'))
        {
            push(s);
        }
    }
    out
}

fn collect_strings(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Object(m) => m.values().for_each(|x| collect_strings(x, out)),
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// No entry point reaches it; `unchanged_days` from git.
    Unreachable,
    /// The name also appears in `count` places the graph could not resolve.
    NameElsewhere { count: u32 },
    /// Its file loads code by name (getattr, reflection, eval, …).
    Reflection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadItem {
    pub id: String,
    /// `function`, `struct`, `trait`.
    pub kind: String,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub end: u32,
    pub confidence: Confidence,
    pub reason: Reason,
    /// Days since the file last changed (git); `None` outside git.
    pub unchanged_days: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadReport {
    pub symbols: usize,
    pub entries: usize,
    /// Entry points per rule name.
    pub entry_rules: BTreeMap<String, usize>,
    pub dead: Vec<DeadItem>,
    pub ambiguous: Vec<DeadItem>,
    pub notes: Vec<String>,
}

fn candidate_kind(k: NodeKind) -> bool {
    matches!(k, NodeKind::Function | NodeKind::Struct | NodeKind::Trait)
}

/// `file#A.b` → `file#A`.
fn parent_id(id: &str) -> Option<&str> {
    let (file, q) = id.split_once('#')?;
    let cut = q.rfind('.')?;
    Some(&id[..file.len() + 1 + cut])
}

/// Entry points of `g` with the rule that made each one an entry.
pub fn entries(root: &Path, g: &Graph, rules: &Rules) -> HashMap<String, usize> {
    let mut ctx = Ctx {
        root,
        files: HashMap::new(),
        when_cache: HashMap::new(),
    };
    let manifest = manifest_entries(root);
    let mut out: HashMap<String, usize> = HashMap::new();
    let main_rule = rules
        .rules
        .iter()
        .position(|r| r.kind == "main")
        .unwrap_or(0);
    // Registrations: names captured anywhere in the project's code.
    let mut registered: HashMap<String, usize> = HashMap::new();
    let files: BTreeSet<String> = g
        .nodes()
        .filter(|n| n.kind == NodeKind::Module)
        .map(|n| n.id.clone())
        .collect();
    for (i, r) in rules.rules.iter().enumerate() {
        let Some(reg) = &r.registrations else {
            continue;
        };
        for rel in &files {
            if let Some(f) = ctx.file(rel) {
                for c in reg.captures_iter(&f.text) {
                    if let Some(m) = c.get(1) {
                        registered.entry(m.as_str().to_owned()).or_insert(i);
                    }
                }
            }
        }
    }
    for n in g.nodes() {
        let Some(file) = n
            .file
            .as_ref()
            .map(|f| f.to_string_lossy().replace('\\', "/"))
        else {
            continue;
        };
        let hit = if n.kind == NodeKind::Module {
            if manifest.contains(&file) {
                Some(main_rule)
            } else {
                ctx.module_entry(rules, &file)
            }
        } else if candidate_kind(n.kind) {
            if manifest.contains(&file) {
                Some(main_rule)
            } else {
                ctx.symbol_entry(rules, &file, &n.name, n.span)
                    .or_else(|| registered.get(&n.name).copied())
            }
        } else {
            None
        };
        if let Some(i) = hit {
            out.insert(n.id.clone(), i);
        }
    }
    out
}

/// HTTP routes among the nodes `ids` by the `route` entry rules: (rule, file, line, name). A
/// route file (Next.js `route.ts`, Django `views.py`, Laravel controllers, …) counts once.
pub fn routes(
    root: &Path,
    g: &Graph,
    ids: &BTreeSet<String>,
) -> Vec<(String, String, u32, String)> {
    let all = Rules::load(root);
    let rules = Rules {
        rules: all
            .rules
            .into_iter()
            .filter(|r| r.kind == "route")
            .collect(),
        ignore: None,
        notes: Vec::new(),
    };
    let mut ctx = Ctx {
        root,
        files: HashMap::new(),
        when_cache: HashMap::new(),
    };
    let mut files = BTreeSet::new();
    let mut out = Vec::new();
    for id in ids {
        let Some(n) = g.node(id) else { continue };
        let Some(file) = n
            .file
            .as_ref()
            .map(|f| f.to_string_lossy().replace('\\', "/"))
        else {
            continue;
        };
        if let Some(i) = ctx.module_entry(&rules, &file) {
            if files.insert(file.clone()) {
                out.push((rules.rules[i].name.clone(), file.clone(), 1, file));
            }
            continue;
        }
        if candidate_kind(n.kind)
            && let Some(i) = ctx.symbol_entry(&rules, &file, &n.name, n.span)
        {
            out.push((
                rules.rules[i].name.clone(),
                file,
                n.span.map_or(1, |s| s.0),
                n.name.clone(),
            ));
        }
    }
    out
}

/// Everything reachable from `entries` over dependency edges; a reachable member makes its
/// type and file reachable, a reachable trait/interface its members.
pub fn reachable(g: &Graph, entries: impl IntoIterator<Item = String>) -> HashSet<String> {
    let mut out_edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in g.edges().filter(|e| e.kind.is_dependency()) {
        out_edges
            .entry(e.source.as_str())
            .or_default()
            .push(e.target.as_str());
    }
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in g.nodes() {
        if let Some(p) = parent_id(&n.id) {
            children.entry(p).or_default().push(n.id.as_str());
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = entries.into_iter().collect();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let mut next: Vec<String> = Vec::new();
        next.extend(
            out_edges
                .get(id.as_str())
                .into_iter()
                .flatten()
                .map(|s| (*s).to_owned()),
        );
        if let Some(p) = parent_id(&id) {
            next.push(p.to_owned());
        }
        if let Some(n) = g.node(&id) {
            if let Some(f) = &n.file {
                next.push(f.to_string_lossy().replace('\\', "/"));
            }
            if n.kind == NodeKind::Trait {
                next.extend(
                    children
                        .get(id.as_str())
                        .into_iter()
                        .flatten()
                        .map(|s| (*s).to_owned()),
                );
            }
        }
        queue.extend(next.into_iter().filter(|x| !seen.contains(x)));
    }
    seen
}

/// Identifier counts of `names` over the project's code and reference files, skipping the
/// line ranges in `skip` (the candidates themselves).
fn mentions(
    root: &Path,
    files: &BTreeSet<String>,
    names: &HashSet<&str>,
    skip: &HashMap<String, Vec<(u32, u32)>>,
) -> HashMap<String, u32> {
    let mut count: HashMap<String, u32> = HashMap::new();
    let mut all: BTreeSet<String> = files.clone();
    for e in ignore::WalkBuilder::new(root)
        .hidden(true)
        .build()
        .flatten()
    {
        let p = e.path();
        if p.extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| REF_EXTS.contains(&x))
            && e.metadata().is_ok_and(|m| m.len() <= code::MAX_FILE_BYTES)
            && !p.components().any(|c| c.as_os_str() == "node_modules")
        {
            all.insert(super::rel(root, p));
        }
    }
    for rel in &all {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let ranges = skip.get(rel);
        for (i, line) in text.lines().enumerate() {
            let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
            if ranges.is_some_and(|r| r.iter().any(|(a, b)| *a <= n && n <= *b)) {
                continue;
            }
            for m in IDENT.find_iter(line) {
                if names.contains(m.as_str()) {
                    *count.entry(m.as_str().to_owned()).or_default() += 1;
                }
            }
        }
    }
    count
}

/// Dead and ambiguous symbols of the project graph.
pub fn find(root: &Path, g: &Graph) -> DeadReport {
    let rules = Rules::load(root);
    let entry_map = entries(root, g, &rules);
    let mut entry_rules: BTreeMap<String, usize> = BTreeMap::new();
    for i in entry_map.values() {
        *entry_rules.entry(rules.rules[*i].name.clone()).or_default() += 1;
    }
    let live = reachable(g, entry_map.keys().cloned());
    let candidates: Vec<&kor_graph::GraphNode> = g
        .nodes()
        .filter(|n| candidate_kind(n.kind) && !live.contains(&n.id))
        .filter(|n| {
            n.file.as_ref().is_some_and(|f| {
                let rel = f.to_string_lossy().replace('\\', "/");
                !code::is_test_path(&rel) && !rules.ignored(&rel)
            })
        })
        .collect();
    let symbols = g.nodes().filter(|n| candidate_kind(n.kind)).count();
    let mut skip: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
    let mut file_of: HashMap<String, String> = HashMap::new();
    for n in &candidates {
        if let (Some(f), Some(sp)) = (&n.file, n.span) {
            let rel = f.to_string_lossy().replace('\\', "/");
            skip.entry(rel.clone()).or_default().push(sp);
            file_of.insert(n.id.clone(), rel);
        }
    }
    let names: HashSet<&str> = candidates.iter().map(|n| n.name.as_str()).collect();
    let code_files: BTreeSet<String> = g
        .nodes()
        .filter(|n| n.kind == NodeKind::Module)
        .map(|n| n.id.clone())
        .collect();
    let counts = mentions(root, &code_files, &names, &skip);
    let last = if git::is_repo(root) {
        Some(git::last_change(root))
    } else {
        None
    };
    let now = super::now_secs();
    let mut reflective: HashMap<String, bool> = HashMap::new();
    let mut dead = Vec::new();
    let mut ambiguous = Vec::new();
    for n in candidates {
        let file = file_of.get(&n.id).cloned().unwrap_or_default();
        let span = n.span.unwrap_or((1, 1));
        let unchanged_days = last
            .as_ref()
            .and_then(|m| m.get(&file))
            .map(|t| (now - t) / 86_400);
        let refl = *reflective.entry(file.clone()).or_insert_with(|| {
            std::fs::read_to_string(root.join(&file)).is_ok_and(|t| REFLECTION.is_match(&t))
        });
        let (confidence, reason) = match counts.get(&n.name) {
            Some(c) => (Confidence::Ambiguous, Reason::NameElsewhere { count: *c }),
            None if refl => (Confidence::Ambiguous, Reason::Reflection),
            None if unchanged_days.is_some_and(|d| d >= STALE_DAYS) => {
                (Confidence::High, Reason::Unreachable)
            }
            None => (Confidence::Medium, Reason::Unreachable),
        };
        let item = DeadItem {
            id: n.id.clone(),
            kind: n.kind.as_str().to_owned(),
            name: n.name.clone(),
            file,
            line: span.0,
            end: span.1,
            confidence,
            reason,
            unchanged_days,
        };
        if confidence == Confidence::Ambiguous {
            ambiguous.push(item);
        } else {
            dead.push(item);
        }
    }
    let order = |a: &DeadItem, b: &DeadItem| {
        (a.confidence, &a.file, a.line).cmp(&(b.confidence, &b.file, b.line))
    };
    dead.sort_by(order);
    ambiguous.sort_by(order);
    let mut notes = rules.notes;
    if last.is_none() {
        notes.push("not a git repository: no high-confidence results".into());
    }
    DeadReport {
        symbols,
        entries: entry_map.len(),
        entry_rules,
        dead,
        ambiguous,
        notes,
    }
}

/// The report as findings (`kor.dead-code.unreachable`, `kor.dead-code.ambiguous`).
pub fn report(r: &DeadReport, l: &Localizer) -> Report {
    let mut out = Report::new("dead-code");
    for d in r.dead.iter().chain(&r.ambiguous) {
        let (rule, severity) = match d.confidence {
            Confidence::High => ("kor.dead-code.unreachable", Severity::Low),
            Confidence::Medium => ("kor.dead-code.unreachable", Severity::Info),
            Confidence::Ambiguous => ("kor.dead-code.ambiguous", Severity::Info),
        };
        let mut a = args!("kind" => d.kind.clone(), "name" => d.name.clone(), "line" => d.line, "end" => d.end);
        let id = match (&d.reason, d.unchanged_days) {
            (Reason::Unreachable, Some(days)) => {
                a.set("days", days);
                "metrics-f-dead-stale"
            }
            (Reason::Unreachable, None) => "metrics-f-dead-unreachable",
            (Reason::NameElsewhere { count }, _) => {
                a.set("count", *count);
                "metrics-f-dead-name"
            }
            (Reason::Reflection, _) => "metrics-f-dead-reflection",
        };
        out.findings.push(Finding {
            rule: rule.into(),
            source: "kor".into(),
            severity,
            file: d.file.clone().into(),
            line: d.line,
            message: l.t_args(id, &a),
            ..Finding::default()
        });
    }
    out.notes.clone_from(&r.notes);
    out
}

// ---------------------------------------------------------------------------------------------
// --prune
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct PruneOptions {
    /// Also remove medium-confidence items (recently changed files).
    pub include_medium: bool,
    /// Compiler / type-checker command; detected from the project when `None`.
    pub check: Option<String>,
    /// Test command; `[orchestrator] test_command` or detected when `None`.
    pub test: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmdRun {
    pub command: String,
    pub ok: bool,
    /// The last 8 000 characters.
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunePlan {
    pub branch: String,
    /// Branch the PR goes into (the one checked out when pruning started).
    pub base: String,
    pub removed: Vec<DeadItem>,
    /// Put back because the checker complained about their files.
    pub restored: Vec<DeadItem>,
    /// Not touched: their files have uncommitted changes.
    pub skipped: Vec<DeadItem>,
    pub check: Option<CmdRun>,
    pub tests: Option<CmdRun>,
    pub commit: Option<String>,
    /// Committed on `branch` and verified: ready to push after confirmation.
    pub ready: bool,
    /// Message ids (`prune-no-checker`, …; localized as `deadcode-<id>`) or raw error text.
    pub notes: Vec<String>,
}

/// Compiler/type-checker and test commands for the project's build system.
pub fn detect_commands(root: &Path) -> (Option<String>, Option<String>) {
    let has = |f: &str| root.join(f).exists();
    if has("Cargo.toml") {
        return (
            Some("cargo check --all-targets --quiet".into()),
            Some("cargo test --quiet".into()),
        );
    }
    if has("go.mod") {
        return (Some("go vet ./...".into()), Some("go test ./...".into()));
    }
    if has("package.json") {
        let pkg: serde_json::Value = std::fs::read(root.join("package.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let test = pkg["scripts"]["test"]
            .as_str()
            .filter(|t| !t.contains("no test specified"))
            .map(|_| "npm test --silent".to_owned());
        let check = has("tsconfig.json").then(|| "npx --no-install tsc --noEmit".to_owned());
        return (check, test);
    }
    if has("pyproject.toml") || has("setup.py") || has("requirements.txt") {
        let pytest = has("pytest.ini")
            || has("conftest.py")
            || has("tests")
            || std::fs::read_to_string(root.join("pyproject.toml"))
                .is_ok_and(|t| t.contains("[tool.pytest"));
        return (
            Some("python -m compileall -q .".into()),
            pytest.then(|| "python -m pytest -q".to_owned()),
        );
    }
    if has("pom.xml") {
        return (
            Some("mvn -q -DskipTests compile".into()),
            Some("mvn -q test".into()),
        );
    }
    if has("build.gradle") || has("build.gradle.kts") {
        let g = if has("gradlew") {
            "./gradlew"
        } else {
            "gradle"
        };
        return (
            Some(format!("{g} -q compileJava")),
            Some(format!("{g} -q test")),
        );
    }
    let sln = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let n = PathBuf::from(e.file_name().to_string_lossy().to_lowercase());
            n.extension().is_some_and(|x| x == "sln" || x == "csproj")
        });
    if sln {
        return (
            Some("dotnet build -nologo -v q".into()),
            Some("dotnet test -nologo".into()),
        );
    }
    (None, None)
}

/// Run `command` in `dir` through the Bash tool with the session's sandbox settings.
pub async fn run_command(
    config: &kor_config::Config,
    l: &Localizer,
    dir: &Path,
    command: &str,
) -> CmdRun {
    let fail = |output: String| CmdRun {
        command: command.to_owned(),
        ok: false,
        output,
    };
    let mut ctx = match kor_tools::ToolCtx::new(dir) {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()),
    };
    let sbx = crate::sandbox::setup(l, &config.sandbox, crate::offline_mode());
    ctx.sandbox = sbx.kind;
    ctx.network = sbx.network && !crate::offline_mode();
    ctx.sandbox_options = sbx.options;
    ctx.scrub_env = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| k.ends_with("_API_KEY") || k.ends_with("_TOKEN"))
        .collect();
    let Some(bash) = kor_tools::default_tools()
        .into_iter()
        .find(|t| t.name() == "Bash")
    else {
        return fail("no shell tool".into());
    };
    let o = bash
        .call(
            serde_json::json!({ "command": command, "timeout": 600_000 }),
            &ctx,
        )
        .await;
    let mut output = o.content;
    if output.len() > 8_000 {
        let mut cut = output.len() - 8_000;
        while !output.is_char_boundary(cut) {
            cut += 1;
        }
        output = format!("[…]{}", &output[cut..]);
    }
    CmdRun {
        command: command.to_owned(),
        ok: !o.is_error,
        output,
    }
}

/// Lines to delete for `item` (1-based, inclusive): its span plus the doc comments, attributes
/// and decorators directly above it.
fn removal_span(lines: &[&str], item: &DeadItem) -> (usize, usize) {
    let mut start = item.line as usize;
    let end = (item.end as usize).min(lines.len());
    while start > 1 {
        let above = lines[start - 2].trim_start();
        if above.is_empty() {
            break;
        }
        let directive = [
            "#include", "#define", "#if", "#endif", "#else", "#pragma", "#import", "#!",
        ]
        .iter()
        .any(|p| above.starts_with(p));
        let attached = !directive
            && ["///", "//!", "/**", "*", "//", "#[", "@", "#"]
                .iter()
                .any(|p| above.starts_with(p));
        if !attached {
            break;
        }
        start -= 1;
    }
    (start, end)
}

/// Delete `items` from `text`; nested items inside another removed one are skipped.
pub fn remove_items(text: &str, items: &[&DeadItem]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut spans: Vec<(usize, usize)> = items.iter().map(|i| removal_span(&lines, i)).collect();
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let mut keep = vec![true; lines.len()];
    for (a, b) in &merged {
        for k in keep.iter_mut().take(*b).skip(a - 1) {
            *k = false;
        }
        // At the end of the file, drop the blank lines the item leaves behind.
        if *b >= lines.len() {
            let mut k = a - 1;
            while k > 0 && lines[k - 1].trim().is_empty() {
                keep[k - 1] = false;
                k -= 1;
            }
        }
        // Avoid leaving two blank lines where the item was.
        let before_blank = *a < 2 || lines[a - 2].trim().is_empty();
        if before_blank && *b < lines.len() && lines[*b].trim().is_empty() {
            keep[*b] = false;
        }
    }
    let mut out = String::with_capacity(text.len());
    for (l, _) in lines.iter().zip(&keep).filter(|(_, k)| **k) {
        out.push_str(l);
        out.push('\n');
    }
    if !text.ends_with('\n') {
        out.pop();
    }
    out
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn exclude_worktrees(root: &Path) {
    let Ok(p) = git::git(root, &["rev-parse", "--git-path", "info/exclude"]) else {
        return;
    };
    let p = root.join(p.trim());
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    if !text.lines().any(|l| l.trim() == "/.kor/worktrees/") {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        let _ = std::fs::write(&p, format!("{text}{sep}/.kor/worktrees/\n"));
    }
}

fn identity(dir: &Path) -> Vec<&'static str> {
    if git::user_email(dir).is_some() {
        Vec::new()
    } else {
        vec!["-c", "user.name=Kor", "-c", "user.email=kor@localhost"]
    }
}

/// Write the deletion on a new branch in its own worktree, verify it and commit it. Nothing is
/// pushed; the user's working tree and branch are not touched.
#[allow(clippy::too_many_lines)] // one linear procedure: select, write, verify, commit
pub async fn prune(
    root: &Path,
    report: &DeadReport,
    o: &PruneOptions,
    config: &kor_config::Config,
    l: &Localizer,
) -> Result<PrunePlan, String> {
    if !git::is_repo(root) {
        return Err("not a git repository".into());
    }
    let base = git::git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned();
    let branch = format!("kor/dead-code/{}", stamp());
    let mut plan = PrunePlan {
        branch: branch.clone(),
        base,
        removed: Vec::new(),
        restored: Vec::new(),
        skipped: Vec::new(),
        check: None,
        tests: None,
        commit: None,
        ready: false,
        notes: Vec::new(),
    };
    let wanted: Vec<&DeadItem> = report
        .dead
        .iter()
        .filter(|d| {
            d.confidence == Confidence::High
                || (o.include_medium && d.confidence == Confidence::Medium)
        })
        .collect();
    if wanted.is_empty() {
        plan.notes.push("prune-nothing".into());
        return Ok(plan);
    }
    let (detected_check, detected_test) = detect_commands(root);
    let config_test =
        Some(config.orchestrator.test_command.trim().to_owned()).filter(|t| !t.is_empty());
    let check_cmd = o.check.clone().or(detected_check);
    let test_cmd = o.test.clone().or(config_test).or(detected_test);
    let Some(check_cmd) = check_cmd else {
        plan.notes.push("prune-no-checker".into());
        return Ok(plan);
    };
    let mut by_file: BTreeMap<String, Vec<&DeadItem>> = BTreeMap::new();
    for d in wanted {
        // Items in files with uncommitted changes would not match HEAD's lines.
        let clean = git::git(root, &["diff", "--quiet", "HEAD", "--", &d.file]).is_ok()
            && git::git(root, &["ls-files", "--error-unmatch", "--", &d.file]).is_ok();
        if clean {
            by_file.entry(d.file.clone()).or_default().push(d);
        } else {
            plan.skipped.push(d.clone());
        }
    }
    if by_file.is_empty() {
        plan.notes.push("prune-all-dirty".into());
        return Ok(plan);
    }
    exclude_worktrees(root);
    let wt = root
        .join(".kor")
        .join("worktrees")
        .join(format!("dead-code-{}", stamp()));
    let wt_arg = wt.to_string_lossy().into_owned();
    git::git(
        root,
        &["worktree", "add", "-q", "-b", &branch, &wt_arg, "HEAD"],
    )?;
    let cleanup = |delete_branch: bool| {
        let _ = git::git(root, &["worktree", "remove", "--force", &wt_arg]);
        if delete_branch {
            let _ = git::git(root, &["branch", "-D", &branch]);
        }
    };
    for (file, items) in &by_file {
        let p = wt.join(file);
        match std::fs::read_to_string(&p) {
            Ok(text) => {
                if std::fs::write(&p, remove_items(&text, items)).is_ok() {
                    plan.removed.extend(items.iter().map(|d| (*d).clone()));
                }
            }
            Err(e) => plan.notes.push(format!("{file}: {e}")),
        }
    }
    for _round in 0..3 {
        let run = run_command(config, l, &wt, &check_cmd).await;
        let ok = run.ok;
        if ok {
            plan.check = Some(run);
            break;
        }
        let out = run.output.replace('\\', "/");
        let blamed: BTreeSet<String> = plan
            .removed
            .iter()
            .filter(|d| {
                out.contains(&d.file)
                    || out.contains(&format!("`{}`", d.name))
                    || out.contains(&format!("'{}'", d.name))
            })
            .map(|d| d.file.clone())
            .collect();
        plan.check = Some(run);
        if blamed.is_empty() {
            break;
        }
        for f in &blamed {
            let _ = git::git(&wt, &["checkout", "HEAD", "--", f]);
        }
        let (back, keep): (Vec<DeadItem>, Vec<DeadItem>) = plan
            .removed
            .drain(..)
            .partition(|d| blamed.contains(&d.file));
        plan.restored.extend(back);
        plan.removed = keep;
        if plan.removed.is_empty() {
            break;
        }
    }
    if !plan.check.as_ref().is_some_and(|c| c.ok) || plan.removed.is_empty() {
        plan.notes.push("prune-check-failed".into());
        cleanup(true);
        return Ok(plan);
    }
    if let Some(t) = &test_cmd {
        let run = run_command(config, l, &wt, t).await;
        let ok = run.ok;
        plan.tests = Some(run);
        if !ok {
            plan.notes.push("prune-tests-failed".into());
            cleanup(true);
            return Ok(plan);
        }
    } else {
        plan.notes.push("prune-no-tests".into());
    }
    let files: Vec<&str> = plan
        .removed
        .iter()
        .map(|d| d.file.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut add = vec!["add", "--"];
    add.extend(&files);
    git::git(&wt, &add)?;
    let mut commit = identity(&wt);
    let msg = format!(
        "chore: remove dead code ({} symbols, kor dead-code)\n\n{}",
        plan.removed.len(),
        plan.removed
            .iter()
            .map(|d| format!("- {}:{} {}", d.file, d.line, d.name))
            .collect::<Vec<_>>()
            .join("\n")
    );
    commit.extend(["commit", "-q", "-m", &msg]);
    match git::git(&wt, &commit) {
        Ok(_) => {
            plan.commit = git::head(&wt);
            plan.ready = plan.commit.is_some();
            cleanup(false);
        }
        Err(e) => {
            plan.notes.push(format!("git commit: {e}"));
            cleanup(true);
        }
    }
    Ok(plan)
}

fn why(d: &DeadItem) -> String {
    match (&d.reason, d.unchanged_days) {
        (Reason::Unreachable, Some(days)) => {
            format!("unreachable from every entry point; file unchanged for {days} days")
        }
        _ => "unreachable from every entry point".to_owned(),
    }
}

/// Title and body of the pull request for a verified plan.
pub fn pr_request(plan: &PrunePlan) -> PrRequest {
    let mut body = format!(
        "Removes {} symbols that no entry point reaches (`kor dead-code --prune`, spec §15.14).\n\n## Removed\n\n| Symbol | Where | Why |\n| --- | --- | --- |\n",
        plan.removed.len()
    );
    for d in &plan.removed {
        let _ = writeln!(
            body,
            "| {} `{}` | `{}:{}-{}` | {} |",
            d.kind,
            d.name,
            d.file,
            d.line,
            d.end,
            why(d)
        );
    }
    if !plan.restored.is_empty() {
        body.push_str("\n## Kept (the checker needed them)\n\n");
        for d in &plan.restored {
            let _ = writeln!(body, "- `{}` in `{}`", d.name, d.file);
        }
    }
    body.push_str("\n## Verification\n\n");
    for (label, run) in [("Checker", &plan.check), ("Tests", &plan.tests)] {
        match run {
            Some(r) => {
                let _ = writeln!(
                    body,
                    "- {label}: `{}` — {}",
                    r.command,
                    if r.ok { "passed" } else { "failed" }
                );
            }
            None => {
                let _ = writeln!(body, "- {label}: not run");
            }
        }
    }
    body.push_str("\nAmbiguous symbols (dynamic calls, reflection, names used in strings or templates) were not touched.\n");
    PrRequest {
        head: plan.branch.clone(),
        base: plan.base.clone(),
        title: format!("chore: remove {} unreachable symbols", plan.removed.len()),
        body,
        draft: false,
    }
}

/// Push the verified branch and open the PR with `create` (the forge call). Call only after
/// the user confirmed.
pub async fn publish_with<F, Fut>(
    root: &Path,
    plan: &PrunePlan,
    create: F,
) -> Result<PrRef, ForgeError>
where
    F: FnOnce(PrRequest) -> Fut,
    Fut: std::future::Future<Output = Result<PrRef, ForgeError>>,
{
    if !plan.ready {
        return Err(ForgeError::Git(
            "the dead-code branch is not verified".into(),
        ));
    }
    forge::push(root, &plan.branch)?;
    create(pr_request(plan)).await
}

/// [`publish_with`] through `forge::create_pull_request` (the remote's forge and token).
pub async fn publish(root: &Path, plan: &PrunePlan) -> Result<PrRef, ForgeError> {
    publish_with(root, plan, |req| async move {
        forge::create_pull_request(root, &req).await
    })
    .await
}

#[cfg(test)]
mod tests;
