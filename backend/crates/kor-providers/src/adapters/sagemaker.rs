//! §4.2: the `sagemaker` adapter. Requests are signed and sent by [`super::bedrock::Aws`] (one
//! SigV4 client for both AWS services); this is the endpoint's answer: an OpenAI-compatible
//! container (vLLM, LMI, TGI) answering `POST /endpoints/{name}/invocations` whole.

use kor_core::{FinishReason, StreamEvent, Usage};
use serde_json::Value;

/// An OpenAI Chat Completions answer (SageMaker containers) as stream events.
pub(crate) fn chat_events(v: &Value) -> Option<Vec<StreamEvent>> {
    let choice = v["choices"].get(0)?;
    let msg = &choice["message"];
    let mut out = Vec::new();
    if let Some(t) = msg["content"].as_str().filter(|t| !t.is_empty()) {
        out.push(StreamEvent::TextDelta { text: t.to_owned() });
    }
    for (i, c) in msg["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let index = u32::try_from(i).unwrap_or(u32::MAX);
        out.push(StreamEvent::ToolCallStart {
            index,
            id: c["id"].as_str().unwrap_or_default().to_owned(),
            name: c["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        });
        out.push(StreamEvent::ToolCallDelta {
            index,
            json: c["function"]["arguments"]
                .as_str()
                .unwrap_or("{}")
                .to_owned(),
        });
        out.push(StreamEvent::ToolCallEnd { index });
    }
    out.push(StreamEvent::Usage {
        usage: Usage {
            input: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            ..Usage::default()
        },
    });
    out.push(StreamEvent::Finish {
        reason: match choice["finish_reason"].as_str() {
            Some("tool_calls") => FinishReason::ToolUse,
            Some("length") => FinishReason::MaxTokens,
            Some("content_filter") => FinishReason::ContentFiltered,
            _ => FinishReason::Stop,
        },
    });
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sagemaker_chat_answer() {
        let v = json!({"choices": [{"message": {"content": "ok", "tool_calls": [{"id": "c", "function": {"name": "Bash", "arguments": "{\"cmd\":\"ls\"}"}}]}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 3, "completion_tokens": 1}});
        let ev = chat_events(&v).unwrap();
        assert_eq!(ev[0], StreamEvent::TextDelta { text: "ok".into() });
        assert!(ev.contains(&StreamEvent::Finish {
            reason: FinishReason::ToolUse
        }));
    }
}
