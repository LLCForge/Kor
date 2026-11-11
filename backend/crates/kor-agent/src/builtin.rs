//! Agents shipped inside Kor: specialised subagents (reviewers per language, build-error
//! resolvers, architecture, security, testing, docs…) available through the `Agent` tool without
//! any setup. Each is a Claude Code-style Markdown file under `builtin/agents/`.
//!
//! User and project agents (`.claude/agents`, `.kor/agents`) win over a built-in with the same name.

use std::path::PathBuf;

use kor_compat::frontmatter::split;
use kor_compat::{AgentDef, Eco, Scope, Source};

include!("builtin_list.rs");

/// Every built-in agent, parsed.
pub fn agents() -> Vec<AgentDef> {
    AGENTS
        .iter()
        .map(|(file, text)| {
            let (front, body) = split(text);
            AgentDef {
                name: front.get("name").unwrap_or(file).to_owned(),
                description: front.get("description").unwrap_or_default().to_owned(),
                tools: front.list("tools"),
                model: front.get("model").map(str::to_owned),
                prompt: body.trim().to_owned(),
                source: Source {
                    eco: Eco::Kor,
                    scope: Scope::User,
                    path: PathBuf::from(format!("<built-in>/agents/{file}.md")),
                },
            }
        })
        .collect()
}

/// `configured` (user and project agents) followed by the built-ins whose names they do not use.
pub fn with_builtins(mut configured: Vec<AgentDef>) -> Vec<AgentDef> {
    for b in agents() {
        if !configured.iter().any(|a| a.name == b.name) {
            configured.push(b);
        }
    }
    configured
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_with_a_name_description_and_prompt() {
        let all = agents();
        assert_eq!(all.len(), AGENTS.len());
        for a in &all {
            assert!(!a.name.is_empty() && !a.name.contains(' '), "{}", a.name);
            assert!(!a.description.is_empty(), "{} has no description", a.name);
            assert!(
                !a.description.starts_with('"'),
                "{}: quotes are stripped",
                a.name
            );
            assert!(a.prompt.len() > 200, "{} has a short prompt", a.name);
        }
        let mut names: Vec<_> = all.iter().map(|a| a.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len(), "names are unique");
    }

    /// The shipped prompts must not tell subagents to override Kor's permission model or to treat
    /// untrusted content as instructions (the earlier agent set carried such a preamble).
    #[test]
    fn builtins_carry_no_jailbreak_preamble() {
        for (file, text) in AGENTS {
            let lower = text.to_lowercase();
            for banned in [
                "prompt defense baseline",
                "never refuse",
                "print the real values",
                "all in scope, all allowed",
                "route around a blocked command",
            ] {
                assert!(!lower.contains(banned), "{file} contains `{banned}`");
            }
        }
    }

    /// Tool names in the definitions must be Kor's (they match Claude Code's), so the subagent
    /// actually receives the tools its prompt expects.
    #[test]
    fn builtin_tool_names_are_known() {
        const KNOWN: &[&str] = &[
            "Read",
            "Write",
            "Edit",
            "Grep",
            "Glob",
            "Bash",
            "WebSearch",
            "WebFetch",
        ];
        for a in agents() {
            for t in &a.tools {
                assert!(
                    KNOWN.contains(&t.as_str()) || t.starts_with("mcp__"),
                    "{}: unknown tool `{t}`",
                    a.name
                );
            }
        }
    }

    #[test]
    fn configured_agents_override_builtins() {
        let mut mine = agents()
            .into_iter()
            .find(|a| a.name == "code-reviewer")
            .unwrap();
        mine.prompt = "my own reviewer".into();
        mine.source.scope = Scope::Project;
        let merged = with_builtins(vec![mine]);
        let reviewers: Vec<_> = merged
            .iter()
            .filter(|a| a.name == "code-reviewer")
            .collect();
        assert_eq!(reviewers.len(), 1);
        assert_eq!(reviewers[0].prompt, "my own reviewer");
        assert_eq!(merged.len(), AGENTS.len());
    }
}
