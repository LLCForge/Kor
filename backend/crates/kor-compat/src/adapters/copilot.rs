//! GitHub Copilot: `.github/copilot-instructions.md` (always), `.github/instructions/*.instructions.md`
//! (`applyTo` globs; without it the file is attached by hand → `Manual`, or `OnRequest` with a
//! `description`), `.github/prompts/*.prompt.md` (slash commands), `.vscode/mcp.json`.
//! Global: `~/.github/copilot-instructions.md`.

use super::{Adapter, commands, mcp, md_rules, plain};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Copilot;

impl Adapter for Copilot {
    fn dirs(&self) -> &'static [&'static str] {
        &[".github", ".vscode"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        let Some(home) = &o.home else { return };
        let p = home.join(".github/copilot-instructions.md");
        plain(c, &p, Eco::Copilot, Scope::User, Applies::Always);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let gh = o.root.join(".github");
        let (eco, scope) = (Eco::Copilot, Scope::Project);
        plain(
            c,
            &gh.join("copilot-instructions.md"),
            eco,
            scope,
            Applies::Always,
        );
        let dir = gh.join("instructions");
        md_rules(
            c,
            &dir,
            ".instructions.md",
            eco,
            scope,
            &Applies::Manual,
            &[],
        );
        commands(c, &gh.join("prompts"), eco, scope, ".prompt");
        mcp(c, &o.root.join(".vscode/mcp.json"), eco, scope);
    }
}
