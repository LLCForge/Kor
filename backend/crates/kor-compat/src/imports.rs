//! `@path` imports in instruction files (max depth 5).
//!
//! Imported files are appended after the importing file. A project file may only import
//! files inside the project; a user file may import from the home directory. Otherwise a
//! cloned repository could pull `~/.ssh/id_rsa` into the prompt sent to a provider.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 5;

/// `@token`s outside code fences and inline code.
pub(crate) fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut in_code = false;
        let mut prev_space = true;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '`' {
                in_code = !in_code;
            } else if ch == '@' && prev_space && !in_code {
                let tok: String = chars[i + 1..]
                    .iter()
                    .take_while(|c| !c.is_whitespace() && **c != '`')
                    .collect();
                let tok = tok
                    .trim_end_matches(['.', ',', ';', ':', ')', '"', '\''])
                    .to_owned();
                if !tok.is_empty() {
                    i += tok.chars().count();
                    out.push(tok);
                }
            }
            prev_space = ch.is_whitespace() || ch == '(';
            i += 1;
        }
    }
    out
}

pub(crate) fn resolve(tok: &str, base: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let p = if let Some(rest) = tok.strip_prefix("~/") {
        home?.join(rest)
    } else {
        base.join(tok)
    };
    p.canonicalize().ok().filter(|p| p.is_file())
}

/// `text` followed by the contents of its imports (recursively).
pub(crate) fn expand(
    text: &str,
    base: &Path,
    allowed_root: &Path,
    home: Option<&Path>,
    warnings: &mut Vec<String>,
    imported: &mut Vec<PathBuf>,
) -> String {
    let mut out = text.to_owned();
    let mut seen = Vec::new();
    expand_into(
        text,
        base,
        allowed_root,
        home,
        warnings,
        &mut out,
        &mut seen,
        1,
    );
    imported.extend(seen);
    out
}

#[allow(clippy::too_many_arguments)]
fn expand_into(
    text: &str,
    base: &Path,
    allowed_root: &Path,
    home: Option<&Path>,
    warnings: &mut Vec<String>,
    out: &mut String,
    seen: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for tok in tokens(text) {
        let Some(path) = resolve(&tok, base, home) else {
            continue;
        };
        if !path.starts_with(allowed_root) {
            warnings.push(format!(
                "import `@{tok}` points outside {} and was ignored",
                allowed_root.display()
            ));
            continue;
        }
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        let Some(body) = crate::read(&path, warnings) else {
            continue;
        };
        let _ = write!(out, "\n\n<!-- imported: @{tok} -->\n{}", body.trim());
        let dir = path.parent().unwrap_or(base).to_owned();
        expand_into(
            &body,
            &dir,
            allowed_root,
            home,
            warnings,
            out,
            seen,
            depth + 1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_scanning_skips_code_and_emails() {
        let t =
            "See @docs/a.md and (@b.md).\nmail me@x.com\n`@not.md`\n```\n@fenced.md\n```\n@c.md";
        assert_eq!(tokens(t), ["docs/a.md", "b.md", "c.md"]);
    }

    #[test]
    fn imports_expand_recursively_and_stay_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let proj = root.join("proj");
        std::fs::create_dir_all(proj.join("docs")).unwrap();
        std::fs::write(proj.join("docs/a.md"), "A text @b.md").unwrap();
        std::fs::write(proj.join("docs/b.md"), "B text @a.md").unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();

        let mut w = Vec::new();
        let mut imported = Vec::new();
        let out = expand(
            "Main @docs/a.md @../secret.txt",
            &proj,
            &proj,
            None,
            &mut w,
            &mut imported,
        );
        assert_eq!(imported.len(), 2);
        assert!(out.contains("A text") && out.contains("B text"));
        assert_eq!(out.matches("A text").count(), 1, "cycle must not repeat");
        assert!(!out.contains("SECRET"));
        assert_eq!(w.len(), 1);
    }
}
