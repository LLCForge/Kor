//! Kor's on-demand helper programs (Master Build Prompt §24.2, "Katman B").
//!
//! Each component (Python, Node, the `PDFium` renderer, Tesseract models, headless Chromium) is
//! pinned in `components.toml` with a per-target URL, exact size and SHA-256. On first use it is
//! downloaded over HTTPS (resumable, size-capped), verified, unpacked into a temporary folder and
//! renamed into `<kor data>/components/<name>/<version>/` in one step. No admin rights, no PATH
//! changes; deleting that folder removes everything. Privacy mode never downloads; an archive
//! fetched elsewhere can be installed with [`Store::add_from`] (`kor component add --from`).
//!
//! Callers ask the user before the first download ([`info`] gives size and version); `ensure`
//! itself only reports progress.

pub mod bundle;
mod extract;
mod fetch;
mod manifest;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub use manifest::{Archive, Artifact, Component, Manifest};

/// Download progress of one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub name: String,
    pub done: u64,
    pub total: Option<u64>,
}

static PRIVACY: AtomicBool = AtomicBool::new(false);
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Turn downloads off (privacy mode). `kor-runtime` sets it with the session's privacy mode.
pub fn set_privacy(on: bool) {
    PRIVACY.store(on, Ordering::Relaxed);
}

pub fn privacy() -> bool {
    PRIVACY.load(Ordering::Relaxed)
}

/// This machine as a manifest target: `<os>-<arch>[-musl]`, e.g. `windows-x86_64`.
pub fn target() -> String {
    let musl = if cfg!(target_env = "musl") {
        "-musl"
    } else {
        ""
    };
    format!("{}-{}{musl}", std::env::consts::OS, std::env::consts::ARCH)
}

/// What `kor component list` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub name: String,
    pub version: String,
    pub about: String,
    /// Download size for this machine; `None` when no build exists for it.
    pub size: Option<u64>,
    /// Installed main file.
    pub installed: Option<PathBuf>,
}

/// What a check of one installed component found (`kor component verify`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Every installed file matches what was recorded at install time.
    Ok,
    /// The current version is not installed.
    Missing,
    /// An older version is installed; `kor component update` brings the pinned one.
    Outdated { installed: String },
    /// Files that are gone or changed since install (`/`-separated paths).
    Broken(Vec<String>),
    /// Installed before Kor kept a file list; only the main file could be checked.
    Unrecorded,
}

/// Installed file list written next to every install: path → (size, sha256).
const RECEIPT: &str = ".kor-files.json";

/// Every file under `dir` as `/`-separated relative path → (size, sha256), the receipt itself
/// left out.
fn record(dir: &Path) -> Result<std::collections::BTreeMap<String, (u64, String)>, String> {
    fn walk(
        base: &Path,
        dir: &Path,
        out: &mut std::collections::BTreeMap<String, (u64, String)>,
    ) -> Result<(), String> {
        for e in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let p = e.path();
            let ft = e.file_type().map_err(|e| e.to_string())?;
            if ft.is_dir() {
                walk(base, &p, out)?;
            } else if ft.is_file() {
                let rel = p
                    .strip_prefix(base)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel != RECEIPT {
                    let size = e.metadata().map_err(|e| e.to_string())?.len();
                    out.insert(rel, (size, fetch::sha256_file(&p)?));
                }
            }
        }
        Ok(())
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Compare `dir` with its receipt. `deep` re-hashes every file; otherwise only presence and
/// size are checked (cheap enough for every `ensure`, even for multi-GB models).
fn check_dir(dir: &Path, deep: bool) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(RECEIPT)).ok()?;
    let files: std::collections::BTreeMap<String, (u64, String)> =
        serde_json::from_str(&text).ok()?;
    let bad = files
        .iter()
        .filter(|(rel, (size, sum))| {
            let p = native(dir, rel);
            match std::fs::metadata(&p) {
                Ok(m) if m.len() == *size => {
                    deep && fetch::sha256_file(&p).ok().as_deref() != Some(sum.as_str())
                }
                _ => true,
            }
        })
        .map(|(rel, _)| rel.clone())
        .collect();
    Some(bad)
}

