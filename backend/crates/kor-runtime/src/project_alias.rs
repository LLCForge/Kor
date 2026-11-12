//! §6.5: in privacy mode, surfaces the operating system shows outside Kor's window (desktop
//! notifications, the lock screen, the taskbar jump list) name a project by a stable alias
//! (`proje-1a2b`) instead of its folder name or path. Kor's own windows keep the real names.

use std::path::Path;

/// Stable alias of a project root: the same folder always gets the same one.
pub fn alias(root: &Path) -> String {
    use sha2::{Digest as _, Sha256};
    let key = root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    let h = Sha256::digest(key.as_bytes());
    format!("proje-{:02x}{:02x}", h[0], h[1])
}

/// The label for `root` on an OS surface: its folder name, or its alias in privacy mode.
pub fn label(root: &Path) -> String {
    if crate::privacy_mode() {
        alias(root)
    } else {
        root.file_name().map_or_else(
            || root.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    }
}

/// `text` for an OS surface: in privacy mode every path of `roots` and every folder name of
/// them (as a whole word) is replaced by the project's alias; unchanged otherwise.
pub fn mask(text: &str, roots: &[&Path]) -> String {
    if crate::privacy_mode() {
        mask_always(text, roots)
    } else {
        text.to_owned()
    }
}

fn mask_always(text: &str, roots: &[&Path]) -> String {
    let mut out = text.to_owned();
    for root in roots {
        let a = alias(root);
        let full = root
            .to_string_lossy()
            .trim_end_matches(['/', '\\'])
            .to_owned();
        if full.is_empty() {
            continue;
        }
        // Both separators: messages may carry either form of the same path.
        for form in [
            full.clone(),
            full.replace('\\', "/"),
            full.replace('/', "\\"),
        ] {
            out = out.replace(&form, &a);
        }
        if let Some(name) = root.file_name().map(|n| n.to_string_lossy().into_owned())
            && name.chars().count() >= 3
        {
            out = replace_word(&out, &name, &a);
        }
    }
    out
}

/// Replace `word` where it is not part of a longer name (letters, digits, `_`, `-`, `.` around).
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let part =
        |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(word) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + word.len()..].chars().next();
        out.push_str(&rest[..i]);
        if part(before) || part(after) {
            out.push_str(word);
        } else {
            out.push_str(with);
        }
        rest = &rest[i + word.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths_become_a_stable_alias() {
        let root = Path::new("/home/u/work/acme-shop");
        let a = alias(root);
        assert!(a.starts_with("proje-") && a.len() == 10, "{a}");
        assert_eq!(a, alias(Path::new("/home/u/work/acme-shop/")), "stable");
        assert_ne!(a, alias(Path::new("/home/u/work/other")));
        let text = "Edit /home/u/work/acme-shop/src/main.rs in acme-shop (not acme-shop-v2)";
        let m = mask_always(text, &[root]);
        assert_eq!(m, format!("Edit {a}/src/main.rs in {a} (not acme-shop-v2)"));
        assert_eq!(
            mask_always("C:\\Work\\Shop\\a.rs", &[Path::new("C:\\Work\\Shop")]),
            format!("{}\\a.rs", alias(Path::new("C:\\Work\\Shop")))
        );
        // Outside privacy mode nothing changes.
        assert_eq!(mask(text, &[root]), text);
        assert_eq!(label(root), "acme-shop");
    }
}
