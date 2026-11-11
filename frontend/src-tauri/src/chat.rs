//! Sohbet yürütme: oturum başına bellekte bir `kor_agent::Agent` (CLI ile aynı kurulum: `kor_runtime::prepare`),
//! tur başına bir görev. Ajan olayları arayüzün `AgentEvent` sözleşmesine (`frontend/src/desktop/ipc.ts`) çevrilip
//! `kor:event` olarak yayılır; izin soruları `confirm` olayı + `approve` çağrısıyla yanıtlanır.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{
    Agent, AgentEvent, Approver, FileDecision, PermissionReply, PermissionRequest, Review,
    StopReason,
};
use kor_core::{FinishReason, PermissionMode};
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot};

use crate::state::AppState;
use crate::{changes, meta};

struct Chat {
    agent: Agent,
    path: Option<PathBuf>,
}

struct Running {
    cancel: CancellationToken,
    since: u64,
    /// Yeniden yüklenen arayüz çalışan turu ve bekleyen onay kartlarını bundan kurar (`liveSessions`).
    events: Vec<Value>,
}

struct Pending {
    session_id: String,
    reply: oneshot::Sender<Review>,
}

/// §28.13 question / §28.1 compaction preview waiting for the renderer (`prefs.answer`).
struct PendingAnswer {
    session_id: String,
    reply: oneshot::Sender<Value>,
}

#[derive(Default)]
pub struct Chats {
    chats: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Chat>>>>,
    running: Mutex<HashMap<String, Running>>,
    pending: Mutex<HashMap<String, Pending>>,
    answers: Mutex<HashMap<String, PendingAnswer>>,
    /// Tur sürerken gönderilen mesajlar (`g2Send` → `queued`); tur bitince `g2QueueFlush` tek turda gönderir.
    queues: Mutex<HashMap<String, Vec<Queued>>>,
    next_id: AtomicU64,
}

struct Queued {
    id: String,
    text: String,
    attachments: Vec<String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // Haritalar basit kayıtlardır; bir panikten sonra okumaya devam etmek güvenli.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Chats {
    /// Drops the idle in-memory agents under `root` so the next message rebuilds them from their
    /// session file with fresh project settings (trust changed: AGENTS.md, project agents, mode).
    /// A running turn keeps its agent until it ends.
    pub fn reload_idle_under(&self, root: &std::path::Path) {
        let running: Vec<String> = lock(&self.running).keys().cloned().collect();
        let mut chats = lock(&self.chats);
        let ids: Vec<String> = chats.keys().filter(|id| !running.contains(id)).cloned().collect();
        for id in ids {
            let Some(chat) = chats.get(&id) else { continue };
            let Ok(c) = chat.try_lock() else { continue };
            let chat_root = kor_tools::without_verbatim(&c.agent.ctx.root);
            if !chat_root.starts_with(root) {
                continue;
            }
            // Keep the session's folder for the rebuild (it may differ from the open folder).
            lock(root_overrides()).insert(id.clone(), c.agent.ctx.root.clone());
            drop(c);
            chats.remove(&id);
        }
    }

    pub fn running_ids(&self) -> Vec<String> {
        lock(&self.running).keys().cloned().collect()
    }

    /// `LiveSession[]`.
    pub fn live(&self) -> Value {
        Value::Array(
            lock(&self.running)
                .iter()
                .map(|(id, r)| json!({ "sessionId": id, "since": r.since, "events": r.events }))
                .collect(),
        )
    }

    /// Bellekteki sohbetin dökümü (diskteki dosya tur bitince yazılır; çalışırken bu daha günceldir).
    pub fn transcript(&self, id: &str) -> Option<Value> {
        let chat = lock(&self.chats).get(id).cloned()?;
        let guard = chat.try_lock().ok()?;
        Some(crate::store::transcript(&guard.agent.messages))
    }

    pub fn forget(&self, id: &str) {
        changes::forget(id);
        self.abort(id);
        lock(&self.chats).remove(id);
        lock(&self.queues).remove(id);
    }

    /// "Skip permissions" was chosen: every approval still waiting is answered "allow".
    pub fn allow_all_pending(&self) {
        for (_, p) in lock(&self.pending).drain() {
            let _ = p.reply.send(PermissionReply::Allow.into());
        }
    }

    /// Turu iptal eder; bekleyen onaylar reddedilmiş sayılır (aksi halde ajan sonsuza dek bekler).
    pub fn abort(&self, id: &str) {
        if let Some(r) = lock(&self.running).get(id) {
            r.cancel.cancel();
        }
        lock(&self.pending).retain(|_, p| p.session_id != id);
        lock(&self.answers).retain(|_, p| p.session_id != id);
    }

    /// Answer a question or a compaction preview (`prefs.answer`): `{ choice, remember }` or
    /// `{ keep: [index…] }` / `{ cancel: true }`.
    pub fn answer(&self, id: &str, value: Value) -> Result<(), String> {
        let p = lock(&self.answers).remove(id).ok_or("Bu soru artık beklenmiyor")?;
        let _ = p.reply.send(value);
        Ok(())
    }

    /// Wait for the renderer's answer to `id`; `None` when the turn was stopped.
    async fn ask(&self, session_id: &str, id: &str) -> Option<Value> {
        let (tx, rx) = oneshot::channel();
        lock(&self.answers).insert(id.to_owned(), PendingAnswer { session_id: session_id.to_owned(), reply: tx });
        rx.await.ok()
    }

