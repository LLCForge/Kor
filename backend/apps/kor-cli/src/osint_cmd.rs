//! `kor os-integration list|status|install|uninstall|feed|last-reply|approve` (§27.10, ADR 0052).
//! The integrations themselves live in `kor_platform::os_integration`; `feed`, `last-reply` and
//! `approve` are the data the GNOME Shell extension, the Plasma widget, the `PowerToys` Run plugin
//! and the Shortcuts.app actions read (JSON / text, already localized).

use std::path::PathBuf;

use kor_i18n::{Localizer, args};
use kor_platform::os_integration::{self as osx, Env, Error, Options};
use serde_json::{Value, json};

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct OsIntegrationArgs {
    #[command(subcommand)]
    pub command: OsCmd,
}

#[derive(Debug, clap::Subcommand)]
pub enum OsCmd {
    /// Integrations for this operating system and their state.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Same as `list`, including WSL distros on Windows.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Install integrations (asks first unless --yes).
    Install {
        ids: Vec<String>,
        #[arg(long, conflicts_with = "ids")]
        all: bool,
        #[arg(long)]
        handoff_url: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Remove what Kor installed.
    Uninstall {
        ids: Vec<String>,
        #[arg(long, conflicts_with = "ids")]
        all: bool,
    },
    /// JSON for shell widgets and launchers: recent projects, sessions, waiting approvals.
    Feed {
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Print the last assistant reply.
    LastReply,
    /// Approve a waiting request (the oldest one without --id).
    Approve {
        #[arg(long)]
        id: Option<String>,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let json = |s: clap::Command| s.mut_arg("json", |a| a.help(l.t("arg-osx-json")));
    c.about(l.t("cmd-osx"))
        .mut_subcommand("list", |s| json(s.about(l.t("cmd-osx-list"))))
        .mut_subcommand("status", |s| json(s.about(l.t("cmd-osx-status"))))
        .mut_subcommand("install", |s| {
            s.about(l.t("cmd-osx-install"))
                .mut_arg("ids", |a| a.help(l.t("arg-osx-ids")))
                .mut_arg("all", |a| a.help(l.t("arg-osx-all")))
                .mut_arg("handoff_url", |a| a.help(l.t("arg-osx-handoff-url")))
                .mut_arg("yes", |a| a.help(l.t("arg-osx-yes")))
        })
        .mut_subcommand("uninstall", |s| {
            s.about(l.t("cmd-osx-uninstall"))
                .mut_arg("ids", |a| a.help(l.t("arg-osx-ids")))
                .mut_arg("all", |a| a.help(l.t("arg-osx-all")))
        })
        .mut_subcommand("feed", |s| {
            s.about(l.t("cmd-osx-feed"))
                .mut_arg("query", |a| a.help(l.t("arg-osx-query")))
                .mut_arg("limit", |a| a.help(l.t("arg-osx-limit")))
        })
        .mut_subcommand("last-reply", |s| s.about(l.t("cmd-osx-last-reply")))
        .mut_subcommand("approve", |s| {
            s.about(l.t("cmd-osx-approve"))
                .mut_arg("id", |a| a.help(l.t("arg-osx-id")))
        })
}

/// The environment for the running `kor`: integrations point at this executable; a desktop
/// bundle next to it provides the `PowerToys` plugin and the icon.
pub fn env() -> anyhow::Result<Env> {
    let exe = std::env::current_exe()?;
    let resources = osx::resources_near(&exe);
    let desktop = exe.with_file_name(kor_platform::exe("kor-desktop"));
    let icon = desktop.is_file().then_some(desktop);
    Ok(Env::current(exe, resources, icon)?)
}

/// Shortcuts texts in the user's language.
pub fn options(l: &Localizer, handoff_url: Option<String>) -> Options {
    Options {
        handoff_url,
        texts: osx::shortcuts::Texts {
            ask: l.t("osx-shortcut-ask"),
        },
    }
}

/// Localized message for an install/uninstall error.
pub fn error_text(l: &Localizer, e: &Error) -> String {
    match e {
        Error::Unknown(id) => l.t_args("osx-unknown", &args!("id" => id.clone())),
        Error::Unavailable(r) | Error::NotManaged(r) => l.t(&format!("osx-reason-{r}")),
        Error::Foreign(p) => l.t_args("osx-foreign", &args!("path" => p.display().to_string())),
        Error::Io(e) => e.to_string(),
    }
}

fn print_status(l: &Localizer, env: &Env, json_out: bool, distros: bool) {
    let all = osx::status_all(env);
    if json_out {
        println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
        return;
    }
    if all.is_empty() {
        println!("{}", l.t("osx-none"));
        return;
    }
    for s in &all {
        let mark = if s.installed {
            "✓"
        } else if s.available {
            "·"
        } else {
            "–"
        };
        let label = l.t(&format!("osx-item-{}", s.id));
        let note = match (s.via, s.reason) {
            (Some(via), _) => format!(" ({})", l.t_args("osx-via", &args!("via" => via))),
            (None, Some(r)) => format!(" ({})", l.t(&format!("osx-reason-{r}"))),
            _ => String::new(),
        };
        println!("{mark} {:<20} {label}{note}", s.id);
    }
    if distros && env.os == kor_platform::Os::Windows {
        let d = kor_platform::wsl::distros();
        if !d.is_empty() {
            println!(
                "\n{}",
                l.t_args("osx-wsl-distros", &args!("list" => d.join(", ")))
            );
        }
    }
}

/// Ids to act on: the explicit list, or every installable item of this OS with `all`.
fn targets(env: &Env, ids: &[String], all: bool, installed_only: bool) -> Vec<String> {
    if !all {
        return ids.to_vec();
    }
    osx::status_all(env)
        .into_iter()
        .filter(|s| matches!(s.kind, "install" | "desktop"))
        .filter(|s| {
            if installed_only {
                s.installed
            } else {
                s.available
            }
        })
        .map(|s| s.id.to_owned())
        .collect()
}

pub fn run(ctx: &Ctx, a: &OsIntegrationArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    match &a.command {
        OsCmd::List { json } | OsCmd::Status { json } => {
            print_status(l, &env()?, *json, matches!(a.command, OsCmd::Status { .. }));
            if !*json {
                let cfg = ctx
                    .config
                    .as_ref()
                    .map(|c| c.config.integrations.clone())
                    .unwrap_or_default();
                println!("\n{}", kor_runtime::autointegrate::status_line(l, &cfg));
            }
            Ok(true)
        }
        OsCmd::Install {
            ids,
            all,
            handoff_url,
            yes,
        } => {
            let mut env = env()?;
            let list = targets(&env, ids, *all, false);
            if list.is_empty() {
                eprintln!("{}", l.t("osx-nothing"));
                return Ok(false);
            }
            let names: Vec<String> = list.iter().map(|i| l.t(&format!("osx-item-{i}"))).collect();
            if !crate::shell_cmd::confirm(
                l,
                &l.t_args("osx-confirm", &args!("list" => names.join(", "))),
                *yes,
            ) {
                return Ok(false);
            }
            let opts = options(l, handoff_url.clone());
            let mut ok = true;
            for id in &list {
                match osx::install(&mut env, id, &opts) {
                    Ok(_) => println!("{}", l.t_args("osx-installed", &args!("id" => id.clone()))),
                    Err(e) => {
                        ok = false;
                        eprintln!("{id}: {}", error_text(l, &e));
                    }
                }
                if id == "portals" {
                    println!("{}", l.t("osx-portals-desktop"));
                }
                if id == "powertoys-run" {
                    println!("{}", l.t("osx-powertoys-restart"));
                }
            }
            Ok(ok)
        }
        OsCmd::Uninstall { ids, all } => {
            let mut env = env()?;
            let list = targets(&env, ids, *all, true);
            let mut ok = true;
            for id in &list {
                match osx::uninstall(&mut env, id) {
                    Ok((_, left)) => {
                        println!("{}", l.t_args("osx-removed", &args!("id" => id.clone())));
                        if let Some(r) = left {
                            println!("{}", l.t(&format!("osx-reason-{r}")));
                        }
                    }
                    Err(e) => {
                        ok = false;
                        eprintln!("{id}: {}", error_text(l, &e));
                    }
                }
            }
            Ok(ok)
        }
        OsCmd::Feed { query, limit } => {
            let waiting = waiting();
            let v = feed(
                l,
                &Sources::current(),
                query.as_deref(),
                (*limit).clamp(1, 50),
                &waiting,
            );
            println!("{v}");
            Ok(true)
        }
        OsCmd::LastReply => {
            if let Some(t) = last_reply(&Sources::current()) {
                println!("{t}");
                Ok(true)
            } else {
                println!("{}", l.t("osx-no-reply"));
                Ok(false)
            }
        }
        OsCmd::Approve { id } => approve(l, id.clone()),
    }
}

/// Where the feed reads from (injected in tests).
#[derive(Debug, Clone)]
pub struct Sources {
    pub sessions: Option<PathBuf>,
    /// Desktop `desktop.json` (session titles and folders).
    pub desktop: Option<PathBuf>,
    /// Projects registered with the trigger daemon.
    pub projects: Vec<PathBuf>,
}

impl Sources {
    fn current() -> Self {
        Self {
            sessions: kor_runtime::sessions_dir(),
            desktop: kor_runtime::kor_home().map(|h| h.join("desktop.json")),
            projects: kor_runtime::triggers::dir()
                .map(|d| kor_runtime::triggers::projects(&d))
                .unwrap_or_default(),
        }
    }
}

/// Approvals the daemon waits for: `(id, summary)`.
fn waiting() -> Vec<(String, String)> {
    let Some(dir) = kor_runtime::triggers::dir() else {
        return Vec::new();
    };
    if kor_runtime::triggers::daemon::state(&dir).is_none() {
        return Vec::new();
    }
    let Ok(rt) = crate::providers::runtime() else {
        return Vec::new();
    };
    let v = rt.block_on(kor_runtime::triggers::daemon::status(&dir));
    v["waiting"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            Some((
                w["id"].as_str()?.to_owned(),
                w["summary"].as_str().unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

fn first_line(s: &str, n: usize) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(n)
        .collect()
}

/// The feed JSON (see module docs).
pub fn feed(
    l: &Localizer,
    src: &Sources,
    query: Option<&str>,
    limit: usize,
    waiting: &[(String, String)],
) -> Value {
    let q = query
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    let hit = |s: &str| {
        q.as_ref()
            .is_none_or(|q| s.to_lowercase().contains(q.as_str()))
    };
    let meta: Value = src
        .desktop
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let files = src
        .sessions
        .as_deref()
        .map(kor_agent::session::list)
        .unwrap_or_default();
    let mut sessions = Vec::new();
    let mut projects: Vec<PathBuf> = Vec::new();
    for p in &files {
        let id = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let m = &meta["sessions"][&id];
        if let Some(root) = m["root"].as_str() {
            let root = PathBuf::from(root);
            if !projects.contains(&root) {
                projects.push(root);
            }
        }
        if sessions.len() >= limit {
            continue;
        }
        let title = m["customTitle"].as_str().map(str::to_owned).or_else(|| {
            kor_agent::session::load(p).ok().and_then(|msgs| {
                msgs.into_iter()
                    .find(|m| m.role == kor_core::Role::User)
                    .map(|m| first_line(&m.plain_text(), 70))
            })
        });
        let title = title
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| id.clone());
        if hit(&title) {
            sessions.push(json!({ "id": id, "title": title, "url": format!("kor://resume?id={}", kor_runtime::osint::encode(&id)) }));
        }
    }
    for p in &src.projects {
        if !projects.contains(p) {
            projects.push(p.clone());
        }
    }
    let projects: Vec<Value> = projects
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| {
            (
                p,
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
            )
        })
        .filter(|(p, n)| hit(n) || hit(&p.to_string_lossy()))
        .take(limit)
        .map(|(p, n)| json!({ "name": n, "path": p, "url": kor_runtime::osint::open_url(p) }))
        .collect();
    let waiting: Vec<Value> = waiting
        .iter()
        .map(|(id, s)| json!({ "id": id, "summary": s }))
        .collect();
    let status = if waiting.is_empty() {
        l.t("osx-feed-idle")
    } else {
        l.t_args("osx-feed-waiting", &args!("count" => waiting.len()))
    };
    json!({
        "status": status,
        "newTask": "kor://new",
        "ask": q.as_ref().map(|_| format!("kor://task?prompt={}", kor_runtime::osint::encode(query.unwrap_or_default().trim()))),
        "askLabel": query.map(|q| l.t_args("osx-feed-ask", &args!("q" => q.trim().to_owned()))),
        "projects": projects,
        "sessions": sessions,
        "waiting": waiting,
        "labels": {
            "newTask": l.t("osx-feed-new-task"),
            "projects": l.t("osx-feed-projects"),
            "sessions": l.t("osx-feed-sessions"),
            "waiting": l.t("osx-feed-waiting-title"),
            "approve": l.t("osx-feed-approve"),
            "open": l.t("osx-feed-open"),
        }
    })
}

/// The newest session's last assistant text.
pub fn last_reply(src: &Sources) -> Option<String> {
    let files = kor_agent::session::list(src.sessions.as_deref()?);
    files.iter().find_map(|p| {
        kor_agent::session::load(p)
            .ok()?
            .into_iter()
            .rev()
            .find(|m| m.role == kor_core::Role::Assistant)
            .map(|m| m.plain_text().trim().to_owned())
            .filter(|t| !t.is_empty())
    })
}

fn approve(l: &Localizer, id: Option<String>) -> anyhow::Result<bool> {
    let Some(id) = id.or_else(|| waiting().into_iter().next().map(|(i, _)| i)) else {
        println!("{}", l.t("triggers-no-pending"));
        return Ok(false);
    };
    let dir = kor_runtime::triggers::dir().ok_or_else(|| anyhow::anyhow!("no Kor data folder"))?;
    let v = crate::providers::runtime()?
        .block_on(kor_runtime::triggers::daemon::control(
            &dir,
            "POST",
            "/control/answer",
            Some(json!({ "id": id, "approve": true })),
        ))
        .map_err(|e| anyhow::anyhow!(e))?;
    let ok = v["ok"] == true;
    println!(
        "{}",
        l.t(if ok {
            "triggers-answered"
        } else {
            "triggers-no-pending"
        })
    );
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};
    use kor_core::{Message, Role};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: OsIntegrationArgs,
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from(["x", "install", "--all", "--yes"]).unwrap();
        assert!(matches!(
            t.a.command,
            OsCmd::Install {
                all: true,
                yes: true,
                ..
            }
        ));
        let t = T::try_parse_from([
            "x",
            "install",
            "shortcuts",
            "handoff",
            "--handoff-url",
            "https://k",
        ])
        .unwrap();
        assert!(
            matches!(&t.a.command, OsCmd::Install { ids, handoff_url: Some(_), .. } if ids.len() == 2)
        );
        assert!(T::try_parse_from(["x", "install", "a", "--all"]).is_err());
        assert!(matches!(
            T::try_parse_from(["x", "feed", "--query", "api"])
                .unwrap()
                .a
                .command,
            OsCmd::Feed { limit: 5, .. }
        ));
        assert!(matches!(
            T::try_parse_from(["x", "approve", "--id", "ab"])
                .unwrap()
                .a
                .command,
            OsCmd::Approve { id: Some(_) }
        ));
        assert!(matches!(
            T::try_parse_from(["x", "last-reply"]).unwrap().a.command,
            OsCmd::LastReply
        ));
        assert!(matches!(
            T::try_parse_from(["x", "uninstall", "--all"])
                .unwrap()
                .a
                .command,
            OsCmd::Uninstall { all: true, .. }
        ));
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            localize(T::command(), &Localizer::new(lang)).debug_assert();
        }
    }

    #[test]
    fn feed_lists_sessions_projects_and_approvals() {
        let d = tempfile::tempdir().unwrap();
        let sessions = d.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let proj = d.path().join("api-server");
        std::fs::create_dir_all(&proj).unwrap();
        let other = d.path().join("web");
        std::fs::create_dir_all(&other).unwrap();
        let write = |id: &str, msgs: &[Message]| {
            kor_agent::session::save(&sessions.join(format!("{id}.jsonl")), msgs).unwrap()
        };
        write(
            "20260101-000000-1",
            &[
                Message::text(Role::User, "fix the login bug"),
                Message::text(Role::Assistant, "Done: login fixed."),
            ],
        );
        write(
            "20260102-000000-2",
            &[Message::text(Role::User, "write docs")],
        );
        let desktop = d.path().join("desktop.json");
        std::fs::write(
            &desktop,
            serde_json::to_string(&json!({ "sessions": { "20260101-000000-1": { "root": proj, "customTitle": "Login" } } })).unwrap(),
        )
        .unwrap();
        let src = Sources {
            sessions: Some(sessions),
            desktop: Some(desktop),
            projects: vec![other.clone()],
        };
        let l = Localizer::new(kor_i18n::Lang::En);
        let v = feed(&l, &src, None, 5, &[("w1".into(), "deploy?".into())]);
        let titles: Vec<&str> = v["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap())
            .collect();
        assert_eq!(
            titles,
            ["write docs", "Login"],
            "newest first, custom title wins"
        );
        assert_eq!(v["sessions"][1]["url"], "kor://resume?id=20260101-000000-1");
        let names: Vec<&str> = v["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["api-server", "web"]);
        assert!(
            v["projects"][0]["url"]
                .as_str()
                .unwrap()
                .starts_with("kor://open?path=")
        );
        assert_eq!(v["waiting"][0]["id"], "w1");
        assert!(v["ask"].is_null());
        let v = feed(&l, &src, Some("login fix"), 5, &[]);
        assert_eq!(v["ask"], "kor://task?prompt=login%20fix");
        let v = feed(&l, &src, Some("api"), 5, &[]);
        assert_eq!(v["projects"].as_array().unwrap().len(), 1);
        assert!(v["sessions"].as_array().unwrap().is_empty());
        assert_eq!(last_reply(&src).as_deref(), Some("Done: login fixed."));
    }
}
