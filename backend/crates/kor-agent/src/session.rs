//! Session transcripts as JSONL (one `Message` per line) for `kor resume`.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use kor_core::Message;

const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;

/// Sortable, collision-resistant id: `YYYYMMDD-HHMMSS-<pid>`.
pub fn new_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let date = crate::prompt::today().replace('-', "");
    let t = secs % 86_400;
    format!(
        "{date}-{:02}{:02}{:02}-{}",
        t / 3600,
        t / 60 % 60,
        t % 60,
        std::process::id()
    )
}

/// Atomically replace the transcript. Owner-only permissions on Unix.
pub fn save(path: &Path, messages: &[Message]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    for m in messages {
        serde_json::to_writer(&mut f, m).map_err(std::io::Error::other)?;
        f.write_all(b"\n")?;
    }
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)
}

pub fn load(path: &Path) -> std::io::Result<Vec<Message>> {
    let f = fs::File::open(path)?;
    BufReader::new(f.take(MAX_SESSION_BYTES))
        .lines()
        .filter(|l| l.as_ref().map_or(true, |l| !l.trim().is_empty()))
        .map(|l| serde_json::from_str(&l?).map_err(std::io::Error::other))
        .collect()
}

/// Session files in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    v.sort();
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_core::Role;

    #[test]
    fn round_trip_and_listing() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("20260101-000000-1.jsonl");
        let b = dir.path().join("20260102-000000-1.jsonl");
        let msgs = vec![
            Message::text(Role::User, "hi"),
            Message::text(Role::Assistant, "hello\nthere"),
        ];
        save(&a, &msgs).unwrap();
        save(&b, &msgs[..1]).unwrap();
        assert_eq!(load(&a).unwrap(), msgs);
        assert_eq!(list(dir.path()), vec![b, a]);
        assert!(new_id().len() > 16);
    }
}
