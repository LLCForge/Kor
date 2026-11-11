//! Dependency inventory for `kor audit --licenses` and `kor deps check|list` (spec §9, ADR 0046).
//!
//! Lockfiles and manifests are read as text (no package manager runs, as in ADR 0020). Every
//! package keeps the `file:line` that declares it: the manifest line for a direct dependency, the
//! lockfile entry for a transitive one. Licenses come from the lockfile, the installed package's
//! own manifest, or stay unknown; they are never guessed.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod check;
mod installed;
pub mod license;
mod lock;

/// Package registry family; the names are OSV's ecosystem ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Ecosystem {
    #[serde(rename = "crates.io")]
    Cargo,
    #[serde(rename = "npm")]
    Npm,
    #[serde(rename = "PyPI")]
    PyPI,
    #[serde(rename = "Go")]
    Go,
    #[serde(rename = "Packagist")]
    Packagist,
    #[serde(rename = "NuGet")]
    NuGet,
    #[serde(rename = "RubyGems")]
    RubyGems,
    #[serde(rename = "Maven")]
    Maven,
}

impl Ecosystem {
    /// OSV ecosystem id (also the display name).
    pub fn osv(self) -> &'static str {
        match self {
            Self::Cargo => "crates.io",
            Self::Npm => "npm",
            Self::PyPI => "PyPI",
            Self::Go => "Go",
            Self::Packagist => "Packagist",
            Self::NuGet => "NuGet",
            Self::RubyGems => "RubyGems",
            Self::Maven => "Maven",
        }
    }
}

/// One resolved (or, without a lockfile pin, declared) package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub ecosystem: Ecosystem,
    pub name: String,
    /// Exact version; empty when the source pins none (e.g. `requests>=2` in requirements.txt).
    pub version: String,
    pub direct: bool,
    /// Declared only for development/tests.
    pub dev: bool,
    /// License as its source declares it (an SPDX expression when the source uses SPDX).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Where the license came from: `lockfile`, or the installed manifest's project-relative path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_from: Option<String>,
    /// Project-relative file that declares it: the manifest for direct, the lockfile otherwise.
    pub file: PathBuf,
    /// 1-based line in `file`.
    pub line: u32,
    /// Direct dependencies that pull this one in (transitive packages only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<String>,
    /// Names this package depends on (from the lockfile), used for `via`.
    #[serde(skip)]
    pub requires: Vec<String>,
}

impl Package {
    pub(crate) fn new(eco: Ecosystem, name: &str, version: &str, file: &Path, line: u32) -> Self {
        Self {
            ecosystem: eco,
            name: name.to_owned(),
            version: version.to_owned(),
            direct: false,
            dev: false,
            license: None,
            license_from: None,
            file: file.to_path_buf(),
            line,
            via: Vec::new(),
            requires: Vec::new(),
        }
    }
}

/// Everything `scan` found.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub packages: Vec<Package>,
    /// Lockfiles / manifests read, project-relative.
    pub sources: Vec<PathBuf>,
    /// Files that could not be parsed: (project-relative path, error).
    pub errors: Vec<(PathBuf, String)>,
}

impl Inventory {
    pub fn direct(&self) -> impl Iterator<Item = &Package> {
        self.packages.iter().filter(|p| p.direct)
    }
}

/// Name comparison key: lowercase, `_`/`.` as `-` (PEP 503; harmless for the other ecosystems,
/// whose registries are case-insensitive or never mix these).
pub(crate) fn norm(name: &str) -> String {
    name.to_ascii_lowercase().replace(['_', '.'], "-")
}

/// 1-based line of byte offset `at`.
pub(crate) fn line_at(text: &str, at: usize) -> u32 {
    u32::try_from(text[..at.min(text.len())].matches('\n').count() + 1).unwrap_or(u32::MAX)
}

/// 1-based line of the first occurrence of `needle`.
pub(crate) fn line_of(text: &str, needle: &str) -> Option<u32> {
    text.find(needle).map(|at| line_at(text, at))
}

