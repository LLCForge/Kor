//! `intent.*`: doğal dil komut yönlendirme için ince köprü (§30, ADR 0056). Yönlendirme
//! `kor_runtime::intent` içinde; yazan/maliyetli/ağa çıkan komutların onayı burada, arka uçta
//! istenir: arayüz bu adımı atlayamaz. Çalıştırma, CLI ile aynı kodu kullanmak için paketlenmiş
//! `kor` ikilisini çağırır.
//! Sözleşme: `frontend/src/desktop/ipc-intent.ts`.
//!
//! - `intent.route(text, { selection?, tabUrl? })` → `Decision` (`{ kind: 'agent' }` kapalıyken)
//! - `intent.run(command, argv)` → `{ code, output, line }` (onay gerekiyorsa önce yerel pencere)
//! - `intent.schedule(prompt, intervalMinutes, firstRun)` → rutin (önce yerel onay)
//! - `intent.list()` → `Intent[]`
//! - `intent.learn(text, command)` / `intent.forget(text)` → `{ ok }`

use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_runtime::intent::{Context, Fallbacks, Router, Run};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

pub const NS: &str = "intent";

const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_OUTPUT: usize = 200_000;
const MAX_TEXT: usize = 4000;

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn kor_exe() -> PathBuf {
    crate::kor_cli::path()
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "route" => route(&root, args).await,
        "run" => run(&root, args, app).await,
        "schedule" => schedule(&root, args, app).await,
        "list" => list(&root),
        "learn" => learn(&root, args),
        "forget" => forget(&root, args),
        _ => return None,
    })
}

fn text_arg<'a>(args: &'a [Value], i: usize, method: &str) -> Result<&'a str, String> {
    let t = s(args, i, method)?.trim();
    if t.is_empty() || t.len() > MAX_TEXT {
        return Err(format!("`{method}`: metin 1-{MAX_TEXT} karakter olmalı"));
    }
    Ok(t)
}

async fn route(root: &Path, args: &[Value]) -> Res {
    let text = text_arg(args, 0, "intent.route")?;
    let loaded = crate::bridge::load_config(root)?;
    if !loaded.config.intent.enabled {
        return Ok(json!({ "kind": "agent" }));
    }
    let opts = args.get(1).cloned().unwrap_or(Value::Null);
    let field = |k: &str| {
        opts.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .map(str::to_owned)
    };
    let model = loaded.config.agent.model.trim().to_owned();
    let ctx = Context {
        selection: field("selection"),
        tab_url: field("tabUrl").filter(|u| u.starts_with("http://") || u.starts_with("https://")),
        model: (!model.is_empty()).then_some(model),
        confirm_all: !loaded.config.intent.auto_run,
        ..Context::default()
    };
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let router = Router::load(Some(root));
    let embedder = kor_agent::semantic::local_embedder();
    let fb = Fallbacks {
        embedder: embedder
            .as_ref()
            .map(|e| e as &dyn kor_agent::semantic::Embedder),
        model: kor_runtime::intent::fallback_model(&l, &loaded),
    };
    to_json(&router.route_full(text, &ctx, &fb).await)
}

/// The argv must be the catalog command's own words followed by plain values (or the declared
/// flags of its parameters): a renderer cannot smuggle extra options through this method.
pub(crate) fn check_argv(router: &Router, command: &str, argv: &[String]) -> Result<Run, String> {
    let i = router
        .catalog
        .get(command)
        .filter(|i| i.run != Run::Never)
        .ok_or("Bilinmeyen komut")?;
    if !command.starts_with("kor") {
        return Err("Eklenti komutları sohbet içinden çalışır".into());
    }
    let words: Vec<&str> = command.split_whitespace().skip(1).collect();
    if argv.len() < words.len() || argv.iter().zip(&words).any(|(a, w)| a != w) {
        return Err("Komut satırı katalogdakiyle eşleşmiyor".into());
    }
    let flags: Vec<&str> = i
        .params
        .iter()
        .filter_map(|p| p.flag.as_deref())
        .chain(words.iter().copied().filter(|w| w.starts_with('-')))
        .collect();
    for a in &argv[words.len()..] {
        if a.len() > 4096 || a.contains(['\0', '\n', '\r']) {
            return Err("Geçersiz argüman".into());
        }
        if a.starts_with('-') && !flags.contains(&a.as_str()) {
            return Err(format!("İzin verilmeyen seçenek: {a}"));
        }
    }
    if argv.len() - words.len() > i.params.len() * 2 {
        return Err("Fazla argüman".into());
    }
    Ok(i.run)
}

