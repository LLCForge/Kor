//! `deploy.*`: `kor deploy` köprüsü (§8.6, ADR 0041). Plan, çalıştırma, sağlık kontrolü ve geri alma
//! `kor_runtime::deploy` içinde; burası ince.
//!
//! - `deploy.envs()` → `{ envs: string[], state: State }` (`.kor/deploy/state.json`)
//! - `deploy.plan(env)` → `Plan` (hiçbir şey çalıştırmaz)
//! - `deploy.run(env, approved: number[], displays: string[], rollback: bool)` → `{ started: true }`.
//!   Kullanıcının gördüğü planın adım metinleri (`displays`) yeniden hesaplanan planla aynı değilse
//!   reddedilir. Yalnızca `approved` içindeki adımlar çalışır; `rollback` = sağlık kontrolü başarısız
//!   olursa geri alma adımlarına önceden onay. Olaylar: `kor:deploy/event` `{ env, event }`,
//!   `kor:deploy/done` `Report`.
//! - `deploy.risk(env)` → `RiskReport`

use std::collections::BTreeSet;
use std::sync::Mutex;

use kor_runtime::deploy::{self, Step, StepKind};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, b, root, s};
use crate::state::AppState;

pub const NS: &str = "deploy";

/// Aynı anda yalnızca bir ortam dağıtılır (ortam adları).
static RUNNING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn ids(args: &[Value], i: usize) -> BTreeSet<usize> {
    args.get(i)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .filter_map(|n| usize::try_from(n).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn displays(args: &[Value], i: usize) -> Vec<String> {
    args.get(i)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Kullanıcının onayladığı adımlar ve önceden verilen geri alma onayı.
fn approver(approved: BTreeSet<usize>, rollback: bool) -> impl FnMut(&Step) -> bool + Send {
    move |st: &Step| {
        if st.kind == StepKind::Rollback {
            rollback
        } else {
            approved.contains(&st.id)
        }
    }
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !matches!(name, "envs" | "plan" | "run" | "risk") {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "envs" => deploy::envs(&root)
            .map(|envs| json!({ "envs": envs, "state": deploy::load_state(&root) })),
        "plan" => match s(args, 0, "deploy.plan") {
            Ok(env) => {
                let (env, root) = (env.to_owned(), root.clone());
                tauri::async_runtime::spawn_blocking(move || deploy::plan(&root, &env))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
                    .and_then(|p| to_json(&p))
            }
            Err(e) => Err(e),
        },
        "risk" => match s(args, 0, "deploy.risk") {
            Ok(env) => {
                let (env, root) = (env.to_owned(), root.clone());
                let now = chrono::Local::now().time();
                tauri::async_runtime::spawn_blocking(move || deploy::risk_check(&root, &env, now))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
                    .and_then(|r| to_json(&r))
            }
            Err(e) => Err(e),
        },
        _ => start(args, app, root).await,
    })
}

async fn start(args: &[Value], app: &AppHandle, root: std::path::PathBuf) -> Res {
    let env = s(args, 0, "deploy.run")?.to_owned();
    let (approved, seen, rollback) = (ids(args, 1), displays(args, 2), b(args, 3));
    let (env2, root2) = (env.clone(), root.clone());
    let plan = tauri::async_runtime::spawn_blocking(move || deploy::plan(&root2, &env2))
        .await
        .map_err(|e| e.to_string())??;
    let now: Vec<String> = plan.steps.iter().map(|st| st.display.clone()).collect();
    if now != seen {
        return Err("Dağıtım planı değişti; planı yeniden inceleyin".into());
    }
    {
        let mut running = RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !running.insert(env.clone()) {
            return Err(format!("{env} için bir dağıtım zaten çalışıyor"));
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut approve = approver(approved, rollback);
        let emit_app = app.clone();
        let env_ev = env.clone();
        let on = move |e: &deploy::Event| {
            let _ = emit_app.emit("kor:deploy/event", json!({ "env": env_ev, "event": e }));
        };
        let report = deploy::run(&root, &plan, &mut approve, &on).await;
        RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&env);
        let _ = app.emit("kor:deploy/done", &report);
    });
    Ok(json!({ "started": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: usize, kind: StepKind) -> Step {
        Step {
            id,
            kind,
            program: "docker".into(),
            args: Vec::new(),
            env: Vec::new(),
            display: String::new(),
            approve: true,
            destructive: false,
            service: None,
        }
    }

    #[test]
    fn only_approved_steps_and_pre_approved_rollback_run() {
        let args = vec![
            json!("prod"),
            json!([1, 3]),
            json!(["a", "b"]),
            json!(false),
        ];
        let mut a = approver(ids(&args, 1), b(&args, 3));
        assert!(a(&step(1, StepKind::Build)));
        assert!(!a(&step(2, StepKind::Up)));
        assert!(!a(&step(9, StepKind::Rollback)));
        assert_eq!(displays(&args, 2), vec!["a", "b"]);
        let mut a = approver(BTreeSet::new(), true);
        assert!(a(&step(9, StepKind::Rollback)));
    }
}
