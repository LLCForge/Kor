//! `prefs.*`, `plan.*`, `sessions.*` (ADR 0050): thin bridge for the settings of §28.9–28.14,
//! §28.2 plans, §28.6 session tree and §26.1 `kor doctor --fix`. Logic: `kor_runtime::prefs`,
//! `kor_runtime::session_tree`, `kor_tools::plans`.
//!
//! - `prefs.auto()` → `AutoRow[]`; `prefs.setAuto(key, on)` → `AutoRow[]`
//! - `prefs.ui()` → readability values; `prefs.setUi(name, value)` → the same
//! - `prefs.agent()` → `{ askThreshold, compactConfirm, planLock }`; `prefs.setAgent(name, value)`
//! - `prefs.keymap()` → `{ mode, tui, composer, files, warnings }`
//! - `prefs.answer(id, value)`: a `question` / `compactPreview` card's answer
//! - `prefs.doctorFix()` → `{ fixed: string[], failed: string[], output }` (asks first, natively)
//! - `plan.list()` → `Plan[]`; `plan.show(id)` → `Plan`; `plan.approve(id?)` → `Plan`
//! - `sessions.list(filter?)`, `sessions.search(text)`, `sessions.mv(id, folder|null)`,
//!   `sessions.pin(id, on)`, `sessions.archive(id, on)`, `sessions.tag(id, add[], remove[])`,
//!   `sessions.color(id, color|null)`, `sessions.rename(id, title|null)`,
//!   `sessions.suggest(id, apply?)`, `sessions.export(dir?)`, `sessions.import(file)`

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

/// Bridge namespaces handled here (`ns/mod.rs`: `"prefs" | "plan" | "sessions" => prefs::call`).
pub const NS: &str = "prefs";

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn user_file(root: &Path) -> Result<PathBuf, String> {
    kor_config::Paths::discover(root).user.ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".to_owned())
}

/// Write one config key in the user file and log it (§28.14).
fn set_key(root: &Path, key: &str, value: &str) -> Result<(), String> {
    if value.len() > 200 {
        return Err("Geçersiz değer".into());
    }
    let file = user_file(root)?;
    kor_config::set_scalar(&file, key, value).map_err(|e| e.to_string())?;
    kor_runtime::prefs::log_config_change(key, value, &file);
    Ok(())
}

fn scalar(v: &Value) -> Result<String, String> {
    match v {
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::String(s) if s.chars().all(|c| c.is_alphanumeric() || " -_.,".contains(c)) => Ok(format!("\"{s}\"")),
        _ => Err("Geçersiz değer".into()),
    }
}

fn plan_json(p: &kor_tools::plans::Plan) -> Value {
    json!({
        "id": p.id, "title": p.title, "status": p.status.as_str(), "session": p.session,
        "created": p.created, "body": p.body, "path": p.path.to_string_lossy(),
        "files": kor_tools::plans::files_in(&p.body),
    })
}

fn tree_dirs() -> Result<(PathBuf, PathBuf), String> {
    let s = kor_runtime::sessions_dir().ok_or("Veri klasörü bulunamadı")?;
    let m = kor_runtime::session_tree::meta_path().ok_or("Veri klasörü bulunamadı")?;
    Ok((s, m))
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).take(20).collect())
        .unwrap_or_default()
}

fn on(args: &[Value], i: usize) -> bool {
    args.get(i).and_then(Value::as_bool).unwrap_or(true)
}

async fn doctor_fix(app: &AppHandle) -> Res {
    // The consent is asked here, in a native dialog, so no caller can skip it.
    if !crate::api::confirm(
        app,
        "Sorunları gider",
        "Kor eksik klasörleri oluşturacak, sahipsiz kilit ve yarım kalmış geçici dosyaları silecek; paketlenmiş kor CLI varsa PATH ve kabuk tamamlamalarını kuracak (profil dosyasına tek satır eklenebilir).",
        "Gider",
    )
    .await
    {
        return Err("İptal edildi".into());
    }
    let mut fixed = Vec::new();
    let mut failed = Vec::new();
    for i in kor_runtime::prefs::find_issues(&kor_runtime::prefs::own_dirs(), std::time::Duration::from_secs(3600)) {
        let p = i.path().display().to_string();
        match kor_runtime::prefs::fix(&i) {
            Ok(()) => fixed.push(p),
            Err(e) => failed.push(format!("{p}: {e}")),
        }
    }
    // PATH and completions need the CLI's own command tree: run the bundled `kor`.
    let exe = crate::kor_cli::find();
    let output = match exe {
        Some(cli) => tokio::process::Command::new(cli)
            .args(["doctor", "--fix", "--yes"])
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_else(|e| e.to_string()),
        None => String::new(),
    };
    Ok(json!({ "fixed": fixed, "failed": failed, "output": output }))
}

