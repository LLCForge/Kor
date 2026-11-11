//! `shellui.*`: tepsi, bildirim merkezi, onay kuyruğu, PiP, "beni çağır" ayarları (§27.1, 27.2, 27.4, 27.8,
//! 27.9, 28.4, 28.5; ADR 0051). İş mantığı `kor_runtime::presence` ve `crate::presence` içinde.
//!
//! - `shellui.state()` → `{ overview, unread, update, recent, pip, progress }` (tepsi açılır penceresi, PiP)
//! - `shellui.status()` → `kor status --json` ile aynı `Overview`
//! - `shellui.queue()` → `{ items, groups }` (`kor approvals`)
//! - `shellui.approve(id)` / `shellui.deny(id)` → kaynak (`desktop`|`daemon`|`remote`)
//! - `shellui.answerMany(ids[], approve)` → `{ ok, failed[] }` (benzer onaylar birlikte)
//! - `shellui.notifications()` → `{ items, unread }`; `notificationsRead(ids?)`, `notificationsClear(ids?)`
//! - `shellui.activity()` (boşta kalma kuralı), `shellui.pause(bool)`
//! - `shellui.pip(show?)`, `shellui.hidePopup()`, `shellui.openMain(sessionId?)`, `shellui.openProject(path)`
//! - `shellui.stop(sessionId)`, `shellui.quickTask(root, prompt)` → `{ sessionId }`
//! - `shellui.settings()` / `shellui.setSetting(key, value)`
//! - `shellui.updateOnExit()` → güncelleme durumu
//!
//! Olaylar: `kor:shellui/changed`, `kor:shellui/notification`, `kor:shellui/answered`, `kor:shellui/openSession`,
//! `kor:shellui/openPanel`, `kor:shellui/openProject`, `kor:shellui/newChat`, `kor:shellui/openUpdate`.

use kor_runtime::presence::notifications::Store;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, opt_s, s};
use crate::presence;
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `shellui::call`'a yönlendirir).
pub const NS: &str = "shellui";

const MAX_PROMPT: usize = 20_000;

fn store() -> Result<Store, String> {
    Store::standard().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

/// `null`/eksik → hepsi; aksi halde kimlik listesi.
fn ids(args: &[Value], i: usize) -> Result<Option<Vec<String>>, String> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| s.len() <= 64)
                    .map(str::to_owned)
                    .ok_or_else(|| "Geçersiz bildirim kimliği".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("Kimlik listesi gerekli".into()),
    }
}

fn notifications() -> Res {
    let st = store()?;
    Ok(json!({ "items": st.list(), "unread": st.unread() }))
}

/// Hızlı görev (tepsi): yeni sohbet seçilen projede başlar, pencere açılmaz.
fn quick_task(app: &AppHandle, state: &AppState, root: &str, prompt: &str) -> Res {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.len() > MAX_PROMPT {
        return Err("Görev metni boş ya da çok uzun".into());
    }
    let dir = if root.is_empty() {
        state.workspace()?
    } else {
        let p = std::path::PathBuf::from(root);
        // Yalnızca son kullanılan klasörler ve açık çalışma klasörü: arayüzden rastgele yol gelmez.
        let known =
            state.recent().iter().any(|r| r == root) || state.root().is_some_and(|r| r == p);
        if !known || !p.is_dir() {
            return Err("Proje son kullanılanlar arasında değil".into());
        }
        p
    };
    let id = crate::meta::new_uuid();
    crate::chat::start_in(
        app,
        kor_tools::without_verbatim(&dir),
        crate::chat::SendArgs {
            session_id: id.clone(),
            text: prompt.to_owned(),
            attachments: Vec::new(),
            open_file: None,
        },
    )?;
    let _ = app.emit("kor:sessionsChanged", ());
    Ok(json!({ "sessionId": id }))
}

