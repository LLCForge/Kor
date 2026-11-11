//! Monorepo workspace awareness (spec §15.15).
//!
//! [`detect`] reads the build tools' own manifests (never writes them) and finds the packages of
//! Cargo workspaces, pnpm / yarn / npm workspaces (plus Turborepo, Nx and Lerna on top), Bazel and
//! Buck packages, Go workspaces, Gradle multi-projects and .NET solutions. On that package graph,
//! [`Workspace::affected`] answers which packages a change touches (changed packages plus every
//! transitive dependent) and [`Workspace::narrowed_commands`] limits build/test commands to them.
//! Everything is local and LLM-free; the only process started is `git` in [`changed_files`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;
use serde_json::Value;

/// Directory depth scanned below the workspace root.
const MAX_DEPTH: usize = 8;
/// Directory entries visited at most (very large trees are cut, not walked forever).
const MAX_ENTRIES: usize = 200_000;
/// Manifest files remembered during the scan.
const MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "project.json",
    "BUILD",
    "BUILD.bazel",
    "BUCK",
];
/// A change to one of these files at the root affects every package.
const GLOBAL_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "yarn.lock",
    "turbo.json",
    "nx.json",
    "lerna.json",
    "go.work",
    "go.work.sum",
    "settings.gradle",
    "settings.gradle.kts",
    "WORKSPACE",
    "WORKSPACE.bazel",
    "MODULE.bazel",
    ".buckconfig",
    "Directory.Build.props",
    "Directory.Packages.props",
];

/// The build tool a package belongs to (and the monorepo kinds found).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Cargo,
    Pnpm,
    Yarn,
    Npm,
    Lerna,
    Nx,
    Turbo,
    Bazel,
    Buck,
    Go,
    Gradle,
    Dotnet,
}

