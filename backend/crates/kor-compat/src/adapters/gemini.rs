//! Gemini CLI / Antigravity: `GEMINI.md` (repository root → working directory), Markdown under
//! `.gemini/`, `.gemini/commands/**/*.toml` (slash commands: `description`, `prompt`),
//! `mcpServers` in `.gemini/settings.json`. Global: `~/.gemini/GEMINI.md`, `~/.gemini/settings.json`,
//! `~/.gemini/commands/`.

use std::path::Path;

use super::{Adapter, mcp, md_rules, plain, src};
use crate::{Applies, Command, Compat, Eco, LoadOptions, Scope, files, project_dirs, read};

pub(crate) struct Gemini;

impl Adapter for Gemini {
    fn dirs(&self) -> &'static [&'static str] {
        &[".gemini"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        let Some(home) = &o.home else { return };
        let dir = home.join(".gemini");
        plain(
            c,
            &dir.join("GEMINI.md"),
            Eco::Gemini,
            Scope::User,
            Applies::Always,
        );
        mcp(c, &dir.join("settings.json"), Eco::Gemini, Scope::User);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        let scope = Scope::Project;
        for d in project_dirs(&o.root) {
            plain(c, &d.join("GEMINI.md"), Eco::Gemini, scope, Applies::Always);
        }
        let dir = o.root.join(".gemini");
        md_rules(
            c,
            &dir,
            ".md",
            Eco::Gemini,
            scope,
            &Applies::Always,
            &["commands"],
        );
        toml_commands(c, &dir.join("commands"), scope);
        mcp(c, &dir.join("settings.json"), Eco::Gemini, scope);
    }
    fn user_late(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.home {
            toml_commands(c, &home.join(".gemini/commands"), Scope::User);
        }
    }
}

/// `commands/git/commit.toml` → `/git:commit`. `{{args}}` becomes `$ARGUMENTS`.
fn toml_commands(c: &mut Compat, dir: &Path, scope: Scope) {
    for p in files(dir, "toml") {
        let Some(text) = read(&p, &mut c.warnings) else {
            continue;
        };
        let t: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                c.warnings.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        let Some(prompt) = t.get("prompt").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(rel) = p.strip_prefix(dir) else {
            continue;
        };
        let name = rel
            .with_extension("")
            .components()
            .map(|x| x.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":");
        c.add_command(Command {
            name,
            description: t
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            argument_hint: None,
            allowed_tools: Vec::new(),
            body: prompt.replace("{{args}}", "$ARGUMENTS").trim().to_owned(),
            source: src(Eco::Gemini, scope, &p),
        });
    }
}
