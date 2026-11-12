//! `OpenCode`: `.opencode/agents/*.md` (and `agent/`) sub-agents, `.opencode/commands/*.md` (and
//! `command/`), `opencode.json` (`instructions` files, `mcp` servers). Global: `~/.config/opencode/`
//! (`AGENTS.md`, `opencode.json`, `agents/`, `commands/`).

use std::path::{Path, PathBuf};

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

/// Files under `base` matching an `instructions` glob (`docs/*.md`, `packages/*/AGENTS.md`), in
/// path order. The walk is bounded like every other compat scan and skips build/vendor folders.
fn expand(base: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    fn walk(
        dir: &Path,
        base: &Path,
        g: &globset::GlobMatcher,
        out: &mut Vec<PathBuf>,
        depth: usize,
    ) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(Result::ok).take(crate::MAX_DIR_ENTRIES) {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            let name = e.file_name();
            if t.is_dir()
                && depth < 6
                && !matches!(
                    name.to_str(),
                    Some(".git" | "node_modules" | "target" | "dist" | "build")
                )
            {
                walk(&p, base, g, out, depth + 1);
            } else if t.is_file() && p.strip_prefix(base).is_ok_and(|r| g.is_match(r)) {
                out.push(p);
            }
        }
    }
    let g = globset::GlobBuilder::new(pattern.trim_start_matches("./"))
        .literal_separator(true)
        .build()
        .map_err(|e| e.to_string())?
        .compile_matcher();
    let mut out = Vec::new();
    walk(base, base, &g, &mut out, 0);
    out.sort();
    Ok(out)
}

/// `instructions`: paths and glob patterns, both confined to `base`.
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
            // SECURITY: patterns that climb out of `base` are refused, like literal paths below.
            if Path::new(entry).is_absolute()
                || entry.starts_with(['/', '\\'])
                || entry.split(['/', '\\']).any(|s| s == "..")
            {
                c.warnings.push(format!(
                    "{}: `{entry}` is outside {}, skipped",
                    path.display(),
                    base.display()
                ));
                continue;
            }
            match expand(base, entry) {
                Ok(found) => {
                    for f in found {
                        plain(c, &f, ECO, scope, Applies::Always);
                    }
                }
                Err(e) => c
                    .warnings
                    .push(format!("{}: `{entry}`: {e}", path.display())),
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_globs_expand_inside_the_project_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs/deep")).unwrap();
        std::fs::create_dir_all(root.join("packages/api")).unwrap();
        std::fs::write(root.join("docs/a.md"), "doc a").unwrap();
        std::fs::write(root.join("docs/deep/b.md"), "doc b").unwrap();
        std::fs::write(root.join("packages/api/AGENTS.md"), "api rules").unwrap();
        std::fs::write(
            root.join("opencode.json"),
            r#"{"instructions": ["docs/*.md", "packages/*/AGENTS.md", "../*.md", "/etc/*"]}"#,
        )
        .unwrap();
        let mut c = Compat::default();
        config(&mut c, &root.join("opencode.json"), root, Scope::Project);
        let texts: Vec<&str> = c.instructions.iter().map(|i| i.text.as_str()).collect();
        // `*` stays within one folder: docs/deep/b.md is not matched by `docs/*.md`.
        assert_eq!(texts, ["doc a", "api rules"], "{:?}", c.warnings);
        assert_eq!(
            c.warnings.iter().filter(|w| w.contains("outside")).count(),
            2,
            "{:?}",
            c.warnings
        );
    }
}
