//! One parser per lockfile / manifest format. Each returns packages with `file` set to the
//! absolute path that declares them (made project-relative by `scan`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Ecosystem, Package, decl_line, lead_token, line_of, norm};

type Res = Result<Vec<Package>, String>;

pub(super) fn parse(path: &Path, dir: &Path, text: &str, cargo_tomls: &[PathBuf]) -> Res {
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.as_str() {
        "Cargo.lock" => cargo(path, dir, text, cargo_tomls),
        "package-lock.json" | "npm-shrinkwrap.json" => npm(path, dir, text),
        "pnpm-lock.yaml" => pnpm(path, dir, text),
        "yarn.lock" => Ok(yarn(path, dir, text)),
        "poetry.lock" => poetry(path, dir, text),
        "uv.lock" => uv(path, dir, text),
        "Pipfile.lock" => pipfile(path, dir, text),
        "go.mod" => Ok(gomod(path, text)),
        "composer.lock" => composer(path, dir, text),
        "packages.lock.json" => nuget(path, dir, text),
        "Gemfile.lock" => Ok(gemfile(path, dir, text)),
        "pom.xml" => Ok(pom(path, text)),
        "gradle.lockfile" => Ok(gradle(path, dir, text)),
        _ => Ok(requirements(path, text)),
    }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// Point a direct package at its manifest line when the manifest declares it.
fn declare(p: &mut Package, manifest: &Path, text: &str) -> bool {
    if let Some((line, dev)) = decl_line(manifest, text, &p.name) {
        p.file = manifest.to_path_buf();
        p.line = line;
        p.dev |= dev;
        true
    } else {
        false
    }
}

fn str_keys(v: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_object))
        .flat_map(|m| m.keys().cloned())
        .collect()
}

fn toml_err(e: &toml::de::Error) -> String {
    e.message().to_owned()
}

// ---------------------------------------------------------------- Rust

#[derive(serde::Deserialize)]
struct CargoLock {
    #[serde(default)]
    package: Vec<CargoPkg>,
}

