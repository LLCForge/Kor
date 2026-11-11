use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    MaxTokens,
    ToolUse,
    ContentFiltered,
    Cancelled,
    Error,
}

/// One event of a provider response. Every completion is a stream of these;
/// non-streaming providers emit a single burst.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    /// Integrity token for the preceding thinking block; must be sent back unchanged.
    ThinkingSignature {
        signature: String,
    },
    ToolCallStart {
        index: u32,
        id: String,
        name: String,
    },
    /// Partial JSON of the tool input; concatenate per `index`.
    ToolCallDelta {
        index: u32,
        json: String,
    },
    ToolCallEnd {
        index: u32,
    },
    Usage {
        usage: Usage,
    },
    Citation {
        url: String,
        title: Option<String>,
    },
    Finish {
        reason: FinishReason,
    },
    /// Untouched provider payload, only emitted in debug mode.
    Raw {
        value: Value,
    },
}
