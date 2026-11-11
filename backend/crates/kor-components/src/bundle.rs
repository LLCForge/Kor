//! Air-gap bundle (Master Build Prompt §6.5, ADR 0034): one zip archive with the Kor CLI, the
//! local model runtime, models, the embedding model and a model catalog snapshot, plus
//! `kor-bundle.json` listing the SHA-256 and size of every other file.
//!
//! Files are stored uncompressed (they are compressed archives or model weights already) and
//! ZIP64 is used past 4 GiB. Writing goes to `<out>.part` and is renamed at the end. Reading
//! checks every listed file against the manifest while extracting; files already extracted
//! with the right hash are skipped, so an interrupted import continues where it stopped.
//! The manifest itself is not signed: components are checked again against the hashes pinned
//! in the binary when they are installed, and the CLI binary needs its minisign signature.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::Progress;
use crate::manifest::is_plain_relative;

/// Name of the manifest inside the archive.
pub const MANIFEST: &str = "kor-bundle.json";
pub const FORMAT: u32 = 1;
const MAX_MANIFEST: u64 = 4 * 1024 * 1024;
const MAX_FILES: usize = 10_000;

/// What a bundled file is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The `kor` executable (`bin/kor[.exe]`).
    Cli,
    /// Its minisign signature (`bin/kor[.exe].minisig`).
    Signature,
    /// A desktop installer, copied as it is.
    Desktop,
    /// A component archive as published (installed with the pinned hash check).
    Component,
    /// The model catalog snapshot (`model-catalog.json`).
    Catalog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// `/`-separated path inside the archive.
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
    pub kind: Kind,
    /// Component name for [`Kind::Component`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub kor_version: String,
    /// Component target the files are for (`windows-x86_64`).
    pub platform: String,
    /// Unix seconds.
    pub created: u64,
    /// Suggested `[offline]` settings for the importing machine.
    #[serde(default)]
    pub runtime: String,
    #[serde(default)]
    pub model: String,
    pub files: Vec<Entry>,
}

impl Manifest {
    /// Sizes, hashes and paths are well formed; paths are unique and relative.
    pub fn check(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("unsupported bundle format {}", self.format));
        }
        if self.files.len() > MAX_FILES {
            return Err("bundle lists too many files".into());
        }
        let mut seen = std::collections::HashSet::new();
        for e in &self.files {
            let hex = e.sha256.len() == 64
                && e.sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !hex
                || e.path == MANIFEST
                || e.path.contains('\\')
                || !is_plain_relative(Path::new(&e.path))
                || !seen.insert(e.path.as_str())
                || (e.kind == Kind::Component) != e.component.is_some()
            {
                return Err(format!("bad bundle entry `{}`", e.path));
            }
        }
        Ok(())
    }

    pub fn total(&self) -> u64 {
        self.files.iter().map(|e| e.size).sum()
    }
}

/// One file to put in a bundle.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: String,
    pub file: PathBuf,
    pub kind: Kind,
    pub component: Option<String>,
}

fn hex(d: &[u8]) -> String {
    d.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Copy `r` into `w` while hashing; at most `limit` bytes are accepted.
fn copy_hashed(
    mut r: impl Read,
    w: &mut impl Write,
    limit: u64,
    name: &str,
    progress: &dyn Fn(Progress),
) -> Result<(u64, String), String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut done = 0u64;
    let mut reported = 0u64;
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > limit {
            return Err(format!("`{name}` is larger than expected"));
        }
        h.update(&buf[..n]);
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        if done - reported >= 8 * 1024 * 1024 {
            reported = done;
            progress(Progress {
                name: name.to_owned(),
                done,
                total: (limit != u64::MAX).then_some(limit),
            });
        }
    }
    Ok((done, hex(&h.finalize())))
}

