//! Turns per-file facts into graph nodes and cross-file edges.
//!
//! This runs over all cached facts on every update; it is pure in-memory work (no I/O) and is
//! cheap next to parsing, which only happens for changed files.

use crate::git::GitStats;
use crate::lang::Lang;
use crate::model::{Confidence, Edge, EdgeKind, GraphNode, NodeId, NodeKind};
use crate::parse::{FileFacts, FileKind, ImportForm, RefKind};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

pub type EdgeKey = (NodeId, NodeId, &'static str);

#[derive(Debug, Default)]
pub struct Derived {
    pub nodes: BTreeMap<NodeId, GraphNode>,
    pub edges: BTreeMap<EdgeKey, Edge>,
}

impl Derived {
    pub(crate) fn node(
        &mut self,
        id: &str,
        kind: NodeKind,
        name: &str,
        file: Option<&str>,
        span: Option<(u32, u32)>,
        text: &str,
    ) {
        self.nodes
            .entry(id.to_owned())
            .or_insert_with(|| GraphNode {
                id: id.to_owned(),
                kind,
                name: name.to_owned(),
                file: file.map(PathBuf::from),
                span,
                community: 0,
                degree: 0,
                churn_30d: 0,
                text: text.to_owned(),
            });
    }

    pub(crate) fn edge(
        &mut self,
        source: &str,
        target: &str,
        kind: EdgeKind,
        confidence: Confidence,
    ) {
        if source == target {
            return;
        }
        let key = (source.to_owned(), target.to_owned(), kind.as_str());
        match self.edges.get_mut(&key) {
            Some(e) if e.confidence.score() >= confidence.score() => {}
            Some(e) => e.confidence = confidence,
            None => {
                self.edges.insert(
                    key,
                    Edge {
                        source: source.to_owned(),
                        target: target.to_owned(),
                        kind,
                        confidence,
                    },
                );
            }
        }
    }
}

/// Lookup tables over the project's file paths.
struct FileIndex<'a> {
    paths: HashSet<&'a str>,
    by_name: HashMap<&'a str, Vec<&'a str>>,
    by_dir: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> FileIndex<'a> {
    fn new(files: &'a BTreeMap<String, FileFacts>) -> Self {
        let mut idx = Self {
            paths: HashSet::new(),
            by_name: HashMap::new(),
            by_dir: HashMap::new(),
        };
        for p in files.keys() {
            idx.paths.insert(p.as_str());
            idx.by_name.entry(file_name(p)).or_default().push(p);
            idx.by_dir.entry(dir_of(p)).or_default().push(p);
        }
        idx
    }

    fn exact(&self, p: &str) -> Option<&'a str> {
        self.paths.get(p).copied()
    }

    /// Files whose path equals `suffix` or ends with `/suffix`.
    fn suffix(&self, suffix: &str) -> Vec<&'a str> {
        let suffix = suffix.trim_start_matches('/');
        self.by_name
            .get(file_name(suffix))
            .map(|v| {
                v.iter()
                    .copied()
                    .filter(|p| *p == suffix || p.ends_with(&format!("/{suffix}")))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Files directly inside a directory whose path ends with `suffix`.
    fn dir_suffix(&self, suffix: &str) -> Vec<&'a str> {
        let mut out: Vec<&str> = Vec::new();
        for (d, files) in &self.by_dir {
            if *d == suffix || d.ends_with(&format!("/{suffix}")) {
                out.extend(files.iter().copied());
            }
        }
        out.sort_unstable();
        out
    }
}

fn file_stem(p: &str) -> &str {
    let n = file_name(p);
    n.split_once('.').map_or(n, |(s, _)| s)
}

