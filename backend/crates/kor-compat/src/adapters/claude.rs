//! `CLAUDE.md`, `.claude/…`, and Kor-native `KOR.md` / `.kor/…` files.
//! Kor uses the same formats under its own names.

use std::path::{Path, PathBuf};

use kor_core::PermissionMode;
use serde_json::Value;

use super::{Adapter, mcp, md_rules, src};
use crate::frontmatter::split;
use crate::{
    AgentDef, Applies, Command, Compat, Eco, Hook, Instruction, LoadOptions, RuleEntry, Scope,
    Skill, Source, files, imports, project_dirs, read,
};

/// Claude Code and Kor's own files (same formats, different names).
pub(crate) struct Claude;

impl Adapter for Claude {
    fn dirs(&self) -> &'static [&'static str] {
        &[".claude", ".kor"]
    }
    fn user(&self, o: &LoadOptions, c: &mut Compat) {
        load_user_instructions(o, c);
    }
    fn project(&self, o: &LoadOptions, c: &mut Compat) {
        load_project(o, c);
        mcp(c, &o.root.join(".mcp.json"), Eco::Claude, Scope::Project);
    }
    fn user_late(&self, o: &LoadOptions, c: &mut Compat) {
        load_user_extras(o, c);
    }
}

/// Kor's own project rules only (`KOR.md`, `.kor/KOR.md`, `.kor/rules`), for `export`.
pub(crate) fn kor_own(root: &Path, c: &mut Compat) {
    for name in ["KOR.md", ".kor/KOR.md"] {
        let p = root.join(name);
        instruction(c, &p, src(Eco::Kor, Scope::Project, &p), root, None);
    }
    rules(c, &root.join(".kor/rules"), Eco::Kor);
}

/// An instruction file with `@imports` expanded inside `allowed_root`.
fn instruction(
    c: &mut Compat,
    path: &Path,
    source: Source,
    allowed_root: &Path,
    home: Option<&Path>,
) {
    let Some(text) = read(path, &mut c.warnings) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    let base = path.parent().unwrap_or(allowed_root);
    let text = imports::expand(
        &text,
        base,
        allowed_root,
        home,
        &mut c.warnings,
        &mut c.imported,
    );
    c.instructions.push(Instruction {
        source,
        applies: Applies::Always,
        text,
    });
}

/// Claude Code's user folder: `$CLAUDE_CONFIG_DIR` when set (as Claude Code reads it), else `~/.claude`.
fn claude_user_dir(home: &Path) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map_or_else(|| home.join(".claude"), PathBuf::from)
}

fn load_user_instructions(o: &LoadOptions, c: &mut Compat) {
    let Some(home) = &o.home else { return };
    instruction(
        c,
        &home.join(".kor/KOR.md"),
        src(Eco::Kor, Scope::User, &home.join(".kor/KOR.md")),
        home,
        Some(home),
    );
    let p = claude_user_dir(home).join("CLAUDE.md");
    instruction(c, &p, src(Eco::Claude, Scope::User, &p), home, Some(home));
}

fn load_project(o: &LoadOptions, c: &mut Compat) {
    let dirs = project_dirs(&o.root);
    let top = dirs.first().cloned().unwrap_or_else(|| o.root.clone());
    let home = o.home.as_deref();
    for d in &dirs {
        for (name, eco, scope) in [
            ("CLAUDE.md", Eco::Claude, Scope::Project),
            (".claude/CLAUDE.md", Eco::Claude, Scope::Project),
            ("CLAUDE.local.md", Eco::Claude, Scope::Local),
            ("KOR.md", Eco::Kor, Scope::Project),
            (".kor/KOR.md", Eco::Kor, Scope::Project),
        ] {
            let p = d.join(name);
            instruction(c, &p, src(eco, scope, &p), &top, home);
        }
    }
    let root = &o.root;
    for (dir, eco) in [(".kor", Eco::Kor), (".claude", Eco::Claude)] {
        rules(c, &root.join(dir).join("rules"), eco);
    }
    settings(c, &root.join(".claude/settings.json"), Scope::Project);
    settings(c, &root.join(".claude/settings.local.json"), Scope::Local);
    for (dir, eco) in [(".kor", Eco::Kor), (".claude", Eco::Claude)] {
        extras(c, &root.join(dir), eco, Scope::Project);
    }
}

fn load_user_extras(o: &LoadOptions, c: &mut Compat) {
    let Some(home) = &o.home else { return };
    let claude = claude_user_dir(home);
    settings(c, &claude.join("settings.json"), Scope::User);
    extras(c, &home.join(".kor"), Eco::Kor, Scope::User);
    extras(c, &claude, Eco::Claude, Scope::User);
}

/// An installed plugin: its `commands/`, `agents/`, `skills/` (also under `.claude/` and `.agents/`).
/// Plugin hooks are not loaded: installing a plugin must not run its code.
pub(crate) fn plugin(c: &mut Compat, dir: &Path) {
    for base in [dir.to_owned(), dir.join(".claude"), dir.join(".agents")] {
        extras(c, &base, Eco::Kor, Scope::User);
    }
}

/// `rules/*.md`: front matter `paths`/`globs` makes a rule conditional; Kor's `activation`
/// (`always`, `on-request`, `manual`) is honoured too.
fn rules(c: &mut Compat, dir: &Path, eco: Eco) {
    md_rules(c, dir, ".md", eco, Scope::Project, &Applies::Always, &[]);
}

