//! Knowledge base speed against spec table 31.3.1 (p95): the measurement behind
//! `kor bench --memory` (ADR 0062) and the ignored `tests/bench.rs`. Synthetic records (≈45 words
//! from a 20k-word vocabulary, 20% of them from 8 very common words) in a fresh base in `dir`.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::embed::{VecIndex, quantize};
use crate::kb::{Knowledge, Record, Result};

/// xorshift64: deterministic, so every run measures the same data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const VOCAB: u64 = 20_000;
const COMMON: [&str; 8] = [
    "fix", "test", "error", "build", "api", "auth", "config", "update",
];

fn word(i: u64) -> String {
    // Pronounceable-ish unique words: w + base-26 letters.
    let mut s = String::from("w");
    let mut n = i;
    loop {
        s.push(char::from(b'a' + u8::try_from(n % 26).unwrap_or(0)));
        n /= 26;
        if n == 0 {
            break;
        }
    }
    s
}

fn words(rng: &mut Rng, n: usize) -> String {
    (0..n)
        .map(|_| {
            if rng.below(10) < 2 {
                COMMON[usize::try_from(rng.below(8)).unwrap_or(0)].to_owned()
            } else {
                word(rng.below(VOCAB))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn record(rng: &mut Rng) -> Record {
    Record {
        project: format!("/projects/p{}", rng.below(40)),
        task: words(rng, 6),
        summary: words(rng, 25),
        files: (0..3)
            .map(|_| format!("src/{}.rs", word(rng.below(VOCAB))))
            .collect(),
        errors: vec![words(rng, 6)],
        commands: vec![format!("cargo test -p {}", word(rng.below(VOCAB)))],
        ..Record::default()
    }
}

/// One measured operation.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Budget id (`kor_runtime::budgets`).
    pub id: &'static str,
    pub label: &'static str,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub target_ms: f64,
    pub samples: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub records: u64,
    pub fill: Duration,
    pub optimize: Duration,
    pub disk_bytes: u64,
    /// Bytes of the in-memory vector index of `vector_count` int8 vectors.
    pub vector_count: usize,
    pub rows: Vec<Row>,
}

fn row(id: &'static str, label: &'static str, mut v: Vec<Duration>, target_ms: f64) -> Row {
    v.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    // Nearest rank, like kor_runtime::budgets::Stat.
    let at = |q: usize| {
        v[(v.len() * q)
            .div_ceil(100)
            .saturating_sub(1)
            .min(v.len() - 1)]
    };
    Row {
        id,
        label,
        p50_ms: ms(at(50)),
        p95_ms: ms(at(95)),
        target_ms,
        samples: v.len(),
    }
}

fn timed(n: usize, mut f: impl FnMut() -> Result<()>) -> Result<Vec<Duration>> {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f()?;
            Ok(t.elapsed())
        })
        .collect()
}

/// Unit vector of `dims` pseudo-random components.
fn unit(rng: &mut Rng, dims: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dims)
        .map(|_| {
            #[allow(clippy::cast_precision_loss)]
            let x = (rng.next() >> 40) as f32 / (1u64 << 23) as f32;
            x - 1.0
        })
        .collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in &mut v {
        *x /= n;
    }
    v
}

