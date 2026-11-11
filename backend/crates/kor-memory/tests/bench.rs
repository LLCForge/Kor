//! Knowledge base speed against spec table 31.3.1 (p95). Ignored by default:
//! `cargo test -p kor-memory --release --test bench -- --ignored --nocapture`
//! (`KOR_BENCH_RECORDS=1000000` for the full 1M run; default 100000). The same measurement is
//! `kor bench --memory` (ADR 0062).

#[test]
#[ignore = "benchmark; run with --release --ignored"]
#[allow(clippy::cast_precision_loss)] // display only
fn knowledge_base_p95() {
    let n: u64 = std::env::var("KOR_BENCH_RECORDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let vectors: usize = std::env::var("KOR_BENCH_VECTORS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let sample = "fix wqzab error wkkd build wplm auth wzzx config update wqq wrrt wbbn fix test api                   wmmo wnnp waab wcc wdd wee wff wgg whh wii wjj";
    let t = std::time::Instant::now();
    for _ in 0..10_000 {
        std::hint::black_box(kor_memory::mask(std::hint::black_box(sample)));
    }
    println!(
        "mask(): {:.1} µs per 25-word field",
        t.elapsed().as_secs_f64() * 100.0
    );
    let dir = tempfile::tempdir().unwrap();
    let r =
        kor_memory::bench::run(dir.path(), n, vectors, &|phase| eprintln!("… {phase}")).unwrap();
    println!(
        "\nknowledge base: {} records, fill {:.1} s ({:.0} rec/s), optimize {:.1} s, disk {} MB",
        r.records,
        r.fill.as_secs_f64(),
        r.records as f64 / r.fill.as_secs_f64(),
        r.optimize.as_secs_f64(),
        r.disk_bytes / 1_000_000
    );
    println!(
        "| operation                          |           p95 |           p50 | target (p95) |      |"
    );
    for row in &r.rows {
        let ok = if row.p95_ms < row.target_ms {
            "ok"
        } else {
            "OVER"
        };
        println!(
            "| {:<34} | {:>10.3} ms | {:>10.3} ms | < {:>7} ms | {ok:<4} |",
            row.label, row.p95_ms, row.p50_ms, row.target_ms
        );
    }
}
