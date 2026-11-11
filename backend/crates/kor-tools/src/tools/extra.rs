//! Extra tools: questions, plan mode, config, sleep,
//! tool search, background job control, cron records, and structured output.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use kor_core::{PermissionMode, ToolSpec};
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput, lock};

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a 1-based option index. `0` is rejected.
pub fn parse_choice(answer: &str, count: usize) -> Result<usize, String> {
    let n: usize = answer
        .trim()
        .parse()
        .map_err(|_| "answer must be a number".to_owned())?;
    if n == 0 || n > count {
        return Err(format!("choose 1-{count}"));
    }
    Ok(n - 1)
}

#[derive(Debug)]
pub(crate) struct AskUserQuestion;

#[async_trait]
impl Tool for AskUserQuestion {
    fn name(&self) -> &'static str {
        "AskUserQuestion"
    }
    fn rule_name(&self) -> &'static str {
        "AskUserQuestion"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "AskUserQuestion".into(),
            description: "Ask the user one multiple-choice question. Options are 2 to 4 concrete labels; `default` is the 1-based option you recommend (Enter takes it). The user's choice comes back as the result.".into(),
            input_schema: json!({"type":"object","properties":{
                "question":{"type":"string"},
                "options":{"type":"array","items":{"type":"string"},"minItems":2,"maxItems":4},
                "default":{"type":"integer","minimum":1,"maximum":4,"description":"1-based recommended option"},
                "answer":{"type":"string","description":"1-based choice, supplied by the UI"},
                "unanswered":{"type":"boolean","description":"set by Kor when no one could answer"}
            },"required":["question","options"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["question"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let options = strings(&input["options"]);
        if !(2..=4).contains(&options.len()) {
            return ToolOutput::err("options must contain 2 to 4 labels");
        }
        match parse_choice(input["answer"].as_str().unwrap_or("1"), options.len()) {
            Ok(i) if input["unanswered"].as_bool() == Some(true) => ToolOutput::ok(format!(
                "no one could answer here; proceeding with the recommended option: {} (list it as an assumption)",
                options[i]
            )),
            Ok(i) => ToolOutput::ok(format!("selected: {}", options[i])),
            Err(e) => ToolOutput::err(e),
        }
    }
}

fn set_mode(ctx: &ToolCtx, mode: PermissionMode, confirm: bool) -> ToolOutput {
    if !confirm {
        return ToolOutput::err("permission confirm is required");
    }
    *lock(&ctx.plan) = mode;
    ToolOutput::ok(format!("permission mode is {}", mode.as_str()))
}

#[derive(Debug)]
pub(crate) struct EnterPlanMode;
#[derive(Debug)]
pub(crate) struct ExitPlanMode;

#[async_trait]
impl Tool for EnterPlanMode {
    fn name(&self) -> &'static str {
        "EnterPlanMode"
    }
    fn rule_name(&self) -> &'static str {
        "EnterPlanMode"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "EnterPlanMode".into(),
            description: "Switch the session to plan mode after the user confirms.".into(),
            input_schema: json!({"type":"object","properties":{"confirm":{"type":"boolean"}},"required":["confirm"],"additionalProperties":false}),
        }
    }
    fn target(&self, _: &Value) -> Option<String> {
        Some("plan".into())
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        set_mode(
            ctx,
            PermissionMode::Plan,
            input["confirm"].as_bool().unwrap_or(false),
        )
    }
}

#[async_trait]
impl Tool for ExitPlanMode {
    fn name(&self) -> &'static str {
        "ExitPlanMode"
    }
    fn rule_name(&self) -> &'static str {
        "ExitPlanMode"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ExitPlanMode".into(),
            description: "Save your plan (Markdown, with the files you will change) to .kor/plans/ and leave plan mode after the user confirms it. Under the plan lock only the user can approve it.".into(),
            input_schema: json!({"type":"object","properties":{
                "confirm":{"type":"boolean"},
                "title":{"type":"string","description":"Short task title"},
                "plan":{"type":"string","description":"The plan in Markdown: steps and the files each step changes"}
            },"required":["confirm"],"additionalProperties":false}),
        }
    }
    fn target(&self, _: &Value) -> Option<String> {
        Some("default".into())
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        // §28.2: the plan is written to `.kor/plans/` (Kor's own folder, allowed in plan mode).
        let saved = match input["plan"].as_str().filter(|p| !p.trim().is_empty()) {
            Some(body) => {
                let title = input["title"].as_str().unwrap_or("");
                match crate::plans::save(&ctx.root, title, body, ctx.plan_gate.session.as_deref()) {
                    Ok(p) => Some(p),
                    Err(e) => return ToolOutput::err(format!("could not save the plan: {e}")),
                }
            }
            None => None,
        };
        // Under the lock only a user action approves a plan; the model cannot leave plan mode.
        if ctx.plan_gate.lock && !ctx.plan_gate.refresh(&ctx.root) {
            return match saved {
                Some(p) => ToolOutput::ok(format!(
                    "plan saved as {} ({}); write tools stay disabled until the user approves it                      (`/go`, `kor plan approve {}` or the desktop button). Stop here and wait.",
                    p.id,
                    crate::display_path(&p.path),
                    p.id
                )),
                None => ToolOutput::err(
                    "plan mode is locked: call ExitPlanMode with `plan` to save your plan, then wait for the user's approval",
                ),
            };
        }
        let out = set_mode(
            ctx,
            PermissionMode::Default,
            input["confirm"].as_bool().unwrap_or(false),
        );
        match saved {
            Some(p) if !out.is_error => {
                ToolOutput::ok(format!("{}; plan saved as {}", out.content, p.id))
            }
            _ => out,
        }
    }
}