#[derive(serde::Deserialize)]
struct CargoPkg {
    name: String,
    version: String,
    source: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

fn cargo(path: &Path, dir: &Path, text: &str, cargo_tomls: &[PathBuf]) -> Res {
    let lock: CargoLock = toml::from_str(text).map_err(|e| toml_err(&e))?;
    // Workspace members are the path packages (no `source`); their manifests declare the direct deps.
    let members: HashSet<&str> = lock
        .package
        .iter()
        .filter(|p| p.source.is_none())
        .map(|p| p.name.as_str())
        .collect();
    let mut manifests: Vec<(PathBuf, String)> = Vec::new();
    for m in cargo_tomls.iter().filter(|m| m.starts_with(dir)) {
        let Some(t) = read(m) else { continue };
        let name = toml::from_str::<toml::Table>(&t)
            .ok()
            .and_then(|v| v.get("package")?.get("name")?.as_str().map(str::to_owned));
        if name.is_some_and(|n| members.contains(n.as_str())) {
            manifests.push((m.clone(), t));
        }
    }
    let direct: HashSet<&str> = lock
        .package
        .iter()
        .filter(|p| p.source.is_none())
        .flat_map(|p| p.dependencies.iter())
        .filter_map(|d| d.split_whitespace().next())
        .filter(|d| !members.contains(d))
        .collect();
    let mut out = Vec::new();
    for p in lock.package.iter().filter(|p| p.source.is_some()) {
        let line = text
            .find(&format!(
                "name = \"{}\"\nversion = \"{}\"",
                p.name, p.version
            ))
            .or_else(|| {
                text.find(&format!(
                    "name = \"{}\"\r\nversion = \"{}\"",
                    p.name, p.version
                ))
            })
            .map_or(1, |at| super::line_at(text, at));
        let mut pkg = Package::new(Ecosystem::Cargo, &p.name, &p.version, path, line);
        pkg.requires = p
            .dependencies
            .iter()
            .filter_map(|d| d.split_whitespace().next().map(str::to_owned))
            .collect();
        if direct.contains(p.name.as_str()) {
            pkg.direct = true;
            for (m, t) in &manifests {
                if declare(&mut pkg, m, t) {
                    break;
                }
            }
        }
        out.push(pkg);
    }
    Ok(out)
}

// ---------------------------------------------------------------- JavaScript

fn npm(path: &Path, dir: &Path, text: &str) -> Res {
    const DEP_KEYS: [&str; 4] = [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ];
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let Some(packages) = v.get("packages").and_then(Value::as_object) else {
        // lockfileVersion 1: nested `dependencies`, no licenses.
        let direct: HashSet<String> = read(&dir.join("package.json"))
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|m| str_keys(&m, &DEP_KEYS).into_iter().collect())
            .unwrap_or_default();
        npm_v1(path, text, v.get("dependencies"), &direct, true, &mut out);
        return Ok(out);
    };
    // Root ("") and workspace entries (keys without node_modules/) declare the direct deps.
    let mut roots: Vec<(String, HashSet<String>)> = Vec::new();
    for (key, e) in packages {
        if !key.contains("node_modules/") {
            roots.push((key.clone(), str_keys(e, &DEP_KEYS).into_iter().collect()));
        }
    }
    let manifests: Vec<(PathBuf, String)> = roots
        .iter()
        .filter_map(|(k, _)| {
            let m = dir.join(k).join("package.json");
            read(&m).map(|t| (m, t))
        })
        .collect();
    for (key, e) in packages {
        let Some(at) = key.rfind("node_modules/") else {
            continue;
        };
        if e.get("link").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let name = &key[at + "node_modules/".len()..];
        let version = e.get("version").and_then(Value::as_str).unwrap_or("");
        let line = line_of(text, &format!("\"{key}\"")).unwrap_or(1);
        let mut p = Package::new(Ecosystem::Npm, name, version, path, line);
        p.dev = e.get("dev").and_then(Value::as_bool) == Some(true)
            || e.get("devOptional").and_then(Value::as_bool) == Some(true);
        if let Some(l) = license_value(e) {
            p.license = Some(l);
            p.license_from = Some("lockfile".into());
        }
        p.requires = str_keys(
            e,
            &["dependencies", "optionalDependencies", "peerDependencies"],
        );
        // Top-level copy: `node_modules/x` (hoisted, any workspace may declare it) or
        // `<workspace>/node_modules/x` (declared by that workspace).
        let parent = key[..at].trim_end_matches('/');
        p.direct = !parent.contains("node_modules")
            && roots
                .iter()
                .any(|(k, names)| names.contains(name) && (parent.is_empty() || parent == k));
        if p.direct {
            for (m, t) in &manifests {
                if declare(&mut p, m, t) {
                    break;
                }
            }
        }
        out.push(p);
    }
    Ok(out)
}

fn npm_v1(
    path: &Path,
    text: &str,
    deps: Option<&Value>,
    direct: &HashSet<String>,
    top: bool,
    out: &mut Vec<Package>,
) {
    let Some(map) = deps.and_then(Value::as_object) else {
        return;
    };
    for (name, e) in map {
        let version = e.get("version").and_then(Value::as_str).unwrap_or("");
        let line = line_of(text, &format!("\"{name}\": {{")).unwrap_or(1);
        let mut p = Package::new(Ecosystem::Npm, name, version, path, line);
        p.dev = e.get("dev").and_then(Value::as_bool) == Some(true);
        p.direct = top && direct.contains(name);
        p.requires = str_keys(e, &["requires"]);
        out.push(p);
        npm_v1(path, text, e.get("dependencies"), direct, false, out);
    }
}

/// `license` (SPDX string), legacy `{ "type": … }` or `licenses: [{ "type": … }]` (= OR).
pub(super) fn license_value(e: &Value) -> Option<String> {
    let one = |v: &Value| match v {
        Value::String(s) => Some(s.trim().to_owned()),
        Value::Object(o) => o.get("type").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    };
    if let Some(l) = e.get("license").and_then(one).filter(|s| !s.is_empty()) {
        return Some(l);
    }
    let all: Vec<String> = e
        .get("licenses")?
        .as_array()?
        .iter()
        .filter_map(one)
        .collect();
    match all.len() {
        0 => None,
        1 => all.into_iter().next(),
        _ => Some(all.join(" OR ")),
    }
}

