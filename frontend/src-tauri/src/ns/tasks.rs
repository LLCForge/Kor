//! `tasks.*`: `kor task` için ince köprü (§8.2, parity.toml, ADR 0037). İş mantığı
//! `kor_runtime::task` içinde. Çalışan görevler arka planda yürür; her denetim noktası
//! `kor:tasks/progress` olayıyla (`TaskState`) yayılır.
//!
//! - `tasks.list()` → `TaskState[]` (açık klasörün git deposu; depo değilse `[]`)
//! - `tasks.status(id)` → `TaskState`
//! - `tasks.start({ goal, agents?, parallel?, budgetUsd?, test?, models?: { rol: model } })` → `{ id }`
//! - `tasks.resume(id)` → `{ started }`
//! - `tasks.resolve(id, index, "replan" | "drop")` → `{ started }` (index 0 tabanlı)
//! - `tasks.fork(id)` → `TaskState`
//! - `tasks.cancel(id)` → çalışıyorsa durdurur ve bitince temizler `{ cancelling: true }`, değilse `TaskState`

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use kor_providers::CancellationToken;
use kor_runtime::task::{self, Engine, Resolution, RuntimeSpawner, Settings, TaskState};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `tasks::call`'a yönlendirir).
pub const NS: &str = "tasks";

/// Bu süreçte çalışan görevler: iptal anahtarı ve "bitince temizle" isteği.
struct Job {
    cancel: CancellationToken,
    cleanup: bool,
}

static RUNNING: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(Mutex::default);

fn running() -> std::sync::MutexGuard<'static, HashMap<String, Job>> {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn repo(state: &AppState) -> Result<PathBuf, String> {
    task::git::repo_root(&root(state)?).map_err(|_| "Açık klasör bir git deposu değil".to_owned())
}

fn to_value(st: &TaskState) -> Res {
    serde_json::to_value(st).map_err(|e| e.to_string())
}

/// `{ goal, agents?, parallel?, budgetUsd?, test?, models? }` → hedef ve ayarlar.
fn settings_from(o: &Value, base: &kor_config::Orchestrator) -> Result<(String, Settings), String> {
    let goal = o["goal"].as_str().unwrap_or_default().trim().to_owned();
    if goal.is_empty() {
        return Err("Görev hedefi boş".into());
    }
    if goal.chars().count() > 8_000 {
        return Err("Görev hedefi çok uzun".into());
    }
    let mut st = Settings::from_config(base);
    let small = |k: &str| {
        o[k].as_u64()
            .filter(|n| (1..=16).contains(n))
            .and_then(|n| usize::try_from(n).ok())
    };
    if let Some(n) = small("agents") {
        st.agents = n;
    }
    if let Some(n) = small("parallel") {
        st.max_parallel = n;
    }
    if let Some(b) = o["budgetUsd"]
        .as_f64()
        .filter(|b| b.is_finite() && *b >= 0.0)
    {
        st.budget_usd = b;
    }
    if let Some(t) = o["test"].as_str() {
        t.trim().clone_into(&mut st.test_command);
    }
    if let Some(m) = o["models"].as_object() {
        for (role, model) in m.iter().take(16) {
            let Some(model) = model.as_str().map(str::trim).filter(|m| !m.is_empty()) else {
                continue;
            };
            if model.parse::<kor_core::ModelRef>().is_err() {
                return Err(format!("Geçersiz model: {model}"));
            }
            st.models.insert(task::slug(role), model.to_owned());
        }
    }
    Ok((goal, st))
}

fn engine(app: &AppHandle, root: PathBuf, loaded: &kor_config::Loaded) -> Result<Engine, String> {
    let lang = loaded.config.ui.language.resolve();
    let l = kor_i18n::Localizer::new(lang);
    let (orchestrator, reviewer) = task::models(&l, loaded, None)?;
    let app = app.clone();
    Ok(Engine {
        root,
        lang,
        orchestrator,
        reviewer,
        spawner: Arc::new(RuntimeSpawner::new(lang, loaded)),
        progress: Arc::new(move |st: &TaskState| {
            let _ = app.emit("kor:tasks/progress", st);
        }),
    })
}

enum Step {
    Run,
    Resolve(usize, Resolution),
}

