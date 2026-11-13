//! `remote.*`: SSH uzak mod (§15.3) ve canlı sistem keşfi (§15.16), ADR 0044. Mantık
//! `kor_runtime::remote` (kurulum/sürüm eşleme, `kor remote-serve` istemcisi) ve
//! `kor_agent::remote::discover` içinde; bu dosya ince köprüdür. Kimlik yalnızca kullanıcının `ssh`
//! anahtarları/ajanıyla; parola saklanmaz. Gizlilik/çevrimdışı modda hiçbir bağlantı açılmaz.
//!
//! - `remote.hosts()` → `{ id, address, scannedAt | null }[]` (connections.toml + taranmış hostlar)
//! - `remote.connect(target, project?)` → `{ target, os, arch, version, replaced, project, tasks }`
//! - `remote.status()` → `{ target, task | null }[]`
//! - `remote.send(target, text, task?)`, `remote.approve(target, id, approval)`, `remote.abort(target)`,
//!   `remote.attach(target, task)`, `remote.tasks(target)`, `remote.disconnect(target)` → `true`
//! - `remote.discover(target, sudoSteps[])` → `{ snapshot, sudoOffered, sudoUsed, markdown, linked }`
//! - `remote.report(host)` → `InfraSnapshot | null`
//!
//! Olaylar: `kor:remote/event` `{ target, event }` (masaüstü sohbetiyle aynı olay şeması),
//! `kor:remote/closed` `{ target }`; keşif açık projeye bağlanınca `kor:graph/updated`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use kor_agent::remote::wire::Command;
use kor_runtime::remote::{BinarySource, Client};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "remote";

struct Conn {
    tx: mpsc::UnboundedSender<Command>,
    task: Option<String>,
}

fn conns() -> &'static Mutex<HashMap<String, Conn>> {
    static C: OnceLock<Mutex<HashMap<String, Conn>>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn connections_file() -> Option<std::path::PathBuf> {
    kor_config::user_dir().map(|d| d.join("connections.toml"))
}

fn privacy(state: &AppState) -> bool {
    kor_runtime::privacy_mode()
        || root(state)
            .ok()
            .and_then(|r| crate::bridge::load_config(&r).ok())
            .is_some_and(|c| c.config.privacy.mode)
}

/// Komut satırı argümanı olamayacak, kısa bir hedef.
fn target_arg(args: &[Value], method: &str) -> Result<String, String> {
    let t = s(args, 0, method)?.trim();
    if t.is_empty() || t.len() > 300 || t.starts_with('-') {
        return Err("Geçersiz uzak sunucu adresi".into());
    }
    Ok(t.to_owned())
}

/// Masaüstüyle gelen `kor` CLI'si (yanındaki ikili) ve sürümü: uzakta aynı sürüm çalışmalı.
fn local_cli() -> Result<(std::path::PathBuf, String), String> {
    use kor_platform::NoWindow as _;
    let exe = crate::kor_cli::find().ok_or(crate::kor_cli::MISSING)?;
    let out = std::process::Command::new(&exe)
        .arg("--version")
        .no_window()
        .output()
        .map_err(|e| e.to_string())?;
    let v = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .to_owned();
    if v.is_empty() {
        return Err("kor CLI sürümü okunamadı".into());
    }
    Ok((exe, v))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let r = match name {
        "hosts" => Ok(hosts()),
        "status" => Ok(status()),
        "report" => report(args),
        "connect" | "discover" if privacy(state) => {
            Err("Gizlilik modunda uzak sunuculara bağlanılmaz".into())
        }
        "connect" => connect(args, app).await,
        "discover" => discover(args, app, state).await,
        "send" | "approve" | "abort" | "attach" | "tasks" | "disconnect" => command(name, args),
        _ => return None,
    };
    Some(r)
}

