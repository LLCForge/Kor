//! Continue: `.continue/rules/*.md` (`alwaysApply`, `globs`, `description` front matter),
//! `.continue/mcpServers/*.{yaml,yml,json}` (one server per file). Global: `~/.continue/rules/`.

use std::path::Path;

use super::{Adapter, mcp_name, md_rules};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Continue;

impl Adapter for Continue {
    fn dirs(&self) -> &'static [&'static str] {
        &[".continue"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.home {
            load(c, &home.join(".continue"), Scope::User);
        }
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        load(c, &o.root.join(".continue"), Scope::Project);
    }
}

fn load(c: &mut Compat, dir: &Path, scope: Scope) {
    md_rules(
        c,
        &dir.join("rules"),
        ".md",
        Eco::Continue,
        scope,
        &Applies::Always,
        &[],
    );
    let mut servers: Vec<_> = ["yaml", "yml", "json"]
        .iter()
        .flat_map(|ext| crate::files(&dir.join("mcpServers"), ext))
        .collect();
    servers.sort();
    for p in servers {
        if let Some(name) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) {
            mcp_name(c, &name, Eco::Continue, scope, &p);
        }
    }
}