/// `name@version` from a pnpm package key (v5 `/name/1.0.0_peer`, v6 `/name@1.0.0(peer)`, v9 `name@1.0.0`).
fn pnpm_key(k: &str) -> Option<(String, String)> {
    let k = k.trim_start_matches('/');
    let k = k.split('(').next()?;
    if let Some(i) = k.get(1..).and_then(|r| r.find('@')).map(|i| i + 1) {
        return Some((k[..i].to_owned(), k[i + 1..].to_owned()));
    }
    let i = k.rfind('/')?;
    Some((k[..i].to_owned(), k[i + 1..].split('_').next()?.to_owned()))
}

fn yaml_keys(v: Option<&serde_yaml::Value>) -> Vec<String> {
    v.and_then(serde_yaml::Value::as_mapping)
        .map(|m| {
            m.keys()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn pnpm(path: &Path, dir: &Path, text: &str) -> Res {
    let v: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    let sections = ["dependencies", "devDependencies", "optionalDependencies"];
    // importer dir → (name → dev)
    let mut importers: Vec<(String, HashMap<String, bool>)> = Vec::new();
    let mut collect = |dir: String, imp: &serde_yaml::Value| {
        let mut names = HashMap::new();
        for s in sections {
            for n in yaml_keys(imp.get(s)) {
                names.entry(n).or_insert(s == "devDependencies");
            }
        }
        importers.push((dir, names));
    };
    if let Some(m) = v.get("importers").and_then(serde_yaml::Value::as_mapping) {
        for (k, imp) in m {
            collect(k.as_str().unwrap_or(".").to_owned(), imp);
        }
    } else {
        collect(".".into(), &v);
    }
    let mut requires: HashMap<(String, String), Vec<String>> = HashMap::new();
    for section in ["snapshots", "packages"] {
        if let Some(m) = v.get(section).and_then(serde_yaml::Value::as_mapping) {
            for (k, e) in m {
                if let Some(id) = k.as_str().and_then(pnpm_key) {
                    let r = requires.entry(id).or_default();
                    r.extend(yaml_keys(e.get("dependencies")));
                    r.extend(yaml_keys(e.get("optionalDependencies")));
                }
            }
        }
    }
    let manifests: Vec<(PathBuf, String)> = importers
        .iter()
        .filter_map(|(d, _)| {
            let m = dir.join(d).join("package.json");
            read(&m).map(|t| (m, t))
        })
        .collect();
    let mut out = Vec::new();
    let Some(pkgs) = v.get("packages").and_then(serde_yaml::Value::as_mapping) else {
        return Ok(out);
    };
    for (k, e) in pkgs {
        let Some(key) = k.as_str() else { continue };
        let Some((name, version)) = pnpm_key(key) else {
            continue;
        };
        let line = line_of(text, key).unwrap_or(1);
        let mut p = Package::new(Ecosystem::Npm, &name, &version, path, line);
        p.dev = e.get("dev").and_then(serde_yaml::Value::as_bool) == Some(true);
        p.requires = requires
            .get(&(name.clone(), version.clone()))
            .cloned()
            .unwrap_or_default();
        if let Some(dev) = importers.iter().find_map(|(_, n)| n.get(&name)) {
            p.direct = true;
            p.dev |= *dev;
            for (m, t) in &manifests {
                if declare(&mut p, m, t) {
                    break;
                }
            }
        }
        out.push(p);
    }
    Ok(out)
}

/// The package name of a yarn spec: `"@babel/core@^7.0.0"` → `@babel/core`, `lodash@npm:^4` → `lodash`.
fn spec_name(spec: &str) -> &str {
    let s = spec.trim().trim_matches('"');
    match s.get(1..).and_then(|r| r.find('@')) {
        Some(i) => &s[..=i],
        None => s,
    }
}

/// yarn.lock v1 and berry share one shape: unindented `specs:` headers, 2-space fields,
/// 4-space dependency names.
fn yarn(path: &Path, dir: &Path, text: &str) -> Vec<Package> {
    struct Entry {
        name: String,
        line: u32,
        version: String,
        deps: Vec<String>,
        workspace: Option<String>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut in_deps = false;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if indent == 0 {
            in_deps = false;
            let header = t.trim_end_matches(':');
            if header.starts_with("__metadata") {
                continue;
            }
            let first = header.split(", ").next().unwrap_or(header);
            let workspace = header
                .split(", ")
                .find_map(|s| s.trim_matches('"').split_once("@workspace:"))
                .map(|(_, d)| d.to_owned());
            entries.push(Entry {
                name: spec_name(first).to_owned(),
                line: u32::try_from(i + 1).unwrap_or(u32::MAX),
                version: String::new(),
                deps: Vec::new(),
                workspace,
            });
            continue;
        }
        let Some(e) = entries.last_mut() else {
            continue;
        };
        if indent == 2 {
            in_deps = t.starts_with("dependencies") || t.starts_with("optionalDependencies");
            if let Some(v) = t.strip_prefix("version") {
                v.trim_start_matches(':')
                    .trim()
                    .trim_matches('"')
                    .clone_into(&mut e.version);
            }
        } else if in_deps && indent >= 4 {
            let n = t.split([' ', ':']).next().unwrap_or("").trim_matches('"');
            if !n.is_empty() {
                e.deps.push(n.to_owned());
            }
        }
    }
    let mut direct: HashMap<String, PathBuf> = HashMap::new();
    let mut manifests: Vec<(PathBuf, String)> = Vec::new();
    for ws in
        std::iter::once(".".to_owned()).chain(entries.iter().filter_map(|e| e.workspace.clone()))
    {
        let m = dir.join(&ws).join("package.json");
        if manifests.iter().any(|(p, _)| *p == m) {
            continue;
        }
        if let Some(t) = read(&m) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                for n in str_keys(
                    &v,
                    &[
                        "dependencies",
                        "devDependencies",
                        "optionalDependencies",
                        "peerDependencies",
                    ],
                ) {
                    direct.entry(n).or_insert_with(|| m.clone());
                }
            }
            manifests.push((m, t));
        }
    }
    let mut out = Vec::new();
    for e in entries.into_iter().filter(|e| e.workspace.is_none()) {
        let mut p = Package::new(Ecosystem::Npm, &e.name, &e.version, path, e.line);
        p.requires = e.deps;
        if let Some(m) = direct.get(&e.name)
            && let Some((_, t)) = manifests.iter().find(|(p, _)| p == m)
        {
            p.direct = true;
            declare(&mut p, m, t);
        }
        out.push(p);
    }
    out
}

// ---------------------------------------------------------------- Python

/// Direct dependency names in pyproject.toml (PEP 621, Poetry, uv dev groups), normalized.
fn pyproject_direct(dir: &Path) -> Option<(PathBuf, String, HashSet<String>)> {
    let m = dir.join("pyproject.toml");
    let t = read(&m)?;
    let v: toml::Table = toml::from_str(&t).ok()?;
    let mut names = HashSet::new();
    let pep508 = |arr: Option<&toml::Value>, names: &mut HashSet<String>| {
        for s in arr.and_then(toml::Value::as_array).into_iter().flatten() {
            if let Some(s) = s.as_str() {
                names.insert(norm(lead_token(s)));
            }
        }
    };
    if let Some(p) = v.get("project") {
        pep508(p.get("dependencies"), &mut names);
        for g in p
            .get("optional-dependencies")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|t| t.values())
        {
            pep508(Some(g), &mut names);
        }
    }
    for g in v
        .get("dependency-groups")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|t| t.values())
    {
        pep508(Some(g), &mut names);
    }
    if let Some(poetry) = v.get("tool").and_then(|t| t.get("poetry")) {
        let mut tables: Vec<&toml::Value> = Vec::new();
        tables.extend(poetry.get("dependencies"));
        tables.extend(poetry.get("dev-dependencies"));
        for g in poetry
            .get("group")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|t| t.values())
        {
            tables.extend(g.get("dependencies"));
        }
        for t in tables.into_iter().filter_map(toml::Value::as_table) {
            names.extend(t.keys().filter(|k| *k != "python").map(|k| norm(k)));
        }
    }
    Some((m, t, names))
}

