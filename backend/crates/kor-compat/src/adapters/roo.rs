//! Roo Code: `.roo/rules/` (always), `.roo/rules-<mode>/` (mode rules), legacy `.roorules` and
//! `.roorules-<mode>`, `.roo/commands/*.md`, `.roo/mcp.json`.
//!
//! Kor has no Roo modes, so mode rules are listed for the model to read when the task fits the
//! mode (`OnRequest`). Kilo Code shares this layout (`kilo.rs`).

use super::{Adapter, commands, dirs_with_prefix, mcp, md_rules, plain};
use crate::{Applies, Compat, Eco, LoadOptions, Scope};

pub(crate) struct Roo;

impl Adapter for Roo {
    fn dirs(&self) -> &'static [&'static str] {
        &[".roo"]
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        family(c, o, ".roo", ".roorules", Eco::Roo, "Roo Code");
    }
}

fn mode_rule(tool: &str, mode: &str) -> Applies {
    Applies::OnRequest {
        description: format!("{tool} `{mode}` mode rules"),
    }
}

/// The Cline-family layout under `dir` with legacy single files named `legacy[-<mode>]`.
pub(super) fn family(
    c: &mut Compat,
    o: &LoadOptions,
    dir: &str,
    legacy: &str,
    eco: Eco,
    tool: &str,
) {
    let base = o.root.join(dir);
    let scope = Scope::Project;
    plain(c, &o.root.join(legacy), eco, scope, Applies::Always);
    md_rules(
        c,
        &base.join("rules"),
        ".md",
        eco,
        scope,
        &Applies::Always,
        &[],
    );
    for (path, mode) in dirs_with_prefix(&base, "rules-") {
        md_rules(c, &path, ".md", eco, scope, &mode_rule(tool, &mode), &[]);
    }
    let mut legacy_modes: Vec<_> = std::fs::read_dir(&o.root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let mode = name.strip_prefix(legacy)?.strip_prefix('-')?.to_owned();
            Some((e.path(), mode))
        })
        .collect();
    legacy_modes.sort();
    for (path, mode) in legacy_modes {
        plain(c, &path, eco, scope, mode_rule(tool, &mode));
    }
    commands(c, &base.join("commands"), eco, scope, "");
    mcp(c, &base.join("mcp.json"), eco, scope);
}
