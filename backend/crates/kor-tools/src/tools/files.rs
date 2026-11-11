//! `MultiEdit`, `Delete`, and `Move`: file operations beyond a single replacement, jailed to the
//! project like `Edit`. `MultiEdit` is all-or-nothing; `Delete` asks like a command (it cannot be
//! undone); the project root itself can be neither deleted nor moved.

use std::path::Path;

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde::Deserialize;
use serde_json::{Value, json};

use super::fs::{apply_one, mtime_ms, remember, require_fresh_read, resolve_in, write_atomic};
use crate::jail::rel;
use crate::{ReadStamp, Tool, ToolCtx, ToolKind, ToolOutput, field, lock, parse};

const MAX_EDITS: usize = 100;

fn is_root(ctx: &ToolCtx, path: &Path) -> bool {
    path == ctx.root || ctx.extra_roots.iter().any(|r| r == path)
}

fn forget(ctx: &ToolCtx, path: &Path) {
    lock(&ctx.read_files).retain(|p, _| !p.starts_with(path));
}

#[derive(Debug)]
pub(crate) struct MultiEdit;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OneEdit {
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiEditIn {
    #[serde(alias = "path")]
    file_path: String,
    edits: Vec<OneEdit>,
}

#[async_trait]
impl Tool for MultiEdit {
    fn name(&self) -> &'static str {
        "MultiEdit"
    }
    fn rule_name(&self) -> &'static str {
        "Edit"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Make several exact-text replacements in one file, in order; each edit sees the result of the previous one. All edits succeed or none is written. Same rules as Edit for each: old_string must match once unless replace_all is true, and the file must have been Read in full first. Prefer this over several Edit calls on the same file.".into(),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "edits": {"type": "array", "minItems": 1, "maxItems": MAX_EDITS, "items": {"type": "object", "properties": {
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean"}
                }, "required": ["old_string", "new_string"], "additionalProperties": false}}
            }, "required": ["file_path", "edits"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        field(input, &["file_path", "path"]).map(str::to_owned)
    }
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<super::review::Proposal>> {
        let a: MultiEditIn = parse(input.clone()).ok()?;
        if a.edits.is_empty() || a.edits.len() > MAX_EDITS {
            return None;
        }
        let new: String = a.edits.iter().map(|e| e.new_string.as_str()).collect();
        let path = super::fs::writable(ctx, &a.file_path, &new)?;
        let file = tokio::fs::read_to_string(&path).await.ok()?;
        require_fresh_read(ctx, &path, &file).ok()?;
        let mut text = file.clone();
        for e in &a.edits {
            if e.old_string.is_empty() {
                return None;
            }
            text = apply_one(&text, &e.old_string, &e.new_string, e.replace_all).ok()?;
        }
        Some(vec![super::review::proposal(
            ctx,
            &path,
            Some(file),
            Some(text),
        )])
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: MultiEditIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.edits.is_empty() || a.edits.len() > MAX_EDITS {
            return ToolOutput::err(format!("edits must hold 1 to {MAX_EDITS} items"));
        }
        let path = match resolve_in(ctx, &a.file_path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if path.extension().is_some_and(|e| e == "ipynb") {
            return ToolOutput::err(
                "File is a Jupyter Notebook. Use the NotebookEdit tool to edit this file.",
            );
        }
        if let Some(team) = &ctx.team_memory {
            for e in &a.edits {
                if let Some(msg) = crate::secret_scan::block_team_write(&path, team, &e.new_string)
                {
                    return ToolOutput::err(msg);
                }
            }
        }
        let Ok(file) = tokio::fs::read_to_string(&path).await else {
            return ToolOutput::err(
                "File does not exist or is not UTF-8 text. Create it with Write first.",
            );
        };
        if let Err(e) = require_fresh_read(ctx, &path, &file) {
            return ToolOutput::err(e);
        }
        let mut text = file;
        for (i, e) in a.edits.iter().enumerate() {
            if e.old_string.is_empty() {
                return ToolOutput::err(format!(
                    "edit {}: old_string is empty; nothing was written",
                    i + 1
                ));
            }
            match apply_one(&text, &e.old_string, &e.new_string, e.replace_all) {
                Ok(next) => text = next,
                Err(msg) => {
                    return ToolOutput::err(format!("edit {}: {msg}\nNothing was written.", i + 1));
                }
            }
        }
        if let Err(e) = write_atomic(&path, text.as_bytes()).await {
            return ToolOutput::err(e.to_string());
        }
        remember(
            ctx,
            &path,
            ReadStamp {
                mtime_ms: mtime_ms(&path),
                partial: false,
                from_read: false,
                content: text,
                offset: 1,
                limit: None,
            },
        );
        ToolOutput::ok(format!(
            "applied {} edit(s) to {}",
            a.edits.len(),
            rel(&ctx.root, &path)
        ))
    }
}

