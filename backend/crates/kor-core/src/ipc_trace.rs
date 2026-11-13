//! IPC trace (spec §17.2, ADR 0061): every call, result and event a surface exchanges with the core,
//! with timing, as JSON lines. Off unless `KOR_IPC_TRACE=<file>` is set (or `enable()` is called, as
//! `kor doctor --ipc-trace` does). Each line is validated against [`crate::ipc`]; violations are
//! written next to the message (`schema`), so a trace doubles as a contract check of a real run.
//!
//! `normalize()` reduces a trace to the core event stream (types, tool names, files, text), which is
//! what the CLI, the TUI and the desktop must agree on for the same scenario.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Environment variable naming the trace file.
pub const ENV: &str = "KOR_IPC_TRACE";

/// Which front end produced a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Cli,
    Tui,
    Desktop,
    /// `kor doctor --ipc-trace` driving the core directly.
    Core,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Tui => "tui",
            Self::Desktop => "desktop",
            Self::Core => "core",
        }
    }
}

/// What a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A bridge call and its arguments.
    Call,
    /// The value a call resolved with (`ms` = duration).
    Result,
    /// A call that failed (`payload` = the error text).
    Error,
    /// An event (`kor:event` items, `kor:<ns>/<x>`).
    Event,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Result => "result",
            Self::Error => "error",
            Self::Event => "event",
        }
    }
}

struct Writer {
    file: File,
    path: PathBuf,
    start: Instant,
}

