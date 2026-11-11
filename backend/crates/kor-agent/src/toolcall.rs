//! Model-agnostic tool calling.
//!
//! Models differ in three ways this module absorbs:
//! 1. **Names.** Models are trained on their own tool names (`shell`, `run_terminal_cmd`,
//!    `str_replace_editor`, `write_to_file`, `read_file`, …). Calls to a known foreign name
//!    are routed to the matching Kor tool.
//! 2. **Arguments.** `target_file` vs `file_path`, `cmd` vs `command`, `old_str` vs
//!    `old_string`… Arguments are renamed to what the target tool's schema declares;
//!    unknown extra keys are dropped instead of failing the call.
//! 3. **Protocol.** Models without native tool calling (or providers that reject the
//!    `tools` field) get the tools described in the system prompt and answer with text
//!    blocks. The common formats are parsed: Hermes/Qwen `<tool_call>`, Llama
//!    `<function=…>`, Mistral `[TOOL_CALLS]`, and fenced JSON.

use std::fmt::Write as _;
use std::sync::LazyLock;

use kor_core::ToolSpec;
use regex::Regex;
use serde_json::{Map, Value, json};

/// How tools reach the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolMode {
    /// Provider-native tool calling (`tools` field, structured calls).
    #[default]
    Native,
    /// Tools described in the system prompt; calls parsed from text.
    Text,
}

impl ToolMode {
    /// Native when the provider manifest declares tool calling, text otherwise.
    pub fn for_provider(tool_calling: bool) -> Self {
        if tool_calling {
            Self::Native
        } else {
            Self::Text
        }
    }
}

/// Foreign tool name (lowercase) → Kor tool name.
const ALIASES: &[(&str, &str)] = &[
    // read
    ("read_file", "Read"),
    ("readfile", "Read"),
    ("read", "Read"),
    ("view", "Read"),
    ("view_file", "Read"),
    ("open_file", "Read"),
    ("cat", "Read"),
    ("file_read", "Read"),
    ("read_text_file", "Read"),
    ("get_file_contents", "Read"),
    ("read_many_files", "Read"),
    // write
    ("write_file", "Write"),
    ("write", "Write"),
    ("create_file", "Write"),
    ("write_to_file", "Write"),
    ("file_write", "Write"),
    ("save_file", "Write"),
    // edit
    ("edit_file", "Edit"),
    ("edit", "Edit"),
    ("replace_in_file", "Edit"),
    ("str_replace", "Edit"),
    ("search_replace", "Edit"),
    ("modify_file", "Edit"),
    ("replace", "Edit"),
    ("file_edit", "Edit"),
    ("multi_edit", "MultiEdit"),
    // delete / move
    ("delete_file", "Delete"),
    ("remove_file", "Delete"),
    ("move_file", "Move"),
    ("rename_file", "Move"),
    // desktop
    ("mouse_drag", "left_click_drag"),
    ("drag", "left_click_drag"),
    ("mouse_hover", "mouse_move"),
    // shell
    ("bash", "Bash"),
    ("shell", "Bash"),
    ("run_shell_command", "Bash"),
    ("run_terminal_cmd", "Bash"),
    ("execute_command", "Bash"),
    ("run_command", "Bash"),
    ("terminal", "Bash"),
    ("exec", "Bash"),
    ("sh", "Bash"),
    ("local_shell", "Bash"),
    ("container.exec", "Bash"),
    ("execute_bash", "Bash"),
    ("run_in_terminal", "Bash"),
    // search
    ("glob", "Glob"),
    ("find_files", "Glob"),
    ("file_search", "Glob"),
    ("find", "Glob"),
    ("grep", "Grep"),
    ("search", "Grep"),
    ("search_files", "Grep"),
    ("grep_search", "Grep"),
    ("ripgrep", "Grep"),
    ("rg", "Grep"),
    ("find_in_files", "Grep"),
    ("codebase_search", "Grep"),
    ("search_file_content", "Grep"),
    // list
    ("ls", "list"),
    ("list_dir", "list"),
    ("list_directory", "list"),
    ("list_files", "list"),
    ("listdir", "list"),
    // web
    ("web_fetch", "WebFetch"),
    ("fetch", "WebFetch"),
    ("fetch_url", "WebFetch"),
    ("open_url", "WebFetch"),
    ("read_url", "WebFetch"),
    ("browse", "WebFetch"),
    ("web_search", "WebSearch"),
    ("search_web", "WebSearch"),
    ("google_search", "WebSearch"),
    ("internet_search", "WebSearch"),
    ("brave_search", "WebSearch"),
    ("duckduckgo_search", "WebSearch"),
    // todo
    ("todo_write", "TodoWrite"),
    ("update_todos", "TodoWrite"),
    ("write_todos", "TodoWrite"),
    // patch
    ("apply_patch", "apply_patch"),
    ("applypatch", "apply_patch"),
    ("patch", "apply_patch"),
];