impl Tool {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Npm => "npm",
            Self::Lerna => "lerna",
            Self::Nx => "nx",
            Self::Turbo => "turbo",
            Self::Bazel => "bazel",
            Self::Buck => "buck",
            Self::Go => "go",
            Self::Gradle => "gradle",
            Self::Dotnet => "dotnet",
        }
    }

    fn is_js(self) -> bool {
        matches!(
            self,
            Self::Pnpm | Self::Yarn | Self::Npm | Self::Lerna | Self::Nx | Self::Turbo
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Package {
    /// The tool's own name: crate name, npm name, Go module path, Gradle path without the leading
    /// `:`, Bazel label (`//dir`), .NET project name.
    pub name: String,
    /// Package directory (absolute).
    pub root: PathBuf,
    /// The manifest that defines it (`Cargo.toml`, `package.json`, `go.mod`, `*.csproj`, ...).
    pub manifest: PathBuf,
    pub tool: Tool,
    /// Names of the workspace packages this one depends on directly.
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Workspace {
    pub root: PathBuf,
    /// Monorepo tools found (several can stack: pnpm + Turborepo, Cargo + Bazel, ...).
    pub kinds: Vec<Tool>,
    /// Sorted by name; empty when `root` is not a monorepo.
    pub packages: Vec<Package>,
}

/// Find the monorepo packages under `root`. Unreadable or malformed manifests are skipped.
#[must_use]
pub fn detect(root: &Path) -> Workspace {
    let root = lexical(&plain(root));
    let files = scan(&root);
    let mut ws = Workspace {
        root,
        ..Workspace::default()
    };
    cargo(&mut ws, &files);
    js(&mut ws, &files);
    nx(&mut ws, &files);
    go(&mut ws);
    gradle(&mut ws);
    dotnet(&mut ws);
    bazel(&mut ws, &files);
    let names: BTreeSet<String> = ws.packages.iter().map(|p| p.name.clone()).collect();
    for p in &mut ws.packages {
        let own = p.name.clone();
        p.deps.retain(|d| *d != own && names.contains(d));
        p.deps.sort();
        p.deps.dedup();
    }
    ws.packages
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.root.cmp(&b.root)));
    ws
}

impl Workspace {
    #[must_use]
    pub fn is_monorepo(&self) -> bool {
        !self.packages.is_empty()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// The innermost package containing `path` (absolute, or relative to the root).
    #[must_use]
    pub fn package_of(&self, path: &Path) -> Option<&Package> {
        let p = self.absolute(path);
        self.packages
            .iter()
            .filter(|k| p.starts_with(&k.root))
            .max_by_key(|k| k.root.components().count())
    }

    /// Package dependency edges `(from, to)`: `from` depends on `to`.
    #[must_use]
    pub fn edges(&self) -> Vec<(&str, &str)> {
        self.packages
            .iter()
            .flat_map(|p| p.deps.iter().map(|d| (p.name.as_str(), d.as_str())))
            .collect()
    }

    /// Every package that depends on `name`, directly or transitively (sorted by name).
    #[must_use]
    pub fn dependents(&self, name: &str) -> Vec<&Package> {
        let mut out = self.closure(BTreeSet::from([name]));
        out.retain(|p| p.name != name);
        out
    }

    /// Packages touched by `changed` files plus all their transitive dependents. A changed
    /// workspace-level file at the root (lockfile, root manifest) affects every package.
    #[must_use]
    pub fn affected<P: AsRef<Path>>(&self, changed: &[P]) -> Vec<&Package> {
        let mut hit = BTreeSet::new();
        for f in changed {
            let abs = self.absolute(f.as_ref());
            let global = abs.parent() == Some(self.root.as_path())
                && abs.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    GLOBAL_FILES.contains(&n.as_ref())
                        || Path::new(n.as_ref())
                            .extension()
                            .is_some_and(|e| e == "sln")
                });
            if global {
                return self.packages.iter().collect();
            }
            if let Some(p) = self.package_of(&abs) {
                hit.insert(p.name.as_str());
            }
        }
        self.closure(hit)
    }

    /// Build/test commands limited to `affected`, one or more per tool, run from the root.
    #[must_use]
    pub fn narrowed_commands(&self, affected: &[&Package]) -> Vec<String> {
        let mut by: BTreeMap<Tool, Vec<&Package>> = BTreeMap::new();
        for p in affected {
            let tool = if p.tool.is_js() { Tool::Npm } else { p.tool };
            by.entry(tool).or_default().push(p);
        }
        let mut out = Vec::new();
        for (tool, pkgs) in by {
            let names = || pkgs.iter().map(|p| quote(&p.name));
            match tool {
                Tool::Cargo => out.push(format!(
                    "cargo test {}",
                    names()
                        .map(|n| format!("-p {n}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                // Every JS flavour lands here; the stacked tool decides the command.
                Tool::Npm | Tool::Pnpm | Tool::Yarn | Tool::Lerna | Tool::Nx | Tool::Turbo => {
                    out.extend(self.js_commands(&pkgs));
                }
                Tool::Go => out.push(format!(
                    "go test {}",
                    pkgs.iter()
                        .map(|p| match self.rel(&p.root).as_str() {
                            "" => "./...".to_owned(),
                            r => quote(&format!("./{r}/...")),
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                Tool::Gradle => {
                    let wrapper = self.root.join("gradlew").is_file()
                        || self.root.join("gradlew.bat").is_file();
                    out.push(format!(
                        "{} {}",
                        if wrapper { "./gradlew" } else { "gradle" },
                        names()
                            .map(|n| format!(":{n}:test"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ));
                }
                Tool::Dotnet => out.extend(
                    pkgs.iter()
                        .map(|p| format!("dotnet test {}", quote(&self.rel(&p.manifest)))),
                ),
                Tool::Bazel | Tool::Buck => out.push(format!(
                    "{} test {}",
                    if tool == Tool::Bazel {
                        "bazel"
                    } else {
                        "buck2"
                    },
                    pkgs.iter()
                        .map(|p| quote(&format!("{}/...", p.name.trim_end_matches('/'))))
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
            }
        }
        out
    }

    fn js_commands(&self, pkgs: &[&Package]) -> Vec<String> {
        let names: Vec<String> = pkgs.iter().map(|p| quote(&p.name)).collect();
        let flags = |f: &str| {
            names
                .iter()
                .map(|n| format!("{f}{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        if self.kinds.contains(&Tool::Nx) {
            return vec![format!("nx run-many -t test -p {}", names.join(","))];
        }
        if self.kinds.contains(&Tool::Turbo) {
            return vec![format!("turbo run test {}", flags("--filter="))];
        }
        if self.kinds.contains(&Tool::Pnpm) {
            return vec![format!("pnpm {} test", flags("--filter "))];
        }
        if self.kinds.contains(&Tool::Yarn) {
            return names
                .iter()
                .map(|n| format!("yarn workspace {n} run test"))
                .collect();
        }
        if self.kinds.contains(&Tool::Npm) {
            return vec![format!("npm test {}", flags("-w "))];
        }
        vec![format!("lerna run test {}", flags("--scope "))]
    }

    /// Packages in `start` and everything depending on them, sorted by name.
    fn closure(&self, start: BTreeSet<&str>) -> Vec<&Package> {
        let mut users: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (from, to) in self.edges() {
            users.entry(to).or_default().push(from);
        }
        let mut seen = start.clone();
        let mut queue: VecDeque<&str> = start.into_iter().collect();
        while let Some(n) = queue.pop_front() {
            for &u in users.get(n).into_iter().flatten() {
                if seen.insert(u) {
                    queue.push_back(u);
                }
            }
        }
        self.packages
            .iter()
            .filter(|p| seen.contains(p.name.as_str()))
            .collect()
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        let p = plain(path);
        lexical(&if p.is_relative() {
            self.root.join(p)
        } else {
            p
        })
    }

    /// `path` relative to the root with `/` separators (`""` for the root itself).
    #[must_use]
    pub fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn add(&mut self, dir: &str, manifest: &str, name: String, tool: Tool, deps: Vec<String>) {
        let root = lexical(&self.root.join(dir));
        // First detector wins a directory (a JS package that is also an Nx project, ...).
        if name.is_empty() || self.packages.iter().any(|p| p.root == root) {
            return;
        }
        self.packages.push(Package {
            name,
            manifest: lexical(&self.root.join(dir).join(manifest)),
            root,
            tool,
            deps,
        });
    }

    fn kind(&mut self, t: Tool) {
        if !self.kinds.contains(&t) {
            self.kinds.push(t);
        }
    }
}

/// Files changed against `base` (default `HEAD`: staged and unstaged edits), plus untracked
/// files, as absolute paths. Runs `git` in `root`.
///
/// # Errors
/// An invalid ref, `git` missing, or `git` failing (not a repository, unknown ref).
pub fn changed_files(root: &Path, base: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let base = base.unwrap_or("HEAD");
    // A ref starting with `-` would be read as an option.
    if base.is_empty() || base.starts_with('-') || base.chars().any(char::is_whitespace) {
        return Err(format!("invalid git ref `{base}`"));
    }
    let mut out = git(root, &["diff", "--name-only", "--relative", base, "--"])?;
    out.extend(git(root, &["ls-files", "--others", "--exclude-standard"])?);
    out.sort();
    out.dedup();
    Ok(out.into_iter().map(|f| root.join(f)).collect())
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------------------------

/// Relative paths (`/`-separated) of every manifest in [`MARKERS`] below `root`, honoring
/// `.gitignore` and skipping hidden, `node_modules`, `target` and `bazel-*` directories.
fn scan(root: &Path) -> Vec<String> {
    let walk = ignore::WalkBuilder::new(root)
        .max_depth(Some(MAX_DEPTH))
        .require_git(false)
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.file_type().is_some_and(|t| t.is_dir())
                && (n == "node_modules" || n == "target" || n.starts_with("bazel-")))
        })
        .build();
    let mut out = Vec::new();
    for e in walk.filter_map(Result::ok).take(MAX_ENTRIES) {
        let n = e.file_name().to_string_lossy();
        if MARKERS.contains(&n.as_ref())
            && e.file_type().is_some_and(|t| t.is_file())
            && let Ok(r) = e.path().strip_prefix(root)
        {
            out.push(r.to_string_lossy().replace('\\', "/"));
        }
    }
    out.sort();
    out
}

/// Directories (relative, non-root) holding `marker` and matching the include globs but none of
/// the excludes. A `!pattern` include counts as an exclude (pnpm, npm).
fn matching_dirs(
    files: &[String],
    marker: &str,
    includes: &[String],
    excludes: &[String],
) -> Vec<String> {
    let (neg, pos): (Vec<&String>, Vec<&String>) =
        includes.iter().partition(|p| p.starts_with('!'));
    let inc = globs(pos.into_iter().map(String::as_str));
    let exc = globs(
        excludes
            .iter()
            .map(String::as_str)
            .chain(neg.into_iter().map(|p| &p[1..])),
    );
    files
        .iter()
        .filter_map(|f| {
            let (dir, name) = f.rsplit_once('/')?;
            (name == marker && inc.is_match(dir) && !exc.is_match(dir)).then(|| dir.to_owned())
        })
        .collect()
}

fn globs<'a>(patterns: impl Iterator<Item = &'a str>) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let p = p.trim().trim_start_matches("./").trim_end_matches('/');
        if let Ok(g) = GlobBuilder::new(p).literal_separator(true).build() {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&read(p)?).ok()
}

fn read_toml(p: &Path) -> Option<toml::Table> {
    read(p)?.parse().ok()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// `"x"` / `'x'` / `x` → `x`, dropping a trailing `# comment`.
fn unquote(s: &str) -> String {
    let s = s.split(" #").next().unwrap_or(s).trim();
    s.trim_matches(|c| c == '"' || c == '\'').to_owned()
}

/// Every `'...'` or `"..."` string literal in `s`, in order.
fn quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut open: Option<(char, String)> = None;
    for c in s.chars() {
        match &mut open {
            Some((q, buf)) if c == *q => {
                out.push(std::mem::take(buf));
                open = None;
            }
            Some((_, buf)) => buf.push(c),
            None if c == '"' || c == '\'' => open = Some((c, String::new())),
            None => {}
        }
    }
    out
}

/// Fold `.` and `..` without touching the disk.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// Drop the Windows verbatim prefix `\\?\` that `canonicalize` adds, so canonical and plain
/// spellings of the same path compare equal.
fn plain(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p.to_owned(),
    }
}

/// A shell word safe in both POSIX shells and `PowerShell` (`@scope/x` would splat there).
fn quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-/:+=,".contains(c))
    {
        s.to_owned()
    } else {
        // shortcut: single quotes inside a name are dropped; no real package name has one.
        format!("'{}'", s.replace('\'', ""))
    }
}

// ---------------------------------------------------------------------------------------------
// Detectors
// ---------------------------------------------------------------------------------------------

/// Cargo: `[workspace] members` / `exclude` globs; the root crate too when it has `[package]`.
fn cargo(ws: &mut Workspace, files: &[String]) {
    let Some(doc) = read_toml(&ws.root.join("Cargo.toml")) else {
        return;
    };
    let Some(w) = doc.get("workspace").and_then(toml::Value::as_table) else {
        return;
    };
    let list = |k: &str| -> Vec<String> {
        w.get(k)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let mut dirs = matching_dirs(files, "Cargo.toml", &list("members"), &list("exclude"));
    if doc.contains_key("package") {
        dirs.insert(0, String::new());
    }
    for dir in dirs {
        let Some(t) = read_toml(&ws.root.join(&dir).join("Cargo.toml")) else {
            continue;
        };
        let Some(name) = t
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        // shortcut: `[target.*.dependencies]` are not read; add them if a repo needs it.
        let deps = ["dependencies", "dev-dependencies", "build-dependencies"]
            .iter()
            .filter_map(|k| t.get(*k).and_then(toml::Value::as_table))
            .flat_map(|tab| {
                tab.iter().map(|(k, v)| {
                    v.get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(k)
                        .to_owned()
                })
            })
            .collect();
        ws.add(&dir, "Cargo.toml", name.to_owned(), Tool::Cargo, deps);
        ws.kind(Tool::Cargo);
    }
}

/// `packages:` of a `pnpm-workspace.yaml` (block list or inline `[a, b]`), without a YAML parser.
fn pnpm_packages(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t', '-']) {
            inside = false;
            if let Some(rest) = t.strip_prefix("packages:") {
                let rest = rest.trim();
                if rest.starts_with('[') {
                    out.extend(
                        rest.trim_matches(|c| c == '[' || c == ']')
                            .split(',')
                            .map(unquote)
                            .filter(|s| !s.is_empty()),
                    );
                } else {
                    inside = true;
                }
            }
            continue;
        }
        if inside && let Some(item) = t.strip_prefix('-') {
            out.push(unquote(item));
        }
    }
    out
}

/// pnpm (`pnpm-workspace.yaml`), yarn / npm (`package.json` `workspaces`), Lerna (`lerna.json`),
/// and Turborepo on top of any of them.
fn js(ws: &mut Workspace, files: &[String]) {
    let root_pkg = read_json(&ws.root.join("package.json"));
    let lerna = read_json(&ws.root.join("lerna.json"));
    let (patterns, tool) = if let Some(t) = read(&ws.root.join("pnpm-workspace.yaml")) {
        (pnpm_packages(&t), Tool::Pnpm)
    } else if let Some(w) = root_pkg.as_ref().and_then(|p| p.get("workspaces")) {
        let list = if w.is_array() {
            strings(Some(w))
        } else {
            strings(w.get("packages"))
        };
        let yarn = ws.root.join("yarn.lock").is_file() || ws.root.join(".yarnrc.yml").is_file();
        (list, if yarn { Tool::Yarn } else { Tool::Npm })
    } else if let Some(l) = &lerna {
        let list = strings(l.get("packages"));
        let list = if list.is_empty() {
            vec!["packages/*".to_owned()]
        } else {
            list
        };
        (list, Tool::Lerna)
    } else {
        return;
    };
    for dir in matching_dirs(files, "package.json", &patterns, &[]) {
        let Some(j) = read_json(&ws.root.join(&dir).join("package.json")) else {
            continue;
        };
        let name = j
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| dir.clone(), str::to_owned);
        let deps = [
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
        ]
        .iter()
        .filter_map(|k| j.get(*k).and_then(Value::as_object))
        .flat_map(|o| o.keys().cloned())
        .collect();
        ws.add(&dir, "package.json", name, tool, deps);
    }
    ws.kind(tool);
    if lerna.is_some() {
        ws.kind(Tool::Lerna);
    }
    if ws.root.join("turbo.json").is_file() {
        ws.kind(Tool::Turbo);
    }
}

/// Nx: `nx.json` at the root, a project per `project.json` (name, `implicitDependencies`).
fn nx(ws: &mut Workspace, files: &[String]) {
    if !ws.root.join("nx.json").is_file() {
        return;
    }
    ws.kind(Tool::Nx);
    for f in files {
        let Some((dir, "project.json")) = f.rsplit_once('/') else {
            continue;
        };
        let Some(j) = read_json(&ws.root.join(f)) else {
            continue;
        };
        let name = j.get("name").and_then(Value::as_str).map_or_else(
            || dir.rsplit('/').next().unwrap_or(dir).to_owned(),
            str::to_owned,
        );
        let deps = strings(j.get("implicitDependencies"))
            .into_iter()
            .filter(|d| !d.starts_with('!'))
            .collect();
        ws.add(dir, "project.json", name, Tool::Nx, deps);
    }
}

/// `use` directives of a `go.work` (single and block form).
fn go_work_uses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block = false;
    for line in text.lines() {
        let t = line.split("//").next().unwrap_or("").trim();
        if block {
            if t == ")" {
                block = false;
            } else if !t.is_empty() {
                out.push(unquote(t));
            }
        } else if let Some(rest) = t.strip_prefix("use")
            && rest.starts_with([' ', '\t', '('])
        {
            match rest.trim() {
                "(" => block = true,
                r => out.push(unquote(r)),
            }
        }
    }
    out
}

/// Go workspaces: modules listed by `go.work`; deps are `require`/`replace` lines naming
/// another module of the workspace.
fn go(ws: &mut Workspace) {
    let Some(text) = read(&ws.root.join("go.work")) else {
        return;
    };
    for u in go_work_uses(&text) {
        let dir = lexical(Path::new(&u)).to_string_lossy().replace('\\', "/");
        let Some(m) = read(&ws.root.join(&dir).join("go.mod")) else {
            continue;
        };
        let Some(name) = m
            .lines()
            .find_map(|l| l.trim().strip_prefix("module "))
            .map(unquote)
        else {
            continue;
        };
        // Candidate tokens; the final pass keeps only workspace module paths.
        let deps = m
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                let l = l
                    .strip_prefix("require")
                    .or_else(|| l.strip_prefix("replace"))
                    .unwrap_or(l);
                l.split_whitespace().next().map(unquote)
            })
            .collect();
        ws.add(&dir, "go.mod", name, Tool::Go, deps);
        ws.kind(Tool::Go);
    }
}

/// Gradle: `include` lines of `settings.gradle(.kts)`; deps are `project(":x")` references.
fn gradle(ws: &mut Workspace) {
    let Some(text) = ["settings.gradle.kts", "settings.gradle"]
        .iter()
        .find_map(|f| read(&ws.root.join(f)))
    else {
        return;
    };
    let includes: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("include") && !l.starts_with("includeBuild"))
        .flat_map(quoted)
        .map(|p| p.trim_start_matches(':').to_owned())
        .filter(|p| !p.is_empty())
        .collect();
    for name in includes {
        // shortcut: a `project(":x").projectDir = ...` override is not followed; dir = path.
        let dir = name.replace(':', "/");
        let (manifest, build) = ["build.gradle.kts", "build.gradle"]
            .iter()
            .find_map(|f| read(&ws.root.join(&dir).join(f)).map(|t| (*f, t)))
            .unwrap_or(("build.gradle", String::new()));
        let deps = build
            .split("project(")
            .skip(1)
            .filter_map(|s| quoted(s).into_iter().next())
            .map(|d| d.trim_start_matches(':').to_owned())
            .collect();
        ws.add(&dir, manifest, name, Tool::Gradle, deps);
        ws.kind(Tool::Gradle);
    }
}

/// .NET: project entries of every `*.sln` at the root; deps are `<ProjectReference>`s.
fn dotnet(ws: &mut Workspace) {
    let Ok(rd) = std::fs::read_dir(&ws.root) else {
        return;
    };
    let mut slns: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sln") && p.is_file())
        .collect();
    slns.sort();
    // (name, manifest relative to the root) for every project in the solution(s).
    let mut projects: Vec<(String, String)> = Vec::new();
    for sln in slns {
        for line in read(&sln).unwrap_or_default().lines() {
            if !line.trim_start().starts_with("Project(") {
                continue;
            }
            let parts: Vec<&str> = line.split('"').collect();
            if let (Some(name), Some(path)) = (parts.get(3), parts.get(5)) {
                let path = path.replace('\\', "/");
                if path.ends_with("proj") && !projects.iter().any(|(_, p)| *p == path) {
                    projects.push(((*name).to_owned(), path));
                }
            }
        }
    }
    let by_manifest: Vec<(PathBuf, String)> = projects
        .iter()
        .map(|(n, p)| (lexical(&ws.root.join(p)), n.clone()))
        .collect();
    for (name, path) in &projects {
        let manifest = lexical(&ws.root.join(path));
        let base = manifest.parent().map(Path::to_owned).unwrap_or_default();
        let deps = read(&manifest)
            .unwrap_or_default()
            .split("<ProjectReference")
            .skip(1)
            .filter_map(|s| {
                let inc = s.split_once("Include=")?.1;
                let r = quoted(inc).into_iter().next()?.replace('\\', "/");
                let target = lexical(&base.join(r));
                by_manifest
                    .iter()
                    .find(|(m, _)| *m == target)
                    .map(|(_, n)| n.clone())
            })
            .collect();
        let (dir, file) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        ws.add(dir, file, name.clone(), Tool::Dotnet, deps);
        ws.kind(Tool::Dotnet);
    }
}

/// Bazel (`WORKSPACE`/`MODULE.bazel`) and Buck (`.buckconfig`): every directory with a
/// `BUILD`/`BUILD.bazel` (Bazel) or `BUCK` file is a package `//dir`; deps are `//other:...`
/// labels in it.
fn bazel(ws: &mut Workspace, files: &[String]) {
    let is_bazel = ["WORKSPACE", "WORKSPACE.bazel", "MODULE.bazel"]
        .iter()
        .any(|f| ws.root.join(f).is_file());
    let is_buck = ws.root.join(".buckconfig").is_file();
    for (on, tool, markers) in [
        (is_bazel, Tool::Bazel, &["BUILD", "BUILD.bazel"][..]),
        (is_buck, Tool::Buck, &["BUCK"][..]),
    ] {
        if !on {
            continue;
        }
        ws.kind(tool);
        for f in files {
            let (dir, file) = f.rsplit_once('/').unwrap_or(("", f.as_str()));
            if !markers.contains(&file) {
                continue;
            }
            // shortcut: labels are read from double-quoted strings only (Starlark allows ').
            let deps = read(&ws.root.join(f))
                .unwrap_or_default()
                .split('"')
                .skip(1)
                .step_by(2)
                .filter_map(|s| s.strip_prefix("//"))
                .map(|s| format!("//{}", s.split(':').next().unwrap_or("")))
                .collect();
            ws.add(dir, file, format!("//{dir}"), tool, deps);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn names(ps: &[&Package]) -> Vec<String> {
        ps.iter().map(|p| p.name.clone()).collect()
    }

    fn summary(ws: &Workspace) -> Vec<(String, Tool, String, Vec<String>)> {
        ws.packages
            .iter()
            .map(|p| (p.name.clone(), p.tool, ws.rel(&p.root), p.deps.clone()))
            .collect()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn cargo_workspace_members_exclude_and_deps() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            r,
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\", \"apps/cli\"]\nexclude = [\"crates/skip\"]\n",
        );
        put(
            r,
            "crates/core/Cargo.toml",
            "[package]\nname = \"core\"\n[dependencies]\nserde = \"1\"\n",
        );
        put(
            r,
            "crates/net/Cargo.toml",
            "[package]\nname = \"net\"\n[dependencies]\ncore.workspace = true\n",
        );
        put(r, "crates/skip/Cargo.toml", "[package]\nname = \"skip\"\n");
        put(
            r,
            "apps/cli/Cargo.toml",
            "[package]\nname = \"cli\"\n[dev-dependencies]\nn = { package = \"net\", path = \"../../crates/net\" }\n",
        );
        put(
            r,
            "crates/core/nested/x/Cargo.toml",
            "[package]\nname = \"deep\"\n",
        );
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Cargo]);
        assert_eq!(
            summary(&ws),
            [
                ("cli".into(), Tool::Cargo, "apps/cli".into(), s(&["net"])),
                ("core".into(), Tool::Cargo, "crates/core".into(), vec![]),
                ("net".into(), Tool::Cargo, "crates/net".into(), s(&["core"])),
            ]
        );
        assert_eq!(
            ws.narrowed_commands(&ws.affected(&["crates/core/src/lib.rs"])),
            ["cargo test -p cli -p core -p net"]
        );
    }

    #[test]
    fn pnpm_with_turbo_and_quoted_scoped_names() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            r,
            "pnpm-workspace.yaml",
            "# ws\npackages:\n  - 'packages/*'\n  - \"apps/**\"\n  - '!**/test'\ncatalog:\n  - nope/*\n",
        );
        put(r, "turbo.json", "{}");
        put(r, "packages/ui/package.json", r#"{"name":"@acme/ui"}"#);
        put(
            r,
            "apps/web/site/package.json",
            r#"{"name":"web","dependencies":{"@acme/ui":"workspace:*","react":"18"}}"#,
        );
        put(r, "apps/test/package.json", r#"{"name":"t"}"#);
        put(r, "nope/x/package.json", r#"{"name":"nope"}"#);
        put(
            r,
            "packages/ui/node_modules/dep/package.json",
            r#"{"name":"dep"}"#,
        );
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Pnpm, Tool::Turbo]);
        assert_eq!(
            summary(&ws),
            [
                ("@acme/ui".into(), Tool::Pnpm, "packages/ui".into(), vec![]),
                (
                    "web".into(),
                    Tool::Pnpm,
                    "apps/web/site".into(),
                    s(&["@acme/ui"])
                ),
            ]
        );
        let a = ws.affected(&[r.join("packages/ui/index.ts")]);
        assert_eq!(names(&a), ["@acme/ui", "web"]);
        assert_eq!(
            ws.narrowed_commands(&a),
            ["turbo run test --filter='@acme/ui' --filter=web"]
        );
    }

    #[test]
    fn pnpm_inline_list() {
        assert_eq!(pnpm_packages("packages: ['a/*', \"b\"]\n"), ["a/*", "b"]);
    }

    #[test]
    fn yarn_npm_and_lerna() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            r,
            "package.json",
            r#"{"workspaces":{"packages":["libs/*"]}}"#,
        );
        put(r, "yarn.lock", "");
        put(r, "libs/a/package.json", r#"{"name":"a"}"#);
        put(
            r,
            "libs/b/package.json",
            r#"{"name":"b","devDependencies":{"a":"*"}}"#,
        );
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Yarn]);
        let a = ws.affected(&["libs/a/x.js"]);
        assert_eq!(
            ws.narrowed_commands(&a),
            ["yarn workspace a run test", "yarn workspace b run test"]
        );

        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(r, "package.json", r#"{"workspaces":["libs/*"]}"#);
        put(r, "libs/a/package.json", r#"{"name":"a"}"#);
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Npm]);
        assert_eq!(
            ws.narrowed_commands(&ws.packages.iter().collect::<Vec<_>>()),
            ["npm test -w a"]
        );

        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(r, "lerna.json", "{}");
        put(r, "packages/p/package.json", r#"{"name":"p"}"#);
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Lerna]);
        assert_eq!(
            ws.narrowed_commands(&ws.packages.iter().collect::<Vec<_>>()),
            ["lerna run test --scope p"]
        );
    }

    #[test]
    fn nx_projects_and_implicit_deps() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(r, "nx.json", "{}");
        put(
            r,
            "apps/shop/project.json",
            r#"{"name":"shop","implicitDependencies":["auth","!x"]}"#,
        );
        put(r, "libs/auth/project.json", "{}");
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Nx]);
        assert_eq!(
            summary(&ws),
            [
                ("auth".into(), Tool::Nx, "libs/auth".into(), vec![]),
                ("shop".into(), Tool::Nx, "apps/shop".into(), s(&["auth"])),
            ]
        );
        assert_eq!(
            ws.narrowed_commands(&ws.affected(&["libs/auth/a.ts"])),
            ["nx run-many -t test -p auth,shop"]
        );
    }

    #[test]
    fn go_work_modules() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            r,
            "go.work",
            "go 1.22\n\nuse (\n\t./svc // main\n\t./lib\n)\nuse ./tools\n",
        );
        put(r, "lib/go.mod", "module example.com/lib\n\ngo 1.22\n");
        put(
            r,
            "svc/go.mod",
            "module example.com/svc\n\nrequire (\n\texample.com/lib v0.0.0\n\tgithub.com/x/y v1.0.0\n)\n",
        );
        put(
            r,
            "tools/go.mod",
            "module example.com/tools\nrequire example.com/svc v0.0.0\n",
        );
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Go]);
        let a = ws.affected(&["lib/lib.go"]);
        assert_eq!(
            names(&a),
            ["example.com/lib", "example.com/svc", "example.com/tools"]
        );
        assert_eq!(
            ws.narrowed_commands(&a),
            ["go test ./lib/... ./svc/... ./tools/..."]
        );
    }

    #[test]
    fn gradle_includes_and_project_deps() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            r,
            "settings.gradle.kts",
            "rootProject.name = \"x\"\ninclude(\":core\", \":app\")\ninclude ':libs:util'\n",
        );
        put(r, "gradlew", "");
        put(
            r,
            "app/build.gradle.kts",
            "dependencies { implementation(project(\":core\")) }\n",
        );
        put(
            r,
            "core/build.gradle",
            "dependencies { implementation project(':libs:util') }\n",
        );
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Gradle]);
        assert_eq!(
            summary(&ws),
            [
                ("app".into(), Tool::Gradle, "app".into(), s(&["core"])),
                (
                    "core".into(),
                    Tool::Gradle,
                    "core".into(),
                    s(&["libs:util"])
                ),
                ("libs:util".into(), Tool::Gradle, "libs/util".into(), vec![]),
            ]
        );
        assert_eq!(
            ws.narrowed_commands(&ws.affected(&["libs/util/src/U.kt"])),
            ["./gradlew :app:test :core:test :libs:util:test"]
        );
    }

    #[test]
    fn dotnet_solution_and_project_references() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            r,
            "App.sln",
            "Project(\"{FAE04EC0}\") = \"Web\", \"src\\Web\\Web.csproj\", \"{1}\"\nEndProject\n\
             Project(\"{FAE04EC0}\") = \"Core\", \"src\\Core\\Core.csproj\", \"{2}\"\nEndProject\n\
             Project(\"{2150E333}\") = \"Solution Items\", \"Solution Items\", \"{3}\"\nEndProject\n",
        );
        put(
            r,
            "src/Web/Web.csproj",
            "<Project><ItemGroup><ProjectReference Include=\"..\\Core\\Core.csproj\" /></ItemGroup></Project>",
        );
        put(r, "src/Core/Core.csproj", "<Project />");
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Dotnet]);
        assert_eq!(
            summary(&ws),
            [
                ("Core".into(), Tool::Dotnet, "src/Core".into(), vec![]),
                ("Web".into(), Tool::Dotnet, "src/Web".into(), s(&["Core"])),
            ]
        );
        assert_eq!(
            ws.narrowed_commands(&ws.affected(&["src/Core/A.cs"])),
            [
                "dotnet test src/Core/Core.csproj",
                "dotnet test src/Web/Web.csproj"
            ]
        );
    }

    #[test]
    fn bazel_and_buck_packages() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(r, "MODULE.bazel", "");
        put(r, "lib/BUILD.bazel", "cc_library(name = \"lib\")\n");
        put(
            r,
            "app/BUILD",
            "cc_binary(name = \"app\", deps = [\"//lib:lib\", \":local\", \"@ext//x\"])\n",
        );
        put(r, "bazel-out/BUILD", "");
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Bazel]);
        assert_eq!(
            summary(&ws),
            [
                ("//app".into(), Tool::Bazel, "app".into(), s(&["//lib"])),
                ("//lib".into(), Tool::Bazel, "lib".into(), vec![]),
            ]
        );
        assert_eq!(
            ws.narrowed_commands(&ws.affected(&["lib/a.cc"])),
            ["bazel test //app/... //lib/..."]
        );

        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(r, ".buckconfig", "");
        put(r, "m/BUCK", "");
        let ws = detect(r);
        assert_eq!(ws.kinds, [Tool::Buck]);
        assert_eq!(
            ws.narrowed_commands(&ws.packages.iter().collect::<Vec<_>>()),
            ["buck2 test //m/..."]
        );
    }

    #[test]
    fn affected_is_transitive_and_global_files_hit_everything() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(r, "Cargo.toml", "[workspace]\nmembers = [\"c/*\"]\n");
        put(r, "c/a/Cargo.toml", "[package]\nname = \"a\"\n");
        put(
            r,
            "c/b/Cargo.toml",
            "[package]\nname = \"b\"\n[dependencies]\na = { path = \"../a\" }\n",
        );
        put(
            r,
            "c/c/Cargo.toml",
            "[package]\nname = \"c\"\n[dependencies]\nb = \"*\"\n",
        );
        put(r, "c/d/Cargo.toml", "[package]\nname = \"d\"\n");
        let ws = detect(r);
        assert_eq!(names(&ws.affected(&["c/a/src/lib.rs"])), ["a", "b", "c"]);
        assert_eq!(names(&ws.affected(&["c/c/x.rs"])), ["c"]);
        assert_eq!(names(&ws.affected(&["README.md"])), Vec::<String>::new());
        assert_eq!(names(&ws.affected(&["Cargo.lock"])), ["a", "b", "c", "d"]);
        assert_eq!(names(&ws.dependents("a")), ["b", "c"]);
        assert_eq!(ws.package_of(Path::new("c/b/src/x.rs")).unwrap().name, "b");
        assert!(ws.package_of(&r.join("elsewhere")).is_none());
        assert_eq!(ws.edges(), [("b", "a"), ("c", "b")]);
    }

    #[test]
    fn plain_directory_is_not_a_monorepo() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "src/main.rs", "");
        let ws = detect(d.path());
        assert!(!ws.is_monorepo() && ws.kinds.is_empty());
    }

    #[test]
    fn changed_files_rejects_option_refs_and_reads_git() {
        let d = tempfile::tempdir().unwrap();
        assert!(changed_files(d.path(), Some("--output=x")).is_err());
        let git_ok = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(d.path())
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git_ok(&["init", "-q"]) {
            return; // no git on this machine
        }
        put(d.path(), "a/x.txt", "1");
        assert!(git_ok(&["add", "."]));
        assert!(git_ok(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "i"
        ]));
        put(d.path(), "a/x.txt", "2");
        put(d.path(), "b/new.txt", "n");
        let got = changed_files(d.path(), None).unwrap();
        assert_eq!(got, [d.path().join("a/x.txt"), d.path().join("b/new.txt")]);
    }
}