/// Line and dev flag of the manifest line declaring dependency `name`. The dev flag is the
/// closest preceding section marker (`[dev-dependencies]`, `"devDependencies"`, `"require-dev"`,
/// `<scope>test</scope>`-style files are handled by their parsers).
pub(crate) fn decl_line(manifest: &Path, text: &str, name: &str) -> Option<(u32, bool)> {
    let file = manifest
        .file_name()
        .map(|f| f.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let want = norm(name);
    let mut dev = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('#') || t.starts_with("//") {
            continue;
        }
        let low = t.to_ascii_lowercase();
        if t.starts_with('[')
            || [
                "\"dependencies\"",
                "\"devdependencies\"",
                "\"peerdependencies\"",
                "\"optionaldependencies\"",
                "\"require\"",
                "\"require-dev\"",
            ]
            .iter()
            .any(|m| low.starts_with(m))
        {
            dev = low.contains("dev");
        }
        let hit = if file.ends_with("proj") {
            low.contains(&format!("include=\"{}\"", name.to_ascii_lowercase()))
        } else if file == "gemfile" {
            low.starts_with("gem ")
                && (low.contains(&format!("'{}'", name.to_ascii_lowercase()))
                    || low.contains(&format!("\"{}\"", name.to_ascii_lowercase())))
        } else if file.starts_with("build.gradle") {
            t.contains(name)
        } else if let Some(inner) = t.strip_prefix('[') {
            // `[dependencies.serde]`, `[target.'cfg(x)'.dependencies.serde]`
            inner
                .trim_end_matches(']')
                .rsplit('.')
                .next()
                .is_some_and(|last| norm(last.trim_matches(['"', '\''])) == want)
                && inner.contains("dependencies")
        } else {
            let tok = lead_token(t);
            norm(tok) == want || tok.split_once('.').is_some_and(|(a, _)| norm(a) == want)
        };
        if hit {
            return Some((u32::try_from(i + 1).unwrap_or(u32::MAX), dev));
        }
    }
    None
}

/// The leading package-name-like token of a line: `serde = "1"` → `serde`, `"react": …` → `react`,
/// `requests[socks]>=2` → `requests`.
pub(crate) fn lead_token(line: &str) -> &str {
    let t = line.trim_start().trim_start_matches(['"', '\'']);
    let end = t
        .find(|c: char| !(c.is_ascii_alphanumeric() || "._-@/".contains(c)))
        .unwrap_or(t.len());
    &t[..end]
}

/// Fill `via` for transitive packages of one lockfile: breadth-first from every direct package.
pub(crate) fn link_via(pkgs: &mut [Package]) {
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in pkgs.iter().enumerate() {
        by_name.entry(norm(&p.name)).or_default().push(i);
    }
    let mut via: Vec<BTreeSet<String>> = vec![BTreeSet::new(); pkgs.len()];
    for (d, root) in pkgs.iter().enumerate().filter(|(_, p)| p.direct) {
        let mut seen = vec![false; pkgs.len()];
        seen[d] = true;
        let mut queue = VecDeque::from([d]);
        while let Some(i) = queue.pop_front() {
            for r in &pkgs[i].requires {
                for &j in by_name.get(&norm(r)).into_iter().flatten() {
                    if !seen[j] {
                        seen[j] = true;
                        if !pkgs[j].direct {
                            via[j].insert(root.name.clone());
                        }
                        queue.push_back(j);
                    }
                }
            }
        }
    }
    for (p, v) in pkgs.iter_mut().zip(via) {
        if !p.direct {
            p.via = v.into_iter().collect();
        }
    }
}

/// Lock/manifest file kinds, in the order a directory's sources are preferred.
const KINDS: &[&str] = &[
    "Cargo.lock",
    "npm-shrinkwrap.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "go.mod",
    "composer.lock",
    "packages.lock.json",
    "Gemfile.lock",
    "pom.xml",
    "gradle.lockfile",
];

/// Directories never walked into (installed packages, build output).
pub(crate) const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    ".git",
    ".venv",
    "venv",
    "dist",
    "build",
    "bower_components",
];

fn is_requirements(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("requirements") && Path::new(&n).extension().is_some_and(|e| e == "txt")
}

/// Lockfiles and manifests under `root`, grouped by directory (gitignore-aware, depth 6).
fn discover(root: &Path) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    let mut by_dir: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let walker = ignore::WalkBuilder::new(root)
        .max_depth(Some(6))
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build();
    // shortcut: at most 200k entries, like the workspace detector (ADR 0020).
    for e in walker.flatten().take(200_000) {
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let name = e.file_name().to_string_lossy();
        if KINDS.contains(&name.as_ref()) || is_requirements(&name) || name == "Cargo.toml" {
            let p = e.into_path();
            if let Some(dir) = p.parent() {
                by_dir.entry(dir.to_path_buf()).or_default().push(p);
            }
        }
    }
    by_dir
}

/// Where `kor deps check` caches osv.dev / registry results: `<kor home>/cache/deps`.
pub fn cache_dir() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("cache").join("deps"))
}