/// Kor parameter → synonyms models use for it.
const SYNONYMS: &[(&str, &[&str])] = &[
    (
        "file_path",
        &[
            "path",
            "filepath",
            "file",
            "filename",
            "target_file",
            "absolute_path",
            "relative_path",
            "file_name",
        ],
    ),
    (
        "path",
        &[
            "file_path",
            "directory",
            "dir",
            "dir_path",
            "target_directory",
            "folder",
            "relative_workspace_path",
            "directory_path",
        ],
    ),
    (
        "content",
        &[
            "contents",
            "text",
            "file_text",
            "code",
            "data",
            "new_content",
            "body",
        ],
    ),
    (
        "old_string",
        &[
            "old_str",
            "old",
            "search",
            "find",
            "old_text",
            "original",
            "target",
            "old_content",
        ],
    ),
    (
        "new_string",
        &[
            "new_str",
            "new",
            "replace",
            "replacement",
            "new_text",
            "updated",
            "new_content",
        ],
    ),
    (
        "command",
        &[
            "cmd",
            "script",
            "commandline",
            "command_line",
            "shell_command",
            "code",
        ],
    ),
    (
        "pattern",
        &[
            "query",
            "regex",
            "search_term",
            "glob_pattern",
            "q",
            "keyword",
            "search_query",
        ],
    ),
    ("url", &["link", "uri", "href", "address"]),
    ("prompt", &["question", "instruction", "goal", "task"]),
    (
        "query",
        &["q", "search_query", "keywords", "search_term", "term"],
    ),
    ("todos", &["items", "tasks", "todo_list"]),
    ("patch", &["input", "diff", "patch_text"]),
];

fn schema_props(spec: &ToolSpec) -> Option<&Map<String, Value>> {
    spec.input_schema
        .get("properties")
        .and_then(Value::as_object)
}

/// `["bash", "-lc", "ls -la"]` → `ls -la`; `["git", "status"]` → `git status`.
fn command_from_array(a: &[Value]) -> String {
    let parts: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
    let shells = [
        "bash",
        "sh",
        "zsh",
        "pwsh",
        "powershell",
        "cmd",
        "/bin/bash",
        "/bin/sh",
    ];
    match parts.as_slice() {
        [sh, flag, script] if shells.contains(sh) && (flag.starts_with('-') || *flag == "/c") => {
            (*script).to_owned()
        }
        _ => parts.join(" "),
    }
}

/// Anthropic's text-editor tool (`str_replace_editor` / `str_replace_based_edit_tool`)
/// multiplexes on `command`.
fn text_editor(input: &Value) -> Result<(&'static str, Value), String> {
    let path = input["path"].clone();
    match input["command"].as_str().unwrap_or_default() {
        "view" => {
            let mut v = json!({ "file_path": path });
            if let Some(r) = input["view_range"].as_array()
                && let (Some(a), Some(b)) = (
                    r.first().and_then(Value::as_u64),
                    r.get(1).and_then(Value::as_i64),
                )
            {
                v["offset"] = a.into();
                if b > 0 {
                    v["limit"] = (u64::try_from(b).unwrap_or(0).saturating_sub(a) + 1).into();
                }
            }
            Ok(("Read", v))
        }
        "create" => Ok((
            "Write",
            json!({"file_path": path, "content": input["file_text"]}),
        )),
        "str_replace" => Ok((
            "Edit",
            json!({"file_path": path, "old_string": input["old_str"], "new_string": input["new_str"]}),
        )),
        other => Err(format!(
            "text editor command `{other}` is not available; use Read, Write or Edit"
        )),
    }
}

