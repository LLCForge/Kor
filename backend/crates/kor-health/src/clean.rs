//! `kor clean --suggest` (§19.3): build caches and tool caches that can be deleted, with sizes.
//! Nothing is deleted without the user's confirmation, and only folders that are recognised as a
//! cache again at deletion time: a project's `target/` next to `Cargo.toml` (or with
//! `CACHEDIR.TAG`), `node_modules` next to `package.json`, Python and JS tool caches, the package
//! managers' download caches, Kor's own disposable cache, and Docker's dangling images and build
//! cache (through `docker`, never by touching its files). Symbolic links are never followed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One thing that can be cleaned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Stable id (kind + path) for `kor clean <id>`.
    pub id: String,
    /// `cargo`, `node`, `python`, `js`, `gradle`, `npm`, `pip`, `yarn`, `go`, `cargo-registry`,
    /// `kor`, `docker`.
    pub kind: String,
    /// Folder (none for `docker`).
    pub path: Option<PathBuf>,
    pub bytes: u64,
    /// The size walk hit its time limit: `bytes` is a lower bound (shown as "≥").
    pub approx: bool,
}

/// Where to look.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// Project folders searched for build caches (depth ≤ [`MAX_DEPTH`]).
    pub roots: Vec<PathBuf>,
    /// User caches: package managers' download caches.
    pub globals: Vec<(String, PathBuf)>,
    /// Ask `docker system df` for reclaimable space.
    pub docker: bool,
    /// Size walk budget per folder.
    pub budget: Duration,
}

pub const MAX_DEPTH: usize = 5;

impl Scan {
    /// The current machine's global caches (only folders that exist are listed later).
    pub fn standard(roots: Vec<PathBuf>) -> Self {
        let mut globals = Vec::new();
        let home = dirs::home_dir();
        let cache = dirs::cache_dir();
        let local = dirs::data_local_dir();
        let mut add = |kind: &str, p: Option<PathBuf>| {
            if let Some(p) = p {
                globals.push((kind.to_owned(), p));
            }
        };
        add("npm", home.as_ref().map(|h| h.join(".npm").join("_cacache")));
        add("npm", local.as_ref().map(|d| d.join("npm-cache").join("_cacache")));
        add("pip", cache.as_ref().map(|d| d.join("pip")));
        add("pip", local.as_ref().map(|d| d.join("pip").join("Cache")));
        add("yarn", cache.as_ref().map(|d| d.join("yarn")));
        add("yarn", cache.as_ref().map(|d| d.join("Yarn")));
        add("yarn", local.as_ref().map(|d| d.join("Yarn").join("Cache")));
        add("go", cache.as_ref().map(|d| d.join("go-build")));
        add("go", local.as_ref().map(|d| d.join("go-build")));
        add("cargo-registry", home.as_ref().map(|h| h.join(".cargo").join("registry").join("cache")));
        // Kor's disposable cache, only where it is not also Kor's data folder (Windows shares them).
        if let (Some(c), d) = (kor_platform::cache_dir(), kor_platform::data_dir())
            && d.as_ref() != Some(&c)
        {
            globals.push(("kor".into(), c));
        }
        globals.sort();
        globals.dedup();
        Self {
            roots,
            globals,
            docker: true,
            budget: Duration::from_secs(4),
        }
    }
}

fn id_of(kind: &str, path: Option<&Path>) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    if let Some(p) = path {
        h.update(p.to_string_lossy().as_bytes());
    }
    h.finalize().iter().take(5).map(|b| format!("{b:02x}")).collect()
}

fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

/// The cache kind of a project folder, or `None` when it is not one Kor may delete.
pub fn project_cache_kind(dir: &Path) -> Option<&'static str> {
    if is_link(dir) || !dir.is_dir() {
        return None;
    }
    let name = dir.file_name()?.to_str()?;
    let parent = dir.parent()?;
    let sibling = |f: &str| parent.join(f).is_file();
    match name {
        "target" if sibling("Cargo.toml") || dir.join("CACHEDIR.TAG").is_file() => Some("cargo"),
        "node_modules" if sibling("package.json") => Some("node"),
        "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" => Some("python"),
        ".turbo" | ".parcel-cache" => Some("js"),
        ".next" | ".nuxt" | ".svelte-kit" if sibling("package.json") => Some("js"),
        ".gradle" if sibling("build.gradle") || sibling("build.gradle.kts") || sibling("settings.gradle") || sibling("settings.gradle.kts") => {
            Some("gradle")
        }
        _ => None,
    }
}

