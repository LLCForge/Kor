//! Request JSON of each adapter with and without `ChatRequest::reasoning`.

use kor_core::{Message, Reasoning, ReasoningEffort as E, Role};
use serde_json::{Value, json};

use crate::ChatRequest;

fn req(model: &str, reasoning: Option<Reasoning>) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        messages: vec![Message::text(Role::User, "hi")],
        max_tokens: Some(4_000),
        temperature: Some(0.2),
        reasoning,
        ..ChatRequest::default()
    }
}

fn compat(r: Option<Reasoning>) -> Value {
    super::openai_compat::body(&req("m", r), "p").unwrap_or_default()
}

fn responses(r: Option<Reasoning>) -> Value {
    super::openai::body(&req("m", r), "p").unwrap_or_default()
}

fn anthropic(r: Option<Reasoning>) -> Value {
    super::anthropic::body(&req("m", r), "p").unwrap_or_default()
}

fn gemini(model: &str, r: Option<Reasoning>) -> Value {
    super::gemini::body(&req(model, r), "p").unwrap_or_default()
}

#[test]
fn without_reasoning_nothing_is_sent() {
    assert!(compat(None).get("reasoning_effort").is_none());
    assert!(responses(None).get("reasoning").is_none());
    let a = anthropic(None);
    assert!(a.get("thinking").is_none() && a.get("output_config").is_none());
    assert_eq!(a["temperature"], json!(0.2_f32));
    assert!(
        gemini("gemini-3-pro", None)["generationConfig"]
            .get("thinkingConfig")
            .is_none()
    );
}

#[test]
fn openai_chat_and_responses_take_effort() {
    assert_eq!(
        compat(Some(Reasoning::effort(E::Xhigh)))["reasoning_effort"],
        "xhigh"
    );
    assert_eq!(
        compat(Some(Reasoning::budget(0)))["reasoning_effort"],
        "none"
    );
    assert_eq!(
        responses(Some(Reasoning::budget(3_000)))["reasoning"],
        json!({"effort": "low"})
    );
}

#[test]
fn anthropic_effort_is_adaptive_thinking() {
    let b = anthropic(Some(Reasoning::effort(E::High)));
    assert_eq!(b["thinking"], json!({"type": "adaptive"}));
    assert_eq!(b["output_config"], json!({"effort": "high"}));
    assert!(b.get("temperature").is_none());
    assert_eq!(b["max_tokens"], 4_000);
    let b = anthropic(Some(Reasoning::effort(E::Minimal)));
    assert_eq!(b["output_config"]["effort"], "low");
}

#[test]
fn anthropic_budget_respects_minimum_and_max_tokens() {
    let b = anthropic(Some(Reasoning::budget(500)));
    assert_eq!(
        b["thinking"],
        json!({"type": "enabled", "budget_tokens": 1_024})
    );
    assert_eq!(b["max_tokens"], 4_000);
    // budget_tokens must stay below max_tokens.
    let b = anthropic(Some(Reasoning::budget(10_000)));
    assert_eq!(b["thinking"]["budget_tokens"], 10_000);
    assert_eq!(b["max_tokens"], 14_000);
    // A budget wins over an effort; 0 turns thinking off.
    let both = Reasoning {
        effort: Some(E::Max),
        budget_tokens: Some(2_048),
    };
    assert_eq!(anthropic(Some(both))["thinking"]["type"], "enabled");
    assert!(
        anthropic(Some(Reasoning::budget(0)))
            .get("thinking")
            .is_none()
    );
}

#[test]
fn gemini_level_on_3_budget_on_2() {
    let g = |m, r| gemini(m, Some(r))["generationConfig"]["thinkingConfig"].clone();
    assert_eq!(
        g("gemini-3-pro", Reasoning::effort(E::Medium)),
        json!({"thinkingLevel": "MEDIUM"})
    );
    assert_eq!(
        g("gemini-3-pro", Reasoning::effort(E::Xhigh)),
        json!({"thinkingLevel": "HIGH"})
    );
    assert_eq!(
        g("gemini-2.5-flash", Reasoning::effort(E::High)),
        json!({"thinkingBudget": 16_384})
    );
    assert_eq!(
        g("gemini-3-pro", Reasoning::budget(0)),
        json!({"thinkingBudget": 0})
    );
}