/// Fill a base of `records` records in `dir` and measure every operation of table 31.3.1, plus
/// the vector top-50 over `vectors` int8 embeddings (0 skips it). `progress` gets a line per
/// phase.
///
/// # Errors
/// On a database error.
#[allow(clippy::too_many_lines)] // one linear measurement script
pub fn run(dir: &Path, records: u64, vectors: usize, progress: &dyn Fn(&str)) -> Result<Report> {
    let path = dir.join("knowledge.sqlite");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    progress("fill");
    let kb = Knowledge::open(&path)?;
    let t = Instant::now();
    // Bulk load in transactions of 1000 (how an import fills a base).
    let mut left = records;
    while left > 0 {
        let n = left.min(1000);
        kb.record_all((0..n).map(|_| record(&mut rng)).collect())?;
        left -= n;
    }
    kb.flush()?;
    let fill = t.elapsed();
    // A continuous fill never leaves the writer idle, so nothing was merged yet; ten years of
    // real use would have had idle merges. Do them now and say how long they took.
    progress("optimize");
    let t = Instant::now();
    kb.optimize()?;
    let optimize = t.elapsed();
    // Closing checkpoints the optimize's large WAL, which would otherwise still be copied back
    // while the write timings below run.
    drop(kb);
    let kb = Knowledge::open(&path)?;

    progress("read");
    let total = i64::try_from(kb.count()?).unwrap_or(i64::MAX).max(1);
    let read = timed(5000, || {
        let id = 1 + i64::try_from(rng.below(u64::try_from(total).unwrap_or(1))).unwrap_or(0);
        kb.get(id).map(|_| ())
    })?;

    progress("search");
    let fts = timed(500, || {
        let q = format!("{} {}", word(rng.below(VOCAB)), word(rng.below(VOCAB)));
        kb.search(&q, None, Some("/projects/p3"), 10).map(|_| ())
    })?;
    // Three different stop-word-only queries, so no single cached answer decides the number.
    let common_queries = ["fix auth error", "build test config", "update api error"];
    let mut i = 0;
    let common = timed(60, || {
        i += 1;
        kb.search(common_queries[i % 3], None, Some("/projects/p3"), 10)
            .map(|_| ())
    })?;
    // Task-start injection: a prompt-sized query, top 5.
    let inject = timed(200, || {
        let q = format!(
            "please {} the {} in {}",
            words(&mut rng, 4),
            words(&mut rng, 4),
            words(&mut rng, 4)
        );
        kb.search(&q, None, Some("/projects/p3"), 5).map(|_| ())
    })?;

    progress("write");
    let mut single = Vec::new();
    for _ in 0..300 {
        let r = record(&mut rng);
        let t = Instant::now();
        kb.record(r)?;
        kb.commit()?;
        single.push(t.elapsed());
    }
    let mut queued = Vec::new();
    for _ in 0..30 {
        let rs: Vec<Record> = (0..1000).map(|_| record(&mut rng)).collect();
        let t = Instant::now();
        for r in rs {
            kb.record(r)?;
        }
        kb.commit()?;
        queued.push(t.elapsed());
    }
    // A write is done when it is durable (§31.3.1: the user never waits for the query layer);
    // how long until the batch is also searchable is its own row.
    let (mut batch, mut indexed) = (Vec::new(), Vec::new());
    for _ in 0..30 {
        let rs: Vec<Record> = (0..1000).map(|_| record(&mut rng)).collect();
        let t = Instant::now();
        kb.record_all(rs)?;
        kb.commit()?;
        batch.push(t.elapsed());
        kb.flush()?;
        indexed.push(t.elapsed());
    }
    // Searches right after 60k more writes with no idle time (worst case before the idle merge).
    let dirty = timed(200, || {
        let q = format!("{} {}", word(rng.below(VOCAB)), word(rng.below(VOCAB)));
        kb.search(&q, None, Some("/projects/p3"), 10).map(|_| ())
    })?;
    drop(kb);

    progress("cold open");
    let cold = timed(5, || {
        let kb = Knowledge::open(&path)?;
        kb.search("fix", None, None, 5)?;
        drop(kb);
        Ok(())
    })?;

    let disk_bytes = std::fs::read_dir(dir)?
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    let mut rows = vec![
        row(
            "memory.write",
            "single write (queued + committed)",
            single,
            1.0,
        ),
        row(
            "memory.batch_queued",
            "1000 x record(), group commits",
            queued,
            50.0,
        ),
        row(
            "memory.batch",
            "batch 1000: record_all(), 1 commit",
            batch,
            50.0,
        ),
        row(
            "memory.indexed",
            "batch 1000 searchable (indexed)",
            indexed,
            100.0,
        ),
        row("memory.read", "key read", read, 0.1),
        row("memory.fts", "FTS search, 2 rare words, top 10", fts, 20.0),
        row(
            "memory.fts_common",
            "FTS search, 3 common words, top 10",
            common,
            20.0,
        ),
        row(
            "memory.fts_dirty",
            "FTS 2 rare words, +60k unmerged",
            dirty,
            20.0,
        ),
        row(
            "memory.inject",
            "context injection query, top 5",
            inject,
            100.0,
        ),
        row("memory.cold_open", "cold open + first search", cold, 300.0),
    ];
    if vectors > 0 {
        progress("vectors");
        let dims = 384;
        let mut ix = VecIndex::default();
        for id in 0..vectors {
            ix.push(
                i64::try_from(id).unwrap_or(0),
                &quantize(&unit(&mut rng, dims)),
            );
        }
        let mut times = Vec::new();
        for _ in 0..100 {
            let q = unit(&mut rng, dims);
            let t = Instant::now();
            let top = ix.top(&q, 50);
            times.push(t.elapsed());
            std::hint::black_box(top);
        }
        rows.push(row(
            "memory.vector",
            "vector top-50 (int8, exact)",
            times,
            30.0,
        ));
    }
    Ok(Report {
        records,
        fill,
        optimize,
        disk_bytes,
        vector_count: vectors,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_run_measures_every_row() {
        let dir = tempfile::tempdir().unwrap();
        let r = run(dir.path(), 2000, 1000, &|_| {}).unwrap();
        let ids: Vec<&str> = r.rows.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            [
                "memory.write",
                "memory.batch_queued",
                "memory.batch",
                "memory.indexed",
                "memory.read",
                "memory.fts",
                "memory.fts_common",
                "memory.fts_dirty",
                "memory.inject",
                "memory.cold_open",
                "memory.vector"
            ]
        );
        assert!(r.rows.iter().all(|r| r.samples > 0 && r.p50_ms <= r.p95_ms));
        assert!(r.disk_bytes > 0);
    }
}
