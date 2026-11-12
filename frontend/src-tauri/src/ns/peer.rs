//! `peer.*`: Kor-to-Kor köprüsü (§31.1, ADR 0057). Mantık `kor_runtime::peer` içinde; bu dosya
//! ince köprüdür. Makinenin anahtarı yapılandırılmış gizli depoda (anahtar zinciri ya da kasa
//! dosyası) durur. Gizlilik/çevrimdışı modda yalnızca bu bilgisayar (loopback) bağlanabilir.
//!
//! - `peer.list()` → `{ self, handoff, serving, peers: PeerRow[] }` (her makinenin yükü ve yetkisi)
//! - `peer.pairCode()` → `{ code, fingerprint, size, modules, expiresAt }`; kod kullanılınca
//!   `kor:peer/paired`
//! - `peer.join(code, ssh?)` → `{ name, fingerprint }`
//! - `peer.run(peer, text, dir?)` → `{ peer, task }` (açık projenin kopyasıyla; `peer` = `auto` en
//!   boş makine), `peer.attach(peer, task)`, `peer.approve(peer, task, id, approval)`,
//!   `peer.abort(peer, task)` → `true`; olaylar `kor:peer/event` `{ peer, task, event }`
//! - `peer.tasks(peer)` → görevler; `peer.result(peer, task)` → `{ task, state, diff, files }`;
//!   `peer.apply(peer, task, files?)` → `Applied[]`
//! - `peer.sync(peer)` → `SyncReport`; `peer.allow(peer, grant)` → `Grant`;
//!   `peer.route(peer, ssh, addrs)`, `peer.remove(peer)` → `true`
//! - `peer.serve(on)` → `{ listening }` (bu masaüstü görev kabul eder)
//! - `peer.standby(peer, sessionId, on)` → `true`; makine kapanınca devralınırsa `kor:peer/handoff`

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use kor_agent::remote::wire::Command;
use kor_runtime::peer::{self, Node, PeerEntry, client, pair, snapshot, standby};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "peer";

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Açık görev akışları: `"peer/task"` → komut kanalı.
fn follows() -> &'static Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Value>>> {
    static F: OnceLock<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Value>>>> =
        OnceLock::new();
    F.get_or_init(Mutex::default)
}

/// Yansıtılan oturumlar: oturum → durdurma işareti.
fn mirrors() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static M: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    M.get_or_init(Mutex::default)
}

/// Bu masaüstündeki dinleyici: adres ve görevi.
type Listener = Mutex<Option<(String, tokio::task::JoinHandle<()>)>>;

fn server() -> &'static Listener {
    static S: OnceLock<Listener> = OnceLock::new();
    S.get_or_init(Mutex::default)
}

/// Masaüstüyle gelen `kor` CLI: görev işçileri onunla başlar (`kor peer worker`).
fn cli() -> Result<std::path::PathBuf, String> {
    crate::kor_cli::find().ok_or_else(|| crate::kor_cli::MISSING.to_owned())
}

fn node(state: &AppState) -> Result<Node, String> {
    let cfg = root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map(|l| l.config.vault)
        .unwrap_or_default();
    let store = kor_runtime::secure::vault_store(&cfg, None).map_err(|e| e.to_string())?;
    let mut n = peer::local_node(store.as_ref())?;
    let dir = n.dir.clone();
    n.spawn = Arc::new(move |p: &str, id: &str| {
        use kor_platform::Detach as _;
        let exe = cli()?;
        let task_dir = dir.join("tasks").join(p).join(id);
        let log = std::fs::File::create(task_dir.join("worker.log")).map_err(|e| e.to_string())?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        std::process::Command::new(exe)
            .args(["peer", "worker", p, id])
            .stdin(std::process::Stdio::null())
            .stdout(log)
            .stderr(err)
            .detach()
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Görev işçisi başlatılamadı: {e}"))
    });
    Ok(n)
}

fn entry(n: &Node, name: &str) -> Result<PeerEntry, String> {
    n.book()?
        .get(name)
        .cloned()
        .ok_or_else(|| format!("{name} adında eşleşmiş makine yok"))
}