#[allow(clippy::too_many_lines)] // one arm per method
pub async fn call(ns: &str, name: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Option<Res> {
    let m = &format!("{ns}.{name}");
    let r = match (ns, name) {
        (NS, "auto") => root(state).and_then(|r| crate::bridge::load_config(&r)).and_then(|c| to_json(&kor_runtime::prefs::auto_rows(&c))),
        (NS, "setAuto") => (|| {
            let r = root(state)?;
            let key = s(args, 0, m)?;
            if !kor_config::AUTO_KEYS.iter().any(|(k, _)| *k == key) {
                return Err(format!("Bilinmeyen otomasyon: {key}"));
            }
            set_key(&r, &format!("auto.{key}"), if on(args, 1) { "true" } else { "false" })?;
            to_json(&kor_runtime::prefs::auto_rows(&crate::bridge::load_config(&r)?))
        })(),
        (NS, "ui") => root(state).and_then(|r| crate::bridge::load_config(&r)).map(|c| kor_runtime::prefs::ui_json(&c.config)),
        (NS, "setUi") => (|| {
            let r = root(state)?;
            let name = s(args, 0, m)?;
            let key = kor_runtime::prefs::UI_KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k).ok_or(format!("Bilinmeyen görünüm ayarı: {name}"))?;
            let value = args.get(1).ok_or("Değer gerekli")?;
            // `lineHeight` arrives as 1.5 and is stored ×100.
            let raw = if name == "lineHeight" {
                let f = value.as_f64().ok_or("Sayı gerekli")?;
                format!("{}", (f * 100.0).round().clamp(110.0, 220.0) as u16)
            } else {
                scalar(value)?
            };
            set_key(&r, key, &raw)?;
            Ok(kor_runtime::prefs::ui_json(&crate::bridge::load_config(&r)?.config))
        })(),
        (NS, "agent") => root(state).and_then(|r| crate::bridge::load_config(&r)).map(|c| {
            let t = match c.config.agent.ask_threshold {
                kor_config::AskThreshold::Never => "never",
                kor_config::AskThreshold::Critical => "critical",
                kor_config::AskThreshold::Often => "often",
            };
            json!({ "askThreshold": t, "compactConfirm": c.config.context.compact_confirm, "planLock": c.config.plan.lock })
        }),
        (NS, "setAgent") => (|| {
            let r = root(state)?;
            let (key, raw) = match s(args, 0, m)? {
                "askThreshold" => {
                    let v = s(args, 1, m)?;
                    if !["never", "critical", "often"].contains(&v) {
                        return Err("never, critical ya da often".into());
                    }
                    ("agent.ask_threshold", format!("\"{v}\""))
                }
                "compactConfirm" => ("context.compact_confirm", on(args, 1).to_string()),
                "planLock" => ("plan.lock", on(args, 1).to_string()),
                other => return Err(format!("Bilinmeyen ayar: {other}")),
            };
            set_key(&r, key, &raw)?;
            Ok(json!({ "ok": true }))
        })(),
        (NS, "keymap") => root(state).and_then(|r| to_json(&kor_runtime::prefs::load_keymap(&r))),
        (NS, "answer") => s(args, 0, m).and_then(|id| {
            state.chats.answer(id, args.get(1).cloned().unwrap_or(Value::Null)).map(|()| json!({ "ok": true }))
        }),
        (NS, "doctorFix") => doctor_fix(app).await,

        ("plan", "list") => root(state).map(|r| Value::Array(kor_tools::plans::list(&r).iter().map(plan_json).collect())),
        ("plan", "show") => (|| {
            let r = root(state)?;
            kor_tools::plans::find(&r, s(args, 0, m)?).map(|p| plan_json(&p)).ok_or_else(|| "Plan bulunamadı".to_owned())
        })(),
        ("plan", "approve") => (|| {
            let r = root(state)?;
            let id = match opt_s(args, 0) {
                Some(id) => id.to_owned(),
                None => kor_tools::plans::latest_draft(&r, None).ok_or("Onay bekleyen plan yok")?.id,
            };
            let p = kor_tools::plans::approve(&r, &id).map_err(|e| format!("Plan onaylanamadı: {e}"))?;
            let _ = app.emit("kor:plan/approved", plan_json(&p));
            Ok(plan_json(&p))
        })(),

        ("sessions", _) => sessions(name, args, m, app),
        _ => return None,
    };
    Some(r)
}

