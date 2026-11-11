//! Kor's persistent project memory (spec §6.1) and global knowledge base (spec §31.3).
//!
//! - [`project`]: plain Markdown per project under `<kor home>/projects/<project>/`
//!   (`architecture.md`, `decisions.md`, `patterns.md`, `hotspots.md`, `sessions/`), next to the
//!   auto-memory folder `memory/` that `kor_agent::memory` keeps. Kor only appends or replaces
//!   its own marked sections, so user edits survive.
//! - [`notes`]: optional git-native copy of `decisions.md` (`git notes --ref=kor`).
//! - [`kb`]: `<kor home>/knowledge.sqlite`, one `SQLite` file with FTS5, written by a single
//!   writer thread with group commit, plus int8 vectors for hybrid recall.
//! - [`embed`]: the local CPU embedding model (an on-demand component) and the vector index.
//!
//! Nothing here touches the network. Secrets are masked before anything is stored ([`mask`]).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

// `kor bench --memory`: table 31.3.1 measured (ADR 0062).
pub mod bench;
// Uses, weekly digest, staleness (§27.15, ADR 0063).
pub mod digest;
pub mod embed;
pub mod kb;
pub mod notes;
pub mod project;

pub use embed::{Embed, Local as LocalEmbedder};
pub use kb::{ExportFormat, Hit, Knowledge, Record};

/// The knowledge base at `<home>/knowledge.sqlite`, with the local embedding model when its
/// component is installed (semantic recall), FTS only otherwise.
///
/// # Errors
/// When the database cannot be opened.
pub fn open_knowledge(home: &Path) -> kb::Result<Knowledge> {
    let embed = embed::Local::installed().map(|l| l as std::sync::Arc<dyn Embed>);
    Knowledge::open_with(&knowledge_path(home), embed)
}
pub use project::{Decision, Doc, Hotspot, ProjectMemory, SessionSummary};

const MAX_SANITIZED: usize = 200;

/// Directory name of a project: non-alphanumerics become `-`, then a hash suffix if the result
/// would exceed 200 characters (the auto-memory layout uses the same name).
pub fn sanitize_path(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if sanitized.len() <= MAX_SANITIZED {
        return sanitized;
    }
    format!("{}-{}", &sanitized[..MAX_SANITIZED], fnv(name))
}

/// FNV-1a of `s` as hex (directory names, edit detection; not for security).
pub(crate) fn fnv(s: &str) -> String {
    let mut hash: u64 = 0x811c_9dc5;
    for b in s.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:x}")
}

/// The project key stored with knowledge records: the root path with `/` separators and
/// without the Windows `\\?\` prefix (so the CLI's and the desktop's roots compare equal).
pub fn project_key(root: &Path) -> String {
    let s = root.to_string_lossy().replace('\\', "/");
    match s.strip_prefix("//?/") {
        Some(rest) if !rest.starts_with("UNC/") => rest.to_owned(),
        _ => s,
    }
}

/// `<home>/projects/<sanitized root>/`: the same folder name as the auto-memory layout
/// (`kor_agent::memory::layout`), so `memory/` and these documents sit together.
pub fn project_dir(home: &Path, root: &Path) -> PathBuf {
    home.join("projects")
        .join(sanitize_path(&root.to_string_lossy().replace('\\', "/")))
}

/// `<home>/knowledge.sqlite`.
pub fn knowledge_path(home: &Path) -> PathBuf {
    home.join("knowledge.sqlite")
}

/// ADR-style decisions of the project at `root` (for `DecidedBy` graph edges).
pub fn decisions(home: &Path, root: &Path) -> Vec<Decision> {
    ProjectMemory::new(home, root).decisions()
}

/// Projects linked to `root` with `kor context add` (their graphs can be opened together).
pub fn linked_projects(home: &Path, root: &Path) -> Vec<PathBuf> {
    ProjectMemory::new(home, root).linked_projects()
}

/// `canonicalize` without the Windows `\\?\` prefix, so paths stay readable and comparable.
pub(crate) fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    let c = p.canonicalize()?;
    Ok(match c.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(s) if !s.starts_with("UNC\\") => PathBuf::from(s),
        _ => c,
    })
}

/// Rules with their replacement, and all of them in one set for a single-pass check.
type Generic = (Vec<(Regex, &'static str)>, Option<regex::RegexSet>);

static GENERIC: LazyLock<Generic> = LazyLock::new(|| {
    let rules: Vec<(Regex, &'static str)> = [
        // `password = hunter2`, `API_KEY: "abc..."`: keep the name, hide the value.
        (
            r#"(?i)\b((?:api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|secret|password|passwd|pwd|token)\s*[:=]\s*)(["']?)[^\s"',;]{4,}"#,
            "${1}${2}[masked]",
        ),
        (r"(?i)\b(bearer\s+)[A-Za-z0-9._~+/=-]{8,}", "${1}[masked]"),
        // `https://user:pass@host`
        (r"(://[^/\s:@]+:)[^@\s/]+@", "${1}[masked]@"),
    ]
    .into_iter()
    .filter_map(|(re, rep)| Regex::new(re).ok().map(|r| (r, rep)))
    .collect();
    // One pass decides whether any rule can apply (most text has no credential at all).
    let any = regex::RegexSet::new(rules.iter().map(|(r, _)| r.as_str())).ok();
    (rules, any)
});

/// `text` with secrets masked: the known key shapes of `kor_tools::redact_secrets` plus
/// `name = value` credentials, bearer tokens and passwords in URLs.
pub fn mask(text: &str) -> String {
    let mut out = kor_tools::redact_secrets(text);
    let (rules, any) = &*GENERIC;
    if any.as_ref().is_some_and(|s| !s.is_match(&out)) {
        return out;
    }
    for (re, rep) in rules {
        if re.is_match(&out) {
            out = re.replace_all(&out, *rep).into_owned();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_known_keys_and_generic_credentials() {
        let out = mask(
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789 password=hunter22 \
             Authorization: Bearer abcdefghijkl https://bob:pa55w0rd@example.com/x",
        );
        assert!(!out.contains("ghp_abc"), "{out}");
        assert!(!out.contains("hunter22"), "{out}");
        assert!(!out.contains("abcdefghijkl"), "{out}");
        assert!(!out.contains("pa55w0rd"), "{out}");
        assert!(out.contains("password=[masked]"), "{out}");
        assert!(out.contains("https://bob:[masked]@example.com"), "{out}");
        assert_eq!(mask("fixed the token parser"), "fixed the token parser");
    }

    #[test]
    fn project_dirs_match_the_auto_memory_layout() {
        let d = project_dir(Path::new("/h"), Path::new("C:\\work\\demo"));
        assert_eq!(d, Path::new("/h").join("projects").join("C--work-demo"));
        assert_eq!(project_key(Path::new(r"\\?\C:\work\demo")), "C:/work/demo");
        assert_eq!(project_key(Path::new(r"\\?\UNC\srv\x")), "//?/UNC/srv/x");
        assert_eq!(
            sanitize_path(&"a".repeat(300)).len(),
            MAX_SANITIZED + 1 + fnv(&"a".repeat(300)).len()
        );
    }
}
