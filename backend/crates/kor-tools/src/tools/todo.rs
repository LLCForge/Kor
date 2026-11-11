//! `TodoWrite`.

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Todo, Tool, ToolCtx, ToolKind, ToolOutput, lock, parse};

#[derive(Debug)]
pub(crate) struct TodoWrite;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    content: String,
    status: String,
    #[serde(rename = "activeForm")]
    active_form: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct In {
    todos: Vec<Item>,
}

fn fmt(todos: &[Todo]) -> String {
    if todos.is_empty() {
        return "(empty)".into();
    }
    todos
        .iter()
        .map(|t| format!("- [{}] {} ({})", t.status, t.content, t.active_form))
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait]
impl Tool for TodoWrite {
    fn name(&self) -> &'static str {
        "TodoWrite"
    }
    fn rule_name(&self) -> &'static str {
        "TodoWrite"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Replace the session todo list. Each item has content, status (pending, in_progress, completed) and activeForm (present continuous, shown while the item is in progress).".into(),
            input_schema: json!({"type": "object", "properties": {
                "todos": {"type": "array", "items": {"type": "object", "properties": {
                    "content": {"type": "string"},
                    "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]},
                    "activeForm": {"type": "string"}
                }, "required": ["content", "status", "activeForm"], "additionalProperties": false}}
            }, "required": ["todos"], "additionalProperties": false}),
        }
    }
    fn target(&self, _input: &Value) -> Option<String> {
        None
    }
    fn summary(&self, input: &Value) -> String {
        let n = input["todos"].as_array().map_or(0, Vec::len);
        format!("{n} todos")
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: In = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        for t in &a.todos {
            if !matches!(t.status.as_str(), "pending" | "in_progress" | "completed") {
                return ToolOutput::err(format!(
                    "invalid status `{}`; use pending, in_progress or completed",
                    t.status
                ));
            }
        }
        let new: Vec<Todo> = a
            .todos
            .into_iter()
            .map(|t| Todo {
                content: t.content,
                status: t.status,
                active_form: t.active_form,
            })
            .collect();
        let mut todos = lock(&ctx.todos);
        let old = std::mem::replace(&mut *todos, new);
        ToolOutput::ok(format!("previous:\n{}\n\nnow:\n{}", fmt(&old), fmt(&todos)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaces_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let out = TodoWrite
            .call(
                json!({"todos": [{"content": "fix", "status": "in_progress", "activeForm": "Fixing"}]}),
                &ctx,
            )
            .await;
        assert!(
            !out.is_error && out.content.contains("Fixing"),
            "{}",
            out.content
        );
        assert_eq!(lock(&ctx.todos).len(), 1);
        assert!(
            TodoWrite
                .call(
                    json!({"todos": [{"content": "x", "status": "nope", "activeForm": "X"}]}),
                    &ctx
                )
                .await
                .is_error
        );
    }
}
