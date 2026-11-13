//! A scripted agent for daemon / editor tests: the real loop and tools, a model that replays
//! queued turns and then answers [`REPLY`].

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kor_agent::{Agent, Permissions};
use kor_config::Trigger;
use kor_core::{FinishReason, Message, PermissionMode, ProviderError, StreamEvent, Usage};
use kor_providers::{
    CancellationToken, ChatRequest, EventStream, ModelInfo, Provider, ProviderManifest, Registry,
};
use kor_runtime::triggers::runner;
use kor_secrets::Redactor;
use kor_tools::{ToolCtx, default_tools};
use serde_json::Value;

pub const REPLY: &str = "Kor looked at the project: all good.";

pub fn text(t: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta { text: t.into() },
        StreamEvent::Usage {
            usage: Usage {
                input: 10,
                output: 5,
                ..Usage::default()
            },
        },
        StreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ]
}

#[allow(dead_code)] // used by the editor test only
pub fn call(id: &str, name: &str, args: &Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            index: 0,
            id: id.into(),
            name: name.into(),
        },
        StreamEvent::ToolCallDelta {
            index: 0,
            json: args.to_string(),
        },
        StreamEvent::ToolCallEnd { index: 0 },
        StreamEvent::Finish {
            reason: FinishReason::ToolUse,
        },
    ]
}

struct Scripted {
    manifest: ProviderManifest,
    turns: Arc<Mutex<VecDeque<Vec<StreamEvent>>>>,
    seen: Arc<Mutex<Vec<Vec<Message>>>>,
}

#[async_trait]
impl Provider for Scripted {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        self.seen.lock().unwrap().push(req.messages);
        let evs = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| text(REPLY));
        Ok(Box::pin(futures::stream::iter(evs.into_iter().map(Ok))))
    }
}

/// Builds agents whose model is the shared script.
#[derive(Default, Clone)]
pub struct Factory {
    pub turns: Arc<Mutex<VecDeque<Vec<StreamEvent>>>>,
    /// Messages of every model call.
    pub seen: Arc<Mutex<Vec<Vec<Message>>>>,
}

#[async_trait]
impl runner::AgentFactory for Factory {
    async fn build(
        &self,
        dir: &Path,
        _project: &Path,
        _t: &Trigger,
        _job: &str,
    ) -> Result<runner::Assembled, String> {
        Ok(runner::Assembled {
            agent: Agent {
                provider: Arc::new(Scripted {
                    manifest: Registry::builtin().get("local").unwrap().clone(),
                    turns: self.turns.clone(),
                    seen: self.seen.clone(),
                }),
                model: "m".into(),
                tools: default_tools(),
                permissions: Permissions::new(PermissionMode::Default, &[], &[], &[]).unwrap(),
                ctx: ToolCtx::new(dir).unwrap(),
                system: "sys".into(),
                messages: Vec::new(),
                usage: Usage::default(),
                max_turns: 10,
                max_tokens: None,
                reasoning: None,
                redactor: Redactor::default(),
                hooks: None,
                rules: Vec::new(),
                auto_compact: false,
                context_window: kor_agent::compact::DEFAULT_CONTEXT_WINDOW,
                memory_extract: false,
                memory_base: None,
                session_notes: None,
                output_budget: None,
                dream: false,
                tool_mode: kor_agent::toolcall::ToolMode::Native,
                verify: kor_agent::verify::Verify::default(),
                stats: kor_agent::repair::ToolStats::default(),
                context: kor_agent::context::Budget::default(),
                cost: None,
                shadow: None,
                repair: None,
            },
            cost: None,
            transcript: None,
        })
    }
}
