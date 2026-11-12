//! `perfmon.*` (§32.3, ADR 0062): Ctrl+Shift+P performans bindirmesi, yerel `perf.log`, düşük güç
//! modu ve `kor bench --ui` için masaüstü ölçümleri. Bütçeler ve günlük biçimi
//! `kor_runtime::budgets` içinde.
//!
//! - `perfmon.config()` → `{ log, lowPower, bench }` (`bench`: `"startup"`, `"full"` ya da `null`)
//! - `perfmon.stats()` → `{ rssMb, selfRssMb, cpuMs, processes, workers, tasks, queue, uptimeMs }`
//! - `perfmon.log({ kind, what, ms, detail? })` → `perf.log`'a bir satır (yalnızca `[perf] log` açıkken)
//! - `perfmon.causes()` → perf.log'daki en çok zaman kaybettiren nedenler (`kor doctor --perf` ile aynı)
//! - `perfmon.ready()` → pencere çizildi ve etkileşime hazır; ölçüm modunda `kor bench`'e bildirilir
//! - `perfmon.idle(seconds)` → `{ rssMb, cpuPercent }` (süreç ağacı: WebView dahil)
//! - `perfmon.benchSearch(query)` → 50k dosyalık geçici bir ağaçta gerçek dosya araması, isabet sayısı
//! - `perfmon.benchDone(results)` / `perfmon.benchError(message)` → sonuçları gönderir, uygulamayı kapatır
//!
//! Kendi Tauri komutu `kor_perf` (kor-bridge.ts `perfmon.*` yollarını buraya gönderir): açılış ölçümü
//! `kor_call` yönlendirmesinden bağımsızdır ve ölçüm/bindirme hiçbir ad alanının yükünü taşımaz.
//! lib.rs `run` açılışta `mark_start` çağırır.

use std::io::Write as _;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use kor_runtime::budgets;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, s};
use crate::state::AppState;

/// Ad alanı. `ns/mod.rs` bunu YÖNLENDİRMEZ: modül lib.rs'den `perfmon` olarak derlenir ve kendi
/// `kor_perf` komutuyla çağrılır (açılış ölçümü genel `kor_call` dağıtımından bağımsız).
#[allow(dead_code)] // tests/ipc_schema.rs ad alanının sahibini bu satırdan okur
pub const NS: &str = "perfmon";

/// Süreç başlangıcı (lib.rs `run`'ın ilk satırı `mark_start` çağırır).
static START: LazyLock<Instant> = LazyLock::new(Instant::now);
/// `[perf] log` (son `config` okumasından); kapalıyken `log` hiçbir şey yazmaz.
static LOG: AtomicBool = AtomicBool::new(false);
/// `kor bench`'e açık bağlantı (ölçüm modunda).
static BENCH: Mutex<Option<TcpStream>> = Mutex::new(None);
/// `benchSearch` için üretilmiş 50k dosyalık ağaç.
static FILES: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Dosya araması ölçümündeki dosya sayısı (§32.1).
const BENCH_FILES: usize = 50_000;

pub fn mark_start() {
    LazyLock::force(&START);
}

fn bench_mode() -> Option<String> {
    std::env::var("KOR_BENCH_PORT").ok()?;
    Some(std::env::var("KOR_BENCH_MODE").unwrap_or_else(|_| "full".into()))
}

