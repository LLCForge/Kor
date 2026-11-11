//! `OpenAI` Codex CLI: `AGENTS.md` hierarchy, `config.toml`, custom prompts.

use std::path::Path;

use kor_core::PermissionMode;
use toml::Table;

use super::{Adapter, commands, mcp_name, plain};
use crate::{
    Applies, Compat, CustomProvider, Eco, Instruction, LoadOptions, Scope, Source, project_dirs,
    read,
};

fn src(scope: Scope, path: &Path) -> Source {
    super::src(Eco::Codex, scope, path)
}

/// Codex CLI plus the universal `AGENTS.md` / `AGENT.md`.
pub(crate) struct Codex;

impl Adapter for Codex {
    fn dirs(&self) -> &'static [&'static str] {
        &[".codex"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        load_user(o, c);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        load_project(o, c);
    }
    fn user_late(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.codex_home {
            // `$CODEX_HOME/prompts/*.md` -> slash commands.
            commands(c, &home.join("prompts"), Eco::Codex, Scope::User, "");
        }
    }
}

/// `AGENTS.override.md` takes the place of `AGENTS.md` in the same directory; `AGENT.md`
/// (singular, used by Amp and others) is read as well.
fn agents_md(c: &mut Compat, dir: &Path, scope: Scope) {
    plain(c, &dir.join("AGENT.md"), Eco::Codex, scope, Applies::Always);
    for name in ["AGENTS.override.md", "AGENTS.md"] {
        let p = dir.join(name);
        if let Some(text) = read(&p, &mut c.warnings) {
            if !text.trim().is_empty() {
                c.instructions.push(Instruction {
                    source: src(scope, &p),
                    applies: Applies::Always,
                    text,
                });
            }
            return;
        }
    }
}

/// Codex approval/sandbox settings → the closest Kor mode.
fn mode(approval: Option<&str>, sandbox: Option<&str>) -> Option<PermissionMode> {
    match (approval, sandbox) {
        (_, Some("read-only")) => Some(PermissionMode::Plan),
        (Some("never"), _) => Some(PermissionMode::Auto),
        (Some(_), _) => Some(PermissionMode::Default),
        (None, _) => None,
    }
}

fn config(c: &mut Compat, path: &Path, scope: Scope) {
    let Some(text) = read(path, &mut c.warnings) else {
        return;
    };
    let t: Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            c.warnings.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    let s = |k: &str| t.get(k).and_then(|v| v.as_str());
    if let Some(m) = mode(s("approval_policy"), s("sandbox_mode")) {
        c.set_mode(m, src(scope, path));
    }
    if let Some(model) = s("model") {
        let provider = s("model_provider").unwrap_or("openai");
        c.set_model(format!("{provider}/{model}"), src(scope, path));
    }
    for name in t
        .get("mcp_servers")
        .and_then(|v| v.as_table())
        .into_iter()
        .flat_map(|m| m.keys())
    {
        mcp_name(c, name, Eco::Codex, scope, path);
    }
    let Some(providers) = t.get("model_providers").and_then(|v| v.as_table()) else {
        return;
    };
    if scope != Scope::User {
        // A project must not redirect a provider (and the user's key) to another host.
        c.warnings.push(format!(
            "{}: [model_providers] in a project config are ignored",
            path.display()
        ));
        return;
    }
    for (id, p) in providers {
        let get = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        let Some(base_url) = get("base_url") else {
            continue;
        };
        c.providers.push(CustomProvider {
            id: id.clone(),
            name: get("name").unwrap_or_else(|| id.clone()),
            base_url,
            env_key: get("env_key"),
            wire_api: get("wire_api").unwrap_or_else(|| "chat".into()),
            source: src(scope, path),
        });
    }
}

pub(crate) fn load_user(o: &LoadOptions, c: &mut Compat) {
    let Some(home) = &o.codex_home else { return };
    agents_md(c, home, Scope::User);
    config(c, &home.join("config.toml"), Scope::User);
}

fn load_project(o: &LoadOptions, c: &mut Compat) {
    for d in project_dirs(&o.root) {
        agents_md(c, &d, Scope::Project);
    }
    config(c, &o.root.join(".codex/config.toml"), Scope::Project);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_mapping() {
        assert_eq!(
            mode(Some("never"), Some("workspace-write")),
            Some(PermissionMode::Auto)
        );
        assert_eq!(
            mode(Some("never"), Some("read-only")),
            Some(PermissionMode::Plan)
        );
        assert_eq!(
            mode(Some("on-request"), None),
            Some(PermissionMode::Default)
        );
        assert_eq!(mode(None, None), None);
    }
}
