//! Licenses from installed packages' own manifests, for packages whose lockfile has none:
//! `node_modules/**/package.json`, Python `*.dist-info/METADATA`, `vendor/*/composer.json`, the
//! Cargo registry cache and the `NuGet` global packages folder. Nothing found → stays unknown.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Ecosystem, Package, norm};

pub(super) fn resolve(dir: &Path, pkgs: &mut [Package]) {
    let mut py: Option<HashMap<(String, String), PathBuf>> = None;
    for p in pkgs.iter_mut().filter(|p| p.license.is_none()) {
        let found = match p.ecosystem {
            Ecosystem::Npm => npm(dir, p),
            Ecosystem::PyPI => {
                let idx = py.get_or_insert_with(|| site_packages(dir));
                idx.get(&(norm(&p.name), p.version.clone()))
                    .and_then(|m| metadata_license(m).map(|l| (l, m.clone())))
            }
            Ecosystem::Packagist => {
                let m = dir.join("vendor").join(&p.name).join("composer.json");
                json_license(&m).map(|l| (l, m))
            }
            Ecosystem::Cargo => cargo(&p.name, &p.version),
            Ecosystem::NuGet => nuget(&p.name, &p.version),
            Ecosystem::Go | Ecosystem::RubyGems | Ecosystem::Maven => None,
        };
        if let Some((license, from)) = found {
            p.license = Some(license);
            p.license_from = Some(from.to_string_lossy().into_owned());
        }
    }
}

fn json_license(m: &Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(m).ok()?).ok()?;
    super::lock::license_value(&v)
}

/// The installed copy whose `version` matches: hoisted, pnpm store, or the lock key's own path.
fn npm(dir: &Path, p: &Package) -> Option<(String, PathBuf)> {
    let nm = dir.join("node_modules");
    let candidates = [
        nm.join(&p.name).join("package.json"),
        nm.join(".pnpm")
            .join(format!("{}@{}", p.name.replace('/', "+"), p.version))
            .join("node_modules")
            .join(&p.name)
            .join("package.json"),
    ];
    candidates.into_iter().find_map(|m| {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&m).ok()?).ok()?;
        (v.get("version").and_then(serde_json::Value::as_str) == Some(p.version.as_str()))
            .then(|| super::lock::license_value(&v))
            .flatten()
            .map(|l| (l, m))
    })
}

/// `(normalized name, version)` → METADATA of every `*.dist-info` in the project's virtualenvs.
fn site_packages(dir: &Path) -> HashMap<(String, String), PathBuf> {
    let mut idx = HashMap::new();
    for venv in [".venv", "venv", "env"] {
        let base = dir.join(venv);
        if !base.is_dir() {
            continue;
        }
        let mut roots = vec![base.join("Lib").join("site-packages")];
        for lib in std::fs::read_dir(base.join("lib"))
            .into_iter()
            .flatten()
            .flatten()
        {
            roots.push(lib.path().join("site-packages"));
        }
        for sp in roots {
            for e in std::fs::read_dir(&sp).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.strip_suffix(".dist-info")
                    && let Some((n, v)) = stem.rsplit_once('-')
                {
                    idx.insert((norm(n), v.to_owned()), e.path().join("METADATA"));
                }
            }
        }
    }
    idx
}

/// Trove classifiers that name one license unambiguously (an exact table, no inference).
const CLASSIFIERS: &[(&str, &str)] = &[
    ("MIT License", "MIT"),
    ("ISC License (ISCL)", "ISC"),
    ("Mozilla Public License 2.0 (MPL 2.0)", "MPL-2.0"),
    ("GNU General Public License v2 (GPLv2)", "GPL-2.0-only"),
    (
        "GNU General Public License v2 or later (GPLv2+)",
        "GPL-2.0-or-later",
    ),
    ("GNU General Public License v3 (GPLv3)", "GPL-3.0-only"),
    (
        "GNU General Public License v3 or later (GPLv3+)",
        "GPL-3.0-or-later",
    ),
    ("GNU Affero General Public License v3", "AGPL-3.0-only"),
    (
        "GNU Affero General Public License v3 or later (AGPLv3+)",
        "AGPL-3.0-or-later",
    ),
    (
        "GNU Lesser General Public License v3 (LGPLv3)",
        "LGPL-3.0-only",
    ),
    (
        "GNU Lesser General Public License v2 or later (LGPLv2+)",
        "LGPL-2.0-or-later",
    ),
    ("The Unlicense (Unlicense)", "Unlicense"),
];

