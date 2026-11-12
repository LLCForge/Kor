//! Observability export (§5.2, Observability > Settings): finished turns as OpenTelemetry traces
//! in a local file (OTLP/JSON, one line per turn, 14 days), the same JSON to an OTLP/HTTP
//! collector on this machine, and Prometheus counters on `127.0.0.1`. Tool summaries and
//! message text are never written: names, timings, outcomes and token counts only.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

/// Trace files are kept this many days.
const KEEP_DAYS: u64 = 14;

/// One tool call inside a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpan {
    pub name: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub ok: bool,
}

/// One finished turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnTrace {
    pub session: String,
    /// `provider/model`.
    pub model: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub ok: bool,
    pub tools: Vec<ToolSpan>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

fn hex_id(seed: &str, n: usize) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    h.finalize().iter().take(n).fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn attr(key: &str, v: Value) -> Value {
    let value = match v {
        Value::Bool(b) => json!({ "boolValue": b }),
        Value::Number(n) if n.is_u64() || n.is_i64() => json!({ "intValue": n.to_string() }),
        Value::Number(n) => json!({ "doubleValue": n }),
        other => {
            json!({ "stringValue": other.as_str().map_or_else(|| other.to_string(), str::to_owned) })
        }
    };
    json!({ "key": key, "value": value })
}

fn nanos(ms: u64) -> String {
    (u128::from(ms) * 1_000_000).to_string()
}

/// The turn as one OTLP/JSON `ExportTraceServiceRequest`: a root span for the turn and a child
/// span per tool call. Ids are derived from the session and start time (stable, no randomness).
pub fn otlp_json(t: &TurnTrace) -> Value {
    let trace_id = hex_id(&format!("{}:{}", t.session, t.start_ms), 16);
    let root_id = hex_id(&format!("{trace_id}:turn"), 8);
    let status = |ok: bool| json!({ "code": if ok { 1 } else { 2 } });
    let mut spans = vec![json!({
        "traceId": trace_id,
        "spanId": root_id,
        "name": "kor.turn",
        "kind": 1,
        "startTimeUnixNano": nanos(t.start_ms),
        "endTimeUnixNano": nanos(t.end_ms),
        "status": status(t.ok),
        "attributes": [
            attr("kor.session", json!(t.session)),
            attr("gen_ai.request.model", json!(t.model)),
            attr("gen_ai.usage.input_tokens", json!(t.input_tokens)),
            attr("gen_ai.usage.output_tokens", json!(t.output_tokens)),
            attr("kor.tool_calls", json!(t.tools.len())),
        ],
    })];
    for (i, s) in t.tools.iter().enumerate() {
        spans.push(json!({
            "traceId": trace_id,
            "spanId": hex_id(&format!("{trace_id}:{i}"), 8),
            "parentSpanId": root_id,
            "name": format!("kor.tool {}", s.name),
            "kind": 1,
            "startTimeUnixNano": nanos(s.start_ms),
            "endTimeUnixNano": nanos(s.end_ms),
            "status": status(s.ok),
            "attributes": [attr("kor.tool", json!(s.name))],
        }));
    }
    json!({
        "resourceSpans": [{
            "resource": { "attributes": [
                attr("service.name", json!("kor")),
                attr("service.version", json!(env!("CARGO_PKG_VERSION"))),
            ]},
            "scopeSpans": [{ "scope": { "name": "kor" }, "spans": spans }],
        }],
    })
}

/// Where trace files go: `<kor home>/traces`.
pub fn trace_dir() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("traces"))
}

/// Append the turn to `traces-DD.jsonl` (day of month) in `dir` and drop files older than
/// `KEEP_DAYS`. A file of the same day name last month is replaced, not appended to.
///
/// # Errors
/// When the folder or file cannot be written.
pub fn write_trace(dir: &Path, t: &TurnTrace) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    std::fs::create_dir_all(dir)?;
    let day = chrono::DateTime::from_timestamp_millis(i64::try_from(t.end_ms).unwrap_or(0))
        .map_or_else(
            || "00".into(),
            |d| d.with_timezone(&chrono::Local).format("%d").to_string(),
        );
    let path = dir.join(format!("traces-{day}.jsonl"));
    prune(dir);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{}", otlp_json(t))?;
    Ok(path)
}

fn prune(dir: &Path) {
    let keep = std::time::Duration::from_secs(KEEP_DAYS * 86_400);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > keep);
        let ours = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            n.starts_with("traces-")
                && std::path::Path::new(n)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("jsonl"))
        });
        if ours && old {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// An OTLP/HTTP endpoint is accepted only on this machine (no data leaves it).
///
/// # Errors
/// A malformed URL, another scheme, or a host other than loopback.
pub fn check_endpoint(url: &str) -> Result<(), String> {
    let u = url.trim();
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .ok_or("the OTLP endpoint must start with http:// or https://")?;
    let host = rest.split('/').next().unwrap_or_default();
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host.split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    if matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
        Ok(())
    } else {
        Err("only 127.0.0.1, localhost or [::1] are accepted".into())
    }
}

