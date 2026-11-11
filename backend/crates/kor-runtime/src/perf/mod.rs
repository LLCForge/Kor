//! `kor perf` (§15.8, ADR 0049): run a command, the project's tests or an HTTP endpoint under the
//! language's profiler, turn the profile into collapsed stacks, draw a flamegraph, map hot frames
//! to project-graph nodes by `file:line`, and compare a run with an earlier one. Numbers come only
//! from measurements (the profiler's samples, Kor's own wall clock, the load generator).
//!
//! - [`parse`]: profile formats → [`Profile`] (collapsed stacks with `file:line` frames).
//! - [`flame`]: [`Profile`] → flamegraph SVG (no script; safe to show as an image).
//! - [`load`]: the small HTTP load generator (network guard, loopback or allowed hosts only).
//! - [`run`]: profiler selection, the sandbox / environment wrapper, stored reports.

// Every public function returns the localized error text itself; must_use / errors-doc and
// short parser-local names are noise here.
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::many_single_char_names,
    clippy::similar_names
)]

pub mod flame;
pub mod load;
pub mod parse;
mod pprof;
pub mod run;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use run::{
    Options, Plan, Report, compare, execute, list, load_report, optimize_prompt, plan, report_file,
};

/// One stack frame. `line` is the function's first line when the format gives it, else the
/// sampled line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub name: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// What a profile's weights count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    #[default]
    Samples,
    Nanoseconds,
    Microseconds,
    Bytes,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Samples => "samples",
            Self::Nanoseconds => "ns",
            Self::Microseconds => "µs",
            Self::Bytes => "bytes",
        }
    }
}

/// Collapsed stacks: root-first frame indices → summed weight. Frames are interned by
/// (name, file); the smallest line seen is kept (a function's start).
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub frames: Vec<Frame>,
    index: HashMap<(String, Option<String>), u32>,
    pub stacks: HashMap<Vec<u32>, u64>,
    pub unit: Unit,
    /// Number of allocations, when the format counts them (Go `alloc_objects`).
    pub count: Option<u64>,
}

/// Self and total weight of one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnStat {
    pub frame: u32,
    pub self_weight: u64,
    pub total_weight: u64,
}

impl Profile {
    pub fn new(unit: Unit) -> Self {
        Self {
            unit,
            ..Self::default()
        }
    }

    /// Intern a frame and return its index.
    pub fn frame(&mut self, name: &str, file: Option<&str>, line: Option<u32>) -> u32 {
        let name = if name.is_empty() { "(anonymous)" } else { name };
        let file = file.filter(|f| !f.is_empty()).map(str::to_owned);
        let key = (name.to_owned(), file);
        if let Some(&i) = self.index.get(&key) {
            let f = &mut self.frames[i as usize];
            if let Some(l) = line
                && f.line.is_none_or(|old| l < old)
            {
                f.line = Some(l);
            }
            return i;
        }
        let i = u32::try_from(self.frames.len()).unwrap_or(u32::MAX);
        self.frames.push(Frame {
            name: key.0.clone(),
            file: key.1.clone(),
            line,
        });
        self.index.insert(key, i);
        i
    }

    /// Add `weight` to a root-first stack (empty stacks and zero weights are ignored).
    pub fn add(&mut self, stack: Vec<u32>, weight: u64) {
        if !stack.is_empty() && weight > 0 {
            *self.stacks.entry(stack).or_default() += weight;
        }
    }

    /// Add every stack of `other` (same unit expected; the caller decides).
    pub fn merge(&mut self, other: &Self) {
        let map: Vec<u32> = other
            .frames
            .iter()
            .map(|f| self.frame(&f.name, f.file.as_deref(), f.line))
            .collect();
        for (s, w) in &other.stacks {
            self.add(s.iter().map(|&i| map[i as usize]).collect(), *w);
        }
        if let Some(c) = other.count {
            self.count = Some(self.count.unwrap_or(0) + c);
        }
    }

    pub fn total(&self) -> u64 {
        self.stacks.values().sum()
    }

    pub fn is_empty(&self) -> bool {
        self.stacks.is_empty()
    }

    /// Frames by self weight (ties: total weight), at most `n`.
    pub fn top(&self, n: usize) -> Vec<FnStat> {
        let mut own = vec![0u64; self.frames.len()];
        let mut all = vec![0u64; self.frames.len()];
        let mut seen = Vec::new();
        for (stack, &w) in &self.stacks {
            if let Some(&leaf) = stack.last() {
                own[leaf as usize] += w;
            }
            seen.clear();
            seen.extend_from_slice(stack);
            seen.sort_unstable();
            seen.dedup(); // recursion counts once per stack
            for &f in &seen {
                all[f as usize] += w;
            }
        }
        let mut v: Vec<FnStat> = (0..self.frames.len())
            .filter(|&i| all[i] > 0)
            .map(|i| FnStat {
                frame: u32::try_from(i).unwrap_or(u32::MAX),
                self_weight: own[i],
                total_weight: all[i],
            })
            .collect();
        v.sort_by(|a, b| {
            (
                b.self_weight,
                b.total_weight,
                &self.frames[a.frame as usize].name,
            )
                .cmp(&(
                    a.self_weight,
                    a.total_weight,
                    &self.frames[b.frame as usize].name,
                ))
        });
        v.truncate(n);
        v
    }

