//! `kor mcp serve`: Kor's project graph, memory, cost and tasks for Claude Code, Cursor and other MCP
//! clients (spec §8.3). Every exposed tool only reads. Which groups are exposed comes from
//! `"serve": {"groups": [...]}` in Kor's `mcp.json` (default: all of [`GROUPS`]) or
//! `kor mcp serve --groups`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use kor_tools::{Tool, ToolCtx};
use serde_json::{Value, json};

use super::serve::Backend;

/// Tool groups that can be exposed.
pub const GROUPS: [&str; 4] = ["graph", "memory", "cost", "tasks"];

/// Cost report for the last `days` days (`kor_runtime::cost::report`, passed in by the caller).
pub type CostFn = Arc<dyn Fn(u32) -> Result<Value, String> + Send + Sync>;

/// `kor task` checkpoints (`kor_runtime::task`): `None` lists them, `Some(id)` is one task.
pub type TasksFn = Arc<dyn Fn(Option<&str>) -> Result<Value, String> + Send + Sync>;

/// Groups from Kor's `mcp.json` (`serve.groups`), else all; unknown names are an error.
pub fn configured_groups(kor_file: &Path, flag: Option<&str>) -> Result<Vec<String>, String> {
    let from_file = crate::mcp_config::read_object(kor_file)?
        .get("serve")
        .and_then(|s| s.get("groups"))
        .map(|g| crate::mcp_config::strings(Some(g)));
    let groups: Vec<String> = match flag {
        Some(f) => f
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        None => from_file.unwrap_or_else(|| GROUPS.iter().map(|s| (*s).to_owned()).collect()),
    };
    if let Some(bad) = groups.iter().find(|g| !GROUPS.contains(&g.as_str())) {
        return Err(format!(
            "unknown tool group `{bad}` (known: {})",
            GROUPS.join(", ")
        ));
    }
    Ok(groups)
}

pub struct KorServe {
    root: PathBuf,
    home: Option<PathBuf>,
    groups: Vec<String>,
    graph: Vec<Arc<dyn Tool>>,
    ctx: ToolCtx,
    cost: Option<CostFn>,
    tasks: Option<TasksFn>,
}

impl std::fmt::Debug for KorServe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KorServe")
            .field("root", &self.root)
            .field("groups", &self.groups)
            .finish_non_exhaustive()
    }
}

impl KorServe {
    /// # Errors
    /// If `root` cannot be opened.
    pub fn new(
        root: &Path,
        home: Option<PathBuf>,
        groups: Vec<String>,
        cost: Option<CostFn>,
        tasks: Option<TasksFn>,
    ) -> Result<Self, String> {
        let ctx = ToolCtx::new(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let graph = if groups.iter().any(|g| g == "graph") {
            let session = crate::graph::Session::new(
                &ctx.root,
                crate::graph::Options {
                    home: home.clone(),
                    ..crate::graph::Options::default()
                },
            );
            session.warm();
            crate::graph::graph_tools(&session)
        } else {
            Vec::new()
        };
        Ok(Self {
            root: ctx.root.clone(),
            home,
            groups,
            graph,
            ctx,
            cost,
            tasks,
        })
    }

    fn has(&self, g: &str) -> bool {
        self.groups.iter().any(|x| x == g)
    }

    fn home(&self) -> Result<&Path, String> {
        self.home
            .as_deref()
            .ok_or_else(|| "no Kor home (KOR_HOME)".into())
    }

    fn recall(&self, args: &Value) -> Result<String, String> {
        let query = args["query"].as_str().unwrap_or_default();
        let limit = args["limit"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(10);
        let kb = kor_memory::open_knowledge(self.home()?).map_err(|e| e.to_string())?;
        let here = kor_memory::project_key(&self.root);
        let only = (args["allProjects"] != true).then_some(here.as_str());
        let hits = kb
            .recall(query, only, Some(&here), limit.clamp(1, 50))
            .map_err(|e| e.to_string())?;
        if hits.is_empty() {
            return Ok("No matching memories.".into());
        }
        let mut out = String::new();
        for h in hits {
            let r = h.record;
            let _ = writeln!(
                out,
                "#{} {} {}\n{}",
                r.id,
                kor_memory::kb::date(r.ts),
                r.task,
                r.summary
            );
            for d in r.decisions.iter().take(5) {
                let _ = writeln!(out, "- decision: {d}");
            }
            for e in r.errors.iter().take(3) {
                let _ = writeln!(out, "- solved: {e}");
            }
            out.push('\n');
        }
        Ok(out)
    }

    fn decisions(&self) -> Result<String, String> {
        let all = kor_memory::decisions(self.home()?, &self.root);
        if all.is_empty() {
            return Ok("No recorded decisions for this project.".into());
        }
        let mut out = String::new();
        for d in all {
            let _ = writeln!(out, "## {} {}\n{}", d.title, d.date, d.body.trim());
            if !d.refs.is_empty() {
                let _ = writeln!(out, "refs: {}", d.refs.join(", "));
            }
            out.push('\n');
        }
        Ok(out)
    }
}

fn spec(name: &str, description: &str, schema: &Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": schema,
        "annotations": {"readOnlyHint": true, "destructiveHint": false}})
}

