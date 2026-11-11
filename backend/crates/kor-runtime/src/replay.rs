//! Session replay (§15.9, ADR 0043): read recordings written by `kor_agent`'s tap
//! (`.kor/sessions/<id>/events.jsonl` + `session.cast`), list them, step through them as a
//! timeline (pause, forward, back, the diff at that point) and export one shareable file.
//! Shared by `kor replay` and the desktop `replay.*` bridge.

use std::fs;
use std::path::{Path, PathBuf};

use kor_tools::record::{CAST, DIR, EVENTS, session_dir, valid_id};
use serde::Serialize;
use serde_json::{Value, json};

/// Format tag of an exported file.
pub const EXPORT_FORMAT: &str = "kor-replay/1";
/// Larger recordings are refused (protects the desktop and the terminal).
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// One recording in `kor replay list`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    /// Unix seconds from the cast header.
    pub started: Option<u64>,
    /// Seconds from the first to the last event.
    pub duration: f64,
    pub prompts: usize,
    /// First prompt, one line, at most 120 characters.
    pub title: String,
    pub tools: usize,
    pub events: usize,
    /// Sum of the recorded model call costs; `estimated` when any was priced from the catalog.
    pub cost_usd: f64,
    pub cost_estimated: bool,
    pub bytes: u64,
}

/// A loaded recording.
#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    pub id: String,
    /// asciinema v2 header.
    pub header: Value,
    /// `[time, "o", text]` lines.
    pub cast: Vec<Value>,
    pub events: Vec<Value>,
}

fn one_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim();
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line.to_owned()
    }
}

fn read_lines(path: &Path) -> Result<Vec<Value>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{}: recording is too large", path.display()));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // A line cut by a crash is skipped, not fatal.
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

/// Recordings of the project at `root`, newest first.
pub fn list(root: &Path) -> Vec<Summary> {
    let Ok(dir) = fs::read_dir(root.join(DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<Summary> = dir
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_owned();
            valid_id(&id).then(|| load(root, &id).ok().map(|r| summarize(&r, &e.path())))?
        })
        .collect();
    out.sort_by(|a, b| b.started.cmp(&a.started).then_with(|| b.id.cmp(&a.id)));
    out
}

fn summarize(r: &Recording, dir: &Path) -> Summary {
    let of = |k: &'static str| r.events.iter().filter(move |e| e["kind"] == k);
    let t = |e: Option<&Value>| e.and_then(|e| e["t"].as_f64()).unwrap_or(0.0);
    let bytes = [EVENTS, CAST]
        .iter()
        .filter_map(|f| fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum();
    Summary {
        id: r.id.clone(),
        started: r.header["timestamp"].as_u64(),
        duration: (t(r.events.last()) - t(r.events.first())).max(0.0),
        prompts: of("prompt").count(),
        title: of("prompt")
            .next()
            .map(|e| one_line(e["text"].as_str().unwrap_or_default(), 120))
            .unwrap_or_default(),
        tools: of("tool_start").count(),
        events: r.events.len(),
        cost_usd: of("usage").filter_map(|e| e["cost_usd"].as_f64()).sum(),
        cost_estimated: of("usage").any(|e| e["cost_estimated"] == true),
        bytes,
    }
}

/// Load recording `id` of `root`, or an exported file when `id` is a path to one.
///
/// # Errors
/// Unknown id, unreadable files or a file that is not a Kor export.
pub fn load(root: &Path, id: &str) -> Result<Recording, String> {
    if !valid_id(id) {
        return import(Path::new(id));
    }
    let dir = session_dir(root, id).map_err(|e| e.to_string())?;
    if !dir.is_dir() {
        let p = Path::new(id);
        return if p.is_file() {
            import(p)
        } else {
            Err(format!(
                "no recording `{id}` in {}",
                root.join(DIR).display()
            ))
        };
    }
    let events = read_lines(&dir.join(EVENTS))?;
    let mut cast = read_lines(&dir.join(CAST)).unwrap_or_default();
    let header = if cast.first().is_some_and(Value::is_object) {
        cast.remove(0)
    } else {
        Value::Null
    };
    Ok(Recording {
        id: id.to_owned(),
        header,
        cast,
        events,
    })
}

fn import(path: &Path) -> Result<Recording, String> {
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{}: file is too large", path.display()));
    }
    let v: Value = serde_json::from_str(
        &fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    if v["format"] != EXPORT_FORMAT {
        return Err(format!("{}: not a Kor replay export", path.display()));
    }
    let arr = |k: &str| v[k].as_array().cloned().unwrap_or_default();
    Ok(Recording {
        id: v["id"].as_str().unwrap_or("import").to_owned(),
        header: v["header"].clone(),
        cast: arr("cast"),
        events: arr("events"),
    })
}

/// The recording as one shareable JSON document (cast and events together). Known key formats
/// and URL passwords are masked once more, in case the recording predates a registered secret.
pub fn export_value(r: &Recording) -> Value {
    let mask = kor_secrets::Redactor::default();
    walk(
        &mask,
        json!({
            "format": EXPORT_FORMAT,
            "id": r.id,
            "header": r.header,
            "cast": r.cast,
            "events": r.events,
        }),
    )
}

fn walk(m: &kor_secrets::Redactor, v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(m.redact_outbound(&s).0.into_owned()),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| walk(m, x)).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, walk(m, x))).collect()),
        other => other,
    }
}

