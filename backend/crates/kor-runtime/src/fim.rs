//! §4.2: fill-in-the-middle with Mistral Codestral (`kor fim`, TUI `/fim`, desktop
//! `fim.complete`): the code that belongs at a cursor position, from the text before and after
//! it. The file must be inside the project; nothing is written unless asked.

use std::path::{Path, PathBuf};

use kor_i18n::{Localizer, args};

/// Text sent around the cursor (each side), so a large file does not go whole.
const CONTEXT_CHARS: usize = 24_000;
const MAX_TOKENS: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub path: PathBuf,
    /// Byte offset of the cursor in the file text.
    pub offset: usize,
    pub text: String,
}

/// The file `file` (relative to `root` or absolute inside it) and the byte offset of 1-based
/// `line`, `col` (end of the line when `col` is `None`).
///
/// # Errors
/// A file outside the project or unreadable, or a position past its end.
pub fn locate(
    l: &Localizer,
    root: &Path,
    file: &str,
    line: usize,
    col: Option<usize>,
) -> Result<Cursor, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = root
        .join(file)
        .canonicalize()
        .map_err(|e| format!("{file}: {e}"))?;
    // SECURITY: the text goes to a remote provider; only files of this project.
    if !path.starts_with(&root) {
        return Err(l.t_args("fim-outside", &args!("file" => file.to_owned())));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
    let past = || {
        l.t_args(
            "fim-position",
            &args!("line" => line, "file" => file.to_owned()),
        )
    };
    let start = if line <= 1 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 2)
            .map(|(i, _)| i + 1)
            .ok_or_else(past)?
    };
    if line == 0 || start > text.len() {
        return Err(past());
    }
    let rest = &text[start..];
    let line_text = rest
        .split('\n')
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r');
    let offset = match col {
        None => start + line_text.len(),
        Some(c) => {
            start
                + line_text
                    .char_indices()
                    .nth(c.saturating_sub(1))
                    .map_or(line_text.len(), |(i, _)| i)
        }
    };
    Ok(Cursor { path, offset, text })
}

/// What Codestral puts at the cursor.
///
/// # Errors
/// Privacy mode (Codestral is remote), no Codestral key, or a provider error.
pub async fn complete(l: &Localizer, c: &Cursor) -> Result<String, String> {
    if crate::privacy_mode() {
        return Err(l.t("fim-privacy"));
    }
    let mut notes = Vec::new();
    let reg = crate::registry(l, &mut notes)?;
    let m = reg.get("codestral").ok_or_else(|| l.t("fim-no-key"))?;
    let key = crate::key_for(m)
        .map_err(|e| e.to_string())?
        .map(|(k, _)| k)
        .ok_or_else(|| l.t("fim-no-key"))?;
    let client = kor_providers::extras::Codestral::new(m, Some(key)).map_err(|e| e.message)?;
    let (prefix, suffix) = around(&c.text, c.offset);
    client
        .fim(
            kor_providers::extras::CODESTRAL_MODEL,
            prefix,
            suffix,
            MAX_TOKENS,
        )
        .await
        .map_err(|e| e.message)
}

/// At most [`CONTEXT_CHARS`] on each side of `offset`, cut at character boundaries.
fn around(text: &str, offset: usize) -> (&str, &str) {
    let (before, after) = text.split_at(offset.min(text.len()));
    let from = before
        .char_indices()
        .rev()
        .nth(CONTEXT_CHARS)
        .map_or(0, |(i, _)| i);
    let to = after
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(after.len(), |(i, _)| i);
    (&before[from..], &after[..to])
}

/// Write `fill` into the file at the cursor.
///
/// # Errors
/// When the file cannot be written.
pub fn insert(c: &Cursor, fill: &str) -> std::io::Result<()> {
    let mut text = c.text.clone();
    text.insert_str(c.offset, fill);
    std::fs::write(&c.path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_is_found_and_stays_inside_the_project() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn add(a: i32, b: i32) -> i32 {\n\n}\n",
        )
        .unwrap();
        let c = locate(&l, dir.path(), "a.rs", 2, None).unwrap();
        let (p, s) = around(&c.text, c.offset);
        assert_eq!((p, s), ("fn add(a: i32, b: i32) -> i32 {\n", "\n}\n"));
        let c1 = locate(&l, dir.path(), "a.rs", 1, Some(4)).unwrap();
        assert_eq!(&c1.text[..c1.offset], "fn ");
        assert!(
            locate(&l, dir.path(), "a.rs", 9, None).is_err(),
            "past the end"
        );
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.rs"), "x").unwrap();
        let rel = format!(
            "../{}/x.rs",
            outside.path().file_name().unwrap().to_string_lossy()
        );
        assert!(
            locate(&l, dir.path(), &rel, 1, None).is_err(),
            "outside the project"
        );
        insert(&c, "    a + b").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
        );
    }
}