fn audit_on() {
    if kor_agent::audit::path().is_none() {
        kor_agent::audit::set_path(kor_runtime::secure::audit_path());
    }
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    audit_on();
    let r = match name {
        "list" => list(state).await,
        "pairCode" => pair_code(state, app).await,
        "join" => join(state, args).await,
        "run" => run(state, args, app).await,
        "attach" => attach(state, args, app).await,
        "approve" | "abort" => command(name, args),
        "tasks" => tasks(state, args).await,
        "result" => result(state, args).await,
        "apply" => apply(state, args).await,
        "sync" => sync(state, args).await,
        "allow" => allow(state, args),
        "route" => route(state, args),
        "remove" => remove(state, args),
        "serve" => serve(state, args).await,
        "standby" => standby_cmd(state, args, app),
        _ => return None,
    };
    Some(r)
}

async fn list(state: &AppState) -> Res {
    let n = node(state)?;
    let book = n.book()?;
    let loads = client::loads(&n).await;
    let peers: Vec<Value> = loads
        .iter()
        .map(|(p, r)| {
            json!({
                "name": p.name, "fingerprint": p.key_bytes().map(|k| peer::fingerprint(&k)).unwrap_or_default(),
                "addrs": p.addrs, "ssh": p.ssh, "grant": p.grant,
                "load": r.as_ref().ok(), "score": r.as_ref().ok().map(client::Load::score),
                "error": r.as_ref().err(),
            })
        })
        .collect();
    let least = client::least_busy(&loads).map(|p| p.name.clone());
    Ok(json!({
        "self": { "name": book.name, "fingerprint": n.identity.fingerprint(), "listen": book.listen },
        "handoff": book.handoff,
        "serving": lock(server()).as_ref().map(|(a, _)| a.clone()),
        "leastBusy": least,
        "peers": peers,
    }))
}

async fn pair_code(state: &AppState, app: &AppHandle) -> Res {
    let n = node(state)?;
    let book = n.book()?;
    let code = pair::create(&n.dir, client::advertised_addrs(&book.listen))?;
    // Dinleyici yoksa kod kullanılana kadar burada dinle.
    if lock(server()).is_none()
        && let Ok(lst) = tokio::net::TcpListener::bind(&book.listen).await
    {
        let n2 = Arc::new(n.clone());
        let id = code.id;
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + pair::TTL;
            while pair::is_pending(&n2.dir, id) && tokio::time::Instant::now() < deadline {
                if let Ok(Ok((s, _))) =
                    tokio::time::timeout(Duration::from_millis(500), lst.accept()).await
                {
                    let n3 = n2.clone();
                    tokio::spawn(async move {
                        let _ = peer::server::serve_conn(&n3, s).await;
                    });
                }
            }
        });
    }
    let before = book.peers.clone();
    let (dir, id, app2) = (n.dir.clone(), code.id, app.clone());
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + pair::TTL;
        while pair::is_pending(&dir, id) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Ok(after) = peer::Book::load(&dir)
            && let Some(p) = after.peers.iter().find(|p| !before.contains(p))
        {
            let _ = app2.emit("kor:peer/paired", json!({ "name": p.name }));
        }
    });
    let (size, modules) = pair::qr_matrix(&code);
    Ok(json!({
        "code": code.render(), "fingerprint": n.identity.fingerprint(), "size": size, "modules": modules,
        "expiresAt": chrono_ms() + u64::try_from(pair::TTL.as_millis()).unwrap_or(0),
    }))
}

fn chrono_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

async fn join(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let code = pair::Code::parse(s(args, 0, "peer.join")?)?;
    let ssh = opt_s(args, 1).filter(|x| !x.trim().is_empty());
    let e = pair::join(&n, &code, ssh).await?;
    Ok(
        json!({ "name": e.name, "fingerprint": e.key_bytes().map(|k| peer::fingerprint(&k)).unwrap_or_default() }),
    )
}

/// Bir görev akışını olaylara bağla; ilk `attached` çerçevesini bekler.
async fn follow(
    app: &AppHandle,
    peer_name: String,
    mut link: peer::noise::Link,
) -> Result<String, String> {
    let task = loop {
        let v = link.recv_within(Duration::from_secs(60)).await?;
        if v["type"] == "error" {
            return Err(v["message"].as_str().unwrap_or("hata").to_owned());
        }
        let _ = app.emit(
            "kor:peer/event",
            json!({ "peer": peer_name, "task": v["task"], "event": v }),
        );
        if v["type"] == "attached"
            && let Some(t) = v["task"].as_str()
        {
            break t.to_owned();
        }
    };
    let key = format!("{peer_name}/{task}");
    lock(follows()).insert(key.clone(), link.tx.clone());
    let (app2, t2) = (app.clone(), task.clone());
    tokio::spawn(async move {
        while let Some(v) = link.recv().await {
            let _ = app2.emit(
                "kor:peer/event",
                json!({ "peer": peer_name, "task": t2, "event": v }),
            );
        }
        lock(follows()).remove(&key);
        let _ = app2.emit(
            "kor:peer/event",
            json!({ "peer": peer_name, "task": t2, "event": { "type": "detached" } }),
        );
    });
    Ok(task)
}