    /// `approval`: 'yes' | 'no' | 'always' (`core/agent.ts` `Approval`). `files` (isteğe bağlı,
    /// `ipc-review.ts` `FileDecisions`): dosya başına `"accept"` | `"reject"` | `{ "edit": içerik }`.
    pub fn approve(&self, id: &str, approval: &str, files: Option<&Value>) -> Result<(), String> {
        let reply = match approval {
            "yes" => PermissionReply::Allow,
            "always" => PermissionReply::AllowAlways,
            "no" => PermissionReply::Deny,
            other => return Err(format!("Bilinmeyen onay yanıtı: {other}")),
        };
        let mut review = Review::from(reply);
        if let Some(map) = files.and_then(Value::as_object) {
            for (path, d) in map {
                let decision = match (d.as_str(), d.get("edit").and_then(Value::as_str)) {
                    (Some("accept"), _) => FileDecision::Accept,
                    (Some("reject"), _) => FileDecision::Reject,
                    (_, Some(content)) => FileDecision::Edit(content.to_owned()),
                    _ => return Err(format!("Geçersiz dosya kararı: {path}")),
                };
                review.files.insert(path.clone(), decision);
            }
        }
        let pending = lock(&self.pending)
            .remove(id)
            .ok_or("Bu onay artık beklenmiyor")?;
        // Tur bu arada bittiyse alıcı yoktur; yanıtın gidecek yeri kalmamıştır.
        let _ = pending.reply.send(review);
        Ok(())
    }

    fn record(&self, session_id: &str, ev: &Value) {
        if let Some(r) = lock(&self.running).get_mut(session_id) {
            r.events.push(ev.clone());
        }
    }
}

/// Olayı oturum kimliğiyle yayar ve canlı tur arabelleğine ekler.
fn emit(app: &AppHandle, session_id: &str, mut body: Value) {
    body["sessionId"] = Value::String(session_id.to_owned());
    app.state::<AppState>().chats.record(session_id, &body);
    // Tepsi, PiP, görev çubuğu, bildirimler ve `kor status` (ADR 0051).
    crate::presence::observe(app, session_id, &body);
    // §17.2 `KOR_IPC_TRACE` / `kor doctor --ipc-trace` (ADR 0061).
    kor_core::ipc_trace::event(kor_core::ipc_trace::Surface::Desktop, "kor:event", &body);
    // Yan panel aynı olayları görür (§18.3, ADR 0058).
    kor_tools::chrome::broadcast_event(&body);
    let _ = app.emit("kor:event", body);
}

/// `kor:g14/alert` (ADR 0061): a slow tool, a turn over its budget or process memory over its budget,
/// with the limits of the observability settings (`g14.getSettings`); nothing when alerts are off.
async fn g14_settings(app: &AppHandle) -> Option<Value> {
    let state = app.state::<AppState>();
    let s = crate::ns::call("g14.getSettings", &[], app, &state).await.ok()?;
    (s["alerts"].as_bool() == Some(true)).then_some(s)
}

fn g14_alert(app: &AppHandle, session_id: &str, kind: &str, message: String) {
    let body = json!({ "at": meta::now_ms(), "sessionId": session_id, "kind": kind, "message": message });
    kor_core::ipc_trace::event(kor_core::ipc_trace::Surface::Desktop, "kor:g14/alert", &body);
    let _ = app.emit("kor:g14/alert", body);
}

struct DesktopApprover {
    app: AppHandle,
    session_id: String,
}