fn hosts() -> Value {
    let mut out: Vec<Value> = Vec::new();
    let infra: Vec<kor_graph::InfraSnapshot> = kor_runtime::kor_home()
        .map(|h| kor_graph::infra::load_dir(&h.join("infra")))
        .unwrap_or_default();
    let scanned = |host: &str| {
        infra
            .iter()
            .find(|s| s.host == host)
            .map_or(Value::Null, |s| json!(s.taken_at))
    };
    if let Some(f) = connections_file()
        && let Ok(text) = std::fs::read_to_string(&f)
        && let Ok(c) = kor_agent::remote::Connections::parse(&text)
    {
        for (id, h) in &c.ssh {
            let host = kor_graph::infra::host_id(&h.host).unwrap_or_default();
            out.push(json!({ "id": id, "address": kor_agent::remote::display_target(h), "host": host, "scannedAt": scanned(&host) }));
        }
    }
    for s in &infra {
        if !out.iter().any(|v| v["host"] == s.host.as_str()) {
            out.push(json!({ "id": s.address, "address": s.address, "host": s.host, "scannedAt": s.taken_at }));
        }
    }
    Value::Array(out)
}

fn status() -> Value {
    Value::Array(
        lock(conns())
            .iter()
            .map(|(t, c)| json!({ "target": t, "task": c.task }))
            .collect(),
    )
}

fn report(args: &[Value]) -> Res {
    let host =
        kor_graph::infra::host_id(s(args, 0, "remote.report")?).ok_or("Geçersiz sunucu adı")?;
    let dir = kor_runtime::kor_home()
        .ok_or("Kor veri klasörü yok")?
        .join("infra");
    Ok(kor_graph::infra::load_dir(&dir)
        .into_iter()
        .find(|s| s.host == host)
        .map_or(Value::Null, |s| json!(s)))
}

async fn connect(args: &[Value], app: &AppHandle) -> Res {
    let target = target_arg(args, "remote.connect")?;
    if lock(conns()).contains_key(&target) {
        return Err("Bu sunucuya zaten bağlı".into());
    }
    let project = opt_s(args, 1).filter(|p| !p.is_empty()).map(str::to_owned);
    let h = kor_agent::remote::resolve_target(&target, connections_file().as_deref())?;
    let (exe, version) = local_cli()?;
    let src = BinarySource::standard(&version, Some(exe));
    let e = kor_runtime::remote::ensure(&kor_agent::remote::SshShell::new(&h), &src)
        .await
        .map_err(|e| format!("Uzak Kor kurulamadı: {e}"))?;
    let mut client = Client::connect(&h, project.as_deref(), e.info.windows).await?;
    let hello = client.hello.clone();
    let (tx, mut rx) = mpsc::unbounded_channel::<Command>();
    lock(conns()).insert(target.clone(), Conn { tx, task: None });
    let (app2, t2) = (app.clone(), target.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                ev = client.next() => {
                    let Some(ev) = ev else { break };
                    if let Some(task) = ev["task"].as_str()
                        && let Some(c) = lock(conns()).get_mut(&t2)
                    {
                        c.task = Some(task.to_owned());
                    }
                    let _ = app2.emit("kor:remote/event", json!({ "target": t2, "event": ev }));
                }
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    let detach = cmd == Command::Detach;
                    if client.send(&cmd).await.is_err() || detach {
                        break;
                    }
                }
            }
        }
        lock(conns()).remove(&t2);
        let _ = app2.emit("kor:remote/closed", json!({ "target": t2 }));
    });
    Ok(json!({
        "target": target,
        "os": e.info.os, "arch": e.info.arch, "version": e.info.version,
        "replaced": e.replaced,
        "project": hello["project"], "tasks": hello["tasks"],
    }))
}

