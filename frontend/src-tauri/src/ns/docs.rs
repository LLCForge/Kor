//! `docs.*`: ajan için özel dokümanlar (§28.7, ADR 0063). İş mantığı `kor_runtime::docs`.
//!
//! - `docs.list()` → `Entry[]`
//! - `docs.add(source, browser)` → `Entry` (dosya, klasör ya da URL; URL ağ korumasından geçer)
//! - `docs.pick()` → dosya seçme penceresi, sonra `docs.add` gibi (iptalde `null`)
//! - `docs.search(query)` → `[{ title, text }]`
//! - `docs.refresh(id?)` → `[{ id, changed, error }]`
//! - `docs.remove(id)` → `null`

use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "docs";

fn kb() -> Result<kor_memory::Knowledge, String> {
    let home = kor_runtime::kor_home().ok_or("Kor veri klasörü bulunamadı")?;
    kor_memory::open_knowledge(&home).map_err(|e| e.to_string())
}

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

async fn pick(app: &AppHandle, root: &std::path::Path) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_directory(root)
        .add_filter(
            "Doküman",
            &[
                "pdf", "docx", "md", "markdown", "txt", "html", "htm", "rst", "adoc",
            ],
        )
        .pick_file(move |f| {
            let _ = tx.send(f.and_then(|p| p.into_path().ok()));
        });
    rx.await
        .ok()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(
        name,
        "list" | "add" | "pick" | "search" | "refresh" | "remove"
    ) {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    // Gizlilik/çevrimdışı modu URL çekmelerine uygulanır.
    let _ = crate::bridge::load_config(&root);
    Some(match name {
        "list" => to_json(&kor_runtime::docs::list(&root)),
        "add" | "pick" => {
            let source = if name == "pick" {
                match pick(app, &root).await {
                    Some(p) => p,
                    None => return Some(Ok(Value::Null)),
                }
            } else {
                match s(args, 0, "docs.add") {
                    Ok(x) => x.to_owned(),
                    Err(e) => return Some(Err(e)),
                }
            };
            match kb() {
                Ok(k) => kor_runtime::docs::add(&k, &root, &source, b(args, 1))
                    .await
                    .and_then(|e| to_json(&e)),
                Err(e) => Err(e),
            }
        }
        "search" => (|| {
            let q = s(args, 0, "docs.search")?;
            let hits = kor_runtime::docs::search(&kb()?, &root, q, 10)?;
            Ok(json!(hits.iter().map(|h| json!({ "title": h.record.task, "text": h.record.summary, "source": h.record.files.first() })).collect::<Vec<_>>()))
        })(),
        "refresh" => match kb() {
            Ok(k) => kor_runtime::docs::refresh(&k, &root, opt_s(args, 0))
                .await
                .map(|rows| {
                    json!(
                        rows.into_iter()
                            .map(|(id, r)| match r {
                                Ok(changed) =>
                                    json!({ "id": id, "changed": changed, "error": null }),
                                Err(e) => json!({ "id": id, "changed": false, "error": e }),
                            })
                            .collect::<Vec<_>>()
                    )
                }),
            Err(e) => Err(e),
        },
        _ => s(args, 0, "docs.remove")
            .and_then(|id| kor_runtime::docs::remove(&kb()?, &root, id))
            .map(|()| Value::Null),
    })
}