#[async_trait]
impl Approver for DesktopApprover {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.review(req).await.reply
    }

    /// Onay kartı dosya listesi (önce/sonra) ve yıkıcılık nedeniyle açılır; yanıt dosya başına gelir.
    async fn review(&self, req: &PermissionRequest) -> Review {
        // The mode is applied when a turn starts; a switch to "skip permissions" during the turn
        // takes effect here, from the next question on.
        if self.app.state::<AppState>().mode() == Some(PermissionMode::BypassPermissions) {
            return PermissionReply::Allow.into();
        }
        let chats = &self.app.state::<AppState>().chats;
        let (tx, rx) = oneshot::channel();
        lock(&chats.pending).insert(
            req.id.clone(),
            Pending {
                session_id: self.session_id.clone(),
                reply: tx,
            },
        );
        // Arayüz onay kartını aynı kimlikli araç kartına bağlar; araç henüz başlamadığından kart burada açılır
        // (izin verilince gelen ToolStart aynı kimlikle tekrar etmez, arayüz yinelemeyi yok sayar).
        emit(
            &self.app,
            &self.session_id,
            json!({ "type": "tool", "id": req.id, "name": req.tool, "summary": req.summary }),
        );
        emit(
            &self.app,
            &self.session_id,
            json!({
                "type": "confirm", "id": req.id, "name": req.tool, "summary": req.summary,
                "files": req.files, "destructive": req.destructive,
            }),
        );
        // Gönderici düşerse (iptal, sohbet silindi) güvenli taraf: reddet.
        let review = rx.await.unwrap_or_else(|_| PermissionReply::Deny.into());
        crate::presence::answered(&self.app, &req.id);
        review
    }

    /// §28.13: the compact choice card (`question` event); the default is preselected.
    async fn choose(&self, q: &kor_agent::ask::Question) -> Option<kor_agent::ask::Choice> {
        let chats = &self.app.state::<AppState>().chats;
        let id = format!("q-{}", q.id);
        emit(
            &self.app,
            &self.session_id,
            json!({ "type": "question", "id": id, "question": q.question, "options": q.options, "default": q.default }),
        );
        let v = chats.ask(&self.session_id, &id).await?;
        let index = v["choice"].as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < q.options.len());
        Some(kor_agent::ask::Choice { index: index.unwrap_or(q.default), remember: v["remember"].as_bool() == Some(true) })
    }

    /// §20.2: a media tool's parameter card (`paramCard` event): every choice at once as chips,
    /// defaults marked, the cost and its confirmation. Answer `{ values }` or `{ cancel: true }`.
    async fn params(&self, card: &kor_tools::ParamCard) -> Option<kor_tools::ParamAnswer> {
        let chats = &self.app.state::<AppState>().chats;
        let id = format!("p-{}", chats.next_id.fetch_add(1, Ordering::Relaxed));
        emit(&self.app, &self.session_id, json!({ "type": "paramCard", "id": id, "card": card }));
        let v = chats.ask(&self.session_id, &id).await?;
        if v["cancel"].as_bool() == Some(true) {
            return Some(kor_tools::ParamAnswer { values: serde_json::Map::new(), go: false });
        }
        let values = v["values"].as_object().cloned().unwrap_or_else(|| card.defaults());
        Some(kor_tools::ParamAnswer { values, go: true })
    }

    /// §28.1: `kor:context/compactPreview` with what the summary replaces; the card answers with
    /// the items to keep or a cancel. Without `compact_confirm` the event is informational.
    async fn compact(&self, items: &[kor_agent::ask::CompactItem], confirm: bool) -> kor_agent::ask::CompactChoice {
        use kor_agent::ask::CompactChoice;
        let chats = &self.app.state::<AppState>().chats;
        let id = format!("c-{}", chats.next_id.fetch_add(1, Ordering::Relaxed));
        let body = json!({ "sessionId": self.session_id, "id": id, "items": items, "confirm": confirm });
        let _ = self.app.emit("kor:context/compactPreview", body.clone());
        emit(&self.app, &self.session_id, json!({ "type": "compactPreview", "id": id, "items": items, "confirm": confirm }));
        if !confirm {
            return CompactChoice::default();
        }
        match chats.ask(&self.session_id, &id).await {
            Some(v) if v["cancel"].as_bool() == Some(true) => CompactChoice::Cancel,
            Some(v) => CompactChoice::Proceed {
                keep: v["keep"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|n| n.as_u64().and_then(|n| usize::try_from(n).ok())).collect())
                    .unwrap_or_default(),
            },
            None => CompactChoice::default(),
        }
    }
}

/// Uzak mod (`kor remote-serve`, ADR 0044) aynı eşlemeyi kullanır. Bağlam dökümü `context.last` /
/// `context.history` köprüsünden okunur (§5.3).
fn to_ui(ev: AgentEvent) -> Option<Value> {
    kor_agent::remote::wire::event_json(ev)
}

/// Ekler ve açık dosya mesaja yol olarak eklenir; ajan gerektiğinde araçlarla okur.
fn compose(text: &str, attachments: &[String], open_file: Option<&str>) -> String {
    let mut out = text.to_owned();
    if !attachments.is_empty() {
        out.push_str("\n\nAttached files (relative to the project root):\n");
        for a in attachments {
            out.push_str("- ");
            out.push_str(a);
            out.push('\n');
        }
    }
    if let Some(f) = open_file.filter(|f| !f.is_empty()) {
        out.push_str("\n\nFile open in the editor: ");
        out.push_str(f);
    }
    out
}

pub struct SendArgs {
    pub session_id: String,
    pub text: String,
    pub attachments: Vec<String>,
    pub open_file: Option<String>,
}

/// Mesaj kutusunun gönderimi (`g2Send`): boştaysa turu başlatır, çalışıyorsa kuyruğa alır.
/// `interrupt`: çalışan tur iptal edilir; tur bitince arayüz kuyruğu `g2QueueFlush` ile hemen gönderir.
/// Dönüş `G2SendResult`.
pub fn g2_send(
    app: &AppHandle,
    session_id: &str,
    text: &str,
    attachments: Vec<String>,
    interrupt: bool,
) -> Result<Value, String> {
    let chats = &app.state::<AppState>().chats;
    if !lock(&chats.running).contains_key(session_id) {
        send(
            app,
            SendArgs {
                session_id: session_id.to_owned(),
                text: text.to_owned(),
                attachments,
                open_file: None,
            },
        )?;
        return Ok(json!({ "status": "started" }));
    }
    let id = format!("q-{}", chats.next_id.fetch_add(1, Ordering::Relaxed));
    lock(&chats.queues)
        .entry(session_id.to_owned())
        .or_default()
        .push(Queued {
            id: id.clone(),
            text: text.to_owned(),
            attachments,
        });
    if interrupt {
        chats.abort(session_id);
    }
    Ok(json!({ "status": "queued", "queueId": id }))
}

/// Kuyruktan çıkarır; `false`: mesaj bu arada gönderildi.
pub fn queue_remove(app: &AppHandle, session_id: &str, queue_id: &str) -> bool {
    let chats = &app.state::<AppState>().chats;
    let mut queues = lock(&chats.queues);
    let Some(list) = queues.get_mut(session_id) else {
        return false;
    };
    let before = list.len();
    list.retain(|q| q.id != queue_id);
    before != list.len()
}

/// Kuyruğu boşaltır; çıkarılan kimlikleri döner.
pub fn queue_clear(app: &AppHandle, session_id: &str) -> Vec<String> {
    let chats = &app.state::<AppState>().chats;
    lock(&chats.queues)
        .remove(session_id)
        .map(|l| l.into_iter().map(|q| q.id).collect())
        .unwrap_or_default()
}

