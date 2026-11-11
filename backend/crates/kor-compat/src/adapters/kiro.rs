//! Kiro: `.kiro/steering/*.md` (`inclusion: always | fileMatch | manual`, `fileMatchPattern`),
//! `.kiro/specs/<name>/*.md` (requirements, design, tasks: listed for the model to read when the
//! task touches that spec), `.kiro/settings/mcp.json`. Global: `~/.kiro/steering/`,
//! `~/.kiro/settings/mcp.json`.

use std::path::Path;

use super::{Adapter, dirs_with_prefix, mcp, md_rules};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Kiro;

impl Adapter for Kiro {
    fn dirs(&self) -> &'static [&'static str] {
        &[".kiro"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.home {
            load(c, &home.join(".kiro"), Scope::User);
        }
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let dir = o.root.join(".kiro");
        load(c, &dir, Scope::Project);
        for (spec, name) in dirs_with_prefix(&dir.join("specs"), "") {
            let applies = Applies::OnRequest {
                description: format!("Kiro spec `{name}`"),
            };
            md_rules(c, &spec, ".md", Eco::Kiro, Scope::Project, &applies, &[]);
        }
    }
}

fn load(c: &mut Compat, dir: &Path, scope: Scope) {
    md_rules(
        c,
        &dir.join("steering"),
        ".md",
        Eco::Kiro,
        scope,
        &Applies::Always,
        &[],
    );
    mcp(c, &dir.join("settings/mcp.json"), Eco::Kiro, scope);
}
