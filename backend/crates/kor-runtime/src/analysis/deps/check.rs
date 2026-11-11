//! `kor deps check` (spec §9, ADR 0046): known vulnerabilities (osv.dev), likely unused
//! dependencies, duplicate-purpose packages and the upgrade path. Network parts go through the
//! guard (`kor_core::net`) and are refused in privacy/offline mode; their last results are cached
//! so `--offline` can show them with their age.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use futures::StreamExt as _;
use kor_i18n::{Localizer, args};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::license::{inventory_notes, label, via_suffix};
use super::{Ecosystem, Inventory, Package, SKIP_DIRS, norm};
use crate::analysis::{Finding, Report, Severity};

/// Service base URLs (overridden by tests).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub osv: String,
    pub crates: String,
    pub npm: String,
    pub pypi: String,
    pub packagist: String,
    pub goproxy: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            osv: "https://api.osv.dev".into(),
            crates: "https://crates.io".into(),
            npm: "https://registry.npmjs.org".into(),
            pypi: "https://pypi.org".into(),
            packagist: "https://repo.packagist.org".into(),
            goproxy: "https://proxy.golang.org".into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Use the cached network results instead of asking osv.dev and the registries.
    pub offline: bool,
    pub endpoints: Endpoints,
    /// Where results are cached; `None` = no cache.
    pub cache_dir: Option<PathBuf>,
}

/// One OSV advisory, reduced to what a finding needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vuln {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// OSV `database_specific.severity` (GitHub advisories), when given.
    pub severity: Option<String>,
    pub cwe: Option<String>,
    /// `fixed` versions from the affected ranges for this package.
    #[serde(default)]
    pub fixed: Vec<String>,
}

/// Newest version a registry reports, with its own deprecation/abandonment note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    pub version: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Cache {
    fetched_at: String,
    /// `eco|name|version` → advisories (empty = checked, none known).
    vulns: BTreeMap<String, Vec<Vuln>>,
    /// `eco|name` → latest.
    latest: BTreeMap<String, Latest>,
}

fn vkey(p: &Package) -> String {
    format!("{}|{}|{}", p.ecosystem.osv(), norm(&p.name), p.version)
}

fn lkey(p: &Package) -> String {
    format!("{}|{}", p.ecosystem.osv(), norm(&p.name))
}

fn cache_path(dir: &Path, root: &Path) -> PathBuf {
    use sha2::Digest as _;
    let h = sha2::Sha256::digest(root.to_string_lossy().as_bytes());
    let hex = h.iter().take(8).fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    });
    dir.join(format!("deps-{hex}.json"))
}

/// Run every check and build the report.
pub async fn check_report(root: &Path, inv: &Inventory, opts: &Options, l: &Localizer) -> Report {
    let mut r = Report::new("deps");
    inventory_notes(inv, l, &mut r.notes);
    r.findings.extend(duplicates(inv, l));
    let (unused, unused_notes) = unused(root, inv, l);
    r.findings.extend(unused);
    r.notes.extend(unused_notes);

    let cache_file = opts.cache_dir.as_ref().map(|d| cache_path(d, root));
    let cache = if opts.offline {
        let cached = cache_file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str::<Cache>(&t).ok());
        match &cached {
            Some(c) => r.notes.push(l.t_args(
                "deps-cache-used",
                &args!("age" => age(&c.fetched_at), "when" => c.fetched_at.clone()),
            )),
            None => r.notes.push(l.t("deps-cache-none")),
        }
        cached
    } else {
        let c = fetch(inv, &opts.endpoints, l, &mut r.notes).await;
        if let (Some(f), Some(c)) = (&cache_file, &c) {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
            if let Ok(t) = serde_json::to_string_pretty(c) {
                let _ = std::fs::write(f, t);
            }
        }
        c
    };
    if let Some(c) = &cache {
        r.findings.extend(vuln_findings(inv, c, l));
        r.findings.extend(upgrade_findings(inv, c, l));
    }
    r.normalize();
    r
}