/// Bekleyen mesajları tek turda gönderir. Tur zaten çalışıyorsa bir şey yapmaz (bitince yine çağrılır).
pub fn queue_flush(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let chats = &app.state::<AppState>().chats;
    if lock(&chats.running).contains_key(session_id) {
        return Ok(());
    }
    let items = lock(&chats.queues).remove(session_id).unwrap_or_default();
    if items.is_empty() {
        return Ok(());
    }
    let consumed: Vec<Value> = items
        .iter()
        .map(|q| json!({ "id": q.id, "text": q.text, "attachments": q.attachments }))
        .collect();
    let text = items
        .iter()
        .map(|q| q.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let attachments = items.iter().flat_map(|q| q.attachments.clone()).collect();
    emit(
        app,
        session_id,
        json!({ "type": "g2:queueConsumed", "items": consumed }),
    );
    send(
        app,
        SendArgs {
            session_id: session_id.to_owned(),
            text,
            attachments,
            open_file: None,
        },
    )
}

/// Turu başlatır ve hemen döner; ilerleme `kor:event` olaylarıyla gelir.
pub fn send(app: &AppHandle, a: SendArgs) -> Result<(), String> {
    if !kor_runtime::valid_session_id(&a.session_id) {
        return Err("Geçersiz oturum kimliği".into());
    }
    let state = app.state::<AppState>();
    {
        let mut running = lock(&state.chats.running);
        if running.contains_key(&a.session_id) {
            return Err("Bu sohbet zaten çalışıyor".into());
        }
        running.insert(
            a.session_id.clone(),
            Running {
                cancel: CancellationToken::new(),
                since: now_ms(),
                events: Vec::new(),
            },
        );
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let id = a.session_id.clone();
        emit(&app, &id, json!({ "type": "turnStart" }));
        let outcome = run_turn(&app, a).await;
        match outcome {
            Ok(()) => {}
            Err(message) => emit(&app, &id, json!({ "type": "error", "message": message })),
        }
        // Not running any more *before* `done`: the renderer answers `done` with `g2QueueFlush`
        // ("send now", queued messages), which does nothing while the session still counts as running.
        lock(&app.state::<AppState>().chats.running).remove(&id);
        emit(&app, &id, json!({ "type": "done" }));
        let _ = app.emit("kor:sessionsChanged", ());
    });
    Ok(())
}

fn root_overrides() -> &'static Mutex<HashMap<String, PathBuf>> {
    static R: std::sync::OnceLock<Mutex<HashMap<String, PathBuf>>> = std::sync::OnceLock::new();
    R.get_or_init(Mutex::default)
}

/// Yeni bir sohbeti açık klasör yerine `root`'ta kurar (zamanlanmış görevler kendi klasöründe çalışır).
/// Sohbet bellekte kurulunca tüketilir.
pub fn start_in(app: &AppHandle, root: PathBuf, a: SendArgs) -> Result<(), String> {
    let id = a.session_id.clone();
    lock(root_overrides()).insert(id.clone(), root);
    send(app, a).inspect_err(|_| {
        lock(root_overrides()).remove(&id);
    })
}

/// Sohbeti bellekten alır; yoksa kayıttan sürdürür ya da yenisini kurar.
/// Kurulumda yapılandırılmış MCP sunucuları da başlatılır; sorunlar sohbette bildirim olarak görünür.
async fn chat_for(
    app: &AppHandle,
    session_id: &str,
) -> Result<Arc<tokio::sync::Mutex<Chat>>, String> {
    let state = app.state::<AppState>();
    if let Some(c) = lock(&state.chats.chats).get(session_id) {
        return Ok(c.clone());
    }
    let root = match lock(root_overrides()).remove(session_id) {
        Some(r) => r,
        None => state.workspace()?,
    };
    crate::bridge::allow_media(app, &root);
    let loaded = crate::bridge::load_config(&root)?;
    let l = Localizer::new(loaded.config.ui.language.resolve());
    let exists = kor_runtime::sessions_dir()
        .is_some_and(|d| d.join(format!("{session_id}.jsonl")).is_file());
    let opts = kor_runtime::Options {
        root,
        model: state.model(),
        mode: state.mode(),
        resume: exists.then(|| kor_runtime::Resume::Id(session_id.to_owned())),
        new_id: (!exists).then(|| session_id.to_owned()),
        surface: kor_agent::prompt::Surface::Desktop,
    };
    let mut notes = Vec::new();
    let prepared = kor_runtime::prepare(&l, &loaded, &opts, &mut notes);
    for n in notes {
        emit(app, session_id, json!({ "type": "notice", "text": n }));
    }
    let mut p = prepared?;
    let mcp = kor_runtime::attach_mcp(&l, &mut p.agent, &p.compat).await;
    for n in mcp.notes {
        emit(app, session_id, json!({ "type": "notice", "text": n }));
    }
    // Aynı sohbet bu arada başka bir yoldan kurulduysa (çift tıklama) ilki kullanılır.
    if let Some(c) = lock(&state.chats.chats).get(session_id) {
        return Ok(c.clone());
    }
    let chat = Arc::new(tokio::sync::Mutex::new(Chat {
        agent: p.agent,
        path: p.path,
    }));
    lock(&state.chats.chats).insert(session_id.to_owned(), chat.clone());
    Ok(chat)
}

