//! Per-file review of a write in the terminal (§12.3, ADR 0030): the line REPL asks file by
//! file (`a`/`e`/`r`, `e` opens `$EDITOR`), the full screen toggles files by number.

use std::fmt::Write as _;

use kor_agent::{FileDecision, PermissionReply, Proposal, Review};
use kor_i18n::{Localizer, args};

/// "Destructive: <reason>" for a reason key from the core classifier.
pub fn destructive(l: &Localizer, key: &str) -> String {
    l.t_args(
        "destructive",
        &args!("reason" => l.t(&format!("destructive-{key}"))),
    )
}

/// Unified diff of one proposal (a deletion diffs against empty).
pub fn diff(f: &Proposal) -> String {
    kor_agent::port::unified_diff(
        &f.path,
        f.before.as_deref().unwrap_or(""),
        f.after.as_deref().unwrap_or(""),
    )
}

/// The full screen's answer: the key pressed, minus the files toggled off by number.
pub fn from_marks(reply: PermissionReply, files: &[String], rejected: &[bool]) -> Review {
    let mut r = Review::from(reply);
    for (path, off) in files.iter().zip(rejected) {
        if *off {
            r.files.insert(path.clone(), FileDecision::Reject);
        }
    }
    r
}

/// Files edited on the full screen are written as edited (unless they were toggled off).
pub fn apply_edits(r: &mut Review, files: &[String], rejected: &[bool], edited: &[Option<String>]) {
    for ((path, off), text) in files.iter().zip(rejected).zip(edited) {
        if let (false, Some(t)) = (*off, text) {
            r.files.insert(path.clone(), FileDecision::Edit(t.clone()));
        }
    }
}

/// ` · 1 a.rs ✓ · 2 b.rs ✗` for the full screen's prompt line.
pub fn marks(files: &[String], rejected: &[bool]) -> String {
    let mut out = String::new();
    for (i, (p, off)) in files.iter().zip(rejected).enumerate() {
        let _ = write!(out, " · {} {p} {}", i + 1, if *off { '✗' } else { '✓' });
    }
    out
}

/// Open `text` in `$VISUAL` / `$EDITOR` (Notepad / vi when unset) and return what was saved.
///
/// # Errors
/// When the editor cannot start, exits with an error, or the file cannot be read back.
pub fn edit_in_editor(path: &str, text: &str) -> Result<String, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = std::env::temp_dir().join(format!("kor-review-{}-{nanos}{ext}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    let result =
        run_editor(&tmp).and_then(|()| std::fs::read_to_string(&tmp).map_err(|e| e.to_string()));
    let _ = std::fs::remove_file(&tmp);
    result
}

fn run_editor(file: &std::path::Path) -> Result<(), String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| kor_platform::default_editor().to_owned());
    // shortcut: split on whitespace, so an editor path containing spaces fails; `code --wait` works.
    let mut parts = editor.split_whitespace();
    let program = parts.next().ok_or("empty $EDITOR")?;
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(file)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program}: {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_become_rejections() {
        let files = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let rejected = [false, true];
        assert_eq!(marks(&files, &rejected), " · 1 a.rs ✓ · 2 b.rs ✗");
        let r = from_marks(PermissionReply::Allow, &files, &rejected);
        assert_eq!(r.reply, PermissionReply::Allow);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files.get("b.rs"), Some(&FileDecision::Reject));
        let mut r = from_marks(PermissionReply::Allow, &files, &rejected);
        apply_edits(
            &mut r,
            &files,
            &rejected,
            &[Some("new".into()), Some("x".into())],
        );
        assert_eq!(r.files.get("a.rs"), Some(&FileDecision::Edit("new".into())));
        assert_eq!(r.files.get("b.rs"), Some(&FileDecision::Reject));
    }

    #[test]
    fn diff_of_a_new_file_and_reason_text() {
        let f = Proposal {
            path: "n.txt".into(),
            before: None,
            after: Some("hi\n".into()),
        };
        assert!(diff(&f).contains("+ hi"));
        let l = Localizer::new(kor_i18n::Lang::En);
        assert_eq!(destructive(&l, "delete-file"), "Destructive: deletes files");
    }
}