/// `perfmon.<name>` (`kor-bridge.ts` → `invoke('kor_perf', { method, args })`).
#[tauri::command]
pub async fn kor_perf(
    method: String,
    args: Vec<Value>,
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Res {
    let name = method.strip_prefix("perfmon.").unwrap_or(&method);
    call(name, &args, &app, &state)
        .await
        .unwrap_or_else(|| Err(format!("`{method}`: bilinmeyen perfmon yöntemi")))
}

async fn call(name: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Option<Res> {
    Some(match name {
        "config" => Ok(config(state)),
        "stats" => Ok(stats()),
        "log" => {
            log(args.first().unwrap_or(&Value::Null));
            Ok(Value::Null)
        }
        "causes" => Ok(causes().await),
        "ready" => ready(app),
        "idle" => Ok(idle(args.first().and_then(Value::as_f64).unwrap_or(5.0)).await),
        "benchSearch" => match s(args, 0, "perfmon.benchSearch") {
            Ok(q) => bench_search(q.to_owned()).await,
            Err(e) => Err(e),
        },
        "benchDone" => finish(
            app,
            json!({ "event": "results", "results": args.first().cloned().unwrap_or(Value::Null) }),
        ),
        "benchError" => finish(
            app,
            json!({ "event": "error", "message": args.first().and_then(Value::as_str).unwrap_or("?") }),
        ),
        _ => return None,
    })
}

fn config(state: &AppState) -> Value {
    let cfg = crate::bridge::load_config(&state.root().unwrap_or_default())
        .map(|l| l.config)
        .unwrap_or_default();
    LOG.store(cfg.perf.log, Ordering::Relaxed);
    json!({
        "log": cfg.perf.log,
        // Background indexers (graph watcher, knowledge embedding) slow down too.
        "lowPower": budgets::apply_low_power(&cfg),
        "bench": bench_mode(),
    })
}

#[allow(clippy::cast_precision_loss)] // MB for display
fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn stats() -> Value {
    let tree = kor_platform::procmem::tree().unwrap_or_default();
    let rt = tokio::runtime::Handle::try_current()
        .ok()
        .map(|h| h.metrics());
    json!({
        "rssMb": mb(tree.rss),
        "selfRssMb": kor_platform::procmem::rss().map(mb),
        "cpuMs": tree.cpu.as_millis(),
        "processes": tree.processes,
        "workers": rt.as_ref().map(tokio::runtime::RuntimeMetrics::num_workers),
        "tasks": rt.as_ref().map(tokio::runtime::RuntimeMetrics::num_alive_tasks),
        "queue": rt.as_ref().map(tokio::runtime::RuntimeMetrics::global_queue_depth),
        "uptimeMs": START.elapsed().as_millis(),
    })
}

/// Bir yavaş kare / IPC çağrısı (arayüz 50 ms'yi aşanları gönderir).
fn log(v: &Value) {
    if !LOG.load(Ordering::Relaxed) {
        return;
    }
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
    let ms = v.get("ms").and_then(Value::as_f64).unwrap_or(0.0);
    let detail = v.get("detail").and_then(Value::as_str).map(str::to_owned);
    let entry = budgets::PerfEntry::now("desktop", text("kind"), text("what"), ms, detail);
    if let Some(p) = budgets::perf_log_path() {
        // Disk yazımı tokio işçisini bekletmesin.
        std::thread::spawn(move || {
            let _ = budgets::append_to(&p, &entry);
        });
    }
}

/// `kor doctor --perf`'in masaüstü karşılığı: en çok 10 neden, en çok zaman kaybettiren önce.
async fn causes() -> Value {
    let Some(path) = budgets::perf_log_path() else {
        return json!([]);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let list: Vec<budgets::Cause> = budgets::causes(&budgets::read_log(&path))
            .into_iter()
            .take(10)
            .collect();
        serde_json::to_value(list).unwrap_or_else(|_| json!([]))
    })
    .await
    .unwrap_or_else(|_| json!([]))
}

fn send_line(v: &Value) -> Result<(), String> {
    let mut guard = BENCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        let port: u16 = std::env::var("KOR_BENCH_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .ok_or("ölçüm modu değil")?;
        let sock = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        *guard = Some(sock);
    }
    let sock = guard.as_mut().ok_or("bağlantı yok")?;
    writeln!(sock, "{v}")
        .and_then(|()| sock.flush())
        .map_err(|e| e.to_string())
}

fn ready(app: &AppHandle) -> Res {
    let ms = START.elapsed().as_secs_f64() * 1000.0;
    if let Some(mode) = bench_mode() {
        send_line(&json!({ "event": "ready", "ms": ms }))?;
        if mode == "startup" {
            app.exit(0);
        }
    }
    Ok(json!({ "ms": ms }))
}

fn finish(app: &AppHandle, line: Value) -> Res {
    if bench_mode().is_none() {
        return Err("ölçüm modu değil".into());
    }
    let r = send_line(&line);
    if let Some(dir) = FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        let _ = std::fs::remove_dir_all(dir);
    }
    app.exit(0);
    r.map(|()| Value::Null)
}

/// Süreç ağacının (WebView dahil) `seconds` boyunca CPU payı ve sonundaki bellek.
async fn idle(seconds: f64) -> Value {
    let window = Duration::from_secs_f64(seconds.clamp(1.0, 60.0));
    let before = kor_platform::procmem::tree().unwrap_or_default();
    tokio::time::sleep(window).await;
    let after = kor_platform::procmem::tree().unwrap_or_default();
    let busy = after.cpu.saturating_sub(before.cpu).as_secs_f64() / window.as_secs_f64();
    json!({ "rssMb": mb(after.rss.max(before.rss)), "cpuPercent": busy * 100.0, "processes": after.processes })
}

/// `count` dosyalık, gerçekçi derinlikte bir proje ağacı.
fn make_tree(root: &std::path::Path, count: usize) -> std::io::Result<()> {
    let dirs = [
        "src",
        "src/ui",
        "src/core",
        "tests",
        "docs",
        "crates/a/src",
        "crates/b/src",
        "web/components",
    ];
    for i in 0..count {
        let dir = root
            .join(dirs[i % dirs.len()])
            .join(format!("m{}", i / 500));
        if i % 500 == 0 || !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
        std::fs::write(
            dir.join(format!(
                "file_{i}_{}.rs",
                ["chat", "view", "model", "store", "util"][i % 5]
            )),
            b"",
        )?;
    }
    Ok(())
}

async fn bench_search(query: String) -> Res {
    if bench_mode().is_none() {
        return Err("ölçüm modu değil".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let root = {
            let mut g = FILES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match g.as_ref() {
                Some(r) => r.clone(),
                None => {
                    let r = std::env::temp_dir()
                        .join(format!("kor-bench-files-{}", std::process::id()));
                    make_tree(&r, BENCH_FILES).map_err(|e| e.to_string())?;
                    *g = Some(r.clone());
                    r
                }
            }
        };
        let hits = crate::api_files::search_files(&root, &query);
        Ok(json!({ "hits": hits.len() }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_report_this_process() {
        let v = stats();
        assert!(v["rssMb"].as_f64().unwrap() > 1.0, "{v}");
        assert!(v["processes"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn a_generated_tree_is_searchable() {
        let dir = tempfile::tempdir().unwrap();
        make_tree(dir.path(), 1200).unwrap();
        let hits = crate::api_files::search_files(dir.path(), "file_1199");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].1.ends_with("file_1199_util.rs"), "{hits:?}");
    }

    #[test]
    fn logging_is_off_until_config_turns_it_on() {
        LOG.store(false, Ordering::Relaxed);
        // No panic, no file: `log` is a no-op while off.
        log(&json!({ "kind": "frame", "what": "x", "ms": 80.0 }));
        assert!(!LOG.load(Ordering::Relaxed));
    }
}