fn find_project_caches(dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if let Some(kind) = project_cache_kind(&p) {
            out.push((kind.to_owned(), p));
            continue;
        }
        let name = e.file_name();
        let hidden = name.to_string_lossy().starts_with('.');
        if depth < MAX_DEPTH && !hidden {
            find_project_caches(&p, depth + 1, out);
        }
    }
}

/// Size of `dir` without following links; `(bytes, complete)`.
pub fn dir_size(dir: &Path, budget: Duration) -> (u64, bool) {
    let deadline = Instant::now() + budget;
    let mut total = 0u64;
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        if Instant::now() > deadline {
            return (total, false);
        }
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue;
            }
            if t.is_dir() {
                stack.push(e.path());
            } else if let Ok(m) = e.metadata() {
                total = total.saturating_add(m.len());
            }
        }
    }
    (total, true)
}

/// Folders only (no sizes): what [`suggest`] measures and [`clean`] re-checks.
fn locate(scan: &Scan) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for r in &scan.roots {
        find_project_caches(r, 0, &mut found);
    }
    for (kind, p) in &scan.globals {
        if p.is_dir() && !is_link(p) {
            found.push((kind.clone(), p.clone()));
        }
    }
    found.sort();
    found.dedup();
    found
}

/// `1.2GB`, `800MB`, `12.5kB`, `0B` (docker's units, powers of 1000).
pub fn parse_docker_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let mul = match unit.trim().to_ascii_uppercase().as_str() {
        "B" => 1.0,
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((n * mul) as u64)
}

/// `docker system df --format '{{json .}}'` → (total size, reclaimable).
pub fn parse_docker_df(out: &str) -> (u64, u64) {
    let mut total = 0;
    let mut reclaim = 0;
    for l in out.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
        total += v["Size"].as_str().and_then(parse_docker_size).unwrap_or(0);
        reclaim += v["Reclaimable"]
            .as_str()
            .and_then(|r| parse_docker_size(r.split(" (").next().unwrap_or(r)))
            .unwrap_or(0);
    }
    (total, reclaim)
}

fn docker(args: &[&str]) -> Option<String> {
    use kor_platform::NoWindow as _;
    let exe = kor_platform::on_path::which("docker")?;
    let out = std::process::Command::new(exe)
        .args(args)
        .no_window()
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Docker's total disk use and what pruning can reclaim (`None` without a running Docker).
pub fn docker_usage() -> Option<(u64, u64)> {
    docker(&["system", "df", "--format", "{{json .}}"]).map(|o| parse_docker_df(&o))
}

/// Everything that can be cleaned, biggest first.
pub fn suggest(scan: &Scan) -> Vec<Candidate> {
    let mut v: Vec<Candidate> = locate(scan)
        .into_iter()
        .map(|(kind, p)| {
            let (bytes, complete) = dir_size(&p, scan.budget);
            Candidate {
                id: id_of(&kind, Some(&p)),
                kind,
                path: Some(p),
                bytes,
                approx: !complete,
            }
        })
        .filter(|c| c.bytes > 0)
        .collect();
    if scan.docker
        && let Some((_, reclaim)) = docker_usage()
        && reclaim > 0
    {
        v.push(Candidate {
            id: id_of("docker", None),
            kind: "docker".into(),
            path: None,
            bytes: reclaim,
            approx: false,
        });
    }
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    v
}

/// Result of cleaning one candidate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub id: String,
    pub kind: String,
    pub path: Option<PathBuf>,
    pub freed: u64,
    pub error: Option<String>,
}