async fn run(state: &AppState, args: &[Value], app: &AppHandle) -> Res {
    let n = node(state)?;
    let want = s(args, 0, "peer.run")?;
    let text = s(args, 1, "peer.run")?.trim().to_owned();
    if text.is_empty() {
        return Err("Görev boş".into());
    }
    let e = if want == "auto" {
        let loads = client::loads(&n).await;
        client::least_busy(&loads)
            .cloned()
            .ok_or("Ulaşılabilir eşleşmiş makine yok")?
    } else {
        entry(&n, want)?
    };
    let ws = if let Some(d) = opt_s(args, 2).filter(|d| !d.trim().is_empty()) {
        client::Workspace::Dir { path: d.to_owned() }
    } else {
        let r = root(state)?;
        client::Workspace::Snapshot {
            bundle: tokio::task::spawn_blocking(move || snapshot::collect(&r))
                .await
                .map_err(|e| e.to_string())??,
        }
    };
    let link = client::run(&n, &e, &text, &ws).await?;
    let task = follow(app, e.name.clone(), link).await?;
    Ok(json!({ "peer": e.name, "task": task }))
}

async fn attach(state: &AppState, args: &[Value], app: &AppHandle) -> Res {
    let n = node(state)?;
    let e = entry(&n, s(args, 0, "peer.attach")?)?;
    let task = s(args, 1, "peer.attach")?;
    let link = client::attach(&n, &e, Some(task), 0).await?;
    follow(app, e.name, link).await.map(|_| json!(true))
}

fn command(name: &str, args: &[Value]) -> Res {
    let key = format!("{}/{}", s(args, 0, name)?, s(args, 1, name)?);
    let c = if name == "approve" {
        Command::Approve {
            id: s(args, 2, name)?.to_owned(),
            approval: s(args, 3, name)?.to_owned(),
            files: None,
        }
    } else {
        Command::Abort
    };
    let tx = lock(follows())
        .get(&key)
        .cloned()
        .ok_or("Bu görev izlenmiyor")?;
    tx.send(serde_json::to_value(c).map_err(|e| e.to_string())?)
        .map_err(|_| "Bağlantı kapandı".to_owned())?;
    Ok(json!(true))
}

async fn tasks(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let e = entry(&n, s(args, 0, "peer.tasks")?)?;
    let mut link = client::attach(&n, &e, None, 0).await?;
    loop {
        let v = link.recv_within(Duration::from_secs(30)).await?;
        if v["type"] == "tasks" {
            return Ok(v["tasks"].clone());
        }
        if v["type"] == "error" {
            return Err(v["message"].as_str().unwrap_or("hata").to_owned());
        }
    }
}

async fn result(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let e = entry(&n, s(args, 0, "peer.result")?)?;
    let r = client::result(&n, &e, s(args, 1, "peer.result")?).await?;
    let files: Vec<Value> = r
        .changes
        .iter()
        .map(|c| {
            let kind = match (&c.before, &c.after) {
                (None, _) => "added",
                (_, None) => "deleted",
                _ => "modified",
            };
            json!({ "path": c.path, "kind": kind })
        })
        .collect();
    Ok(json!({ "task": r.task, "state": r.state, "diff": r.diff, "files": files }))
}

async fn apply(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let name = s(args, 0, "peer.apply")?;
    let task = s(args, 1, "peer.apply")?;
    let e = entry(&n, name)?;
    let only: Option<Vec<String>> = args.get(2).and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    });
    let r = client::result(&n, &e, task).await?;
    let res = snapshot::apply(&root(state)?, &r.changes, only.as_deref());
    peer::audit("apply", name, json!({ "task": task, "files": res.len() }));
    serde_json::to_value(res).map_err(|e| e.to_string())
}

