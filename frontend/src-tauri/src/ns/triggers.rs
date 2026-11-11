//! `triggers.*`: `kor triggers` ve `kor daemon` için ince köprü (§8.4, ADR 0039). İş mantığı
//! `kor_runtime::triggers` içinde; daemon ayrı bir süreçtir (`kor daemon run`), masaüstü onu
//! başlatır, durdurur ve denetim ucuyla konuşur.
//!
//! - `triggers.list()` → `[{ name, event, on, task, root, origin, enabled, model, budgetUsd, approval, pr, …, error }]`
//!   (açık proje izlenmeye başlar)
//! - `triggers.save(trigger, scope)` → `scope` `"project"` (açık projenin `.kor/config.toml`) ya da `"user"`
//! - `triggers.remove(name, origin)` → bool
//! - `triggers.test(name)` → `{ job }` (daemon çalışıyorsa onda, yoksa bu süreçte arka planda)
//! - `triggers.history(name?, limit?)` → `Job[]`
//! - `triggers.answer(id, approve)` → bool
//! - `triggers.hookInstall(repo?)` / `triggers.hookUninstall(repo?)` → yol / bool
//! - `triggers.daemonStatus()` / `daemonStart()` / `daemonStop()` / `daemonLogs(n?)`
//! - `triggers.notifyGet()` → `[notify]` + kasadaki sırların varlığı; `notifySet(obj)`; `notifyTest()`

use std::path::{Path, PathBuf};

use kor_runtime::triggers::{self, daemon, hook, notify};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `triggers::call`'a yönlendirir).
pub const NS: &str = "triggers";

fn dir() -> Result<PathBuf, String> {
    triggers::dir().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

fn user_config() -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("config.toml"))
        .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

fn l(state: &AppState) -> kor_i18n::Localizer {
    let lang = root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map_or(kor_i18n::Lang::Tr, |c| c.config.ui.language.resolve());
    kor_i18n::Localizer::new(lang)
}

fn defs(state: &AppState, dir: &Path) -> Vec<triggers::Defined> {
    if let Ok(r) = root(state)
        && triggers::project_config(&r).is_file()
    {
        let _ = triggers::register(dir, &r);
    }
    let user = user_config().ok();
    triggers::collect(&l(state), user.as_deref(), &triggers::projects(dir))
}

fn str_of(v: &Value, k: &str) -> String {
    v[k].as_str()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(10_000)
        .collect()
}

/// Arayüzden gelen nesne → `Trigger` (doğrulama `save_trigger` içinde).
fn trigger_from(v: &Value) -> Result<kor_config::Trigger, String> {
    let event = serde_json::from_value(v["event"].clone()).map_err(|_| "Geçersiz olay türü")?;
    let approval = match v["approval"].as_str().unwrap_or("notify") {
        "auto" => kor_config::Approval::Auto,
        "deny" => kor_config::Approval::Deny,
        _ => kor_config::Approval::Notify,
    };
    Ok(kor_config::Trigger {
        event,
        task: str_of(v, "task"),
        name: str_of(v, "name"),
        enabled: v["enabled"].as_bool().unwrap_or(true),
        branch: str_of(v, "branch"),
        schedule: str_of(v, "schedule"),
        path: str_of(v, "path"),
        secret: str_of(v, "secret"),
        source: str_of(v, "source"),
        pattern: str_of(v, "pattern"),
        debounce_secs: v["debounceSecs"].as_u64().unwrap_or(0).min(86_400),
        model: str_of(v, "model"),
        budget_usd: v["budgetUsd"]
            .as_f64()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .unwrap_or(0.0),
        root: str_of(v, "root"),
        approval,
        pr: v["pr"].as_bool().unwrap_or(false),
    })
}

/// Masaüstünün yanındaki `kor`, yoksa PATH'teki.
fn kor_exe() -> PathBuf {
    crate::kor_cli::path()
}

fn secret_present(cfg: &kor_config::Vault, name: &str) -> bool {
    (daemon::vault_secrets(cfg))(name).is_some()
}

async fn notify_get(state: &AppState) -> Res {
    let loaded = crate::bridge::load_config(&root(state)?)?;
    let c = &loaded.config;
    let mut v = serde_json::to_value(&c.notify).map_err(|e| e.to_string())?;
    v["publicUrl"] = json!(c.daemon.public_url);
    v["listen"] = json!(c.daemon.listen);
    v["secrets"] = json!({
        notify::TELEGRAM_TOKEN: secret_present(&c.vault, notify::TELEGRAM_TOKEN),
        notify::SLACK_WEBHOOK: secret_present(&c.vault, notify::SLACK_WEBHOOK),
        notify::SMTP_PASSWORD: secret_present(&c.vault, notify::SMTP_PASSWORD),
    });
    Ok(v)
}

