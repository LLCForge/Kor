//! Sandbox blocks → audit log and the user (§6.4, §6.7, ADR 0033). The OS blocked the call;
//! the command tool found the block in its output ([`kor_tools::ToolCtx::note_sandbox_blocks`]),
//! and here each one becomes a `sandbox_denied` audit entry and a notice.

use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::AgentEvent;
use crate::audit::{clip, record};

impl crate::agent::Agent {
    /// Take the blocks the last tool call left and report each once.
    pub(crate) fn report_sandbox_denials(
        &self,
        tool: &str,
        command: &str,
        events: &UnboundedSender<AgentEvent>,
    ) {
        let found = std::mem::take(
            &mut *self
                .ctx
                .sandbox_denials
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for d in found {
            let target = d
                .target
                .as_deref()
                .map(|t| self.redactor.redact(t).into_owned());
            let mut v = json!({
                "tool": tool,
                "command": clip(&self.redactor.redact(command)),
                "backend": d.backend.as_str(),
                "kind": d.kind.as_str(),
                "message": clip(&self.redactor.redact(&d.line)),
                "model": format!("{}/{}", self.provider.id(), self.model),
                "root": self.ctx.root.to_string_lossy(),
            });
            if let Some(t) = &target {
                v[if d.kind == kor_sandbox::DenialKind::Network {
                    "host"
                } else {
                    "path"
                }] = json!(clip(t));
            }
            record("sandbox_denied", v);
            let _ = events.send(AgentEvent::Notice(format!("sandbox: {}", d.describe())));
        }
    }
}
