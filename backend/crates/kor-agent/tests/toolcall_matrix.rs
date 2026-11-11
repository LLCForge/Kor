//! §26.2 acceptance with mocks (ADR 0056): the same 40 tool-call scenarios reach the same Kor
//! tool with the same arguments whichever way a model writes the call — native function calling
//! with its own tool names (OpenAI/Anthropic/Gemini/Mistral/Qwen/Llama style) and the four text
//! emulations local models use (Hermes/Qwen `<tool_call>`, Llama `<function=…>`, Mistral
//! `[TOOL_CALLS]`, fenced JSON). Typed user tools, `tools_search` and `cli_tool` are part of the
//! set. The success rate is reported per call style; the live run against real provider accounts
//! is `kor bench` with the user's keys.

use std::collections::BTreeMap;

use kor_agent::clitools::defs::{DefinedTool, Loaded, Origin, ToolDef};
use kor_agent::toolcall::{canonical, parse_text_calls};
use kor_core::ToolSpec;
use kor_tools::Tool as _;
use serde_json::{Value, json};

/// (name the model uses, its arguments, Kor tool, one argument Kor must receive with its value).
#[allow(clippy::too_many_lines)] // a data table
fn scenarios() -> Vec<(&'static str, Value, &'static str, (&'static str, Value))> {
    vec![
        (
            "read_file",
            json!({"path": "src/main.rs"}),
            "Read",
            ("file_path", json!("src/main.rs")),
        ),
        (
            "Read",
            json!({"file_path": "a.txt"}),
            "Read",
            ("file_path", json!("a.txt")),
        ),
        (
            "view_file",
            json!({"file_path": "b.rs"}),
            "Read",
            ("file_path", json!("b.rs")),
        ),
        (
            "open_file",
            json!({"path": "c.md"}),
            "Read",
            ("file_path", json!("c.md")),
        ),
        (
            "cat",
            json!({"path": "d.toml"}),
            "Read",
            ("file_path", json!("d.toml")),
        ),
        (
            "write_file",
            json!({"path": "x.txt", "content": "hi"}),
            "Write",
            ("content", json!("hi")),
        ),
        (
            "create_file",
            json!({"file_path": "y.txt", "content": "yo"}),
            "Write",
            ("file_path", json!("y.txt")),
        ),
        (
            "write_to_file",
            json!({"path": "z.txt", "content": "z"}),
            "Write",
            ("content", json!("z")),
        ),
        (
            "save_file",
            json!({"path": "w.txt", "content": "w"}),
            "Write",
            ("file_path", json!("w.txt")),
        ),
        (
            "edit_file",
            json!({"path": "a.rs", "old_string": "a", "new_string": "b"}),
            "Edit",
            ("new_string", json!("b")),
        ),
        (
            "str_replace",
            json!({"path": "a.rs", "old_str": "x", "new_str": "y"}),
            "Edit",
            ("old_string", json!("x")),
        ),
        (
            "replace_in_file",
            json!({"file_path": "a.rs", "old_string": "1", "new_string": "2"}),
            "Edit",
            ("file_path", json!("a.rs")),
        ),
        (
            "search_replace",
            json!({"file_path": "q.rs", "old_string": "p", "new_string": "q"}),
            "Edit",
            ("old_string", json!("p")),
        ),
        (
            "delete_file",
            json!({"path": "old.txt"}),
            "Delete",
            ("path", json!("old.txt")),
        ),
        (
            "move_file",
            json!({"source": "a", "destination": "b"}),
            "Move",
            ("destination", json!("b")),
        ),
        (
            "bash",
            json!({"command": "git status"}),
            "Bash",
            ("command", json!("git status")),
        ),
        (
            "shell",
            json!({"command": "ls"}),
            "Bash",
            ("command", json!("ls")),
        ),
        (
            "run_terminal_cmd",
            json!({"command": "cargo test"}),
            "Bash",
            ("command", json!("cargo test")),
        ),
        (
            "execute_command",
            json!({"command": "npm test"}),
            "Bash",
            ("command", json!("npm test")),
        ),
        (
            "run_shell_command",
            json!({"command": "docker ps"}),
            "Bash",
            ("command", json!("docker ps")),
        ),
        (
            "exec",
            json!({"cmd": "kubectl get pods"}),
            "Bash",
            ("command", json!("kubectl get pods")),
        ),
        (
            "Bash",
            json!({"command": "ffmpeg -version"}),
            "Bash",
            ("command", json!("ffmpeg -version")),
        ),
        (
            "glob",
            json!({"pattern": "**/*.rs"}),
            "Glob",
            ("pattern", json!("**/*.rs")),
        ),
        (
            "find_files",
            json!({"pattern": "*.ts"}),
            "Glob",
            ("pattern", json!("*.ts")),
        ),
        (
            "grep",
            json!({"pattern": "TODO"}),
            "Grep",
            ("pattern", json!("TODO")),
        ),
        (
            "search_files",
            json!({"pattern": "fn main"}),
            "Grep",
            ("pattern", json!("fn main")),
        ),
        (
            "ripgrep",
            json!({"pattern": "unsafe"}),
            "Grep",
            ("pattern", json!("unsafe")),
        ),
        (
            "codebase_search",
            json!({"query": "auth"}),
            "Grep",
            ("pattern", json!("auth")),
        ),
        (
            "web_fetch",
            json!({"url": "https://example.com"}),
            "WebFetch",
            ("url", json!("https://example.com")),
        ),
        (
            "fetch_url",
            json!({"url": "https://example.org"}),
            "WebFetch",
            ("url", json!("https://example.org")),
        ),
        (
            "web_search",
            json!({"query": "rust 1.90"}),
            "WebSearch",
            ("query", json!("rust 1.90")),
        ),
        (
            "google_search",
            json!({"query": "tokio"}),
            "WebSearch",
            ("query", json!("tokio")),
        ),
        (
            "todo_write",
            json!({"todos": []}),
            "TodoWrite",
            ("todos", json!([])),
        ),
        (
            "apply_patch",
            json!({"patch": "*** Begin Patch\n*** End Patch"}),
            "apply_patch",
            ("patch", json!("*** Begin Patch\n*** End Patch")),
        ),
        (
            "tools_search",
            json!({"query": "convert video"}),
            "tools_search",
            ("query", json!("convert video")),
        ),
        (
            "tools_search",
            json!({"describe": "ffmpeg"}),
            "tools_search",
            ("describe", json!("ffmpeg")),
        ),
        (
            "cli_tool",
            json!({"tool": "thumbnail", "args": {"video": "a.mp4", "out": "a.png"}}),
            "cli_tool",
            ("tool", json!("thumbnail")),
        ),
        (
            "thumbnail",
            json!({"video": "a.mp4", "out": "a.png"}),
            "thumbnail",
            ("video", json!("a.mp4")),
        ),
        ("git_log", json!({"n": 5}), "git_log", ("n", json!(5))),
        (
            "tool_define",
            json!({"name": "rg_count", "command": ["rg", "-c", "{p}"]}),
            "tool_define",
            ("name", json!("rg_count")),
        ),
    ]
}

fn specs() -> Vec<ToolSpec> {
    let mut v: Vec<ToolSpec> = kor_tools::default_tools()
        .iter()
        .map(|t| t.spec())
        .collect();
    for (name, schema) in [
        (
            "tools_search",
            json!({"type":"object","properties":{"query":{"type":"string"},"describe":{"type":"string"}},"additionalProperties":false}),
        ),
        (
            "cli_tool",
            json!({"type":"object","properties":{"tool":{"type":"string"},"args":{"type":"object"}},"required":["tool"],"additionalProperties":false}),
        ),
        (
            "tool_define",
            json!({"type":"object","properties":{"name":{"type":"string"},"command":{"type":"array","items":{"type":"string"}}},"required":["name","command"],"additionalProperties":false}),
        ),
    ] {
        v.push(ToolSpec {
            name: name.into(),
            description: String::new(),
            input_schema: schema,
        });
    }
    for text in [
        "name = \"thumbnail\"\ncommand = [\"ffmpeg\", \"-i\", \"{video}\", \"{out}\"]\n[args.video]\ntype = \"path\"\nrequired = true\n[args.out]\ntype = \"path\"\nrequired = true\n",
        "name = \"git_log\"\ncommand = \"git log -n {n}\"\nsafety = \"read\"\n[args.n]\ntype = \"integer\"\n",
    ] {
        let def: ToolDef = toml::from_str(text).unwrap();
        let t = DefinedTool::new(Loaded {
            def,
            origin: Origin::User,
            file: "x.toml".into(),
        });
        v.push(t.spec());
    }
    v
}

/// The text a model writes for one call in each emulation style.
fn styles(name: &str, args: &Value) -> Vec<(&'static str, String)> {
    let obj = json!({"name": name, "arguments": args});
    vec![
        (
            "hermes",
            format!("I'll do that.\n<tool_call>\n{obj}\n</tool_call>"),
        ),
        ("llama", format!("<function={name}>{args}</function>")),
        ("mistral", format!("[TOOL_CALLS][{obj}]")),
        ("fenced", format!("Calling the tool:\n```json\n{obj}\n```")),
    ]
}

fn check(
    name: &str,
    args: Value,
    specs: &[ToolSpec],
    want: &str,
    key: &str,
    value: &Value,
) -> bool {
    match canonical(name, args, specs) {
        Ok((tool, input)) => tool == want && input.get(key) == Some(value),
        Err(_) => false,
    }
}

#[test]
fn forty_scenarios_route_the_same_in_every_call_style() {
    let specs = specs();
    let all = scenarios();
    assert_eq!(all.len(), 40);
    let mut rate: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut fails = Vec::new();
    for (name, args, want, (key, value)) in &all {
        let native = check(name, args.clone(), &specs, want, key, value);
        let e = rate.entry("native").or_default();
        e.1 += 1;
        if native {
            e.0 += 1;
        } else {
            fails.push(format!("native {name}"));
        }
        for (style, text) in styles(name, args) {
            let calls = parse_text_calls(&text, &specs);
            let ok = calls.len() == 1
                && calls[0]
                    .1
                    .as_ref()
                    .is_ok_and(|a| check(&calls[0].0, a.clone(), &specs, want, key, value));
            let e = rate.entry(style).or_default();
            e.1 += 1;
            if ok {
                e.0 += 1;
            } else {
                fails.push(format!("{style} {name}: {calls:?}"));
            }
        }
    }
    for (style, (ok, n)) in &rate {
        eprintln!("tool calls via {style}: {ok}/{n}");
    }
    assert!(fails.is_empty(), "{fails:#?}");
}
