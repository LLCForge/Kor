//! Per-file review before a write (spec §12.3, ADR 0030): what an edit tool would write,
//! computed without writing, and the writer for the files the user accepted.

use serde::Serialize;

use super::fs::{mtime_ms, remember, resolve_in, write_atomic};
use crate::{ReadStamp, ToolCtx, lock};

/// One file an edit tool would change. `after: None` deletes the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Proposal {
    /// Project-relative path (absolute outside the root); resolved again on write.
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub(crate) fn proposal(
    ctx: &ToolCtx,
    path: &std::path::Path,
    before: Option<String>,
    after: Option<String>,
) -> Proposal {
    Proposal {
        path: crate::jail::rel(&ctx.root, path),
        before,
        after,
    }
}

/// Why writing `files` is destructive: a deletion, or replacing a file the model never read.
pub fn review_risk(ctx: &ToolCtx, files: &[Proposal]) -> Option<&'static str> {
    if files
        .iter()
        .any(|f| f.after.is_none() && f.before.is_some())
    {
        return Some("delete-file");
    }
    let read = lock(&ctx.read_files);
    files
        .iter()
        .any(|f| {
            f.before.as_deref().is_some_and(|b| !b.trim().is_empty())
                && ctx
                    .resolve(&f.path)
                    .ok()
                    .is_none_or(|p| !read.get(&p).is_some_and(|s| s.from_read))
        })
        .then_some("overwrite-unread")
}

/// Write the reviewed files in order: `Some` is written, `None` deletes. Stops at the first error.
///
/// # Errors
/// The path and the I/O or policy error; earlier files stay written.
pub async fn write_reviewed(
    ctx: &ToolCtx,
    files: &[(String, Option<String>)],
) -> Result<(), String> {
    for (path, content) in files {
        let p = resolve_in(ctx, path).map_err(|o| o.content)?;
        if let Some(c) = content {
            if let Some(team) = &ctx.team_memory
                && let Some(msg) = crate::secret_scan::block_team_write(&p, team, c)
            {
                return Err(msg);
            }
            if let Some(dir) = p.parent() {
                tokio::fs::create_dir_all(dir)
                    .await
                    .map_err(|e| format!("{path}: {e}"))?;
            }
            write_atomic(&p, c.as_bytes())
                .await
                .map_err(|e| format!("{path}: {e}"))?;
            remember(
                ctx,
                &p,
                ReadStamp {
                    mtime_ms: mtime_ms(&p),
                    partial: false,
                    from_read: false,
                    content: c.clone(),
                    offset: 1,
                    limit: None,
                },
            );
        } else {
            match tokio::fs::remove_file(&p).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{path}: {e}")),
            }
            lock(&ctx.read_files).remove(&p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn proposals_write_nothing_and_match_the_tool() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        std::fs::write(ctx.root.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(ctx.root.join("old.txt"), "bye\n").unwrap();

        // Edit of an unread file: no preview (the tool would refuse it).
        let edit = json!({"file_path": "a.txt", "old_string": "one", "new_string": "1"});
        assert!(super::super::fs::Edit.propose(&edit, &ctx).await.is_none());
        super::super::fs::ReadFile
            .call(json!({"file_path": "a.txt"}), &ctx)
            .await;
        let p = super::super::fs::Edit.propose(&edit, &ctx).await.unwrap();
        assert_eq!(p[0].path, "a.txt");
        assert_eq!(p[0].after.as_deref(), Some("1\ntwo\n"));
        assert_eq!(
            std::fs::read_to_string(ctx.root.join("a.txt")).unwrap(),
            "one\ntwo\n"
        );

        let w = json!({"file_path": "new/b.txt", "content": "b"});
        let p = super::super::fs::WriteFile.propose(&w, &ctx).await.unwrap();
        assert_eq!(
            (p[0].before.clone(), p[0].after.as_deref()),
            (None, Some("b"))
        );
        assert!(!ctx.root.join("new").exists());

        let patch = "*** Begin Patch\n*** Update File: a.txt\n-two\n+2\n*** Delete File: old.txt\n*** End Patch";
        let p = super::super::patch::ApplyPatch
            .propose(&json!({"patch": patch}), &ctx)
            .await
            .unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            p[1],
            Proposal {
                path: "old.txt".into(),
                before: Some("bye\n".into()),
                after: None
            }
        );
        assert_eq!(review_risk(&ctx, &p), Some("delete-file"));
        assert!(ctx.root.join("old.txt").exists());
    }

    #[tokio::test]
    async fn risk_flags_unread_overwrites_and_writer_obeys_the_jail() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        std::fs::write(ctx.root.join("a.txt"), "keep\n").unwrap();
        let over = [Proposal {
            path: "a.txt".into(),
            before: Some("keep\n".into()),
            after: Some("x".into()),
        }];
        assert_eq!(review_risk(&ctx, &over), Some("overwrite-unread"));
        let fresh = [Proposal {
            path: "n.txt".into(),
            before: None,
            after: Some("x".into()),
        }];
        assert_eq!(review_risk(&ctx, &fresh), None);

        write_reviewed(
            &ctx,
            &[
                ("d/n.txt".into(), Some("hi".into())),
                ("a.txt".into(), None),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(ctx.root.join("d/n.txt")).unwrap(),
            "hi"
        );
        assert!(!ctx.root.join("a.txt").exists());
        assert!(
            write_reviewed(&ctx, &[("../evil.txt".into(), Some("x".into()))])
                .await
                .is_err()
        );
        assert!(!dir.path().parent().unwrap().join("evil.txt").exists());
    }
}