/// Arayüzde seçilen model/mod sonraki turdan itibaren geçerli olur (CLI'daki `/model`, `/mode` gibi).
fn apply_choices(
    app: &AppHandle,
    session_id: &str,
    agent: &mut Agent,
    l: &Localizer,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    // Oturuma özel seçim (setSessionConfig) genel seçimden önce gelir.
    let own = meta::read()
        .ok()
        .and_then(|m| m.sessions.get(session_id).cloned())
        .unwrap_or_default();
    if let Some(model) = own
        .model
        .clone()
        .filter(|m| !m.is_empty())
        .or_else(|| state.model())
        && let Some((provider, name)) = model.split_once('/')
        && (agent.provider.id() != provider || agent.model != name)
    {
        let mut notes = Vec::new();
        let (p, m, key) = kor_runtime::build_provider(l, &model, &mut notes)?;
        agent.provider = p;
        m.model().clone_into(&mut agent.model);
        if let Some(k) = key {
            agent.redactor.register(k);
        }
    }
    let own_mode = own
        .permission_mode
        .as_deref()
        .and_then(|m| crate::bridge::from_ui_mode(m).ok());
    // "Skip permissions" chosen in the selector wins over a mode stored for this session earlier:
    // the user asked never to be asked.
    let global = state.mode();
    let chosen = if global == Some(PermissionMode::BypassPermissions) { global } else { own_mode.or(global) };
    if let Some(mut mode) = chosen {
        if mode == PermissionMode::Auto && !agent.ctx.sandbox.is_enforcing() {
            return Err(l.t("mode-auto-needs-sandbox"));
        }
        // §28.2: the plan lock and an approved plan decide over the selector.
        let gate = agent.ctx.plan_gate.clone();
        let root = agent.ctx.root.clone();
        let approved_before = gate
            .approved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if mode == PermissionMode::Plan && agent.permissions.mode != PermissionMode::Plan && !approved_before {
            gate.enter();
        }
        let approved = gate.refresh(&root);
        if gate.lock && !approved {
            mode = PermissionMode::Plan;
        } else if mode == PermissionMode::Plan && approved {
            mode = PermissionMode::Default;
        }
        agent.permissions.mode = mode;
    }
    Ok(())
}