static TRACE: Mutex<Option<Writer>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<Writer>> {
    TRACE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Start (or switch) tracing to `path`, appending.
///
/// # Errors
/// When the file cannot be created.
pub fn enable(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    *lock() = Some(Writer {
        file,
        path: path.to_owned(),
        start: Instant::now(),
    });
    Ok(())
}

/// Stop tracing; returns the file that was written.
pub fn disable() -> Option<PathBuf> {
    lock().take().map(|w| w.path)
}

/// Tracing to [`ENV`] when it is set and not empty; a file that cannot be opened is reported once on
/// stderr and tracing stays off (the surface itself keeps working).
pub fn init_from_env() {
    if let Some(p) = std::env::var_os(ENV).filter(|p| !p.is_empty())
        && let Err(e) = enable(Path::new(&p))
    {
        eprintln!("kor: {ENV}={}: {e}", Path::new(&p).display());
    }
}

/// The current trace file, if tracing.
pub fn path() -> Option<PathBuf> {
    lock().as_ref().map(|w| w.path.clone())
}

pub fn enabled() -> bool {
    lock().is_some()
}

/// Append one record. Cheap no-op when tracing is off.
pub fn record(surface: Surface, kind: Kind, name: &str, dur: Option<Duration>, payload: &Value) {
    let mut g = lock();
    let Some(w) = g.as_mut() else { return };
    let schema = match kind {
        Kind::Event => crate::ipc::check_event(name, payload),
        Kind::Result => crate::ipc::check_result(name, payload),
        Kind::Call => payload
            .as_array()
            .map(|a| crate::ipc::check_call(name, a))
            .unwrap_or_default(),
        Kind::Error => Vec::new(),
    };
    let mut line = json!({
        "t": round(w.start.elapsed()),
        "surface": surface.as_str(),
        "kind": kind.as_str(),
        "name": name,
        "payload": payload,
    });
    if let Some(d) = dur {
        line["ms"] = json!(round(d));
    }
    if !schema.is_empty() {
        line["schema"] = json!(schema);
    }
    // A failed write must not break the surface; the trace is a debugging aid.
    let _ = writeln!(w.file, "{line}").and_then(|()| w.file.flush());
}

/// `record(.., Kind::Event, ..)`.
pub fn event(surface: Surface, name: &str, payload: &Value) {
    record(surface, Kind::Event, name, None, payload);
}

fn round(d: Duration) -> f64 {
    (d.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

/// Records of a trace file (unparsable lines are skipped).
///
/// # Errors
/// When the file cannot be read.
pub fn read(path: &Path) -> std::io::Result<Vec<Value>> {
    let f = File::open(path)?;
    Ok(BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect())
}

/// `kor:event` item types every surface produces from the core (`kor_agent::remote::wire`). `notice` is
/// left out: notices describe the surface's environment (MCP not started in privacy mode…), not the task.
pub const CORE_EVENTS: &[&str] = &[
    "text",
    "g3:reasoning",
    "tool",
    "toolResult",
    "status",
    "confirm",
    "error",
];

/// The core event stream of `surface` in a trace, reduced to what must be identical on every surface:
/// event types in order, tool names, files of a `confirm`, the text of each answer. Ids, session ids
/// and timing are dropped. `status` ticks arrive once a second, so their number and position depend on
/// timing: one `status` item at the front says the stream had them. A `tool` card next to the `confirm`
/// for the same tool is the same card (the desktop opens it before asking, the CLI after).
pub fn normalize(records: &[Value], surface: Surface) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut status = false;
    for r in records {
        if r["surface"] != surface.as_str() || r["kind"] != "event" || r["name"] != "kor:event" {
            continue;
        }
        let p = &r["payload"];
        let Some(ty) = p["type"].as_str().filter(|t| CORE_EVENTS.contains(t)) else {
            continue;
        };
        let item = match ty {
            "status" => {
                status = true;
                continue;
            }
            "text" | "g3:reasoning" => {
                let delta = p["delta"].as_str().unwrap_or_default();
                if let Some(last) = out.last_mut().filter(|l| l["type"] == ty)
                    && let Some(s) = last["text"].as_str()
                {
                    last["text"] = json!(format!("{s}{delta}"));
                    continue;
                }
                json!({ "type": ty, "text": delta })
            }
            "tool" => json!({ "type": ty, "name": p["name"] }),
            "confirm" => {
                let files: Vec<&str> = p["files"]
                    .as_array()
                    .map(|f| f.iter().filter_map(|x| x["path"].as_str()).collect())
                    .unwrap_or_default();
                json!({ "type": ty, "name": p["name"], "files": files, "destructive": !p["destructive"].is_null() })
            }
            _ => json!({ "type": ty }),
        };
        let last_is = |t: &str| {
            out.last()
                .is_some_and(|l| l["type"] == t && l["name"] == item["name"])
        };
        if ty == "tool" && last_is("confirm") {
            continue;
        }
        // The desktop sends text in frames, so its approval card can be traced before the text that
        // came before it: the confirm goes where the tool starts.
        if ty == "tool"
            && let Some(i) = out.iter().rposition(|l| l["type"] != "text")
            && out[i]["type"] == "confirm"
            && out[i]["name"] == item["name"]
            && i + 1 < out.len()
        {
            let confirm = out.remove(i);
            out.push(confirm);
            continue;
        }
        if ty == "confirm" && last_is("tool") {
            out.pop();
        }
        out.push(item);
    }
    if status {
        out.insert(0, json!({ "type": "status" }));
    }
    // Trailing whitespace of an answer is presentation, not content.
    for v in &mut out {
        if let Some(s) = v["text"].as_str() {
            v["text"] = json!(s.trim_end());
        }
    }
    out
}

/// Schema violations recorded in a trace: `(name, errors)`.
pub fn violations(records: &[Value]) -> Vec<(String, Vec<String>)> {
    records
        .iter()
        .filter_map(|r| {
            let errs: Vec<String> = r["schema"]
                .as_array()?
                .iter()
                .filter_map(|e| e.as_str().map(str::to_owned))
                .collect();
            Some((r["name"].as_str().unwrap_or_default().to_owned(), errs))
        })
        .collect()
}

/// First difference between two normalized streams, as a readable line; `None` when equal.
pub fn diff(left: &[Value], right: &[Value]) -> Option<String> {
    let len = left.len().max(right.len());
    (0..len).find_map(|i| match (left.get(i), right.get(i)) {
        (Some(l), Some(r)) if l == r => None,
        (l, r) => Some(format!(
            "event {i}: {} ≠ {}",
            l.map_or("(none)".into(), Value::to_string),
            r.map_or("(none)".into(), Value::to_string)
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test touches the process-wide writer, so the steps run in order inside it.
    #[test]
    fn records_validates_and_normalizes() {
        let dir = std::env::temp_dir().join(format!("kor-ipc-trace-{}", std::process::id()));
        let file = dir.join("t.ndjson");
        let _ = std::fs::remove_file(&file);
        enable(&file).unwrap();
        assert!(enabled());
        let ev = |p: Value| event(Surface::Cli, "kor:event", &p);
        ev(
            json!({ "type": "status", "verb": "Közlüyor", "verbIndex": 0, "elapsedMs": 10, "tokens": 0, "sessionId": "s" }),
        );
        ev(
            json!({ "type": "status", "verb": "Dövüyor", "verbIndex": 1, "elapsedMs": 1010, "tokens": 5, "sessionId": "s" }),
        );
        ev(json!({ "type": "text", "delta": "Mer", "sessionId": "s" }));
        ev(json!({ "type": "text", "delta": "haba\n", "sessionId": "s" }));
        ev(
            json!({ "type": "tool", "id": "1", "name": "Write", "summary": "a.txt", "sessionId": "s" }),
        );
        ev(
            json!({ "type": "confirm", "id": "1", "name": "Write", "summary": "a.txt", "files": [{ "path": "a.txt", "before": null, "after": "x" }], "destructive": null, "sessionId": "s" }),
        );
        ev(
            json!({ "type": "tool", "id": "1", "name": "Write", "summary": "a.txt", "sessionId": "s" }),
        );
        ev(json!({ "type": "toolResult", "id": "1", "result": "ok", "sessionId": "s" }));
        ev(json!({ "type": "text", "delta": 7, "sessionId": "s" }));
        record(
            Surface::Desktop,
            Kind::Result,
            "cost.summary",
            Some(Duration::from_millis(3)),
            &json!({}),
        );
        assert_eq!(disable(), Some(file.clone()));
        record(
            Surface::Cli,
            Kind::Event,
            "kor:event",
            None,
            &json!({ "type": "done" }),
        );

        let recs = read(&file).unwrap();
        assert_eq!(recs.len(), 10, "nothing written after disable");
        assert_eq!(recs[9]["ms"], 3.0);
        let bad = violations(&recs);
        assert!(
            bad.iter()
                .any(|(n, e)| n == "kor:event" && e.iter().any(|m| m.contains("delta"))),
            "{bad:?}"
        );
        assert!(
            bad.iter().any(|(n, _)| n == "cost.summary"),
            "result shape checked: {bad:?}"
        );

        let n = normalize(&recs, Surface::Cli);
        assert_eq!(n[0], json!({ "type": "status" }));
        assert_eq!(n[1], json!({ "type": "text", "text": "Merhaba" }));
        // `tool` before and after the `confirm` of the same call fold into the confirm.
        assert_eq!(
            n[2],
            json!({ "type": "confirm", "name": "Write", "files": ["a.txt"], "destructive": false })
        );
        assert_eq!(n[3], json!({ "type": "toolResult" }));
        assert_eq!(
            n.iter().filter(|e| e["type"] == "status").count(),
            1,
            "status ticks fold into one"
        );
        assert!(normalize(&recs, Surface::Desktop).is_empty());
        assert_eq!(diff(&n, &n), None);
        assert!(diff(&n, &n[..2]).unwrap().starts_with("event 2"));
        assert_eq!(
            n.iter().filter(|e| e["type"] == "status").count(),
            1,
            "status ticks fold into one"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn confirm_traced_before_its_text_goes_where_the_tool_starts() {
        let rec = |p: Value| json!({ "surface": "desktop", "kind": "event", "name": "kor:event", "payload": p });
        let recs = [
            rec(json!({ "type": "status" })),
            rec(json!({ "type": "confirm", "name": "Write", "files": [{ "path": "plan.md" }] })),
            rec(json!({ "type": "text", "delta": "plan.md yazacağım." })),
            rec(json!({ "type": "tool", "name": "Write" })),
            rec(json!({ "type": "toolResult" })),
        ];
        let types: Vec<Value> = normalize(&recs, Surface::Desktop)
            .iter()
            .map(|e| e["type"].clone())
            .collect();
        assert_eq!(
            types,
            [
                json!("status"),
                json!("text"),
                json!("confirm"),
                json!("toolResult")
            ]
        );
    }
}
