//! `gitflow.*`: `kor commit` ve `kor changelog` için ince köprü (§15.5, parity.toml). İş mantığı
//! `kor_agent::gitflow` içinde; burası yalnızca JSON'a çevirir. Kabuk entegrasyonu (`kor shell`,
//! `kor fix`, `kor why`) `ns/shell.rs` içindedir.
//!
//! - `gitflow.commitMessage(split?: bool, model?: string)` → `{ files, scope, message, header,
//!   breaking, parts: [{ files, message }] }`. Hiçbir şey commit edilmez.
//! - `gitflow.commit(message: string)` → `{ summary }`. Arayüzdeki onay düğmesi onaydır.
//! - `gitflow.changelog(since?: string, write?: bool)` → `{ section, since, path? }`.

use kor_agent::gitflow::{self, FlowError};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `gitflow::call`'a yönlendirir).
pub const NS: &str = "gitflow";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "commitMessage" => commit_message(state, b(args, 0), opt_s(args, 1)).await,
        "commit" => match s(args, 0, "gitflow.commit") {
            Ok(m) => commit(state, m),
            Err(e) => Err(e),
        },
        "changelog" => changelog(state, opt_s(args, 0), b(args, 1)),
        _ => return None,
    })
}

fn tr(e: &FlowError) -> String {
    match e {
        FlowError::GitMissing => "git bulunamadı (PATH'te yok).".into(),
        FlowError::NotRepo => "Bu klasör bir git deposu değil.".into(),
        FlowError::NothingStaged => {
            "Hazırlanmış (staged) değişiklik yok. Önce `git add` ile ekleyin.".into()
        }
        FlowError::Git(e) => format!("git hatası: {e}"),
        FlowError::Model(e) | FlowError::BadReply(e) => format!("Model yanıtı kullanılamadı: {e}"),
        FlowError::HookExists(p) => {
            format!("{} zaten var ve Kor'un değil; dokunulmadı.", p.display())
        }
    }
}

async fn commit_message(state: &AppState, split: bool, model: Option<&str>) -> Res {
    let dir = root(state)?;
    let loaded = crate::bridge::load_config(&dir)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    // git sorunları model seçilmeden önce bildirilir.
    gitflow::repo_root(&dir)
        .and_then(|r| gitflow::staged(&r))
        .map_err(|e| tr(&e))?;
    let (provider, model) = kor_runtime::oneshot_provider(&l, &loaded, model)?;
    let p = gitflow::propose(&provider, &model, &dir, split)
        .await
        .map_err(|e| tr(&e))?;
    let parts: Vec<Value> = p
        .parts
        .iter()
        .map(|x| json!({ "files": x.files, "message": x.message.render() }))
        .collect();
    Ok(json!({
        "files": p.files,
        "scope": p.scope,
        "message": p.message.render(),
        "header": p.message.header(),
        "breaking": p.message.breaking,
        "parts": parts,
    }))
}

fn commit(state: &AppState, message: &str) -> Res {
    if message.trim().is_empty() || message.len() > 20_000 {
        return Err("Commit mesajı 1-20000 karakter olmalı".into());
    }
    let dir = root(state)?;
    let summary = gitflow::repo_root(&dir)
        .and_then(|r| gitflow::commit(&r, message))
        .map_err(|e| tr(&e))?;
    Ok(json!({ "summary": summary }))
}

fn changelog(state: &AppState, since: Option<&str>, write: bool) -> Res {
    let repo = gitflow::repo_root(&root(state)?).map_err(|e| tr(&e))?;
    let since = since
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| gitflow::latest_tag(&repo));
    let entries = gitflow::log(&repo, since.as_deref()).map_err(|e| tr(&e))?;
    let section = gitflow::section(&entries, "Unreleased");
    if !write || entries.is_empty() {
        return Ok(json!({ "section": section, "since": since, "count": entries.len() }));
    }
    let path = repo.join("CHANGELOG.md");
    let old = match std::fs::read_to_string(&path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    std::fs::write(&path, gitflow::merge_changelog(old.as_deref(), &section))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({
        "section": section,
        "since": since,
        "count": entries.len(),
        "path": path.display().to_string(),
    }))
}