fn notify_set(o: &Value) -> Res {
    let file = user_config()?;
    let set =
        |k: &str, raw: String| kor_config::set_scalar(&file, k, &raw).map_err(|e| e.to_string());
    for k in [
        "telegram_chat",
        "email_to",
        "email_from",
        "smtp_host",
        "smtp_user",
    ] {
        if let Some(v) = o[k].as_str() {
            kor_config::set_string(&file, &format!("notify.{k}"), v.trim())
                .map_err(|e| e.to_string())?;
        }
    }
    if let Some(v) = o["slack"].as_bool() {
        set("notify.slack", v.to_string())?;
    }
    if let Some(p) = o["smtp_port"].as_u64().filter(|p| (1..=65_535).contains(p)) {
        set("notify.smtp_port", p.to_string())?;
    }
    if let Some(m) = o["approval_timeout_mins"]
        .as_u64()
        .filter(|m| (1..=10_080).contains(m))
    {
        set("notify.approval_timeout_mins", m.to_string())?;
    }
    if let Some(u) = o["publicUrl"].as_str() {
        let u = u.trim();
        if !u.is_empty()
            && !u.starts_with("https://")
            && !u.starts_with("http://127.0.0.1")
            && !u.starts_with("http://localhost")
        {
            return Err("Genel adres https:// ile başlamalı".into());
        }
        kor_config::set_string(&file, "daemon.public_url", u).map_err(|e| e.to_string())?;
    }
    Ok(json!(true))
}

async fn notify_test(state: &AppState) -> Res {
    let loaded = crate::bridge::load_config(&root(state)?)?;
    let l = l(state);
    let secrets = daemon::vault_secrets(&loaded.config.vault);
    let mut notes = Vec::new();
    let ns = daemon::notifiers(&l, &loaded.config.notify, &secrets, &mut notes);
    if ns.is_empty() {
        return Err(if notes.is_empty() {
            "Yapılandırılmış bildirim kanalı yok".into()
        } else {
            notes.join("\n")
        });
    }
    let note = notify::Note {
        title: "Kor".into(),
        text: "Kor bildirim denemesi".into(),
        ask: None,
    };
    let errors = notify::broadcast(&ns, &note).await;
    Ok(
        json!({ "sent": ns.iter().map(|n| n.name()).collect::<Vec<_>>(), "errors": errors, "notes": notes }),
    )
}

async fn test(name: &str, state: &AppState) -> Res {
    let dir = dir()?;
    if daemon::state(&dir).is_some() {
        return daemon::control(&dir, "POST", "/control/test", Some(json!({ "name": name }))).await;
    }
    let all = defs(state, &dir);
    let loaded = crate::bridge::load_config(&root(state)?)?;
    let lang = l(state).lang();
    let mut notes = Vec::new();
    let opts = daemon::options(
        &kor_i18n::Localizer::new(lang),
        &loaded.config,
        None,
        false,
        &mut notes,
    )?;
    let env = triggers::runner::Env {
        l: kor_i18n::Localizer::new(lang),
        dir,
        hub: std::sync::Arc::default(),
        notifiers: std::sync::Arc::new(opts.notifiers),
        public_url: None,
        approval_timeout: opts.approval_timeout,
        factory: opts.factory,
    };
    let name = name.to_owned();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = daemon::run_once(&env, &all, &name).await {
            eprintln!("kor: tetikleyici denemesi başarısız ({name}): {e}");
        }
    });
    Ok(json!({ "started": true }))
}

