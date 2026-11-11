//! `kor compat export --to cursor|claude|agents`: Kor's own rules (`KOR.md`, `.kor/KOR.md`,
//! `.kor/rules/*.md`) rendered in another tool's format under `.kor/export/<target>/`. Never
//! written into another tool's real files: the user reviews and copies them.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Applies, Compat, Instruction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Cursor,
    Claude,
    Agents,
}

impl Target {
    pub const ALL: [&'static str; 3] = ["cursor", "claude", "agents"];

    fn dir(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Claude => "claude",
            Self::Agents => "agents",
        }
    }
}

impl std::str::FromStr for Target {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "cursor" => Ok(Self::Cursor),
            "claude" => Ok(Self::Claude),
            "agents" => Ok(Self::Agents),
            _ => Err(format!("unknown target `{s}`: {}", Self::ALL.join(", "))),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exported {
    /// The output folder (`<root>/.kor/export/<target>`).
    pub dir: PathBuf,
    /// Files written, relative to `dir`.
    pub files: Vec<PathBuf>,
    /// Number of Kor rules read.
    pub rules: usize,
    pub warnings: Vec<String>,
}

/// A safe file stem: ASCII letters, digits, `-`, `_`.
fn slug(p: &Path) -> String {
    let s: String = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_owned();
    if s.is_empty() { "rule".into() } else { s }
}

/// One-line YAML scalar, quoted.
fn q(s: &str) -> String {
    serde_json::to_string(&s.replace(['\r', '\n'], " ")).unwrap_or_default()
}

/// `(relative path, content)` for each output file.
#[allow(clippy::too_many_lines)]
fn render(target: Target, rules: &[Instruction]) -> Vec<(PathBuf, String)> {
    let mut used = std::collections::HashSet::new();
    let mut name = |i: &Instruction| {
        let base = slug(&i.source.path);
        let mut n = base.clone();
        let mut k = 2;
        while !used.insert(n.clone()) {
            n = format!("{base}-{k}");
            k += 1;
        }
        n
    };
    let mut out = Vec::new();
    match target {
        Target::Cursor => {
            for i in rules {
                let (always, globs, desc) = match &i.applies {
                    Applies::Always => (true, String::new(), String::new()),
                    Applies::Globs { globs } => (false, globs.join(", "), String::new()),
                    Applies::OnRequest { description } => (false, String::new(), q(description)),
                    Applies::Manual => (false, String::new(), String::new()),
                };
                let text = format!(
                    "---\ndescription: {desc}\nglobs: {globs}\nalwaysApply: {always}\n---\n{}\n",
                    i.text.trim()
                );
                out.push((
                    PathBuf::from(format!(".cursor/rules/{}.mdc", name(i))),
                    text,
                ));
            }
        }
        Target::Claude => {
            let mut main = String::new();
            for i in rules {
                let n = name(i);
                let body = i.text.trim();
                match &i.applies {
                    // `KOR.md` → `CLAUDE.md`; always-on rules → `.claude/rules/`.
                    Applies::Always if is_main(i) => {
                        let _ = writeln!(main, "{body}\n");
                    }
                    Applies::Always => {
                        out.push((rule_path(&n), format!("{body}\n")));
                    }
                    Applies::Globs { globs } => {
                        let mut list = String::new();
                        for g in globs {
                            let _ = writeln!(list, "  - {}", q(g));
                        }
                        out.push((rule_path(&n), format!("---\npaths:\n{list}---\n{body}\n")));
                    }
                    // Claude reads skills by description when relevant.
                    Applies::OnRequest { description } => out.push((
                        PathBuf::from(format!(".claude/skills/{n}/SKILL.md")),
                        format!(
                            "---\nname: {n}\ndescription: {}\n---\n{body}\n",
                            q(description)
                        ),
                    )),
                    // Used only when asked for: a slash command.
                    Applies::Manual => out.push((
                        PathBuf::from(format!(".claude/commands/{n}.md")),
                        format!("{body}\n"),
                    )),
                }
            }
            if !main.is_empty() {
                out.insert(0, (PathBuf::from("CLAUDE.md"), main));
            }
        }
        Target::Agents => {
            let mut doc = String::new();
            for i in rules {
                let body = i.text.trim();
                let n = name(i);
                match &i.applies {
                    Applies::Always if is_main(i) => {
                        let _ = writeln!(doc, "{body}\n");
                    }
                    Applies::Always => {
                        let _ = writeln!(doc, "## {n}\n\n{body}\n");
                    }
                    Applies::Globs { globs } => {
                        let g = globs.join("`, `");
                        let _ = writeln!(
                            doc,
                            "## {n}\n\nApplies to files matching `{g}`.\n\n{body}\n"
                        );
                    }
                    Applies::OnRequest { description } => {
                        let _ = writeln!(doc, "## {n}\n\nWhen relevant: {description}\n\n{body}\n");
                    }
                    Applies::Manual => {
                        let _ = writeln!(
                            doc,
                            "## {n}\n\nOnly when the user asks for `{n}`.\n\n{body}\n"
                        );
                    }
                }
            }
            if !doc.is_empty() {
                out.push((PathBuf::from("AGENTS.md"), doc));
            }
        }
    }
    out
}

fn rule_path(n: &str) -> PathBuf {
    PathBuf::from(format!(".claude/rules/{n}.md"))
}

fn is_main(i: &Instruction) -> bool {
    i.source
        .path
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("KOR.md"))
}