/// Write `files` and the manifest (`meta` with `files` filled in) into `out`.
pub fn write(
    out: &Path,
    mut meta: Manifest,
    files: &[Source],
    progress: &dyn Fn(Progress),
) -> Result<Manifest, String> {
    let part = PathBuf::from(format!("{}.part", out.display()));
    let result = (|| {
        let f = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(f));
        meta.files.clear();
        for s in files {
            let size = std::fs::metadata(&s.file)
                .map_err(|e| format!("{}: {e}", s.file.display()))?
                .len();
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(size >= u64::from(u32::MAX));
            zip.start_file(s.path.as_str(), opts)
                .map_err(|e| e.to_string())?;
            let src =
                std::fs::File::open(&s.file).map_err(|e| format!("{}: {e}", s.file.display()))?;
            progress(Progress {
                name: s.path.clone(),
                done: 0,
                total: Some(size),
            });
            let (n, sha256) = copy_hashed(src, &mut zip, size, &s.path, progress)?;
            if n != size {
                return Err(format!("{} changed while it was copied", s.file.display()));
            }
            progress(Progress {
                name: s.path.clone(),
                done: n,
                total: Some(size),
            });
            meta.files.push(Entry {
                path: s.path.clone(),
                size,
                sha256,
                kind: s.kind,
                component: s.component.clone(),
            });
        }
        meta.check()?;
        let json = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
        zip.start_file(MANIFEST, zip::write::SimpleFileOptions::default())
            .map_err(|e| e.to_string())?;
        zip.write_all(&json).map_err(|e| e.to_string())?;
        let mut w = zip.finish().map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        w.get_ref().sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, out).map_err(|e| e.to_string())?;
    Ok(meta)
}

fn open(bundle: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(bundle).map_err(|e| format!("{}: {e}", bundle.display()))?;
    zip::ZipArchive::new(f).map_err(|e| {
        format!(
            "{} is not a complete Kor bundle (copy it again): {e}",
            bundle.display()
        )
    })
}

