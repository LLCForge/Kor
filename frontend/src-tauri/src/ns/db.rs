//! `db.*`: proje grafındaki veritabanı şeması (§7.3, ADR 0024) için ince köprü (`kor db connect|
//! refresh|tables|show`). Şema okuma `kor_agent::remote::db::introspect` içinde (yalnızca katalog
//! SELECT'leri, salt okunur oturum, zaman aşımı); anlık görüntü `.kor/graph/db/<ad>.json` olarak
//! kaydedilir ve açık projenin grafına (`graph.*` ile aynı örnek) katman olarak eklenir. Parolalar
//! anahtar zincirinden okunur, hiçbir yanıtta yer almaz.
//!
//! - `db.connect(name)` → `{ snapshot: { connection, kind, taken_at, tables: [...] }, path, tables }`
//! - `db.refresh(name?)` → `{ refreshed: string[], errors: [{ name, error }], tables }`
//! - `db.tables()` → `TableInfo[]` (`node`, `columns`, `references`, `referenced_by`, `readers`,
//!   `writers`, `live`)
//! - `db.show(table)` → `TableInfo | null`
//!
//! - `db.connect(name, slow?)`: `slow` = en yavaş sorgular da okunur (isteğe bağlı, §8.5)
//! - `db.indexes()` → `{ tables: [{ connection, table, indexes }], suggestions: IndexSuggestion[] }`
//! - `db.migrateNew(description)` → `{ style, prompt }`: arayüz istemi sohbet kutusuna koyar
//! - `db.check(sql)` → yıkıcı ifadeler (`string[]`)
//! - `db.trial(name, sql)` → kopyadaki şema değişikliği (`string[]`); yıkıcı SQL için zorunlu
//! - `db.apply(name, sql)` → `true`; yıkıcı SQL yalnızca başarılı bir denemeden sonra çalışır
//!
//! Olay: grafı değiştiren her çağrıdan sonra `kor:graph/updated` `{ root, stats, status }`.

use std::path::Path;
use std::sync::PoisonError;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `db::call`'a yönlendirir).
pub const NS: &str = "db";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(
        name,
        "connect"
            | "refresh"
            | "tables"
            | "show"
            | "indexes"
            | "migrateNew"
            | "check"
            | "trial"
            | "apply"
    ) {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let allow_network = !privacy(&root);
    if let Some(r) = extra(name, args, &root, allow_network).await {
        return Some(r);
    }
    let previous = kor_graph::db::load_snapshots(&root);
    let slow_of = |n: &str| match name {
        "connect" => crate::api::b(args, 1),
        _ => previous
            .iter()
            .any(|s| s.connection == n && !s.slow_queries.is_empty()),
    };
    let names: Vec<String> = match name {
        "connect" => match s(args, 0, "db.connect") {
            Ok(n) => vec![n.to_owned()],
            Err(e) => return Some(Err(e)),
        },
        "refresh" => match opt_s(args, 0) {
            Some(n) => vec![n.to_owned()],
            None => kor_graph::db::load_snapshots(&root)
                .into_iter()
                .map(|s| s.connection)
                .collect(),
        },
        _ => Vec::new(),
    };
    let file = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let mut done = Vec::new();
    let mut errors = Vec::new();
    for n in &names {
        let mut snap =
            kor_agent::remote::db::introspect(file.clone(), n, &root, allow_network).await;
        if slow_of(n)
            && let Ok(sn) = snap.as_mut()
        {
            match kor_agent::remote::db::slow_queries(file.clone(), n, allow_network).await {
                Ok(q) => sn.slow_queries = q,
                Err(e) => errors
                    .push(json!({ "name": n, "error": format!("Yavaş sorgular okunamadı: {e}") })),
            }
        }
        match snap.and_then(|snap| {
            kor_graph::db::save_snapshot(&root, &snap)
                .map(|p| (snap, p))
                .map_err(|e| e.to_string())
        }) {
            Ok(v) => done.push(v),
            Err(e) => errors.push(json!({ "name": n, "error": e })),
        }
    }
    if name == "connect"
        && let Some(e) = errors.first()
    {
        let msg = e["error"].as_str().unwrap_or_default();
        return Some(Err(format!("Veritabanı şeması okunamadı: {msg}")));
    }
    let (name, args, app) = (name.to_owned(), args.to_vec(), app.clone());
    // Graf güncellemesi diski okur: arayüz iş parçacığını bekletmesin.
    Some(
        tauri::async_runtime::spawn_blocking(move || {
            graph_part(&name, &args, &app, &root, &done, errors)
        })
        .await
        .unwrap_or_else(|e| Err(format!("Graf işlemi başarısız: {e}"))),
    )
}

