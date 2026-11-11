//! `memory.*`: proje hafızası (§6.1) ve bilgi tabanı (§31.3) için ince köprü: `kor recall`,
//! `kor forget`, `kor knowledge export`, `kor workspace`, `kor context add|remove|projects`,
//! `kor memory notes push|pull`. İş mantığı `kor_memory` içinde; burası yalnızca JSON'a çevirir.
//!
//! - `memory.recall(query, project?: string | null, limit?: number)` → `{ hits }`
//!   (`project` verilirse yalnızca o proje; açık proje sıralamada öne çıkar)
//! - `memory.forget(id: number)` → `{ deleted }`
//! - `memory.export(format: "md" | "json", dir: string)` → `{ count, dir }`
//! - `memory.workspaces()` / `memory.workspaceAdd(pattern)` / `memory.workspaceRemove(path)`
//! - `memory.projects()` / `memory.link(path)` / `memory.unlink(path)` (açık projeye bağlı projeler)
//! - `memory.decisions()` → `{ decisions, dir }` (decisions.md girdileri)
//! - `memory.notesPush()` / `memory.notesPull()` (`[memory] git_notes = true` gerekir)
//! - `memory.semantic()` → `{ installed, component, embedded, records }` (yerel gömme modeli)
//! - `memory.reindex()` → `{ embedded, total }` (vektörü olmayan kayıtları yerel modelle göm;
//!   model `components.add("embed-e5-small")` ile kurulur)
//!
//! `memory.recall` model kuruluysa FTS ile vektör benzerliğini birleştirir (ADR 0023).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kor_memory::{ExportFormat, Knowledge, ProjectMemory};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `memory::call`'a yönlendirir).
pub const NS: &str = "memory";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let args = args.to_vec();
    let root = root(state);
    let git_notes = root
        .as_ref()
        .ok()
        .and_then(|r| crate::bridge::load_config(r).ok())
        .is_some_and(|l| l.config.memory.git_notes);
    let name = name.to_owned();
    // SQLite ve git çağrıları engelleyicidir: arayüz iş parçacığını tutmasın.
    let out = tauri::async_runtime::spawn_blocking(move || match name.as_str() {
        "recall" => Some(recall(&args, root)),
        "forget" => Some(forget(&args)),
        "export" => Some(export(&args)),
        "workspaces" => {
            Some(kb().and_then(|k| Ok(json!({ "folders": k.workspaces().map_err(err)? }))))
        }
        "workspaceAdd" => Some(workspace_add(&args)),
        "workspaceRemove" => Some(workspace_remove(&args)),
        "projects" => Some(project(root).map(|(_, m)| json!({ "projects": m.linked_projects() }))),
        "link" => Some(link(&args, root)),
        "unlink" => Some(unlink(&args, root)),
        "decisions" => {
            Some(project(root).map(|(_, m)| json!({ "decisions": m.decisions(), "dir": m.dir })))
        }
        "semantic" => Some(semantic()),
        "reindex" => Some(reindex()),
        "notesPush" => Some(notes(root, git_notes, true)),
        "notesPull" => Some(notes(root, git_notes, false)),
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

/// Süreç boyunca tek bilgi tabanı: vektör indeksi ve gömme modeli her çağrıda yeniden
/// yüklenmez. Model sonradan kurulursa bir sonraki çağrıda gömmeli olarak yeniden açılır.
static KB: Mutex<Option<Arc<Knowledge>>> = Mutex::new(None);

fn kb() -> Result<Arc<Knowledge>, String> {
    let mut open = KB.lock().map_err(err)?;
    if let Some(k) = open.as_ref()
        && (k.semantic() || kor_memory::LocalEmbedder::installed().is_none())
    {
        return Ok(k.clone());
    }
    let k = Arc::new(kor_memory::open_knowledge(&home()?).map_err(err)?);
    *open = Some(k.clone());
    Ok(k)
}

fn semantic() -> Res {
    let k = kb()?;
    Ok(json!({
        "installed": k.semantic(),
        "component": kor_memory::embed::MODEL_COMPONENT,
        "embedded": k.embedded().map_err(err)?,
        "records": k.count().map_err(err)?,
    }))
}

fn reindex() -> Res {
    let k = kb()?;
    if !k.semantic() {
        return Err(format!(
            "Yerel gömme modeli kurulu değil: önce {} bileşenini kurun",
            kor_memory::embed::MODEL_COMPONENT
        ));
    }
    let embedded = k.reindex(&|_, _| {}).map_err(err)?;
    Ok(json!({ "embedded": embedded, "total": k.embedded().map_err(err)? }))
}

fn project(root: Result<PathBuf, String>) -> Result<(PathBuf, ProjectMemory), String> {
    let root = root?;
    let mem = ProjectMemory::new(&home()?, &root);
    Ok((root, mem))
}

fn recall(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let query = s(args, 0, "memory.recall")?;
    if query.trim().is_empty() || query.len() > 500 {
        return Err("Arama 1-500 karakter olmalı".into());
    }
    let only = args
        .get(1)
        .and_then(Value::as_str)
        .map(|p| kor_memory::project_key(Path::new(p)));
    let limit = match args.get(2) {
        None | Some(Value::Null) => 20,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=100).contains(n))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("Sonuç sayısı 1-100 olmalı")?,
    };
    let here = root.ok().map(|r| kor_memory::project_key(&r));
    let hits = kb()?
        .recall(query, only.as_deref(), here.as_deref(), limit)
        .map_err(err)?;
    Ok(json!({ "hits": hits }))
}