/// The checked manifest of a bundle.
pub fn read_manifest(bundle: &Path) -> Result<Manifest, String> {
    let mut zip = open(bundle)?;
    let mut text = Vec::new();
    zip.by_name(MANIFEST)
        .map_err(|_| format!("{} has no {MANIFEST}", bundle.display()))?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_MANIFEST {
        return Err(format!("{MANIFEST} is too large"));
    }
    let m: Manifest = serde_json::from_slice(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    m.check()?;
    Ok(m)
}

fn sha256_file(p: &Path) -> Option<String> {
    crate::fetch::sha256_file(p).ok()
}

/// Extract and verify every listed file into `dest` (`dest/<path>`). A file already there with
/// the right size and hash is kept (resume). Any mismatch deletes that file and fails.
pub fn extract(
    bundle: &Path,
    dest: &Path,
    progress: &dyn Fn(Progress),
) -> Result<Manifest, String> {
    let m = read_manifest(bundle)?;
    let mut zip = open(bundle)?;
    for e in &m.files {
        let target = e.path.split('/').fold(dest.to_path_buf(), |p, s| p.join(s));
        if std::fs::metadata(&target).is_ok_and(|md| md.len() == e.size)
            && sha256_file(&target).as_deref() == Some(e.sha256.as_str())
        {
            progress(Progress {
                name: e.path.clone(),
                done: e.size,
                total: Some(e.size),
            });
            continue;
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let part = PathBuf::from(format!("{}.part", target.display()));
        let r = zip
            .by_name(&e.path)
            .map_err(|_| format!("bundle is missing `{}`", e.path))?;
        let mut w = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let copied = copy_hashed(r, &mut w, e.size, &e.path, progress);
        drop(w);
        match copied {
            Ok((n, sum)) if n == e.size && sum == e.sha256 => {}
            Ok(_) => {
                let _ = std::fs::remove_file(&part);
                return Err(format!(
                    "`{}` in the bundle does not match its sha256 (damaged or altered)",
                    e.path
                ));
            }
            Err(err) => {
                let _ = std::fs::remove_file(&part);
                return Err(err);
            }
        }
        std::fs::rename(&part, &target).map_err(|e| e.to_string())?;
        progress(Progress {
            name: e.path.clone(),
            done: e.size,
            total: Some(e.size),
        });
    }
    Ok(m)
}

/// One listed file's bytes, verified against the manifest (at most `max` bytes).
pub fn read_file(bundle: &Path, path: &str, max: u64) -> Result<Vec<u8>, String> {
    let m = read_manifest(bundle)?;
    let e = m
        .files
        .iter()
        .find(|e| e.path == path)
        .ok_or_else(|| format!("bundle has no `{path}`"))?;
    if e.size > max {
        return Err(format!("`{path}` is too large"));
    }
    let mut zip = open(bundle)?;
    let r = zip
        .by_name(path)
        .map_err(|_| format!("bundle is missing `{path}`"))?;
    let mut out = Vec::new();
    let (n, sum) = copy_hashed(r, &mut out, e.size, path, &|_| {})?;
    if n != e.size || sum != e.sha256 {
        return Err(format!(
            "`{path}` in the bundle does not match its sha256 (damaged or altered)"
        ));
    }
    Ok(out)
}

/// Starts like a zip file (`PK\x03\x04`).
pub fn is_bundle(file: &Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(file)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == *b"PK\x03\x04"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Manifest {
        Manifest {
            format: FORMAT,
            kor_version: "0.0.0".into(),
            platform: "windows-x86_64".into(),
            created: 1,
            runtime: "local-llamacpp".into(),
            model: "local-llamacpp/tiny".into(),
            files: Vec::new(),
        }
    }

    fn make(dir: &Path) -> (PathBuf, Manifest) {
        std::fs::write(dir.join("a.bin"), b"model weights").unwrap();
        std::fs::write(dir.join("c.json"), b"{\"models\":[]}").unwrap();
        let out = dir.join("b.zip");
        let srcs = [
            Source {
                path: "components/tiny/a.bin".into(),
                file: dir.join("a.bin"),
                kind: Kind::Component,
                component: Some("tiny".into()),
            },
            Source {
                path: "catalog/model-catalog.json".into(),
                file: dir.join("c.json"),
                kind: Kind::Catalog,
                component: None,
            },
        ];
        let m = write(&out, meta(), &srcs, &|_| {}).unwrap();
        (out, m)
    }

    #[test]
    fn round_trip_and_resume() {
        let dir = tempfile::tempdir().unwrap();
        let (out, m) = make(dir.path());
        assert!(is_bundle(&out));
        assert!(!PathBuf::from(format!("{}.part", out.display())).exists());
        assert_eq!(m.files.len(), 2);
        assert_eq!(read_manifest(&out).unwrap(), m);
        let dest = dir.path().join("x");
        extract(&out, &dest, &|_| {}).unwrap();
        let got = dest.join("components").join("tiny").join("a.bin");
        assert_eq!(std::fs::read(&got).unwrap(), b"model weights");
        // A half-written file from an interrupted import is replaced; a complete one is kept.
        std::fs::write(&got, b"model").unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        extract(&out, &dest, &|p| seen.lock().unwrap().push(p.done)).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), b"model weights");
        assert_eq!(
            read_file(&out, "catalog/model-catalog.json", 1024).unwrap(),
            b"{\"models\":[]}"
        );
        assert!(read_file(&out, "catalog/model-catalog.json", 3).is_err());
    }

    #[test]
    fn tampered_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (out, _) = make(dir.path());
        // Same length, different bytes: the stored (uncompressed) data is patched in place.
        let mut raw = std::fs::read(&out).unwrap();
        let at = raw.windows(13).position(|w| w == b"model weights").unwrap();
        raw[at] = b'M';
        std::fs::write(&out, &raw).unwrap();
        let dest = dir.path().join("x");
        let err = extract(&out, &dest, &|_| {}).unwrap_err();
        assert!(
            err.contains("sha256") || err.contains("CRC") || err.contains("checksum"),
            "{err}"
        );
        assert!(!dest.join("components").join("tiny").join("a.bin").exists());
        assert!(read_file(&out, "components/tiny/a.bin", 1024).is_err());
    }

    #[test]
    fn truncated_bundle_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (out, _) = make(dir.path());
        let raw = std::fs::read(&out).unwrap();
        std::fs::write(&out, &raw[..raw.len() / 2]).unwrap();
        assert!(read_manifest(&out).unwrap_err().contains("not a complete"));
    }

    #[test]
    fn manifest_rejects_unsafe_paths() {
        let mut m = meta();
        let ok = Entry {
            path: "bin/kor".into(),
            size: 1,
            sha256: "0".repeat(64),
            kind: Kind::Cli,
            component: None,
        };
        m.files.push(ok.clone());
        assert!(m.check().is_ok());
        for bad in ["../x", "/etc/x", "a\\..\\b", MANIFEST, ""] {
            let mut m = meta();
            m.files.push(Entry {
                path: bad.into(),
                ..ok.clone()
            });
            assert!(m.check().is_err(), "{bad}");
        }
        let mut m = meta();
        m.files.push(ok.clone());
        m.files.push(ok);
        assert!(m.check().is_err(), "duplicate");
    }
}