async fn run_turn(app: &AppHandle, a: SendArgs) -> Result<(), String> {
    let id = a.session_id.clone();
    let chat = chat_for(app, &id).await?;
    let mut chat = chat.lock().await;
    let root = chat.agent.ctx.root.clone();
    // A new chat is listed in the sidebar now, not only after its first turn ends: its file is
    // written with the first message (the full transcript replaces it when the turn ends).
    if let Some(p) = chat.path.clone().filter(|p| !p.exists()) {
        let mut first = chat.agent.messages.clone();
        first.push(kor_core::Message::text(kor_core::Role::User, a.text.clone()));
        if kor_agent::session::save(&p, &first).is_ok() {
            let _ = app.emit("kor:sessionsChanged", ());
        }
    }
    crate::presence::turn_started(app, &id, &root, &a.text);
    let loaded = crate::bridge::load_config(&root)?;
    let l = Localizer::new(loaded.config.ui.language.resolve());
    apply_choices(app, &id, &mut chat.agent, &l)?;
    kor_runtime::refresh_memory(&mut chat.agent);
    apply_desktop_prefs(&mut chat.agent, &id);

    let cancel = lock(&app.state::<AppState>().chats.running)
        .get(&id)
        .map(|r| r.cancel.clone())
        .ok_or("Tur kaydı bulunamadı")?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let approver = DesktopApprover {
        app: app.clone(),
        session_id: id.clone(),
    };
    let text = compose(&a.text, &a.attachments, a.open_file.as_deref());
    let usage_model = format!("{}/{}", chat.agent.provider.id(), chat.agent.model);
    let agent = &mut chat.agent;
    let turn = async move {
        let r = agent.send(text, &tx, &approver, cancel).await;
        drop(tx);
        r
    };
    let mut turn_usage = kor_core::Usage::default();
    let alerts = g14_settings(app).await;
    let turn_start = std::time::Instant::now();
    let pump = async {
        // İzin reddedilen çağrı için ajan yalnızca ToolEnd yayar; arayüz sonucu bir araç kartına bağladığından
        // eksik `tool` olayı sonuçtan önce üretilir.
        let mut started = std::collections::HashSet::new();
        let mut tool_clock: HashMap<String, std::time::Instant> = HashMap::new();
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::Usage(u) = &ev {
                turn_usage += *u;
                crate::presence::usage(app, &id, &usage_model, u);
            }
            // `agent.status` (§16.2): saniyede bir gelir, canlı tur arabelleğine yazılmaz; KorStatus
            // `kor:agent/status` olayını dinler ve çekirdeğin fiilini/süresini çizer.
            if matches!(ev, AgentEvent::Status(_)) {
                if let Some(mut body) = to_ui(ev) {
                    body["sessionId"] = Value::String(id.clone());
                    // Tepsi/PiP/`kor status` aynı fiili gösterir (ADR 0051 tracker `type: "status"`).
                    crate::presence::observe(app, &id, &body);
                    kor_core::ipc_trace::event(kor_core::ipc_trace::Surface::Desktop, "kor:event", &body);
                    kor_tools::chrome::broadcast_event(&json!({ "type": "agentStatus", "status": body }));
                    let _ = app.emit("kor:agent/status", body);
                }
                continue;
            }
            if let AgentEvent::ToolEnd { id: tool_id, name, .. } = &ev
                && let (Some(s), Some(t0)) = (&alerts, tool_clock.remove(tool_id))
            {
                let ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
                if ms >= s["slowToolMs"].as_u64().unwrap_or(30_000) {
                    g14_alert(app, &id, "slowTool", format!("{name} {} sn sürdü", ms / 1000));
                }
            }
            let change = match &ev {
                AgentEvent::ToolEnd {
                    id: tool_id,
                    is_error,
                    ..
                } => changes::after_tool(&id, &root, tool_id, *is_error),
                _ => None,
            };
            match &ev {
                AgentEvent::ToolStart {
                    id: tool_id,
                    name,
                    summary,
                } => {
                    started.insert(tool_id.clone());
                    tool_clock.insert(tool_id.clone(), std::time::Instant::now());
                    changes::before_tool(&id, &root, tool_id, name, summary);
                }
                AgentEvent::ToolEnd {
                    id: tool_id, name, ..
                } if started.insert(tool_id.clone()) => {
                    emit(
                        app,
                        &id,
                        json!({ "type": "tool", "id": tool_id, "name": name, "summary": "" }),
                    );
                }
                _ => {}
            }
            if let Some(body) = to_ui(ev) {
                emit(app, &id, body);
            }
            if let Some(body) = change {
                emit(app, &id, body);
            }
        }
    };
    let (result, ()) = tokio::join!(turn, pump);
    if let Some(s) = &alerts {
        let ms = u64::try_from(turn_start.elapsed().as_millis()).unwrap_or(u64::MAX);
        if ms >= s["turnBudgetMs"].as_u64().unwrap_or(600_000) {
            g14_alert(app, &id, "turnBudget", format!("Tur {} sn sürdü (bütçe {} sn)", ms / 1000, s["turnBudgetMs"].as_u64().unwrap_or(0) / 1000));
        }
        let state = app.state::<AppState>();
        if let Ok(o) = crate::ns::call("g14.overview", &[], app, &state).await {
            let (mb, budget) = (o["health"]["memoryMb"].as_f64().unwrap_or(0.0), s["memoryBudgetMb"].as_f64().unwrap_or(f64::MAX));
            if mb > budget {
                g14_alert(app, &id, "memoryBudget", format!("Bellek {mb:.0} MB (bütçe {budget:.0} MB)"));
            }
        }
    }
    // Kenar çubuğunun proje grubu ve modeli, kullanım geçmişi.
    let model_ref = format!("{}/{}", chat.agent.provider.id(), chat.agent.model);
    // The shared ledger (`kor cost`) is written per model call by the agent's cost control.
    let root_shown = root.to_string_lossy().into_owned();
    let _ = meta::update(|m| {
        let s = m.session(&id);
        s.root = Some(root_shown);
        s.model = Some(model_ref.clone());
        if turn_usage.input + turn_usage.output > 0 {
            m.record_usage(meta::UsageEntry {
                at: meta::now_ms(),
                model: model_ref,
                prompt: turn_usage.input,
                completion: turn_usage.output,
            });
        }
        Ok(())
    });

    if let Some(p) = &chat.path
        && let Err(e) = kor_agent::session::save(p, &chat.agent.messages)
    {
        emit(
            app,
            &id,
            json!({ "type": "notice", "text": l.t_args("error-generic", &args!("error" => e.to_string())) }),
        );
    }
    let max_turns = chat.agent.max_turns;
    let note = |key: &str| {
        emit(
            app,
            &id,
            json!({ "type": "notice", "text": l.t_args(key, &args!("n" => max_turns)) }),
        )
    };
    match result {
        Ok(StopReason::Done(FinishReason::Stop | FinishReason::ToolUse))
        | Ok(StopReason::Cancelled) => Ok(()),
        Ok(StopReason::Done(FinishReason::MaxTokens)) => {
            note("stop-max-tokens");
            Ok(())
        }
        Ok(StopReason::Done(FinishReason::ContentFiltered)) => {
            Err(l.t_args("stop-filtered", &args!("n" => max_turns)))
        }
        Ok(StopReason::Done(_)) => {
            note("stop-cancelled");
            Ok(())
        }
        Ok(StopReason::Budget | StopReason::MaxTurns) => {
            note("stop-max-turns");
            Ok(())
        }
        Ok(StopReason::Blocked(reason)) => {
            Err(l.t_args("prompt-blocked", &args!("reason" => reason)))
        }
        Err(e) => Err(kor_runtime::provider_error(&l, &e)),
    }
}

/// Çıktı stili ve kullanıcı sistem istemi (masaüstü tercihleri) sistem isteminin sonuna eklenir.
const PREFS_MARKER: &str = "\n# Desktop preferences\n";

fn apply_desktop_prefs(agent: &mut Agent, session_id: &str) {
    if let Some(i) = agent.system.find(PREFS_MARKER) {
        agent.system.truncate(i);
    }
    let Ok(m) = meta::read() else {
        return;
    };
    let mut extra = String::new();
    if let Some(p) = meta::style_prompt(&m.output_style) {
        extra.push_str(p);
        extra.push('\n');
    }
    if let Some(p) = m.system_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
        extra.push_str("## User instructions\n");
        extra.push_str(p.trim());
        extra.push('\n');
    }
    if let Some(p) = crate::ns::session_prompt_block(session_id) {
        extra.push_str(&p);
        extra.push('\n');
    }
    if let Some(map) = crate::api_files::project_map_text(&agent.ctx.root) {
        extra.push_str("## Project map\n");
        extra.push_str(&map);
        extra.push('\n');
    }
    // Model başına araç çağırma modu (ayarlar > modeller).
    let model = format!("{}/{}", agent.provider.id(), agent.model);
    match m.tool_modes.get(&model).map(String::as_str) {
        Some("native") => agent.tool_mode = kor_agent::toolcall::ToolMode::Native,
        Some("text") => agent.tool_mode = kor_agent::toolcall::ToolMode::Text,
        _ => {}
    }
    if !extra.is_empty() {
        agent.system.push_str(PREFS_MARKER);
        agent.system.push_str(&extra);
    }
}

