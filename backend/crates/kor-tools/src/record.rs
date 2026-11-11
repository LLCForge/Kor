//! Session recording (§15.9, ADR 0043): `<project>/.kor/sessions/<id>/events.jsonl` (Kor's event
//! stream: prompt, tool calls, diffs, approvals, cost) and `session.cast` (asciinema v2: a header
//! line, then `[seconds, "o", text]` lines).
//!
//! This is only the writer. It never masks: everything handed to it must already have gone through
//! the session's `Redactor` (`kor_agent::record` does that at its single tap).

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Recordings of a project, relative to its root.
pub const DIR: &str = ".kor/sessions";
pub const EVENTS: &str = "events.jsonl";
pub const CAST: &str = "session.cast";
/// Terminal size written into the cast header.
pub const WIDTH: u16 = 120;
pub const HEIGHT: u16 = 40;

/// Letters, digits, `-` and `_`, 1–128 bytes: safe as a folder name.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Folder of recording `id` under `root`.
///
/// # Errors
/// If the id is not a safe folder name.
pub fn session_dir(root: &Path, id: &str) -> io::Result<PathBuf> {
    if !valid_id(id) {
        return Err(io::Error::other("invalid session id"));
    }
    Ok(root.join(DIR).join(id))
}

/// The asciinema v2 header line.
pub fn cast_header(id: &str, unix: u64) -> Value {
    json!({
        "version": 2,
        "width": WIDTH,
        "height": HEIGHT,
        "timestamp": unix,
        "title": format!("kor {id}"),
        "env": {"TERM": "xterm-256color"},
    })
}

/// Runs before a direct write so events still queued at the tap are written first.
pub type SyncFn = Arc<dyn Fn() + Send + Sync>;

struct Files {
    events: File,
    cast: File,
}

/// Appends to one session's recording. Each line is one `write_all`, so a crash loses at most
/// the line being written.
pub struct Recorder {
    id: String,
    dir: PathBuf,
    start: Instant,
    files: Mutex<Files>,
    sync: Mutex<Option<SyncFn>>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `t` of the last line of an existing `events.jsonl` (a resumed session keeps counting).
fn last_t(path: &Path) -> f64 {
    let Ok(f) = File::open(path) else { return 0.0 };
    io::BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok()?["t"].as_f64())
        .last()
        .unwrap_or(0.0)
}

impl Recorder {
    /// Start (or continue) recording `id` under `root/.kor/sessions/`. The folder gets a
    /// `.gitignore` so recordings are never committed by accident.
    ///
    /// # Errors
    /// If the id is unsafe or the files cannot be created.
    pub fn create(root: &Path, id: &str) -> io::Result<Self> {
        let dir = session_dir(root, id)?;
        fs::create_dir_all(&dir)?;
        let ignore = root.join(DIR).join(".gitignore");
        if !ignore.exists() {
            fs::write(&ignore, "*\n")?;
        }
        let events_path = dir.join(EVENTS);
        let offset = last_t(&events_path);
        let cast_path = dir.join(CAST);
        let new_cast = !cast_path.exists();
        let open = |p: &Path| OpenOptions::new().create(true).append(true).open(p);
        let mut cast = open(&cast_path)?;
        if new_cast {
            let unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            cast.write_all(format!("{}\n", cast_header(id, unix)).as_bytes())?;
        }
        let start = Instant::now()
            .checked_sub(Duration::from_secs_f64(offset))
            .unwrap_or_else(Instant::now);
        Ok(Self {
            id: id.to_owned(),
            dir,
            start,
            files: Mutex::new(Files {
                events: open(&events_path)?,
                cast,
            }),
            sync: Mutex::new(None),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Seconds since the recording started, millisecond precision.
    pub fn now(&self) -> f64 {
        (self.start.elapsed().as_secs_f64() * 1000.0).round() / 1000.0
    }

    /// Append one event (an object; `t` is set here). The content must be masked already.
    pub fn write(&self, mut event: Value) {
        if let Some(o) = event.as_object_mut() {
            o.insert("t".into(), json!(self.now()));
        }
        let line = format!("{event}\n");
        let _ = lock(&self.files).events.write_all(line.as_bytes());
    }

    /// Append terminal output to the cast (`\n` becomes `\r\n` as a terminal shows it). The text
    /// must be masked already.
    pub fn output(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
        let line = format!("{}\n", json!([self.now(), "o", text]));
        let _ = lock(&self.files).cast.write_all(line.as_bytes());
    }

    /// Set what [`Recorder::sync`] runs (the tap's flush), or clear it.
    pub fn set_sync(&self, f: Option<SyncFn>) {
        *lock(&self.sync) = f;
    }

    /// Write out whatever the tap still holds, before a direct [`Recorder::write`].
    pub fn sync(&self) {
        let f = lock(&self.sync).clone();
        if let Some(f) = f {
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_valid_cast_and_event_lines_and_resumes_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recorder::create(dir.path(), "s-1").unwrap();
        r.write(json!({"kind": "prompt", "text": "hi"}));
        r.output("a\nb");
        r.output("");
        drop(r);
        let cast = fs::read_to_string(dir.path().join(DIR).join("s-1").join(CAST)).unwrap();
        let lines: Vec<Value> = cast
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{cast}");
        assert_eq!(lines[0]["version"], 2);
        assert_eq!(lines[0]["width"], WIDTH);
        assert!(lines[0]["timestamp"].as_u64().unwrap() > 0);
        assert!(lines[1][0].is_f64() && lines[1][1] == "o" && lines[1][2] == "a\r\nb");
        assert_eq!(
            fs::read_to_string(dir.path().join(DIR).join(".gitignore")).unwrap(),
            "*\n"
        );

        // Resume: no second header, time continues from the last event.
        let events = dir.path().join(DIR).join("s-1").join(EVENTS);
        fs::write(&events, "{\"kind\":\"prompt\",\"t\":30.5}\n").unwrap();
        let r = Recorder::create(dir.path(), "s-1").unwrap();
        assert!(r.now() >= 30.5);
        r.output("c");
        let cast = fs::read_to_string(dir.path().join(DIR).join("s-1").join(CAST)).unwrap();
        assert_eq!(
            cast.lines().filter(|l| l.contains("\"version\"")).count(),
            1
        );
    }

    #[test]
    fn unsafe_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "../x", "a/b", "a b"] {
            assert!(Recorder::create(dir.path(), bad).is_err(), "{bad}");
        }
    }
}
