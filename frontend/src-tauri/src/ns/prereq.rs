//! `prereq.*`: `kor prereq list|install` (spec §24.3 Layer C, ADR 0064). Plans come from
//! `kor_platform::sysdeps`; the desktop shows the exact command, asks once in a native dialog
//! (never silent) and runs it with the GUI elevation (UAC, `pkexec`); the OS asks for the
//! password. Output lines go out as `kor:prereq/progress` `{ id, line }`.
//!
//! - `prereq.list()` → `[{ id, installed, permission, command, manager, elevation, action, reason }]`
//! - `prereq.install(id, manager?)` → `{ ok, code, row }` (`ok: false, code: null` = declined)

use std::io::{BufRead, BufReader};
use std::process::Stdio;

use kor_platform::sysdeps::{self, Host, Manager, NoPlan, Prereq};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, opt_s, s};
use crate::state::AppState;

pub const NS: &str = "prereq";

/// One row; `command` is what runs from the desktop (GUI elevation).
fn row(st: &sysdeps::Status) -> Value {
    let (command, manager, elevation, action, reason) = match &st.plan {
        Ok(p) => (Some(p.command_line(true)), p.manager.map(Manager::id), Some(p.elevation), Some(p.action), None),
        Err(NoPlan::NoManager) => (None, None, None, None, Some("no-manager")),
        Err(NoPlan::NotNeeded) => (None, None, None, None, Some("not-needed")),
    };
    json!({
        "id": st.prereq.id(),
        "installed": st.installed,
        "permission": st.prereq.is_permission(),
        "command": command,
        "manager": manager,
        "elevation": elevation,
        "action": action,
        "reason": reason,
    })
}

fn list() -> Value {
    Value::Array(sysdeps::status(&Host::detect()).iter().map(row).collect())
}

fn one(p: Prereq, host: &Host) -> Value {
    row(&sysdeps::Status { prereq: p, installed: sysdeps::installed(p), plan: sysdeps::plan(p, host, None) })
}

async fn install(app: &AppHandle, id: &str, manager: Option<&str>) -> Res {
    let p = Prereq::parse(id).ok_or_else(|| format!("Bilinmeyen gereksinim: {id}"))?;
    let manager = match manager.filter(|m| !m.is_empty()) {
        Some(m) => Some(Manager::parse(m).ok_or_else(|| format!("Bilinmeyen paket yöneticisi: {m}"))?),
        None => None,
    };
    let host = Host::detect();
    let plan = sysdeps::plan(p, &host, manager).map_err(|e| match e {
        NoPlan::NotNeeded => "Bu sistemde gerekmiyor".to_owned(),
        NoPlan::NoManager => "Desteklenen bir paket yöneticisi bulunamadı; Kor bu olmadan çalışmaya devam eder".to_owned(),
    })?;
    let command = plan.command_line(true);
    let msg = format!(
        "Kor şu komutu çalıştıracak:\n\n{command}\n\nŞifre gerekirse sistem sorar; Kor şifreyi görmez. Reddederseniz Kor çalışmaya devam eder, yalnızca buna ihtiyaç duyan özellik kapalı kalır."
    );
    if !crate::api::confirm(app, "Sistem gereksinimi", &msg, "Çalıştır").await {
        return Ok(json!({ "ok": false, "code": null, "row": one(p, &host) }));
    }
    let app2 = app.clone();
    let code = tauri::async_runtime::spawn_blocking(move || -> Result<Option<i32>, String> {
        let mut child = sysdeps::command(&plan, true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Başlatılamadı: {e}"))?;
        let pump = |src: Box<dyn std::io::Read + Send>, app: AppHandle, id: &'static str| {
            std::thread::spawn(move || {
                for line in BufReader::new(src).lines().map_while(Result::ok) {
                    let line = line.trim().to_owned();
                    if !line.is_empty() {
                        let _ = app.emit("kor:prereq/progress", json!({ "id": id, "line": line }));
                    }
                }
            })
        };
        let out = child.stdout.take().map(|o| pump(Box::new(o), app2.clone(), p.id()));
        let err = child.stderr.take().map(|o| pump(Box::new(o), app2.clone(), p.id()));
        let status = child.wait().map_err(|e| e.to_string())?;
        for h in [out, err].into_iter().flatten() {
            let _ = h.join();
        }
        Ok(status.code())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(json!({ "ok": code == Some(0), "code": code, "row": one(p, &Host::detect()) }))
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(_ns: &str, name: &str, args: &[Value], app: &AppHandle, _state: &AppState) -> Option<Res> {
    Some(match name {
        "list" => tauri::async_runtime::spawn_blocking(list).await.map_err(|e| e.to_string()),
        "install" => match s(args, 0, "prereq.install") {
            Ok(id) => install(app, id, opt_s(args, 1)).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use kor_platform::Os;

    use super::*;

    #[test]
    fn rows_carry_the_gui_command() {
        let host = Host { os: Os::Linux, managers: vec![Manager::Apt], root: false };
        let st = sysdeps::Status { prereq: Prereq::Docker, installed: Some(false), plan: sysdeps::plan(Prereq::Docker, &host, None) };
        let r = row(&st);
        assert_eq!(r["id"], "docker");
        assert_eq!(r["command"], "pkexec apt-get install -y docker.io");
        assert_eq!(r["manager"], "apt");
        assert_eq!(r["elevation"], "sudo");
        assert_eq!(r["action"], "install");
        let none = row(&sysdeps::Status { prereq: Prereq::Podman, installed: Some(false), plan: Err(NoPlan::NoManager) });
        assert_eq!(none["reason"], "no-manager");
        assert!(none["command"].is_null());
        let perm = row(&sysdeps::Status {
            prereq: Prereq::Microphone,
            installed: None,
            plan: sysdeps::plan(Prereq::Microphone, &Host { os: Os::Mac, managers: vec![], root: false }, None),
        });
        assert_eq!(perm["permission"], true);
        assert_eq!(perm["action"], "open-settings");
        assert!(perm["installed"].is_null());
    }
}