async fn answer(app: &AppHandle, args: &[Value], approve: bool, m: &str) -> Res {
    let id = s(args, 0, m)?;
    presence::answer(app, id, approve).await.map(Value::from)
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("{NS}.{name}");
    let m = m.as_str();
    Some(match name {
        "state" => Ok(presence::state_json(app).await),
        "status" => serde_json::to_value(kor_runtime::presence::overview::collect().await)
            .map_err(|e| e.to_string()),
        "queue" => {
            let items = presence::queue_items().await;
            let groups = kor_runtime::presence::queue::groups(&items);
            Ok(json!({ "items": items, "groups": groups }))
        }
        "approve" => answer(app, args, true, m).await,
        "deny" => answer(app, args, false, m).await,
        "answerMany" => {
            let approve = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            match ids(args, 0) {
                Ok(Some(list)) => {
                    let mut failed = Vec::new();
                    for id in &list {
                        if let Err(e) = presence::answer(app, id, approve).await {
                            failed.push(json!({ "id": id, "error": e }));
                        }
                    }
                    Ok(json!({ "ok": failed.is_empty(), "failed": failed }))
                }
                Ok(None) => Err("Onay kimlikleri gerekli".into()),
                Err(e) => Err(e),
            }
        }
        "notifications" => notifications(),
        "notificationsRead" => ids(args, 0).and_then(|ids| {
            store()?.mark_read(ids.as_deref())?;
            let _ = app.emit("kor:shellui/changed", ());
            notifications()
        }),
        "notificationsClear" => ids(args, 0).and_then(|ids| {
            store()?.clear(ids.as_deref())?;
            let _ = app.emit("kor:shellui/changed", ());
            notifications()
        }),
        "activity" => {
            presence::activity();
            Ok(Value::Null)
        }
        "pause" => {
            presence::set_paused(app, args.first().and_then(Value::as_bool).unwrap_or(true));
            Ok(Value::Bool(presence::paused()))
        }
        "pip" => {
            match args.first().and_then(Value::as_bool) {
                Some(show) => presence::set_pip(app, show),
                None => presence::toggle_pip(app),
            }
            Ok(Value::Null)
        }
        "hidePopup" => {
            presence::hide_popup(app);
            Ok(Value::Null)
        }
        "openMain" => {
            presence::hide_popup(app);
            presence::open_target(
                app,
                opt_s(args, 0).filter(|s| kor_runtime::valid_session_id(s)),
                false,
            );
            Ok(Value::Null)
        }
        "openProject" => s(args, 0, m).and_then(|path| {
            if !state.recent().iter().any(|r| r == path) {
                return Err("Proje son kullanılanlar arasında değil".into());
            }
            presence::hide_popup(app);
            presence::show_main(app);
            let _ = app.emit("kor:shellui/openProject", path);
            Ok(Value::Null)
        }),
        "stop" => s(args, 0, m).map(|id| {
            state.chats.abort(id);
            Value::Null
        }),
        "quickTask" => s(args, 0, m).and_then(|root| quick_task(app, state, root, s(args, 1, m)?)),
        "settings" => Ok(presence::settings_json()),
        "setSetting" => s(args, 0, m)
            .and_then(|key| presence::set_setting(app, key, args.get(1).unwrap_or(&Value::Null))),
        "updateOnExit" => {
            let r = crate::packaging::prepare_on_exit(app, state).await;
            if r.is_ok() {
                presence::set_update_on_exit(true);
            }
            r
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_lists_are_validated() {
        assert_eq!(ids(&[], 0).unwrap(), None);
        assert_eq!(ids(&[Value::Null], 0).unwrap(), None);
        assert_eq!(
            ids(&[json!(["n1", "n2"])], 0).unwrap(),
            Some(vec!["n1".into(), "n2".into()])
        );
        assert!(ids(&[json!([1])], 0).is_err());
        assert!(ids(&[json!("n1")], 0).is_err());
        assert!(ids(&[json!(["x".repeat(65)])], 0).is_err());
    }
}