fn mark_direct(out: &mut [Package], manifest: Option<&(PathBuf, String, HashSet<String>)>) {
    for p in out {
        match manifest {
            Some((m, t, names)) => {
                if names.contains(&norm(&p.name)) {
                    p.direct = true;
                    declare(p, m, t);
                }
            }
            // No manifest: nothing tells direct from transitive, so all count as declared here.
            None => p.direct = true,
        }
    }
}

#[derive(serde::Deserialize)]
struct PyLock {
    #[serde(default)]
    package: Vec<toml::Table>,
}

fn poetry(path: &Path, dir: &Path, text: &str) -> Res {
    let lock: PyLock = toml::from_str(text).map_err(|e| toml_err(&e))?;
    let mut out = Vec::new();
    for t in &lock.package {
        let (Some(name), Some(version)) = (
            t.get("name").and_then(toml::Value::as_str),
            t.get("version").and_then(toml::Value::as_str),
        ) else {
            continue;
        };
        let line = line_of(text, &format!("name = \"{name}\"")).unwrap_or(1);
        let mut p = Package::new(Ecosystem::PyPI, name, version, path, line);
        p.dev = t.get("category").and_then(toml::Value::as_str) == Some("dev");
        p.requires = t
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .map(|d| d.keys().cloned().collect())
            .unwrap_or_default();
        out.push(p);
    }
    mark_direct(&mut out, pyproject_direct(dir).as_ref());
    Ok(out)
}