/// `path#A.B~2` is owned by `B` when its qualified name ends with `B.<name>`.
fn owned_by(id: &str, owner: &str, name: &str) -> bool {
    let qual = id.rsplit_once('#').map_or(id, |(_, q)| q);
    let qual = qual.split_once('~').map_or(qual, |(q, _)| q);
    qual.strip_suffix(name)
        .and_then(|q| q.strip_suffix('.'))
        .is_some_and(|q| q == owner || q.ends_with(&format!(".{owner}")))
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

pub fn dir_of(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

fn join(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn one_or_ambiguous(hits: Vec<&str>, score: f32) -> Vec<(String, Confidence)> {
    match hits.len() {
        1 => vec![(hits[0].to_owned(), Confidence::Inferred { score })],
        2..=3 => hits
            .into_iter()
            .map(|h| (h.to_owned(), Confidence::Ambiguous))
            .collect(),
        _ => Vec::new(),
    }
}

const JS_EXTS: [&str; 7] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".d.ts"];

fn js_candidates(base: &str) -> Vec<String> {
    let mut c = vec![base.to_owned()];
    if let Some(stem) = base.strip_suffix(".js") {
        c.push(format!("{stem}.ts"));
        c.push(format!("{stem}.tsx"));
    }
    c.extend(JS_EXTS.iter().map(|e| format!("{base}{e}")));
    c.extend(JS_EXTS.iter().map(|e| format!("{base}/index{e}")));
    c
}

/// Directory that holds a Rust file's child modules.
fn rust_module_dir(path: &str) -> String {
    let name = file_name(path);
    let dir = dir_of(path);
    if matches!(name, "mod.rs" | "lib.rs" | "main.rs") {
        dir.to_owned()
    } else {
        let stem = name.trim_end_matches(".rs");
        if dir.is_empty() {
            stem.to_owned()
        } else {
            format!("{dir}/{stem}")
        }
    }
}

fn rust_import(
    from: &str,
    spec: &str,
    form: ImportForm,
    idx: &FileIndex<'_>,
) -> Vec<(String, Confidence)> {
    let mdir = rust_module_dir(from);
    if form == ImportForm::ModDecl {
        return [format!("{mdir}/{spec}.rs"), format!("{mdir}/{spec}/mod.rs")]
            .iter()
            .find_map(|c| idx.exact(c.trim_start_matches('/')))
            .map(|p| vec![(p.to_owned(), Confidence::Extracted)])
            .unwrap_or_default();
    }
    let path = spec
        .split('{')
        .next()
        .unwrap_or(spec)
        .trim_end_matches("::");
    let mut segs: Vec<&str> = path
        .split("::")
        .map(|s| s.split(" as ").next().unwrap_or(s).trim())
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() {
        return Vec::new();
    }
    let base = match segs[0] {
        "crate" => {
            segs.remove(0);
            let mut d = from;
            loop {
                d = dir_of(d);
                if d.is_empty() || file_name(d) == "src" {
                    break d.to_owned();
                }
            }
        }
        "self" => {
            segs.remove(0);
            mdir
        }
        "super" => {
            let mut d = mdir.as_str();
            while segs.first() == Some(&"super") {
                segs.remove(0);
                d = dir_of(d);
            }
            d.to_owned()
        }
        krate => {
            // A workspace crate: `kor_core::x` → `<..>/kor-core/src/x.rs`.
            let dashed = krate.replace('_', "-");
            let lib = idx.suffix(&format!("{dashed}/src/lib.rs"));
            let lib = if lib.is_empty() {
                idx.suffix(&format!("{krate}/src/lib.rs"))
            } else {
                lib
            };
            let [lib] = lib.as_slice() else {
                return Vec::new();
            };
            segs.remove(0);
            dir_of(lib).to_owned()
        }
    };
    for k in (0..=segs.len()).rev() {
        let rel = segs[..k].join("/");
        let cands = if rel.is_empty() {
            vec![
                format!("{base}/lib.rs"),
                format!("{base}/main.rs"),
                format!("{base}/mod.rs"),
            ]
        } else {
            vec![format!("{base}/{rel}.rs"), format!("{base}/{rel}/mod.rs")]
        };
        if let Some(p) = cands
            .iter()
            .find_map(|c| idx.exact(c.trim_start_matches('/')))
        {
            return if p == from {
                Vec::new()
            } else {
                vec![(p.to_owned(), Confidence::Extracted)]
            };
        }
    }
    Vec::new()
}

fn python_import(from: &str, spec: &str, idx: &FileIndex<'_>) -> Vec<(String, Confidence)> {
    let dots = spec.chars().take_while(|c| *c == '.').count();
    let rest = spec[dots..].trim().replace('.', "/");
    let cands = |base: &str| -> Vec<String> {
        let b = if base.is_empty() {
            rest.clone()
        } else if rest.is_empty() {
            base.to_owned()
        } else {
            format!("{base}/{rest}")
        };
        if rest.is_empty() {
            vec![format!("{b}/__init__.py")]
        } else {
            vec![format!("{b}.py"), format!("{b}/__init__.py")]
        }
    };
    if dots > 0 {
        let mut base = dir_of(from);
        for _ in 1..dots {
            base = dir_of(base);
        }
        return cands(base)
            .iter()
            .find_map(|c| idx.exact(c.trim_start_matches('/')))
            .map(|p| vec![(p.to_owned(), Confidence::Extracted)])
            .unwrap_or_default();
    }
    if let Some(p) = cands("").iter().find_map(|c| idx.exact(c)) {
        return vec![(p.to_owned(), Confidence::Extracted)];
    }
    cands("")
        .iter()
        .map(|c| idx.suffix(c))
        .find(|h| !h.is_empty())
        .map(|h| one_or_ambiguous(h, 0.8))
        .unwrap_or_default()
}

fn js_import(from: &str, spec: &str, idx: &FileIndex<'_>) -> Vec<(String, Confidence)> {
    if spec.starts_with('.') {
        let Some(base) = join(dir_of(from), spec) else {
            return Vec::new();
        };
        return js_candidates(&base)
            .iter()
            .find_map(|c| idx.exact(c))
            .map(|p| vec![(p.to_owned(), Confidence::Extracted)])
            .unwrap_or_default();
    }
    // Path aliases (`@/x`, `~/x`); bare package names never match a project file.
    let Some(rest) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) else {
        return Vec::new();
    };
    js_candidates(rest)
        .iter()
        .map(|c| idx.suffix(c))
        .find(|h| !h.is_empty())
        .map(|h| one_or_ambiguous(h, 0.7))
        .unwrap_or_default()
}

fn dir_files(hits: Vec<&str>, ext: &str, score: f32) -> Vec<(String, Confidence)> {
    hits.into_iter()
        .filter(|p| p.ends_with(ext))
        .take(10)
        .map(|p| (p.to_owned(), Confidence::Inferred { score }))
        .collect()
}

fn go_import(spec: &str, idx: &FileIndex<'_>) -> Vec<(String, Confidence)> {
    let segs: Vec<&str> = spec.split('/').collect();
    for i in 0..segs.len() {
        let hits = idx.dir_suffix(&segs[i..].join("/"));
        if !hits.is_empty() {
            return dir_files(hits, ".go", 0.9);
        }
    }
    Vec::new()
}

/// `a.b.C` (Java, Kotlin, Scala, Haskell…) → `a/b/C<ext>`; `a.b.*` → every `<ext>` file in `a/b`.
fn dotted_import(spec: &str, ext: &str, idx: &FileIndex<'_>) -> Vec<(String, Confidence)> {
    if let Some(pkg) = spec.strip_suffix(".*") {
        return dir_files(idx.dir_suffix(&pkg.replace('.', "/")), ext, 0.8);
    }
    let segs: Vec<&str> = spec.split('.').collect();
    // `a.b.C` or a static member import `a.b.C.m`.
    for k in [segs.len(), segs.len().saturating_sub(1)] {
        if k == 0 {
            continue;
        }
        let hits = idx.suffix(&format!("{}{ext}", segs[..k].join("/")));
        match hits.len() {
            0 => {}
            1 => return vec![(hits[0].to_owned(), Confidence::Extracted)],
            _ => return one_or_ambiguous(hits, 0.8),
        }
    }
    Vec::new()
}

fn csharp_import(spec: &str, idx: &FileIndex<'_>) -> Vec<(String, Confidence)> {
    let ns = spec.rsplit('=').next().unwrap_or(spec).trim();
    dir_files(idx.dir_suffix(&ns.replace('.', "/")), ".cs", 0.5)
}

fn path_import(
    from: &str,
    spec: &str,
    exts: &[&str],
    score: f32,
    idx: &FileIndex<'_>,
) -> Vec<(String, Confidence)> {
    // PHP: `__DIR__ . '/x.php'` → `/x.php` (joined to the file's directory below).
    let spec = match spec.rsplit_once("__DIR__") {
        Some((_, rest)) => rest.trim_start_matches([' ', '.', '\'', '"']),
        None => spec,
    };
    let spec = spec.trim_matches(['\'', '"', ' ']);
    let with_ext: Vec<String> = std::iter::once(spec.to_owned())
        .chain(exts.iter().map(|e| format!("{spec}{e}")))
        .collect();
    if let Some(p) = with_ext
        .iter()
        .filter_map(|c| join(dir_of(from), c))
        .find_map(|c| idx.exact(&c))
    {
        return vec![(p.to_owned(), Confidence::Extracted)];
    }
    with_ext
        .iter()
        .map(|c| idx.suffix(c.trim_start_matches("./")))
        .find(|h| !h.is_empty())
        .map(|h| one_or_ambiguous(h, score))
        .unwrap_or_default()
}

fn php_use(spec: &str, idx: &FileIndex<'_>) -> Vec<(String, Confidence)> {
    let spec = spec
        .trim_start_matches("function ")
        .trim_start_matches("const ");
    let spec = spec
        .split(" as ")
        .next()
        .unwrap_or(spec)
        .trim()
        .trim_start_matches('\\');
    let segs: Vec<&str> = spec.split('\\').collect();
    // PSR-4 maps a namespace prefix to a directory, so drop leading segments until a file matches.
    for i in 0..segs.len() {
        let hits = idx.suffix(&format!("{}.php", segs[i..].join("/")));
        if !hits.is_empty() {
            return if i == 0 && hits.len() == 1 {
                vec![(hits[0].to_owned(), Confidence::Extracted)]
            } else {
                one_or_ambiguous(hits, 0.8)
            };
        }
    }
    Vec::new()
}

fn resolve_import(
    lang: Lang,
    from: &str,
    spec: &str,
    form: ImportForm,
    idx: &FileIndex<'_>,
) -> Vec<(String, Confidence)> {
    match lang {
        Lang::Rust => rust_import(from, spec, form, idx),
        Lang::Python => python_import(from, spec, idx),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => js_import(from, spec, idx),
        Lang::Go => go_import(spec, idx),
        Lang::Java => dotted_import(spec, ".java", idx),
        Lang::CSharp => csharp_import(spec, idx),
        Lang::C | Lang::Cpp => path_import(from, spec, &[], 0.7, idx),
        Lang::Php if form == ImportForm::Relative => path_import(from, spec, &[], 0.7, idx),
        Lang::Php => php_use(spec, idx),
        Lang::Ruby => path_import(
            from,
            spec,
            &[".rb"],
            if form == ImportForm::Relative {
                0.9
            } else {
                0.7
            },
            idx,
        ),
        Lang::Scala => dotted_import(spec, ".scala", idx),
        Lang::Kotlin => dotted_import(spec, ".kt", idx),
        Lang::Groovy => dotted_import(spec, ".groovy", idx),
        Lang::Haskell => dotted_import(spec, ".hs", idx),
        Lang::D => dotted_import(spec, ".d", idx),
        Lang::Elm => dotted_import(spec, ".elm", idx),
        // The rest name files: `source ./x.sh`, `#include "a.h"`, `@import("b.zig")`, `import "a.proto"`.
        other => path_import(
            from,
            spec,
            other.extensions(),
            if form == ImportForm::Relative {
                0.9
            } else {
                0.7
            },
            idx,
        ),
    }
}

/// Methods of standard library types in the supported languages. A receiver call with one of these
/// names almost never targets project code, and linking it to a same-named project method floods
/// the graph with false edges.
/// shortcut: a name list, not type inference; upgrade to receiver-type resolution if precision matters.
const STD_METHODS: &[&str] = &[
    "add",
    "all",
    "and_then",
    "any",
    "append",
    "apply",
    "as_bytes",
    "as_mut",
    "as_ref",
    "as_str",
    "bind",
    "borrow",
    "bytes",
    "call",
    "catch",
    "chain",
    "chars",
    "clear",
    "clone",
    "cloned",
    "close",
    "collect",
    "contains",
    "contains_key",
    "copied",
    "count",
    "dedup",
    "default",
    "display",
    "ends_with",
    "entry",
    "enumerate",
    "equals",
    "err",
    "expect",
    "extend",
    "filter",
    "filter_map",
    "find",
    "first",
    "flat_map",
    "flatten",
    "flush",
    "fold",
    "forEach",
    "format",
    "from",
    "get",
    "get_mut",
    "hashCode",
    "includes",
    "indexOf",
    "insert",
    "into",
    "into_iter",
    "is_empty",
    "is_err",
    "is_none",
    "is_ok",
    "is_some",
    "items",
    "iter",
    "iter_mut",
    "join",
    "keys",
    "last",
    "len",
    "length",
    "lines",
    "lock",
    "map",
    "map_err",
    "matches",
    "max",
    "min",
    "next",
    "ok",
    "ok_or",
    "ok_or_else",
    "or_default",
    "or_else",
    "or_insert",
    "or_insert_with",
    "parse",
    "pop",
    "position",
    "push",
    "push_str",
    "put",
    "read",
    "read_to_string",
    "recv",
    "remove",
    "replace",
    "retain",
    "rev",
    "send",
    "size",
    "skip",
    "slice",
    "sort",
    "sort_by",
    "splice",
    "split",
    "starts_with",
    "strip",
    "sum",
    "take",
    "then",
    "toLowerCase",
    "toString",
    "toUpperCase",
    "to_lowercase",
    "to_owned",
    "to_string",
    "to_uppercase",
    "trim",
    "truncate",
    "unwrap",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
    "update",
    "values",
    "write",
    "write_all",
    "zip",
];

/// Stable ids for a file's definitions: `path#Outer.inner`, with `~2`, `~3` for repeats.
fn def_ids(path: &str, f: &FileFacts) -> Vec<NodeId> {
    let mut quals: Vec<String> = Vec::with_capacity(f.defs.len());
    for d in &f.defs {
        let q = match (d.parent, &d.container) {
            (Some(p), _) => format!(
                "{}.{}",
                quals.get(p as usize).map_or("", String::as_str),
                d.name
            ),
            (None, Some(c)) => format!("{c}.{}", d.name),
            (None, None) => d.name.clone(),
        };
        quals.push(q);
    }
    let mut seen: HashMap<&str, u32> = HashMap::new();
    quals
        .iter()
        .map(|q| {
            let n = seen.entry(q.as_str()).or_insert(0);
            *n += 1;
            if *n == 1 {
                format!("{path}#{q}")
            } else {
                format!("{path}#{q}~{n}")
            }
        })
        .collect()
}

fn module_kind(f: &FileFacts) -> NodeKind {
    match f.kind {
        FileKind::Code | FileKind::Sql | FileKind::Prisma => NodeKind::Module,
        FileKind::Env | FileKind::Config => NodeKind::Config,
        FileKind::Adr => NodeKind::Decision,
    }
}

/// Build the whole derived graph from cached facts, git statistics and live database snapshots.
#[allow(clippy::too_many_lines)]
pub fn derive(
    files: &BTreeMap<String, FileFacts>,
    git: &GitStats,
    live: &[crate::db::DbSnapshot],
) -> Derived {
    let mut g = Derived::default();
    let idx = FileIndex::new(files);
    let mut ids: HashMap<&str, Vec<NodeId>> = HashMap::new();
    // name → (file, id, kind) for code symbols.
    let mut by_name: HashMap<&str, Vec<(&str, NodeId, NodeKind)>> = HashMap::new();

    for (path, f) in files {
        let name = if f.kind == FileKind::Adr && !f.title.is_empty() {
            f.title.as_str()
        } else {
            path.as_str()
        };
        g.node(
            path,
            module_kind(f),
            name,
            Some(path),
            Some((1, f.lines.max(1))),
            &f.lang,
        );
        let fids = def_ids(path, f);
        for (i, d) in f.defs.iter().enumerate() {
            g.node(
                &fids[i],
                d.kind,
                &d.name,
                Some(path),
                Some((d.start, d.end)),
                "",
            );
            by_name.entry(d.name.as_str()).or_default().push((
                path.as_str(),
                fids[i].clone(),
                d.kind,
            ));
        }
        for (i, d) in f.defs.iter().enumerate() {
            let owner = match (d.parent, &d.container) {
                (Some(p), _) => fids.get(p as usize).cloned(),
                (None, Some(c)) => f
                    .defs
                    .iter()
                    .position(|x| &x.name == c && x.kind != NodeKind::Function)
                    .map(|j| fids[j].clone()),
                (None, None) => None,
            };
            g.edge(
                owner.as_deref().unwrap_or(path),
                &fids[i],
                EdgeKind::Contains,
                Confidence::Extracted,
            );
        }
        ids.insert(path.as_str(), fids);
    }

    // Imports: file → file.
    let mut imported: HashMap<&str, HashSet<String>> = HashMap::new();
    for (path, f) in files {
        let Some(lang) = Lang::from_name(&f.lang) else {
            continue;
        };
        for imp in &f.imports {
            for (target, conf) in resolve_import(lang, path, &imp.spec, imp.form, &idx) {
                g.edge(path, &target, EdgeKind::Imports, conf);
                imported.entry(path.as_str()).or_default().insert(target);
            }
        }
    }

    // Calls / extends / implements, by tiers: same file, imported files, same directory, project-wide.
    let empty = HashSet::new();
    for (path, f) in files {
        let fids = &ids[path.as_str()];
        let imports = imported.get(path.as_str()).unwrap_or(&empty);
        let dir = dir_of(path);
        for r in &f.refs {
            if r.method && STD_METHODS.binary_search(&r.name.as_str()).is_ok() {
                continue;
            }
            let Some(cands) = by_name.get(r.name.as_str()) else {
                continue;
            };
            // `Type::f()` / `module::f()`: only defs owned by that type or living in that module file.
            let qualified: Vec<(&str, NodeId, NodeKind)>;
            let cands = match r.qualifier.as_deref() {
                Some("Self" | "self" | "crate" | "super") | None => cands,
                Some(q) => {
                    qualified = cands
                        .iter()
                        .filter(|c| owned_by(&c.1, q, &r.name) || file_stem(c.0) == q)
                        .cloned()
                        .collect();
                    if qualified.is_empty() {
                        continue; // an external type (`Vec::new`, `String::from`)
                    }
                    &qualified
                }
            };
            let type_ref = r.kind != RefKind::Call;
            let cands: Vec<&(&str, NodeId, NodeKind)> = cands
                .iter()
                .filter(|c| !type_ref || c.2 != NodeKind::Function)
                .collect();
            let source = match &r.source {
                Some(s) => f
                    .defs
                    .iter()
                    .position(|d| &d.name == s && d.kind != NodeKind::Function)
                    .map(|j| fids[j].clone()),
                None => r.from.and_then(|i| fids.get(i as usize).cloned()),
            };
            let source = source.unwrap_or_else(|| path.clone());
            let mut targets: Vec<(&NodeId, Confidence)> = Vec::new();
            for tier in 0..3 {
                let in_tier = |p: &str| match tier {
                    0 => p == path,
                    1 => imports.contains(p),
                    _ => dir_of(p) == dir,
                };
                let hits: Vec<&NodeId> = cands
                    .iter()
                    .filter(|c| in_tier(c.0))
                    .map(|c| &c.1)
                    .collect();
                if !hits.is_empty() {
                    let conf = match (hits.len(), tier) {
                        (1, 2) => Confidence::Inferred { score: 0.8 },
                        (1, _) => Confidence::Extracted,
                        _ => Confidence::Ambiguous,
                    };
                    targets = hits.into_iter().take(3).map(|h| (h, conf)).collect();
                    break;
                }
            }
            // A receiver call's target type is unknown: no project-wide guess.
            if targets.is_empty() && !r.method {
                targets = match cands.len() {
                    1 => vec![(&cands[0].1, Confidence::Inferred { score: 0.6 })],
                    2..=3 => cands
                        .iter()
                        .map(|c| (&c.1, Confidence::Ambiguous))
                        .collect(),
                    _ => Vec::new(),
                };
            }
            let kind = match r.kind {
                RefKind::Call => EdgeKind::Calls,
                RefKind::Extends => EdgeKind::Extends,
                RefKind::Implements => EdgeKind::Implements,
            };
            for (t, conf) in targets {
                g.edge(&source, t, kind, conf);
            }
        }
    }

    // Config / env layer.
    for (path, f) in files {
        for k in &f.env_keys {
            let id = format!("env:{k}");
            g.node(&id, NodeKind::EnvVar, k, Some(path), None, "");
            g.edge(path, &id, EdgeKind::Contains, Confidence::Extracted);
        }
    }
    for (path, f) in files {
        let fids = &ids[path.as_str()];
        for r in &f.env_reads {
            let id = format!("env:{}", r.text);
            g.node(&id, NodeKind::EnvVar, &r.text, None, None, "");
            let src = r
                .from
                .and_then(|i| fids.get(i as usize))
                .map_or(path.as_str(), String::as_str);
            g.edge(src, &id, EdgeKind::ReadsEnv, Confidence::Extracted);
        }
    }

    // Database-schema layer.
    crate::db::layer(&mut g, files, &ids, live);

    // Rationale layer: WHY/NOTE comments and ADR mentions.
    for (path, f) in files {
        let fids = &ids[path.as_str()];
        for (k, n) in f.notes.iter().enumerate() {
            let id = format!("{path}#note:{}", k + 1);
            let name: String = n.text.chars().take(60).collect();
            g.node(
                &id,
                NodeKind::Decision,
                &name,
                Some(path),
                Some((n.line, n.line)),
                &n.text,
            );
            let src = n
                .from
                .and_then(|i| fids.get(i as usize))
                .map_or(path.as_str(), String::as_str);
            g.edge(src, &id, EdgeKind::DecidedBy, Confidence::Extracted);
        }
        if f.kind == FileKind::Adr {
            if let Some(node) = g.nodes.get_mut(path) {
                node.text.clone_from(&f.title);
            }
            for m in &f.mentions {
                if let Some(p) = idx.exact(m.trim_start_matches("./")) {
                    g.edge(p, path, EdgeKind::DecidedBy, Confidence::Extracted);
                    continue;
                }
                let last = m.rsplit([':', '.', '/']).next().unwrap_or(m);
                let hits = by_name.get(last).map(Vec::as_slice).unwrap_or_default();
                if (1..=3).contains(&hits.len()) {
                    for (_, id, _) in hits {
                        g.edge(
                            id,
                            path,
                            EdgeKind::DecidedBy,
                            Confidence::Inferred { score: 0.5 },
                        );
                    }
                }
            }
        }
    }

    // Git layer.
    for (a, b, ratio) in &git.cochange {
        if files.contains_key(a) && files.contains_key(b) {
            g.edge(
                a,
                b,
                EdgeKind::CoChanges { ratio: *ratio },
                Confidence::Inferred { score: *ratio },
            );
        }
    }
    for n in g.nodes.values_mut() {
        if let Some(f) = &n.file {
            n.churn_30d = git
                .churn
                .get(f.to_string_lossy().as_ref())
                .copied()
                .unwrap_or(0);
        }
    }

    let mut degree: HashMap<&str, u32> = HashMap::new();
    for e in g.edges.values() {
        if e.kind != EdgeKind::Contains {
            *degree.entry(e.source.as_str()).or_default() += 1;
            *degree.entry(e.target.as_str()).or_default() += 1;
        }
    }
    let degree: HashMap<String, u32> = degree.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    for n in g.nodes.values_mut() {
        n.degree = degree.get(&n.id).copied().unwrap_or(0);
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{classify, parse};

    fn facts(files: &[(&str, &str)]) -> BTreeMap<String, FileFacts> {
        files
            .iter()
            .filter_map(|(p, t)| {
                let (k, l) = classify(p)?;
                Some(((*p).to_owned(), parse(k, l, t)))
            })
            .collect()
    }

    fn has(g: &Derived, s: &str, t: &str, kind: &str) -> Option<Confidence> {
        g.edges
            .get(&(
                s.to_owned(),
                t.to_owned(),
                match kind {
                    "calls" => "calls",
                    "imports" => "imports",
                    "implements" => "implements",
                    "extends" => "extends",
                    "reads_env" => "reads_env",
                    "decided_by" => "decided_by",
                    _ => "contains",
                },
            ))
            .map(|e| e.confidence)
    }

    #[test]
    fn cross_file_rust() {
        let files = facts(&[
            (
                "src/lib.rs",
                "mod store;\nuse crate::store::Db;\npub fn run() { let d = Db::open(); d.save(); }\n",
            ),
            (
                "src/store.rs",
                "pub struct Db;\nimpl Db {\n pub fn open() -> Self { Db }\n pub fn save(&self) { let _ = std::env::var(\"DB_URL\"); }\n}\n// WHY: one db per process\n",
            ),
            (".env", "DB_URL=postgres://secret\n"),
            (
                "docs/decisions/0001-db.md",
                "# Use one Db\nSee `Db` in `src/store.rs`.\n",
            ),
        ]);
        let g = derive(&files, &GitStats::default(), &[]);
        assert_eq!(
            has(&g, "src/lib.rs", "src/store.rs", "imports"),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            has(&g, "src/lib.rs#run", "src/store.rs#Db.open", "calls"),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            has(&g, "src/lib.rs#run", "src/store.rs#Db.save", "calls"),
            Some(Confidence::Extracted)
        );
        assert!(has(&g, "src/store.rs#Db", "src/store.rs#Db.save", "contains").is_some());
        assert!(has(&g, "src/store.rs#Db.save", "env:DB_URL", "reads_env").is_some());
        assert_eq!(
            g.nodes["env:DB_URL"].file.as_deref(),
            Some(std::path::Path::new(".env"))
        );
        assert!(has(&g, "src/store.rs", "src/store.rs#note:1", "decided_by").is_some());
        assert!(
            has(
                &g,
                "src/store.rs",
                "docs/decisions/0001-db.md",
                "decided_by"
            )
            .is_some()
        );
        assert!(
            has(
                &g,
                "src/store.rs#Db",
                "docs/decisions/0001-db.md",
                "decided_by"
            )
            .is_some()
        );
        assert_eq!(g.nodes["docs/decisions/0001-db.md"].name, "Use one Db");
        assert!(g.nodes["src/lib.rs#run"].degree >= 2);
        assert!(
            !g.nodes
                .values()
                .any(|n| n.text.contains("secret") || n.name.contains("secret"))
        );
    }

    #[test]
    fn imports_per_language_and_confidence() {
        let files = facts(&[
            (
                "web/app.ts",
                "import { helper } from './util';\nexport function main() { helper(); other(); }\n",
            ),
            ("web/util.ts", "export function helper() {}\n"),
            ("lib/x.ts", "export function other() {}\n"),
            ("py/pkg/a.py", "from .b import thing\nimport pkg.c\n"),
            ("py/pkg/b.py", "def thing(): pass\n"),
            ("py/pkg/c.py", ""),
            (
                "go/cmd/main.go",
                "package main\nimport \"example.com/m/internal/srv\"\nfunc main() { srv.Start() }\n",
            ),
            ("go/internal/srv/srv.go", "package srv\nfunc Start() {}\n"),
            ("java/a/b/C.java", "package a.b;\nclass C implements I {}\n"),
            ("java/a/b/I.java", "package a.b;\ninterface I {}\n"),
            ("java/x/Main.java", "import a.b.C;\nclass Main {}\n"),
            ("c/main.c", "#include \"util.h\"\n"),
            ("c/util.h", "int util(void);\n"),
            ("rb/a.rb", "require_relative 'b'\n"),
            ("rb/b.rb", ""),
        ]);
        let g = derive(&files, &GitStats::default(), &[]);
        assert_eq!(
            has(&g, "web/app.ts", "web/util.ts", "imports"),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            has(&g, "web/app.ts#main", "web/util.ts#helper", "calls"),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            has(&g, "web/app.ts#main", "lib/x.ts#other", "calls"),
            Some(Confidence::Inferred { score: 0.6 })
        );
        assert_eq!(
            has(&g, "py/pkg/a.py", "py/pkg/b.py", "imports"),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            has(&g, "py/pkg/a.py", "py/pkg/c.py", "imports"),
            Some(Confidence::Inferred { score: 0.8 })
        );
        assert!(has(&g, "go/cmd/main.go", "go/internal/srv/srv.go", "imports").is_some());
        assert!(
            has(
                &g,
                "go/cmd/main.go#main",
                "go/internal/srv/srv.go#Start",
                "calls"
            )
            .is_some()
        );
        assert_eq!(
            has(&g, "java/x/Main.java", "java/a/b/C.java", "imports"),
            Some(Confidence::Extracted)
        );
        assert!(has(&g, "java/a/b/C.java#C", "java/a/b/I.java#I", "implements").is_some());
        assert_eq!(
            has(&g, "c/main.c", "c/util.h", "imports"),
            Some(Confidence::Extracted)
        );
        assert_eq!(
            has(&g, "rb/a.rb", "rb/b.rb", "imports"),
            Some(Confidence::Extracted)
        );
    }

    #[test]
    fn std_method_calls_do_not_hit_project_methods() {
        assert!(
            STD_METHODS.windows(2).all(|w| w[0] < w[1]),
            "STD_METHODS must stay sorted"
        );
        let files = facts(&[
            (
                "a/keys.rs",
                "pub struct Keys;\nimpl Keys { pub fn new() -> Self { Keys } pub fn map(&self) {} pub fn rebind(&self) {} }\n",
            ),
            (
                "b/use.rs",
                "fn go(v: Vec<u8>, k: Keys) { let _ = v.iter().map(|x| x); let s = format!(\"x\"); k.rebind(); let _ = String::new(); let _ = Keys::new(); }\nfn format() {}\n",
            ),
        ]);
        let g = derive(&files, &GitStats::default(), &[]);
        assert!(
            !g.edges
                .keys()
                .any(|k| k.1 == "a/keys.rs#Keys.map" && k.2 == "calls"),
            "std .map() linked"
        );
        assert!(
            !g.edges
                .keys()
                .any(|k| k.0 == "b/use.rs#go" && k.1 == "b/use.rs#format"),
            "format! macro linked"
        );
        assert!(g.edges.contains_key(&(
            "b/use.rs#go".to_owned(),
            "a/keys.rs#Keys.new".to_owned(),
            "calls"
        )));
        assert_eq!(
            g.edges
                .keys()
                .filter(|k| k.1 == "a/keys.rs#Keys.new" && k.2 == "calls")
                .count(),
            1,
            "String::new linked"
        );
        // A receiver call outside same file / imports / directory is not guessed project-wide.
        assert!(
            !g.edges
                .keys()
                .any(|k| k.1 == "a/keys.rs#Keys.rebind" && k.2 == "calls")
        );
    }

    #[test]
    fn duplicate_names_are_ambiguous_and_ids_stable() {
        let files = facts(&[
            ("a/one.py", "def go():\n    run()\n"),
            ("b/x.py", "def run(): pass\n"),
            ("c/y.py", "def run(): pass\ndef run(): pass\n"),
        ]);
        let g = derive(&files, &GitStats::default(), &[]);
        assert_eq!(
            has(&g, "a/one.py#go", "b/x.py#run", "calls"),
            Some(Confidence::Ambiguous)
        );
        assert!(g.nodes.contains_key("c/y.py#run~2"));
        let git = GitStats {
            cochange: vec![("a/one.py".into(), "b/x.py".into(), 0.75)],
            churn: [("b/x.py".to_owned(), 4)].into(),
            ..GitStats::default()
        };
        let g = derive(&files, &git, &[]);
        assert!(
            g.edges
                .contains_key(&("a/one.py".to_owned(), "b/x.py".to_owned(), "co_changes"))
        );
        assert_eq!(g.nodes["b/x.py#run"].churn_30d, 4);
    }
}