const CONFIG_KEYS: &[&str] = &[
    "theme",
    "editorMode",
    "verbose",
    "preferredNotifChannel",
    "autoCompactEnabled",
    "autoMemoryEnabled",
    "autoDreamEnabled",
    "fileCheckpointingEnabled",
    "showTurnDuration",
];

pub fn config_allowed(key: &str) -> bool {
    CONFIG_KEYS.contains(&key)
}

#[derive(Debug)]
pub(crate) struct ConfigTool;

#[async_trait]
impl Tool for ConfigTool {
    fn name(&self) -> &'static str {
        "Config"
    }
    fn rule_name(&self) -> &'static str {
        "Config"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "Config".into(),
            description: "Read or write a supported settings key (theme, editorMode, verbose, autoCompactEnabled, autoMemoryEnabled, autoDreamEnabled, ...).".into(),
            input_schema: json!({"type":"object","properties":{
                "key":{"type":"string"},
                "value":{"type":"string"}
            },"required":["key"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["key"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let key = input["key"].as_str().unwrap_or("");
        if !config_allowed(key) {
            return ToolOutput::err(format!("refusing unsupported setting `{key}`"));
        }
        let path = ctx.root.join(".kor").join("session-settings.json");
        let mut map: serde_json::Map<String, Value> = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if let Some(value) = input.get("value").and_then(Value::as_str) {
            let _ = fs::create_dir_all(ctx.root.join(".kor"));
            map.insert(key.to_owned(), Value::String(value.to_owned()));
            let _ = fs::write(
                &path,
                serde_json::to_string_pretty(&map).unwrap_or_default(),
            );
            return ToolOutput::ok(format!("set {key}"));
        }
        ToolOutput::ok(
            map.get(key)
                .map_or_else(|| "unset".to_owned(), std::string::ToString::to_string),
        )
    }
}

#[derive(Debug)]
pub(crate) struct SleepTool;

#[async_trait]
impl Tool for SleepTool {
    fn name(&self) -> &'static str {
        "Sleep"
    }
    fn rule_name(&self) -> &'static str {
        "Sleep"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "Sleep".into(),
            description:
                "Wait. The user can interrupt the sleep at any time. Prefer this over Bash sleep."
                    .into(),
            input_schema: json!({"type":"object","properties":{"seconds":{"type":"number"}},"required":["seconds"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["seconds"].as_f64().map(|n| format!("{n}s"))
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let seconds = input["seconds"].as_f64().unwrap_or(0.0);
        if seconds < 0.0 {
            return ToolOutput::err("seconds must be >= 0");
        }
        let millis = (seconds * 1000.0).round();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let dur = Duration::from_millis(millis as u64);
        tokio::select! {
            () = ctx.cancel.cancelled() => ToolOutput::err("sleep interrupted"),
            () = tokio::time::sleep(dur) => ToolOutput::ok(format!("slept {seconds}s")),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ToolSearch;

#[async_trait]
impl Tool for ToolSearch {
    fn name(&self) -> &'static str {
        "ToolSearch"
    }
    fn rule_name(&self) -> &'static str {
        "ToolSearch"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ToolSearch".into(),
            description: "Load deferred tools (listed in the system prompt) so their parameters become available. query: exact names separated by commas or spaces, or a word that appears in their names (e.g. `event`, `reminder`).".into(),
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["query"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let query = input["query"].as_str().unwrap_or("");
        let words: Vec<String> = query
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(|w| w.trim_start_matches("select:").to_ascii_lowercase())
            .filter(|w| w.len() >= 2)
            .collect();
        let about = lock(&ctx.deferred_about).clone();
        let mut found: Vec<String> = lock(&ctx.deferred)
            .iter()
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                let text = about
                    .get(*name)
                    .map(|d| d.to_lowercase())
                    .unwrap_or_default();
                words
                    .iter()
                    .any(|w| lower == *w || lower.contains(w.as_str()) || text.contains(w.as_str()))
            })
            .cloned()
            .collect();
        if found.is_empty() {
            return ToolOutput::err(format!("no deferred tool matches `{query}`"));
        }
        found.sort();
        lock(&ctx.revealed).extend(found.iter().cloned());
        ToolOutput::ok(format!("now available: {}", found.join(", ")))
    }
}

#[derive(Debug)]
pub(crate) struct TaskOutput;
#[derive(Debug)]
pub(crate) struct TaskStop;

#[async_trait]
impl Tool for TaskOutput {
    fn name(&self) -> &'static str {
        "TaskOutput"
    }
    fn rule_name(&self) -> &'static str {
        "TaskOutput"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "TaskOutput".into(),
            description: "Read output of a background task.".into(),
            input_schema: json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["task_id"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let id = input["task_id"].as_str().unwrap_or("");
        match lock(&ctx.jobs).get(id) {
            Some(job) => ToolOutput::ok(if job.output.is_empty() {
                "(no output yet)".into()
            } else {
                job.output.clone()
            }),
            None => ToolOutput::err(format!("no task {id}")),
        }
    }
}

#[async_trait]
impl Tool for TaskStop {
    fn name(&self) -> &'static str {
        "TaskStop"
    }
    fn rule_name(&self) -> &'static str {
        "TaskStop"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "TaskStop".into(),
            description: "Cancel a background task.".into(),
            input_schema: json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["task_id"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let id = input["task_id"].as_str().unwrap_or("");
        match lock(&ctx.jobs).get(id) {
            Some(job) => {
                job.cancel.cancel();
                ToolOutput::ok(format!("stopped {id}"))
            }
            None => ToolOutput::err(format!("no task {id}")),
        }
    }
}

fn cron_dir(ctx: &ToolCtx) -> PathBuf {
    ctx.root.join(".kor").join("cron")
}

#[derive(Debug)]
pub(crate) struct CronCreate;
#[derive(Debug)]
pub(crate) struct CronList;
#[derive(Debug)]
pub(crate) struct CronDelete;

#[async_trait]
impl Tool for CronCreate {
    fn name(&self) -> &'static str {
        "CronCreate"
    }
    fn rule_name(&self) -> &'static str {
        "CronCreate"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "CronCreate".into(),
            description:
                "Record a local cron entry under .kor/cron. Does not install a system crontab."
                    .into(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"schedule":{"type":"string"},"prompt":{"type":"string"}},"required":["id","schedule","prompt"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["id"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let id = input["id"].as_str().unwrap_or("");
        if id.is_empty() || id.contains(['/', '\\']) {
            return ToolOutput::err("invalid id");
        }
        let _ = fs::create_dir_all(cron_dir(ctx));
        let body = json!({"id": id, "schedule": input["schedule"], "prompt": input["prompt"]});
        fs::write(cron_dir(ctx).join(format!("{id}.json")), body.to_string()).map_or_else(
            |e| ToolOutput::err(e.to_string()),
            |()| ToolOutput::ok(format!("created cron {id}")),
        )
    }
}

#[async_trait]
impl Tool for CronList {
    fn name(&self) -> &'static str {
        "CronList"
    }
    fn rule_name(&self) -> &'static str {
        "CronList"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "CronList".into(),
            description: "List local cron records.".into(),
            input_schema: json!({"type":"object","properties":{}}),
        }
    }
    fn target(&self, _: &Value) -> Option<String> {
        Some(".kor/cron".into())
    }
    async fn call(&self, _: Value, ctx: &ToolCtx) -> ToolOutput {
        let mut names = Vec::new();
        if let Ok(rd) = fs::read_dir(cron_dir(ctx)) {
            for ent in rd.flatten() {
                if let Some(s) = ent.path().file_stem().and_then(|s| s.to_str()) {
                    names.push(s.to_owned());
                }
            }
        }
        names.sort();
        ToolOutput::ok(if names.is_empty() {
            "no cron jobs".into()
        } else {
            names.join("\n")
        })
    }
}

