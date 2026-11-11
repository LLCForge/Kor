//! Session recording tap (§15.9, ADR 0043). `Agent::send` is the one call every front end makes
//! (CLI, TUI, desktop), so the recording is tapped there once: the event stream is forwarded
//! through [`send`], approvals through a wrapping approver, and diffs and cost are written where
//! they are known. Everything goes through the session's `Redactor` before it is written.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;

use async_trait::async_trait;
use kor_core::{ProviderError, Usage};
use kor_providers::CancellationToken;
use kor_secrets::Redactor;
use kor_tools::Proposal;
use kor_tools::record::Recorder;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::review::{FileDecision, Review};
use crate::{Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, StopReason};

/// Tool output and file contents above this are cut in the recording.
const MAX_TEXT: usize = 64 * 1024;
const MAX_FILE: usize = 512 * 1024;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every string in `v` masked (keys stay: they are Kor's own field names).
pub(crate) fn mask_value(mask: &Redactor, v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(mask.redact(&s).into_owned()),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| mask_value(mask, x)).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, x)| (k, mask_value(mask, x)))
                .collect(),
        ),
        other => other,
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[... {} bytes not recorded]", &s[..end], s.len() - end)
}

fn files_json(files: &[Proposal]) -> Value {
    let side = |c: &Option<String>| match c {
        Some(c) if c.len() > MAX_FILE => json!(null),
        Some(c) => json!(c),
        None => json!(null),
    };
    Value::Array(
        files
            .iter()
            .map(|f| {
                let big = [&f.before, &f.after]
                    .iter()
                    .any(|c| c.as_ref().is_some_and(|c| c.len() > MAX_FILE));
                json!({"path": f.path, "before": side(&f.before), "after": side(&f.after), "truncated": big})
            })
            .collect(),
    )
}

/// Events on their way from the agent loop to the front end.
struct Tap {
    rx: UnboundedReceiver<AgentEvent>,
    out: UnboundedSender<AgentEvent>,
    rec: Arc<Recorder>,
    mask: Redactor,
    text: String,
    thinking: String,
}

impl Tap {
    fn write(&self, v: Value) {
        self.rec.write(mask_value(&self.mask, v));
    }

    fn output(&self, s: &str) {
        self.rec.output(&self.mask.redact(s));
    }

    /// Streamed text is written as one block (masked as a whole: a secret split across two
    /// deltas is still found).
    fn flush(&mut self) {
        if !self.thinking.is_empty() {
            let t = std::mem::take(&mut self.thinking);
            self.write(json!({"kind": "thinking", "text": t}));
        }
        if !self.text.is_empty() {
            let t = std::mem::take(&mut self.text);
            self.output(&format!("{t}\n"));
            self.write(json!({"kind": "text", "text": t}));
        }
    }

    fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.on(ev);
        }
    }

    fn on(&mut self, ev: AgentEvent) {
        match &ev {
            AgentEvent::Text(t) => self.text.push_str(t),
            AgentEvent::Thinking(t) => self.thinking.push_str(t),
            AgentEvent::ToolStart { id, name, summary } => {
                self.flush();
                self.output(&format!("\x1b[36m● {name}\x1b[0m {summary}\n"));
                self.write(
                    json!({"kind": "tool_start", "id": id, "name": name, "summary": summary}),
                );
            }
            AgentEvent::ToolEnd {
                id,
                name,
                output,
                is_error,
            } => {
                self.flush();
                let first = output.lines().next().unwrap_or_default();
                let color = if *is_error { "31" } else { "2" };
                self.output(&format!("  \x1b[{color}m⎿ {}\x1b[0m\n", clip(first, 200)));
                self.write(json!({"kind": "tool_end", "id": id, "name": name,
                    "output": clip(output, MAX_TEXT), "is_error": is_error}));
            }
            AgentEvent::Notice(n) => {
                self.flush();
                self.output(&format!("\x1b[33m! {n}\x1b[0m\n"));
                self.write(json!({"kind": "notice", "text": n}));
            }
            // Usage is written with its cost where it is booked ([`usage`]).
            AgentEvent::ToolProgress { .. }
            | AgentEvent::Usage(_)
            | AgentEvent::Context(_)
            | AgentEvent::Estimate(_)
            | AgentEvent::Status(_) => {}
        }
        let _ = self.out.send(ev);
    }
}

