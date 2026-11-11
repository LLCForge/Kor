//! Profile formats → [`Profile`] (collapsed stacks). Supported: collapsed text (inferno,
//! py-spy raw, async-profiler, Kor's cProfile wrapper), speedscope JSON (py-spy, dotnet-trace;
//! sampled and evented), Chrome `.cpuprofile` and `.heapprofile` (Node), pprof (Go, gzip or
//! plain protobuf), the Firefox profiler's processed format (samply, with its
//! `--unstable-presymbolicate` sidecar), `perf script` text and macOS `sample` call graphs.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;

use serde_json::Value;

use super::{Profile, Unit};

/// What a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Collapsed,
    Speedscope,
    ChromeCpu,
    ChromeHeap,
    Pprof,
    Firefox,
    PerfScript,
    MacSample,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collapsed => "collapsed",
            Self::Speedscope => "speedscope",
            Self::ChromeCpu => "cpuprofile",
            Self::ChromeHeap => "heapprofile",
            Self::Pprof => "pprof",
            Self::Firefox => "firefox",
            Self::PerfScript => "perf-script",
            Self::MacSample => "sample",
        }
    }
}

/// CPU and memory parts of one file (a pprof heap profile or a `.heapprofile` is memory).
#[derive(Debug, Default)]
pub struct Parsed {
    pub format: Option<Format>,
    pub cpu: Option<Profile>,
    pub memory: Option<Profile>,
}

/// Largest profile file read (decompressed).
const MAX_BYTES: u64 = 512 << 20;

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_BYTES)
        .read_to_end(&mut out)
        .map_err(|e| format!("gzip: {e}"))?;
    Ok(out)
}

/// Guess the format from the content.
pub fn detect(bytes: &[u8]) -> Option<Format> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    let head = &bytes[start..];
    if head.first() == Some(&b'{') {
        let v: Value = serde_json::from_slice(bytes).ok()?;
        return if v.get("nodes").is_some() && v.get("samples").is_some() {
            Some(Format::ChromeCpu)
        } else if v.get("head").is_some() {
            Some(Format::ChromeHeap)
        } else if v.get("profiles").is_some() && v.get("shared").is_some() {
            Some(Format::Speedscope)
        } else if v.get("threads").is_some() && v.get("meta").is_some() {
            Some(Format::Firefox)
        } else {
            None
        };
    }
    let text = std::str::from_utf8(bytes).ok();
    match text {
        None => Some(Format::Pprof),
        Some(t) if t.contains("Call graph:") => Some(Format::MacSample),
        Some(t) if t.lines().any(is_perf_frame) => Some(Format::PerfScript),
        Some(t) if t.lines().any(|l| collapsed_line(l).is_some()) => Some(Format::Collapsed),
        // A protobuf can be valid UTF-8 by chance; pprof starts with field 1 (sample_type).
        Some(_) if head.first() == Some(&0x0a) => Some(Format::Pprof),
        Some(_) => None,
    }
}

/// Read and parse a file; gzip is undone first. A Firefox profile picks up its
/// `*.syms.json` sidecar when one is next to it.
pub fn parse_file(path: &Path) -> Result<Parsed, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_BYTES {
        return Err(format!("{}: larger than {} MiB", path.display(), MAX_BYTES >> 20));
    }
    let mut bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        bytes = gunzip(&bytes)?;
    }
    let format = detect(&bytes).ok_or_else(|| format!("{}: unknown profile format", path.display()))?;
    let mut syms = None;
    if format == Format::Firefox {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = name.trim_end_matches(".gz").trim_end_matches(".json");
        for cand in [format!("{stem}.syms.json"), format!("{name}.syms.json")] {
            if let Ok(s) = std::fs::read(path.with_file_name(cand)) {
                syms = serde_json::from_slice::<Value>(&s).ok();
                break;
            }
        }
    }
    parse(&bytes, format, syms.as_ref())
}

