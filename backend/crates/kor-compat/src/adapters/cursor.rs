//! Cursor: `.cursor/rules/*.mdc` (+ global `~/.cursor/rules/`), legacy `.cursorrules`,
//! `.cursor/commands/*.md`, `.cursor/mcp.json` (+ `~/.cursor/mcp.json`).
//!
//! The four rule types map one to one: Always (`alwaysApply`), Auto Attached (`globs`),
//! Agent Requested (`description` only) → `OnRequest`, Manual (none) → `Manual`.

use super::{Adapter, commands, mcp, md_rules, plain};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Cursor;

impl Adapter for Cursor {
    fn dirs(&self) -> &'static [&'static str] {
        &[".cursor"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        let Some(home) = &o.home else { return };
        let dir = home.join(".cursor");
        md_rules(
            c,
            &dir.join("rules"),
            ".mdc",
            Eco::Cursor,
            Scope::User,
            &Applies::Manual,
            &[],
        );
        mcp(c, &dir.join("mcp.json"), Eco::Cursor, Scope::User);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let dir = o.root.join(".cursor");
        plain(
            c,
            &o.root.join(".cursorrules"),
            Eco::Cursor,
            Scope::Project,
            Applies::Always,
        );
        md_rules(
            c,
            &dir.join("rules"),
            ".mdc",
            Eco::Cursor,
            Scope::Project,
            &Applies::Manual,
            &[],
        );
        commands(c, &dir.join("commands"), Eco::Cursor, Scope::Project, "");
        mcp(c, &dir.join("mcp.json"), Eco::Cursor, Scope::Project);
    }
}
