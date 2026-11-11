//! Reasoning ("thinking") control for one model call. Provider adapters translate it into each
//! API's own form; a provider without such a control ignores it.

use serde::{Deserialize, Serialize};

/// Effort level, the union of what the major APIs accept today (`OpenAI`: none..xhigh/max,
/// Anthropic `output_config.effort`: low..max, Gemini `thinkingLevel`: minimal..high). Adapters
/// clamp a level their API lacks to the nearest one it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// Effort ⇄ thinking-token budget, for APIs that take only one form. Each level owns the budgets
/// up to its own value, so `effort → budget → effort` round-trips. 24 576 is Gemini 2.5 Flash's
/// budget ceiling; 32 768 is Gemini 2.5 Pro's.
const BUDGETS: [(ReasoningEffort, u32); 7] = [
    (ReasoningEffort::None, 0),
    (ReasoningEffort::Minimal, 1_024),
    (ReasoningEffort::Low, 4_096),
    (ReasoningEffort::Medium, 8_192),
    (ReasoningEffort::High, 16_384),
    (ReasoningEffort::Xhigh, 24_576),
    (ReasoningEffort::Max, 32_768),
];

impl ReasoningEffort {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Thinking-token budget this level stands for (see `BUDGETS`).
    #[must_use]
    pub fn budget(self) -> u32 {
        BUDGETS
            .iter()
            .find(|(e, _)| *e == self)
            .map_or(0, |(_, b)| *b)
    }

    /// Smallest level whose budget covers `tokens`; above the table it is `Max`.
    #[must_use]
    pub fn from_budget(tokens: u32) -> Self {
        BUDGETS
            .iter()
            .find(|(_, b)| tokens <= *b)
            .map_or(Self::Max, |(e, _)| *e)
    }
}

/// Requested reasoning: an effort level, a token budget, or both (each adapter sends the form
/// its API takes, deriving the other one when needed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reasoning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

impl Reasoning {
    #[must_use]
    pub fn effort(effort: ReasoningEffort) -> Self {
        Self {
            effort: Some(effort),
            budget_tokens: None,
        }
    }

    #[must_use]
    pub fn budget(tokens: u32) -> Self {
        Self {
            effort: None,
            budget_tokens: Some(tokens),
        }
    }

    /// The effort level, else the one derived from the budget; `None` when neither is set.
    #[must_use]
    pub fn level(self) -> Option<ReasoningEffort> {
        self.effort
            .or_else(|| self.budget_tokens.map(ReasoningEffort::from_budget))
    }

    /// The token budget, else the one derived from the effort; `None` when neither is set.
    #[must_use]
    pub fn tokens(self) -> Option<u32> {
        self.budget_tokens
            .or_else(|| self.effort.map(ReasoningEffort::budget))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_and_budget_round_trip() {
        for (e, b) in BUDGETS {
            assert_eq!(e.budget(), b);
            assert_eq!(ReasoningEffort::from_budget(b), e);
        }
        assert_eq!(ReasoningEffort::from_budget(5_000), ReasoningEffort::Medium);
        assert_eq!(
            ReasoningEffort::from_budget(1_000_000),
            ReasoningEffort::Max
        );
        assert_eq!(Reasoning::budget(2_000).level(), Some(ReasoningEffort::Low));
        assert_eq!(
            Reasoning::effort(ReasoningEffort::High).tokens(),
            Some(16_384)
        );
        assert_eq!(Reasoning::default().level(), None);
    }

    #[test]
    fn serde_uses_lowercase_names() {
        let r: Reasoning = serde_json::from_str(r#"{"effort":"xhigh"}"#).unwrap_or_default();
        assert_eq!(r, Reasoning::effort(ReasoningEffort::Xhigh));
    }
}