#[async_trait]
impl Backend for KorServe {
    fn name(&self) -> String {
        "kor".into()
    }

    fn tools(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .graph
            .iter()
            .map(|t| {
                let s = t.spec();
                spec(&s.name, &s.description, &s.input_schema)
            })
            .collect();
        if self.has("memory") {
            out.push(spec(
                "memory_recall",
                "Search Kor's memory of past tasks in this project (what was done, decisions, errors solved).",
                &json!({"type": "object", "properties": {
                    "query": {"type": "string", "maxLength": 500},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50},
                    "allProjects": {"type": "boolean", "description": "Search every project, not only this one"}
                }, "required": ["query"]}),
            ));
            out.push(spec(
                "memory_decisions",
                "Architecture decisions recorded for this project, with the files they concern.",
                &json!({"type": "object", "properties": {}}),
            ));
        }
        if self.has("cost") && self.cost.is_some() {
            out.push(spec(
                "cost_summary",
                "Kor's model usage and cost per model for the last days, today, this month and the month's projection (≈ marks estimates).",
                &json!({"type": "object", "properties": {"days": {"type": "integer", "minimum": 1, "maximum": 365}}}),
            ));
        }
        if self.has("tasks") && self.tasks.is_some() {
            out.push(spec(
                "task_list",
                "Multi-agent tasks of this repository (`kor task`): id, goal, status, newest first.",
                &json!({"type": "object", "properties": {}}),
            ));
            out.push(spec(
                "task_status",
                "One `kor task`: goal, status, sub-tasks, test runs and merge conflicts.",
                &json!({"type": "object", "properties": {"id": {"type": "string", "maxLength": 40}}, "required": ["id"]}),
            ));
        }
        out
    }

    async fn call(&self, name: &str, args: Value) -> Result<String, String> {
        if let Some(t) = self.graph.iter().find(|t| t.name() == name) {
            let out = t.call(args, &self.ctx).await;
            return if out.is_error {
                Err(out.content)
            } else {
                Ok(out.content)
            };
        }
        let schema = self
            .tools()
            .into_iter()
            .find(|t| t["name"] == name)
            .ok_or("unknown tool")?;
        super::validate(&schema["inputSchema"], &args)?;
        match name {
            "memory_recall" => self.recall(&args),
            "memory_decisions" => self.decisions(),
            "cost_summary" => {
                let days = args["days"]
                    .as_u64()
                    .and_then(|d| u32::try_from(d).ok())
                    .unwrap_or(30);
                let f = self.cost.as_ref().ok_or("cost is not available")?;
                f(days).and_then(|v| serde_json::to_string_pretty(&v).map_err(|e| e.to_string()))
            }
            "task_list" | "task_status" => {
                let f = self.tasks.as_ref().ok_or("tasks are not available")?;
                f(args["id"].as_str())
                    .and_then(|v| serde_json::to_string_pretty(&v).map_err(|e| e.to_string()))
            }
            _ => Err("unknown tool".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    /// A whole JSON-RPC session over stdio: initialize, tools/list, tools/call (graph, memory,
    /// cost), an unknown tool and method, and a notification that gets no answer.
    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::many_single_char_names, clippy::too_many_lines)]
    async fn kor_mcp_serve_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let home = dir.path().join("home");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn parse_invoice(x: u32) -> u32 { x }\npub fn total() -> u32 { parse_invoice(1) }\n").unwrap();
        let mem = kor_memory::ProjectMemory::new(&home, &ToolCtx::new(&root).unwrap().root);
        mem.ensure().unwrap();
        std::fs::write(
            mem.dir.join("decisions.md"),
            "# Decisions\n## 2026-11-01 — Use SQLite\nLocal first.\n",
        )
        .unwrap();

        let kor_file = dir.path().join("cfg").join("mcp.json");
        assert_eq!(configured_groups(&kor_file, None).unwrap(), GROUPS);
        let tasks: TasksFn = Arc::new(|id| match id {
            None => Ok(json!([{"id": "1", "goal": "split parser"}])),
            Some(i) => Err(format!("task #{i} not found")),
        });
        crate::mcp_config::write_atomic(
            &kor_file,
            &json!({"serve": {"groups": ["graph", "memory"]}}),
        )
        .unwrap();
        assert_eq!(
            configured_groups(&kor_file, None).unwrap(),
            ["graph", "memory"]
        );
        assert!(configured_groups(&kor_file, Some("graph,shell")).is_err());

        let cost: CostFn = Arc::new(|days| Ok(json!({"days": days, "total": {"usd": 0.5}})));
        let groups = configured_groups(&kor_file, Some("graph,memory,cost,tasks")).unwrap();
        let server = KorServe::new(&root, Some(home), groups, Some(cost), Some(tasks)).unwrap();

        let (a, b) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            let (r, w) = tokio::io::split(b);
            super::super::serve::serve(&server, r, w).await
        });
        let (r, mut w) = tokio::io::split(a);
        let mut lines = BufReader::new(r).lines();
        let mut ask = async |raw: String| -> Value {
            w.write_all(format!("{raw}\n").as_bytes()).await.unwrap();
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
        };
        let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}).to_string()).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "kor");
        // A notification gets no reply, so the next line read answers id 2.
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let list = ask(format!("{note}\n{list}")).await;
        assert_eq!(list["id"], 2);
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "graph_query",
                "graph_explain",
                "graph_path",
                "graph_impact",
                "memory_recall",
                "memory_decisions",
                "cost_summary",
                "task_list",
                "task_status"
            ]
        );
        assert!(
            list["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["annotations"]["readOnlyHint"] == true)
        );

        let call = |id: u64, name: &str, args: Value| {
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string()
        };
        let q = ask(call(3, "graph_query", json!({"query": "parse_invoice"}))).await;
        let text = q["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("parse_invoice"), "{q}");
        let d = ask(call(4, "memory_decisions", json!({}))).await;
        assert!(
            d["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Use SQLite"),
            "{d}"
        );
        let c = ask(call(5, "cost_summary", json!({"days": 7}))).await;
        assert!(
            c["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"days\": 7")
        );
        let bad = ask(call(6, "cost_summary", json!({"days": "x"}))).await;
        assert_eq!(bad["result"]["isError"], true);
        let t = ask(call(9, "task_list", json!({}))).await;
        assert!(
            t["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("split parser")
        );
        let t = ask(call(10, "task_status", json!({"id": "7"}))).await;
        assert_eq!(t["result"]["isError"], true);
        let unknown = ask(call(7, "Bash", json!({"command": "rm -rf /"}))).await;
        assert_eq!(unknown["error"]["code"], -32602);
        let m =
            ask(json!({"jsonrpc": "2.0", "id": 8, "method": "resources/list"}).to_string()).await;
        assert_eq!(m["error"]["code"], -32601);
    }
}
