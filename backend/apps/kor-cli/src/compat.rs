//! `kor compat status`, `kor trust`, and the glue that turns `kor-compat` output into
//! agent settings (providers, permissions, slash commands).

use std::path::Path;

use kor_compat::{Applies, Compat, Scope, template};
use kor_i18n::{Localizer, args};

use crate::Ctx;

pub use kor_runtime::{load_compat as load, project_root, trust_store};

/// Message text with the project root shortened to `.` and Windows prefixes removed.
fn tidy(msg: &str, root: &Path) -> String {
    msg.replace(r"\\?\", "")
        .replace(&kor_tools::display_path(root), ".")
        .replace('\\', "/")
}

fn rel(root: &Path, p: &Path) -> String {
    let p = p.strip_prefix(root).unwrap_or(p);
    kor_tools::display_path(p).replace('\\', "/")
}

/// Lines shown per section without `--all`.
const SECTION_LIMIT: usize = 25;

#[allow(clippy::too_many_lines)]
pub fn status(ctx: &Ctx, all: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let limit = if all { usize::MAX } else { SECTION_LIMIT };
    let more = |n: usize| {
        if n > limit {
            println!(
                "  … {}",
                l.t_args("compat-more", &args!("count" => n - limit))
            );
        }
    };
    let root = project_root()?;
    let c = load(&root);
    let section = |key: &str, n: usize| println!("\n{}", l.t_args(key, &args!("count" => n)));
    println!(
        "{}",
        l.t_args(
            "compat-root",
            &args!("root" => kor_tools::display_path(&root))
        )
    );
    println!(
        "{}",
        l.t(if c.trusted {
            "compat-trusted"
        } else {
            "compat-untrusted"
        })
    );

    section("compat-instructions", c.instructions.len());
    for i in c.instructions.iter().take(limit) {
        let how = match &i.applies {
            Applies::Always => "always".to_owned(),
            Applies::Globs { globs } => format!("files: {}", globs.join(", ")),
            Applies::OnRequest { description } => format!("on request: {description}"),
            Applies::Manual => "manual".to_owned(),
        };
        println!(
            "  [{:?}/{:?}] {}  ({how})",
            i.source.eco,
            i.source.scope,
            rel(&root, &i.source.path)
        );
    }
    more(c.instructions.len());

    let p = &c.permissions;
    section(
        "compat-permissions",
        p.allow.len() + p.ask.len() + p.deny.len(),
    );
    for (kind, list) in [("allow", &p.allow), ("ask", &p.ask), ("deny", &p.deny)] {
        for r in list {
            println!("  {kind}: {}  ({})", r.rule, rel(&root, &r.source.path));
        }
    }
    if let Some((m, s)) = &p.mode {
        println!("  mode: {m}  ({})", rel(&root, &s.path));
    }

    section("compat-hooks", c.hooks.len());
    for h in c.hooks.iter().take(limit) {
        let m = h
            .matcher
            .as_deref()
            .map(|m| format!(" [{m}]"))
            .unwrap_or_default();
        println!(
            "  {}{m}: {}  ({})",
            h.event,
            h.command,
            rel(&root, &h.source.path)
        );
    }
    more(c.hooks.len());
    section("compat-commands", c.commands.len());
    for cmd in c.commands.iter().take(limit) {
        let d = cmd.description.as_deref().unwrap_or_default();
        println!("  /{}  {d}  ({})", cmd.name, rel(&root, &cmd.source.path));
    }
    more(c.commands.len());
    section("compat-agents", c.agents.len());
    for a in c.agents.iter().take(limit) {
        println!(
            "  {}: {}  ({})",
            a.name,
            a.description,
            rel(&root, &a.source.path)
        );
    }
    more(c.agents.len());
    section("compat-skills", c.skills.len());
    for s in c.skills.iter().take(limit) {
        let d: String = s.description.chars().take(100).collect();
        println!("  {}: {d}  ({})", s.name, rel(&root, &s.source.path));
    }
    more(c.skills.len());
    section("compat-mcp", c.mcp_servers.len());
    for m in c.mcp_servers.iter().take(limit) {
        println!(
            "  {}  [{:?}] ({})",
            m.name,
            m.source.eco,
            rel(&root, &m.source.path)
        );
    }
    more(c.mcp_servers.len());
    section("compat-providers", c.providers.len());
    for p in &c.providers {
        println!("  {} → {} ({})", p.id, p.base_url, p.wire_api);
    }
    if !c.env.is_empty() {
        section("compat-env", c.env.len());
        for (k, _, s) in &c.env {
            println!("  {k}  ({})", rel(&root, &s.path));
        }
    }
    if let Some((m, s)) = &c.model {
        println!(
            "\n{}",
            l.t_args(
                "compat-model",
                &args!("model" => m.clone(), "source" => rel(&root, &s.path))
            )
        );
    }
    if !c.ignore_files.is_empty() {
        section("compat-ignore", c.ignore_files.len());
        for f in &c.ignore_files {
            println!("  {}", rel(&root, f));
        }
    }
    if !c.untrusted_skipped.is_empty() {
        section("compat-skipped", c.untrusted_skipped.len());
        for s in &c.untrusted_skipped {
            println!("  {}", tidy(s, &root));
        }
        println!("{}", l.t("compat-trust-hint"));
    }
    if !c.warnings.is_empty() {
        section("compat-warnings", c.warnings.len());
        for w in c.warnings.iter().take(limit) {
            println!("  {}", tidy(w, &root));
        }
        more(c.warnings.len());
    }
    Ok(true)
}

/// `kor compat conflicts`: rules written differently by two tools, with the winner.
pub fn conflicts(ctx: &Ctx) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = project_root()?;
    let found = kor_compat::conflicts(&load(&root));
    if found.is_empty() {
        println!("{}", l.t("compat-conflicts-none"));
        return Ok(true);
    }
    println!(
        "{}",
        l.t_args("compat-conflicts", &args!("count" => found.len()))
    );
    for x in &found {
        let what = match &x.key {
            kor_compat::ConflictKey::Globs { globs } => {
                l.t_args("compat-conflict-globs", &args!("globs" => globs.join(", ")))
            }
            kor_compat::ConflictKey::Name { name } => {
                l.t_args("compat-conflict-name", &args!("name" => name.clone()))
            }
        };
        println!("\n{what}");
        for s in &x.rules {
            let mark = if s.path == x.winner { "*" } else { " " };
            println!("  {mark} [{:?}] {}", s.eco, rel(&root, &s.path));
        }
    }
    println!("\n{}", l.t("compat-conflicts-winner"));
    Ok(true)
}