#[async_trait]
impl Tool for CronDelete {
    fn name(&self) -> &'static str {
        "CronDelete"
    }
    fn rule_name(&self) -> &'static str {
        "CronDelete"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "CronDelete".into(),
            description: "Delete a local cron record.".into(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["id"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let id = input["id"].as_str().unwrap_or("");
        let path = cron_dir(ctx).join(format!("{id}.json"));
        if !path.is_file() {
            return ToolOutput::err("no such cron job");
        }
        fs::remove_file(path).map_or_else(
            |e| ToolOutput::err(e.to_string()),
            |()| ToolOutput::ok(format!("deleted {id}")),
        )
    }
}

pub fn synthetic_output(schema_ok: bool, payload: &Value) -> Result<Value, String> {
    if !schema_ok {
        return Err("payload does not match the output schema".into());
    }
    if !payload.is_object() {
        return Err("structured output must be an object".into());
    }
    Ok(payload.clone())
}

#[derive(Debug)]
pub(crate) struct SyntheticOutput;

#[async_trait]
impl Tool for SyntheticOutput {
    fn name(&self) -> &'static str {
        "SyntheticOutput"
    }
    fn rule_name(&self) -> &'static str {
        "SyntheticOutput"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "SyntheticOutput".into(),
            description: "Return a JSON object that matches the requested schema.".into(),
            input_schema: json!({"type":"object","properties":{"payload":{"type":"object"},"valid":{"type":"boolean"}},"required":["payload"],"additionalProperties":false}),
        }
    }
    fn target(&self, _: &Value) -> Option<String> {
        Some("structured".into())
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let valid = input["valid"].as_bool().unwrap_or(true);
        match synthetic_output(valid, &input["payload"]) {
            Ok(v) => ToolOutput::ok(v.to_string()),
            Err(e) => ToolOutput::err(e),
        }
    }
}