fn repo_arg(args: &[Value], state: &AppState) -> Result<PathBuf, String> {
    opt_s(args, 0)
        .filter(|r| !r.is_empty())
        .map_or_else(|| root(state), |r| Ok(PathBuf::from(r)))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = || format!("{NS}.{name}");
    Some(match name {
        "list" => dir().map(|d| {
            json!(
                defs(state, &d)
                    .iter()
                    .map(triggers::describe)
                    .collect::<Vec<_>>()
            )
        }),
        "save" => {
            async {
                let t = trigger_from(args.first().ok_or("Tetikleyici gerekli")?)?;
                let file = match opt_s(args, 1) {
                    Some("user") => user_config()?,
                    _ => {
                        let r = root(state)?;
                        triggers::register(&dir()?, &r)?;
                        triggers::project_config(&r)
                    }
                };
                triggers::save_trigger(&l(state), &file, &t).map(|()| json!(true))
            }
            .await
        }
        "remove" => {
            async {
                let name = s(args, 0, &m())?;
                let origin = PathBuf::from(s(args, 1, &m())?);
                // Yalnızca bilinen ayar dosyaları: kullanıcı ayarı ya da izlenen bir projenin dosyası.
                let d = dir()?;
                let known = user_config().is_ok_and(|u| u == origin)
                    || triggers::projects(&d)
                        .iter()
                        .any(|p| triggers::project_config(p) == origin);
                if !known {
                    return Err("Bilinmeyen ayar dosyası".into());
                }
                triggers::remove_trigger(&origin, name).map(|ok| json!(ok))
            }
            .await
        }
        "test" => match s(args, 0, &m()) {
            Ok(n) => test(n, state).await,
            Err(e) => Err(e),
        },
        "history" => dir().map(|d| {
            let limit = args
                .get(1)
                .and_then(Value::as_u64)
                .map_or(50, |n| usize::try_from(n).unwrap_or(50).clamp(1, 500));
            json!(triggers::history(
                &d,
                opt_s(args, 0).filter(|s| !s.is_empty()),
                limit
            ))
        }),
        "answer" => {
            async {
                let id = s(args, 0, &m())?;
                let v = daemon::control(
                    &dir()?,
                    "POST",
                    "/control/answer",
                    Some(json!({ "id": id, "approve": b(args, 1) })),
                )
                .await?;
                Ok(json!(v["ok"] == true))
            }
            .await
        }
        "hookInstall" => {
            async {
                let d = dir()?;
                let repo = repo_arg(args, state)?;
                let p = hook::install(&repo, &daemon::inbox(&d))?;
                if let Ok(r) = root(state)
                    && triggers::project_config(&r).is_file()
                {
                    let _ = triggers::register(&d, &r);
                }
                Ok(json!(p.display().to_string()))
            }
            .await
        }
        "hookUninstall" => repo_arg(args, state)
            .and_then(|r| hook::uninstall(&r))
            .map(|ok| json!(ok)),
        "daemonStatus" => match dir() {
            Ok(d) => Ok(daemon::status(&d).await),
            Err(e) => Err(e),
        },
        "daemonStart" => {
            async {
                let d = dir()?;
                if let Ok(r) = root(state)
                    && triggers::project_config(&r).is_file()
                {
                    let _ = triggers::register(&d, &r);
                }
                let exe = kor_exe();
                let st = tauri::async_runtime::spawn_blocking(move || {
                    daemon::spawn_background(&exe, &[])
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(|log| format!("kor daemon başlamadı; günlük: {log}"))?;
                Ok(json!({ "running": true, "addr": st.addr, "pid": st.pid }))
            }
            .await
        }
        "daemonStop" => match dir() {
            Ok(d) => Ok(json!(daemon::stop_daemon(&d).await)),
            Err(e) => Err(e),
        },
        "daemonLogs" => dir().map(|d| {
            let n = args
                .first()
                .and_then(Value::as_u64)
                .map_or(200, |n| usize::try_from(n).unwrap_or(200).clamp(1, 5000));
            json!(daemon::log_tail(&d, n))
        }),
        // Oturum açılışında başlatma: yalnızca kullanıcının açık isteğiyle (kullanıcı düzeyi, yönetici değil).
        "daemonInstall" => kor_platform::service::install(&kor_exe()).map(|w| json!(w)).map_err(|e| e.to_string()),
        "daemonUninstall" => kor_platform::service::uninstall().map(|()| json!(true)).map_err(|e| e.to_string()),
        "notifyGet" => notify_get(state).await,
        "notifySet" => match args.first() {
            Some(o) if o.is_object() => notify_set(o),
            _ => Err(format!("`{}`: nesne gerekli", m())),
        },
        "notifyTest" => notify_test(state).await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_from_renderer_object() {
        let t = trigger_from(&json!({
            "event": "git_push", "name": " ci ", "task": "testleri çalıştır", "branch": "main",
            "approval": "auto", "pr": true, "budgetUsd": -3, "debounceSecs": 999_999
        }))
        .unwrap();
        assert_eq!(t.event, kor_config::TriggerEvent::GitPush);
        assert_eq!((t.name.as_str(), t.branch.as_str()), ("ci", "main"));
        assert_eq!(t.approval, kor_config::Approval::Auto);
        assert!(t.pr && t.enabled);
        assert!(t.budget_usd.abs() < f64::EPSILON, "negative budget dropped");
        assert_eq!(t.debounce_secs, 86_400);
        assert!(trigger_from(&json!({ "event": "nope", "task": "x" })).is_err());
    }

    #[test]
    fn kor_exe_has_a_name() {
        assert!(kor_exe().to_string_lossy().contains("kor"));
    }
}
