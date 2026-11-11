//! Skills that ship with Kor (`assets/builtin-skills.zip`, built by `scripts/pack-skills.py`):
//! agent-skills, humanizer, scroll-craft, `HyperFrames` and the awesome index. They are unpacked
//! into `<kor home>/builtin-skills/` the first time a session starts (and again when Kor ships a
//! new set), then load like a plugin: listed in the system prompt with their descriptions, so the
//! model picks the one that matches what the user wrote. They load last, so a user or project
//! skill with the same name wins.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

static ARCHIVE: &[u8] = include_bytes!("../../../../assets/builtin-skills.zip");
const STAMP: &str = ".kor-builtin-stamp";
/// Largest single file unpacked (the archive is ours; this only guards a damaged copy).
const MAX_FILE: u64 = 8 * 1024 * 1024;

fn fingerprint(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().take(8).fold(String::new(), |mut s, b| {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
        s
    })
}

/// The unpacked skills folder (a plugin root: `skills/<name>/SKILL.md`), unpacking it first when
/// missing or from another Kor version. `None` when the folder cannot be written; Kor then runs
/// without the built-in skills.
pub fn ensure(home: &Path) -> Option<PathBuf> {
    ensure_from(home, ARCHIVE)
}

fn ensure_from(home: &Path, archive: &[u8]) -> Option<PathBuf> {
    let dir = home.join("builtin-skills");
    let want = fingerprint(archive);
    if std::fs::read_to_string(dir.join(STAMP)).is_ok_and(|s| s.trim() == want) {
        return Some(dir);
    }
    // Unpack beside it and swap, so a session never sees half a set.
    let tmp = home.join(format!("builtin-skills.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = unpack(archive, &tmp) {
        let _ = std::fs::remove_dir_all(&tmp);
        tracing_note(&format!("built-in skills not installed: {e}"));
        return None;
    }
    std::fs::write(tmp.join(STAMP), &want).ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::rename(&tmp, &dir).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
        // Another process installed the same set at the same moment: use it.
        return std::fs::read_to_string(dir.join(STAMP))
            .is_ok_and(|s| s.trim() == want)
            .then_some(dir);
    }
    Some(dir)
}

fn unpack(archive: &[u8], to: &Path) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        // Never write outside `to` (`..`, absolute paths).
        let Some(rel) = f.enclosed_name() else {
            return Err(format!("unsafe path in archive (entry {i})"));
        };
        let path = to.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if f.size() > MAX_FILE {
            return Err(format!("{}: too large", rel.display()));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut data = Vec::with_capacity(usize::try_from(f.size()).unwrap_or(0));
        f.by_ref()
            .take(MAX_FILE)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn tracing_note(msg: &str) {
    eprintln!("kor: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_set_unpacks_once_and_loads_as_skills() {
        let home = tempfile::tempdir().unwrap();
        let dir = ensure(home.path()).expect("installed");
        assert!(dir.join("skills/humanizer/SKILL.md").is_file());
        assert!(dir.join("skills/hyperframes/SKILL.md").is_file());
        assert!(dir.join("skills/awesome-lists/awesome.md").is_file());
        assert!(dir.join("skills/humanizer/LICENSE.upstream").is_file());
        // Second call: stamp matches, nothing is rewritten.
        let marker = dir.join("skills/humanizer/marker");
        std::fs::write(&marker, "x").unwrap();
        assert_eq!(ensure(home.path()), Some(dir.clone()));
        assert!(marker.is_file(), "an up-to-date set is left alone");

        let mut c = kor_compat::load(&kor_compat::LoadOptions {
            root: home.path().to_owned(),
            home: None,
            codex_home: None,
            trusted: false,
        });
        kor_compat::add_plugin(&mut c, &dir);
        let names: Vec<&str> = c.skills.iter().map(|s| s.name.as_str()).collect();
        for want in [
            "humanizer",
            "scroll-craft",
            "hyperframes",
            "awesome-lists",
            "test-driven-development",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
        assert!(c.skills.len() >= 37, "{}", c.skills.len());
    }

    #[test]
    fn a_new_set_replaces_the_old_and_bad_paths_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let make = |files: &[(&str, &str)]| {
            let mut buf = std::io::Cursor::new(Vec::new());
            {
                let mut z = zip::ZipWriter::new(&mut buf);
                for (name, body) in files {
                    z.start_file(*name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    std::io::Write::write_all(&mut z, body.as_bytes()).unwrap();
                }
                z.finish().unwrap();
            }
            buf.into_inner()
        };
        let v1 = make(&[("skills/a/SKILL.md", "---\nname: a\ndescription: x\n---\n")]);
        let v2 = make(&[("skills/b/SKILL.md", "---\nname: b\ndescription: y\n---\n")]);
        let dir = ensure_from(home.path(), &v1).unwrap();
        assert!(dir.join("skills/a/SKILL.md").is_file());
        ensure_from(home.path(), &v2).unwrap();
        assert!(!dir.join("skills/a").exists(), "the old set is gone");
        assert!(dir.join("skills/b/SKILL.md").is_file());
        let evil = make(&[("../escape.txt", "no")]);
        let other = tempfile::tempdir().unwrap();
        assert_eq!(ensure_from(other.path(), &evil), None);
        assert!(!other.path().parent().unwrap().join("escape.txt").exists());
    }
}