async fn sync(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let e = entry(&n, s(args, 0, "peer.sync")?)?;
    let r = client::sync(&n, &e).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

fn allow(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let name = s(args, 0, "peer.allow")?;
    let grant: peer::Grant = serde_json::from_value(args.get(1).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("Geçersiz yetki: {e}"))?;
    let mut book = n.book()?;
    let p = book
        .peers
        .iter_mut()
        .find(|p| p.name == name)
        .ok_or("Eşleşmiş makine yok")?;
    p.grant = grant.clone();
    book.save(&n.dir)?;
    peer::audit(
        "grant",
        name,
        serde_json::to_value(&grant).unwrap_or(Value::Null),
    );
    serde_json::to_value(grant).map_err(|e| e.to_string())
}

fn route(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let name = s(args, 0, "peer.route")?;
    let mut book = n.book()?;
    let p = book
        .peers
        .iter_mut()
        .find(|p| p.name == name)
        .ok_or("Eşleşmiş makine yok")?;
    p.ssh = opt_s(args, 1)
        .filter(|x| !x.trim().is_empty())
        .map(str::to_owned);
    if let Some(a) = args.get(2).and_then(Value::as_array) {
        p.addrs = a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    book.save(&n.dir)?;
    Ok(json!(true))
}

fn remove(state: &AppState, args: &[Value]) -> Res {
    let n = node(state)?;
    let name = s(args, 0, "peer.remove")?;
    let mut book = n.book()?;
    book.remove(name)?;
    book.save(&n.dir)?;
    peer::audit("removed", name, json!({}));
    Ok(json!(true))
}

async fn serve(state: &AppState, args: &[Value]) -> Res {
    if !b(args, 0) {
        if let Some((_, h)) = lock(server()).take() {
            h.abort();
        }
        return Ok(json!({ "listening": null }));
    }
    if let Some((a, _)) = lock(server()).as_ref() {
        return Ok(json!({ "listening": a }));
    }
    let n = Arc::new(node(state)?);
    let addr = n.book()?.listen;
    let lst = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("{addr}: {e}"))?;
    let h = tokio::spawn(async move {
        let _ = peer::server::listen(n, lst).await;
    });
    *lock(server()) = Some((addr.clone(), h));
    Ok(json!({ "listening": addr }))
}

fn standby_cmd(state: &AppState, args: &[Value], app: &AppHandle) -> Res {
    let name = s(args, 0, "peer.standby")?.to_owned();
    let session = s(args, 1, "peer.standby")?.to_owned();
    if !kor_runtime::valid_session_id(&session) {
        return Err("Geçersiz oturum kimliği".into());
    }
    if !b(args, 2) {
        if let Some(t) = lock(mirrors()).remove(&session) {
            t.store(true, Ordering::Relaxed);
        }
        let n = node(state)?;
        if let Ok(e) = entry(&n, &name) {
            tokio::spawn(async move {
                let _ = standby::release(&n, &e, &session).await;
            });
        }
        return Ok(json!(true));
    }
    let n = node(state)?;
    let e = entry(&n, &name)?;
    let mut book = n.book()?;
    book.handoff = Some(name.clone());
    book.save(&n.dir)?;
    let transcript = kor_runtime::sessions_dir()
        .ok_or("Veri klasörü yok")?
        .join(format!("{session}.jsonl"));
    let token = Arc::new(AtomicBool::new(false));
    lock(mirrors()).insert(session.clone(), token.clone());
    let mut m = standby::Mirror {
        session: session.clone(),
        transcript,
        project: root(state)?,
        sent_digest: None,
    };
    let app2 = app.clone();
    tokio::spawn(async move {
        while !token.load(Ordering::Relaxed) {
            if let Ok(standby::Renewed::TakenOver(task)) = standby::renew(&n, &e, &mut m).await {
                let _ = app2.emit(
                    "kor:peer/handoff",
                    json!({ "peer": e.name, "session": m.session, "task": task }),
                );
                lock(mirrors()).remove(&m.session);
                break;
            }
            tokio::time::sleep(standby::INTERVAL).await;
        }
    });
    Ok(json!(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approve_needs_a_followed_task() {
        let r = command(
            "approve",
            &[json!("srv"), json!("r-1"), json!("c1"), json!("yes")],
        );
        assert_eq!(r.unwrap_err(), "Bu görev izlenmiyor");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        lock(follows()).insert("srv/r-2".into(), tx);
        command("abort", &[json!("srv"), json!("r-2")]).unwrap();
        assert_eq!(rx.try_recv().unwrap()["type"], "abort");
    }
}
