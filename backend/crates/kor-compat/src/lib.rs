//! Ecosystem compatibility: read other AI coding tools' files (Claude Code, Codex, Cursor,
//! Windsurf, Cline, Roo, Kilo, Copilot, Gemini, Aider, `OpenCode`, Continue, Kiro, Devin) and Kor's
//! own, in place and unchanged, into one model. Nothing here writes to another tool's files;
//! `export` writes only under `.kor/export/`. Each tool is one file in `adapters/`.
//!
//! Trust: files of an untrusted project may add instructions, commands and skills, and may
//! restrict permissions, but may not grant permissions, run hooks or set environment.

mod adapters;
mod conflicts;
pub mod export;
pub mod frontmatter;
mod imports;
pub mod template;
pub mod templates;
pub mod trust;

use std::path::{Path, PathBuf};

use kor_core::PermissionMode;
use serde::Serialize;

/// Files larger than this are skipped with a warning.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// At most this many entries are read from one directory.
const MAX_DIR_ENTRIES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Eco {
    Kor,
    Claude,
    Codex,
    Cursor,
    Windsurf,
    Cline,
    Roo,
    Kilo,
    Copilot,
    Gemini,
    Aider,
    OpenCode,
    Continue,
    Kiro,
    Devin,
    /// An unknown `.<tool>/rules/` folder (generic Markdown reader).
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Project,
    /// Personal, uncommitted project files (`CLAUDE.local.md`, `settings.local.json`).
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub eco: Eco,
    pub scope: Scope,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Applies {
    Always,
    /// Attach when the agent touches a file matching one of these globs.
    Globs {
        globs: Vec<String>,
    },
    /// Listed by description; the model reads it when relevant.
    OnRequest {
        description: String,
    },
    /// Only when the user names it (Cursor "Manual", Windsurf/Kiro `manual`); never automatic.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Instruction {
    pub source: Source,
    pub applies: Applies,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleEntry {
    pub rule: String,
    pub source: Source,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Permissions {
    pub allow: Vec<RuleEntry>,
    pub ask: Vec<RuleEntry>,
    pub deny: Vec<RuleEntry>,
    pub mode: Option<(PermissionMode, Source)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hook {
    /// Hook event name (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`, ...).
    pub event: String,
    /// Tool-name pattern for tool events; `None` matches all.
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_secs: Option<u64>,
    pub source: Source,
    /// Kor `hooks.toml` extras (§28.12): conditions and non-command actions; `None` for a plain
    /// command hook (every Claude Code hook).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kor: Option<KorHook>,
}

/// Conditions and actions only Kor's own `hooks.toml` has (§28.12).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct KorHook {
    /// Only when the tool's file matches one of these globs (project-relative).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Only on a git branch matching this glob.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Only when the session's model (`provider/model`) matches this glob.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Call this MCP tool (`server/tool`) instead of a command; `args` is its JSON input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    /// Show this text to the user instead of running anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify: Option<String>,
    /// Kept in the file but not run.
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Command {
    /// Invoked as `/name`; subdirectories become `dir:name`.
    pub name: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub allowed_tools: Vec<String>,
    pub body: String,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub prompt: String,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
    pub source: Source,
}

/// An MCP server another tool defines. Listed only; starting servers is `kor_agent::mcp_tools`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpServer {
    pub name: String,
    pub source: Source,
}

/// An OpenAI-compatible provider declared in `~/.codex/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub env_key: Option<String>,
    /// `chat` or `responses`.
    pub wire_api: String,
    pub source: Source,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Compat {
    pub trusted: bool,
    pub instructions: Vec<Instruction>,
    pub permissions: Permissions,
    pub hooks: Vec<Hook>,
    pub commands: Vec<Command>,
    /// Task templates from files (spec 15.11); built-ins are added by [`templates::all`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<templates::Template>,
    pub agents: Vec<AgentDef>,
    pub skills: Vec<Skill>,
    pub providers: Vec<CustomProvider>,
    pub mcp_servers: Vec<McpServer>,
    pub env: Vec<(String, String, Source)>,
    pub model: Option<(String, Source)>,
    /// Ignore files at the project root (`.korignore`, `.cursorignore`, `.aiderignore`).
    pub ignore_files: Vec<PathBuf>,
    /// What an untrusted project tried to configure and Kor ignored.
    pub untrusted_skipped: Vec<String>,
    pub warnings: Vec<String>,
    /// Files pulled in through `@imports`; not repeated as standalone instructions.
    #[serde(skip)]
    #[doc(hidden)]
    pub imported: Vec<PathBuf>,
}

/// Precedence of a setting's source: local > project > user; Claude over Codex at equal scope.
fn rank(s: &Source) -> (u8, u8) {
    let scope = match s.scope {
        Scope::User => 0,
        Scope::Project => 1,
        Scope::Local => 2,
    };
    (scope, u8::from(s.eco != Eco::Codex))
}

#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// Canonical working directory (the project root for tools).
    pub root: PathBuf,
    pub home: Option<PathBuf>,
    /// `$CODEX_HOME`, defaulting to `~/.codex`.
    pub codex_home: Option<PathBuf>,
    pub trusted: bool,
}

/// Read a text file, bounded. Missing files are `None`; oversized ones warn.
pub(crate) fn read(path: &Path, warnings: &mut Vec<String>) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_FILE_BYTES {
        warnings.push(format!(
            "{}: larger than {MAX_FILE_BYTES} bytes, skipped",
            path.display()
        ));
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

/// Files with extension `ext` under `dir` (recursive), sorted, bounded.
pub(crate) fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(Result::ok).take(MAX_DIR_ENTRIES) {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && depth < 4 {
                walk(&p, ext, out, depth + 1);
            } else if t.is_file() && p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, ext, &mut out, 0);
    out.sort();
    out.truncate(MAX_DIR_ENTRIES);
    out
}

/// Project directories from the repository root (nearest ancestor with `.git`) down to `root`.
/// Without a repository, only `root`.
pub(crate) fn project_dirs(root: &Path) -> Vec<PathBuf> {
    let Some(repo) = root.ancestors().find(|d| d.join(".git").exists()) else {
        return vec![root.to_owned()];
    };
    let mut dirs: Vec<PathBuf> = root
        .ancestors()
        .take_while(|d| d.starts_with(repo))
        .map(Path::to_owned)
        .collect();
    dirs.reverse();
    dirs
}

impl Compat {
    fn grant(&mut self, scope: Scope, what: &str) -> bool {
        if scope == Scope::User || self.trusted {
            true
        } else {
            self.untrusted_skipped.push(what.to_owned());
            false
        }
    }

    /// Add a permission-widening setting only if allowed by trust.
    pub(crate) fn add_allow(&mut self, rule: String, source: Source) {
        if self.grant(
            source.scope,
            &format!("allow `{rule}` from {}", source.path.display()),
        ) {
            self.permissions.allow.push(RuleEntry { rule, source });
        }
    }

    pub(crate) fn set_mode(&mut self, mode: PermissionMode, source: Source) {
        let widening = mode.widens();
        let outranked = self
            .permissions
            .mode
            .as_ref()
            .is_some_and(|(_, s)| rank(s) > rank(&source));
        if outranked {
            return;
        }
        if !widening
            || self.grant(
                source.scope,
                &format!("mode `{mode}` from {}", source.path.display()),
            )
        {
            self.permissions.mode = Some((mode, source));
        }
    }

    pub(crate) fn set_model(&mut self, model: String, source: Source) {
        if self
            .model
            .as_ref()
            .is_none_or(|(_, s)| rank(s) <= rank(&source))
        {
            self.model = Some((model, source));
        }
    }

    pub(crate) fn add_hook(&mut self, hook: Hook) {
        if self.grant(
            hook.source.scope,
            &format!("{} hook from {}", hook.event, hook.source.path.display()),
        ) {
            self.hooks.push(hook);
        }
    }

    pub(crate) fn add_env(&mut self, key: String, value: String, source: Source) {
        if self.grant(
            source.scope,
            &format!("env `{key}` from {}", source.path.display()),
        ) {
            self.env.push((key, value, source));
        }
    }

    /// Add a command unless a higher-priority one with the same name exists.
    pub(crate) fn add_command(&mut self, c: Command) {
        if !self.commands.iter().any(|x| x.name == c.name) {
            self.commands.push(c);
        }
    }

    pub(crate) fn add_skill(&mut self, s: Skill) {
        if !self.skills.iter().any(|x| x.name == s.name) {
            self.skills.push(s);
        }
    }

    pub(crate) fn add_agent(&mut self, a: AgentDef) {
        if !self.agents.iter().any(|x| x.name == a.name) {
            self.agents.push(a);
        }
    }

    /// Instructions in the merge order of spec 3.2 (stable within a tier).
    fn order_instructions(&mut self) {
        self.instructions.sort_by_key(tier);
    }

    /// Instructions without duplicates (same text from two files, e.g. a CLAUDE.md that
    /// imports AGENTS.md).
    fn dedup_instructions(&mut self) {
        let imported: Vec<PathBuf> = self
            .imported
            .iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        let mut seen = std::collections::HashSet::new();
        self.instructions.retain(|i| {
            let was_imported = i
                .source
                .path
                .canonicalize()
                .is_ok_and(|p| imported.contains(&p));
            !was_imported && seen.insert(i.text.trim().to_owned())
        });
    }
}

/// Read everything for a project.
pub fn load(opts: &LoadOptions) -> Compat {
    let mut c = Compat {
        trusted: opts.trusted,
        ..Compat::default()
    };
    // Order matters: instructions run general → specific; for commands, skills and
    // agents the first definition of a name wins, so project-level readers come first.
    for a in adapters::REGISTRY {
        a.user(opts, &mut c);
    }
    for a in adapters::REGISTRY {
        a.project(opts, &mut c);
    }
    for a in adapters::REGISTRY {
        a.user_late(opts, &mut c);
    }
    nested_rules(&opts.root, &mut c);

    for name in [
        ".korignore",
        ".cursorignore",
        ".cursorindexingignore",
        ".aiderignore",
    ] {
        let p = opts.root.join(name);
        if p.is_file() {
            c.ignore_files.push(p);
        }
    }
    c.order_instructions();
    c.dedup_instructions();
    c
}

/// Rule files in subdirectories of the root (`pkg/AGENTS.md`, `pkg/CLAUDE.md`, `pkg/KOR.md`),
/// scoped to their directory: attached the first time the agent touches a file below it, never
/// inlined. Monorepo packages get their own rules this way (spec §15.15). Hidden, `node_modules`
/// and `target` directories are skipped; depth and directory count are bounded.
fn nested_rules(root: &Path, c: &mut Compat) {
    fn walk(root: &Path, dir: &Path, depth: usize, budget: &mut usize, c: &mut Compat) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut subdirs: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .take(MAX_DIR_ENTRIES)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    !n.starts_with('.') && n != "node_modules" && n != "target"
                })
            })
            .collect();
        subdirs.sort();
        for d in subdirs {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let rel = d
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            for (name, eco) in [
                ("AGENTS.md", Eco::Codex),
                ("CLAUDE.md", Eco::Claude),
                ("KOR.md", Eco::Kor),
            ] {
                let p = d.join(name);
                if let Some(text) = read(&p, &mut c.warnings)
                    && !text.trim().is_empty()
                {
                    c.instructions.push(Instruction {
                        source: Source {
                            eco,
                            scope: Scope::Project,
                            path: p,
                        },
                        applies: Applies::Globs {
                            globs: vec![format!("{rel}/**")],
                        },
                        text,
                    });
                }
            }
            if depth < 4 {
                walk(root, &d, depth + 1, budget, c);
            }
        }
    }
    let mut budget = 2_000;
    walk(root, root, 1, &mut budget, c);
}

