//! `acp.*`: Agent Client Protocol (§28.8, ADR 0057). Mantık `kor_runtime::acp` içinde.
//!
//! - `acp.setup()` → `{ command, args, protocolVersion, zed }`: editörün Kor'u ajan olarak
//!   başlatması için komut ve Zed `settings.json` bloğu (`kor acp`).
//! - `acp.agents()` → `{ name, command, args }[]`: `[acp.agents]` içindeki ACP ajanları.
//! - `acp.run(agent, prompt)` → `{ stopReason, text }`: bir ACP ajanını açık projede alt-ajan olarak
//!   çalıştırır; güncellemeler `kor:acp/update` `{ agent, update }`, izin istekleri
//!   `kor:acp/permission` `{ id, agent, toolCall, options }` olarak gelir ve `acp.answer(id,
//!   optionId | null)` ile yanıtlanır (yanıt yoksa 5 dakika sonra reddedilir).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use kor_runtime::acp::client::{AcpClient, AgentSpec, ClientUi};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "acp";

type Waiting = Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>;

fn waiting() -> &'static Waiting {
    static W: OnceLock<Waiting> = OnceLock::new();
    W.get_or_init(Waiting::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let r = match name {
        "setup" => setup(),
        "agents" => agents(state),
        "run" => run(state, args, app).await,
        "answer" => answer(args),
        _ => return None,
    };
    Some(r)
}

fn setup() -> Res {
    let exe = crate::kor_cli::find().ok_or(crate::kor_cli::MISSING)?;
    Ok(kor_runtime::acp::editor_setup(&exe))
}

fn configured(state: &AppState) -> Result<std::collections::BTreeMap<String, AgentSpec>, String> {
    let r = root(state)?;
    Ok(crate::bridge::load_config(&r)?.config.acp.agents)
}

fn agents(state: &AppState) -> Res {
    let list: Vec<Value> = configured(state)?
        .into_iter()
        .map(|(name, a)| json!({ "name": name, "command": a.command, "args": a.args }))
        .collect();
    Ok(Value::Array(list))
}

/// Güncellemeleri olay olarak yayar, izinleri kullanıcıya sorar.
struct DesktopUi {
    app: AppHandle,
    agent: String,
    text: Mutex<String>,
}

#[async_trait]
impl ClientUi for DesktopUi {
    fn update(&self, u: &Value) {
        if u["sessionUpdate"] == "agent_message_chunk"
            && let Some(t) = u["content"]["text"].as_str()
        {
            lock(&self.text).push_str(t);
        }
        let _ = self.app.emit("kor:acp/update", json!({ "agent": self.agent, "update": u }));
    }

    async fn permission(&self, tool_call: &Value, options: &[Value]) -> Option<String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(waiting()).insert(id, tx);
        let _ = self.app.emit(
            "kor:acp/permission",
            json!({ "id": id, "agent": self.agent, "toolCall": tool_call, "options": options }),
        );
        let picked = tokio::time::timeout(Duration::from_secs(300), rx).await.ok().and_then(Result::ok).flatten();
        lock(waiting()).remove(&id);
        picked
    }
}

async fn run(state: &AppState, args: &[Value], app: &AppHandle) -> Res {
    let agent = s(args, 0, "acp.run")?.to_owned();
    let prompt = s(args, 1, "acp.run")?.trim().to_owned();
    if prompt.is_empty() {
        return Err("İstem boş".into());
    }
    let spec = configured(state)?
        .get(&agent)
        .cloned()
        .ok_or_else(|| format!("{agent} adında ACP ajanı yapılandırılmamış"))?;
    let ui = Arc::new(DesktopUi {
        app: app.clone(),
        agent: agent.clone(),
        text: Mutex::new(String::new()),
    });
    let c = AcpClient::spawn(&agent, &spec, &root(state)?, ui.clone()).await?;
    let stop = c.prompt(&prompt).await?;
    let text = lock(&ui.text).clone();
    Ok(json!({ "stopReason": stop, "text": text }))
}

fn answer(args: &[Value]) -> Res {
    let id = args.first().and_then(Value::as_u64).ok_or("`acp.answer`: kimlik gerekli")?;
    let tx = lock(waiting()).remove(&id).ok_or("Bu izin isteği artık beklemiyor")?;
    let _ = tx.send(opt_s(args, 1).map(str::to_owned));
    Ok(json!(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_reach_the_waiting_request_once() {
        let (tx, mut rx) = oneshot::channel();
        lock(waiting()).insert(99, tx);
        answer(&[json!(99), json!("allow")]).unwrap();
        assert_eq!(rx.try_recv().unwrap().as_deref(), Some("allow"));
        assert!(answer(&[json!(99), json!("allow")]).is_err());
        assert!(setup().is_ok());
    }
}
