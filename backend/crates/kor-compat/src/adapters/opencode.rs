//! `OpenCode`: `.opencode/agents/*.md` (and `agent/`) sub-agents, `.opencode/commands/*.md` (and
//! `command/`), `opencode.json` (`instructions` files, `mcp` servers). Global: `~/.config/opencode/`
//! (`AGENTS.md`, `opencode.json`, `agents/`, `commands/`).

use std::path::Path;

use serde_json::Value;

use super::claude::agent_files;
use super::{Adapter, commands, mcp, plain};
use crate::{Applies, Compat, Eco, LoadOptions, Scope, read};

pub(crate) struct OpenCode;

const ECO: Eco = Eco::OpenCode;

impl Adapter for OpenCode {
    fn dirs(&self) -> &'static [&'static str] {
        &[".opencode"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        let Some(home) = &o.home else { return };
        let dir = home.join(".config/opencode");
        plain(c, &dir.join("AGENTS.md"), ECO, Scope::User, Applies::Always);
        config(c, &dir.join("opencode.json"), &dir, Scope::User);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        config(c, &o.root.join("opencode.json"), &o.root, Scope::Project);
        extras(c, &o.root.join(".opencode"), Scope::Project);
    }
    fn user_late(&self, o: &LoadOptions, c: &mut Compat) {
        if let Some(home) = &o.home {
            extras(c, &home.join(".config/opencode"), Scope::User);
        }
    }
}

fn extras(c: &mut Compat, dir: &Path, scope: Scope) {
    for sub in ["agents", "agent"] {
        agent_files(c, &dir.join(sub), ECO, scope);
    }
    for sub in ["commands", "command"] {
        commands(c, &dir.join(sub), ECO, scope, "");
    }
}

/// `instructions`: literal paths inside `base` (glob patterns are reported, not expanded).
fn config(c: &mut Compat, path: &Path, base: &Path, scope: Scope) {
    mcp(c, path, ECO, scope);
    let Some(text) = read(path, &mut c.warnings) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return; // already reported by `mcp`
    };
    let inside = base.canonicalize().ok();
    for entry in v["instructions"].as_array().into_iter().flatten() {
        let Some(entry) = entry.as_str() else {
            continue;
        };
        if entry.contains(['*', '?', '[']) {
            c.warnings.push(format!(
                "{}: instruction pattern `{entry}` is not expanded; name files directly",
                path.display()
            ));
            continue;
        }
        let Ok(target) = base.join(entry).canonicalize() else {
            continue;
        };
        if inside.as_ref().is_some_and(|i| target.starts_with(i)) {
            plain(c, &target, ECO, scope, Applies::Always);
        } else {
            c.warnings.push(format!(
                "{}: `{entry}` is outside {}, skipped",
                path.display(),
                base.display()
            ));
        }
    }
}