fn uv(path: &Path, dir: &Path, text: &str) -> Res {
    let lock: PyLock = toml::from_str(text).map_err(|e| toml_err(&e))?;
    let names_of = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| {
                d.get("name")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            })
            .collect()
    };
    let groups = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|t| t.values())
            .flat_map(|g| names_of(Some(g)))
            .collect()
    };
    let mut direct: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for t in &lock.package {
        let (Some(name), version) = (
            t.get("name").and_then(toml::Value::as_str),
            t.get("version").and_then(toml::Value::as_str).unwrap_or(""),
        ) else {
            continue;
        };
        let mut requires = names_of(t.get("dependencies"));
        let src = t.get("source");
        if src.is_some_and(|s| s.get("editable").is_some() || s.get("virtual").is_some()) {
            // The project itself (and workspace members): its requirements are the direct deps.
            direct.extend(requires.iter().map(|n| norm(n)));
            direct.extend(groups(t.get("dev-dependencies")).iter().map(|n| norm(n)));
            direct.extend(
                groups(t.get("optional-dependencies"))
                    .iter()
                    .map(|n| norm(n)),
            );
            continue;
        }
        requires.extend(groups(t.get("optional-dependencies")));
        let line = line_of(text, &format!("name = \"{name}\"")).unwrap_or(1);
        let mut p = Package::new(Ecosystem::PyPI, name, version, path, line);
        p.requires = requires;
        out.push(p);
    }
    let manifest = pyproject_direct(dir).map(|(m, t, mut names)| {
        names.extend(direct);
        (m, t, names)
    });
    mark_direct(&mut out, manifest.as_ref());
    Ok(out)
}

fn pipfile(path: &Path, dir: &Path, text: &str) -> Res {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (section, dev) in [("default", false), ("develop", true)] {
        for (name, e) in v
            .get(section)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let version = e
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_start_matches("==");
            let line = line_of(text, &format!("\"{name}\": {{")).unwrap_or(1);
            let mut p = Package::new(Ecosystem::PyPI, name, version, path, line);
            p.dev = dev;
            out.push(p);
        }
    }
    let manifest = read(&dir.join("Pipfile")).and_then(|t| {
        let v: toml::Table = toml::from_str(&t).ok()?;
        let names = ["packages", "dev-packages"]
            .iter()
            .filter_map(|s| v.get(*s).and_then(toml::Value::as_table))
            .flat_map(|t| t.keys().map(|k| norm(k)))
            .collect();
        Some((dir.join("Pipfile"), t, names))
    });
    mark_direct(&mut out, manifest.as_ref());
    Ok(out)
}

