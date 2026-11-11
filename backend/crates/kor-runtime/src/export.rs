//! `kor export --all` (Master Build Prompt §22.2): every piece of the user's data in one zip, so
//! nothing ties them to Kor. Kor's data folder (sessions, memory, knowledge base, audit log, undo
//! journals, gallery, notifications) and settings folder go in as they are: plain Markdown, JSON
//! and SQLite. Left out: secrets (they live in the OS keychain; key-like files are skipped too),
//! downloaded components and caches (re-downloadable, not the user's data), lock files.
//! `manifest.json` at the archive root lists every file and where it came from.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// What one export wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Serialize)]
struct Manifest<'a> {
    kor: &'a str,
    created: u64,
    roots: Vec<(&'a str, String)>,
    files: Vec<String>,
    skipped: Vec<String>,
}

/// Folders that are not the user's data (re-downloadable or rebuilt).
const SKIP_DIRS: &[&str] = &[
    "components",
    "cache",
    "caches",
    "tmp",
    "models",
    "runtimes",
    "bin",
    "node_modules",
    "target",
];

fn skipped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".lock")
        || n.ends_with(".key")
        || n.ends_with(".pem")
        || n.ends_with(".p12")
        || n.ends_with(".pfx")
        || n.contains("secret")
        || n == ".env"
        || n.starts_with(".env.")
}

fn walk(
    base: &Path,
    dir: &Path,
    prefix: &str,
    files: &mut Vec<(String, PathBuf)>,
    skip: &mut Vec<String>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = path
            .strip_prefix(base)
            .map_or_else(|_| name.clone(), |r| r.to_string_lossy().replace('\\', "/"));
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            skip.push(format!("{prefix}/{rel}"));
        } else if ft.is_dir() {
            if SKIP_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
                skip.push(format!("{prefix}/{rel}/"));
            } else {
                walk(base, &path, prefix, files, skip);
            }
        } else if skipped(&name) {
            skip.push(format!("{prefix}/{rel}"));
        } else {
            files.push((format!("{prefix}/{rel}"), path));
        }
    }
}

/// Export `roots` (archive folder name → source folder) into the zip at `out`.
///
/// # Errors
/// When the archive cannot be written.
pub fn export_from(
    roots: &[(&str, PathBuf)],
    out: &Path,
    version: &str,
) -> Result<Summary, String> {
    let mut files = Vec::new();
    let mut skip = Vec::new();
    for (name, dir) in roots {
        // The archive itself may sit inside an exported folder: never include it.
        walk(dir, dir, name, &mut files, &mut skip);
    }
    let out_abs = std::fs::canonicalize(out.parent().unwrap_or(Path::new(".")))
        .ok()
        .map(|p| p.join(out.file_name().unwrap_or_default()));
    files.retain(|(_, p)| std::fs::canonicalize(p).ok() != out_abs);
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut bytes = 0u64;
    for (name, path) in &files {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        bytes += data.len() as u64;
        z.start_file(name.as_str(), opts)
            .map_err(|e| e.to_string())?;
        z.write_all(&data).map_err(|e| e.to_string())?;
    }
    let manifest = Manifest {
        kor: version,
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        roots: roots
            .iter()
            .map(|(n, p)| (*n, p.display().to_string()))
            .collect(),
        files: files.iter().map(|(n, _)| n.clone()).collect(),
        skipped: skip,
    };
    z.start_file("manifest.json", opts)
        .map_err(|e| e.to_string())?;
    z.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    z.finish().map_err(|e| e.to_string())?;
    Ok(Summary {
        path: out.to_owned(),
        files: files.len(),
        bytes,
    })
}

/// `kor export --all`: Kor's data folder and settings folder.
///
/// # Errors
/// When no data folder is known or the archive cannot be written.
pub fn export_all(out: &Path, version: &str) -> Result<Summary, String> {
    let mut roots = Vec::new();
    if let Some(home) = crate::kor_home().filter(|p| p.is_dir()) {
        roots.push(("data", home));
    }
    if let Some(cfg) = kor_config::user_dir().filter(|p| p.is_dir()) {
        roots.push(("config", cfg));
    }
    if roots.is_empty() {
        return Err("no Kor data folder on this computer".into());
    }
    export_from(&roots, out, version)
}

/// Default archive name: `kor-export-YYYYMMDD-HHMMSS.zip` in the current folder.
pub fn default_name() -> String {
    format!(
        "kor-export-{}.zip",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn exports_user_data_and_leaves_secrets_caches_and_itself_out() {
        let d = tempfile::tempdir().unwrap();
        let data = d.path().join("data");
        std::fs::create_dir_all(data.join("sessions")).unwrap();
        std::fs::create_dir_all(data.join("components/llama")).unwrap();
        std::fs::write(data.join("sessions/a.jsonl"), "{\"role\":\"user\"}\n").unwrap();
        std::fs::write(data.join("knowledge.sqlite"), b"SQLite format 3\0").unwrap();
        std::fs::write(data.join("components/llama/model.gguf"), b"big").unwrap();
        std::fs::write(data.join("vault-secret.json"), "x").unwrap();
        std::fs::write(data.join("updater.key"), "x").unwrap();
        let cfg = d.path().join("config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.toml"), "[ui]\nlanguage = \"tr\"\n").unwrap();
        let out = data.join("export.zip");
        let s = export_from(&[("data", data.clone()), ("config", cfg)], &out, "0.8.0").unwrap();
        assert_eq!(s.files, 3, "{s:?}");
        let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = z.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "config/config.toml",
                "data/knowledge.sqlite",
                "data/sessions/a.jsonl",
                "manifest.json"
            ]
        );
        let mut m = String::new();
        z.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut m)
            .unwrap();
        assert!(
            m.contains("\"kor\": \"0.8.0\"")
                && m.contains("data/components/")
                && m.contains("updater.key"),
            "{m}"
        );
    }
}