/// Write [`export_value`] to `out` (default `<root>/kor-replay-<id>.json`).
///
/// # Errors
/// Unknown recording or a write failure.
pub fn export(root: &Path, id: &str, out: Option<&Path>) -> Result<PathBuf, String> {
    let r = load(root, id)?;
    let out = out.map_or_else(
        || root.join(format!("kor-replay-{}.json", r.id)),
        Path::to_path_buf,
    );
    let text = serde_json::to_string_pretty(&export_value(&r)).map_err(|e| e.to_string())?;
    fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out)
}

/// One step of the timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    /// Index into the recording's events.
    pub index: usize,
    pub t: f64,
    pub kind: String,
    /// One line for the timeline (`● Bash cargo test`).
    pub title: String,
}

/// Step-by-step position in a recording: pause is the caller's, forward / back / seek here.
#[derive(Debug, Clone)]
pub struct Timeline {
    pub steps: Vec<Step>,
    pos: usize,
}

/// The timeline line of an event.
pub fn title(e: &Value) -> String {
    let s = |k: &str| e[k].as_str().unwrap_or_default();
    match s("kind") {
        "prompt" => format!("> {}", one_line(s("text"), 100)),
        "text" | "thinking" => one_line(s("text"), 100),
        "tool_start" => format!("● {} {}", s("name"), one_line(s("summary"), 80)),
        "tool_end" => {
            let mark = if e["is_error"] == true { "✗" } else { "✓" };
            format!("  {mark} {} {}", s("name"), one_line(s("output"), 80))
        }
        "diff" => {
            let files: Vec<&str> = e["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f["path"].as_str())
                .collect();
            format!("Δ {}", files.join(", "))
        }
        "ask" => format!("? {} {}", s("tool"), one_line(s("summary"), 80)),
        "approval" => format!("  → {}", s("reply")),
        "usage" => {
            let cost = e["cost_usd"].as_f64().map_or(String::new(), |c| {
                let approx = if e["cost_estimated"] == true {
                    "≈"
                } else {
                    ""
                };
                format!(" {approx}${c:.4}")
            });
            format!("¤ {} in / {} out{cost}", e["input"], e["output"])
        }
        "notice" => format!("! {}", one_line(s("text"), 100)),
        "end" => format!(
            "■ {}",
            if s("error").is_empty() {
                s("stop")
            } else {
                s("error")
            }
        ),
        other => other.to_owned(),
    }
}

impl Timeline {
    pub fn new(events: &[Value]) -> Self {
        let steps = events
            .iter()
            .enumerate()
            .map(|(index, e)| Step {
                index,
                t: e["t"].as_f64().unwrap_or(0.0),
                kind: e["kind"].as_str().unwrap_or_default().to_owned(),
                title: title(e),
            })
            .collect();
        Self { steps, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.pos)
    }

    pub fn at_end(&self) -> bool {
        self.pos + 1 >= self.steps.len()
    }

    /// One step forward; `false` at the end.
    pub fn forward(&mut self) -> bool {
        let moved = !self.at_end();
        if moved {
            self.pos += 1;
        }
        moved
    }

    /// One step back; `false` at the start.
    pub fn back(&mut self) -> bool {
        let moved = self.pos > 0;
        if moved {
            self.pos -= 1;
        }
        moved
    }

    /// Move to step `i` (clamped).
    pub fn go(&mut self, i: usize) {
        self.pos = i.min(self.steps.len().saturating_sub(1));
    }

    /// The last step at or before `t` seconds.
    pub fn seek(&mut self, t: f64) {
        self.pos = self.steps.iter().rposition(|s| s.t <= t).unwrap_or(0);
    }

    /// Seconds to wait before the next step when playing (`speed` × faster, long pauses capped).
    pub fn delay(&self, speed: f64) -> f64 {
        let (Some(a), Some(b)) = (self.steps.get(self.pos), self.steps.get(self.pos + 1)) else {
            return 0.0;
        };
        ((b.t - a.t).max(0.0) / speed.max(0.1)).min(2.0)
    }

    /// The latest diff at or before the current step (index into the events).
    pub fn diff_at(&self) -> Option<usize> {
        let end = (self.pos + 1).min(self.steps.len());
        self.steps[..end]
            .iter()
            .rev()
            .find(|s| s.kind == "diff")
            .map(|s| s.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(t: f64, v: Value) -> Value {
        let mut v = v;
        v["t"] = json!(t);
        v
    }

    fn sample() -> Vec<Value> {
        vec![
            ev(
                0.0,
                json!({"kind": "prompt", "text": "fix the bug\nplease"}),
            ),
            ev(
                1.0,
                json!({"kind": "diff", "id": "c1", "files": [{"path": "a.rs", "before": "a", "after": "b"}]}),
            ),
            ev(
                1.5,
                json!({"kind": "tool_start", "name": "Edit", "summary": "a.rs"}),
            ),
            ev(
                9.0,
                json!({"kind": "tool_end", "name": "Edit", "output": "ok", "is_error": false}),
            ),
            ev(
                9.5,
                json!({"kind": "usage", "input": 10, "output": 2, "cost_usd": 0.0012, "cost_estimated": true}),
            ),
            ev(
                10.0,
                json!({"kind": "diff", "id": "c2", "files": [{"path": "b.rs", "before": null, "after": "x"}]}),
            ),
            ev(12.0, json!({"kind": "end", "stop": "Done(Stop)"})),
        ]
    }

    #[test]
    fn stepping_seeking_and_the_diff_at_each_point() {
        let mut tl = Timeline::new(&sample());
        assert_eq!(tl.len(), 7);
        assert_eq!(tl.current().unwrap().title, "> fix the bug");
        assert_eq!(tl.diff_at(), None);
        assert!(!tl.back());
        assert!(tl.forward());
        assert_eq!(tl.diff_at(), Some(1));
        tl.go(4);
        assert_eq!(tl.current().unwrap().title, "¤ 10 in / 2 out ≈$0.0012");
        assert_eq!(tl.diff_at(), Some(1));
        assert!(tl.forward());
        assert_eq!(tl.diff_at(), Some(5));
        assert!(tl.back() && tl.back());
        assert_eq!(
            tl.diff_at(),
            Some(1),
            "stepping back shows the earlier diff"
        );
        tl.seek(9.2);
        assert_eq!(tl.pos(), 3);
        tl.seek(-1.0);
        assert_eq!(tl.pos(), 0);
        tl.go(99);
        assert!(tl.at_end() && !tl.forward());
        tl.go(2);
        assert!((tl.delay(1.0) - 2.0).abs() < 1e-9, "7.5 s gap is capped");
        assert!((tl.delay(4.0) - 1.875).abs() < 1e-9);
        assert!(Timeline::new(&[]).diff_at().is_none());
    }

    #[test]
    fn list_load_and_export_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let rec = kor_tools::record::Recorder::create(dir.path(), "s1").unwrap();
        for e in sample() {
            let mut e = e;
            e.as_object_mut().unwrap().remove("t");
            rec.write(e);
        }
        rec.output("> fix the bug\n");
        drop(rec);
        let l = list(dir.path());
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, "s1");
        assert_eq!(l[0].title, "fix the bug");
        assert_eq!((l[0].prompts, l[0].tools, l[0].events), (1, 1, 7));
        assert!(l[0].cost_estimated && (l[0].cost_usd - 0.0012).abs() < 1e-12);

        let r = load(dir.path(), "s1").unwrap();
        assert_eq!(r.header["version"], 2);
        assert_eq!(r.cast.len(), 1);
        let out = export(dir.path(), "s1", None).unwrap();
        let back = load(dir.path(), out.to_str().unwrap()).unwrap();
        assert_eq!(back.events, r.events);
        assert_eq!(back.cast, r.cast);
        assert!(load(dir.path(), "nope").is_err());
        assert!(load(dir.path(), "../x").is_err());
    }

    #[test]
    fn export_masks_known_key_formats_again() {
        let r = Recording {
            id: "x".into(),
            header: json!({"version": 2}),
            cast: vec![json!([0.1, "o", "postgres://u:hunter2pass@db/x"])],
            events: vec![
                json!({"kind": "text", "text": "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA"}),
            ],
        };
        let v = export_value(&r).to_string();
        assert!(
            !v.contains("hunter2pass") && !v.contains("AAAAAAAAAAAAAAAA"),
            "{v}"
        );
        assert!(v.contains(EXPORT_FORMAT));
    }
}