/// Components under one root, described by one manifest.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    manifest: Manifest,
    offline: bool,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>, manifest: Manifest) -> Self {
        Self {
            root: root.into(),
            manifest,
            offline: false,
        }
    }

    /// `<kor data>/components` with the built-in manifest.
    pub fn open() -> Option<Self> {
        kor_platform::data_dir().map(|d| Self::new(d.join("components"), Manifest::builtin()))
    }

    /// Refuse downloads even when the global privacy switch is off.
    #[must_use]
    pub fn offline(mut self, on: bool) -> Self {
        self.offline = on;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn build(&self, name: &str, target: &str) -> Result<(&Component, &Artifact), String> {
        let c = self
            .manifest
            .get(name)
            .ok_or_else(|| format!("unknown component `{name}`"))?;
        let a = c
            .artifact(target)
            .ok_or_else(|| format!("component `{name}` has no published build for {target} yet"))?;
        Ok((c, a))
    }

    fn version_dir(&self, c: &Component) -> PathBuf {
        self.root.join(&c.name).join(&c.version)
    }

    /// Installed main file for `target`, without network.
    pub fn path_for(&self, name: &str, target: &str) -> Option<PathBuf> {
        let (c, a) = self.build(name, target).ok()?;
        let p = native(&self.version_dir(c), &a.exe);
        p.exists().then_some(p)
    }

    pub fn list(&self, target: &str) -> Vec<Status> {
        self.manifest
            .components
            .iter()
            .map(|c| Status {
                name: c.name.clone(),
                version: c.version.clone(),
                about: c.about.clone(),
                size: c.artifact(target).map(|a| a.size),
                installed: self.path_for(&c.name, target),
            })
            .collect()
    }

    /// Download (resuming a partial file), verify and install; the components it `requires`
    /// first.
    pub async fn ensure_for(
        &self,
        name: &str,
        target: &str,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<PathBuf, String> {
        if let Some(c) = self.manifest.get(name) {
            for dep in &c.requires {
                self.ensure_one(dep, target, progress).await?;
            }
        }
        self.ensure_one(name, target, progress).await
    }

    async fn ensure_one(
        &self,
        name: &str,
        target: &str,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<PathBuf, String> {
        if let Some(p) = self.path_for(name, target) {
            let (c, _) = self.build(name, target)?;
            let dir = self.version_dir(c);
            // A file deleted or cut short since install: drop the copy and download it again.
            if check_dir(&dir, false).is_none_or(|bad| bad.is_empty()) {
                return Ok(p);
            }
            tokio::fs::remove_dir_all(&dir)
                .await
                .map_err(|e| e.to_string())?;
        }
        let (c, a) = self.build(name, target)?;
        if self.offline || privacy() {
            return Err(format!(
                "privacy mode is on, so Kor does not download `{name}`. Install it offline: \
                 kor component add {name} --from <file> (file: {}, sha256 {})",
                a.url, a.sha256
            ));
        }
        let _guard = INSTALL.lock().await;
        if let Some(p) = self.path_for(name, target) {
            return Ok(p); // installed while we waited
        }
        let dir = self.root.join(&c.name);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let part = dir.join(format!("{}.part", c.version));
        let mut last = String::new();
        for _ in 0..3 {
            match fetch::download(&a.url, &part, a.size, name, progress).await {
                Ok(()) => {
                    last.clear();
                    break;
                }
                Err(e) => last = e,
            }
        }
        if !last.is_empty() {
            return Err(format!("could not download `{name}`: {last}"));
        }
        let (store, c, a, part2) = (self.clone(), c.clone(), a.clone(), part.clone());
        let installed = tokio::task::spawn_blocking(move || store.install(&c, &a, &part2, true))
            .await
            .map_err(|e| e.to_string())?;
        if installed.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        installed
    }

    /// The published file of `name` for `target` (for an air-gap bundle), without installing
    /// it: an installed single-file component (a model) is used as it is, anything else is
    /// downloaded into `cache` (resumable, size-capped) and verified. Never in privacy mode.
    pub async fn artifact_file(
        &self,
        name: &str,
        target: &str,
        cache: &Path,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<PathBuf, String> {
        let (c, a) = self.build(name, target)?;
        if a.archive == Archive::File
            && let Some(p) = self.path_for(name, target)
            && std::fs::metadata(&p).is_ok_and(|m| m.len() == a.size)
        {
            return Ok(p); // verified when it was installed
        }
        let ext = match a.archive {
            Archive::Zip => "zip",
            Archive::TarGz => "tar.gz",
            Archive::TarXz => "tar.xz",
            Archive::File => "bin",
        };
        let dest = cache.join(format!("{}-{}-{}.{ext}", c.name, c.version, a.target));
        let verified = |p: &Path| {
            std::fs::metadata(p).is_ok_and(|m| m.len() == a.size)
                && fetch::sha256_file(p).is_ok_and(|s| s == a.sha256)
        };
        if verified(&dest) {
            return Ok(dest);
        }
        if self.offline || privacy() {
            return Err(format!(
                "privacy mode is on, so Kor does not download `{name}` ({})",
                a.url
            ));
        }
        tokio::fs::create_dir_all(cache)
            .await
            .map_err(|e| e.to_string())?;
        let part = dest.with_extension(format!("{ext}.part"));
        let mut last = String::new();
        for _ in 0..3 {
            match fetch::download(&a.url, &part, a.size, name, progress).await {
                Ok(()) => {
                    last.clear();
                    break;
                }
                Err(e) => last = e,
            }
        }
        if !last.is_empty() {
            return Err(format!("could not download `{name}`: {last}"));
        }
        if !verified(&part) {
            let _ = std::fs::remove_file(&part);
            return Err(format!(
                "checksum mismatch for `{name}`: expected sha256 {}",
                a.sha256
            ));
        }
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        Ok(dest)
    }

    /// Install from an archive already on disk (offline / air-gapped). Same SHA-256 check.
    pub fn add_from(&self, name: &str, target: &str, file: &Path) -> Result<PathBuf, String> {
        let (c, a) = self.build(name, target)?;
        self.install(c, a, file, false)
    }

    /// Verify, unpack into a temporary folder, rename into place, drop older versions.
    fn install(
        &self,
        c: &Component,
        a: &Artifact,
        file: &Path,
        consume: bool,
    ) -> Result<PathBuf, String> {
        let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
        let sum = fetch::sha256_file(file)?;
        if size != a.size || sum != a.sha256 {
            return Err(format!(
                "checksum mismatch for `{}`: expected sha256 {}, got {sum}",
                c.name, a.sha256
            ));
        }
        let dir = self.root.join(&c.name);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!(".tmp-{}-{}", c.version, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let result = (|| {
            extract::unpack(a.archive, file, &tmp, &a.exe)?;
            let exe = native(&tmp, &a.exe);
            if !exe.exists() {
                return Err(format!("the `{}` archive has no {}", c.name, a.exe));
            }
            kor_platform::make_executable(&exe).map_err(|e| e.to_string())?;
            let files = record(&tmp)?;
            std::fs::write(
                tmp.join(RECEIPT),
                serde_json::to_vec(&files).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let fin = self.version_dir(c);
            if fin.exists() {
                std::fs::remove_dir_all(&fin).map_err(|e| e.to_string())?;
            }
            rename_retry(&tmp, &fin)?;
            Ok(native(&fin, &a.exe))
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
            return result;
        }
        if consume {
            let _ = std::fs::remove_file(file);
        }
        // Older versions and leftovers of this component.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_name() != c.version.as_str() {
                    let p = e.path();
                    let _ = if p.is_dir() {
                        std::fs::remove_dir_all(p)
                    } else {
                        std::fs::remove_file(p)
                    };
                }
            }
        }
        result
    }

    /// Delete every installed version of `name`. `Ok(false)` when nothing was installed.
    pub fn remove(&self, name: &str) -> Result<bool, String> {
        let c = self
            .manifest
            .get(name)
            .ok_or_else(|| format!("unknown component `{name}`"))?;
        let dir = self.root.join(&c.name);
        if !dir.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Installed version folders of `name` (temporary and partial downloads left out).
    fn installed_versions(&self, name: &str) -> Vec<String> {
        std::fs::read_dir(self.root.join(name)).map_or_else(
            |_| Vec::new(),
            |rd| {
                rd.flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !n.starts_with(".tmp-"))
                    .collect()
            },
        )
    }

    /// State of `name` on disk; `deep` re-hashes every installed file.
    pub fn health(&self, name: &str, target: &str, deep: bool) -> Health {
        let Ok((c, _)) = self.build(name, target) else {
            return Health::Missing;
        };
        if self.path_for(name, target).is_none() {
            return self
                .installed_versions(name)
                .into_iter()
                .find(|v| *v != c.version)
                .map_or(Health::Missing, |installed| Health::Outdated { installed });
        }
        match check_dir(&self.version_dir(c), deep) {
            None => Health::Unrecorded,
            Some(bad) if bad.is_empty() => Health::Ok,
            Some(bad) => Health::Broken(bad),
        }
    }

    /// Components with an older version installed and a newer pinned build for `target`.
    pub fn outdated(&self, target: &str) -> Vec<String> {
        self.manifest
            .components
            .iter()
            .filter(|c| matches!(self.health(&c.name, target, false), Health::Outdated { .. }))
            .map(|c| c.name.clone())
            .collect()
    }

    /// Deep-check `name` and delete a broken copy, so the next use downloads it again.
    ///
    /// # Errors
    /// When the broken copy cannot be deleted.
    pub fn verify(&self, name: &str, target: &str) -> Result<Health, String> {
        let h = self.health(name, target, true);
        if let (Health::Broken(_), Ok((c, _))) = (&h, self.build(name, target)) {
            std::fs::remove_dir_all(self.version_dir(c)).map_err(|e| e.to_string())?;
        }
        Ok(h)
    }
}

/// `dir` + a `/`-separated manifest path, with the platform's separators.
fn native(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part))
}

/// Antivirus scanners briefly hold freshly written files on Windows; retry the rename.
fn rename_retry(from: &Path, to: &Path) -> Result<(), String> {
    let mut last = None;
    for _ in 0..10 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(last.map_or_else(|| "rename failed".to_owned(), |e| e.to_string()))
}

/// The manifest name for a user-facing alias: `browser` is the headless Chromium (spec §15.6).
pub fn canonical(name: &str) -> &str {
    if name.eq_ignore_ascii_case("browser") {
        "chromium"
    } else {
        name
    }
}

/// Name, version, about text and download size for this machine (for consent prompts).
pub fn info(name: &str) -> Option<Status> {
    let store = Store::open()?;
    store.list(&target()).into_iter().find(|s| s.name == name)
}

/// All components with their state on this machine.
pub fn list() -> Vec<Status> {
    Store::open().map(|s| s.list(&target())).unwrap_or_default()
}

/// Installed component's main executable or directory, if present (no network).
pub fn path(name: &str) -> Option<PathBuf> {
    Store::open()?.path_for(name, &target())
}

/// Make sure a component is installed under `<kor data>/components/<name>/`, downloading and
/// verifying it on first use (never in privacy mode). Returns the executable/dir path.
pub async fn ensure(
    name: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<PathBuf, String> {
    let store = Store::open().ok_or("no data directory for Kor components")?;
    store.ensure_for(name, &target(), progress).await
}

/// Offline install of `name` from a local archive (verified against the pinned SHA-256).
pub fn add_from(name: &str, file: &Path) -> Result<PathBuf, String> {
    Store::open()
        .ok_or("no data directory for Kor components")?
        .add_from(name, &target(), file)
}

/// Remove an installed component.
pub fn remove(name: &str) -> Result<bool, String> {
    Store::open()
        .ok_or("no data directory for Kor components")?
        .remove(name)
}

/// Deep-check `name` on this machine (broken copies are deleted for re-download).
pub fn verify(name: &str) -> Result<Health, String> {
    Store::open()
        .ok_or("no data directory for Kor components")?
        .verify(name, &target())
}

/// Components with an older version installed.
pub fn outdated() -> Vec<String> {
    Store::open()
        .map(|s| s.outdated(&target()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn target_names_this_machine() {
        let t = super::target();
        assert!(t.starts_with(std::env::consts::OS), "{t}");
        assert!(t.contains(std::env::consts::ARCH), "{t}");
    }

    #[test]
    fn browser_is_the_chromium_component() {
        assert_eq!(super::canonical("browser"), "chromium");
        assert_eq!(super::canonical("node"), "node");
        let m = super::Manifest::builtin();
        assert!(m.get(super::canonical("browser")).is_some());
    }
}