fn age(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts).map_or_else(
        |_| "?".into(),
        |t| {
            let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0);
            match secs {
                s if s < 3600 => format!("{}m", s / 60),
                s if s < 86_400 => format!("{}h", s / 3600),
                s => format!("{}d", s / 86_400),
            }
        },
    )
}

fn client() -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("kor/", env!("CARGO_PKG_VERSION"), " (deps check)"))
        .build()
        .map_err(|e| e.to_string())
}

/// Ask osv.dev and the registries. `None` when the guard refused osv.dev (nothing to cache).
async fn fetch(
    inv: &Inventory,
    ep: &Endpoints,
    l: &Localizer,
    notes: &mut Vec<String>,
) -> Option<Cache> {
    if let Err(b) = kor_core::net::check_url(&ep.osv) {
        notes.push(b.message);
        notes.push(l.t("deps-offline-hint"));
        return None;
    }
    let http = match client() {
        Ok(c) => c,
        Err(e) => {
            notes.push(l.t_args("deps-osv-failed", &args!("error" => e)));
            return None;
        }
    };
    let mut cache = Cache {
        fetched_at: chrono::Utc::now().to_rfc3339(),
        ..Cache::default()
    };
    match osv(&http, &ep.osv, inv).await {
        Ok((vulns, capped)) => {
            let n: usize = vulns.values().map(Vec::len).sum();
            notes.push(l.t_args(
                "deps-osv-queried",
                &args!("count" => vulns.len(), "vulns" => n),
            ));
            if capped > 0 {
                notes.push(l.t_args("deps-osv-capped", &args!("count" => capped)));
            }
            cache.vulns = vulns;
        }
        Err(e) => {
            notes.push(l.t_args("deps-osv-failed", &args!("error" => e)));
            return None;
        }
    }
    let mut seen = HashSet::new();
    let direct: Vec<&Package> = inv
        .direct()
        .filter(|p| !p.version.is_empty() && seen.insert(lkey(p)))
        .take(MAX_REGISTRY)
        .collect();
    let mut reported = HashSet::new();
    let results: Vec<(String, Result<Option<Latest>, String>)> = futures::stream::iter(direct)
        .map(|p| {
            let http = &http;
            async move { (lkey(p), latest(http, ep, p).await) }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut failed: BTreeMap<String, String> = BTreeMap::new();
    for (key, res) in results {
        match res {
            Ok(Some(v)) => {
                cache.latest.insert(key, v);
            }
            Ok(None) => {}
            Err(e) => {
                let eco = key.split('|').next().unwrap_or("").to_owned();
                if reported.insert(eco.clone()) {
                    failed.insert(eco, e);
                }
            }
        }
    }
    for (registry, error) in failed {
        notes.push(l.t_args(
            "deps-registry-failed",
            &args!("registry" => registry, "error" => error),
        ));
    }
    Some(cache)
}

/// Bounds on network work per run.
const MAX_ADVISORIES: usize = 300;
const MAX_REGISTRY: usize = 300;

/// `/v1/querybatch` (ids only), then `/v1/vulns/{id}` for the details. Returns the advisories per
/// package key and how many ids were left out by [`MAX_ADVISORIES`].
async fn osv(
    http: &reqwest::Client,
    base: &str,
    inv: &Inventory,
) -> Result<(BTreeMap<String, Vec<Vuln>>, usize), String> {
    let mut seen = HashSet::new();
    let pkgs: Vec<&Package> = inv
        .packages
        .iter()
        .filter(|p| !p.version.is_empty() && seen.insert(vkey(p)))
        .collect();
    let mut ids_per_pkg: Vec<(&Package, Vec<String>)> = Vec::new();
    for chunk in pkgs.chunks(1000) {
        let queries: Vec<Value> = chunk
            .iter()
            .map(|p| json!({ "package": { "name": p.name, "ecosystem": p.ecosystem.osv() }, "version": p.version }))
            .collect();
        let resp = http
            .post(format!("{base}/v1/querybatch"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json!({ "queries": queries }).to_string())
            .send()
            .await
            .map_err(|e| kor_core::net::describe(&e))?;
        let v = json_body(resp).await?;
        let results = v
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (p, res) in chunk.iter().zip(results) {
            let ids = res
                .get("vulns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| x.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            ids_per_pkg.push((p, ids));
        }
    }
    let mut unique: Vec<String> = ids_per_pkg
        .iter()
        .flat_map(|(_, ids)| ids.clone())
        .collect();
    unique.sort();
    unique.dedup();
    let capped = unique.len().saturating_sub(MAX_ADVISORIES);
    unique.truncate(MAX_ADVISORIES);
    let details: HashMap<String, Value> = futures::stream::iter(unique)
        .map(|id| async move {
            let resp = http
                .get(format!("{base}/v1/vulns/{id}"))
                .send()
                .await
                .ok()?;
            let v = json_body(resp).await.ok()?;
            Some((id, v))
        })
        .buffer_unordered(8)
        .filter_map(|x| async move { x })
        .collect()
        .await;
    let mut out = BTreeMap::new();
    for (p, ids) in ids_per_pkg {
        let vulns = ids
            .iter()
            .filter_map(|id| details.get(id).map(|d| vuln_of(id, d, p)))
            .collect();
        out.insert(vkey(p), vulns);
    }
    Ok((out, capped))
}

async fn json_body(resp: reqwest::Response) -> Result<Value, String> {
    let bytes = resp
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn vuln_of(id: &str, d: &Value, p: &Package) -> Vuln {
    let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
    let summary = s(d.get("summary"))
        .or_else(|| {
            s(d.get("details")).map(|t| t.lines().next().unwrap_or("").chars().take(200).collect())
        })
        .unwrap_or_default();
    let dbs = d.get("database_specific");
    let fixed = d
        .get("affected")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| {
            a.pointer("/package/name")
                .and_then(Value::as_str)
                .is_some_and(|n| norm(n) == norm(&p.name))
        })
        .flat_map(|a| {
            a.get("ranges")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .flat_map(|r| {
            r.get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|e| s(e.get("fixed")))
        .collect();
    Vuln {
        id: id.to_owned(),
        summary,
        aliases: d
            .get("aliases")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str().map(str::to_owned))
            .collect(),
        severity: s(dbs.and_then(|x| x.get("severity"))),
        cwe: dbs
            .and_then(|x| x.get("cwe_ids"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_owned),
        fixed,
    }
}

/// Numeric `major.minor.patch` (a leading `v` and any pre-release/build suffix ignored).
fn semver(v: &str) -> Option<[u64; 3]> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.');
    let mut out = [0u64; 3];
    out[0] = it.next()?.parse().ok()?;
    for slot in out.iter_mut().skip(1) {
        if let Some(x) = it.next() {
            *slot = x.parse().ok()?;
        }
    }
    Some(out)
}

/// The smallest fixed version above `current`.
fn first_fix<'a>(current: &str, fixed: &'a [String]) -> Option<&'a str> {
    let cur = semver(current);
    fixed
        .iter()
        .filter_map(|f| semver(f).map(|s| (s, f.as_str())))
        .filter(|(s, _)| cur.is_none_or(|c| *s > c))
        .min_by_key(|(s, _)| *s)
        .map(|(_, f)| f)
}

fn vuln_findings(inv: &Inventory, c: &Cache, l: &Localizer) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in &inv.packages {
        for v in c.vulns.get(&vkey(p)).into_iter().flatten() {
            let sev = v.severity.as_deref().and_then(Severity::parse);
            let cve = v.aliases.iter().find(|a| a.starts_with("CVE-"));
            let id = cve.map_or_else(|| v.id.clone(), |c| format!("{} / {c}", v.id));
            let mut message = l.t_args(
                "deps-vuln",
                &args!("pkg" => label(p), "id" => id, "summary" => v.summary.clone()),
            );
            if sev.is_none() {
                let _ = write!(message, " {}", l.t("deps-vuln-no-severity"));
            }
            message.push_str(&via_suffix(p, l));
            out.push(Finding {
                rule: v.id.clone(),
                source: "osv".into(),
                severity: sev.unwrap_or(Severity::Medium),
                file: p.file.clone(),
                line: p.line,
                column: None,
                message,
                fix: first_fix(&p.version, &v.fixed).map(|f| {
                    l.t_args(
                        "deps-vuln-fix",
                        &args!("pkg" => p.name.clone(), "version" => f.to_owned()),
                    )
                }),
                cwe: v.cwe.clone(),
            });
        }
    }
    out
}

fn registry_name(e: Ecosystem) -> &'static str {
    match e {
        Ecosystem::Cargo => "crates.io",
        Ecosystem::Npm => "npm",
        Ecosystem::PyPI => "pypi",
        Ecosystem::Packagist => "packagist",
        Ecosystem::Go => "proxy.golang.org",
        Ecosystem::NuGet | Ecosystem::RubyGems | Ecosystem::Maven => "",
    }
}

fn upgrade_findings(inv: &Inventory, c: &Cache, l: &Localizer) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in inv.direct() {
        let Some(latest) = c.latest.get(&lkey(p)) else {
            continue;
        };
        let (Some(cur), Some(new)) = (semver(&p.version), semver(&latest.version)) else {
            continue;
        };
        let note = latest.note.as_ref().map(|n| {
            format!(
                " {}",
                l.t_args("deps-registry-note", &args!("note" => n.clone()))
            )
        });
        if new <= cur && note.is_none() {
            continue;
        }
        let (rule, severity, kind) = if new <= cur {
            ("deprecated", Severity::Low, "")
        } else if new[0] != cur[0] {
            ("upgrade-major", Severity::Low, "major")
        } else if new[1] != cur[1] {
            ("upgrade-minor", Severity::Info, "minor")
        } else {
            ("upgrade-patch", Severity::Info, "patch")
        };
        let mut message = if kind.is_empty() {
            label(p)
        } else {
            l.t_args(
                "deps-upgrade",
                &args!(
                    "pkg" => p.name.clone(),
                    "from" => p.version.clone(),
                    "to" => latest.version.clone(),
                    "kind" => l.t(&format!("deps-kind-{kind}"))
                ),
            )
        };
        message.push_str(&note.unwrap_or_default());
        out.push(Finding {
            rule: rule.into(),
            source: registry_name(p.ecosystem).into(),
            severity,
            file: p.file.clone(),
            line: p.line,
            column: None,
            message,
            fix: None,
            cwe: None,
        });
    }
    out
}

fn go_escape(module: &str) -> String {
    module
        .chars()
        .flat_map(|c| {
            if c.is_ascii_uppercase() {
                vec!['!', c.to_ascii_lowercase()]
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Latest version from the package's registry; `Ok(None)` for registries Kor does not ask.
async fn latest(
    http: &reqwest::Client,
    ep: &Endpoints,
    p: &Package,
) -> Result<Option<Latest>, String> {
    let url = match p.ecosystem {
        Ecosystem::Cargo => format!("{}/api/v1/crates/{}", ep.crates, p.name),
        Ecosystem::Npm => format!("{}/{}/latest", ep.npm, p.name.replace('/', "%2F")),
        Ecosystem::PyPI => format!("{}/pypi/{}/json", ep.pypi, p.name),
        Ecosystem::Packagist => format!("{}/p2/{}.json", ep.packagist, p.name),
        Ecosystem::Go => format!("{}/{}/@latest", ep.goproxy, go_escape(&p.name)),
        Ecosystem::NuGet | Ecosystem::RubyGems | Ecosystem::Maven => return Ok(None),
    };
    kor_core::net::check_url(&url).map_err(|b| b.message)?;
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let v = json_body(resp).await?;
    let s = |x: Option<&Value>| x.and_then(Value::as_str).map(str::to_owned);
    Ok(match p.ecosystem {
        Ecosystem::Cargo => s(v.pointer("/crate/max_stable_version"))
            .or_else(|| s(v.pointer("/crate/newest_version")))
            .map(|version| Latest {
                version,
                note: None,
            }),
        Ecosystem::Npm => s(v.get("version")).map(|version| Latest {
            version,
            note: s(v.get("deprecated")),
        }),
        Ecosystem::PyPI => s(v.pointer("/info/version")).map(|version| Latest {
            version,
            note: (v.pointer("/info/yanked") == Some(&Value::Bool(true)))
                .then(|| "yanked".to_owned()),
        }),
        Ecosystem::Packagist => {
            let first = v.pointer(&format!(
                "/packages/{}/0",
                p.name.replace('~', "~0").replace('/', "~1")
            ));
            first.and_then(|f| {
                let note = match f.get("abandoned") {
                    Some(Value::Bool(true)) => Some("abandoned".to_owned()),
                    Some(Value::String(r)) => Some(format!("abandoned → {r}")),
                    _ => None,
                };
                s(f.get("version")).map(|version| Latest { version, note })
            })
        }
        Ecosystem::Go => s(v.get("Version")).map(|version| Latest {
            version: version.trim_start_matches('v').to_owned(),
            note: None,
        }),
        Ecosystem::NuGet | Ecosystem::RubyGems | Ecosystem::Maven => None,
    })
}

/// Curated groups of packages that do the same job (direct dependencies of one manifest).
const DUPLICATES: &[(Ecosystem, &str, &[&str])] = &[
    (
        Ecosystem::Npm,
        "dates",
        &["moment", "dayjs", "date-fns", "luxon"],
    ),
    (
        Ecosystem::Npm,
        "http",
        &["axios", "node-fetch", "got", "superagent", "ky", "request"],
    ),
    (
        Ecosystem::Npm,
        "utils",
        &["lodash", "lodash-es", "underscore", "ramda"],
    ),
    (Ecosystem::Npm, "test", &["jest", "vitest", "mocha", "ava"]),
    (Ecosystem::Npm, "bcrypt", &["bcrypt", "bcryptjs"]),
    (Ecosystem::Npm, "classnames", &["classnames", "clsx"]),
    (Ecosystem::PyPI, "http", &["requests", "httpx"]),
    (Ecosystem::PyPI, "json", &["simplejson", "ujson", "orjson"]),
    (
        Ecosystem::Cargo,
        "http",
        &["reqwest", "ureq", "isahc", "surf"],
    ),
    (Ecosystem::Cargo, "errors", &["anyhow", "eyre"]),
    (
        Ecosystem::Packagist,
        "http",
        &["guzzlehttp/guzzle", "symfony/http-client"],
    ),
];

fn duplicates(inv: &Inventory, l: &Localizer) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut by_file: BTreeMap<(&Path, Ecosystem), Vec<&Package>> = BTreeMap::new();
    for p in inv.direct() {
        by_file
            .entry((p.file.as_path(), p.ecosystem))
            .or_default()
            .push(p);
    }
    for ((_, eco), pkgs) in by_file {
        for (geco, purpose, group) in DUPLICATES {
            if *geco != eco {
                continue;
            }
            let mut hit: Vec<&Package> = pkgs
                .iter()
                .copied()
                .filter(|p| group.contains(&norm(&p.name).as_str()))
                .collect();
            hit.sort_by_key(|p| p.line);
            hit.dedup_by(|a, b| a.name == b.name);
            if hit.len() < 2 {
                continue;
            }
            let names: Vec<&str> = hit.iter().map(|p| p.name.as_str()).collect();
            out.push(Finding {
                rule: "duplicate-purpose".into(),
                source: "kor".into(),
                severity: Severity::Low,
                file: hit[1].file.clone(),
                line: hit[1].line,
                column: None,
                message: l.t_args(
                    "deps-duplicate",
                    &args!("purpose" => l.t(&format!("deps-purpose-{purpose}")), "names" => names.join(", ")),
                ),
                fix: None,
                cwe: None,
            });
        }
    }
    out
}

/// Python distributions whose import name differs from the package name.
const PY_IMPORTS: &[(&str, &str)] = &[
    ("pyyaml", "yaml"),
    ("pillow", "pil"),
    ("beautifulsoup4", "bs4"),
    ("scikit-learn", "sklearn"),
    ("python-dateutil", "dateutil"),
    ("opencv-python", "cv2"),
    ("opencv-python-headless", "cv2"),
    ("protobuf", "google"),
    ("python-dotenv", "dotenv"),
    ("attrs", "attr"),
    ("pyjwt", "jwt"),
    ("psycopg2-binary", "psycopg2"),
    ("pymupdf", "fitz"),
];

static JS_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\bfrom\s*|\brequire\s*\(\s*|\bimport\s*\(\s*|\bimport\s+)['"]([^'"\s]+)['"]"#)
        .expect("regex")
});
static RS_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)::|\b(?:extern crate|use)\s+([A-Za-z_][A-Za-z0-9_]*)")
        .expect("regex")
});
static PY_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:from\s+([A-Za-z_]\w*)|import\s+([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*))")
        .expect("regex")
});
static GO_STRING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([a-zA-Z0-9.\-]+\.[a-z]+/[^"\s]*)""#).expect("regex"));

#[derive(Default)]
struct Imports {
    seen: HashMap<Ecosystem, bool>,
    names: HashMap<Ecosystem, HashSet<String>>,
}

fn source_imports(root: &Path) -> Imports {
    let mut im = Imports::default();
    let walker = ignore::WalkBuilder::new(root)
        .max_depth(Some(12))
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build();
    // shortcut: 50k files of at most 1 MB each; bigger trees stop early (a note says so via fewer hits).
    for e in walker.flatten().take(50_000) {
        if !e.file_type().is_some_and(|t| t.is_file())
            || e.metadata().map_or(true, |m| m.len() > 1_000_000)
        {
            continue;
        }
        let ext = e
            .path()
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let eco = match ext.as_str() {
            "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" | "vue" | "svelte"
            | "astro" => Ecosystem::Npm,
            "rs" => Ecosystem::Cargo,
            "py" => Ecosystem::PyPI,
            "go" => Ecosystem::Go,
            _ => continue,
        };
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        im.seen.insert(eco, true);
        let set = im.names.entry(eco).or_default();
        match eco {
            Ecosystem::Npm => {
                for c in JS_IMPORT.captures_iter(&text) {
                    let spec = c[1].trim_start_matches("node:");
                    if spec.starts_with(['.', '/', '#', '~']) {
                        continue;
                    }
                    let mut parts = spec.split('/');
                    let first = parts.next().unwrap_or("");
                    let name = if first.starts_with('@') {
                        format!("{first}/{}", parts.next().unwrap_or(""))
                    } else {
                        first.to_owned()
                    };
                    set.insert(name);
                }
            }
            Ecosystem::Cargo => {
                for c in RS_PATH.captures_iter(&text) {
                    if let Some(m) = c.get(1).or_else(|| c.get(2)) {
                        set.insert(m.as_str().to_owned());
                    }
                }
            }
            Ecosystem::PyPI => {
                for c in PY_IMPORT.captures_iter(&text) {
                    let names = c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str());
                    set.extend(names.split(',').map(|n| n.trim().to_ascii_lowercase()));
                }
            }
            _ => {
                for c in GO_STRING.captures_iter(&text) {
                    set.insert(c[1].to_owned());
                }
            }
        }
    }
    im
}

/// `scripts`: the package.json `scripts` text (a CLI run from there counts as used).
fn is_used(p: &Package, set: &HashSet<String>, scripts: &str) -> bool {
    match p.ecosystem {
        Ecosystem::Npm => {
            set.contains(&p.name)
                || p.name.starts_with("@types/")
                || scripts.contains(p.name.rsplit('/').next().unwrap_or(&p.name))
        }
        Ecosystem::Cargo => set.contains(&p.name.replace('-', "_")),
        Ecosystem::PyPI => {
            let n = norm(&p.name);
            let mapped = PY_IMPORTS.iter().find(|(d, _)| *d == n).map(|(_, i)| *i);
            set.contains(&n.replace('-', "_")) || mapped.is_some_and(|m| set.contains(m))
        }
        Ecosystem::Go => set
            .iter()
            .any(|i| i == &p.name || i.starts_with(&format!("{}/", p.name))),
        _ => true,
    }
}

/// Direct runtime dependencies no source file imports. Only for ecosystems that have source
/// files here; the whole project is searched, so a monorepo never reports a sibling's import.
fn unused(root: &Path, inv: &Inventory, l: &Localizer) -> (Vec<Finding>, Vec<String>) {
    let candidates: Vec<&Package> = inv
        .direct()
        .filter(|p| {
            !p.dev
                && matches!(
                    p.ecosystem,
                    Ecosystem::Npm | Ecosystem::Cargo | Ecosystem::PyPI | Ecosystem::Go
                )
        })
        .collect();
    if candidates.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let scripts: String = candidates
        .iter()
        .filter(|p| p.ecosystem == Ecosystem::Npm)
        .map(|p| root.join(&p.file))
        .collect::<HashSet<_>>()
        .into_iter()
        .filter_map(|m| std::fs::read_to_string(m).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter_map(|v| v.get("scripts").map(Value::to_string))
        .collect();
    let im = source_imports(root);
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let mut skipped = HashSet::new();
    for p in candidates {
        if !im.seen.get(&p.ecosystem).copied().unwrap_or(false) {
            if skipped.insert(p.ecosystem) {
                notes.push(l.t_args(
                    "deps-unused-skipped",
                    &args!("ecosystem" => p.ecosystem.osv()),
                ));
            }
            continue;
        }
        if !is_used(
            p,
            im.names.get(&p.ecosystem).unwrap_or(&HashSet::new()),
            &scripts,
        ) {
            out.push(Finding {
                rule: "unused-dependency".into(),
                source: "kor".into(),
                severity: Severity::Low,
                file: p.file.clone(),
                line: p.line,
                column: None,
                message: l.t_args("deps-unused", &args!("pkg" => p.name.clone())),
                fix: None,
                cwe: None,
            });
        }
    }
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_and_fix_selection() {
        assert_eq!(semver("v1.2.3-rc.1"), Some([1, 2, 3]));
        assert_eq!(semver("2"), Some([2, 0, 0]));
        assert_eq!(semver("abc"), None);
        let fixed = vec!["1.0.5".to_owned(), "2.1.0".to_owned(), "1.2.0".to_owned()];
        assert_eq!(first_fix("1.1.0", &fixed), Some("1.2.0"));
        assert_eq!(first_fix("3.0.0", &fixed), None);
    }

    #[test]
    fn go_module_paths_are_escaped() {
        assert_eq!(
            go_escape("github.com/BurntSushi/toml"),
            "github.com/!burnt!sushi/toml"
        );
    }

    #[test]
    fn duplicate_groups_per_manifest() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let f = Path::new("package.json");
        let mk = |n: &str, line| {
            let mut p = Package::new(Ecosystem::Npm, n, "1.0.0", f, line);
            p.direct = true;
            p
        };
        let inv = Inventory {
            packages: vec![mk("moment", 3), mk("dayjs", 4), mk("react", 5)],
            ..Inventory::default()
        };
        let d = duplicates(&inv, &l);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 4);
        assert!(d[0].message.contains("moment, dayjs"), "{}", d[0].message);
    }

    #[test]
    fn imports_per_language() {
        let set: HashSet<String> = ["serde_json", "tokio"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let f = Path::new("Cargo.toml");
        assert!(is_used(
            &Package::new(Ecosystem::Cargo, "serde-json", "1", f, 1),
            &set,
            ""
        ));
        assert!(!is_used(
            &Package::new(Ecosystem::Cargo, "rand", "1", f, 1),
            &set,
            ""
        ));
        let py: HashSet<String> = ["yaml".to_owned()].into();
        assert!(is_used(
            &Package::new(Ecosystem::PyPI, "PyYAML", "6", f, 1),
            &py,
            ""
        ));
        let caps: Vec<String> = JS_IMPORT
            .captures_iter("import x from '@scope/pkg/sub';\nconst y = require(\"lodash\");\nimport './local';")
            .map(|c| c[1].to_owned())
            .collect();
        assert_eq!(caps, ["@scope/pkg/sub", "lodash", "./local"]);
    }
}