/// requirements*.txt; pip-compile `# via` annotations tell transitive packages apart.
fn requirements(path: &Path, text: &str) -> Vec<Package> {
    let mut out: Vec<Package> = Vec::new();
    // child index → parents named in `# via`
    let mut via: Vec<(usize, Vec<String>, bool)> = Vec::new();
    let mut in_via = false;
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if let Some(c) = t.strip_prefix('#') {
            let c = c.trim();
            let names = if let Some(rest) = c.strip_prefix("via") {
                in_via = true;
                rest.trim()
            } else if in_via {
                c
            } else {
                continue;
            };
            if let (Some(last), false) = (via.last_mut(), names.is_empty()) {
                if names.starts_with("-r") || names.starts_with("-c") {
                    last.2 = true;
                } else {
                    last.1
                        .push(names.split_whitespace().next().unwrap_or(names).to_owned());
                }
            }
            continue;
        }
        in_via = false;
        if t.is_empty() || t.starts_with('-') || raw.starts_with([' ', '\t']) || t.contains("://") {
            continue;
        }
        let spec = t
            .split([';', '#'])
            .next()
            .unwrap_or(t)
            .trim_end_matches('\\')
            .trim();
        let name = lead_token(spec);
        if name.is_empty() {
            continue;
        }
        let version = spec
            .split_once("==")
            .map(|(_, v)| v.split([',', ' ']).next().unwrap_or("").trim().to_owned())
            .unwrap_or_default();
        let mut p = Package::new(
            Ecosystem::PyPI,
            name,
            &version,
            path,
            u32::try_from(i + 1).unwrap_or(u32::MAX),
        );
        p.direct = true;
        out.push(p);
        via.push((out.len() - 1, Vec::new(), false));
    }
    let annotated = via.iter().any(|(_, parents, r)| *r || !parents.is_empty());
    if annotated {
        for (idx, parents, from_file) in via {
            out[idx].direct = from_file;
            let child = out[idx].name.clone();
            for parent in parents {
                if let Some(pp) = out.iter_mut().find(|p| norm(&p.name) == norm(&parent)) {
                    pp.requires.push(child.clone());
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- Go, PHP, .NET, Ruby, JVM

/// go.mod lists every required module since Go 1.17; `// indirect` marks transitive ones.
/// (go.sum holds hashes of every version ever considered, so it is not a version source.)
fn gomod(path: &Path, text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim();
        let spec = if in_block {
            if t.starts_with(')') {
                in_block = false;
                continue;
            }
            t
        } else if let Some(rest) = t.strip_prefix("require") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                in_block = true;
                continue;
            }
            rest
        } else {
            continue;
        };
        let (code, comment) = spec.split_once("//").unwrap_or((spec, ""));
        let mut parts = code.split_whitespace();
        let (Some(module), Some(version)) = (parts.next(), parts.next()) else {
            continue;
        };
        let mut p = Package::new(
            Ecosystem::Go,
            module,
            version.trim_start_matches('v'),
            path,
            u32::try_from(i + 1).unwrap_or(u32::MAX),
        );
        p.direct = !comment.contains("indirect");
        out.push(p);
    }
    out
}

fn composer(path: &Path, dir: &Path, text: &str) -> Res {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let platform = |n: &str| {
        n == "php" || n.starts_with("ext-") || n.starts_with("lib-") || n.starts_with("composer-")
    };
    let manifest = dir.join("composer.json");
    let mtext = read(&manifest);
    let direct: HashSet<String> = mtext
        .as_deref()
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .map(|m| {
            str_keys(&m, &["require", "require-dev"])
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for (section, dev) in [("packages", false), ("packages-dev", true)] {
        for e in v
            .get(section)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = e.get("name").and_then(Value::as_str) else {
                continue;
            };
            let version = e.get("version").and_then(Value::as_str).unwrap_or("");
            let line = line_of(text, &format!("\"name\": \"{name}\"")).unwrap_or(1);
            let mut p = Package::new(Ecosystem::Packagist, name, version, path, line);
            p.dev = dev;
            // Several composer licenses mean the package is available under any of them.
            let lic: Vec<&str> = e
                .get("license")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if !lic.is_empty() {
                p.license = Some(lic.join(" OR "));
                p.license_from = Some("lockfile".into());
            }
            p.requires = str_keys(e, &["require"])
                .into_iter()
                .filter(|n| !platform(n))
                .collect();
            if direct.contains(name) {
                p.direct = true;
                if let Some(t) = &mtext {
                    declare(&mut p, &manifest, t);
                }
            }
            out.push(p);
        }
    }
    Ok(out)
}

fn nuget(path: &Path, dir: &Path, text: &str) -> Res {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let projects: Vec<(PathBuf, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.to_string_lossy().ends_with("proj"))
        })
        .filter_map(|p| read(&p).map(|t| (p, t)))
        .collect();
    let mut seen: BTreeMap<(String, String), Package> = BTreeMap::new();
    for fw in v
        .get("dependencies")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.values())
    {
        for (id, e) in fw.as_object().into_iter().flatten() {
            let kind = e.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "Project" {
                continue;
            }
            let version = e.get("resolved").and_then(Value::as_str).unwrap_or("");
            let key = (id.to_ascii_lowercase(), version.to_owned());
            if seen.contains_key(&key) {
                continue;
            }
            let line = line_of(text, &format!("\"{id}\": {{")).unwrap_or(1);
            let mut p = Package::new(Ecosystem::NuGet, id, version, path, line);
            p.direct = kind == "Direct";
            p.requires = str_keys(e, &["dependencies"]);
            if p.direct {
                for (m, t) in &projects {
                    if declare(&mut p, m, t) {
                        break;
                    }
                }
            }
            seen.insert(key, p);
        }
    }
    Ok(seen.into_values().collect())
}