fn rel(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

/// Read every lockfile under `root` and resolve licenses from installed manifests.
pub fn scan(root: &Path) -> Inventory {
    let by_dir = discover(root);
    let cargo_tomls: Vec<PathBuf> = by_dir
        .values()
        .flatten()
        .filter(|p| p.file_name().is_some_and(|f| f == "Cargo.toml"))
        .cloned()
        .collect();
    let mut inv = Inventory::default();
    for (dir, files) in &by_dir {
        let has = |k: &str| files.iter().find(|p| p.file_name().is_some_and(|f| f == k));
        let mut picked: Vec<&PathBuf> = Vec::new();
        // One source per ecosystem family per directory, the most precise first.
        for group in [
            &["Cargo.lock"][..],
            &[
                "npm-shrinkwrap.json",
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
            ],
            &["poetry.lock", "uv.lock", "Pipfile.lock"],
            &["go.mod"],
            &["composer.lock"],
            &["packages.lock.json"],
            &["Gemfile.lock"],
            &["gradle.lockfile", "pom.xml"],
        ] {
            if let Some(p) = group.iter().find_map(|k| has(k)) {
                picked.push(p);
            }
        }
        let python_lock = ["poetry.lock", "uv.lock", "Pipfile.lock"]
            .iter()
            .any(|k| has(k).is_some());
        if !python_lock {
            let mut reqs: Vec<&PathBuf> = files
                .iter()
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|f| is_requirements(&f.to_string_lossy()))
                })
                .collect();
            reqs.sort();
            picked.extend(reqs);
        }
        for path in picked {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    inv.errors.push((rel(root, path), e.to_string()));
                    continue;
                }
            };
            match lock::parse(path, dir, &text, &cargo_tomls) {
                Ok(mut pkgs) => {
                    link_via(&mut pkgs);
                    installed::resolve(dir, &mut pkgs);
                    for p in &mut pkgs {
                        p.file = rel(root, &p.file);
                        if let Some(from) = &p.license_from
                            && from != "lockfile"
                        {
                            p.license_from = Some(
                                rel(root, Path::new(from))
                                    .to_string_lossy()
                                    .replace('\\', "/"),
                            );
                        }
                    }
                    inv.packages.extend(pkgs);
                    inv.sources.push(rel(root, path));
                }
                Err(e) => inv.errors.push((rel(root, path), e)),
            }
        }
    }
    inv.packages.sort_by(|a, b| {
        (a.ecosystem, norm(&a.name), &a.version, &a.file).cmp(&(
            b.ecosystem,
            norm(&b.name),
            &b.version,
            &b.file,
        ))
    });
    inv.packages.dedup_by(|a, b| {
        a.ecosystem == b.ecosystem && a.name == b.name && a.version == b.version && a.file == b.file
    });
    inv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decl_line_finds_keys_sections_and_dev() {
        let cargo = "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\ntokio.workspace = true\n\n[dev-dependencies]\nwiremock = \"0.6\"\n\n[dependencies.regex]\nversion = \"1\"\n";
        let p = Path::new("Cargo.toml");
        assert_eq!(decl_line(p, cargo, "serde"), Some((5, false)));
        assert_eq!(decl_line(p, cargo, "tokio"), Some((6, false)));
        assert_eq!(decl_line(p, cargo, "wiremock"), Some((9, true)));
        assert_eq!(decl_line(p, cargo, "regex"), Some((11, false)));
        assert_eq!(decl_line(p, cargo, "missing"), None);
        let pkg = "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"@scope/a\": \"^1\"\n  },\n  \"devDependencies\": {\n    \"jest\": \"^29\"\n  }\n}\n";
        let p = Path::new("package.json");
        assert_eq!(decl_line(p, pkg, "@scope/a"), Some((4, false)));
        assert_eq!(decl_line(p, pkg, "jest"), Some((7, true)));
        let req = "# pinned\nRequests[socks]==2.31.0\nzope.interface>=5\n";
        let p = Path::new("requirements.txt");
        assert_eq!(decl_line(p, req, "requests"), Some((2, false)));
        assert_eq!(decl_line(p, req, "zope-interface"), Some((3, false)));
        let cs =
            "<ItemGroup>\n  <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.1\" />\n";
        assert_eq!(
            decl_line(Path::new("App.csproj"), cs, "newtonsoft.json"),
            Some((2, false))
        );
        let gem = "source 'https://rubygems.org'\ngem 'rails', '~> 7.0'\n";
        assert_eq!(
            decl_line(Path::new("Gemfile"), gem, "rails"),
            Some((2, false))
        );
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn via_follows_requires_from_direct_roots() {
        let f = Path::new("x.lock");
        let mut a = Package::new(Ecosystem::Npm, "a", "1", f, 1);
        a.direct = true;
        a.requires = vec!["b".into()];
        let mut b = Package::new(Ecosystem::Npm, "b", "1", f, 2);
        b.requires = vec!["c".into()];
        let c = Package::new(Ecosystem::Npm, "c", "1", f, 3);
        let mut d = Package::new(Ecosystem::Npm, "d", "1", f, 4);
        d.direct = true;
        d.requires = vec!["c".into()];
        let mut v = vec![a, b, c, d];
        link_via(&mut v);
        assert_eq!(v[1].via, ["a"]);
        assert_eq!(v[2].via, ["a", "d"]);
        assert!(v[0].via.is_empty());
    }
}
