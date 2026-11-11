//! `packages.*`: monorepo paketleri (§15.15) için ince köprü (`kor packages`, `kor packages
//! affected|deps|cost`). Algılama, paket grafı ve daraltılmış komutlar `kor_graph::workspace`
//! içinde; burası yalnızca JSON'a çevirir. Yollar çalışma alanı köküne göre `/` ayraçlıdır.
//!
//! - `packages.list()` → `{ root, kinds, packages: [{ name, tool, root, manifest, deps }] }`
//! - `packages.affected(since?: string)` → `{ changed, packages, commands }`
//! - `packages.deps(name)` → `{ package, deps, dependents }`
//! - `packages.cost(days?: number)` → `{ rows }` (`rows[].model` paket adıdır; `"-"` = pakette değil)

use std::path::Path;

use kor_graph::workspace::{self, Package, Workspace};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `packages::call`'a yönlendirir).
pub const NS: &str = "packages";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let ws = match root(state) {
        Ok(r) => workspace::detect(&r),
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "list" => Ok(json!({
            "root": ws.root,
            "kinds": ws.kinds,
            "packages": ws.packages.iter().map(|p| view(&ws, p)).collect::<Vec<_>>(),
        })),
        "affected" => affected(&ws, opt_s(args, 0)),
        "deps" => s(args, 0, "packages.deps").and_then(|n| deps(&ws, n)),
        "cost" => cost(&ws, args),
        _ => return None,
    })
}

fn view(ws: &Workspace, p: &Package) -> Value {
    json!({
        "name": p.name,
        "tool": p.tool,
        "root": ws.rel(&p.root),
        "manifest": ws.rel(&p.manifest),
        "deps": p.deps,
    })
}

fn affected(ws: &Workspace, since: Option<&str>) -> Res {
    let changed = workspace::changed_files(&ws.root, since)
        .map_err(|e| format!("Git değişiklikleri okunamadı: {e}"))?;
    let hit = ws.affected(&changed);
    Ok(json!({
        "changed": changed.iter().map(|c| ws.rel(c)).collect::<Vec<_>>(),
        "packages": hit.iter().map(|p| view(ws, p)).collect::<Vec<_>>(),
        "commands": ws.narrowed_commands(&hit),
    }))
}

fn deps(ws: &Workspace, name: &str) -> Res {
    let p = ws
        .get(name)
        .ok_or_else(|| format!("`{name}` adlı paket yok"))?;
    let users: Vec<&str> = ws
        .dependents(name)
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    Ok(json!({ "package": view(ws, p), "deps": p.deps, "dependents": users }))
}

fn cost(ws: &Workspace, args: &[Value]) -> Res {
    use kor_runtime::cost;
    let days = match args.first() {
        None | Some(Value::Null) => 30,
        Some(v) => v
            .as_u64()
            .filter(|d| (1..=365).contains(d))
            .and_then(|d| u32::try_from(d).ok())
            .ok_or("Gün sayısı 1-365 olmalı")?,
    };
    let entries: Vec<_> = kor_runtime::usage::file()
        .map(|p| kor_runtime::usage::read(&p))
        .unwrap_or_default()
        .into_iter()
        .filter(|e| {
            e.path
                .as_deref()
                .is_some_and(|p| Path::new(p).starts_with(&ws.root))
        })
        .collect();
    let rows = cost::rows_by(&entries, &cost::Prices::load(), cost::now_ms(), days, |e| {
        e.path
            .as_deref()
            .and_then(|p| ws.package_of(Path::new(p)))
            .map_or_else(|| "-".to_owned(), |p| p.name.clone())
    });
    Ok(json!({ "rows": rows }))
}
