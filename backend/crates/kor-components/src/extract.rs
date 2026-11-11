//! Unpacking verified downloads. Every entry path and link target must stay inside the
//! destination; one bad entry rejects the whole archive.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component as PathPart, Path};

use crate::manifest::{Archive, is_plain_relative};

/// `link` (a symlink stored at `entry`) must resolve inside the archive root.
fn link_stays_inside(entry: &Path, link: &Path) -> bool {
    if link.is_absolute() || link.has_root() {
        return false;
    }
    // Depth of the link's directory, then walk the target lexically.
    let mut depth = entry.components().count().saturating_sub(1);
    for part in link.components() {
        match part {
            PathPart::Normal(_) => depth += 1,
            PathPart::CurDir => {}
            PathPart::ParentDir => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            PathPart::RootDir | PathPart::Prefix(_) => return false,
        }
    }
    true
}

fn bad(name: &Path) -> String {
    format!(
        "archive entry escapes the install directory: {}",
        name.display()
    )
}

fn untar(reader: impl Read, dest: &Path) -> Result<(), String> {
    let mut ar = tar::Archive::new(reader);
    ar.set_overwrite(true);
    for entry in ar.entries().map_err(|e| e.to_string())? {
        let mut e = entry.map_err(|e| e.to_string())?;
        let path = e.path().map_err(|e| e.to_string())?.into_owned();
        if !is_plain_relative(&path) {
            if path.components().all(|p| p == PathPart::CurDir) {
                continue; // `./` itself
            }
            return Err(bad(&path));
        }
        let kind = e.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let target = e
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| bad(&path))?
                .into_owned();
            // Hard links name a path from the archive root, symlinks one from their own folder.
            let ok = if kind.is_hard_link() {
                is_plain_relative(&target)
            } else {
                link_stays_inside(&path, &target)
            };
            if !ok {
                return Err(bad(&path));
            }
        } else if !(kind.is_file() || kind.is_dir() || kind.is_contiguous()) {
            continue; // devices, fifos and pax/GNU metadata carry nothing we need
        }
        if !e.unpack_in(dest).map_err(|e| e.to_string())? {
            return Err(bad(&path));
        }
    }
    Ok(())
}

fn unzip(file: File, dest: &Path) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let raw = std::path::PathBuf::from(f.name().map_err(|e| e.to_string())?.into_owned());
        if f.enclosed_name().is_none() || !is_plain_relative(&raw) {
            return Err(bad(&raw));
        }
        if f.is_symlink() {
            let mut target = String::new();
            f.read_to_string(&mut target).map_err(|e| e.to_string())?;
            if !link_stays_inside(&raw, Path::new(&target)) {
                return Err(bad(&raw));
            }
        }
    }
    z.extract(dest).map_err(|e| e.to_string())
}

/// Unpack `file` into the (empty) `dest`. For [`Archive::File`] the download is copied to
/// `dest/<exe>`.
pub(crate) fn unpack(kind: Archive, file: &Path, dest: &Path, exe: &str) -> Result<(), String> {
    let open = || File::open(file).map_err(|e| e.to_string());
    match kind {
        Archive::Zip => unzip(open()?, dest),
        Archive::TarGz => untar(flate2::read::GzDecoder::new(BufReader::new(open()?)), dest),
        Archive::TarXz => untar(
            lzma_rust2::XzReader::new(BufReader::new(open()?), true),
            dest,
        ),
        Archive::File => {
            let to = dest.join(exe);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(file, to).map(drop).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::link_stays_inside;
    use std::path::Path;

    #[test]
    fn links_must_stay_inside() {
        assert!(link_stays_inside(
            Path::new("python/bin/python3"),
            Path::new("python3.13")
        ));
        assert!(link_stays_inside(Path::new("a/b/c"), Path::new("../lib/x")));
        assert!(!link_stays_inside(Path::new("a/b"), Path::new("../../etc")));
        assert!(!link_stays_inside(Path::new("a"), Path::new("/etc/passwd")));
    }
}
