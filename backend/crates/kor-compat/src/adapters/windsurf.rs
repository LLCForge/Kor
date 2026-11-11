//! Windsurf / Devin Desktop: `.windsurf/rules/*.md` (`trigger:` front matter), legacy
//! `.windsurfrules`, `.windsurf/workflows/*.md` (slash commands). Global:
//! `~/.codeium/windsurf/memories/global_rules.md` (always) and the other memory files there,
//! which are suggestions, not rules: listed for the model to read when relevant.

use super::{Adapter, commands, mcp, md_rules, plain};
use crate::{Applies, Compat, Eco, LoadOptions, Scope, files};

pub(crate) struct Windsurf;

impl Adapter for Windsurf {
    fn dirs(&self) -> &'static [&'static str] {
        &[".windsurf"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        let Some(home) = &o.home else { return };
        let base = home.join(".codeium/windsurf");
        let memories = base.join("memories");
        let global = memories.join("global_rules.md");
        plain(c, &global, Eco::Windsurf, Scope::User, Applies::Always);
        for p in files(&memories, "md").into_iter().filter(|p| *p != global) {
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            plain(
                c,
                &p,
                Eco::Windsurf,
                Scope::User,
                Applies::OnRequest {
                    description: format!("Windsurf memory `{name}` (a suggestion, not a rule)"),
                },
            );
        }
        mcp(c, &base.join("mcp_config.json"), Eco::Windsurf, Scope::User);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let dir = o.root.join(".windsurf");
        let eco = Eco::Windsurf;
        plain(
            c,
            &o.root.join(".windsurfrules"),
            eco,
            Scope::Project,
            Applies::Always,
        );
        md_rules(
            c,
            &dir.join("rules"),
            ".md",
            eco,
            Scope::Project,
            &Applies::Always,
            &[],
        );
        commands(c, &dir.join("workflows"), eco, Scope::Project, "");
    }
}