/// Route a call to a Kor tool and shape its arguments for that tool's schema.
///
/// Returns the Kor tool name and the arguments, or an error message for the model.
pub fn canonical(name: &str, input: Value, specs: &[ToolSpec]) -> Result<(String, Value), String> {
    let exact = specs.iter().any(|s| s.name == name);
    let lower = name.to_ascii_lowercase();
    let (target, mut input) = if exact {
        (name.to_owned(), input)
    } else if matches!(
        lower.as_str(),
        "str_replace_editor" | "str_replace_based_edit_tool" | "text_editor"
    ) {
        let (t, v) = text_editor(&input)?;
        (t.to_owned(), v)
    } else if let Some(s) = specs.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
        (s.name.clone(), input)
    } else if let Some((_, t)) = ALIASES.iter().find(|(a, _)| *a == lower) {
        ((*t).to_owned(), input)
    } else {
        return Err(format!("unknown tool `{name}`"));
    };
    let Some(spec) = specs.iter().find(|s| s.name == target) else {
        return Err(format!(
            "tool `{target}` (for `{name}`) is not available in this session"
        ));
    };
    // Some models send arguments as a JSON string.
    if let Value::String(s) = &input
        && let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(s)
    {
        input = v;
    }
    if input.is_null() {
        input = json!({});
    }
    let Some(obj) = input.as_object_mut() else {
        return Ok((target, input));
    };
    let Some(props) = schema_props(spec) else {
        return Ok((target, input));
    };
    for (param, synonyms) in SYNONYMS {
        if !props.contains_key(*param) || obj.contains_key(*param) {
            continue;
        }
        if let Some(found) = synonyms
            .iter()
            .find(|s| obj.contains_key(**s) && !props.contains_key(**s))
        {
            let v = obj.remove(*found).unwrap_or(Value::Null);
            obj.insert((*param).to_owned(), v);
        }
    }
    if let Some(Value::Array(a)) = obj.get("command") {
        let joined = command_from_array(a);
        obj.insert("command".into(), joined.into());
    }
    // Wrong-case field names and `"3"`-for-3 values, fitted to the schema.
    crate::schema::coerce(&mut input, &spec.input_schema);
    if let Some(obj) = input.as_object_mut() {
        // Tools reject unknown keys; drop extras rather than fail the whole call.
        obj.retain(|k, _| props.contains_key(k));
    }
    Ok((target, input))
}

/// [`canonical`], then validation against the tool's schema (spec 15.13): input that does not
/// validate never reaches the tool. Returns the tool name, the arguments and whether the
/// arguments had to be changed to fit.
///
/// # Errors
/// Unknown tool or schema violation, worded for the model.
pub fn prepare(
    name: &str,
    input: Value,
    specs: &[ToolSpec],
) -> Result<(String, Value, bool), String> {
    let before = input.clone();
    let (target, value) = canonical(name, input, specs)?;
    if let Some(spec) = specs.iter().find(|s| s.name == target) {
        crate::schema::validate(&value, &spec.input_schema)
            .map_err(|e| format!("invalid input for `{target}`: {e}. Fix the call and retry."))?;
    }
    let changed = value != before;
    Ok((target, value, changed))
}

/// System-prompt section that teaches a model the text tool protocol.
pub fn text_protocol_prompt(specs: &[ToolSpec]) -> String {
    let mut p = String::from(
        "\n\n# Tools\n\
         You can use tools. To call a tool, write a block exactly like this (one JSON object per block):\n\
         <tool_call>\n{\"name\": \"TOOL_NAME\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n\
         You may write several blocks in one reply. Then stop and wait: each result arrives in a \
         <tool_result> block in the next message. Never write <tool_result> yourself and never \
         guess results. When no tool is needed, answer normally.\n\nAvailable tools:\n",
    );
    for s in specs {
        let params = s
            .input_schema
            .get("properties")
            .cloned()
            .unwrap_or(json!({}));
        let required = s.input_schema.get("required").cloned().unwrap_or(json!([]));
        let _ = write!(
            p,
            "\n## {}\n{}\nparameters: {}\nrequired: {}\n",
            s.name,
            s.description.trim(),
            params,
            required
        );
    }
    p
}