/// §8.5 yöntemleri (ADR 0041): grafı değiştirmezler.
async fn extra(name: &str, args: &[Value], root: &Path, allow_network: bool) -> Option<Res> {
    let file = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let sql = |i: usize, m: &str| -> Result<String, String> {
        let v = s(args, i, m)?;
        if v.len() > 1024 * 1024 {
            return Err("SQL 1 MB'tan büyük".into());
        }
        Ok(v.to_owned())
    };
    Some(match name {
        "indexes" => {
            let snaps = kor_graph::db::load_snapshots(root);
            let tables: Vec<Value> = snaps
                .iter()
                .flat_map(|s| {
                    s.tables
                        .iter()
                        .filter(|t| !t.indexes.is_empty())
                        .map(|t| json!({ "connection": s.connection, "table": t.name, "indexes": t.indexes }))
                })
                .collect();
            Ok(
                json!({ "tables": tables, "suggestions": kor_runtime::dbops::suggest_indexes(&snaps) }),
            )
        }
        "migrateNew" => s(args, 0, "db.migrateNew").map(|d| {
            let style = kor_runtime::dbops::migration_style(root, d);
            let prompt = kor_runtime::dbops::migration_prompt(&style, d);
            json!({ "style": style, "prompt": prompt })
        }),
        "check" => {
            sql(0, "db.check").map(|q| json!(kor_agent::remote::db::destructive_statements(&q)))
        }
        "trial" => match (s(args, 0, "db.trial"), sql(1, "db.trial")) {
            (Ok(n), Ok(q)) => kor_agent::remote::db::trial(file, n, root, &q, allow_network)
                .await
                .map(|c| json!(c)),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "apply" => match (s(args, 0, "db.apply"), sql(1, "db.apply")) {
            (Ok(n), Ok(q)) => kor_agent::remote::db::apply(file, n, root, &q, allow_network)
                .await
                .map(|()| json!(true)),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        _ => return None,
    })
}

fn privacy(root: &Path) -> bool {
    kor_runtime::privacy_mode()
        || crate::bridge::load_config(root).is_ok_and(|c| c.config.privacy.mode)
}

fn graph_part(
    name: &str,
    args: &[Value],
    app: &AppHandle,
    root: &Path,
    done: &[(kor_graph::DbSnapshot, std::path::PathBuf)],
    errors: Vec<Value>,
) -> Res {
    let g = crate::ns::graph::graph_for(app, root)?;
    let mut g = g.lock().unwrap_or_else(PoisonError::into_inner);
    if !done.is_empty() {
        let stats = g
            .refresh_db()
            .map_err(|e| format!("Graf güncellenemedi: {e}"))?;
        let _ = app.emit(
            "kor:graph/updated",
            json!({ "root": root, "stats": stats, "status": g.status() }),
        );
    }
    match name {
        "connect" => {
            let (snap, path) = done.first().ok_or("bağlantı okunamadı")?;
            Ok(json!({ "snapshot": snap, "path": path, "tables": g.tables().len() }))
        }
        "refresh" => Ok(json!({
            "refreshed": done.iter().map(|(s, _)| s.connection.clone()).collect::<Vec<_>>(),
            "errors": errors,
            "tables": g.tables().len(),
        })),
        "tables" => to_json(&g.tables()),
        "show" => to_json(&g.table(s(args, 0, "db.show")?)),
        _ => Err(format!("`db.{name}` henüz Kor arka ucunda yok")),
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}
