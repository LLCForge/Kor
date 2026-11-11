//! `mcpdev.*`: MCP geliştirici modu / denetleyici paneli (§28.15, ADR 0063). İş mantığı
//! `kor_runtime::mcp_dev`; `kor mcp dev` ile aynı oturum.
//!
//! - `mcpdev.start(path, command?)` → `{ id, inventory }`; trafik `kor:mcpdev/traffic`
//!   `{ id, traffic }`, dosya değişince yeniden yükleme `kor:mcpdev/reloaded` `{ id, inventory, diff }`
//!   ya da `{ id, error }`
//! - `mcpdev.inspect(id)` → `Inventory`, `mcpdev.reload(id)` → `{ inventory, diff }`
//! - `mcpdev.call(id, tool, argsJson)`, `mcpdev.read(id, uri)`, `mcpdev.prompt(id, name, argsJson)`
//! - `mcpdev.log(id)` → `Traffic[]`, `mcpdev.stop(id)` → `bool`, `mcpdev.list()` → `[{ id, path }]`

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use kor_runtime::mcp_dev::{self, Inventory, Session, Sink, Target, Traffic};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, opt_s, s};
use crate::state::AppState;

pub const NS: &str = "mcpdev";

struct Dev {
    target: Target,
    session: Option<Arc<Session>>,
    tools: Vec<Value>,
    /// The file watcher (dropping it stops hot reload).
    _watch: Option<Box<dyn std::any::Any + Send>>,
}

type Devs = Mutex<BTreeMap<u64, Dev>>;
static DEVS: std::sync::LazyLock<Devs> = std::sync::LazyLock::new(Devs::default);
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn devs() -> std::sync::MutexGuard<'static, BTreeMap<u64, Dev>> {
    DEVS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn sink(app: &AppHandle, id: u64) -> Sink {
    let app = app.clone();
    Arc::new(move |t: &Traffic| {
        let _ = app.emit("kor:mcpdev/traffic", json!({ "id": id, "traffic": t }));
    })
}

fn id_arg(args: &[Value]) -> Result<u64, String> {
    args.first()
        .and_then(Value::as_u64)
        .ok_or_else(|| "Oturum numarası eksik".into())
}

fn session(id: u64) -> Result<Arc<Session>, String> {
    devs()
        .get(&id)
        .and_then(|d| d.session.clone())
        .ok_or_else(|| "Sunucu çalışmıyor".into())
}

fn args_json(args: &[Value], i: usize) -> Result<Value, String> {
    match opt_s(args, i).map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(json!({})),
        Some(t) => serde_json::from_str(t).map_err(|e| format!("Argümanlar JSON değil: {e}")),
    }
}

async fn restart(app: &AppHandle, id: u64) -> Result<(Inventory, mcp_dev::SchemaDiff), String> {
    let (target, old, before) = {
        let mut d = devs();
        let dev = d.get_mut(&id).ok_or("Bilinmeyen oturum")?;
        (dev.target.clone(), dev.session.take(), dev.tools.clone())
    };
    if let Some(o) = old {
        o.stop();
    }
    let s = Session::start(&target, sink(app, id)).await?;
    let inv = s.inspect().await?;
    let diff = mcp_dev::schema_diff(&before, &inv.tools);
    if let Some(dev) = devs().get_mut(&id) {
        dev.tools.clone_from(&inv.tools);
        dev.session = Some(Arc::new(s));
    }
    Ok((inv, diff))
}

async fn start(app: &AppHandle, args: &[Value]) -> Res {
    let path = std::path::PathBuf::from(s(args, 0, "mcpdev.start")?);
    let target = match opt_s(args, 1).map(str::trim).filter(|c| !c.is_empty()) {
        Some(cmd) => {
            let mut parts = cmd.split_whitespace().map(str::to_owned);
            let command = parts.next().ok_or("Komut boş")?;
            let cwd = if path.is_file() {
                path.parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default()
            } else {
                path.clone()
            };
            Target {
                command,
                args: parts.collect(),
                cwd,
                watch: path.clone(),
            }
        }
        None => mcp_dev::detect(&path)?,
    };
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let s = Session::start(&target, sink(app, id)).await?;
    let inv = s.inspect().await?;
    let watch = {
        let app = app.clone();
        let rt = tokio::runtime::Handle::current();
        mcp_dev::watch(&target.watch, move || {
            let app = app.clone();
            rt.spawn(async move {
                let body = match restart(&app, id).await {
                    Ok((inventory, diff)) => {
                        json!({ "id": id, "inventory": inventory, "diff": diff })
                    }
                    Err(e) => json!({ "id": id, "error": e }),
                };
                let _ = app.emit("kor:mcpdev/reloaded", body);
            });
        })
        .ok()
        .map(|w| Box::new(w) as Box<dyn std::any::Any + Send>)
    };
    devs().insert(
        id,
        Dev {
            target,
            session: Some(Arc::new(s)),
            tools: inv.tools.clone(),
            _watch: watch,
        },
    );
    Ok(json!({ "id": id, "inventory": inv }))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    _state: &AppState,
) -> Option<Res> {
    Some(match name {
        "start" => start(app, args).await,
        "list" => Ok(json!(devs().iter().map(|(id, d)| json!({ "id": id, "path": d.target.watch.to_string_lossy(), "running": d.session.is_some() })).collect::<Vec<_>>())),
        "inspect" => match id_arg(args).and_then(session) {
            Ok(s) => s.inspect().await.and_then(|i| serde_json::to_value(i).map_err(|e| e.to_string())),
            Err(e) => Err(e),
        },
        "reload" => match id_arg(args) {
            Ok(id) => restart(app, id).await.map(|(inventory, diff)| json!({ "inventory": inventory, "diff": diff })),
            Err(e) => Err(e),
        },
        "call" | "prompt" => match (id_arg(args).and_then(session), s(args, 1, "mcpdev.call"), args_json(args, 2)) {
            (Ok(s), Ok(n), Ok(a)) => if name == "call" { s.call_tool(n, a).await } else { s.get_prompt(n, a).await },
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        },
        "read" => match (id_arg(args).and_then(session), s(args, 1, "mcpdev.read")) {
            (Ok(s), Ok(uri)) => s.read_resource(uri).await,
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "log" => id_arg(args).and_then(session).map(|s| json!(s.traffic())),
        "stop" => id_arg(args).map(|id| {
            let gone = devs().remove(&id);
            if let Some(s) = gone.as_ref().and_then(|d| d.session.as_ref()) {
                s.stop();
            }
            json!(gone.is_some())
        }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_ids() {
        assert_eq!(
            args_json(&[json!(1), json!("t"), json!("{\"a\":1}")], 2).unwrap(),
            json!({ "a": 1 })
        );
        assert_eq!(args_json(&[json!(1), json!("t")], 2).unwrap(), json!({}));
        assert!(args_json(&[json!(1), json!("t"), json!("{x")], 2).is_err());
        assert!(id_arg(&[json!("x")]).is_err());
        assert!(session(424_242).is_err());
    }
}