fn sessions(name: &str, args: &[Value], m: &str, app: &AppHandle) -> Res {
    use kor_runtime::session_tree as tree;
    let (sd, md) = tree_dirs()?;
    let err = |e: tree::TreeError| e.to_string();
    let changed = |r: Result<(), tree::TreeError>| {
        r.map_err(err)?;
        let _ = app.emit("kor:sessionsChanged", Value::Null);
        Ok(json!({ "ok": true }))
    };
    match name {
        "list" => {
            let f = args.first().cloned().unwrap_or(Value::Null);
            let filter = tree::Filter {
                tag: f["tag"].as_str().map(str::to_owned),
                folder: f["folder"].as_str().map(str::to_owned),
                pinned: f["pinned"].as_bool().unwrap_or(false),
                archived: f["archived"].as_bool().unwrap_or(false),
            };
            to_json(&tree::list(&sd, &md, &filter).map_err(err)?)
        }
        "search" => {
            let q = s(args, 0, m)?;
            if q.len() > 300 {
                return Err("Arama metni çok uzun".into());
            }
            // Only the local embedding model: transcripts never leave the machine.
            let model = kor_memory::LocalEmbedder::installed();
            to_json(&tree::search(&sd, &md, q, 30, model.as_deref().map(|e| e as &dyn kor_memory::Embed)).map_err(err)?)
        }
        "mv" => changed(tree::move_to(&sd, &md, s(args, 0, m)?, opt_s(args, 1))),
        "pin" => changed(tree::pin(&sd, &md, s(args, 0, m)?, on(args, 1))),
        "archive" => changed(tree::archive(&sd, &md, s(args, 0, m)?, on(args, 1))),
        "tag" => {
            let id = s(args, 0, m)?;
            let tags = tree::tag(&sd, &md, id, &strings(args.get(1)), &strings(args.get(2))).map_err(err)?;
            let _ = app.emit("kor:sessionsChanged", Value::Null);
            Ok(json!(tags))
        }
        "color" => changed(tree::color(&sd, &md, s(args, 0, m)?, opt_s(args, 1))),
        "rename" => changed(tree::rename(&sd, &md, s(args, 0, m)?, opt_s(args, 1))),
        "suggest" => {
            let id = s(args, 0, m)?;
            if !tree::valid_id(id) {
                return Err("Geçersiz oturum".into());
            }
            let msgs = kor_agent::session::load(&sd.join(format!("{id}.jsonl"))).map_err(|_| "Oturum bulunamadı".to_owned())?;
            let sug = tree::suggest(&msgs);
            if on(args, 1) && args.get(1).is_some() {
                tree::rename(&sd, &md, id, Some(&sug.title)).map_err(err)?;
                tree::tag(&sd, &md, id, &sug.tags, &[]).map_err(err)?;
                let _ = app.emit("kor:sessionsChanged", Value::Null);
            }
            to_json(&sug)
        }
        "export" => {
            let dir = match opt_s(args, 0) {
                Some(d) => PathBuf::from(d),
                None => dirs::document_dir().ok_or("Belgeler klasörü yok")?.join("Kor").join("Oturumlar"),
            };
            let n = tree::export_markdown(&sd, &md, &dir).map_err(err)?;
            Ok(json!({ "count": n, "dir": dir.to_string_lossy() }))
        }
        "import" => {
            let id = tree::import_markdown(&sd, &md, Path::new(s(args, 0, m)?)).map_err(err)?;
            let _ = app.emit("kor:sessionsChanged", Value::Null);
            Ok(json!({ "id": id }))
        }
        other => Err(format!("`sessions.{other}` yok")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_are_quoted_and_checked() {
        assert_eq!(scalar(&json!(true)).unwrap(), "true");
        assert_eq!(scalar(&json!(16)).unwrap(), "16");
        assert_eq!(scalar(&json!("wide")).unwrap(), "\"wide\"");
        assert!(scalar(&json!("a\"b")).is_err());
        assert!(scalar(&json!({"x": 1})).is_err());
        assert_eq!(strings(Some(&json!(["a", 1, "b"]))), ["a", "b"]);
        let p = kor_tools::plans::Plan {
            id: "2026-11-10-x".into(),
            path: PathBuf::from("p.md"),
            title: "X".into(),
            status: kor_tools::plans::Status::Draft,
            session: None,
            created: 1,
            body: "edit src/a.rs".into(),
        };
        assert_eq!(plan_json(&p)["files"], json!(["src/a.rs"]));
    }
}