#[derive(Debug)]
pub(crate) struct Delete;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteIn {
    #[serde(alias = "file_path")]
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[async_trait]
impl Tool for Delete {
    fn name(&self) -> &'static str {
        "Delete"
    }
    fn rule_name(&self) -> &'static str {
        "Delete"
    }
    // A deletion cannot be undone, so it asks like a command, not like an edit.
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Delete a file, or a directory. A non-empty directory needs recursive: true. The project root cannot be deleted. This cannot be undone; prefer it over shell commands like rm or Remove-Item.".into(),
            input_schema: json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "recursive": {"type": "boolean", "description": "Delete a directory with everything in it"}
            }, "required": ["path"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        field(input, &["path", "file_path"]).map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: DeleteIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let path = match resolve_in(ctx, &a.path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if is_root(ctx, &path) {
            return ToolOutput::err("Refusing to delete the project root.");
        }
        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(m) => m,
            Err(e) => return ToolOutput::err(format!("{}: {e}", rel(&ctx.root, &path))),
        };
        let shown = rel(&ctx.root, &path);
        let result = if !meta.is_dir() {
            tokio::fs::remove_file(&path).await
        } else if a.recursive {
            tokio::fs::remove_dir_all(&path).await
        } else {
            tokio::fs::remove_dir(&path).await
        };
        match result {
            Ok(()) => {
                forget(ctx, &path);
                ToolOutput::ok(format!("deleted {shown}"))
            }
            Err(_) if meta.is_dir() && !a.recursive => ToolOutput::err(format!(
                "{shown} is a directory that is not empty; pass recursive: true to delete it with its contents"
            )),
            Err(e) => ToolOutput::err(format!("{shown}: {e}")),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Move;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveIn {
    source: String,
    destination: String,
    #[serde(default)]
    overwrite: bool,
}

#[async_trait]
impl Tool for Move {
    fn name(&self) -> &'static str {
        "Move"
    }
    fn rule_name(&self) -> &'static str {
        "Edit"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Move or rename a file or directory inside the project. Missing parent directories of the destination are created. An existing destination file is replaced only with overwrite: true; an existing destination directory is never replaced.".into(),
            input_schema: json!({"type": "object", "properties": {
                "source": {"type": "string"},
                "destination": {"type": "string"},
                "overwrite": {"type": "boolean"}
            }, "required": ["source", "destination"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        let source = field(input, &["source"])?;
        let destination = field(input, &["destination"]).unwrap_or("");
        Some(format!("{source} -> {destination}"))
    }
    fn touches(&self, input: &Value) -> Vec<String> {
        ["source", "destination"]
            .iter()
            .filter_map(|k| field(input, &[k]).map(str::to_owned))
            .collect()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: MoveIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let (from, to) = match (resolve_in(ctx, &a.source), resolve_in(ctx, &a.destination)) {
            (Ok(f), Ok(t)) => (f, t),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        if is_root(ctx, &from) || is_root(ctx, &to) {
            return ToolOutput::err("Refusing to move the project root or replace it.");
        }
        if from == to {
            return ToolOutput::err("source and destination are the same path");
        }
        let Ok(meta) = tokio::fs::symlink_metadata(&from).await else {
            return ToolOutput::err(format!("{} does not exist", rel(&ctx.root, &from)));
        };
        if meta.is_dir() && to.starts_with(&from) {
            return ToolOutput::err("Cannot move a directory into itself.");
        }
        if let Ok(existing) = tokio::fs::symlink_metadata(&to).await {
            if existing.is_dir() {
                return ToolOutput::err(format!(
                    "{} is an existing directory; give the full destination path including the new name",
                    rel(&ctx.root, &to)
                ));
            }
            if !a.overwrite {
                return ToolOutput::err(format!(
                    "{} already exists; pass overwrite: true to replace it",
                    rel(&ctx.root, &to)
                ));
            }
            if meta.is_dir() {
                return ToolOutput::err("Cannot replace a file with a directory.");
            }
        }
        if let Some(dir) = to.parent()
            && let Err(e) = tokio::fs::create_dir_all(dir).await
        {
            return ToolOutput::err(e.to_string());
        }
        // shortcut: rename only, so a move across volumes fails; project paths share one volume.
        match tokio::fs::rename(&from, &to).await {
            Ok(()) => {
                forget(ctx, &from);
                forget(ctx, &to);
                ToolOutput::ok(format!(
                    "moved {} -> {}",
                    rel(&ctx.root, &from),
                    rel(&ctx.root, &to)
                ))
            }
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::fs::ReadFile;

    async fn read(ctx: &ToolCtx, path: &str) {
        let out = ReadFile.call(json!({"file_path": path}), ctx).await;
        assert!(!out.is_error, "{}", out.content);
    }

    #[tokio::test]
    async fn multi_edit_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let file = ctx.root.join("a.txt");
        std::fs::write(&file, "one two\r\nthree\r\n").unwrap();

        let unread = MultiEdit
            .call(
                json!({"file_path": "a.txt", "edits": [{"old_string": "one", "new_string": "1"}]}),
                &ctx,
            )
            .await;
        assert!(unread.is_error && unread.content.contains("Read it first"));

        read(&ctx, "a.txt").await;
        let bad = MultiEdit
            .call(
                json!({"file_path": "a.txt", "edits": [
                    {"old_string": "one", "new_string": "1"},
                    {"old_string": "missing", "new_string": "x"}
                ]}),
                &ctx,
            )
            .await;
        assert!(
            bad.is_error && bad.content.starts_with("edit 2:"),
            "{}",
            bad.content
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "one two\r\nthree\r\n"
        );

        let ok = MultiEdit
            .call(
                json!({"file_path": "a.txt", "edits": [
                    {"old_string": "one", "new_string": "1"},
                    {"old_string": "1 two", "new_string": "1 2"},
                    {"old_string": "three\nx", "new_string": "?"}
                ]}),
                &ctx,
            )
            .await;
        assert!(
            ok.is_error,
            "the third edit fails on the result of the first two"
        );
        let ok = MultiEdit
            .call(
                json!({"file_path": "a.txt", "edits": [
                    {"old_string": "one", "new_string": "1"},
                    {"old_string": "1 two", "new_string": "1 2"}
                ]}),
                &ctx,
            )
            .await;
        assert!(!ok.is_error, "{}", ok.content);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "1 2\r\nthree\r\n");
    }

    #[tokio::test]
    async fn delete_refuses_the_root_and_outside_and_needs_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        std::fs::create_dir_all(ctx.root.join("d/sub")).unwrap();
        std::fs::write(ctx.root.join("d/sub/f.txt"), "x").unwrap();
        std::fs::write(ctx.root.join("g.txt"), "x").unwrap();

        for path in [".", "", "..", "../x"] {
            let out = Delete.call(json!({"path": path}), &ctx).await;
            assert!(out.is_error, "{path}: {}", out.content);
        }
        assert!(ctx.root.exists());

        let out = Delete.call(json!({"path": "d"}), &ctx).await;
        assert!(
            out.is_error && out.content.contains("recursive"),
            "{}",
            out.content
        );
        let out = Delete
            .call(json!({"path": "d", "recursive": true}), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(!ctx.root.join("d").exists());

        let out = Delete.call(json!({"path": "g.txt"}), &ctx).await;
        assert_eq!(out.content, "deleted g.txt");
        assert_eq!(Delete.kind(), ToolKind::Execute);
    }

    #[tokio::test]
    async fn move_renames_and_guards_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        std::fs::write(ctx.root.join("a.txt"), "a").unwrap();
        std::fs::write(ctx.root.join("b.txt"), "b").unwrap();
        std::fs::create_dir(ctx.root.join("dir")).unwrap();

        let out = Move
            .call(
                json!({"source": "a.txt", "destination": "new/deep/a.txt"}),
                &ctx,
            )
            .await;
        assert_eq!(out.content, "moved a.txt -> new/deep/a.txt");
        assert_eq!(
            std::fs::read_to_string(ctx.root.join("new/deep/a.txt")).unwrap(),
            "a"
        );

        let out = Move
            .call(
                json!({"source": "new/deep/a.txt", "destination": "b.txt"}),
                &ctx,
            )
            .await;
        assert!(out.is_error && out.content.contains("overwrite"));
        let out = Move
            .call(
                json!({"source": "new/deep/a.txt", "destination": "b.txt", "overwrite": true}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(ctx.root.join("b.txt")).unwrap(),
            "a"
        );

        for (s, d) in [
            ("b.txt", "dir"),
            ("new", "new/inner"),
            (".", "x"),
            ("b.txt", "../b.txt"),
        ] {
            let out = Move
                .call(json!({"source": s, "destination": d}), &ctx)
                .await;
            assert!(out.is_error, "{s} -> {d}: {}", out.content);
        }
    }
}
