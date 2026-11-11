//! `apply_patch`: the V4A patch format OpenAI/Codex models are trained to emit.
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path        (+ lines)
//! *** Delete File: path
//! *** Update File: path
//! *** Move to: new/path     (optional)
//! @@ optional context
//!  context line
//! -removed line
//! +added line
//! *** End Patch
//! ```
//!
//! Every hunk must match the file exactly once (after the previous hunk). Nothing is
//! written unless the whole patch applies.

use async_trait::async_trait;
use kor_core::ToolSpec;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<(String, String)>,
    },
}

fn parse(patch: &str) -> Result<Vec<Op>, String> {
    let text = patch.replace("\r\n", "\n");
    let mut lines = text.lines().peekable();
    // Tolerate a missing Begin line and leading prose/fences.
    while let Some(l) = lines.peek() {
        if l.starts_with("*** ") {
            break;
        }
        lines.next();
    }
    if lines.peek() == Some(&"*** Begin Patch") {
        lines.next();
    }
    let mut ops = Vec::new();
    while let Some(line) = lines.next() {
        if line == "*** End Patch" {
            break;
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let mut content = String::new();
            while let Some(l) = lines.peek().filter(|l| !l.starts_with("*** ")) {
                content.push_str(l.strip_prefix('+').unwrap_or(l));
                content.push('\n');
                lines.next();
            }
            ops.push(Op::Add {
                path: path.trim().to_owned(),
                content,
            });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(Op::Delete {
                path: path.trim().to_owned(),
            });
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            let mut move_to = None;
            if let Some(dest) = lines.peek().and_then(|l| l.strip_prefix("*** Move to: ")) {
                move_to = Some(dest.trim().to_owned());
                lines.next();
            }
            let mut hunks = Vec::new();
            let (mut old, mut new, mut started) = (Vec::new(), Vec::new(), false);
            let flush = |old: &mut Vec<String>,
                         new: &mut Vec<String>,
                         hunks: &mut Vec<(String, String)>| {
                if !old.is_empty() || !new.is_empty() {
                    hunks.push((old.join("\n"), new.join("\n")));
                }
                old.clear();
                new.clear();
            };
            while let Some(l) = lines.peek().copied() {
                if l.starts_with("*** ") && l != "*** End of File" {
                    break;
                }
                lines.next();
                if l == "*** End of File" {
                    continue;
                }
                if l.starts_with("@@") {
                    if started {
                        flush(&mut old, &mut new, &mut hunks);
                    }
                    started = true;
                    continue;
                }
                started = true;
                match l.chars().next() {
                    Some('-') => old.push(l[1..].to_owned()),
                    Some('+') => new.push(l[1..].to_owned()),
                    Some(' ') => {
                        old.push(l[1..].to_owned());
                        new.push(l[1..].to_owned());
                    }
                    None => {
                        old.push(String::new());
                        new.push(String::new());
                    }
                    _ => return Err(format!("unexpected patch line: {l}")),
                }
            }
            flush(&mut old, &mut new, &mut hunks);
            if hunks.is_empty() && move_to.is_none() {
                return Err(format!("update of {path} has no changes"));
            }
            ops.push(Op::Update {
                path: path.trim().to_owned(),
                move_to,
                hunks,
            });
        } else if !line.trim().is_empty() && !line.starts_with("```") {
            return Err(format!("unexpected patch line: {line}"));
        }
    }
    if ops.is_empty() {
        return Err("patch contains no file operations".into());
    }
    Ok(ops)
}

/// Apply hunks in order; each old block must occur once at or after the previous hunk.
fn apply_hunks(text: &str, hunks: &[(String, String)]) -> Result<String, String> {
    let mut out = text.to_owned();
    let mut from = 0;
    for (i, (old, new)) in hunks.iter().enumerate() {
        if old.is_empty() {
            // Pure insertion with no context: append.
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(new);
            out.push('\n');
            continue;
        }
        // Only whole-line matches count, so `one` does not hit `someone`.
        let hits: Vec<usize> = out[from..]
            .match_indices(old.as_str())
            .map(|(p, _)| from + p)
            .filter(|&at| {
                (at == 0 || out.as_bytes()[at - 1] == b'\n')
                    && matches!(out.as_bytes().get(at + old.len()), None | Some(b'\n'))
            })
            .collect();
        let at = match hits.as_slice() {
            [] => return Err(format!("hunk {} does not match the file", i + 1)),
            [at] => *at,
            _ => {
                return Err(format!(
                    "hunk {} matches more than once; add context",
                    i + 1
                ));
            }
        };
        out.replace_range(at..at + old.len(), new);
        from = at + new.len();
    }
    Ok(out)
}

type Writes = Vec<(std::path::PathBuf, Option<String>)>;