/// `commands/`, `agents/`, `skills/` under a `.kor` or `.claude` directory.
fn extras(c: &mut Compat, base: &Path, eco: Eco, scope: Scope) {
    let cmd_dir = base.join("commands");
    for p in files(&cmd_dir, "md") {
        if let Some(cmd) = command(c, &cmd_dir, &p, src(eco, scope, &p)) {
            c.add_command(cmd);
        }
    }
    agent_files(c, &base.join("agents"), eco, scope);
    if eco == Eco::Kor {
        crate::templates::load_dir(c, &base.join("templates"), eco, scope);
    }
    let Ok(rd) = std::fs::read_dir(base.join("skills")) else {
        return;
    };
    let mut dirs: Vec<_> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let f = d.join("SKILL.md");
        let Some(text) = read(&f, &mut c.warnings) else {
            continue;
        };
        let (front, _) = split(&text);
        let name = front.get("name").map_or_else(|| stem(&d), str::to_owned);
        let Some(description) = front.get("description") else {
            c.warnings.push(format!(
                "{}: SKILL.md has no description, skipped",
                f.display()
            ));
            continue;
        };
        c.add_skill(Skill {
            name,
            description: description.to_owned(),
            dir: d.clone(),
            source: src(eco, scope, &f),
        });
    }
}

/// Sub-agent definitions: Markdown with `name`, `description`, `tools`, `model` front matter.
pub(crate) fn agent_files(c: &mut Compat, dir: &Path, eco: Eco, scope: Scope) {
    for p in files(dir, "md") {
        let Some(text) = read(&p, &mut c.warnings) else {
            continue;
        };
        let (front, body) = split(&text);
        let name = front.get("name").map_or_else(|| stem(&p), str::to_owned);
        c.add_agent(AgentDef {
            name,
            description: front.get("description").unwrap_or_default().to_owned(),
            tools: front.list("tools"),
            model: front.get("model").map(str::to_owned),
            prompt: body.trim().to_owned(),
            source: src(eco, scope, &p),
        });
    }
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A Markdown command file; subdirectories become a `dir:` namespace.
pub(crate) fn command(c: &mut Compat, dir: &Path, p: &Path, source: Source) -> Option<Command> {
    let text = read(p, &mut c.warnings)?;
    let (front, body) = split(&text);
    let rel = p.strip_prefix(dir).ok()?.with_extension("");
    let name = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":");
    Some(Command {
        name,
        description: front.get("description").map(str::to_owned),
        argument_hint: front
            .get("argument-hint")
            .or_else(|| front.get("args"))
            .map(str::to_owned),
        allowed_tools: front.list("allowed-tools"),
        body: body.trim().to_owned(),
        source,
    })
}

fn mode_from_claude(s: &str) -> Option<PermissionMode> {
    Some(match s {
        "default" => PermissionMode::Default,
        "plan" => PermissionMode::Plan,
        "acceptEdits" | "accept-edits" => PermissionMode::AcceptEdits,
        "dontAsk" | "dont-ask" => PermissionMode::DontAsk,
        // Another tool's "never ask" must not silently disable Kor's prompts: map it to
        // `auto`, which still needs an OS sandbox and still asks for dangerous commands,
        // desktop actions and elevation. Kor's own `--mode bypass-permissions` is unchanged.
        "bypassPermissions" | "bypass-permissions" | "auto" => PermissionMode::Auto,
        _ => return None,
    })
}

/// `settings.json`: permissions, env, hooks, model.
fn settings(c: &mut Compat, path: &Path, scope: Scope) {
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
    let s = || src(Eco::Claude, scope, path);
    let list = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let perms = &v["permissions"];
    for r in list(&perms["allow"]) {
        c.add_allow(r, s());
    }
    for r in list(&perms["ask"]) {
        c.permissions.ask.push(RuleEntry {
            rule: r,
            source: s(),
        });
    }
    for r in list(&perms["deny"]) {
        c.permissions.deny.push(RuleEntry {
            rule: r,
            source: s(),
        });
    }
    if let Some(m) = perms["defaultMode"].as_str() {
        match mode_from_claude(m) {
            Some(mode) => c.set_mode(mode, s()),
            None => c
                .warnings
                .push(format!("{}: unknown defaultMode `{m}`", path.display())),
        }
    }
    if let Some(env) = v["env"].as_object() {
        for (k, val) in env {
            if let Some(val) = val.as_str() {
                c.add_env(k.clone(), val.to_owned(), s());
            }
        }
    }
    if let Some(m) = v["model"].as_str() {
        c.set_model(m.to_owned(), s());
    }
    let Some(hooks) = v["hooks"].as_object() else {
        return;
    };
    for (event, groups) in hooks {
        for g in groups.as_array().into_iter().flatten() {
            let matcher = g["matcher"]
                .as_str()
                .filter(|m| !m.is_empty() && *m != "*")
                .map(str::to_owned);
            for h in g["hooks"].as_array().into_iter().flatten() {
                if h["type"].as_str().unwrap_or("command") != "command" {
                    c.warnings.push(format!(
                        "{}: only `command` hooks are supported",
                        path.display()
                    ));
                    continue;
                }
                let Some(command) = h["command"].as_str() else {
                    continue;
                };
                c.add_hook(Hook {
                    event: event.clone(),
                    matcher: matcher.clone(),
                    command: command.to_owned(),
                    timeout_secs: h["timeout"].as_u64(),
                    source: s(),
                    kor: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn claude_bypass_maps_to_auto() {
        assert_eq!(
            mode_from_claude("bypassPermissions"),
            Some(PermissionMode::Auto)
        );
        assert_eq!(
            mode_from_claude("acceptEdits"),
            Some(PermissionMode::AcceptEdits)
        );
    }
}
