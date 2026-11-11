//! Tool registry (spec 3.1). One file per tool: a unit struct implementing [`Adapter`], listed in
//! [`REGISTRY`]. The shared helpers below cover the common shapes (a single Markdown file, a
//! folder of rule files with front matter, a command folder, an MCP JSON file), so most adapters
//! are a handful of paths.

use std::path::Path;

use serde_json::Value;

use crate::frontmatter::{Front, split};
use crate::{
    Applies, Compat, Eco, Instruction, LoadOptions, McpServer, Scope, Source, files, read,
};

pub(crate) mod aider;
pub(crate) mod claude;
pub(crate) mod cline;
pub(crate) mod codex;
pub(crate) mod continue_dev;
pub(crate) mod copilot;
pub(crate) mod cursor;
pub(crate) mod devin;
pub(crate) mod gemini;
pub(crate) mod generic;
pub(crate) mod kilo;
pub(crate) mod kiro;
pub(crate) mod opencode;
pub(crate) mod roo;
pub(crate) mod windsurf;

/// One tool's reader. All methods default to doing nothing.
pub(crate) trait Adapter: Sync {
    /// Dot-directories at the project root this tool owns; the generic reader skips them.
    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }
    /// Global instructions and MCP definitions (read before project files).
    fn user(&self, _o: &LoadOptions, _c: &mut Compat) {}
    /// Project files.
    fn project(&self, _o: &LoadOptions, _c: &mut Compat) {}
    /// Global settings, commands, agents and skills: read after the project's, so a project
    /// definition of the same name wins.
    fn user_late(&self, _o: &LoadOptions, _c: &mut Compat) {}
}

/// Order matters for same-named commands/agents/skills (first wins); instruction order is
/// fixed afterwards by `tier`. Generic stays last.
pub(crate) static REGISTRY: &[&dyn Adapter] = &[
    &claude::Claude,
    &codex::Codex,
    &cursor::Cursor,
    &windsurf::Windsurf,
    &cline::Cline,
    &roo::Roo,
    &kilo::Kilo,
    &copilot::Copilot,
    &gemini::Gemini,
    &aider::Aider,
    &opencode::OpenCode,
    &continue_dev::Continue,
    &kiro::Kiro,
    &devin::Devin,
    &generic::Generic,
];

pub(crate) fn src(eco: Eco, scope: Scope, path: &Path) -> Source {
    Source {
        eco,
        scope,
        path: path.to_owned(),
    }
}

/// A whole file as one instruction (no front matter handling, no imports).
pub(crate) fn plain(c: &mut Compat, path: &Path, eco: Eco, scope: Scope, applies: Applies) {
    if let Some(text) = read(path, &mut c.warnings)
        && !text.trim().is_empty()
    {
        c.instructions.push(Instruction {
            source: src(eco, scope, path),
            applies,
            text,
        });
    }
}

/// Rule files under `dir` whose name ends with `suffix` (recursive, except the `skip`
/// subdirectories). Front matter picks the activation; without it, `fallback`.
pub(crate) fn md_rules(
    c: &mut Compat,
    dir: &Path,
    suffix: &str,
    eco: Eco,
    scope: Scope,
    fallback: &Applies,
    skip: &[&str],
) {
    let ext = suffix.rsplit('.').next().unwrap_or("md");
    for p in files(dir, ext) {
        let skipped = p
            .strip_prefix(dir)
            .ok()
            .and_then(|r| r.components().next())
            .is_some_and(|first| skip.iter().any(|s| first.as_os_str() == *s));
        if skipped || !p.to_string_lossy().ends_with(suffix) {
            continue;
        }
        let Some(text) = read(&p, &mut c.warnings) else {
            continue;
        };
        let (front, body) = split(&text);
        if body.trim().is_empty() {
            continue;
        }
        c.instructions.push(Instruction {
            source: src(eco, scope, &p),
            applies: activation(&front, fallback),
            text: body.trim().to_owned(),
        });
    }
}

