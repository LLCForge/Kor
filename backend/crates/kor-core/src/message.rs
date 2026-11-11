use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Author of a message in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    /// Results of tool calls, for providers that model them as a separate role.
    Tool,
}

/// Where image bytes come from. Adapters convert to the provider's wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

/// One piece of a message. A message is an ordered list of parts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
    },
    /// A file attached by path; adapters inline or upload it.
    File {
        path: String,
        media_type: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// Model reasoning. `signature` round-trips provider integrity tokens untouched.
    Thinking {
        text: String,
        signature: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<ContentPart>,
}

impl Message {
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![ContentPart::Text { text: text.into() }],
        }
    }

    /// Concatenated plain text of all `Text` parts.
    pub fn plain_text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// A tool the model may call. `input_schema` is a JSON Schema object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_round_trips_with_tagged_parts() {
        let msg = Message {
            role: Role::Assistant,
            parts: vec![
                ContentPart::Text { text: "hi".into() },
                ContentPart::ToolUse {
                    id: "t1".into(),
                    name: "read_file".into(),
                    input: json!({"path": "a.rs"}),
                },
            ],
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["parts"][1]["type"], "tool_use");
        let back: Message = serde_json::from_value(v).unwrap();
        assert_eq!(back, msg);
        assert_eq!(back.plain_text(), "hi");
    }
}
