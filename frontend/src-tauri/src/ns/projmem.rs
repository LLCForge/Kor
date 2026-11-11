//! `projmem.*`: Hafıza panelinin `memory.*`'da olmayan yöntemleri: proje hafıza belgelerini (§6.1)
//! okuma/yazma, oturum özetleri, karar ekleme, bilgi tabanı zaman çizelgesi (§31.3) ve klasör
//! seçici. İş mantığı `kor_memory::project` içinde; burası yalnızca JSON'a çevirir.
//!
//! - `projmem.overview()` → `{ project, dir, docs: [{ file, exists }], sessions: string[], gitNotes }`
//! - `projmem.read(file)` → `{ content, mtimeMs, sections: [{ name, start, end, edited }] }`
//! - `projmem.write(file, content, expectedMtimeMs: number | null)` → `{ ok: true, mtimeMs }` ya da
//!   `{ ok: false, reason: "conflict" }` (Kor ya da başka bir araç dosyayı bu arada değiştirdiyse)
//! - `projmem.session(day)` → `{ content }` (salt okunur)
//! - `projmem.addDecision(title, body, refs: string[])` → `{ decisions }` (`decisions.md`'ye ekler)
//! - `projmem.recent(project: string | null, limit?: number)` → `{ records }` (yeniden eskiye)
//! - `projmem.pickFolder()` → seçilen klasör ya da `null`

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use kor_memory::{Decision, Doc, Knowledge, ProjectMemory};
use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `projmem::call`'a yönlendirir).
pub const NS: &str = "projmem";

/// Belge boyutu üst sınırı; hafıza belgeleri sisteme isteme girer, büyümesin.
const MAX_DOC: usize = 512 * 1024;

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if name == "pickFolder" {
        return Some(pick_folder(app).await);
    }
    let args = args.to_vec();
    let root = root(state);
    let git_notes = root
        .as_ref()
        .ok()
        .and_then(|r| crate::bridge::load_config(r).ok())
        .is_some_and(|l| l.config.memory.git_notes);
    let name = name.to_owned();
    // Dosya ve SQLite çağrıları engelleyicidir: arayüz iş parçacığını tutmasın.
    let out = tauri::async_runtime::spawn_blocking(move || match name.as_str() {
        "overview" => Some(overview(root, git_notes)),
        "read" => Some(read(&args, root)),
        "write" => Some(write(&args, root)),
        "session" => Some(session(&args, root)),
        "addDecision" => Some(add_decision(&args, root)),
        "recent" => Some(recent(&args)),
        _ => None,
    })
    .await;
    match out {
        Ok(r) => r,
        Err(e) => Some(Err(e.to_string())),
    }
}

fn err(e: impl std::fmt::Display) -> String {
    format!("Hafıza hatası: {e}")
}

fn home() -> Result<PathBuf, String> {
    kor_runtime::kor_home().ok_or_else(|| "Kor ana klasörü bulunamadı (KOR_HOME)".to_owned())
}

fn project(root: Result<PathBuf, String>) -> Result<(PathBuf, ProjectMemory), String> {
    let root = root?;
    let mem = ProjectMemory::new(&home()?, &root);
    Ok((root, mem))
}

fn doc(args: &[Value], method: &str) -> Result<Doc, String> {
    let file = s(args, 0, method)?;
    Doc::from_file(file).ok_or_else(|| format!("Bilinmeyen hafıza belgesi: {file}"))
}

fn mtime_ms(p: &Path) -> Option<f64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_micros() as f64 / 1000.0)
}

fn overview(root: Result<PathBuf, String>, git_notes: bool) -> Res {
    let (root, mem) = project(root)?;
    let docs: Vec<Value> = Doc::ALL
        .iter()
        .map(|d| json!({ "file": d.file(), "exists": mem.path(*d).is_file() }))
        .collect();
    Ok(json!({
        "project": kor_memory::project_key(&root),
        "dir": mem.dir,
        "docs": docs,
        "sessions": mem.session_days(),
        "gitNotes": git_notes,
    }))
}

fn read(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let d = doc(args, "projmem.read")?;
    let (_, mem) = project(root)?;
    let content = mem.text(d);
    Ok(json!({
        "content": content,
        // Henüz olmayan belge 0: ilk kayıtta çakışma denetimi "yok" olarak geçer.
        "mtimeMs": mtime_ms(&mem.path(d)).unwrap_or(0.0),
        "sections": kor_memory::project::sections(&content),
    }))
}

fn write(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let d = doc(args, "projmem.write")?;
    let content = s(args, 1, "projmem.write")?;
    if content.len() > MAX_DOC {
        return Err("Belge çok büyük (en fazla 512 KB)".into());
    }
    let expected = args.get(2).and_then(Value::as_f64);
    let (_, mem) = project(root)?;
    let path = mem.path(d);
    // Okunduktan sonra Kor (bölüm yazımı, karar ekleme) ya da başka biri değiştirdiyse üzerine yazma.
    if let Some(e) = expected {
        let now = mtime_ms(&path).unwrap_or(0.0);
        if (now - e).abs() > 1.0 {
            return Ok(json!({ "ok": false, "reason": "conflict" }));
        }
    }
    mem.write_doc(d, content).map_err(err)?;
    Ok(json!({ "ok": true, "mtimeMs": mtime_ms(&path).unwrap_or(0.0) }))
}

fn session(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let day = s(args, 0, "projmem.session")?;
    let (_, mem) = project(root)?;
    let content = mem.session(day).ok_or("Oturum özeti bulunamadı")?;
    Ok(json!({ "content": content }))
}

fn add_decision(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let title = s(args, 0, "projmem.addDecision")?.trim();
    let body = args.get(1).and_then(Value::as_str).unwrap_or_default();
    if title.is_empty() || title.chars().count() > 200 {
        return Err("Başlık 1-200 karakter olmalı".into());
    }
    if body.len() > 20_000 {
        return Err("Gerekçe en fazla 20.000 karakter olabilir".into());
    }
    let refs: Vec<String> = args
        .get(2)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .take(50)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let (_, mem) = project(root)?;
    let date = kor_memory::kb::date(chrono::Utc::now().timestamp());
    mem.add_decision(&Decision {
        title: title.to_owned(),
        body: body.to_owned(),
        date,
        refs,
    })
    .map_err(err)?;
    Ok(json!({ "decisions": mem.decisions() }))
}

fn recent(args: &[Value]) -> Res {
    let only = args
        .first()
        .and_then(Value::as_str)
        .map(|p| kor_memory::project_key(Path::new(p)));
    let limit = match args.get(1) {
        None | Some(Value::Null) => 50,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=200).contains(n))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("Kayıt sayısı 1-200 olmalı")?,
    };
    let kb = Knowledge::open(&kor_memory::knowledge_path(&home()?)).map_err(err)?;
    Ok(json!({ "records": kb.recent(only.as_deref(), limit).map_err(err)? }))
}

async fn pick_folder(app: &AppHandle) -> Res {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map_or(Value::Null, |p| {
            Value::String(
                kor_tools::without_verbatim(&p)
                    .to_string_lossy()
                    .into_owned(),
            )
        }))
}