fn forget(args: &[Value]) -> Res {
    let id = args
        .first()
        .and_then(Value::as_i64)
        .filter(|i| *i > 0)
        .ok_or("Kayıt numarası pozitif bir tam sayı olmalı")?;
    Ok(json!({ "deleted": kb()?.forget(id).map_err(err)? }))
}

fn export(args: &[Value]) -> Res {
    let format = match s(args, 0, "memory.export")? {
        "md" => ExportFormat::Markdown,
        "json" => ExportFormat::Json,
        _ => return Err("Biçim md ya da json olmalı".into()),
    };
    let dir = PathBuf::from(s(args, 1, "memory.export")?);
    if !dir.is_absolute() {
        return Err("Hedef klasör tam yol olmalı".into());
    }
    let count = kb()?.export(&dir, format).map_err(err)?;
    Ok(json!({ "count": count, "dir": dir }))
}

fn workspace_add(args: &[Value]) -> Res {
    let pattern = s(args, 0, "memory.workspaceAdd")?;
    let added = kb()?.workspace_add(pattern).map_err(err)?;
    Ok(json!({ "added": added }))
}

fn workspace_remove(args: &[Value]) -> Res {
    let path = s(args, 0, "memory.workspaceRemove")?;
    Ok(json!({ "removed": kb()?.workspace_remove(path).map_err(err)? }))
}

fn link(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let path = PathBuf::from(s(args, 0, "memory.link")?);
    let (root, mem) = project(root)?;
    let added = mem.link(&root, &path).map_err(err)?;
    Ok(json!({ "added": added.is_some(), "projects": mem.linked_projects() }))
}

fn unlink(args: &[Value], root: Result<PathBuf, String>) -> Res {
    let path = PathBuf::from(s(args, 0, "memory.unlink")?);
    let (_, mem) = project(root)?;
    let removed = mem.unlink(&path).map_err(err)?;
    Ok(json!({ "removed": removed, "projects": mem.linked_projects() }))
}

fn notes(root: Result<PathBuf, String>, enabled: bool, push: bool) -> Res {
    if !enabled {
        return Err("Git notları kapalı: kor config set memory.git_notes true".into());
    }
    let (root, mem) = project(root)?;
    if push {
        let commit = kor_memory::notes::push(&root, &mem).map_err(err)?;
        Ok(json!({ "commit": commit }))
    } else {
        let added = kor_memory::notes::pull(&root, &mem).map_err(err)?;
        Ok(json!({ "added": added }))
    }
}
