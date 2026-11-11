//! Vector search speed (spec table 31.3.1: top-50 over 1M embeddings, p95 < 30 ms) and the real
//! local model. Both ignored by default:
//! `cargo test -p kor-memory --release --test vectors -- --ignored --nocapture`
//! (`KOR_BENCH_VECTORS` = count, default 1000000; `KOR_E5_DIR` = a folder with the
//! `model.safetensors` and `tokenizer.json` of the `embed-e5-small` component, else the
//! installed component is used).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kor_memory::embed::{Embed, Kind, Local, VecIndex, quantize};

fn unit(rng: &mut u64, dims: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dims)
        .map(|_| {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 7;
            *rng ^= *rng << 17;
            // Uniform in [-1, 1); precision is irrelevant for synthetic data.
            #[allow(clippy::cast_precision_loss)]
            let x = (*rng >> 40) as f32 / (1u64 << 23) as f32;
            x - 1.0
        })
        .collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in &mut v {
        *x /= n;
    }
    v
}

fn p(mut v: Vec<Duration>, q: f64) -> f64 {
    v.sort();
    // Index into a short sorted list.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((v.len() - 1) as f64 * q) as usize;
    v[i].as_secs_f64() * 1000.0
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn vector_top50_p95() {
    let n: usize = std::env::var("KOR_BENCH_VECTORS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let dims = 384;
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let t = Instant::now();
    let mut ix = VecIndex::default();
    for id in 0..n {
        ix.push(
            i64::try_from(id).unwrap_or(0),
            &quantize(&unit(&mut rng, dims)),
        );
    }
    let fill = t.elapsed();
    // Display only.
    #[allow(clippy::cast_precision_loss)]
    let mb = (n * (dims + 12)) as f64 / 1_048_576.0;
    let mut times = Vec::new();
    for _ in 0..100 {
        let q = unit(&mut rng, dims);
        let t = Instant::now();
        let top = ix.top(&q, 50);
        times.push(t.elapsed());
        assert_eq!(top.len(), 50);
    }
    println!(
        "vector top-50 over {n} x {dims} int8: p50 {:.1} ms, p95 {:.1} ms (target < 30 ms); \
         build {:.1} s, {:.0} MB in memory, {} threads",
        p(times.clone(), 0.5),
        p(times, 0.95),
        fill.as_secs_f64(),
        mb,
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
    );
}

fn local() -> Option<Arc<Local>> {
    match std::env::var_os("KOR_E5_DIR") {
        Some(d) => {
            let d = PathBuf::from(d);
            Some(Arc::new(Local::new(
                d.join("model.safetensors"),
                d.join("tokenizer.json"),
            )))
        }
        None => Local::installed(),
    }
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
#[ignore = "needs the embed-e5-small component (or KOR_E5_DIR)"]
#[allow(clippy::many_single_char_names)]
fn real_model_ranks_meaning_across_languages() {
    let Some(m) = local() else {
        panic!("install it first: kor component add embed-e5-small");
    };
    let docs: Vec<String> = [
        "Integrated the iyzico payment API and verified webhook signatures",
        "iyzico ödeme entegrasyonu: webhook imzası doğrulandı",
        "Fixed the login token refresh bug in auth.rs",
        "Postgres migration adds an index on orders(created_at)",
        "Updated the README badges and CI cache",
        "Dark mode toggle in settings page",
    ]
    .map(str::to_owned)
    .to_vec();
    let t = Instant::now();
    let v = m.embed(&docs, Kind::Passage).unwrap();
    println!(
        "first call (load + embed): {:.2} s",
        t.elapsed().as_secs_f64()
    );
    assert_eq!(v[0].len(), 384);
    for query in [
        "iyzico ile ödeme entegrasyonunu nasıl yaptık",
        "payment webhook",
    ] {
        let q = m.embed(&[query.to_owned()], Kind::Query).unwrap().remove(0);
        let s: Vec<f32> = v.iter().map(|d| cos(&q, d)).collect();
        println!("{query}: {s:?}");
        let best = s[0].max(s[1]);
        // Both payment records (English and Turkish) above the floor, the rest outside the band.
        assert!(s[0].min(s[1]) >= m.min_similarity(), "{s:?}");
        assert!(
            s[2..].iter().all(|x| *x < best - m.similarity_band()),
            "{s:?}"
        );
    }

    // Throughput: record-sized passages (~60 words), batches of 16.
    let passage = "Fixed the login token refresh bug: the refresh ran after expiry, so the \
        session dropped. Now it refreshes 60 s before expiry and retries once. Files: \
        src/auth.rs, src/session.rs. Command: cargo test -p auth. Error solved: E0502 cannot \
        borrow the token map, cloned the key first.";
    let batch: Vec<String> = (0..16).map(|i| format!("{i} {passage}")).collect();
    let t = Instant::now();
    let rounds = 4;
    for _ in 0..rounds {
        m.embed(&batch, Kind::Passage).unwrap();
    }
    let secs = t.elapsed().as_secs_f64();
    let mut qt = Vec::new();
    for i in 0..20 {
        let t = Instant::now();
        m.embed(&[format!("login token bug {i}")], Kind::Query)
            .unwrap();
        qt.push(t.elapsed());
    }
    println!(
        "throughput: {:.1} passages/s (batches of 16); query embed p50 {:.1} ms, p95 {:.1} ms",
        f64::from(16 * rounds) / secs,
        p(qt.clone(), 0.5),
        p(qt, 0.95)
    );
}