/// POST the turn to an OTLP/HTTP collector on this machine. Redirects are refused.
///
/// # Errors
/// A non-local endpoint, a network error or a non-2xx answer.
pub async fn post_otlp(endpoint: &str, t: &TurnTrace) -> Result<(), String> {
    check_endpoint(endpoint)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(5))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .post(endpoint.trim())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(otlp_json(t).to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_success() {
        Ok(())
    } else {
        Err(format!("the collector answered {}", r.status()))
    }
}

// ---------------------------------------------------------------------------------------------
// Prometheus
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Counters {
    turns: BTreeMap<(String, bool), u64>,
    tools: BTreeMap<(String, bool), u64>,
    tokens: BTreeMap<(String, &'static str), u64>,
    turn_ms_sum: u64,
}

static COUNTERS: Mutex<Option<Counters>> = Mutex::new(None);

/// Count a finished turn (always, so the endpoint has data the moment it is switched on).
pub fn count(t: &TurnTrace) {
    let mut g = COUNTERS.lock().unwrap_or_else(PoisonError::into_inner);
    let c = g.get_or_insert_with(Counters::default);
    *c.turns.entry((t.model.clone(), t.ok)).or_default() += 1;
    for s in &t.tools {
        *c.tools.entry((s.name.clone(), s.ok)).or_default() += 1;
    }
    *c.tokens.entry((t.model.clone(), "input")).or_default() += t.input_tokens;
    *c.tokens.entry((t.model.clone(), "output")).or_default() += t.output_tokens;
    c.turn_ms_sum = c
        .turn_ms_sum
        .saturating_add(t.end_ms.saturating_sub(t.start_ms));
}

fn label(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

/// The counters in Prometheus' text format.
pub fn render() -> String {
    let g = COUNTERS.lock().unwrap_or_else(PoisonError::into_inner);
    let empty = Counters::default();
    let c = g.as_ref().unwrap_or(&empty);
    let mut out = String::new();
    out.push_str("# HELP kor_turns_total Finished agent turns.\n# TYPE kor_turns_total counter\n");
    for ((m, ok), n) in &c.turns {
        let _ = writeln!(
            out,
            "kor_turns_total{{model=\"{}\",ok=\"{ok}\"}} {n}",
            label(m)
        );
    }
    out.push_str("# HELP kor_tool_calls_total Tool calls.\n# TYPE kor_tool_calls_total counter\n");
    for ((t, ok), n) in &c.tools {
        let _ = writeln!(
            out,
            "kor_tool_calls_total{{tool=\"{}\",ok=\"{ok}\"}} {n}",
            label(t)
        );
    }
    out.push_str("# HELP kor_tokens_total Model tokens.\n# TYPE kor_tokens_total counter\n");
    for ((m, kind), n) in &c.tokens {
        let _ = writeln!(
            out,
            "kor_tokens_total{{model=\"{}\",kind=\"{kind}\"}} {n}",
            label(m)
        );
    }
    out.push_str("# HELP kor_turn_seconds_total Time spent in turns.\n# TYPE kor_turn_seconds_total counter\n");
    #[allow(clippy::cast_precision_loss)] // milliseconds, shown in seconds
    let secs = c.turn_ms_sum as f64 / 1000.0;
    let _ = writeln!(out, "kor_turn_seconds_total {secs}");
    out
}

// ---------------------------------------------------------------------------------------------
// Exporter state (shared by the desktop's settings page and the turn loop)
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Status {
    otlp_sent: u64,
    otlp_error: Option<String>,
    trace_error: Option<String>,
    prometheus: Option<(u16, tokio_util::sync::CancellationToken)>,
    prometheus_error: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);

fn with_status<T>(f: impl FnOnce(&mut Status) -> T) -> T {
    let mut g = STATUS.lock().unwrap_or_else(PoisonError::into_inner);
    f(g.get_or_insert_with(Status::default))
}

/// Export a finished turn as the settings say: counted for Prometheus always, appended to the
/// trace file when `trace_file`, posted to `otlp` when it is set. Failures are kept for the
/// settings page, never raised: observability must not break a turn.
pub async fn export(t: &TurnTrace, trace_file: bool, otlp: &str) {
    count(t);
    if trace_file && let Some(dir) = trace_dir() {
        let e = write_trace(&dir, t).err().map(|e| e.to_string());
        with_status(|s| s.trace_error = e);
    }
    if !otlp.trim().is_empty() {
        let r = post_otlp(otlp, t).await;
        with_status(|s| match r {
            Ok(()) => {
                s.otlp_sent += 1;
                s.otlp_error = None;
            }
            Err(e) => s.otlp_error = Some(e),
        });
    }
}

/// Run the `/metrics` endpoint on `port`, or stop it with `None`. Idempotent: the same port
/// keeps the running server. Must be called inside a tokio runtime.
///
/// # Errors
/// When the port cannot be bound (also kept for [`status`]).
pub fn set_prometheus(port: Option<u16>) -> Result<(), String> {
    if with_status(|s| s.prometheus.as_ref().map(|p| p.0)) == port && port.is_some() {
        return Ok(());
    }
    if let Some((_, stop)) = with_status(|s| s.prometheus.take()) {
        stop.cancel();
    }
    let Some(port) = port else {
        with_status(|s| s.prometheus_error = None);
        return Ok(());
    };
    let bound = std::net::TcpListener::bind(("127.0.0.1", port))
        .and_then(|l| l.set_nonblocking(true).map(|()| l))
        .map_err(|e| format!("127.0.0.1:{port}: {e}"));
    let handle = tokio::runtime::Handle::try_current().map_err(|e| e.to_string());
    match bound.and_then(|l| handle.map(|h| (l, h))) {
        Ok((l, h)) => {
            let stop = tokio_util::sync::CancellationToken::new();
            let s2 = stop.clone();
            h.spawn(async move {
                if let Err(e) = serve_metrics(l, s2).await {
                    with_status(|s| s.prometheus_error = Some(e));
                }
            });
            with_status(|s| {
                s.prometheus = Some((port, stop));
                s.prometheus_error = None;
            });
            Ok(())
        }
        Err(e) => {
            with_status(|s| s.prometheus_error = Some(e.clone()));
            Err(e)
        }
    }
}

/// What the settings page shows (`exportStatus`).
pub fn status() -> Value {
    with_status(|s| {
        json!({
            "traceDir": trace_dir().map(|d| d.display().to_string()).unwrap_or_default(),
            "traceError": s.trace_error,
            "prometheusUrl": s.prometheus.as_ref().map(|(p, _)| format!("http://127.0.0.1:{p}/metrics")),
            "prometheusError": s.prometheus_error,
            "otlpSent": s.otlp_sent,
            "otlpError": s.otlp_error,
        })
    })
}

/// Serve `/metrics` on an already bound loopback listener until `stop` fires.
///
/// # Errors
/// When the listener cannot be handed to tokio.
pub async fn serve_metrics(
    listener: std::net::TcpListener,
    stop: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
    loop {
        let (mut sock, _) = tokio::select! {
            () = stop.cancelled() => return Ok(()),
            r = listener.accept() => match r { Ok(x) => x, Err(_) => continue },
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), sock.read(&mut buf))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let (status, body, ctype) =
                if req.starts_with("GET /metrics ") || req.starts_with("GET / ") {
                    ("200 OK", render(), "text/plain; version=0.0.4")
                } else {
                    ("404 Not Found", "not found\n".to_owned(), "text/plain")
                };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(body.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn() -> TurnTrace {
        TurnTrace {
            session: "s1".into(),
            model: "venice/m".into(),
            start_ms: 1_700_000_000_000,
            end_ms: 1_700_000_002_500,
            ok: true,
            tools: vec![ToolSpan {
                name: "Bash".into(),
                start_ms: 1_700_000_000_100,
                end_ms: 1_700_000_000_900,
                ok: false,
            }],
            input_tokens: 120,
            output_tokens: 30,
        }
    }

    #[test]
    fn a_turn_becomes_an_otlp_trace_with_a_child_span_per_tool() {
        let v = otlp_json(&turn());
        let spans = v["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0]["name"], "kor.turn");
        assert_eq!(spans[1]["parentSpanId"], spans[0]["spanId"]);
        assert_eq!(
            spans[1]["status"]["code"], 2,
            "a failed tool is an error span"
        );
        assert_eq!(spans[0]["traceId"].as_str().unwrap().len(), 32);
        assert_eq!(spans[0]["endTimeUnixNano"], "1700000002500000000");
        assert!(
            !v.to_string().contains("summary"),
            "no tool summaries or text"
        );
    }

    #[test]
    fn traces_go_to_a_daily_file_and_endpoints_stay_local() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_trace(dir.path(), &turn()).unwrap();
        write_trace(dir.path(), &turn()).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 2);
        assert!(
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("traces-")
        );
        assert!(check_endpoint("http://127.0.0.1:4318/v1/traces").is_ok());
        assert!(check_endpoint("http://localhost:4318/v1/traces").is_ok());
        assert!(check_endpoint("http://[::1]:4318/v1/traces").is_ok());
        assert!(check_endpoint("https://collector.example.com/v1/traces").is_err());
        assert!(check_endpoint("http://127.0.0.1.evil.com/").is_err());
        assert!(check_endpoint("ftp://127.0.0.1/").is_err());
    }

    #[tokio::test]
    async fn metrics_count_turns_tools_and_tokens() {
        count(&turn());
        let text = render();
        assert!(
            text.contains("kor_turns_total{model=\"venice/m\",ok=\"true\"}"),
            "{text}"
        );
        assert!(
            text.contains("kor_tool_calls_total{tool=\"Bash\",ok=\"false\"}"),
            "{text}"
        );
        assert!(
            text.contains("kor_tokens_total{model=\"venice/m\",kind=\"input\"}"),
            "{text}"
        );
        // Served on loopback through the same switch the settings page uses.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        set_prometheus(Some(port)).unwrap();
        set_prometheus(Some(port)).unwrap(); // idempotent: no second bind
        let url = status()["prometheusUrl"].as_str().unwrap().to_owned();
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert!(body.contains("kor_turns_total"));
        set_prometheus(None).unwrap();
        assert!(status()["prometheusUrl"].is_null());
    }
}