fn gemfile(path: &Path, dir: &Path, text: &str) -> Vec<Package> {
    let mut out: Vec<Package> = Vec::new();
    let mut section = "";
    let mut direct: HashSet<String> = HashSet::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if indent == 0 && !t.is_empty() {
            section = if matches!(t, "GEM" | "GIT") {
                "gem"
            } else if t == "DEPENDENCIES" {
                "deps"
            } else {
                ""
            };
            continue;
        }
        match (section, indent) {
            ("gem", 4) => {
                let (name, rest) = t.split_once(' ').unwrap_or((t, ""));
                let version = rest.trim_matches(['(', ')']);
                // `nokogiri (1.15.4-x86_64-linux)`: platform suffix is not part of the version.
                let version = version.split_once('-').map_or(version, |(v, _)| v);
                out.push(Package::new(
                    Ecosystem::RubyGems,
                    name,
                    version,
                    path,
                    u32::try_from(i + 1).unwrap_or(u32::MAX),
                ));
            }
            ("gem", 6) => {
                if let Some(p) = out.last_mut() {
                    p.requires.push(t.split(' ').next().unwrap_or(t).to_owned());
                }
            }
            ("deps", 2) => {
                direct.insert(t.split([' ', '!']).next().unwrap_or(t).to_owned());
            }
            _ => {}
        }
    }
    let manifest = dir.join("Gemfile");
    let mtext = read(&manifest);
    for p in &mut out {
        if direct.contains(&p.name) {
            p.direct = true;
            if let Some(t) = &mtext {
                declare(p, &manifest, t);
            }
        }
    }
    out
}

fn xml_text<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = line.find(&open)? + open.len();
    let end = line[start..].find("</")? + start;
    Some(line[start..end].trim())
}

