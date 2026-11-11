//! `skill` tool: progressive loading of `SKILL.md` skills that may live outside the
//! project (e.g. `~/.claude/skills`), confined to each skill's own directory.

use std::fmt::Write as _;
use std::io::Read;

use async_trait::async_trait;
use kor_compat::Skill;
use kor_core::ToolSpec;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};

const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug)]
pub struct SkillTool {
    pub skills: Vec<Skill>,
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }
    fn rule_name(&self) -> &'static str {
        "Skill"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Load a skill's instructions (SKILL.md) by name, or one of its bundled files with `file`. Use it when a listed skill matches the task.".into(),
            input_schema: json!({"type": "object", "properties": {
                "name": {"type": "string"},
                "file": {"type": "string", "description": "Path inside the skill directory"}
            }, "required": ["name"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["name"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let Some(name) = input["name"].as_str() else {
            return ToolOutput::err("invalid input: `name` is required");
        };
        let Some(skill) = self.skills.iter().find(|s| s.name == name) else {
            let names: Vec<_> = self
                .skills
                .iter()
                .take(50)
                .map(|s| s.name.as_str())
                .collect();
            let more = self.skills.len().saturating_sub(names.len());
            let tail = if more > 0 {
                format!(" (and {more} more)")
            } else {
                String::new()
            };
            return ToolOutput::err(format!(
                "unknown skill `{name}`; available: {}{tail}",
                names.join(", ")
            ));
        };
        let Ok(dir) = skill.dir.canonicalize() else {
            return ToolOutput::err("skill directory is missing");
        };
        let file = input["file"].as_str().unwrap_or("SKILL.md");
        let path = match kor_tools::resolve(&dir, file) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let mut text = String::new();
        if let Err(e) =
            std::fs::File::open(&path).and_then(|f| f.take(MAX_BYTES).read_to_string(&mut text))
        {
            return ToolOutput::err(format!("{file}: {e}"));
        }
        if file == "SKILL.md" {
            let mut entries: Vec<String> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != "SKILL.md")
                .collect();
            entries.sort();
            if !entries.is_empty() {
                let _ = write!(
                    text,
                    "\n\n[Bundled files, readable with skill(name, file): {}]",
                    entries.join(", ")
                );
            }
        }
        ToolOutput::ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_compat::{Eco, Scope, Source};

    #[tokio::test]
    async fn loads_body_and_bundled_files_but_not_outside() {
        let dir = tempfile::tempdir().unwrap();
        let sk = dir.path().join("pdf");
        std::fs::create_dir(&sk).unwrap();
        std::fs::write(sk.join("SKILL.md"), "---\nname: pdf\n---\nUse pdftotext.").unwrap();
        std::fs::write(sk.join("forms.md"), "Fill forms.").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "nope").unwrap();
        let tool = SkillTool {
            skills: vec![Skill {
                name: "pdf".into(),
                description: "PDFs".into(),
                dir: sk.clone(),
                source: Source {
                    eco: Eco::Claude,
                    scope: Scope::User,
                    path: sk.join("SKILL.md"),
                },
            }],
        };
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let o = tool.call(json!({"name": "pdf"}), &ctx).await;
        assert!(o.content.contains("Use pdftotext.") && o.content.contains("forms.md"));
        assert_eq!(
            tool.call(json!({"name": "pdf", "file": "forms.md"}), &ctx)
                .await
                .content,
            "Fill forms."
        );
        assert!(
            tool.call(json!({"name": "pdf", "file": "../secret.txt"}), &ctx)
                .await
                .is_error
        );
        assert!(tool.call(json!({"name": "nope"}), &ctx).await.is_error);
    }
}
