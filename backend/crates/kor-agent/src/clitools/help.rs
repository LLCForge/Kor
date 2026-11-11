//! `kor tool describe <name>`: a program's own `--help` (or `-h`, `help`, `man`), summarized
//! deterministically — usage line, one-line description, subcommands, flags — and cached in
//! `<kor home>/tools/help/<name>.json`. The help text runs inside the session sandbox with the
//! network off; only programs found on `PATH` can be described. A model summary is optional
//! ([`model_summary`]).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kor_core::{Message, Role};
use kor_providers::{CancellationToken, ChatRequest, Provider};
use serde::{Deserialize, Serialize};

use super::defs::{ArgDef, ArgType, Safety, ToolDef};

const HELP_TIMEOUT: Duration = Duration::from_secs(8);
/// Help text kept for parsing (some programs print megabytes).
const MAX_HELP: usize = 64 * 1024;
const MAX_ITEMS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Help {
    pub name: String,
    pub path: PathBuf,
    /// `--help`, `-h`, `help` or `man`.
    pub source: String,
    pub usage: String,
    pub summary: String,
    pub subcommands: Vec<(String, String)>,
    pub flags: Vec<(String, String)>,
    /// Model-written summary when one was asked for (`kor tool describe --model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_summary: Option<String>,
    /// First lines of the raw help, for the model.
    pub excerpt: String,
}

/// Program names Kor will run `--help` for: a plain command name, nothing path-like.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        && !name.starts_with(['-', '.'])
}

fn cache_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// The cached summary of `name`, if any.
pub fn cached(dir: &Path, name: &str) -> Option<Help> {
    if !valid_name(name) {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(cache_file(dir, name)).ok()?).ok()
}

/// # Errors
/// When the file cannot be written.
pub fn store(dir: &Path, h: &Help) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        cache_file(dir, &h.name),
        serde_json::to_vec_pretty(h).map_err(std::io::Error::other)?,
    )
}

fn is_flag_line(l: &str) -> Option<(String, String)> {
    let t = l.trim_start();
    let indent = l.len() - t.len();
    if !t.starts_with('-') || indent > 12 || t.starts_with("---") {
        return None;
    }
    // Flag spec up to the first run of two spaces (or a tab), then its description.
    let (spec, desc) = match t.find("  ").or_else(|| t.find('\t')) {
        Some(i) => (t[..i].trim(), t[i..].trim()),
        None => (t.trim(), ""),
    };
    if spec.is_empty() || spec.len() > 60 {
        return None;
    }
    Some((spec.to_owned(), desc.chars().take(160).collect()))
}

fn is_section(l: &str) -> bool {
    let t = l.trim();
    !t.is_empty()
        && !l.starts_with(char::is_whitespace)
        && (t.ends_with(':') || t.chars().all(|c| c.is_ascii_uppercase() || c == ' '))
}

