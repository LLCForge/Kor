//! `NotebookEdit`.

use std::path::Path;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::jail::rel;
use crate::{Tool, ToolCtx, ToolKind, ToolOutput, parse};

#[derive(Debug)]
pub(crate) struct NotebookEdit;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct In {
    notebook_path: String,
    cell_id: Option<String>,
    new_source: String,
    cell_type: Option<String>,
    edit_mode: Option<String>,
}

fn as_source_array(s: &str) -> Value {
    if s.is_empty() {
        return json!([]);
    }
    let mut parts = Vec::new();
    for line in s.split_inclusive('\n') {
        parts.push(Value::String(line.to_owned()));
    }
    Value::Array(parts)
}

fn cell_id(cell: &Value, index: usize) -> String {
    cell["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map_or_else(|| format!("cell-{index}"), str::to_owned)
}

fn find_cell(cells: &[Value], id: &str) -> Option<usize> {
    cells
        .iter()
        .enumerate()
        .find_map(|(i, c)| (cell_id(c, i) == id || c["id"].as_str() == Some(id)).then_some(i))
}

async fn load(path: &Path) -> Result<Value, String> {
    let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid notebook: {e}"))
}

#[async_trait]
impl Tool for NotebookEdit {
    fn name(&self) -> &'static str {
        "NotebookEdit"
    }
    fn rule_name(&self) -> &'static str {
        "NotebookEdit"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Edit one cell of a Jupyter notebook. edit_mode is replace (default), insert or delete. cell_id selects the cell; insert places the new cell after it, or at the start when cell_id is omitted. insert requires cell_type (code or markdown).".into(),
            input_schema: json!({"type": "object", "properties": {
                "notebook_path": {"type": "string"},
                "cell_id": {"type": "string"},
                "new_source": {"type": "string"},
                "cell_type": {"type": "string", "enum": ["code", "markdown"]},
                "edit_mode": {"type": "string", "enum": ["replace", "insert", "delete"]}
            }, "required": ["notebook_path", "new_source"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["notebook_path"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: In = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let mode = a.edit_mode.as_deref().unwrap_or("replace");
        if !matches!(mode, "replace" | "insert" | "delete") {
            return ToolOutput::err("edit_mode must be replace, insert or delete");
        }
        if mode == "insert" && !matches!(a.cell_type.as_deref(), Some("code" | "markdown")) {
            return ToolOutput::err("insert requires cell_type code or markdown");
        }
        let path = match ctx.resolve(&a.notebook_path) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let mut nb = match load(&path).await {
            Ok(v) => v,
            Err(e) => return ToolOutput::err(e),
        };
        let Some(cells) = nb["cells"].as_array_mut() else {
            return ToolOutput::err("notebook has no cells array");
        };
        let idx = a.cell_id.as_deref().and_then(|id| find_cell(cells, id));
        match mode {
            "delete" => {
                let Some(i) = idx else {
                    return ToolOutput::err("cell_id was not found");
                };
                cells.remove(i);
            }
            "insert" => {
                let kind = a.cell_type.clone().unwrap_or_else(|| "code".into());
                let id = format!("cell-{}", cells.len() + 1);
                let cell = json!({
                    "cell_type": kind,
                    "id": id,
                    "metadata": {},
                    "source": as_source_array(&a.new_source),
                    "outputs": [],
                    "execution_count": null,
                });
                let at = idx.map_or(0, |i| i + 1);
                cells.insert(at, cell);
            }
            _ => {
                let Some(i) = idx.or_else(|| (cells.len() == 1).then_some(0)) else {
                    return ToolOutput::err("cell_id was not found");
                };
                if let Some(kind) = a.cell_type.as_deref() {
                    cells[i]["cell_type"] = json!(kind);
                }
                cells[i]["source"] = as_source_array(&a.new_source);
            }
        }
        let text = match serde_json::to_string_pretty(&nb) {
            Ok(t) => t,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        if let Err(e) = tokio::fs::write(&path, text).await {
            return ToolOutput::err(e.to_string());
        }
        ToolOutput::ok(format!(
            "{mode} {} in {}",
            a.cell_id.unwrap_or_default(),
            rel(&ctx.root, &path)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaces_inserts_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let nb = r#"{"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[{"cell_type":"code","id":"c1","source":["print(1)\n"],"metadata":{},"outputs":[]}]}"#;
        std::fs::write(dir.path().join("n.ipynb"), nb).unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let r = NotebookEdit
            .call(
                json!({"notebook_path": "n.ipynb", "cell_id": "c1", "new_source": "print(2)\n", "edit_mode": "replace"}),
                &ctx,
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        let ins = NotebookEdit
            .call(
                json!({"notebook_path": "n.ipynb", "cell_id": "c1", "new_source": "# note", "cell_type": "markdown", "edit_mode": "insert"}),
                &ctx,
            )
            .await;
        assert!(!ins.is_error, "{}", ins.content);
        let text = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
        assert!(
            text.contains("print(2)") && text.contains("# note"),
            "{text}"
        );
        let del = NotebookEdit
            .call(
                json!({"notebook_path": "n.ipynb", "cell_id": "c1", "new_source": "", "edit_mode": "delete"}),
                &ctx,
            )
            .await;
        assert!(!del.is_error, "{}", del.content);
    }
}
