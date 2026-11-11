//! Keep `~/Kor/` live: a `notify` watcher on the folder and on Kor's stores runs [`super::sync`]
//! and the waiting `Görevler/` tasks within half a second (`kor daemon`, the desktop app).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::{Approver, PermissionReply, PermissionRequest};
use kor_core::{Message, Role};
use notify::{RecursiveMode, Watcher as _};
use serde::Serialize;
use tokio::sync::mpsc;

use super::{FileTask, Layout, SyncReport, TaskOutcome, TaskRunner, run_pending, sync};

/// Coalescing window for bursts of file events.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// What one pass did (sent to `on_change`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pass {
    pub sync: SyncReport,
    pub tasks: Vec<TaskOutcome>,
    pub inbox: usize,
}

/// Running watcher; dropping it stops watching.
pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("korfolder::Watcher")
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One sync + task pass (also what `kor folder sync` runs).
pub async fn pass(l: &Layout, runner: &dyn TaskRunner) -> Pass {
    let sync = sync(l).unwrap_or_else(|e| SyncReport {
        notes: vec![e.to_string()],
        ..SyncReport::default()
    });
    Pass {
        sync,
        tasks: run_pending(l, runner).await,
        inbox: super::inbox_files(l).len(),
    }
}

/// Watch `~/Kor/`, `<home>/sessions` and `<home>/undo`; must be called inside a Tokio runtime.
///
/// # Errors
/// When the OS watcher cannot be created or the folder is not enabled.
pub fn start(
    l: Layout,
    runner: Arc<dyn TaskRunner>,
    on_change: Arc<dyn Fn(&Pass) + Send + Sync>,
) -> Result<Watcher, String> {
    if !l.enabled() {
        return Err("~/Kor is not enabled".into());
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok_and(|e| !e.kind.is_access()) {
            let _ = tx.send(());
        }
    })
    .map_err(|e| e.to_string())?;
    w.watch(&l.root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    for d in ["sessions", "undo"] {
        let p = l.home.join(d);
        if p.is_dir() {
            let _ = w.watch(&p, RecursiveMode::NonRecursive);
        }
    }
    let task = tokio::spawn(async move {
        // First pass right away: tasks dropped while nothing was watching.
        let mut first = true;
        loop {
            if !first {
                if rx.recv().await.is_none() {
                    return;
                }
                tokio::time::sleep(DEBOUNCE).await;
                while rx.try_recv().is_ok() {}
            }
            first = false;
            if !l.enabled() {
                return;
            }
            let p = pass(&l, runner.as_ref()).await;
            // Our own writes cause events too; drop those so the loop settles.
            tokio::time::sleep(Duration::from_millis(50)).await;
            while rx.try_recv().is_ok() {}
            on_change(&p);
        }
    });
    Ok(Watcher { _watcher: w, task })
}

/// Nobody is present to approve: every request is denied.
struct Unattended;

#[async_trait]
impl Approver for Unattended {
    fn surface(&self) -> &'static str {
        "unattended"
    }

    async fn approve(&self, _req: &PermissionRequest) -> PermissionReply {
        PermissionReply::Deny
    }
}

/// The normal agent ([`crate::prepare`]) in the task's folder and mode.
#[derive(Debug, Clone, Copy)]
pub struct AgentRunner {
    pub lang: kor_i18n::Lang,
}

#[async_trait]
impl TaskRunner for AgentRunner {
    async fn run(&self, task: &FileTask) -> Result<String, String> {
        let l = kor_i18n::Localizer::new(self.lang);
        let paths = kor_config::Paths::discover(&task.root);
        let loaded = kor_config::load(&paths, std::env::vars(), &[]).map_err(|e| e.to_string())?;
        let opts = crate::Options {
            root: task.root.clone(),
            model: task.model.clone(),
            mode: Some(task.mode),
            resume: None,
            new_id: Some(format!("korfolder-{}", kor_agent::session::new_id())),
            surface: kor_agent::prompt::Surface::default(),
        };
        let mut notes = Vec::new();
        let mut p = crate::prepare(&l, &loaded, &opts, &mut notes)?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let stop = p
            .agent
            .send(
                task.prompt.clone(),
                &tx,
                &Unattended,
                kor_providers::CancellationToken::new(),
            )
            .await
            .map_err(|e| e.to_string())?;
        drop(tx);
        let _ = drain.await;
        if let Some(path) = &p.path {
            let _ = kor_agent::session::save(path, &p.agent.messages);
        }
        let answer = last_answer(&p.agent.messages);
        Ok(match stop {
            kor_agent::StopReason::Done(_) => answer,
            other => format!("{answer}\n\n> {other:?}"),
        })
    }
}

/// Text of the final assistant message.
pub fn last_answer(msgs: &[Message]) -> String {
    msgs.iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.plain_text().trim().is_empty())
        .map(|m| m.plain_text().trim().to_owned())
        .unwrap_or_default()
}

/// Start watching when `~/Kor/` is enabled (daemon / desktop start-up). `None` when it is off.
pub fn start_if_enabled(
    lang: kor_i18n::Lang,
    on_change: Arc<dyn Fn(&Pass) + Send + Sync>,
) -> Option<Result<Watcher, String>> {
    let l = Layout::current().filter(Layout::enabled)?;
    Some(start(l, Arc::new(AgentRunner { lang }), on_change))
}