/// Deterministic summary of a help text.
pub fn summarize(name: &str, path: &Path, source: &str, text: &str) -> Help {
    let text: String = text.chars().take(MAX_HELP).collect();
    let lines: Vec<&str> = text.lines().collect();
    let mut usage = String::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("usage:")
            || lower.starts_with("usage ")
            || (lower == "usage" || lower == "synopsis")
        {
            t.split_once(':')
                .map_or("", |(_, r)| r)
                .trim()
                .clone_into(&mut usage);
            if usage.is_empty() {
                lines
                    .get(i + 1)
                    .map_or("", |s| s.trim())
                    .clone_into(&mut usage);
            }
            break;
        }
    }
    let summary = lines
        .iter()
        .map(|l| l.trim())
        .find(|t| {
            let lower = t.to_ascii_lowercase();
            !t.is_empty()
                && !t.starts_with('-')
                && !lower.starts_with("usage")
                && !t.ends_with(':')
                && t.len() > 8
                && !t.starts_with(&format!("{name} ["))
        })
        .map(|t| t.chars().take(200).collect())
        .unwrap_or_default();
    let mut flags = Vec::new();
    let mut subcommands = Vec::new();
    let mut in_commands = false;
    for l in &lines {
        if is_section(l) {
            let lower = l.trim().to_ascii_lowercase();
            in_commands = lower.contains("command") && !lower.contains("option");
            continue;
        }
        if let Some(f) = is_flag_line(l) {
            if flags.len() < MAX_ITEMS && !flags.iter().any(|(s, _): &(String, String)| *s == f.0) {
                flags.push(f);
            }
            continue;
        }
        if in_commands && l.starts_with(char::is_whitespace) && subcommands.len() < MAX_ITEMS {
            let t = l.trim_start();
            let (cmd, desc) = t
                .split_once("  ")
                .map_or((t, ""), |(a, b)| (a.trim(), b.trim()));
            let ok = !cmd.is_empty()
                && cmd.len() <= 32
                && cmd
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ',' | ' '))
                && cmd.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
            if ok && !desc.is_empty() {
                let first = cmd.split([',', ' ']).next().unwrap_or(cmd).to_owned();
                subcommands.push((first, desc.chars().take(160).collect()));
            }
        }
    }
    Help {
        name: name.to_owned(),
        path: path.to_owned(),
        source: source.to_owned(),
        usage,
        summary,
        subcommands,
        flags,
        model_summary: None,
        excerpt: lines
            .iter()
            .take(40)
            .copied()
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Compact text of a summary for the model or the terminal.
pub fn render(h: &Help) -> String {
    let mut out = format!("{} ({}, from `{}`)\n", h.name, h.path.display(), h.source);
    if let Some(m) = &h.model_summary {
        let _ = writeln!(out, "{m}");
    } else if !h.summary.is_empty() {
        let _ = writeln!(out, "{}", h.summary);
    }
    if !h.usage.is_empty() {
        let _ = writeln!(out, "usage: {}", h.usage);
    }
    if !h.subcommands.is_empty() {
        out.push_str("subcommands:\n");
        for (c, d) in &h.subcommands {
            let _ = writeln!(out, "  {c}  {d}");
        }
    }
    if !h.flags.is_empty() {
        out.push_str("flags:\n");
        for (f, d) in &h.flags {
            let _ = writeln!(out, "  {f}  {d}");
        }
    }
    out
}

/// Run `args` of `path` sandboxed (network off), stdout+stderr, cut to [`MAX_HELP`].
async fn run_help(path: &Path, args: &[&str], ctx: &kor_tools::ToolCtx) -> Option<String> {
    let mut policy = ctx.sandbox_policy();
    policy.network = false;
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args)
        .current_dir(&ctx.root)
        .env("PAGER", "cat")
        .env("MANPAGER", "cat")
        .env("NO_COLOR", "1");
    for v in &ctx.scrub_env {
        cmd.env_remove(v);
    }
    let child = kor_sandbox::spawn(ctx.sandbox, &policy, &cmd).ok()?;
    let out = tokio::time::timeout(HELP_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().len() < 40 {
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    let text: String = text.chars().take(MAX_HELP).collect();
    (text.trim().len() >= 20).then_some(text)
}

/// Describe the PATH program `name` (cached unless `refresh`).
///
/// # Errors
/// Invalid name, not on PATH, or no help text.
pub async fn describe(
    dir: &Path,
    name: &str,
    refresh: bool,
    ctx: &kor_tools::ToolCtx,
) -> Result<Help, String> {
    if !valid_name(name) {
        return Err(format!("`{name}` is not a command name"));
    }
    if !refresh && let Some(h) = cached(dir, name) {
        return Ok(h);
    }
    let path =
        kor_platform::on_path::which(name).ok_or_else(|| format!("`{name}` is not on PATH"))?;
    let mut found = None;
    for (source, args) in [
        ("--help", &["--help"][..]),
        ("-h", &["-h"][..]),
        ("help", &["help"][..]),
    ] {
        if let Some(t) = run_help(&path, args, ctx).await {
            found = Some((source, t));
            break;
        }
    }
    if found.is_none()
        && let Some(man) = kor_platform::on_path::which("man")
        && let Some(t) = run_help(&man, &[name], ctx).await
    {
        found = Some(("man", t));
    }
    let (source, text) = found.ok_or_else(|| format!("`{name}` printed no help"))?;
    let h = summarize(name, &path, source, &text);
    let _ = store(dir, &h);
    Ok(h)
}

/// A short model-written summary of the help (optional, `kor tool describe --model`).
///
/// # Errors
/// The provider error.
pub async fn model_summary(
    provider: &Arc<dyn Provider>,
    model: &str,
    h: &Help,
) -> Result<String, String> {
    let req = ChatRequest {
        model: model.to_owned(),
        system: Some("Summarize a command-line program's help for a coding agent: what it does in one sentence, then the 5 most useful invocations as `command` — short description lines. No preamble.".into()),
        messages: vec![Message::text(Role::User, format!("{}\n\n{}", render(h), h.excerpt))],
        tools: Vec::new(),
        max_tokens: Some(400),
        temperature: Some(0.0),
        reasoning: None,
    };
    let stream = provider
        .complete(req, CancellationToken::new())
        .await
        .map_err(|e| e.message)?;
    let text = crate::subagent::collect_text(stream)
        .await
        .map_err(|e| e.message)?;
    let text = text.trim().to_owned();
    if text.is_empty() {
        Err("empty summary".into())
    } else {
        Ok(text)
    }
}

/// A typed tool definition generated from a help summary: the program, an optional subcommand
/// from the parsed list, flags restricted to the parsed ones, and positional arguments.
pub fn generate(h: &Help) -> ToolDef {
    let mut args = std::collections::BTreeMap::new();
    if !h.subcommands.is_empty() {
        args.insert(
            "subcommand".to_owned(),
            ArgDef {
                ty: ArgType::Enum,
                required: false,
                description: "subcommand".into(),
                values: h.subcommands.iter().map(|(c, _)| c.clone()).collect(),
                ..ArgDef::default()
            },
        );
    }
    let flag_names: Vec<String> = h
        .flags
        .iter()
        .flat_map(|(spec, _)| {
            spec.split([',', ' ', '='])
                .filter(|p| p.starts_with('-') && p.len() > 1)
                .map(|p| p.trim_end_matches(['[', '<']).to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    args.insert(
        "flags".to_owned(),
        ArgDef {
            ty: ArgType::Array,
            required: false,
            description: "flags, each one of the program's own (a value may follow `=`)".into(),
            values: flag_names,
            ..ArgDef::default()
        },
    );
    args.insert(
        "args".to_owned(),
        ArgDef {
            ty: ArgType::Array,
            required: false,
            description: "positional arguments".into(),
            ..ArgDef::default()
        },
    );
    let mut command = vec![h.name.clone()];
    if !h.subcommands.is_empty() {
        command.push("{subcommand}".into());
    }
    command.extend(["{flags}".to_owned(), "{args}".to_owned()]);
    ToolDef {
        name: format!("{}_cli", h.name.replace(['-', '.', '+'], "_")),
        description: if h.summary.is_empty() {
            format!("Run {}", h.name)
        } else {
            h.summary.clone()
        },
        command,
        args,
        safety: Safety::Write,
        approval: false,
        examples: Vec::new(),
        timeout_s: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT_LIKE: &str = "usage: tool [--version] [-C <path>] <command> [<args>]\n\nA small tool that does things.\n\nAvailable commands:\n   init       Create an empty repository\n   status     Show the working tree status\n   commit     Record changes\n\nOptions:\n  -v, --verbose        Be more verbose\n  -C <path>            Run as if started in <path>\n      --version        Print the version\n";

    #[test]
    fn help_text_is_summarized_and_turned_into_a_schema() {
        let h = summarize("tool", Path::new("/bin/tool"), "--help", GIT_LIKE);
        assert_eq!(h.usage, "tool [--version] [-C <path>] <command> [<args>]");
        assert_eq!(h.summary, "A small tool that does things.");
        assert_eq!(
            h.subcommands
                .iter()
                .map(|s| s.0.as_str())
                .collect::<Vec<_>>(),
            ["init", "status", "commit"]
        );
        assert_eq!(
            h.flags[0],
            ("-v, --verbose".into(), "Be more verbose".into())
        );
        assert_eq!(h.flags.len(), 3);
        let r = render(&h);
        assert!(r.contains("subcommands:\n  init  Create an empty repository"));
        let d = generate(&h);
        assert_eq!(d.name, "tool_cli");
        assert_eq!(d.command, ["tool", "{subcommand}", "{flags}", "{args}"]);
        assert!(d.args["flags"].values.contains(&"--verbose".to_owned()));
        assert!(d.args["flags"].values.contains(&"-C".to_owned()));
        assert_eq!(d.args["subcommand"].values, ["init", "status", "commit"]);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn names_and_cache() {
        assert!(valid_name("git") && valid_name("redis-cli") && valid_name("g++"));
        assert!(!valid_name("../x") && !valid_name("-rf") && !valid_name("a b") && !valid_name(""));
        let dir = tempfile::tempdir().unwrap();
        let h = summarize("tool", Path::new("/bin/tool"), "--help", GIT_LIKE);
        store(dir.path(), &h).unwrap();
        assert_eq!(cached(dir.path(), "tool"), Some(h));
        assert_eq!(cached(dir.path(), "../tool"), None);
    }

    #[tokio::test]
    async fn describes_a_real_program_on_path() {
        // `git` is on every CI image and developer machine this repository is built on.
        if kor_platform::on_path::which("git").is_none() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let ctx = kor_tools::ToolCtx::new(root.path()).unwrap();
        let cache = tempfile::tempdir().unwrap();
        let h = describe(cache.path(), "git", false, &ctx).await.unwrap();
        assert!(
            !h.flags.is_empty() || !h.subcommands.is_empty() || !h.usage.is_empty(),
            "{h:?}"
        );
        assert!(cached(cache.path(), "git").is_some());
        assert!(
            describe(cache.path(), "kor-no-such-program-xyz", false, &ctx)
                .await
                .is_err()
        );
    }
}