fn loaded(app: &AppHandle, id: &str) -> Option<Arc<tokio::sync::Mutex<Chat>>> {
    lock(&app.state::<AppState>().chats.chats).get(id).cloned()
}

fn is_running(app: &AppHandle, id: &str) -> bool {
    lock(&app.state::<AppState>().chats.running).contains_key(id)
}

/// Bellekteki sohbetin toplam token kullanımı (sohbet bu oturumda açılmadıysa yok).
pub async fn session_usage(app: &AppHandle, id: &str) -> Option<kor_core::Usage> {
    let chat = loaded(app, id)?;
    let c = chat.lock().await;
    Some(c.agent.usage)
}

/// `ContextInfo`: tahmini bağlam, model penceresi ve sıkıştırma eşiği.
pub async fn context_info(app: &AppHandle, id: Option<&str>) -> Value {
    let compact = meta::read().map(|m| m.compact).unwrap_or_default();
    let window = kor_agent::compact::DEFAULT_CONTEXT_WINDOW;
    let ratio = compact.ratio.unwrap_or(0.92);
    let mut at = kor_agent::compact::threshold(window).min((window as f64 * ratio) as u64);
    if let Some(max) = compact.max_tokens {
        at = at.min(max);
    }
    let (used, usage, compactions) = match id.and_then(|i| loaded(app, i)) {
        Some(chat) => {
            let c = chat.lock().await;
            let used = kor_agent::compact::estimate_tokens(&c.agent.messages)
                + (c.agent.system.len() / 4) as u64;
            let compactions = c
                .agent
                .messages
                .iter()
                .filter(|m| {
                    m.parts.iter().any(|p| {
                        matches!(p, kor_core::ContentPart::Text { text }
                        if text.starts_with(kor_agent::compact::BOUNDARY)
                            || text.starts_with(kor_agent::compact::SESSION_BOUNDARY))
                    })
                })
                .count();
            (used, c.agent.usage, compactions)
        }
        None => (0, kor_core::Usage::default(), 0),
    };
    json!({
        "usedTokens": used,
        "limitTokens": window,
        "compactAtTokens": at,
        "autoCompactRatio": ratio,
        "promptTokens": usage.input,
        "cachedTokens": usage.cache_read,
        "calls": 0,
        "compactions": compactions,
        "savedTokens": 0,
        "compactionCostTokens": 0,
        "mapSavedTokens": Value::Null,
    })
}

/// §28.1 `context.items(sessionId)`: bağlamdaki parçalar (mesajlar, dosyalar, araç çıktıları) ve sabitlenenler.
pub async fn context_items(app: &AppHandle, id: &str) -> Result<Value, String> {
    let chat = chat_for(app, id).await?;
    let c = chat.lock().await;
    Ok(json!({
        "items": kor_agent::ask::compact_items(&c.agent.messages),
        "pinned": c.agent.context.pinned,
        "window": c.agent.context_window,
    }))
}