// Anything up to the closing tag (or the end of the reply), so broken JSON reaches repair.
static TOOL_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<tool_call>\s*(.*?)\s*(?:</tool_call>|$)").expect("static"));
static FUNCTION_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<function=([\w.\-]+)>\s*(\{.*?\})\s*</function>").expect("static")
});
static MISTRAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\[TOOL_CALLS\]\s*(\[.*\])").expect("static"));
static FENCED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)```(?:json|tool_call|tool)?\s*\n(\{.*?\})\s*\n```").expect("static")
});

/// `{"name", "arguments"|"parameters"|"input"|"args"}` or `{"tool", "args"}`.
fn call_from_object(v: &Value) -> Option<(String, Value)> {
    let name = v
        .get("name")
        .or_else(|| v.get("tool"))
        .or_else(|| v.get("function"))?;
    let name = match name {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("name")?.as_str()?.to_owned(),
        _ => return None,
    };
    let args = ["arguments", "parameters", "input", "args"]
        .iter()
        .find_map(|k| v.get(*k).cloned())
        .or_else(|| v.get("function").and_then(|f| f.get("arguments")).cloned())
        .unwrap_or(json!({}));
    let args = match args {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        a => a,
    };
    Some((name, args))
}

/// A text tool call: `(name, arguments)`. A block that is not valid JSON comes back as
/// `Err(raw text)` (name `""` when unknown) for the repair layer instead of being dropped.
pub type TextCall = (String, Result<Value, String>);

/// A broken whole-call block (`{"name": .., "arguments": ..}` text) after local repair.
pub fn call_from_text(raw: &str) -> Option<(String, Value)> {
    crate::repair::lenient(raw)
        .as_ref()
        .and_then(call_from_object)
}

/// Tool calls written as text. Fenced JSON only counts when it names a known tool, so
/// ordinary JSON examples in an answer are not executed.
pub fn parse_text_calls(text: &str, specs: &[ToolSpec]) -> Vec<TextCall> {
    let mut calls: Vec<TextCall> = Vec::new();
    for c in TOOL_CALL.captures_iter(text) {
        let raw = c[1].trim();
        if raw.is_empty() {
            continue;
        }
        match serde_json::from_str(raw)
            .ok()
            .as_ref()
            .and_then(call_from_object)
        {
            Some((name, args)) => calls.push((name, Ok(args))),
            None => calls.push((String::new(), Err(raw.to_owned()))),
        }
    }
    for c in FUNCTION_TAG.captures_iter(text) {
        let args = serde_json::from_str::<Value>(&c[2]).map_err(|_| c[2].to_owned());
        calls.push((c[1].to_owned(), args));
    }
    if let Some(c) = MISTRAL.captures(text) {
        match serde_json::from_str::<Value>(&c[1]) {
            Ok(Value::Array(items)) => calls.extend(
                items
                    .iter()
                    .filter_map(call_from_object)
                    .map(|(n, a)| (n, Ok(a))),
            ),
            _ => calls.push((String::new(), Err(c[1].to_owned()))),
        }
    }
    if calls.is_empty() {
        for c in FENCED.captures_iter(text) {
            let Some((name, args)) = serde_json::from_str(&c[1])
                .ok()
                .as_ref()
                .and_then(call_from_object)
            else {
                continue;
            };
            if canonical(&name, args.clone(), specs).is_ok() {
                calls.push((name, Ok(args)));
            }
        }
    }
    calls
}

/// The message that carries text-protocol results back to the model.
pub fn results_message(results: &[(String, String, bool)]) -> String {
    let mut s = String::new();
    for (name, content, is_error) in results {
        let status = if *is_error { " status=\"error\"" } else { "" };
        let _ = write!(
            s,
            "<tool_result name=\"{name}\"{status}>\n{content}\n</tool_result>\n"
        );
    }
    s
}

