//! Fallback for tools without an adapter: any `.<tool>/rules/**/*.md` at the project root
//! (front matter as in `activation`) and `.<tool>/mcp.json`. Only the `rules/` subfolder is
//! read, so unrelated dot-folders (`.changeset`, `.husky`, ...) never become instructions.

use super::{Adapter, REGISTRY, mcp, md_rules};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Generic;

/// Dot-folders that are never an AI tool's.
const NOT_TOOLS: &[&str] = &[".git", ".hg", ".svn", ".idea", ".venv", ".cache"];

impl Adapter for Generic {
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let known: Vec<&str> = REGISTRY.iter().flat_map(|a| a.dirs()).copied().collect();
        let mut dirs: Vec<_> = std::fs::read_dir(&o.root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| {
                    n.starts_with('.')
                        && !known.contains(&n.as_ref())
                        && !NOT_TOOLS.contains(&n.as_ref())
                })
            })
            .collect();
        dirs.sort();
        for d in dirs {
            let rules = d.join("rules");
            md_rules(
                c,
                &rules,
                ".md",
                Eco::Other,
                Scope::Project,
                &Applies::Always,
                &[],
            );
            mcp(c, &d.join("mcp.json"), Eco::Other, Scope::Project);
        }
    }
}