/// Parse `bytes` known to be `format`.
pub fn parse(bytes: &[u8], format: Format, syms: Option<&Value>) -> Result<Parsed, String> {
    let json = || serde_json::from_slice::<Value>(bytes).map_err(|e| format!("{}: {e}", format.as_str()));
    let text = || String::from_utf8_lossy(bytes);
    let mut out = Parsed {
        format: Some(format),
        ..Parsed::default()
    };
    match format {
        Format::Collapsed => out.cpu = Some(collapsed(&text())),
        Format::PerfScript => out.cpu = Some(perf_script(&text())),
        Format::MacSample => out.cpu = Some(mac_sample(&text())),
        Format::Speedscope => out.cpu = Some(speedscope(&json()?)?),
        Format::ChromeCpu => out.cpu = Some(chrome_cpu(&json()?)?),
        Format::ChromeHeap => out.memory = Some(chrome_heap(&json()?)?),
        Format::Firefox => out.cpu = Some(firefox(&json()?, syms)?),
        Format::Pprof => {
            let raw = if bytes.starts_with(&[0x1f, 0x8b]) { gunzip(bytes)? } else { bytes.to_vec() };
            let (p, memory) = super::pprof::parse(&raw)?;
            if memory {
                out.memory = Some(p);
            } else {
                out.cpu = Some(p);
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Text formats
// ---------------------------------------------------------------------------------------------

fn collapsed_line(l: &str) -> Option<(&str, u64)> {
    let (stack, n) = l.trim_end().rsplit_once(' ')?;
    let n = n.parse().ok()?;
    (!stack.trim().is_empty() && !l.starts_with(char::is_whitespace)).then_some((stack, n))
}

/// `name (file:line)` → parts; anything else is a bare name.
fn split_frame(f: &str) -> (&str, Option<&str>, Option<u32>) {
    let f = f.trim();
    if let Some(inner) = f.strip_suffix(')')
        && let Some((name, loc)) = inner.rsplit_once(" (")
    {
        return match loc.rsplit_once(':') {
            Some((file, line)) if line.bytes().all(|b| b.is_ascii_digit()) && !line.is_empty() => {
                (name, Some(file), line.parse().ok())
            }
            _ => (name, Some(loc), None),
        };
    }
    (f, None, None)
}

/// Collapsed stacks (`a;b;c 12`). A `# unit=<unit>` line sets the unit.
pub fn collapsed(text: &str) -> Profile {
    let mut p = Profile::new(Unit::Samples);
    for l in text.lines() {
        if let Some(u) = l.strip_prefix("# unit=") {
            p.unit = match u.trim() {
                "microseconds" => Unit::Microseconds,
                "nanoseconds" => Unit::Nanoseconds,
                "bytes" => Unit::Bytes,
                _ => Unit::Samples,
            };
            continue;
        }
        let Some((stack, n)) = collapsed_line(l) else { continue };
        let s = stack
            .split(';')
            .map(|f| {
                let (name, file, line) = split_frame(f);
                p.frame(name, file, line)
            })
            .collect();
        p.add(s, n);
    }
    p
}

fn is_perf_frame(l: &str) -> bool {
    let t = l.trim_start();
    l.starts_with(['\t', ' ']) && t.split_whitespace().next().is_some_and(|a| a.len() >= 4 && a.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `perf script` output: a header line per sample, then frames leaf first
/// (`addr symbol+0xoff (dso)`), blank line between samples.
pub fn perf_script(text: &str) -> Profile {
    let mut p = Profile::new(Unit::Samples);
    let mut leaf_first: Vec<u32> = Vec::new();
    let flush = |p: &mut Profile, v: &mut Vec<u32>| {
        if !v.is_empty() {
            v.reverse();
            p.add(std::mem::take(v), 1);
        }
    };
    for l in text.lines() {
        if is_perf_frame(l) {
            let mut parts = l.trim_start().splitn(2, char::is_whitespace);
            let _addr = parts.next();
            let rest = parts.next().unwrap_or("").trim();
            let sym = rest.rsplit_once(" (").map_or(rest, |(s, _)| s).trim();
            let sym = sym.rsplit_once("+0x").map_or(sym, |(s, _)| s);
            let f = p.frame(sym, None, None);
            leaf_first.push(f);
        } else {
            flush(&mut p, &mut leaf_first);
        }
    }
    flush(&mut p, &mut leaf_first);
    p
}

/// macOS `sample` call graph: indented tree with inclusive counts; self = count − children.
pub fn mac_sample(text: &str) -> Profile {
    let mut p = Profile::new(Unit::Samples);
    // (indent, frame, count, children's count)
    let mut path: Vec<(usize, u32, u64, u64)> = Vec::new();
    let emit = |p: &mut Profile, path: &[(usize, u32, u64, u64)], i: usize| {
        let (_, _, count, kids) = path[i];
        let stack = path[..=i].iter().map(|e| e.1).collect();
        p.add(stack, count.saturating_sub(kids));
    };
    let mut inside = false;
    for l in text.lines() {
        if l.starts_with("Call graph:") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if l.trim().is_empty() || l.starts_with("Total number in stack") {
            break;
        }
        let indent = l.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
        if !l[..indent].chars().all(|c| " +!:|".contains(c)) {
            continue;
        }
        let rest = &l[indent..];
        let Some((n, body)) = rest.split_once(' ') else { continue };
        let Ok(count) = n.parse::<u64>() else { continue };
        let name = body.split("  (in ").next().unwrap_or(body).split("  ").next().unwrap_or(body).trim();
        let loc = body.split_whitespace().last().and_then(|t| {
            let (f, ln) = t.rsplit_once(':')?;
            Some((f, ln.parse::<u32>().ok()?))
        });
        while let Some(&(ind, ..)) = path.last() {
            if ind < indent {
                break;
            }
            emit(&mut p, &path, path.len() - 1);
            path.pop();
        }
        if let Some(parent) = path.last_mut() {
            parent.3 += count;
        }
        let f = p.frame(name, loc.map(|l| l.0), loc.map(|l| l.1));
        path.push((indent, f, count, 0));
    }
    while !path.is_empty() {
        emit(&mut p, &path, path.len() - 1);
        path.pop();
    }
    p
}

// ---------------------------------------------------------------------------------------------
// JSON formats
// ---------------------------------------------------------------------------------------------

fn u32_of(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// Speedscope weights in the profile's unit → (Kor unit, factor to it).
fn speedscope_unit(u: &str) -> (Unit, f64) {
    match u {
        "nanoseconds" => (Unit::Nanoseconds, 1.0),
        "microseconds" => (Unit::Microseconds, 1.0),
        "milliseconds" => (Unit::Microseconds, 1e3),
        "seconds" => (Unit::Microseconds, 1e6),
        "bytes" => (Unit::Bytes, 1.0),
        _ => (Unit::Samples, 1.0),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // non-negative, rounded
fn weight(x: f64) -> u64 {
    if x.is_finite() && x > 0.0 { x.round() as u64 } else { 0 }
}

/// Speedscope file: `shared.frames` and `profiles` (sampled or evented).
pub fn speedscope(v: &Value) -> Result<Profile, String> {
    let frames = v["shared"]["frames"].as_array().ok_or("speedscope: no shared.frames")?;
    let profiles = v["profiles"].as_array().ok_or("speedscope: no profiles")?;
    let unit = profiles.first().and_then(|p| p["unit"].as_str()).unwrap_or("none");
    let (unit, k) = speedscope_unit(unit);
    let mut p = Profile::new(unit);
    let ids: Vec<u32> = frames
        .iter()
        .map(|f| p.frame(f["name"].as_str().unwrap_or(""), f["file"].as_str(), f["line"].as_u64().and_then(|l| u32::try_from(l).ok())))
        .collect();
    let map = |i: &Value| u32_of(i).and_then(|i| ids.get(i as usize).copied());
    for prof in profiles {
        match prof["type"].as_str() {
            Some("sampled") => {
                let samples = prof["samples"].as_array().ok_or("speedscope: no samples")?;
                let weights = prof["weights"].as_array();
                for (i, s) in samples.iter().enumerate() {
                    let stack: Vec<u32> = s.as_array().map(|a| a.iter().filter_map(map).collect()).unwrap_or_default();
                    let w = weights.and_then(|w| w.get(i)).and_then(Value::as_f64).unwrap_or(1.0);
                    p.add(stack, weight(w * k));
                }
            }
            Some("evented") => {
                let mut open: Vec<u32> = Vec::new();
                let mut last = prof["startValue"].as_f64().unwrap_or(0.0);
                for e in prof["events"].as_array().ok_or("speedscope: no events")? {
                    let at = e["at"].as_f64().unwrap_or(last);
                    if !open.is_empty() {
                        p.add(open.clone(), weight((at - last) * k));
                    }
                    last = at;
                    let Some(f) = map(&e["frame"]) else { continue };
                    match e["type"].as_str() {
                        Some("O") => open.push(f),
                        Some("C") => {
                            if let Some(pos) = open.iter().rposition(|&x| x == f) {
                                open.truncate(pos);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(p)
}

fn call_frame(p: &mut Profile, cf: &Value) -> Option<u32> {
    let name = cf["functionName"].as_str().unwrap_or("");
    if name == "(root)" {
        return None;
    }
    let url = cf["url"].as_str().filter(|u| !u.is_empty());
    // Chrome line numbers are 0-based; -1 means unknown.
    let line = cf["lineNumber"].as_i64().filter(|l| *l >= 0).and_then(|l| u32::try_from(l + 1).ok());
    let name = if name.is_empty() {
        url.map_or("(anonymous)", |u| u.rsplit('/').next().unwrap_or(u))
    } else {
        name
    };
    Some(p.frame(name, url, line))
}

/// Chrome / Node `.cpuprofile`: node tree and sample ids (one sample each).
pub fn chrome_cpu(v: &Value) -> Result<Profile, String> {
    let nodes = v["nodes"].as_array().ok_or("cpuprofile: no nodes")?;
    let mut p = Profile::new(Unit::Samples);
    let mut parent: HashMap<u64, u64> = HashMap::new();
    let mut frame: HashMap<u64, Option<u32>> = HashMap::new();
    for n in nodes {
        let id = n["id"].as_u64().ok_or("cpuprofile: node without id")?;
        for c in n["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            parent.insert(c, id);
        }
        frame.insert(id, call_frame(&mut p, &n["callFrame"]));
    }
    let stack_of = |mut id: u64| {
        let mut s = Vec::new();
        let mut guard = 0;
        loop {
            if let Some(Some(f)) = frame.get(&id) {
                s.push(*f);
            }
            guard += 1;
            match parent.get(&id) {
                Some(&up) if guard < 4096 => id = up,
                _ => break,
            }
        }
        s.reverse();
        s
    };
    let samples = v["samples"].as_array().filter(|s| !s.is_empty());
    let mut counts: HashMap<u64, u64> = HashMap::new();
    match samples {
        Some(s) => s.iter().filter_map(Value::as_u64).for_each(|id| *counts.entry(id).or_default() += 1),
        None => nodes.iter().for_each(|n| {
            if let (Some(id), Some(h)) = (n["id"].as_u64(), n["hitCount"].as_u64()) {
                counts.insert(id, h);
            }
        }),
    }
    for (id, w) in counts {
        let s = stack_of(id);
        p.add(s, w);
    }
    Ok(p)
}

/// Node `.heapprofile` (sampling heap profiler): a call tree with `selfSize` bytes.
pub fn chrome_heap(v: &Value) -> Result<Profile, String> {
    fn walk(p: &mut Profile, n: &Value, stack: &mut Vec<u32>, depth: usize) {
        if depth > 4096 {
            return;
        }
        let f = call_frame(p, &n["callFrame"]);
        if let Some(f) = f {
            stack.push(f);
        }
        if let Some(sz) = n["selfSize"].as_u64() {
            p.add(stack.clone(), sz);
        }
        for c in n["children"].as_array().into_iter().flatten() {
            walk(p, c, stack, depth + 1);
        }
        if f.is_some() {
            stack.pop();
        }
    }
    let head = v.get("head").ok_or("heapprofile: no head")?;
    let mut p = Profile::new(Unit::Bytes);
    walk(&mut p, head, &mut Vec::new(), 0);
    Ok(p)
}

/// Strings of the Firefox processed format: per thread (`stringArray`/`stringTable`) or shared.
fn strings<'a>(profile: &'a Value, thread: &'a Value) -> Vec<&'a str> {
    [&thread["stringArray"], &thread["stringTable"], &profile["shared"]["stringArray"]]
        .into_iter()
        .find_map(Value::as_array)
        .map(|a| a.iter().map(|s| s.as_str().unwrap_or("")).collect())
        .unwrap_or_default()
}

/// The presymbolication sidecar: library debug name → sorted (rva, size, name, file, line).
type SymTable = HashMap<String, Vec<(u64, u64, String, Option<String>, Option<u32>)>>;

fn sym_table(syms: &Value) -> SymTable {
    let st: Vec<&str> = syms["string_table"].as_array().map(|a| a.iter().map(|s| s.as_str().unwrap_or("")).collect()).unwrap_or_default();
    let text = |v: &Value| v.as_str().map(str::to_owned).or_else(|| v.as_u64().and_then(|i| st.get(usize::try_from(i).ok()?)).map(|s| (*s).to_owned()));
    let mut out = SymTable::new();
    for lib in syms["data"].as_array().into_iter().flatten() {
        let Some(name) = lib["debug_name"].as_str() else { continue };
        let mut v: Vec<_> = lib["symbol_table"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let rva = s["rva"].as_u64()?;
                let size = s["size"].as_u64().unwrap_or(0);
                let sym = text(&s["symbol"])?;
                let inner = s["frames"].as_array().and_then(|f| f.last());
                let file = inner.and_then(|f| text(&f["file"]));
                let line = inner.and_then(|f| f["line"].as_u64()).and_then(|l| u32::try_from(l).ok());
                Some((rva, size, sym, file, line))
            })
            .collect();
        v.sort_by_key(|e| e.0);
        out.insert(name.to_owned(), v);
    }
    out
}

/// Firefox profiler processed format (samply). Unsymbolicated names (`0x…`) are resolved with
/// the sidecar when given.
pub fn firefox(v: &Value, syms: Option<&Value>) -> Result<Profile, String> {
    let threads = v["threads"].as_array().ok_or("firefox: no threads")?;
    let libs = v["libs"].as_array();
    let table = syms.map(sym_table).unwrap_or_default();
    let mut p = Profile::new(Unit::Samples);
    for t in threads {
        let s = strings(v, t);
        let col = |table: &str, c: &str| t[table][c].as_array().cloned().unwrap_or_default();
        let (st_frame, st_prefix) = (col("stackTable", "frame"), col("stackTable", "prefix"));
        let (fr_func, fr_addr, fr_line) = (col("frameTable", "func"), col("frameTable", "address"), col("frameTable", "line"));
        let (fn_name, fn_file, fn_line, fn_res) = (col("funcTable", "name"), col("funcTable", "fileName"), col("funcTable", "lineNumber"), col("funcTable", "resource"));
        let res_lib = col("resourceTable", "lib");
        let str_at = |v: Option<&Value>| v.and_then(Value::as_u64).and_then(|i| s.get(usize::try_from(i).ok()?)).copied();
        // Frame index → Kor frame.
        let mut memo: HashMap<usize, u32> = HashMap::new();
        let mut frame_of = |p: &mut Profile, fi: usize| -> Option<u32> {
            if let Some(&f) = memo.get(&fi) {
                return Some(f);
            }
            let func = usize::try_from(fr_func.get(fi)?.as_u64()?).ok()?;
            let mut name = str_at(fn_name.get(func)).unwrap_or("").to_owned();
            let mut file = str_at(fn_file.get(func)).map(str::to_owned);
            let mut line = fn_line.get(func).and_then(Value::as_u64).or_else(|| fr_line.get(fi).and_then(Value::as_u64)).and_then(|l| u32::try_from(l).ok());
            if name.starts_with("0x") || name.is_empty() {
                let lib = fn_res.get(func).and_then(Value::as_u64).and_then(|r| res_lib.get(usize::try_from(r).ok()?)).and_then(Value::as_u64);
                let dbg = lib.and_then(|l| libs?.get(usize::try_from(l).ok()?)?["debugName"].as_str());
                let addr = fr_addr.get(fi).and_then(Value::as_u64);
                if let (Some(dbg), Some(addr)) = (dbg, addr)
                    && let Some(list) = table.get(dbg)
                {
                    let i = list.partition_point(|e| e.0 <= addr);
                    if let Some(e) = i.checked_sub(1).and_then(|i| list.get(i))
                        && (e.1 == 0 || addr < e.0 + e.1)
                    {
                        name.clone_from(&e.2);
                        if e.3.is_some() {
                            file.clone_from(&e.3);
                            line = e.4;
                        }
                    }
                }
            }
            let f = p.frame(&name, file.as_deref(), line);
            memo.insert(fi, f);
            Some(f)
        };
        let samples = &t["samples"];
        let stacks = samples["stack"].as_array().cloned().unwrap_or_default();
        let weights = samples["weight"].as_array();
        let mut cache: HashMap<usize, Vec<u32>> = HashMap::new();
        for (i, st) in stacks.iter().enumerate() {
            let Some(mut si) = st.as_u64().and_then(|x| usize::try_from(x).ok()) else { continue };
            let leaf = si;
            let stack = if let Some(s) = cache.get(&leaf) {
                s.clone()
            } else {
                let mut s = Vec::new();
                for _ in 0..4096 {
                    if let Some(f) = st_frame.get(si).and_then(Value::as_u64).and_then(|f| frame_of(&mut p, usize::try_from(f).ok()?)) {
                        s.push(f);
                    }
                    match st_prefix.get(si).and_then(Value::as_u64) {
                        Some(up) => si = usize::try_from(up).unwrap_or(usize::MAX),
                        None => break,
                    }
                }
                s.reverse();
                cache.insert(leaf, s.clone());
                s
            };
            let w = weights.and_then(|w| w.get(i)).and_then(Value::as_f64).unwrap_or(1.0);
            p.add(stack, weight(w));
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn top_name(p: &Profile) -> (&str, u64) {
        let t = &p.top(1)[0];
        (p.frames[t.frame as usize].name.as_str(), t.self_weight)
    }

    #[test]
    fn collapsed_and_detection() {
        let text = "main (src/main.rs:3);fib (src/main.rs:10) 8\nmain (src/main.rs:3) 2\n";
        assert_eq!(detect(text.as_bytes()), Some(Format::Collapsed));
        let p = collapsed(text);
        assert_eq!(p.total(), 10);
        assert_eq!(top_name(&p), ("fib", 8));
        let f = &p.frames[p.top(1)[0].frame as usize];
        assert_eq!((f.file.as_deref(), f.line), (Some("src/main.rs"), Some(10)));
        let c = collapsed("# unit=microseconds\nf (C:\\x\\y.py:4) 5\n");
        assert_eq!(c.unit, Unit::Microseconds);
        assert_eq!(c.frames[0].file.as_deref(), Some("C:\\x\\y.py"));
    }

    #[test]
    fn perf_script_text() {
        let t = include_str!("fixtures/perf-script.txt");
        assert_eq!(detect(t.as_bytes()), Some(Format::PerfScript));
        let p = perf_script(t);
        assert_eq!(p.total(), 3);
        assert_eq!(top_name(&p), ("hot::fib", 2));
        let top = p.top(3);
        let main = top.iter().find(|s| p.frames[s.frame as usize].name == "main").unwrap();
        assert_eq!(main.total_weight, 3);
    }

    #[test]
    fn mac_sample_text() {
        let t = include_str!("fixtures/sample.txt");
        assert_eq!(detect(t.as_bytes()), Some(Format::MacSample));
        let p = mac_sample(t);
        assert_eq!(p.total(), 100);
        assert_eq!(top_name(&p), ("fib", 70));
        let f = &p.frames[p.top(1)[0].frame as usize];
        assert_eq!((f.file.as_deref(), f.line), (Some("hot.c"), Some(4)));
    }

    #[test]
    fn speedscope_sampled_and_evented() {
        let v: Value = serde_json::from_str(include_str!("fixtures/profile.speedscope.json")).unwrap();
        assert_eq!(detect(include_bytes!("fixtures/profile.speedscope.json")), Some(Format::Speedscope));
        let p = speedscope(&v).unwrap();
        assert_eq!(p.unit, Unit::Microseconds);
        // sampled: 3 × 10 ms in fib, 1 × 10 ms in main; evented: main 0..5, fib 1..4 (ms)
        assert_eq!(p.total(), 40_000 + 5_000);
        assert_eq!(top_name(&p), ("fib", 33_000));
        let ev = json!({"shared":{"frames":[{"name":"a"},{"name":"b"}]},"profiles":[{"type":"evented","unit":"none","startValue":0,"endValue":10,
            "events":[{"type":"O","frame":0,"at":0},{"type":"O","frame":1,"at":2},{"type":"C","frame":1,"at":6},{"type":"C","frame":0,"at":10}]}]});
        let p = speedscope(&ev).unwrap();
        assert_eq!((p.total(), top_name(&p)), (10, ("a", 6)));
    }

    #[test]
    fn chrome_cpu_and_heap() {
        let b = include_bytes!("fixtures/node.cpuprofile");
        assert_eq!(detect(b), Some(Format::ChromeCpu));
        let p = chrome_cpu(&serde_json::from_slice(b).unwrap()).unwrap();
        assert_eq!(p.total(), 6);
        assert_eq!(top_name(&p), ("fib", 4));
        let f = &p.frames[p.top(1)[0].frame as usize];
        assert_eq!((f.file.as_deref(), f.line), (Some("file:///C:/proj/a.js"), Some(1)));
        let h = include_bytes!("fixtures/node.heapprofile");
        let parsed = parse(h, detect(h).unwrap(), None).unwrap();
        let m = parsed.memory.unwrap();
        assert_eq!((m.unit, m.total()), (Unit::Bytes, 98_304 + 4_096));
        assert_eq!(top_name(&m), ("grow", 98_304));
    }

    #[test]
    fn firefox_with_sidecar() {
        let v: Value = serde_json::from_str(include_str!("fixtures/samply.json")).unwrap();
        let syms: Value = serde_json::from_str(include_str!("fixtures/samply.syms.json")).unwrap();
        assert_eq!(detect(include_bytes!("fixtures/samply.json")), Some(Format::Firefox));
        let p = firefox(&v, Some(&syms)).unwrap();
        assert_eq!(p.total(), 4);
        assert_eq!(top_name(&p), ("hot::fib", 3));
        let f = &p.frames[p.top(1)[0].frame as usize];
        assert_eq!((f.file.as_deref(), f.line), (Some("src/lib.rs"), Some(5)));
        // Without the sidecar the addresses stay as names.
        let raw = firefox(&v, None).unwrap();
        assert!(raw.frames.iter().any(|f| f.name.starts_with("0x")));
    }

    #[test]
    fn pprof_cpu_and_heap_from_go() {
        // Real `go test -cpuprofile/-memprofile` output (Go 1.2x, gzip). Reference numbers from
        // `go tool pprof -top`: 5 samples (50 ms); hot.Alloc 92.26% of alloc_space.
        let cpu = parse(include_bytes!("fixtures/go-cpu.pprof"), Format::Pprof, None).unwrap().cpu.unwrap();
        assert_eq!((cpu.unit, cpu.total()), (Unit::Samples, 5));
        let fib = cpu.top(50).into_iter().find(|s| cpu.frames[s.frame as usize].name == "hot.Fib").unwrap();
        assert_eq!(fib.self_weight, 1);
        assert!(cpu.frames[fib.frame as usize].file.as_deref().unwrap().ends_with("hot.go"));
        let mem = parse(include_bytes!("fixtures/go-mem.pprof"), Format::Pprof, None).unwrap().memory.unwrap();
        assert_eq!(mem.unit, Unit::Bytes);
        let (name, w) = top_name(&mem);
        assert_eq!(name, "hot.Alloc");
        let share = w * 10_000 / mem.total();
        assert!((9_200..=9_250).contains(&share), "{share}");
        assert!(mem.count.unwrap() > 0);
        assert_eq!(detect(&gunzip(include_bytes!("fixtures/go-cpu.pprof")).unwrap()), Some(Format::Pprof));
    }
}
