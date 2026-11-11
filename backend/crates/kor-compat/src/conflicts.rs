//! `kor compat conflicts` (spec 3.2): the same rule written differently by two tools.
//!
//! "Same rule" (docs/decisions/0003-compat-registry-and-conflicts.md): rules from different tools
//! that either target the same glob set, or have the same file name (`testing.mdc` and
//! `testing.md`), and whose texts differ. The winner is the rule in the higher merge tier; on a
//! tie, the most recently modified file. Nothing is dropped: all of them stay loaded in merge
//! order, so the winner is read last.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::{Applies, Compat, Source, tier};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConflictKey {
    Globs { globs: Vec<String> },
    Name { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub key: ConflictKey,
    /// In merge order (lowest priority first).
    pub rules: Vec<Source>,
    pub winner: PathBuf,
}

/// File name without tool-specific suffixes, lowercased: `Testing.instructions.md` → `testing`.
fn rule_name(p: &std::path::Path) -> Option<String> {
    let name = p.file_name()?.to_string_lossy().to_ascii_lowercase();
    let mut name = name.as_str();
    for suffix in [".mdc", ".md", ".instructions", ".prompt"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    Some(name.to_owned())
}

pub fn conflicts(c: &Compat) -> Vec<Conflict> {
    let mut groups: BTreeMap<String, (ConflictKey, Vec<usize>)> = BTreeMap::new();
    for (n, i) in c.instructions.iter().enumerate() {
        if let Applies::Globs { globs } = &i.applies {
            let mut globs = globs.clone();
            globs.sort();
            globs.dedup();
            let key = format!("g:{}", globs.join("\n"));
            groups
                .entry(key)
                .or_insert_with(|| (ConflictKey::Globs { globs }, Vec::new()))
                .1
                .push(n);
        }
        if let Some(name) = rule_name(&i.source.path) {
            groups
                .entry(format!("n:{name}"))
                .or_insert_with(|| (ConflictKey::Name { name }, Vec::new()))
                .1
                .push(n);
        }
    }
    let mut seen: Vec<Vec<usize>> = Vec::new();
    let mut out = Vec::new();
    for (_, (key, idx)) in groups {
        let rules: Vec<_> = idx.iter().map(|&n| &c.instructions[n]).collect();
        let tools = rules.iter().any(|r| r.source.eco != rules[0].source.eco);
        let texts = rules.iter().any(|r| r.text.trim() != rules[0].text.trim());
        if !tools || !texts || seen.contains(&idx) {
            continue;
        }
        let winner = rules
            .iter()
            .max_by_key(|r| {
                let modified = std::fs::metadata(&r.source.path)
                    .and_then(|m| m.modified())
                    .ok();
                (tier(r), modified)
            })
            .map(|r| r.source.path.clone())
            .unwrap_or_default();
        out.push(Conflict {
            key,
            rules: rules.iter().map(|r| r.source.clone()).collect(),
            winner,
        });
        seen.push(idx);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Eco, Instruction, Scope};

    fn rule(eco: Eco, path: &str, applies: Applies, text: &str) -> Instruction {
        Instruction {
            source: Source {
                eco,
                scope: Scope::Project,
                path: path.into(),
            },
            applies,
            text: text.into(),
        }
    }

    #[test]
    fn same_globs_or_same_name_across_tools_with_different_text() {
        let g = |s: &str| Applies::Globs {
            globs: vec![s.into()],
        };
        let c = Compat {
            instructions: vec![
                rule(
                    Eco::Cursor,
                    ".cursor/rules/testing.mdc",
                    g("**/*.rs"),
                    "use nextest",
                ),
                rule(
                    Eco::Roo,
                    ".roo/rules/rust.md",
                    g("**/*.rs"),
                    "use cargo test",
                ),
                rule(
                    Eco::Copilot,
                    "a/testing.instructions.md",
                    Applies::Always,
                    "tests!",
                ),
                // Same text in two tools is not a conflict.
                rule(
                    Eco::Cursor,
                    ".cursor/rules/style.mdc",
                    Applies::Always,
                    "same",
                ),
                rule(
                    Eco::Windsurf,
                    ".windsurf/rules/style.md",
                    Applies::Always,
                    "same",
                ),
                // Same tool twice is not a conflict either.
                rule(Eco::Kiro, "x/api.md", Applies::Always, "a"),
                rule(Eco::Kiro, "y/api.md", Applies::Always, "b"),
            ],
            ..Compat::default()
        };
        let found = conflicts(&c);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert_eq!(
            found[0].key,
            ConflictKey::Globs {
                globs: vec!["**/*.rs".into()]
            }
        );
        assert_eq!(found[0].rules.len(), 2);
        assert_eq!(
            found[1].key,
            ConflictKey::Name {
                name: "testing".into()
            }
        );
        // Globs rank above an always-on tool rule in the merge order.
        assert_eq!(found[1].winner, PathBuf::from(".cursor/rules/testing.mdc"));
    }
}