/// `sed -i` that targets `..` is denied, matching the in-place edit guard.
pub fn deny_sed_escape(command: &str) -> bool {
    let lower = command.trim();
    lower.starts_with("sed")
        && lower.contains("-i")
        && (lower.contains("../") || lower.contains("..\\"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_visible;

    #[test]
    fn choice_parser_and_config_allowlist() {
        assert_eq!(parse_choice("2", 3).unwrap(), 1);
        assert!(parse_choice("0", 3).is_err());
        assert!(config_allowed("theme"));
        assert!(config_allowed("autoDreamEnabled"));
        assert!(!config_allowed("apiKey"));
    }

    #[tokio::test]
    async fn plan_mode_sleep_cron_and_tool_search() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        assert!(
            EnterPlanMode
                .call(json!({"confirm": false}), &ctx)
                .await
                .is_error
        );
        assert!(
            !EnterPlanMode
                .call(json!({"confirm": true}), &ctx)
                .await
                .is_error
        );
        assert_eq!(*lock(&ctx.plan), PermissionMode::Plan);
        assert!(
            !ExitPlanMode
                .call(json!({"confirm": true}), &ctx)
                .await
                .is_error
        );
        assert_eq!(*lock(&ctx.plan), PermissionMode::Default);
        ctx.cancel.cancel();
        let slept = SleepTool.call(json!({"seconds": 30}), &ctx).await;
        assert!(slept.content.contains("interrupted"), "{}", slept.content);
        assert!(
            !CronCreate
                .call(
                    json!({"id":"daily","schedule":"0 9 * * *","prompt":"hi"}),
                    &ctx
                )
                .await
                .is_error
        );
        let listed = CronList.call(json!({}), &ctx).await;
        assert!(listed.content.contains("daily"), "{}", listed.content);
        assert!(!CronDelete.call(json!({"id":"daily"}), &ctx).await.is_error);
        lock(&ctx.deferred).insert("Grep".into());
        assert!(!tool_visible(&ctx, "Grep"));
        assert!(
            !ToolSearch
                .call(json!({"query":"Grep"}), &ctx)
                .await
                .is_error
        );
        assert!(tool_visible(&ctx, "Grep"));
        lock(&ctx.deferred).extend([
            "create_event".to_owned(),
            "search_events".to_owned(),
            "weather".to_owned(),
        ]);
        let out = ToolSearch.call(json!({"query":"event"}), &ctx).await;
        assert_eq!(out.content, "now available: create_event, search_events");
        assert!(!tool_visible(&ctx, "weather"));
        assert!(ToolSearch.call(json!({"query":"zz"}), &ctx).await.is_error);
        lock(&ctx.deferred_about).insert("weather".into(), "Current weather and a forecast".into());
        let out = ToolSearch.call(json!({"query":"forecast"}), &ctx).await;
        assert_eq!(out.content, "now available: weather");
        assert!(deny_sed_escape("sed -i 's/a/b/' ../secret"));
        assert!(!deny_sed_escape("sed -n '1p' file"));
        assert!(synthetic_output(false, &json!({})).is_err());
        assert!(synthetic_output(true, &json!({"ok": true})).is_ok());
    }
}