/// pom.xml `<dependencies>` (best effort): direct only, `${property}` versions from `<properties>`.
fn pom(path: &Path, text: &str) -> Vec<Package> {
    let mut props: HashMap<String, String> = HashMap::new();
    let mut in_props = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("<properties>") {
            in_props = true;
        } else if t.starts_with("</properties>") {
            in_props = false;
        } else if in_props
            && let Some(tag) = t.strip_prefix('<').and_then(|r| r.split('>').next())
            && let Some(v) = xml_text(t, tag)
        {
            props.insert(tag.to_owned(), v.to_owned());
        }
    }
    let mut out = Vec::new();
    let (mut group, mut artifact, mut version, mut scope, mut line_no) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        0u32,
    );
    let (mut in_dep, mut skip_depth) = (false, 0u32);
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("<dependencyManagement>")
            || t.starts_with("<plugins>")
            || t.starts_with("<pluginManagement>")
        {
            skip_depth += 1;
        } else if t.starts_with("</dependencyManagement>")
            || t.starts_with("</plugins>")
            || t.starts_with("</pluginManagement>")
        {
            skip_depth = skip_depth.saturating_sub(1);
        }
        if skip_depth > 0 {
            continue;
        }
        if t.starts_with("<dependency>") {
            in_dep = true;
            (group, artifact, version, scope) = Default::default();
        } else if in_dep {
            if let Some(v) = xml_text(t, "groupId") {
                v.clone_into(&mut group);
            } else if let Some(v) = xml_text(t, "artifactId") {
                v.clone_into(&mut artifact);
                line_no = u32::try_from(i + 1).unwrap_or(u32::MAX);
            } else if let Some(v) = xml_text(t, "version") {
                version = v
                    .strip_prefix("${")
                    .and_then(|r| r.strip_suffix('}'))
                    .map_or_else(
                        || v.to_owned(),
                        |k| props.get(k).cloned().unwrap_or_default(),
                    );
            } else if let Some(v) = xml_text(t, "scope") {
                v.clone_into(&mut scope);
            } else if t.starts_with("</dependency>") {
                in_dep = false;
                if !group.is_empty() && !artifact.is_empty() {
                    let mut p = Package::new(
                        Ecosystem::Maven,
                        &format!("{group}:{artifact}"),
                        &version,
                        path,
                        line_no,
                    );
                    p.direct = true;
                    p.dev = scope == "test";
                    out.push(p);
                }
            }
        }
    }
    out
}

/// gradle.lockfile `group:artifact:version=configurations`.
fn gradle(path: &Path, dir: &Path, text: &str) -> Vec<Package> {
    let build: Vec<(PathBuf, String)> = ["build.gradle", "build.gradle.kts"]
        .iter()
        .filter_map(|f| read(&dir.join(f)).map(|t| (dir.join(f), t)))
        .collect();
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.starts_with('#') || t.starts_with("empty=") {
            continue;
        }
        let Some((coord, configs)) = t.split_once('=') else {
            continue;
        };
        let mut parts = coord.split(':');
        let (Some(group), Some(artifact), Some(version)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let name = format!("{group}:{artifact}");
        let mut p = Package::new(
            Ecosystem::Maven,
            &name,
            version,
            path,
            u32::try_from(i + 1).unwrap_or(u32::MAX),
        );
        p.dev = configs
            .split(',')
            .all(|c| c.to_ascii_lowercase().contains("test"));
        for (m, bt) in &build {
            if bt.contains(&name) {
                p.direct = true;
                declare(&mut p, m, bt);
                break;
            }
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pnpm_keys_of_all_versions() {
        assert_eq!(
            pnpm_key("/lodash/4.17.21"),
            Some(("lodash".into(), "4.17.21".into()))
        );
        assert_eq!(
            pnpm_key("/@babel/core/7.0.0_x"),
            Some(("@babel/core".into(), "7.0.0".into()))
        );
        assert_eq!(
            pnpm_key("/@babel/core@7.0.0(react@18)"),
            Some(("@babel/core".into(), "7.0.0".into()))
        );
        assert_eq!(
            pnpm_key("react-dom@18.2.0(react@18.2.0)"),
            Some(("react-dom".into(), "18.2.0".into()))
        );
    }

    #[test]
    fn yarn_spec_names() {
        assert_eq!(spec_name("\"@babel/core@^7.0.0\""), "@babel/core");
        assert_eq!(spec_name("lodash@npm:^4.17.21"), "lodash");
    }

    #[test]
    fn requirements_with_and_without_pip_compile_annotations() {
        let plain = requirements(
            Path::new("r.txt"),
            "requests==2.31.0\nflask>=2\n-r other.txt\n",
        );
        assert_eq!(plain.len(), 2);
        assert!(plain.iter().all(|p| p.direct));
        assert_eq!(plain[0].version, "2.31.0");
        assert_eq!(plain[1].version, "");
        let compiled = "certifi==2023.7.22\n    # via requests\nrequests==2.31.0\n    # via -r requirements.in\n";
        let c = requirements(Path::new("r.txt"), compiled);
        assert!(!c[0].direct);
        assert!(c[1].direct);
        assert_eq!(c[1].requires, ["certifi"]);
    }
}