/// `kor compat export --to <target>`: writes only under `.kor/export/<target>/`.
pub fn export(ctx: &Ctx, to: &str) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let target: kor_compat::export::Target = to.parse().map_err(|_| {
        anyhow::anyhow!(l.t_args(
            "compat-export-bad-target",
            &args!("target" => to.to_owned(), "targets" => kor_compat::export::Target::ALL.join(", "))
        ))
    })?;
    let root = project_root()?;
    let out = kor_compat::export::export(&root, target)?;
    for w in &out.warnings {
        println!("  {}", tidy(w, &root));
    }
    if out.files.is_empty() {
        println!("{}", l.t("compat-export-empty"));
        return Ok(true);
    }
    println!(
        "{}",
        l.t_args(
            "compat-export-done",
            &args!("count" => out.rules, "dir" => rel(&root, &out.dir))
        )
    );
    for f in &out.files {
        println!("  {}", kor_tools::display_path(f).replace('\\', "/"));
    }
    println!("{}", l.t("compat-export-copy-hint"));
    Ok(true)
}

pub fn trust(ctx: &Ctx, revoke: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = project_root()?;
    let Some(store) = trust_store() else {
        anyhow::bail!("no data directory");
    };
    kor_compat::trust::set(&store, &root, !revoke)?;
    let key = if revoke {
        "trust-revoked"
    } else {
        "trust-granted"
    };
    println!(
        "{}",
        l.t_args(key, &args!("root" => kor_tools::display_path(&root)))
    );
    Ok(true)
}