/// Delete the candidates with these ids — after the user confirmed. Each folder is found again by
/// scanning (an id never names an arbitrary path) and must still be a recognised cache.
pub fn clean(scan: &Scan, ids: &[String]) -> Vec<Outcome> {
    let mut out = Vec::new();
    let located = locate(scan);
    for id in ids {
        if let Some((kind, p)) = located.iter().find(|(k, p)| &id_of(k, Some(p)) == id) {
            let still = if scan.globals.iter().any(|(k, g)| k == kind && g == p) {
                p.is_dir() && !is_link(p)
            } else {
                project_cache_kind(p).is_some()
            };
            if !still {
                out.push(Outcome { id: id.clone(), kind: kind.clone(), path: Some(p.clone()), freed: 0, error: Some("not a cache folder any more".into()) });
                continue;
            }
            let (bytes, _) = dir_size(p, scan.budget);
            let error = std::fs::remove_dir_all(p).err().map(|e| e.to_string());
            out.push(Outcome {
                id: id.clone(),
                kind: kind.clone(),
                path: Some(p.clone()),
                freed: if error.is_none() { bytes } else { 0 },
                error,
            });
        } else if *id == id_of("docker", None) && scan.docker {
            let before = docker_usage().map_or(0, |(t, _)| t);
            let a = docker(&["builder", "prune", "--force"]);
            let b = docker(&["image", "prune", "--force"]);
            let after = docker_usage().map_or(before, |(t, _)| t);
            out.push(Outcome {
                id: id.clone(),
                kind: "docker".into(),
                path: None,
                freed: before.saturating_sub(after),
                error: (a.is_none() && b.is_none()).then(|| "docker prune failed".into()),
            });
        } else {
            out.push(Outcome { id: id.clone(), kind: String::new(), path: None, freed: 0, error: Some("unknown id; run `kor clean --suggest` again".into()) });
        }
    }
    out
}

/// `12.3 GB`, `640 MB`, `12 kB` (powers of 1024, like file managers on Windows).
#[allow(clippy::cast_precision_loss)]
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 || v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, bytes: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn suggests_only_recognised_caches_and_deletes_only_on_their_ids() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("proj");
        write(&proj.join("Cargo.toml"), 10);
        write(&proj.join("target/debug/a.rlib"), 5000);
        write(&proj.join("web/package.json"), 10);
        write(&proj.join("web/node_modules/x/index.js"), 3000);
        // Not caches: a `target` folder of a non-Rust project, `node_modules` without package.json.
        write(&proj.join("docs/target/keep.md"), 9000);
        write(&proj.join("lib/node_modules/keep.js"), 9000);
        write(&proj.join("src/__pycache__/m.pyc"), 100);
        let global = d.path().join("pipcache");
        write(&global.join("http/a"), 700);

        let scan = Scan { roots: vec![proj.clone()], globals: vec![("pip".into(), global.clone())], docker: false, budget: Duration::from_secs(5) };
        let s = suggest(&scan);
        let kinds: Vec<(&str, u64)> = s.iter().map(|c| (c.kind.as_str(), c.bytes)).collect();
        assert_eq!(kinds, [("cargo", 5000), ("node", 3000), ("pip", 700), ("python", 100)]);
        assert!(s.iter().all(|c| !c.approx));

        // Nothing happens without ids; an unknown id deletes nothing.
        assert!(clean(&scan, &[]).is_empty());
        let r = clean(&scan, &["deadbeef00".into()]);
        assert!(r[0].error.is_some());
        assert!(proj.join("target").exists());

        let cargo = s.iter().find(|c| c.kind == "cargo").unwrap().id.clone();
        let pip = s.iter().find(|c| c.kind == "pip").unwrap().id.clone();
        let r = clean(&scan, &[cargo, pip]);
        assert_eq!(r.iter().map(|o| o.freed).sum::<u64>(), 5700);
        assert!(!proj.join("target").exists() && !global.exists());
        assert!(proj.join("docs/target/keep.md").exists() && proj.join("lib/node_modules/keep.js").exists());
        assert!(proj.join("web/node_modules").exists() && proj.join("Cargo.toml").exists());
    }

    #[test]
    fn docker_and_sizes() {
        assert_eq!(parse_docker_size("1.5GB"), Some(1_500_000_000));
        assert_eq!(parse_docker_size("0B"), Some(0));
        assert_eq!(parse_docker_size("12.5kB"), Some(12_500));
        let df = "{\"Type\":\"Images\",\"Size\":\"10GB\",\"Reclaimable\":\"4GB (40%)\"}\n{\"Type\":\"Build Cache\",\"Size\":\"2GB\",\"Reclaimable\":\"2GB\"}\n";
        assert_eq!(parse_docker_df(df), (12_000_000_000, 6_000_000_000));
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1536), "1.5 kB");
        assert_eq!(human(18 * 1024 * 1024 * 1024), "18.0 GB");
    }
}