/// §28.1 `context.drop(sessionId, index)` / `context.pin(sessionId, index, on)`. Çalışırken reddedilir.
pub async fn context_change(app: &AppHandle, id: &str, index: usize, pin: Option<bool>) -> Result<Value, String> {
    if is_running(app, id) {
        return Err("Sohbet çalışırken bağlam değiştirilemez".into());
    }
    let chat = chat_for(app, id).await?;
    let mut c = chat.lock().await;
    let n = kor_agent::ask::compact_items(&c.agent.messages).len();
    if index >= n {
        return Err("Böyle bir parça yok".into());
    }
    match pin {
        Some(true) => {
            if !c.agent.context.pinned.contains(&index) {
                c.agent.context.pinned.push(index);
            }
        }
        Some(false) => c.agent.context.pinned.retain(|i| *i != index),
        None => {
            kor_agent::ask::drop_item(&mut c.agent.messages, index);
            if let Some(p) = &c.path {
                kor_agent::session::save(p, &c.agent.messages).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(json!({
        "items": kor_agent::ask::compact_items(&c.agent.messages),
        "pinned": c.agent.context.pinned,
        "window": c.agent.context_window,
    }))
}

/// `/compact`: eski turları özetler. Sohbet çalışırken reddedilir.
pub async fn compact(app: &AppHandle, id: &str) -> Result<Value, String> {
    if is_running(app, id) {
        return Err("Sohbet çalışırken sıkıştırılamaz".into());
    }
    let chat = chat_for(app, id).await?;
    let mut c = chat.lock().await;
    let before = kor_agent::compact::estimate_tokens(&c.agent.messages);
    let done = kor_agent::compact::compact_now(&mut c.agent, CancellationToken::new(), "manual")
        .await
        .map_err(|e| e.to_string())?;
    let after = kor_agent::compact::estimate_tokens(&c.agent.messages);
    if done && let Some(p) = &c.path {
        kor_agent::session::save(p, &c.agent.messages).map_err(|e| e.to_string())?;
    }
    let _ = app.emit("kor:sessionsChanged", ());
    Ok(if done {
        json!({ "compacted": true, "beforeTokens": before, "afterTokens": after })
    } else {
        json!({ "compacted": false, "reason": "Özetlenecek bir şey yok", "beforeTokens": before, "afterTokens": before })
    })
}

/// Sıkıştırma özetini kullanıcının düzelttiği metinle değiştirir.
pub async fn edit_compaction_summary(
    app: &AppHandle,
    id: &str,
    body: &str,
) -> Result<bool, String> {
    if is_running(app, id) {
        return Err("Sohbet çalışırken değiştirilemez".into());
    }
    let chat = chat_for(app, id).await?;
    let mut c = chat.lock().await;
    let Some(first) = c.agent.messages.first_mut() else {
        return Ok(false);
    };
    let Some(kor_core::ContentPart::Text { text }) = first.parts.first_mut() else {
        return Ok(false);
    };
    let boundary = [
        kor_agent::compact::BOUNDARY,
        kor_agent::compact::SESSION_BOUNDARY,
    ]
    .into_iter()
    .find(|b| text.starts_with(b));
    let Some(boundary) = boundary else {
        return Ok(false);
    };
    *text = format!("{boundary}\n{}", body.trim());
    if let Some(p) = &c.path {
        kor_agent::session::save(p, &c.agent.messages).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// Kullanıcının yazdığı mesajlar (araç sonuçları değil) ve dizinleri.
fn user_turns(messages: &[kor_core::Message]) -> Vec<(usize, String)> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == kor_core::Role::User)
        .filter(|(_, m)| {
            !m.parts
                .iter()
                .any(|p| matches!(p, kor_core::ContentPart::ToolResult { .. }))
        })
        .filter_map(|(i, m)| {
            let text: String = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    kor_core::ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            (!text.starts_with(kor_agent::compact::BOUNDARY)
                && !text.starts_with(kor_agent::compact::SESSION_BOUNDARY))
            .then_some((i, text))
        })
        .collect()
}

/// Mesaj düzenleme / yeniden üretme: geçmiş, hedef kullanıcı mesajından önceye kesilir (sonra yeni metin
/// gönderilir). `ordinal` yoksa metni tutan son mesaj seçilir; metin geçmişle uyuşmazsa reddedilir.
pub async fn rewind(
    app: &AppHandle,
    id: &str,
    ordinal: Option<usize>,
    original: &str,
) -> Result<(), String> {
    if is_running(app, id) {
        return Err("Sohbet çalışırken geçmiş değiştirilemez".into());
    }
    let chat = chat_for(app, id).await?;
    let mut c = chat.lock().await;
    let turns = user_turns(&c.agent.messages);
    let want = original.trim();
    let matches = |t: &str| t.trim() == want || t.starts_with(want);
    let index = match ordinal {
        Some(n) => turns.get(n).filter(|(_, t)| matches(t)).map(|(i, _)| *i),
        None => None,
    }
    .or_else(|| {
        turns
            .iter()
            .rev()
            .find(|(_, t)| matches(t))
            .map(|(i, _)| *i)
    })
    .ok_or("Mesaj geçmişte bulunamadı; sohbeti yenileyip tekrar dene")?;
    c.agent.messages.truncate(index);
    if let Some(p) = &c.path {
        kor_agent::session::save(p, &c.agent.messages).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Sohbetin araç izinleri (oturum araç kapsamı paneli).
pub async fn tool_scopes(app: &AppHandle, id: &str) -> Option<Value> {
    let chat = loaded(app, id)?;
    let c = chat.lock().await;
    let list: Vec<Value> = c
        .agent
        .tools
        .iter()
        .map(|t| {
            let kind = match t.kind() {
                kor_tools::ToolKind::Read => "read",
                kor_tools::ToolKind::Edit => "edit",
                kor_tools::ToolKind::Execute => "exec",
            };
            let level = c
                .agent
                .permissions
                .tool_level(t.rule_name())
                .unwrap_or("default");
            json!({ "name": t.name(), "kind": kind, "level": level })
        })
        .collect();
    Some(Value::Array(list))
}

/// Bu sohbette bir aracın izin düzeyi (desensiz kural eklenir/kaldırılır).
pub async fn set_tool_scope(
    app: &AppHandle,
    id: &str,
    tool: &str,
    level: &str,
) -> Result<Value, String> {
    if !matches!(level, "default" | "allow" | "ask" | "deny") {
        return Err(format!("Bilinmeyen düzey: {level}"));
    }
    let chat = chat_for(app, id).await?;
    {
        let mut c = chat.lock().await;
        let rule_name = c
            .agent
            .tools
            .iter()
            .find(|t| t.name() == tool)
            .map(|t| t.rule_name().to_owned())
            .ok_or_else(|| format!("Bilinmeyen araç: {tool}"))?;
        c.agent
            .permissions
            .set_tool_level(&rule_name, (level != "default").then_some(level));
    }
    tool_scopes(app, id)
        .await
        .ok_or_else(|| "Sohbet bulunamadı".into())
}

#[cfg(test)]
mod tests {
    use super::compose;

    #[test]
    fn attachments_and_open_file_are_listed() {
        let t = compose("düzelt", &["src/a.rs".into()], Some("src/b.rs"));
        assert!(t.starts_with("düzelt\n\nAttached files"));
        assert!(t.contains("- src/a.rs\n"));
        assert!(t.ends_with("File open in the editor: src/b.rs"));
        assert_eq!(compose("x", &[], Some("")), "x");
    }
}