    /// `name (file:line);… weight` lines, sorted (the common intermediate, readable by
    /// [`parse::collapsed`] and inferno-style tools).
    pub fn to_collapsed(&self) -> String {
        let mut lines: Vec<String> = self
            .stacks
            .iter()
            .map(|(s, w)| {
                let frames: Vec<String> = s
                    .iter()
                    .map(|&i| frame_text(&self.frames[i as usize]))
                    .collect();
                format!("{} {w}", frames.join(";"))
            })
            .collect();
        lines.sort();
        let mut out = String::new();
        if self.unit != Unit::Samples {
            let _ = writeln!(out, "# unit={}", unit_key(self.unit));
        }
        for l in lines {
            out.push_str(&l);
            out.push('\n');
        }
        out
    }
}

fn unit_key(u: Unit) -> &'static str {
    match u {
        Unit::Samples => "samples",
        Unit::Nanoseconds => "nanoseconds",
        Unit::Microseconds => "microseconds",
        Unit::Bytes => "bytes",
    }
}

/// `name (file:line)`; `;` would split a collapsed stack, so it is replaced.
pub fn frame_text(f: &Frame) -> String {
    let name = f.name.replace(';', ":");
    match (&f.file, f.line) {
        (Some(file), Some(l)) => format!("{name} ({}:{l})", file.replace(';', ":")),
        (Some(file), None) => format!("{name} ({})", file.replace(';', ":")),
        _ => name,
    }
}

/// `file` relative to `root` with `/` separators, when it lies inside it. Accepts `file://`
/// URLs (Node) and either separator; drive letters compare case-insensitively.
pub fn relative_to(root: &Path, file: &str) -> Option<String> {
    let f = file.strip_prefix("file://").unwrap_or(file);
    // file:///C:/x → /C:/x
    let f = if f.len() > 3 && f.starts_with('/') && f.as_bytes()[2] == b':' {
        &f[1..]
    } else {
        f
    };
    let f = f.replace('\\', "/");
    let r = root.to_string_lossy().replace('\\', "/");
    let r = r.trim_end_matches('/');
    if r.is_empty() {
        return None;
    }
    let windows_like = r.as_bytes().get(1) == Some(&b':');
    let head = f.get(..r.len())?;
    let same = if windows_like {
        head.eq_ignore_ascii_case(r)
    } else {
        head == r
    };
    if same && f.as_bytes().get(r.len()) == Some(&b'/') {
        return Some(f[r.len() + 1..].to_owned());
    }
    // Already relative (e.g. `src/main.rs`) and present in the project.
    let rel = f.trim_start_matches("./");
    (!rel.starts_with('/')
        && !rel.contains(':')
        && !rel.starts_with("..")
        && root.join(rel).is_file())
    .then(|| rel.to_owned())
}

/// Function name without module path, receiver, generics or Rust's legacy hash suffix:
/// `kor::perf::top::h0123456789abcdef` → `top`, `main.(*Server).handle` → `handle`.
pub fn short_name(name: &str) -> &str {
    let mut n = name.trim();
    if let Some(i) = n.rfind("::h")
        && n.len() - i == 19
        && n[i + 3..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        n = &n[..i];
    }
    // Drop a trailing generic / argument list.
    for (open, close) in [('<', '>'), ('(', ')')] {
        if n.ends_with(close)
            && let Some(i) = n.find(open)
            && i > 0
        {
            n = &n[..i];
        }
    }
    let cut = [
        n.rfind("::").map(|i| i + 2),
        n.rfind('.').map(|i| i + 1),
        n.rfind('/').map(|i| i + 1),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let s = &n[cut..];
    if s.is_empty() { n } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_interns_and_ranks() {
        let mut p = Profile::new(Unit::Samples);
        let main = p.frame("main", Some("src/main.rs"), Some(10));
        let hot = p.frame("hot", Some("src/main.rs"), Some(30));
        let again = p.frame("hot", Some("src/main.rs"), Some(25));
        assert_eq!(hot, again);
        assert_eq!(p.frames[hot as usize].line, Some(25));
        p.add(vec![main, hot], 7);
        p.add(vec![main, hot, hot], 1);
        p.add(vec![main], 2);
        p.add(vec![], 9);
        assert_eq!(p.total(), 10);
        let top = p.top(5);
        assert_eq!(
            (top[0].frame, top[0].self_weight, top[0].total_weight),
            (hot, 8, 8)
        );
        assert_eq!(
            (top[1].frame, top[1].self_weight, top[1].total_weight),
            (main, 2, 10)
        );
        let c = p.to_collapsed();
        assert!(
            c.contains("main (src/main.rs:10);hot (src/main.rs:25) 7\n"),
            "{c}"
        );
        let back = parse::collapsed(&c);
        assert_eq!(back.total(), 10);
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(short_name("kor::perf::top::h0123456789abcdef"), "top");
        assert_eq!(short_name("main.(*Server).handle"), "handle");
        assert_eq!(short_name("Vec<T>::push"), "push");
        assert_eq!(
            short_name("core::ptr::drop_in_place<alloc::vec::Vec<u8>>"),
            "drop_in_place"
        );
        assert_eq!(short_name("fib"), "fib");
        let root = Path::new("C:\\Proj");
        assert_eq!(
            relative_to(root, "file:///c:/Proj/src/a.js").as_deref(),
            Some("src/a.js")
        );
        assert_eq!(
            relative_to(root, "C:\\Proj\\x\\y.py").as_deref(),
            Some("x/y.py")
        );
        assert_eq!(relative_to(root, "C:\\Other\\y.py"), None);
        assert_eq!(
            relative_to(Path::new("/home/u/p"), "/home/u/p/src/m.rs").as_deref(),
            Some("src/m.rs")
        );
        assert_eq!(relative_to(Path::new("/home/u/p"), "/home/u/px/m.rs"), None);
    }
}
