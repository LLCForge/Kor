//! The detached worker of a delegated task (`kor peer worker <peer> <id>`, ADR 0057): the
//! remote-serve worker loop of ADR 0044 ([`crate::remote::run_worker`]) with an agent that runs in
//! the task's root under the peer's permission matrix:
//!
//! - every tool is wrapped by [`super::policy`] (read-only unless granted);
//! - the permission mode is `default` and only this machine's deny rules apply (its allow rules
//!   are for its own user, not for a peer), so edits and commands ask the delegating user;
//! - no MCP servers of this machine are attached.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use kor_agent::{AgentEvent, Approver, StopReason};
use kor_core::PermissionMode;
use kor_providers::CancellationToken;
use tokio::sync::mpsc;

use super::policy::{self, Policy};
use super::server::{Meta, read_meta};
use crate::remote::{TaskStore, TurnRunner, run_worker};

/// Agent turns of one delegated task.
pub struct PeerTurns {
    pub l: kor_i18n::Localizer,
    pub loaded: kor_config::Loaded,
    pub id: String,
    pub meta: Meta,
    pub policy: Arc<Policy>,
    prepared: Option<crate::Prepared>,
}

impl std::fmt::Debug for PeerTurns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerTurns")
            .field("id", &self.id)
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

impl PeerTurns {
    fn prepare(&mut self, events: &mpsc::UnboundedSender<AgentEvent>) -> Result<(), String> {
        let exists = crate::sessions_dir()
            .is_some_and(|d| d.join(format!("{}.jsonl", self.id)).is_file());
        let opts = crate::Options {
            root: self.meta.root.clone(),
            model: None,
            mode: Some(PermissionMode::Default),
            resume: exists.then(|| crate::Resume::Id(self.id.clone())),
            new_id: (!exists).then(|| self.id.clone()),
            surface: kor_agent::prompt::Surface::Terminal,
        };
        let mut notes = Vec::new();
        let prepared = crate::prepare(&self.l, &self.loaded, &opts, &mut notes);
        for n in notes {
            let _ = events.send(AgentEvent::Notice(n));
        }
        let mut p = prepared?;
        let deny = &self.loaded.config.permissions.deny;
        p.agent.permissions = kor_agent::Permissions::new(PermissionMode::Default, &[], &[], deny)
            .or_else(|_| kor_agent::Permissions::new(PermissionMode::Default, &[], &[], &[]))
            .map_err(|e| e.to_string())?;
        p.agent.tools = policy::guard(std::mem::take(&mut p.agent.tools), &self.policy);
        self.prepared = Some(p);
        Ok(())
    }
}

#[async_trait]
impl TurnRunner for PeerTurns {
    async fn turn(
        &mut self,
        text: String,
        events: &mpsc::UnboundedSender<AgentEvent>,
        approver: &dyn Approver,
        cancel: CancellationToken,
    ) -> Result<Option<String>, String> {
        if self.prepared.is_none() {
            self.prepare(events)?;
        }
        let p = self.prepared.as_mut().ok_or("agent not ready")?;
        crate::refresh_memory(&mut p.agent);
        let r = p.agent.send(text, events, approver, cancel).await;
        if let Some(path) = &p.path {
            kor_agent::session::save(path, &p.agent.messages).map_err(|e| e.to_string())?;
        }
        let max = p.agent.max_turns;
        match r {
            Ok(StopReason::Budget | StopReason::MaxTurns) => Ok(Some(
                self.l
                    .t_args("stop-max-turns", &kor_i18n::args!("n" => max)),
            )),
            Ok(StopReason::Blocked(reason)) => Err(self
                .l
                .t_args("prompt-blocked", &kor_i18n::args!("reason" => reason))),
            Ok(_) => Ok(None),
            Err(e) => Err(crate::provider_error(&self.l, &e)),
        }
    }
}

/// Run task `id` of `peer` (the peer's current grant applies; a removed peer's task stops).
///
/// # Errors
/// When the task, its metadata or the peer is gone.
pub async fn run(
    l: kor_i18n::Localizer,
    loaded: kor_config::Loaded,
    dir: &Path,
    peer: &str,
    id: &str,
) -> Result<(), String> {
    if !super::valid_name(peer) || !kor_agent::remote::wire::valid_task_id(id) {
        return Err("invalid peer or task".into());
    }
    let store = TaskStore {
        dir: dir.join("tasks").join(peer),
    };
    let meta = read_meta(&store.task_dir(id)).ok_or("task metadata missing")?;
    let grant = super::Book::load(dir)?
        .get(peer)
        .map(|p| p.grant.clone())
        .ok_or_else(|| format!("{peer} is no longer paired"))?;
    let policy = Arc::new(Policy {
        peer: peer.to_owned(),
        task: id.to_owned(),
        grant,
        workspace: meta.workspace,
    });
    let mut turns = PeerTurns {
        l,
        loaded,
        id: id.to_owned(),
        meta,
        policy,
        prepared: None,
    };
    run_worker(&store, id, &mut turns).await
}