/// Görevi arka planda yürüt; bitince (iptal istendiyse) çalışma ağaçlarını temizle.
fn spawn(app: &AppHandle, engine: Engine, mut st: TaskState, step: Step) -> Res {
    let id = st.id.clone();
    let cancel = CancellationToken::new();
    {
        let mut jobs = running();
        if jobs.contains_key(&id) {
            return Err(format!("#{id} zaten çalışıyor"));
        }
        jobs.insert(
            id.clone(),
            Job {
                cancel: cancel.clone(),
                cleanup: false,
            },
        );
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let r = match step {
            Step::Run => engine.run(&mut st, &cancel).await,
            Step::Resolve(i, how) => engine.resolve(&mut st, i, how, &cancel).await,
        };
        if let Err(e) = r {
            st.error = Some(e);
            let _ = app.emit("kor:tasks/progress", &st);
        }
        let cleanup = running().remove(&st.id).is_some_and(|j| j.cleanup);
        if cleanup && let Ok(done) = task::cancel(&engine.root, &st.id) {
            let _ = app.emit("kor:tasks/progress", &done);
        }
    });
    Ok(json!({ "id": id, "started": true }))
}

fn start(o: &Value, app: &AppHandle, state: &AppState) -> Res {
    let dir = root(state)?;
    let loaded = crate::bridge::load_config(&dir)?;
    let (goal, settings) = settings_from(o, &loaded.config.orchestrator)?;
    let (repo, st) = task::create(&dir, &goal, settings)?;
    let e = engine(app, repo, &loaded)?;
    spawn(app, e, st, Step::Run)
}

fn resume(id: &str, step: Step, app: &AppHandle, state: &AppState) -> Res {
    let repo = repo(state)?;
    let loaded = crate::bridge::load_config(&root(state)?)?;
    let st = task::load(&repo, id)?;
    let e = engine(app, repo, &loaded)?;
    spawn(app, e, st, step)
}

fn cancel(id: &str, state: &AppState) -> Res {
    if let Some(job) = running().get_mut(id) {
        job.cleanup = true;
        job.cancel.cancel();
        return Ok(json!({ "cancelling": true }));
    }
    to_value(&task::cancel(&repo(state)?, id)?)
}

fn resolution(v: Option<&Value>) -> Result<Resolution, String> {
    match v.and_then(Value::as_str) {
        Some("replan") => Ok(Resolution::Replan),
        Some("drop") => Ok(Resolution::Drop),
        _ => Err("Çözüm \"replan\" ya da \"drop\" olmalı".into()),
    }
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("tasks.{name}");
    Some(match name {
        "list" => match repo(state) {
            Ok(r) => serde_json::to_value(task::list(&r)).map_err(|e| e.to_string()),
            Err(_) => Ok(json!([])),
        },
        "status" => s(args, 0, &m)
            .and_then(|id| repo(state).and_then(|r| task::load(&r, id)))
            .and_then(|st| to_value(&st)),
        "start" => start(args.first().unwrap_or(&Value::Null), app, state),
        "resume" => s(args, 0, &m).and_then(|id| resume(id, Step::Run, app, state)),
        "resolve" => s(args, 0, &m).and_then(|id| {
            let index = args
                .get(1)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| format!("`{m}`: 2. bağımsız değişken sayı olmalı"))?;
            let how = resolution(args.get(2))?;
            resume(id, Step::Resolve(index, how), app, state)
        }),
        "fork" => s(args, 0, &m)
            .and_then(|id| repo(state).and_then(|r| task::fork(&r, id)))
            .and_then(|st| to_value(&st)),
        "cancel" => s(args, 0, &m).and_then(|id| cancel(id, state)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_options_are_validated() {
        let base = kor_config::Orchestrator::default();
        let (goal, s) = settings_from(
            &json!({ "goal": " auth ", "agents": 3, "parallel": 99, "budgetUsd": 2.5,
                     "test": " cargo test ", "models": { "Back End": "openai/gpt-4.1", "docs": "" } }),
            &base,
        )
        .unwrap();
        assert_eq!(goal, "auth");
        assert_eq!(s.agents, 3);
        assert_eq!(
            s.max_parallel,
            usize::from(base.max_parallel),
            "out of range ignored"
        );
        assert!((s.budget_usd - 2.5).abs() < 1e-9);
        assert_eq!(s.test_command, "cargo test");
        assert_eq!(
            s.models.get("back-end").map(String::as_str),
            Some("openai/gpt-4.1")
        );
        assert!(!s.models.contains_key("docs"));
        assert!(settings_from(&json!({ "goal": "" }), &base).is_err());
        assert!(settings_from(&json!({ "goal": "x", "models": { "a": "nope" } }), &base).is_err());
        assert!(resolution(Some(&json!("drop"))).is_ok());
        assert!(resolution(Some(&json!("merge"))).is_err());
    }
}