async fn run(root: &Path, args: &[Value], app: &AppHandle) -> Res {
    let command = s(args, 0, "intent.run")?;
    let argv: Vec<String> = args
        .get(1)
        .and_then(Value::as_array)
        .ok_or("`intent.run`: argv listesi gerekli")?
        .iter()
        .map(|v| v.as_str().map(str::to_owned).ok_or("argv metin olmalı"))
        .collect::<Result<_, _>>()?;
    let mut router = Router::load(Some(root));
    let class = check_argv(&router, command, &argv)?;
    let line = format!("kor {}", argv.join(" "));
    let loaded = crate::bridge::load_config(root)?;
    if (class == Run::Confirm || !loaded.config.intent.auto_run)
        && !crate::api::confirm(app, "Kor komutu çalıştırılsın mı?", &line, "Çalıştır").await
    {
        return Err("Vazgeçildi".into());
    }
    let lang = loaded.config.ui.language.resolve().code();
    let mut cmd = tokio::process::Command::new(kor_exe());
    cmd.args(&argv)
        .args(["--lang", lang])
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("KOR_UI", "line")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    kor_platform::NoWindow::no_window(&mut cmd);
    let out = tokio::time::timeout(RUN_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "Komut zaman aşımına uğradı".to_owned())?
        .map_err(|e| format!("kor çalıştırılamadı: {e}"))?;
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        output.push_str(&err);
    }
    let output: String = output.chars().take(MAX_OUTPUT).collect();
    if let Some(p) = router.plan(
        command,
        &argv.join(" "),
        &Context::default(),
        kor_runtime::intent::Via::Keyword,
        1.0,
    ) {
        router.remember(&p);
    }
    Ok(json!({ "code": out.status.code(), "output": output, "line": line }))
}

async fn schedule(root: &Path, args: &[Value], app: &AppHandle) -> Res {
    let prompt = text_arg(args, 0, "intent.schedule")?;
    let minutes = args
        .get(1)
        .and_then(Value::as_u64)
        .filter(|m| *m > 0)
        .ok_or("Aralık sayı olmalı")?;
    let first = args.get(2).and_then(Value::as_u64).unwrap_or(0);
    let when = kor_runtime::cost::fmt_time(first);
    let body = format!("{prompt}\n\nHer {minutes} dakikada bir · ilk çalışma {when}");
    if !crate::api::confirm(app, "Zamanlanmış görev oluşturulsun mu?", &body, "Oluştur").await
    {
        return Err("Vazgeçildi".into());
    }
    let file = kor_runtime::routines::file().ok_or("Kor klasörü bulunamadı")?;
    let mut m = serde_json::Map::new();
    m.insert("prompt".into(), prompt.into());
    m.insert("intervalMinutes".into(), minutes.into());
    m.insert("root".into(), root.display().to_string().into());
    m.insert("firstRun".into(), first.into());
    kor_runtime::routines::add(&file, &m, kor_runtime::cost::now_ms())
}

fn list(root: &Path) -> Res {
    let router = Router::load(Some(root));
    let intents: Vec<_> = router
        .catalog
        .intents
        .iter()
        .filter(|i| i.run != Run::Never)
        .collect();
    Ok(
        json!({ "intents": intents, "learned": router.learned.patterns, "errors": router.catalog.errors }),
    )
}

fn learn(root: &Path, args: &[Value]) -> Res {
    let text = text_arg(args, 0, "intent.learn")?;
    let command = s(args, 1, "intent.learn")?;
    let mut router = Router::load(Some(root));
    router
        .learn(text, command)
        .map_err(|_| "Bilinmeyen komut".to_owned())?;
    Ok(json!({ "ok": true }))
}

fn forget(root: &Path, args: &[Value]) -> Res {
    let text = text_arg(args, 0, "intent.forget")?;
    let mut router = Router::load(Some(root));
    let removed = router.learned.forget(text);
    if removed && let Some(f) = kor_runtime::intent::learned_file() {
        router.learned.save(&f)?;
    }
    Ok(json!({ "ok": removed }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_must_match_the_catalog_command() {
        let r = Router::new(
            kor_runtime::intent::Catalog::builtin(),
            kor_runtime::intent::Learned::default(),
        );
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(check_argv(&r, "kor cost", &v(&["cost"])), Ok(Run::Auto));
        assert_eq!(check_argv(&r, "kor fix", &v(&["fix"])), Ok(Run::Confirm));
        assert!(check_argv(&r, "kor cost", &v(&["fix"])).is_err());
        assert!(check_argv(&r, "kor cost", &v(&["cost", "--yes"])).is_err());
        assert!(check_argv(&r, "kor explain", &v(&["explain", "src/main.rs"])).is_ok());
        assert!(check_argv(&r, "kor explain", &v(&["explain", "a", "b", "c", "d", "e"])).is_err());
        assert!(check_argv(&r, "kor", &v(&[])).is_err());
        assert!(check_argv(&r, "kor nope", &v(&["nope"])).is_err());
    }
}