/// Merge tier (spec 3.2): global user files → `AGENTS.md`/`AGENT.md` → tool-specific project
/// files → path-scoped (glob) rules → Kor's own `KOR.md` / `.kor/rules`. Later tiers win.
pub(crate) fn tier(i: &Instruction) -> u8 {
    if i.source.scope == Scope::User {
        return 0;
    }
    let name = i
        .source
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_uppercase())
        .unwrap_or_default();
    if matches!(
        name.as_str(),
        "AGENTS.MD" | "AGENTS.OVERRIDE.MD" | "AGENT.MD"
    ) {
        1
    } else if i.source.eco == Eco::Kor {
        4
    } else if matches!(i.applies, Applies::Globs { .. }) {
        3
    } else {
        2
    }
}

pub use conflicts::{Conflict, ConflictKey, conflicts};

/// Add an installed plugin's commands, templates, agents and skills; earlier definitions of a
/// name win.
pub fn add_plugin(c: &mut Compat, dir: &Path) {
    adapters::claude::plugin(c, dir);
}

/// Providers declared in the user's Codex config (`$CODEX_HOME/config.toml`), with warnings.
pub fn codex_user_providers(codex_home: &Path) -> (Vec<CustomProvider>, Vec<String>) {
    let mut c = Compat::default();
    adapters::codex::load_user(
        &LoadOptions {
            root: PathBuf::new(),
            home: None,
            codex_home: Some(codex_home.to_owned()),
            trusted: false,
        },
        &mut c,
    );
    (c.providers, c.warnings)
}

#[cfg(test)]
mod nested_tests {
    use super::*;

    #[test]
    fn package_rule_files_are_scoped_to_their_directory() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for (rel, text) in [
            ("AGENTS.md", "root rules"),
            ("packages/web/AGENTS.md", "web rules"),
            ("packages/api/KOR.md", "api rules"),
            ("packages/web/node_modules/x/CLAUDE.md", "vendored"),
        ] {
            let p = r.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let c = load(&LoadOptions {
            root: r.to_owned(),
            home: None,
            codex_home: None,
            trusted: false,
        });
        let got: Vec<(&str, &Applies)> = c
            .instructions
            .iter()
            .map(|i| (i.text.trim(), &i.applies))
            .collect();
        let globs = |g: &str| Applies::Globs {
            globs: vec![g.to_owned()],
        };
        assert!(got.contains(&("root rules", &Applies::Always)));
        assert!(got.contains(&("web rules", &globs("packages/web/**"))));
        assert!(got.contains(&("api rules", &globs("packages/api/**"))));
        assert!(!got.iter().any(|(t, _)| *t == "vendored"));
    }
}