/// Provider errors that mean "this model/endpoint does not accept native tools".
pub fn rejects_native_tools(e: &kor_core::ProviderError) -> bool {
    let m = e.message.to_ascii_lowercase();
    e.kind == kor_core::ProviderErrorKind::InvalidRequest
        && (m.contains("tool") || m.contains("function"))
        && (m.contains("support")
            || m.contains("not allowed")
            || m.contains("unknown")
            || m.contains("invalid")
            || m.contains("unrecognized"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ToolSpec> {
        let spec = |name: &str, props: Value| ToolSpec {
            name: name.into(),
            description: format!("{name} tool"),
            input_schema: json!({"type": "object", "properties": props, "additionalProperties": false}),
        };
        vec![
            spec("Read", json!({"file_path": {}, "offset": {}, "limit": {}})),
            spec("Write", json!({"file_path": {}, "content": {}})),
            spec(
                "Edit",
                json!({"file_path": {}, "old_string": {}, "new_string": {}, "replace_all": {}}),
            ),
            spec("Bash", json!({"command": {}, "timeout": {}})),
            spec("Grep", json!({"pattern": {}, "path": {}})),
            spec("list", json!({"path": {}})),
            spec("apply_patch", json!({"patch": {}})),
        ]
    }

    #[test]
    fn foreign_names_and_arguments_map_to_kor_tools() {
        let s = specs();
        let c = |n: &str, v: Value| canonical(n, v, &s).unwrap();
        assert_eq!(
            c("read_file", json!({"target_file": "a.rs"})),
            ("Read".into(), json!({"file_path": "a.rs"}))
        );
        assert_eq!(
            c(
                "run_terminal_cmd",
                json!({"cmd": "ls", "is_background": false})
            ),
            ("Bash".into(), json!({"command": "ls"}))
        );
        assert_eq!(
            c("shell", json!({"command": ["bash", "-lc", "cargo test"]})),
            ("Bash".into(), json!({"command": "cargo test"}))
        );
        assert_eq!(
            c("shell", json!({"command": ["git", "status"]})),
            ("Bash".into(), json!({"command": "git status"}))
        );
        assert_eq!(
            c(
                "replace_in_file",
                json!({"path": "a", "old_str": "x", "new_str": "y"})
            ),
            (
                "Edit".into(),
                json!({"file_path": "a", "old_string": "x", "new_string": "y"})
            )
        );
        assert_eq!(
            c("write_to_file", json!({"path": "a", "file_text": "z"})),
            ("Write".into(), json!({"file_path": "a", "content": "z"}))
        );
        assert_eq!(
            c("search_files", json!({"regex": "fn", "path": "src"})),
            ("Grep".into(), json!({"pattern": "fn", "path": "src"}))
        );
        assert_eq!(
            c("list_dir", json!({"relative_workspace_path": "src"})),
            ("list".into(), json!({"path": "src"}))
        );
        assert_eq!(c("BASH", json!({"command": "ls"})).0, "Bash");
        assert_eq!(
            c("Read", json!("{\"file_path\":\"x\"}")),
            ("Read".into(), json!({"file_path": "x"}))
        );
        assert!(canonical("launch_rockets", json!({}), &s).is_err());
    }

    #[test]
    fn anthropic_text_editor_commands() {
        let s = specs();
        assert_eq!(
            canonical(
                "str_replace_editor",
                json!({"command": "view", "path": "a", "view_range": [10, 20]}),
                &s
            )
            .unwrap(),
            (
                "Read".into(),
                json!({"file_path": "a", "offset": 10, "limit": 11})
            )
        );
        assert_eq!(
            canonical(
                "str_replace_based_edit_tool",
                json!({"command": "str_replace", "path": "a", "old_str": "x", "new_str": "y"}),
                &s
            )
            .unwrap()
            .0,
            "Edit"
        );
        assert!(
            canonical(
                "str_replace_editor",
                json!({"command": "undo_edit", "path": "a"}),
                &s
            )
            .is_err()
        );
    }

    #[test]
    fn parses_common_text_formats() {
        let s = specs();
        let hermes = "Let me look.\n<tool_call>\n{\"name\": \"Read\", \"arguments\": {\"file_path\": \"a.rs\"}}\n</tool_call>";
        assert_eq!(
            parse_text_calls(hermes, &s),
            [("Read".into(), Ok(json!({"file_path": "a.rs"})))]
        );

        let two = "<tool_call>{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}</tool_call><tool_call>{\"name\":\"Read\",\"parameters\":{\"path\":\"b\"}}</tool_call>";
        let calls = parse_text_calls(two, &s);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("Bash".into(), Ok(json!({"command": "ls"}))));

        let llama = "<function=list>{\"path\": \".\"}</function>";
        assert_eq!(
            parse_text_calls(llama, &s),
            [("list".into(), Ok(json!({"path": "."})))]
        );

        let mistral = "[TOOL_CALLS] [{\"name\": \"Grep\", \"arguments\": {\"pattern\": \"x\"}}]";
        assert_eq!(parse_text_calls(mistral, &s)[0].0, "Grep");

        let fenced = "```json\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"x\"}}\n```";
        assert_eq!(parse_text_calls(fenced, &s).len(), 1);
        let example = "```json\n{\"name\": \"Alice\", \"arguments\": {}}\n```";
        assert!(
            parse_text_calls(example, &s).is_empty(),
            "plain JSON is not a call"
        );
        let unterminated = "<tool_call>{\"name\":\"Read\",\"arguments\":{\"file_path\":\"z\"}}";
        assert_eq!(parse_text_calls(unterminated, &s).len(), 1);

        // Broken JSON is handed to the repair layer, not dropped.
        let broken = "<tool_call>{'name': 'Read', 'arguments': {'file_path': 'z'</tool_call>";
        let calls = parse_text_calls(broken, &s);
        let [(name, Err(raw))] = calls.as_slice() else {
            panic!("{calls:?}")
        };
        assert!(name.is_empty());
        assert_eq!(
            call_from_text(raw),
            Some(("Read".into(), json!({"file_path": "z"})))
        );
    }

    #[test]
    fn prepare_fits_types_and_rejects_schema_violations() {
        let typed = vec![ToolSpec {
            name: "Read".into(),
            description: String::new(),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string"}, "limit": {"type": "integer", "minimum": 1}
            }, "required": ["file_path"], "additionalProperties": false}),
        }];
        assert_eq!(
            prepare("Read", json!({"FILE_PATH": "a", "limit": "3"}), &typed).unwrap(),
            ("Read".into(), json!({"file_path": "a", "limit": 3}), true)
        );
        assert_eq!(
            prepare("Read", json!({"file_path": "a"}), &typed).unwrap(),
            ("Read".into(), json!({"file_path": "a"}), false)
        );
        let e = prepare("Read", json!({"limit": 0}), &typed).unwrap_err();
        assert!(e.contains("missing required field `file_path`"), "{e}");
        let e = prepare("Read", json!({"file_path": "a", "limit": 0}), &typed).unwrap_err();
        assert!(e.contains("`limit` must be at least 1"), "{e}");
    }

    #[test]
    fn prompt_lists_tools_and_results_are_tagged() {
        let p = text_protocol_prompt(&specs());
        assert!(p.contains("## Read") && p.contains("<tool_call>"));
        let r = results_message(&[
            ("Read".into(), "hi".into(), false),
            ("Bash".into(), "boom".into(), true),
        ]);
        assert!(r.contains("<tool_result name=\"Read\">\nhi"));
        assert!(r.contains("name=\"Bash\" status=\"error\""));
    }

    #[test]
    fn detects_tool_rejection() {
        let e = |m: &str| {
            kor_core::ProviderError::new("p", kor_core::ProviderErrorKind::InvalidRequest, m)
        };
        assert!(rejects_native_tools(&e(
            "This model does not support tools"
        )));
        assert!(rejects_native_tools(&e(
            "tools is not supported for this model"
        )));
        assert!(!rejects_native_tools(&e("max_tokens too large")));
    }
}