/// `License-Expression` (PEP 639), else a one-line `License`, else exactly one known classifier.
pub(super) fn metadata_license(m: &Path) -> Option<String> {
    let text = std::fs::read_to_string(m).ok()?;
    let headers = text.split("\n\n").next().unwrap_or(&text);
    let field = |key: &str| {
        headers.lines().find_map(|l| {
            l.strip_prefix(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty() && v != "UNKNOWN")
        })
    };
    if let Some(e) = field("License-Expression:") {
        return Some(e);
    }
    // A long `License:` is the license text itself, not an identifier.
    if let Some(l) = field("License:").filter(|l| l.len() <= 64) {
        return Some(l);
    }
    let found: Vec<&str> = headers
        .lines()
        .filter_map(|l| l.strip_prefix("Classifier: License :: OSI Approved :: "))
        .filter_map(|c| {
            CLASSIFIERS
                .iter()
                .find(|(k, _)| *k == c.trim())
                .map(|(_, v)| *v)
        })
        .collect();
    (found.len() == 1).then(|| found[0].to_owned())
}

/// `$CARGO_HOME/registry/src/*/<name>-<version>/Cargo.toml` `package.license`.
fn cargo(name: &str, version: &str) -> Option<(String, PathBuf)> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))?;
    for reg in std::fs::read_dir(home.join("registry").join("src"))
        .ok()?
        .flatten()
    {
        let m = reg
            .path()
            .join(format!("{name}-{version}"))
            .join("Cargo.toml");
        if let Ok(t) = std::fs::read_to_string(&m) {
            let v: toml::Table = toml::from_str(&t).ok()?;
            return v
                .get("package")?
                .get("license")?
                .as_str()
                .map(|l| (l.to_owned(), m));
        }
    }
    None
}

/// `~/.nuget/packages/<id>/<version>/<id>.nuspec` `<license type="expression">`.
fn nuget(id: &str, version: &str) -> Option<(String, PathBuf)> {
    let base = std::env::var_os("NUGET_PACKAGES")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".nuget").join("packages")))?;
    let lower = id.to_ascii_lowercase();
    let m = base
        .join(&lower)
        .join(version.to_ascii_lowercase())
        .join(format!("{lower}.nuspec"));
    let t = std::fs::read_to_string(&m).ok()?;
    let start = t.find("<license type=\"expression\">")? + "<license type=\"expression\">".len();
    let end = t[start..].find("</license>")? + start;
    Some((t[start..end].trim().to_owned(), m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_prefers_expression_then_field_then_one_classifier() {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("METADATA");
        let write = |s: &str| std::fs::write(&m, s).unwrap();
        write("Name: a\nLicense-Expression: MIT OR Apache-2.0\nLicense: BSD\n\nbody");
        assert_eq!(metadata_license(&m).as_deref(), Some("MIT OR Apache-2.0"));
        write("Name: a\nLicense: BSD-3-Clause\n");
        assert_eq!(metadata_license(&m).as_deref(), Some("BSD-3-Clause"));
        write(
            "Name: a\nClassifier: License :: OSI Approved :: GNU Affero General Public License v3\n",
        );
        assert_eq!(metadata_license(&m).as_deref(), Some("AGPL-3.0-only"));
        write("Name: a\nClassifier: License :: OSI Approved :: BSD License\n");
        assert_eq!(metadata_license(&m), None);
    }
}
