//! `watch.*`: `kor watch` köprüsü (§8.7, ADR 0041). Algılama, olay kaydı ve düzeltme akışı
//! `kor_runtime::watch` içinde.
//!
//! - `watch.start(source)` → `{ id, source }`; olaylar `kor:watch/event` `{ id, event }`
//!   (`listening`, `incident`, `repeat`, `limited`, `ended`)
//! - `watch.stop(id)` → `bool`, `watch.list()` → `[{ id, source }]`
//! - `watch.incidents()` → `Incident[]`, `watch.clear(id?)` → silinen sayısı
//! - `watch.fixStart(id)` → `{ incident, prompt }`: `kor/fix-<id>` dalı açılır; arayüz istemi
//!   sohbet kutusuna koyar, ajan kullanıcının gönderdiği mesajla (dosya bazında onaylı) çalışır
//! - `watch.fixFiles()` → değişen dosyalar, `watch.fixFinish(id)` → `PrRef` (commit + push + PR)

use std::collections::BTreeMap;
use std::sync::Mutex;

use kor_runtime::watch::{self, CancellationToken, Source};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "watch";

/// Çalışan izleyiciler: kimlik → (kaynak, durdurma).
static RUNNING: Mutex<BTreeMap<u64, (String, CancellationToken)>> = Mutex::new(BTreeMap::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn running() -> std::sync::MutexGuard<'static, BTreeMap<u64, (String, CancellationToken)>> {
    RUNNING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

pub async fn call(_ns: &str, name: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Option<Res> {
    if !matches!(name, "start" | "stop" | "list" | "incidents" | "clear" | "fixStart" | "fixFiles" | "fixFinish") {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "start" => start(args, app, root),
        "stop" => {
            let id = args.first().and_then(Value::as_u64).unwrap_or(0);
            Ok(json!(running().remove(&id).map(|(_, c)| c.cancel()).is_some()))
        }
        "list" => Ok(json!(running().iter().map(|(id, (src, _))| json!({ "id": id, "source": src })).collect::<Vec<_>>())),
        "incidents" => to_json(&watch::incidents(&root)),
        "clear" => watch::clear(&root, opt_s(args, 0)).map(|n| json!(n)),
        "fixStart" => match s(args, 0, "watch.fixStart") {
            Ok(id) => watch::fix_start(&root, id).map(|(incident, prompt)| json!({ "incident": incident, "prompt": prompt })),
            Err(e) => Err(e),
        },
        "fixFiles" => watch::fix_files(&root).map(|f| json!(f)),
        "fixFinish" => match s(args, 0, "watch.fixFinish") {
            Ok(id) => watch::fix_finish(&root, id).await.map(|pr| json!({ "url": pr.url, "number": pr.number, "forge": pr.forge })),
            Err(e) => Err(e),
        },
        _ => Err(format!("`watch.{name}` henüz Kor arka ucunda yok")),
    })
}

fn start(args: &[Value], app: &AppHandle, root: std::path::PathBuf) -> Res {
    let text = s(args, 0, "watch.start")?;
    let source = Source::parse(text, &root)?;
    let cfg = watch::load_config(&root)?;
    let label = source.label();
    if running().values().any(|(src, _)| *src == label) {
        return Err(format!("{label} zaten izleniyor"));
    }
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cancel = CancellationToken::new();
    running().insert(id, (label.clone(), cancel.clone()));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let emit_app = app.clone();
        let on = move |e: &watch::Event| {
            let _ = emit_app.emit("kor:watch/event", json!({ "id": id, "event": e }));
        };
        let _ = watch::watch(&root, &source, &cfg, &on, cancel).await;
        running().remove(&id);
    });
    Ok(json!({ "id": id, "source": label }))
}