/// Refuse to write through a symlink: `.kor/export` pointing at `.cursor` would otherwise let
/// an export overwrite another tool's real files.
fn no_symlink(p: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => Err(io::Error::other(format!(
            "{} is a symbolic link; refusing to write through it",
            p.display()
        ))),
        _ => Ok(()),
    }
}

/// Render Kor's rules for `target` into `<root>/.kor/export/<target>/`, replacing that folder.
pub fn export(root: &Path, target: Target) -> io::Result<Exported> {
    let mut c = Compat::default();
    crate::adapters::claude::kor_own(root, &mut c);
    let kor = root.join(".kor");
    let dir = kor.join("export").join(target.dir());
    for p in [&kor, &kor.join("export"), &dir] {
        no_symlink(p)?;
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let mut files = Vec::new();
    for (rel, text) in render(target, &c.instructions) {
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text)?;
        files.push(rel);
    }
    Ok(Exported {
        dir,
        files,
        rules: c.instructions.len(),
        warnings: c.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Eco, LoadOptions, Scope, load};

    fn project() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join(".kor/rules")).unwrap();
        std::fs::write(r.join("KOR.md"), "Main Kor rules.\n").unwrap();
        let rules = [
            ("always.md", "Always on.\n"),
            (
                "rust.md",
                "---\nglobs: src/**/*.rs, *.toml\n---\nRust rule.\n",
            ),
            (
                "db.md",
                "---\ndescription: Database work\nactivation: on-request\n---\nDB rule.\n",
            ),
            (
                "release.md",
                "---\nactivation: manual\n---\nRelease steps.\n",
            ),
        ];
        for (n, t) in rules {
            std::fs::write(r.join(".kor/rules").join(n), t).unwrap();
        }
        t
    }

    /// Exported Cursor rules load back with the same activation; nothing outside `.kor/export`.
    #[test]
    fn cursor_round_trip() {
        let t = project();
        let root = t.path().canonicalize().unwrap();
        let out = export(&root, Target::Cursor).unwrap();
        assert_eq!(out.rules, 5);
        assert!(
            !root.join(".cursor").exists(),
            "never writes the real tool folder"
        );
        let back = load(&LoadOptions {
            root: out.dir.clone(),
            home: None,
            codex_home: None,
            trusted: false,
        });
        let mut got: Vec<_> = back
            .instructions
            .iter()
            .filter(|i| i.source.eco == Eco::Cursor && i.source.scope == Scope::Project)
            .map(|i| (slug(&i.source.path), i.applies.clone()))
            .collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("always".into(), Applies::Always),
                (
                    "db".into(),
                    Applies::OnRequest {
                        description: "Database work".into()
                    }
                ),
                ("kor".into(), Applies::Always),
                ("release".into(), Applies::Manual),
                (
                    "rust".into(),
                    Applies::Globs {
                        globs: vec!["src/**/*.rs".into(), "*.toml".into()]
                    }
                ),
            ]
        );
    }

    #[test]
    fn claude_and_agents_layouts() {
        let t = project();
        let root = t.path();
        let claude = export(root, Target::Claude).unwrap();
        let mut f: Vec<_> = claude
            .files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        f.sort();
        assert_eq!(
            f,
            [
                ".claude/commands/release.md",
                ".claude/rules/always.md",
                ".claude/rules/rust.md",
                ".claude/skills/db/SKILL.md",
                "CLAUDE.md"
            ]
        );
        let rust = std::fs::read_to_string(claude.dir.join(".claude/rules/rust.md")).unwrap();
        assert!(rust.starts_with("---\npaths:\n  - \"src/**/*.rs\"\n"));
        let agents = export(root, Target::Agents).unwrap();
        let doc = std::fs::read_to_string(agents.dir.join("AGENTS.md")).unwrap();
        assert!(doc.starts_with("Main Kor rules."));
        assert!(doc.contains("Applies to files matching `src/**/*.rs`, `*.toml`."));
        assert!(!root.join("CLAUDE.md").exists() && !root.join("AGENTS.md").exists());
        // A second export replaces the folder instead of piling up stale files.
        std::fs::remove_file(root.join(".kor/rules/db.md")).unwrap();
        let again = export(root, Target::Claude).unwrap();
        assert!(!again.dir.join(".claude/skills/db").exists());
    }

    #[test]
    fn targets_parse() {
        assert_eq!("Cursor".parse::<Target>(), Ok(Target::Cursor));
        assert!("windsurf".parse::<Target>().is_err());
    }
}