/// `Agent::send` with the session recorded to `rec`.
pub(crate) async fn send(
    agent: &mut Agent,
    rec: Arc<Recorder>,
    text: String,
    events: &UnboundedSender<AgentEvent>,
    approver: &dyn Approver,
    cancel: CancellationToken,
) -> Result<StopReason, ProviderError> {
    let mask = agent.redactor.clone();
    let (tx, rx) = unbounded_channel();
    let tap = Arc::new(Mutex::new(Tap {
        rx,
        out: events.clone(),
        rec: Arc::clone(&rec),
        mask: mask.clone(),
        text: String::new(),
        thinking: String::new(),
    }));
    {
        let t = lock(&tap);
        let model = format!("{}/{}", agent.provider.id(), agent.model);
        t.write(json!({"kind": "prompt", "text": text, "model": model}));
        t.output(&format!("\x1b[1m> {text}\x1b[0m\n"));
    }
    // Direct writes (diffs, approvals, cost) first write out what is still queued.
    let weak = Arc::downgrade(&tap);
    rec.set_sync(Some(Arc::new(move || {
        if let Some(t) = weak.upgrade()
            && let Ok(mut t) = t.try_lock()
        {
            t.pump();
            t.flush();
        }
    })));
    let approver = Recording {
        inner: approver,
        rec: Arc::clone(&rec),
        mask,
    };
    let work = agent.send_plain(text, &tx, &approver, cancel);
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            biased;
            () = std::future::poll_fn(|cx| {
                let mut t = lock(&tap);
                match t.rx.poll_recv(cx) {
                    Poll::Ready(Some(ev)) => {
                        t.on(ev);
                        Poll::Ready(())
                    }
                    _ => Poll::Pending,
                }
            }) => {}
            r = &mut work => break r,
        }
    };
    rec.set_sync(None);
    let mut t = lock(&tap);
    t.pump();
    t.flush();
    let end = match &result {
        Ok(stop) => json!({"kind": "end", "stop": format!("{stop:?}")}),
        Err(e) => json!({"kind": "end", "error": e.to_string()}),
    };
    t.write(end);
    result
}

/// Approvals and the diffs they were asked for.
struct Recording<'a> {
    inner: &'a dyn Approver,
    rec: Arc<Recorder>,
    mask: Redactor,
}

impl Recording<'_> {
    fn write(&self, v: Value) {
        self.rec.write(mask_value(&self.mask, v));
    }

    fn asked(&self, req: &PermissionRequest) {
        self.rec.sync();
        if !req.files.is_empty() {
            self.write(json!({"kind": "diff", "id": req.id, "tool": req.tool, "files": files_json(&req.files)}));
        }
        self.write(
            json!({"kind": "ask", "id": req.id, "tool": req.tool, "summary": req.summary,
            "destructive": req.destructive}),
        );
        self.rec.output(
            &self
                .mask
                .redact(&format!("\x1b[35m? {} {}\x1b[0m\n", req.tool, req.summary)),
        );
    }

    fn answered(
        &self,
        req: &PermissionRequest,
        reply: PermissionReply,
        files: &BTreeMap<String, FileDecision>,
    ) {
        let reply = match reply {
            PermissionReply::Allow => "allow",
            PermissionReply::AllowAlways => "always",
            PermissionReply::Deny => "deny",
        };
        let files: serde_json::Map<String, Value> = files
            .iter()
            .map(|(p, d)| {
                let d = match d {
                    FileDecision::Accept => "accept",
                    FileDecision::Reject => "reject",
                    FileDecision::Edit(_) => "edit",
                };
                (p.clone(), json!(d))
            })
            .collect();
        self.write(json!({"kind": "approval", "id": req.id, "tool": req.tool, "reply": reply, "files": files}));
        self.rec.output(&format!("  \x1b[35m→ {reply}\x1b[0m\n"));
    }
}

#[async_trait]
impl Approver for Recording<'_> {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.asked(req);
        let r = self.inner.approve(req).await;
        self.answered(req, r, &BTreeMap::new());
        r
    }

    async fn review(&self, req: &PermissionRequest) -> Review {
        self.asked(req);
        let r = self.inner.review(req).await;
        self.answered(req, r.reply, &r.files);
        r
    }

    async fn choose(&self, q: &crate::ask::Question) -> Option<crate::ask::Choice> {
        self.inner.choose(q).await
    }

    async fn compact(
        &self,
        items: &[crate::ask::CompactItem],
        confirm: bool,
    ) -> crate::ask::CompactChoice {
        self.inner.compact(items, confirm).await
    }
}

/// An edit that runs without asking: its diff is recorded before it is written.
pub(crate) async fn auto_diff(agent: &Agent, tool: &dyn kor_tools::Tool, input: &Value, id: &str) {
    let Some(rec) = &agent.ctx.record else { return };
    if tool.kind() != kor_tools::ToolKind::Edit {
        return;
    }
    let Some(files) = tool
        .propose(input, &agent.ctx)
        .await
        .filter(|f| !f.is_empty())
    else {
        return;
    };
    rec.sync();
    let v = json!({"kind": "diff", "id": id, "tool": tool.name(), "files": files_json(&files)});
    rec.write(mask_value(&agent.redactor, v));
}

/// Tokens and cost of one model call (`cost`: dollars and whether the catalog estimated it).
pub(crate) fn usage(agent: &Agent, usage: &Usage, cost: Option<(f64, bool)>) {
    let Some(rec) = &agent.ctx.record else { return };
    rec.sync();
    let v = json!({
        "kind": "usage",
        "model": format!("{}/{}", agent.provider.id(), agent.model),
        "input": usage.input,
        "output": usage.output,
        "cache_read": usage.cache_read,
        "cache_write": usage.cache_write,
        "reasoning": usage.reasoning,
        "cost_usd": cost.map(|c| c.0),
        "cost_estimated": cost.is_some_and(|c| c.1),
    });
    rec.write(mask_value(&agent.redactor, v));
}
