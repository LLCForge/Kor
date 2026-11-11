//! Cost control seen from the agent loop (Master Build Prompt §5.1): the smart cost router picks
//! a model per task, every model call is reported, and the budget is checked before each call.
//! The policy (catalog, ledger, config) lives in `kor-runtime`; the loop only follows it.

use std::sync::Arc;

use kor_core::Usage;
use kor_providers::Provider;
use serde_json::Value;

use crate::toolcall::ToolMode;

/// Sent to the reviewing model in draft-then-verify mode.
pub const VERIFY_PROMPT: &str = "[Draft-then-verify] You are reviewing the work above, which a \
cheaper model drafted. Check the changes and the answer for mistakes. Fix real problems with the \
tools; do not redo work that is correct. When everything is right, reply briefly that it is \
verified.";

/// A model to switch to.
#[derive(Clone)]
pub struct Route {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub tool_mode: ToolMode,
    /// Shown to the user: the category and why this model was chosen.
    pub reason: String,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("provider", &self.provider.id())
            .field("model", &self.model)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// The router's answer for one task.
#[derive(Debug, Clone)]
pub struct Plan {
    pub main: Route,
    /// Draft-then-verify: this model reviews once the drafting model is done.
    pub verify: Option<Route>,
}

/// What a limit asks for once it is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnLimit {
    Downgrade,
    Pause,
    Ask,
}

/// Spend state before a model call.
#[derive(Debug, Clone, PartialEq)]
pub enum Spend {
    Ok,
    /// The alert threshold was crossed (reported once); `fields` go to `on-budget-alert`.
    Alert {
        message: String,
        fields: Value,
    },
    /// A limit is reached.
    Over {
        action: OnLimit,
        message: String,
        fields: Value,
    },
}

/// How a task ended, for the router's success history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed,
    /// Cancelled or blocked: says nothing about the model.
    Unknown,
}

/// Time and cost estimate of the task routed last (§27.16), shown before the first model call.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    /// `≈ 4m · ≈ $0.03 · ~12 files` (localized).
    pub line: String,
    /// How it was estimated (localized).
    pub method: String,
    pub secs: u64,
    /// `None` when the model has no known price.
    pub usd: Option<f64>,
    pub files: u32,
    /// Asked before the first model call when the estimate would cross a budget limit.
    pub ask: Option<String>,
    /// Catalog (input, output) USD per million tokens, so a front end can show the spend so far.
    #[serde(skip)]
    pub price: Option<(f64, f64)>,
}

pub trait CostControl: Send + Sync {
    /// Estimate of the task routed last; `None` when estimates are not available.
    fn preflight(&self) -> Option<Preflight> {
        None
    }
    /// After `task_done`: the actual time and cost next to the estimate (§27.16), for the user.
    fn finished(&self) -> Option<String> {
        None
    }
    /// Model(s) for a new task; `None` keeps the current model. Implementations log the decision.
    fn route(&self, task: &str) -> Option<Plan>;
    /// One finished model call: tokens and, when the provider reported it, its cost in USD.
    /// Returns the call's cost as the ledger counts it and whether it is estimated from
    /// catalog prices (for the audit log); `None` when no price is known.
    fn record(&self, model: &str, usage: &Usage, cost_usd: Option<f64>) -> Option<(f64, bool)>;
    /// The agent read or changed `path`; later calls are attributed to its package (§15.15).
    fn touched(&self, _path: &std::path::Path) {}
    /// The task routed last ended.
    fn task_done(&self, outcome: Outcome);
    /// Checked before every model call.
    fn spend(&self) -> Spend;
    /// The cheapest known model, for `on_limit = "downgrade"`.
    fn downgrade(&self) -> Option<Route>;
    /// The user agreed to continue over the limit (`on_limit = "ask"`); stop asking this session.
    fn allow_over(&self);
}
