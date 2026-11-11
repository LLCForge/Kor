//! Task list: `TaskCreate`, `TaskGet`, `TaskUpdate`, `TaskList`.
//!
//! Tasks are JSON files under `<project>/.kor/tasks`. This sits beside `TodoWrite`,
//! which is the in-memory session checklist.

use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, parse};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Task {
    id: String,
    subject: String,
    #[serde(default)]
    description: String,
    status: String,
}

fn dir(ctx: &ToolCtx) -> PathBuf {
    ctx.root.join(".kor").join("tasks")
}

fn path(ctx: &ToolCtx, id: &str) -> Result<PathBuf, ToolOutput> {
    if id.is_empty() || id.contains(['/', '\\', '.', '\0']) {
        return Err(ToolOutput::err("invalid task id"));
    }
    Ok(dir(ctx).join(format!("{id}.json")))
}

fn load(ctx: &ToolCtx, id: &str) -> Result<Task, ToolOutput> {
    let p = path(ctx, id)?;
    let text = fs::read_to_string(&p).map_err(|_| ToolOutput::err(format!("no task {id}")))?;
    serde_json::from_str(&text).map_err(|e| ToolOutput::err(e.to_string()))
}

fn save(ctx: &ToolCtx, task: &Task) -> Result<(), ToolOutput> {
    let d = dir(ctx);
    fs::create_dir_all(&d).map_err(|e| ToolOutput::err(e.to_string()))?;
    let p = path(ctx, &task.id)?;
    fs::write(p, serde_json::to_string_pretty(task).unwrap_or_default())
        .map_err(|e| ToolOutput::err(e.to_string()))
}

fn next_id(ctx: &ToolCtx) -> String {
    let mut n = 1u64;
    if let Ok(rd) = fs::read_dir(dir(ctx)) {
        for ent in rd.flatten() {
            if let Some(stem) = ent.path().file_stem().and_then(|s| s.to_str())
                && let Ok(v) = stem.parse::<u64>()
            {
                n = n.max(v + 1);
            }
        }
    }
    n.to_string()
}

#[derive(Debug)]
pub(crate) struct TaskCreate;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateIn {
    subject: String,
    #[serde(default)]
    description: String,
}

#[async_trait]
impl Tool for TaskCreate {
    fn name(&self) -> &'static str {
        "TaskCreate"
    }
    fn rule_name(&self) -> &'static str {
        "TaskCreate"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Create a task in the project task list (.kor/tasks). Returns the new id."
                .into(),
            input_schema: json!({"type":"object","properties":{
                "subject":{"type":"string"},
                "description":{"type":"string"}
            },"required":["subject"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["subject"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: CreateIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.subject.trim().is_empty() {
            return ToolOutput::err("subject is empty");
        }
        let task = Task {
            id: next_id(ctx),
            subject: a.subject,
            description: a.description,
            status: "pending".into(),
        };
        let id = task.id.clone();
        if let Err(e) = save(ctx, &task) {
            return e;
        }
        ToolOutput::ok(format!("created task {id}"))
    }
}

#[derive(Debug)]
pub(crate) struct TaskGet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdIn {
    task_id: String,
}

#[async_trait]
impl Tool for TaskGet {
    fn name(&self) -> &'static str {
        "TaskGet"
    }
    fn rule_name(&self) -> &'static str {
        "TaskGet"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Read one task by id.".into(),
            input_schema: json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["task_id"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: IdIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        match load(ctx, &a.task_id) {
            Ok(t) => ToolOutput::ok(serde_json::to_string(&t).unwrap_or_default()),
            Err(e) => e,
        }
    }
}

#[derive(Debug)]
pub(crate) struct TaskUpdate;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateIn {
    task_id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[async_trait]
impl Tool for TaskUpdate {
    fn name(&self) -> &'static str {
        "TaskUpdate"
    }
    fn rule_name(&self) -> &'static str {
        "TaskUpdate"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Update a task's status, subject, or description. Status is pending, in_progress, or completed.".into(),
            input_schema: json!({"type":"object","properties":{
                "task_id":{"type":"string"},
                "status":{"type":"string"},
                "subject":{"type":"string"},
                "description":{"type":"string"}
            },"required":["task_id"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["task_id"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: UpdateIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let mut task = match load(ctx, &a.task_id) {
            Ok(t) => t,
            Err(e) => return e,
        };
        if let Some(status) = a.status {
            if !matches!(status.as_str(), "pending" | "in_progress" | "completed") {
                return ToolOutput::err("status must be pending, in_progress, or completed");
            }
            task.status = status;
        }
        if let Some(s) = a.subject {
            task.subject = s;
        }
        if let Some(d) = a.description {
            task.description = d;
        }
        if let Err(e) = save(ctx, &task) {
            return e;
        }
        ToolOutput::ok(format!("updated task {}", task.id))
    }
}

#[derive(Debug)]
pub(crate) struct TaskList;

#[async_trait]
impl Tool for TaskList {
    fn name(&self) -> &'static str {
        "TaskList"
    }
    fn rule_name(&self) -> &'static str {
        "TaskList"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "List tasks in the project task list.".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }
    fn target(&self, _: &Value) -> Option<String> {
        Some(".kor/tasks".into())
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        if let Err(e) = parse::<Value>(input) {
            return e;
        }
        let mut tasks = Vec::new();
        if let Ok(rd) = fs::read_dir(dir(ctx)) {
            for ent in rd.flatten() {
                if ent.path().extension().is_some_and(|e| e == "json")
                    && let Ok(text) = fs::read_to_string(ent.path())
                    && let Ok(task) = serde_json::from_str::<Task>(&text)
                {
                    tasks.push(task);
                }
            }
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        if tasks.is_empty() {
            return ToolOutput::ok("no tasks");
        }
        let lines: Vec<String> = tasks
            .iter()
            .map(|t| format!("{} [{}] {}", t.id, t.status, t.subject))
            .collect();
        ToolOutput::ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_list_update() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let made = TaskCreate
            .call(
                json!({"subject": "port tasks", "description": "local files"}),
                &ctx,
            )
            .await;
        assert!(!made.is_error, "{}", made.content);
        let listed = TaskList.call(json!({}), &ctx).await;
        assert!(listed.content.contains("port tasks"), "{}", listed.content);
        let updated = TaskUpdate
            .call(json!({"task_id": "1", "status": "completed"}), &ctx)
            .await;
        assert!(!updated.is_error, "{}", updated.content);
        let got = TaskGet.call(json!({"task_id": "1"}), &ctx).await;
        assert!(got.content.contains("completed"), "{}", got.content);
    }
}