/// What a `/name args` line expands to before it is sent.
pub struct Expansion {
    pub parts: Vec<template::Part>,
    /// Permission rules the command grants while it runs (`allowed-tools`).
    pub allowed_tools: Vec<String>,
    /// A template's inputs did not validate; nothing must be sent.
    pub error: Option<kor_compat::templates::InputError>,
}

/// Look up a custom command, task template or skill for `/name`.
pub fn expand(c: &Compat, name: &str, args: &str) -> Option<Expansion> {
    if let Some(cmd) = c.commands.iter().find(|x| x.name == name) {
        // A command from an untrusted project must not grant itself tools.
        let allowed_tools = if cmd.source.scope == Scope::User || c.trusted {
            cmd.allowed_tools.clone()
        } else {
            Vec::new()
        };
        return Some(Expansion {
            parts: template::parts(cmd, args),
            allowed_tools,
            error: None,
        });
    }
    // Templates render to plain text only: no `!` commands, no `@` files (spec 15.11).
    if let Some(t) = kor_compat::templates::find(c, name) {
        let rendered = kor_compat::templates::split_words(args)
            .and_then(|w| kor_compat::templates::values(&t, &w))
            .map(|v| kor_compat::templates::render(&t, &v));
        return Some(match rendered {
            Ok(text) => Expansion {
                parts: vec![template::Part::Text(text)],
                allowed_tools: Vec::new(),
                error: None,
            },
            Err(e) => Expansion {
                parts: Vec::new(),
                allowed_tools: Vec::new(),
                error: Some(e),
            },
        });
    }
    c.skills
        .iter()
        .find(|s| s.name == name || slash_name(s) == name)
        .map(|s| Expansion {
            parts: vec![template::Part::Text(format!(
                "Use the `{}` skill (load it with the skill tool) for this task: {args}",
                s.name
            ))],
            allowed_tools: Vec::new(),
            error: None,
        })
}

pub fn custom_help(l: &Localizer, c: &Compat) {
    println!("{}", l.t("repl-help-custom"));
    for cmd in &c.commands {
        let hint = cmd.argument_hint.as_deref().unwrap_or_default();
        let d = cmd.description.as_deref().unwrap_or_default();
        println!("    /{} {hint}   {d}", cmd.name);
    }
    for t in kor_compat::templates::all(c) {
        if !c.commands.iter().any(|x| x.name == t.name) {
            println!("    /{} {}   {}", t.name, usage(&t), t.description);
        }
    }
    for s in c.skills.iter().take(HELP_SKILLS) {
        let d: String = s.description.chars().take(80).collect();
        println!("    /{}   {d}", slash_name(s));
    }
    if c.skills.len() > HELP_SKILLS {
        println!(
            "    … {}",
            l.t_args(
                "compat-more",
                &args!("count" => c.skills.len() - HELP_SKILLS)
            )
        );
    }
}

const HELP_SKILLS: usize = 20;