/// Plan everything first so a failing hunk leaves the tree untouched. `None` content deletes.
async fn plan(ops: &[Op], ctx: &ToolCtx) -> Result<(Writes, Vec<String>), String> {
    let mut writes: Writes = Vec::new();
    let mut summary = Vec::new();
    for op in ops {
        match op {
            Op::Add { path, content } => {
                let p = ctx.resolve(path)?;
                if p.exists() {
                    return Err(format!("{path} already exists; use Update File"));
                }
                writes.push((p, Some(content.clone())));
                summary.push(format!("A {path}"));
            }
            Op::Delete { path } => {
                let p = ctx.resolve(path)?;
                if !p.is_file() {
                    return Err(format!("{path} does not exist"));
                }
                writes.push((p, None));
                summary.push(format!("D {path}"));
            }
            Op::Update {
                path,
                move_to,
                hunks,
            } => {
                let p = ctx.resolve(path)?;
                let raw = tokio::fs::read_to_string(&p)
                    .await
                    .map_err(|e| format!("{path}: {e}"))?;
                let crlf = raw.contains("\r\n");
                let mut text = apply_hunks(&raw.replace("\r\n", "\n"), hunks)
                    .map_err(|e| format!("{path}: {e}"))?;
                if crlf {
                    text = text.replace('\n', "\r\n");
                }
                if let Some(dest) = move_to {
                    let d = ctx.resolve(dest)?;
                    writes.push((d, Some(text)));
                    writes.push((p, None));
                    summary.push(format!("R {path} -> {dest}"));
                } else {
                    writes.push((p, Some(text)));
                    summary.push(format!("M {path}"));
                }
            }
        }
    }
    Ok((writes, summary))
}

#[derive(Debug)]
pub(crate) struct ApplyPatch;

#[async_trait]
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
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
            description: "Apply a multi-file patch in the V4A format (*** Begin Patch / *** Add File / *** Update File / *** Delete File, hunks with ' ', '-', '+' lines, *** End Patch). Nothing is written unless every hunk matches.".into(),
            input_schema: json!({"type": "object", "properties": {
                "patch": {"type": "string", "description": "The full patch text"}
            }, "required": ["patch"], "additionalProperties": false}),
        }
    }
    fn touches(&self, input: &Value) -> Vec<String> {
        let ops = input["patch"]
            .as_str()
            .and_then(|p| parse(p).ok())
            .unwrap_or_default();
        ops.into_iter()
            .flat_map(|o| match o {
                Op::Add { path, .. } | Op::Delete { path } => vec![path],
                Op::Update { path, move_to, .. } => std::iter::once(path).chain(move_to).collect(),
            })
            .collect()
    }
    fn target(&self, input: &Value) -> Option<String> {
        let ops = parse(input["patch"].as_str()?).ok()?;
        ops.first().map(|o| match o {
            Op::Add { path, .. } | Op::Delete { path } | Op::Update { path, .. } => path.clone(),
        })
    }
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<super::review::Proposal>> {
        let (writes, _) = plan(&parse(input["patch"].as_str()?).ok()?, ctx)
            .await
            .ok()?;
        let mut out = Vec::new();
        for (p, after) in writes {
            let before = tokio::fs::read_to_string(&p).await.ok();
            out.push(super::review::proposal(ctx, &p, before, after));
        }
        Some(out)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(patch) = input["patch"].as_str() else {
            return ToolOutput::err("invalid input: `patch` is required");
        };
        let ops = match parse(patch) {
            Ok(o) => o,
            Err(e) => return ToolOutput::err(e),
        };
        let (writes, summary) = match plan(&ops, ctx).await {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        for (path, content) in writes {
            let r = match content {
                Some(c) => {
                    async {
                        if let Some(dir) = path.parent() {
                            tokio::fs::create_dir_all(dir).await?;
                        }
                        let tmp = path.with_extension("kor-patch-tmp");
                        tokio::fs::write(&tmp, c.as_bytes()).await?;
                        tokio::fs::rename(&tmp, &path).await
                    }
                    .await
                }
                None => tokio::fs::remove_file(&path).await,
            };
            if let Err(e) = r {
                return ToolOutput::err(format!(
                    "{}: {e} (patch partially applied)",
                    path.display()
                ));
            }
        }
        ToolOutput::ok(format!("applied patch:\n{}", summary.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_update_delete_and_move() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("old.txt"), "bye\n").unwrap();
        std::fs::write(dir.path().join("crlf.txt"), "x\r\ny\r\n").unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let patch = "*** Begin Patch\n*** Update File: a.rs\n@@ fn a\n fn a() {\n-    1\n+    10\n }\n@@ fn b\n fn b() {\n-    2\n+    20\n }\n*** Add File: new/n.txt\n+hello\n+world\n*** Delete File: old.txt\n*** Update File: crlf.txt\n*** Move to: moved.txt\n-y\n+z\n*** End Patch";
        let out = ApplyPatch.call(json!({"patch": patch}), &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "fn a() {\n    10\n}\n\nfn b() {\n    20\n}\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new/n.txt")).unwrap(),
            "hello\nworld\n"
        );
        assert!(!dir.path().join("old.txt").exists());
        assert!(!dir.path().join("crlf.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("moved.txt")).unwrap(),
            "x\r\nz\r\n"
        );
    }

    #[tokio::test]
    async fn failing_hunk_writes_nothing_and_jail_holds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let patch = "*** Begin Patch\n*** Add File: b.txt\n+x\n*** Update File: a.txt\n-missing\n+y\n*** End Patch";
        assert!(
            ApplyPatch
                .call(json!({"patch": patch}), &ctx)
                .await
                .is_error
        );
        assert!(!dir.path().join("b.txt").exists());
        let escape = "*** Begin Patch\n*** Add File: ../evil.txt\n+x\n*** End Patch";
        assert!(
            ApplyPatch
                .call(json!({"patch": escape}), &ctx)
                .await
                .is_error
        );
        let dup = "*** Begin Patch\n*** Update File: a.txt\n-o\n+0\n*** End Patch";
        std::fs::write(dir.path().join("a.txt"), "o\no\n").unwrap();
        assert!(
            ApplyPatch
                .call(json!({"patch": dup}), &ctx)
                .await
                .content
                .contains("more than once")
        );
    }
}