/// Front matter of any supported tool → Kor's activation.
///
/// - Cursor / Continue: `alwaysApply`, `globs`, `description` (description only = agent requested).
/// - Claude / Cline: `paths`. Copilot: `applyTo`. Kiro: `inclusion` + `fileMatchPattern`.
/// - Windsurf: `trigger: always_on | glob | model_decision | manual`. Kor: `activation`.
pub(crate) fn activation(front: &Front, fallback: &Applies) -> Applies {
    let mut globs = front.list("globs");
    for k in ["paths", "applyTo", "fileMatchPattern"] {
        globs.extend(front.list(k));
    }
    let description = front.get("description").map(str::to_owned);
    let on_request = |d: Option<String>| {
        d.map_or(Applies::Manual, |description| Applies::OnRequest {
            description,
        })
    };
    let trigger = ["trigger", "inclusion", "activation"]
        .iter()
        .find_map(|k| front.get(k))
        .map(|t| t.to_ascii_lowercase().replace('-', "_"));
    match trigger.as_deref() {
        Some("always_on" | "always") => return Applies::Always,
        Some("manual") => return Applies::Manual,
        Some("model_decision" | "agent_requested" | "on_request" | "agent") => {
            return on_request(description);
        }
        // `glob`, `filematch`, `auto_attached`, `globs`: decided by the globs below.
        _ => {}
    }
    let every_file = !globs.is_empty()
        && globs
            .iter()
            .all(|g| matches!(g.as_str(), "**" | "**/*" | "*"));
    if front.bool("alwaysApply") || every_file {
        return Applies::Always;
    }
    if !globs.is_empty() {
        return Applies::Globs { globs };
    }
    if description.is_some() && (*fallback == Applies::Manual || front.get("alwaysApply").is_some())
    {
        return on_request(description);
    }
    fallback.clone()
}

/// Markdown slash commands under `dir` (`sub/x.md` → `/sub:x`). `strip` removes a name suffix
/// such as Copilot's `.prompt`.
pub(crate) fn commands(c: &mut Compat, dir: &Path, eco: Eco, scope: Scope, strip: &str) {
    for p in files(dir, "md") {
        if let Some(mut cmd) = claude::command(c, dir, &p, src(eco, scope, &p)) {
            if let Some(n) = cmd.name.strip_suffix(strip) {
                cmd.name.truncate(n.len());
            }
            c.add_command(cmd);
        }
    }
}

/// Server names from an MCP JSON file (`mcpServers`, VS Code `servers`, `OpenCode` `mcp`).
/// Only names are kept: commands, URLs and env may carry secrets.
pub(crate) fn mcp(c: &mut Compat, path: &Path, eco: Eco, scope: Scope) {
    let Some(text) = read(path, &mut c.warnings) else {
        return;
    };
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            c.warnings.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    for key in ["mcpServers", "servers", "mcp"] {
        for name in v[key].as_object().into_iter().flat_map(|m| m.keys()) {
            mcp_name(c, name, eco, scope, path);
        }
    }
}

pub(crate) fn mcp_name(c: &mut Compat, name: &str, eco: Eco, scope: Scope, path: &Path) {
    c.mcp_servers.push(McpServer {
        name: name.to_owned(),
        source: src(eco, scope, path),
    });
}

/// Subdirectories of `dir` whose name starts with `prefix`, with the rest of the name; sorted.
pub(crate) fn dirs_with_prefix(dir: &Path, prefix: &str) -> Vec<(std::path::PathBuf, String)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(prefix)?.to_owned();
            (!rest.is_empty()).then(|| (e.path(), rest))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(front: &str, fallback: &Applies) -> Applies {
        activation(&split(&format!("---\n{front}\n---\nbody\n")).0, fallback)
    }

    #[test]
    fn activation_modes_of_every_tool() {
        let always = Applies::Always;
        let manual = Applies::Manual;
        // Cursor's four modes.
        assert_eq!(act("alwaysApply: true", &manual), Applies::Always);
        assert_eq!(
            act("globs: src/**/*.ts, *.tsx\nalwaysApply: false", &manual),
            Applies::Globs {
                globs: vec!["src/**/*.ts".into(), "*.tsx".into()]
            }
        );
        assert_eq!(
            act("description: DB rules\nalwaysApply: false", &manual),
            Applies::OnRequest {
                description: "DB rules".into()
            }
        );
        assert_eq!(act("alwaysApply: false", &manual), Applies::Manual);
        // Windsurf triggers.
        assert_eq!(act("trigger: always_on", &always), Applies::Always);
        assert_eq!(act("trigger: manual", &always), Applies::Manual);
        assert_eq!(
            act("trigger: model_decision\ndescription: d", &always),
            Applies::OnRequest {
                description: "d".into()
            }
        );
        assert_eq!(
            act("trigger: glob\nglobs: *.rs", &always),
            Applies::Globs {
                globs: vec!["*.rs".into()]
            }
        );
        // Kiro, Copilot, Cline.
        assert_eq!(
            act(
                "inclusion: fileMatch\nfileMatchPattern: \"app/**\"",
                &always
            ),
            Applies::Globs {
                globs: vec!["app/**".into()]
            }
        );
        assert_eq!(act("applyTo: \"**\"", &manual), Applies::Always);
        assert_eq!(
            act("paths:\n  - docs/**", &always),
            Applies::Globs {
                globs: vec!["docs/**".into()]
            }
        );
        // A description alone does not demote an always-on rule of a tool without that mode.
        assert_eq!(act("description: x", &always), Applies::Always);
        assert_eq!(act("activation: on-request", &always), Applies::Manual);
    }
}