/// A template's inputs as flags: `--name <string> [--method <GET|POST>]`.
pub fn usage(t: &kor_compat::templates::Template) -> String {
    t.inputs
        .iter()
        .map(|i| {
            let flag = format!("--{} <{}>", i.name, i.kind);
            if i.required {
                flag
            } else {
                format!("[{flag}]")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One template input error as a localized line.
pub fn input_error(l: &Localizer, template: &str, e: &kor_compat::templates::InputError) -> String {
    use kor_compat::templates::InputError as E;
    let t = template.to_owned();
    match e {
        E::Unknown(n) => l.t_args(
            "templates-err-unknown",
            &args!("template" => t, "name" => n.clone()),
        ),
        E::Missing(n) => l.t_args(
            "templates-err-missing",
            &args!("template" => t, "name" => n.clone()),
        ),
        E::Duplicate(n) => l.t_args("templates-err-duplicate", &args!("name" => n.clone())),
        E::Invalid { name, expected } => l.t_args(
            "templates-err-invalid",
            &args!("name" => name.clone(), "expected" => expected.clone()),
        ),
        E::ExtraValue(v) => l.t_args("templates-err-extra", &args!("value" => v.clone())),
        E::Syntax => l.t("templates-err-syntax"),
    }
}

/// How a skill is typed after `/`: its name, or its directory name if the name has spaces.
fn slash_name(s: &kor_compat::Skill) -> String {
    if s.name.contains(char::is_whitespace) {
        s.dir
            .file_name()
            .map_or_else(|| s.name.clone(), |n| n.to_string_lossy().into_owned())
    } else {
        s.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_compat::{CustomProvider, Eco, Source};
    use kor_runtime::manifest_from_codex;

    #[test]
    fn template_usage_and_errors_are_localized() {
        use kor_compat::templates::InputError;
        let t = kor_compat::templates::builtins()
            .into_iter()
            .find(|t| t.name == "new-endpoint")
            .unwrap();
        assert_eq!(
            usage(&t),
            "--name <string> [--method <GET|POST|PUT|PATCH|DELETE>] [--route <string>] [--auth <bool>]"
        );
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            let l = Localizer::new(lang);
            for e in [
                InputError::Unknown("x".into()),
                InputError::Missing("name".into()),
                InputError::Duplicate("x".into()),
                InputError::Invalid {
                    name: "auth".into(),
                    expected: "bool".into(),
                },
                InputError::ExtraValue("v".into()),
                InputError::Syntax,
            ] {
                let s = input_error(&l, "new-endpoint", &e);
                assert!(
                    !s.is_empty() && !s.contains("templates-err"),
                    "{lang:?}: {s}"
                );
            }
        }
    }

    /// `/new-endpoint users` on the interactive screen expands the template to plain text; bad
    /// inputs carry an error instead, so nothing is sent.
    #[test]
    fn template_slash_invocation_renders_or_reports() {
        use kor_compat::templates::InputError;
        let c = Compat::default();
        let exp = expand(&c, "new-endpoint", "users --method POST").unwrap();
        assert!(exp.error.is_none());
        let [template::Part::Text(text)] = exp.parts.as_slice() else {
            panic!("one text part");
        };
        assert!(text.contains("Add a POST endpoint for the `users` resource."));
        let exp = expand(&c, "new-endpoint", "users --method TRACE").unwrap();
        assert!(exp.parts.is_empty());
        assert!(matches!(exp.error, Some(InputError::Invalid { .. })));
        assert!(expand(&c, "no-such-template", "").is_none());
    }

    #[test]
    fn codex_provider_becomes_a_valid_manifest() {
        let p = CustomProvider {
            id: "myproxy".into(),
            name: "My proxy".into(),
            base_url: "https://llm.example.com/v1".into(),
            env_key: Some("MYPROXY_API_KEY".into()),
            wire_api: "responses".into(),
            source: Source {
                eco: Eco::Codex,
                scope: Scope::User,
                path: "config.toml".into(),
            },
        };
        let m = manifest_from_codex(&p).unwrap();
        assert_eq!(m.adapter, kor_providers::AdapterKind::Openai);
        assert_eq!(m.key_env(), "MYPROXY_API_KEY");
        let bad = CustomProvider {
            base_url: "http://remote.example.com".into(),
            ..p
        };
        assert!(
            manifest_from_codex(&bad).is_err(),
            "plaintext remote must be rejected"
        );
    }
}