fn command(name: &str, args: &[Value]) -> Res {
    let method = format!("remote.{name}");
    let target = target_arg(args, &method)?;
    let cmd = match name {
        "send" => {
            let text = s(args, 1, &method)?;
            if text.trim().is_empty() || text.len() > 200_000 {
                return Err("Mesaj boş ya da çok uzun".into());
            }
            Command::Send {
                text: text.to_owned(),
                task: opt_s(args, 2).filter(|t| !t.is_empty()).map(str::to_owned),
            }
        }
        "approve" => {
            let approval = s(args, 2, &method)?;
            if !matches!(approval, "yes" | "no" | "always") {
                return Err(format!("Bilinmeyen onay yanıtı: {approval}"));
            }
            Command::Approve {
                id: s(args, 1, &method)?.to_owned(),
                approval: approval.to_owned(),
                files: args.get(3).filter(|v| v.is_object()).cloned(),
            }
        }
        "abort" => Command::Abort,
        "attach" => {
            let task = s(args, 1, &method)?;
            if !kor_agent::remote::wire::valid_task_id(task) {
                return Err("Geçersiz görev kimliği".into());
            }
            Command::Attach {
                task: task.to_owned(),
                from: 0,
            }
        }
        "tasks" => Command::Tasks,
        _ => Command::Detach,
    };
    let map = lock(conns());
    let c = map.get(&target).ok_or("Bu sunucuya bağlı değil")?;
    c.tx.send(cmd).map_err(|_| "Bağlantı kapandı".to_owned())?;
    Ok(json!(true))
}

async fn discover(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let target = target_arg(args, "remote.discover")?;
    let allowed: Vec<String> = args
        .get(1)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let known = kor_agent::remote::discover::step_ids();
    if let Some(bad) = allowed.iter().find(|s| !known.contains(s.as_str())) {
        return Err(format!("Bilinmeyen tarama adımı: {bad}"));
    }
    let h = kor_agent::remote::resolve_target(&target, connections_file().as_deref())?;
    let host = kor_graph::infra::host_id(&h.host).ok_or("Geçersiz sunucu adı")?;
    let address = kor_agent::remote::display_target(&h);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // sudo yalnızca arayüzde adım adım işaretlenen adımlar için.
    let ask = |o: &kor_agent::remote::discover::SudoOffer| allowed.contains(&o.step);
    let sh = kor_agent::remote::SshShell::new(&h);
    let d = kor_agent::remote::discover::discover(&sh, &address, &host, now, &ask)
        .await
        .map_err(|e| format!("Tarama başarısız: {e}"))?;
    let dir = kor_runtime::kor_home()
        .ok_or("Kor veri klasörü yok")?
        .join("infra");
    let (md, _) = kor_agent::remote::discover::save(&dir, &d)?;
    let mut linked = false;
    if let Ok(root) = root(state) {
        let snap = d.snapshot.clone();
        let app = app.clone();
        linked = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
            kor_graph::infra::save(&kor_graph::infra::infra_dir(&root), &snap)
                .map_err(|e| e.to_string())?;
            let g = crate::ns::graph::graph_for(&app, &root)?;
            let mut g = g.lock().unwrap_or_else(PoisonError::into_inner);
            let stats = g.refresh_infra().map_err(|e| e.to_string())?;
            let _ = app.emit(
                "kor:graph/updated",
                json!({ "root": root, "stats": stats, "status": g.status() }),
            );
            Ok(true)
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    Ok(json!({
        "snapshot": d.snapshot, "sudoOffered": d.sudo_offered, "sudoUsed": d.sudo_used,
        "markdown": md, "linked": linked,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_checked_before_anything_runs() {
        assert!(target_arg(&[json!("-oProxyCommand=x")], "remote.connect").is_err());
        assert!(target_arg(&[json!("")], "remote.connect").is_err());
        assert_eq!(target_arg(&[json!(" web1 ")], "x").unwrap(), "web1");
        let e = command("send", &[json!("nowhere"), json!("hi")]).unwrap_err();
        assert_eq!(e, "Bu sunucuya bağlı değil");
        assert!(command("approve", &[json!("web1"), json!("c1"), json!("maybe")]).is_err());
        assert!(command("attach", &[json!("web1"), json!("../x")]).is_err());
        assert!(report(&[json!("../etc")]).is_err());
        assert!(status().as_array().is_some());
    }
}
