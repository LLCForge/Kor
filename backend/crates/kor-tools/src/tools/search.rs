//! `glob` and `grep`, both respecting `.gitignore` and friends.

use std::fmt::Write as _;
use std::path::PathBuf;

use async_trait::async_trait;
use kor_core::ToolSpec;
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::jail::rel;
use crate::{Tool, ToolCtx, ToolKind, ToolOutput, parse};

const GLOB_CAP: usize = 100;
const GREP_HEAD: usize = 250;
const VCS: &[&str] = &[".git", ".svn", ".hg", ".bzr", ".jj", ".sl"];
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MATCH_LINE: usize = 300;

/// Files under `base`, honouring ignore files; also skips `.git`.
fn walk(base: PathBuf) -> impl Iterator<Item = PathBuf> {
    let mut b = ignore::WalkBuilder::new(base);
    for name in crate::AI_IGNORE_FILES {
        b.add_custom_ignore_filename(name);
    }
    b.hidden(false)
        .filter_entry(move |e| e.file_name().to_str().is_none_or(|n| !VCS.contains(&n)))
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
}

#[derive(Debug)]
pub(crate) struct Glob;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobIn {
    pattern: String,
    path: Option<String>,
}

#[async_trait]
impl Tool for Glob {
    fn name(&self) -> &'static str {
        "Glob"
    }
    fn rule_name(&self) -> &'static str {
        "Glob"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Find files by glob pattern (e.g. `src/**/*.rs`). Results are paths relative to the project root, at most 100. Ignored files are skipped.".into(),
            input_schema: json!({"type": "object", "properties": {
                "pattern": {"type": "string"}, "path": {"type": "string", "description": "Directory to search (default: root)"}
            }, "required": ["pattern"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        Some(input["path"].as_str().unwrap_or(".").to_owned())
    }
    fn summary(&self, input: &Value) -> String {
        input["pattern"].as_str().unwrap_or_default().to_owned()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: GlobIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let base = match ctx.resolve(a.path.as_deref().unwrap_or(".")) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let matcher = match globset::GlobBuilder::new(&a.pattern)
            .literal_separator(true)
            .build()
        {
            Ok(g) => g.compile_matcher(),
            Err(e) => return ToolOutput::err(format!("invalid pattern: {e}")),
        };
        let root = ctx.root.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut hits: Vec<String> = walk(base.clone())
                .filter(|p| matcher.is_match(rel(&base, p)))
                .map(|p| rel(&root, &p))
                .take(GLOB_CAP + 1)
                .collect();
            hits.sort();
            hits
        })
        .await;
        match result {
            Ok(mut hits) => {
                let more = hits.len() > GLOB_CAP;
                hits.truncate(GLOB_CAP);
                let mut out = hits.join("\n");
                if more {
                    let _ = write!(out, "\n… more than {GLOB_CAP} files; narrow the pattern");
                }
                ToolOutput::ok(if out.is_empty() {
                    "no files found".into()
                } else {
                    out
                })
            }
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Grep;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrepIn {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    output_mode: Option<String>,
    #[serde(rename = "-B")]
    before: Option<usize>,
    #[serde(rename = "-A")]
    after: Option<usize>,
    #[serde(rename = "-C")]
    around_flag: Option<usize>,
    context: Option<usize>,
    #[serde(rename = "-n")]
    line_numbers: Option<bool>,
    #[serde(rename = "-i", alias = "case_insensitive")]
    insensitive: Option<bool>,
    #[serde(rename = "type")]
    file_type: Option<String>,
    head_limit: Option<usize>,
    offset: Option<usize>,
    multiline: Option<bool>,
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "Grep"
    }
    fn rule_name(&self) -> &'static str {
        "Grep"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Search file contents with a regular expression. output_mode is content, files_with_matches (default) or count. head_limit defaults to 250 (0 means no limit). -i is case insensitive. -A/-B/-C/context add lines around content matches. multiline lets `.` match newlines.".into(),
            input_schema: json!({"type": "object", "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string"},
                "glob": {"type": "string"},
                "output_mode": {"type": "string", "enum": ["content", "files_with_matches", "count"]},
                "-B": {"type": "integer"},
                "-A": {"type": "integer"},
                "-C": {"type": "integer"},
                "context": {"type": "integer"},
                "-n": {"type": "boolean"},
                "-i": {"type": "boolean"},
                "type": {"type": "string"},
                "head_limit": {"type": "integer"},
                "offset": {"type": "integer"},
                "multiline": {"type": "boolean"}
            }, "required": ["pattern"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        Some(input["path"].as_str().unwrap_or(".").to_owned())
    }
    fn summary(&self, input: &Value) -> String {
        input["pattern"].as_str().unwrap_or_default().to_owned()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: GrepIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let mode = a.output_mode.as_deref().unwrap_or("files_with_matches");
        if !matches!(mode, "content" | "files_with_matches" | "count") {
            return ToolOutput::err("output_mode must be content, files_with_matches or count");
        }
        let glob_pat = if let Some(g) = a.glob.clone() {
            Some(g)
        } else if let Some(t) = &a.file_type {
            Some(match t.as_str() {
                "rust" | "rs" => "*.rs".into(),
                "py" | "python" => "*.py".into(),
                "js" => "*.js".into(),
                "ts" => "*.ts".into(),
                "tsx" => "*.tsx".into(),
                "go" => "*.go".into(),
                "java" => "*.java".into(),
                "md" | "markdown" => "*.md".into(),
                other => return ToolOutput::err(format!("unknown file type `{other}`")),
            })
        } else {
            None
        };
        let re = match RegexBuilder::new(&a.pattern)
            .case_insensitive(a.insensitive.unwrap_or(false))
            .dot_matches_new_line(a.multiline.unwrap_or(false))
            .size_limit(1 << 20)
            .build()
        {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(format!("invalid regex: {e}")),
        };
        let filter = match glob_pat.as_deref().map(globset::Glob::new).transpose() {
            Ok(g) => g.map(|g| g.compile_matcher()),
            Err(e) => return ToolOutput::err(format!("invalid glob: {e}")),
        };
        let numbered = a.line_numbers.unwrap_or(true);
        let around = a.context.or(a.around_flag).unwrap_or(0);
        let before = a.before.unwrap_or(around);
        let after = a.after.unwrap_or(around);
        let limit = match a.head_limit {
            Some(0) => usize::MAX,
            Some(n) => n,
            None => GREP_HEAD,
        };
        let skip = a.offset.unwrap_or(0);
        let mode = mode.to_owned();
        let base = match ctx.resolve(a.path.as_deref().unwrap_or(".")) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let root = ctx.root.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let files = if base.is_file() {
                vec![base]
            } else {
                walk(base).collect()
            };
            for p in files {
                if let Some(f) = &filter
                    && !p.file_name().is_some_and(|n| f.is_match(n))
                {
                    continue;
                }
                if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&p) else {
                    continue;
                };
                if bytes.iter().take(8192).any(|&b| b == 0) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                let lines: Vec<&str> = text.lines().collect();
                let hits: Vec<usize> = lines
                    .iter()
                    .enumerate()
                    .filter_map(|(i, line)| re.is_match(line).then_some(i))
                    .collect();
                if hits.is_empty() {
                    continue;
                }
                let rel_path = rel(&root, &p);
                match mode.as_str() {
                    "files_with_matches" => out.push(rel_path),
                    "count" => out.push(format!("{rel_path}:{}", hits.len())),
                    _ => {
                        let mut shown = vec![false; lines.len()];
                        for i in hits {
                            let from = i.saturating_sub(before);
                            let to = (i + after).min(lines.len().saturating_sub(1));
                            if to >= from {
                                shown[from..=to].fill(true);
                            }
                        }
                        for (j, line) in lines.iter().enumerate() {
                            if !shown[j] {
                                continue;
                            }
                            let line: String = line.chars().take(MAX_MATCH_LINE).collect();
                            if numbered {
                                out.push(format!("{rel_path}:{}:{line}", j + 1));
                            } else {
                                out.push(format!("{rel_path}:{line}"));
                            }
                        }
                    }
                }
                if out.len() > limit {
                    break;
                }
            }
            out
        })
        .await;
        match result {
            Ok(mut hits) => {
                if skip > 0 {
                    hits = hits.into_iter().skip(skip).collect();
                }
                let truncated = hits.len() > limit;
                hits.truncate(limit);
                let mut out = hits.join("\n");
                if truncated {
                    let _ = write!(out, "\n… more than {limit} matches; narrow the search");
                }
                ToolOutput::ok(if out.is_empty() {
                    "no matches".into()
                } else {
                    out
                })
            }
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn glob_and_grep_respect_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join(".cursorignore"), "secret.rs\n").unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {}\n// TODO fix\n").unwrap();
        std::fs::write(root.join("src/deep/b.rs"), "fn helper() {}\n").unwrap();
        std::fs::write(root.join("src/secret.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("target/c.rs"), "fn main() {}\n").unwrap();
        // `ignore` only honours .gitignore inside a git repository.
        std::fs::create_dir(root.join(".git")).unwrap();
        let mut ctx = ToolCtx::new(root).unwrap();
        assert!(ctx.load_ai_ignore().is_empty());
        assert!(
            crate::tools::fs::ReadFile
                .call(json!({"path": "src/secret.rs"}), &ctx)
                .await
                .is_error
        );

        let g = Glob.call(json!({"pattern": "**/*.rs"}), &ctx).await;
        assert_eq!(g.content, "src/a.rs\nsrc/deep/b.rs");
        let g = Glob
            .call(json!({"pattern": "*.rs", "path": "src"}), &ctx)
            .await;
        assert_eq!(g.content, "src/a.rs");

        let r = Grep.call(json!({"pattern": "fn main"}), &ctx).await;
        assert_eq!(r.content, "src/a.rs");
        let r = Grep
            .call(
                json!({"pattern": "fn main", "output_mode": "content"}),
                &ctx,
            )
            .await;
        assert_eq!(r.content, "src/a.rs:1:fn main() {}");
        let r = Grep
            .call(
                json!({"pattern": "todo", "case_insensitive": true, "glob": "*.rs", "output_mode": "content"}),
                &ctx,
            )
            .await;
        assert_eq!(r.content, "src/a.rs:2:// TODO fix");
        assert!(Grep.call(json!({"pattern": "("}), &ctx).await.is_error);
        assert!(
            Glob.call(json!({"pattern": "*", "path": ".."}), &ctx)
                .await
                .is_error
        );
    }
}
