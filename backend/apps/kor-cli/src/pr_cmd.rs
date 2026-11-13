//! `kor pr create|review|respond|list|status|ci|issue|release` (§8.8, §28.7, ADR 0040). Forge logic lives in
//! `kor_runtime::forge` and `kor_runtime::review`; this module asks, prints and confirms.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Subcommand;
use kor_agent::gitflow;
use kor_i18n::{Localizer, args};
use kor_providers::Provider;
use kor_runtime::forge::{self, Forge, ForgeError, PrRequest};
use kor_runtime::review;

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum PrCmd {
    /// Push the current branch and open a pull request with a written description.
    Create {
        /// Target branch (default: the remote's default branch).
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        draft: bool,
        /// Use this title instead of the generated one.
        #[arg(long)]
        title: Option<String>,
        /// Push and create without asking.
        #[arg(long, short)]
        yes: bool,
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Review a pull request and post the review as comments (after confirmation).
    Review {
        /// PR number or URL.
        target: String,
        #[arg(long)]
        persona: Option<String>,
        #[arg(long, short)]
        yes: bool,
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Draft answers and fixes for review comments; review each file, then push and reply.
    Respond {
        /// PR number or URL.
        target: String,
        #[arg(long, short)]
        yes: bool,
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Open pull requests.
    List {
        #[arg(long)]
        json: bool,
    },
    /// The pull request of the current branch.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// CI checks of the current commit; --fix starts a task that fixes the failing ones.
    Ci {
        #[arg(long)]
        fix: bool,
        #[arg(long, short)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Turn an issue (Azure DevOps: work item) into a task.
    Issue {
        /// Issue number or URL.
        target: String,
        /// Start the task now (otherwise the task text is printed).
        #[arg(long)]
        run: bool,
        #[arg(long, short)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Publish release notes for a tag on the forge.
    Release {
        tag: String,
        /// Previous tag (default: the tag before `tag`).
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        draft: bool,
        #[arg(long, short)]
        yes: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-pr"))
        .mut_subcommand("create", |s| s.about(l.t("cmd-pr-create")))
        .mut_subcommand("review", |s| s.about(l.t("cmd-pr-review")))
        .mut_subcommand("respond", |s| s.about(l.t("cmd-pr-respond")))
        .mut_subcommand("list", |s| s.about(l.t("cmd-pr-list")))
        .mut_subcommand("status", |s| s.about(l.t("cmd-pr-status")))
        .mut_subcommand("ci", |s| s.about(l.t("cmd-pr-ci")))
        .mut_subcommand("issue", |s| s.about(l.t("cmd-pr-issue")))
        .mut_subcommand("release", |s| s.about(l.t("cmd-pr-release")))
}

fn fail(l: &Localizer, e: &ForgeError) -> bool {
    eprintln!("{}", forge::error_text(l, e));
    false
}

pub(crate) fn repo(l: &Localizer) -> anyhow::Result<Option<PathBuf>> {
    Ok(match gitflow::repo_root(&std::env::current_dir()?) {
        Ok(r) => Some(r),
        Err(e) => {
            fail(l, &e.into());
            None
        }
    })
}

/// The one-shot model, or `None` with the reason printed (callers fall back or stop).
pub(crate) fn model(ctx: &Ctx, m: Option<&str>) -> Option<(Arc<dyn Provider>, String)> {
    let l = &ctx.l;
    let loaded = match &ctx.config {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("error-generic", &args!("error" => e.to_string()))
            );
            return None;
        }
    };
    match kor_runtime::oneshot_provider(l, loaded, m) {
        Ok(p) => {
            eprintln!(
                "{}",
                l.t_args(
                    "pr-sending",
                    &args!("provider" => p.0.manifest().name.clone())
                )
            );
            Some(p)
        }
        Err(e) => {
            eprintln!("{e}");
            None
        }
    }
}

fn number(l: &Localizer, target: &str) -> Option<u64> {
    let n = forge::pr_number(target);
    if n.is_none() {
        eprintln!(
            "{}",
            l.t_args("pr-bad-target", &args!("target" => target.to_owned()))
        );
    }
    n
}

#[allow(clippy::too_many_lines)] // one arm per subcommand
pub fn run(ctx: &Ctx, cmd: PrCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(root) = repo(l)? else {
        return Ok(false);
    };
    let rt = crate::providers::runtime()?;
    match cmd {
        PrCmd::Create {
            base,
            draft,
            title,
            yes,
            model: m,
        } => Ok(create(
            ctx,
            &rt,
            &root,
            base,
            draft,
            title,
            yes,
            m.as_deref(),
        )),
        PrCmd::Review {
            target,
            persona,
            yes,
            model: m,
        } => Ok(pr_review(
            ctx,
            &rt,
            &root,
            &target,
            persona.as_deref(),
            yes,
            m.as_deref(),
        )),
        PrCmd::Respond {
            target,
            yes,
            model: m,
        } => Ok(respond(ctx, &rt, &root, &target, yes, m.as_deref())),
        PrCmd::List { json } => {
            let f = match Forge::open(&root) {
                Ok(f) => f,
                Err(e) => return Ok(fail(l, &e)),
            };
            let prs = match rt.block_on(f.list()) {
                Ok(p) => p,
                Err(e) => return Ok(fail(l, &e)),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&prs)?);
            } else if prs.is_empty() {
                println!("{}", l.t("pr-none-open"));
            }
            if !json {
                for p in &prs {
                    let draft = if p.draft { " [draft]" } else { "" };
                    println!(
                        "#{:<5} {}{draft}  {} → {}  @{}",
                        p.number, p.title, p.head, p.base, p.author
                    );
                }
            }
            Ok(true)
        }
        PrCmd::Status { json } => {
            let branch = match forge::current_branch(&root) {
                Ok(b) => b,
                Err(e) => return Ok(fail(l, &e)),
            };
            let f = match Forge::open(&root) {
                Ok(f) => f,
                Err(e) => return Ok(fail(l, &e)),
            };
            let pr = match rt.block_on(f.for_branch(&branch)) {
                Ok(p) => p,
                Err(e) => return Ok(fail(l, &e)),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&pr)?);
                return Ok(true);
            }
            match pr {
                Some(p) => println!(
                    "#{} {} [{}{}]\n{} → {}\n{}",
                    p.number,
                    p.title,
                    p.state,
                    if p.draft { ", draft" } else { "" },
                    p.head,
                    p.base,
                    p.url
                ),
                None => println!("{}", l.t_args("pr-status-none", &args!("branch" => branch))),
            }
            Ok(true)
        }
        PrCmd::Ci { fix, yes, json } => ci(ctx, &rt, &root, fix, yes, json),
        PrCmd::Issue {
            target,
            run,
            yes,
            json,
        } => issue(ctx, &rt, &root, &target, run, yes, json),
        PrCmd::Release {
            tag,
            since,
            draft,
            yes,
        } => Ok(release(ctx, &rt, &root, &tag, since, draft, yes)),
    }
}

fn head_sha(root: &Path) -> Result<String, ForgeError> {
    Ok(gitflow::git(root, &["rev-parse", "HEAD"])?
        .trim()
        .to_owned())
}

/// `kor pr ci`: the checks of HEAD, the failing logs, and with `--fix` a task to fix them.
fn ci(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    fix: bool,
    yes: bool,
    json: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let (branch, sha) = match forge::current_branch(root).and_then(|b| Ok((b, head_sha(root)?))) {
        Ok(p) => p,
        Err(e) => return Ok(fail(l, &e)),
    };
    let f = match Forge::open(root) {
        Ok(f) => f,
        Err(e) => return Ok(fail(l, &e)),
    };
    let checks = match rt.block_on(f.checks(&sha)) {
        Ok(c) => c,
        Err(e) => return Ok(fail(l, &e)),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
        return Ok(!checks.iter().any(forge::Check::failed));
    }
    if checks.is_empty() {
        println!(
            "{}",
            l.t_args(
                "pr-ci-none",
                &args!("sha" => sha[..sha.len().min(12)].to_owned())
            )
        );
        return Ok(true);
    }
    for c in &checks {
        println!("{:<8} {}  {}", c.state, c.name, c.url);
    }
    let failed: Vec<_> = checks.into_iter().filter(forge::Check::failed).collect();
    if failed.is_empty() {
        println!("{}", l.t("pr-ci-green"));
        return Ok(true);
    }
    if !fix {
        println!(
            "{}",
            l.t_args("pr-ci-failed-hint", &args!("n" => failed.len()))
        );
        return Ok(false);
    }
    let with_logs: Vec<_> = failed
        .into_iter()
        .map(|c| {
            let log = rt.block_on(f.check_log(&c));
            (c, log)
        })
        .collect();
    let goal = forge::ci_goal(&branch, &sha, &with_logs);
    if !crate::shell_cmd::confirm(
        l,
        &l.t_args(
            "pr-ci-fix-confirm",
            &args!("n" => with_logs.len(), "branch" => branch),
        ),
        yes,
    ) {
        return Ok(false);
    }
    crate::task_cmd::run_goal(ctx, goal)
}

/// `kor pr issue`: an issue as a task (printed, or started with `--run`).
fn issue(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    target: &str,
    run: bool,
    yes: bool,
    json: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(num) = forge::pr_number(target) else {
        eprintln!(
            "{}",
            l.t_args("pr-bad-target", &args!("target" => target.to_owned()))
        );
        return Ok(false);
    };
    let f = match Forge::open(root) {
        Ok(f) => f,
        Err(e) => return Ok(fail(l, &e)),
    };
    let i = match rt.block_on(f.issue(num)) {
        Ok(i) => i,
        Err(e) => return Ok(fail(l, &e)),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&i)?);
        return Ok(true);
    }
    let goal = forge::issue_goal(&i);
    if !run {
        println!("{goal}");
        return Ok(true);
    }
    println!("#{} {}\n{}", i.number, i.title, i.url);
    if !crate::shell_cmd::confirm(
        l,
        &l.t_args("pr-issue-confirm", &args!("number" => i.number)),
        yes,
    ) {
        return Ok(false);
    }
    crate::task_cmd::run_goal(ctx, goal)
}

/// `kor pr release`: Keep a Changelog notes of `since..tag`, confirmed, then published.
fn release(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    tag: &str,
    since: Option<String>,
    draft: bool,
    yes: bool,
) -> bool {
    let l = &ctx.l;
    let sha = match gitflow::git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tag}^{{commit}}"),
        ],
    ) {
        Ok(s) if !tag.starts_with('-') => s.trim().to_owned(),
        _ => {
            eprintln!(
                "{}",
                l.t_args("pr-release-no-tag", &args!("tag" => tag.to_owned()))
            );
            return false;
        }
    };
    let since = since.or_else(|| {
        gitflow::git(
            root,
            &["describe", "--tags", "--abbrev=0", &format!("{tag}^")],
        )
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    });
    let entries = match gitflow::log_until(root, since.as_deref(), tag) {
        Ok(e) => e,
        Err(e) => return fail(l, &e.into()),
    };
    let notes = forge::release_notes(&entries);
    let f = match Forge::open(root) {
        Ok(f) => f,
        Err(e) => return fail(l, &e),
    };
    println!("{tag}\n\n{notes}\n");
    if !crate::shell_cmd::confirm(
        l,
        &l.t_args(
            "pr-release-confirm",
            &args!("tag" => tag.to_owned(), "forge" => f.remote.kind.id()),
        ),
        yes,
    ) {
        return false;
    }
    match rt.block_on(f.publish_release(tag, &sha, &notes, draft)) {
        Ok(url) => {
            println!("{}", l.t_args("pr-release-published", &args!("url" => url)));
            true
        }
        Err(e) => fail(l, &e),
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn create(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    base: Option<String>,
    draft: bool,
    title: Option<String>,
    yes: bool,
    model_arg: Option<&str>,
) -> bool {
    let l = &ctx.l;
    let branch = match forge::current_branch(root) {
        Ok(name) => name,
        Err(e) => return fail(l, &e),
    };
    let base = base.unwrap_or_else(|| forge::default_base(root));
    if branch == base {
        eprintln!("{}", l.t_args("pr-same-branch", &args!("branch" => branch)));
        return false;
    }
    // Forge and token first: nothing is pushed for a PR that cannot be opened.
    let client = match Forge::open(root) {
        Ok(client) => client,
        Err(e) => return fail(l, &e),
    };
    let provider = model(ctx, model_arg);
    let described = rt.block_on(forge::describe(
        provider.as_ref().map(|(prov, id)| (prov, id.as_str())),
        root,
        &base,
        &branch,
    ));
    let (mut desc, model_err) = match described {
        Ok(pair) => pair,
        Err(e) => return fail(l, &e),
    };
    if let Some(err) = model_err {
        eprintln!(
            "{}",
            l.t_args("pr-description-fallback", &args!("error" => err))
        );
    }
    if let Some(custom) = title.filter(|t| !t.trim().is_empty()) {
        custom.trim().clone_into(&mut desc.title);
    }
    // §27.14: stale marked diagrams are regenerated and committed with the PR (ADR 0063).
    let diagrams = crate::diagram_cmd::sync::sync_for_pr(root);
    if !diagrams.is_empty() {
        desc.body.push_str("\n\n### Diagrams\n");
        for d in &diagrams {
            let _ = writeln!(
                desc.body,
                "- {}",
                l.t_args("diagram-updated", &args!("summary" => d.clone()))
            );
        }
    }
    println!(
        "{}\n\n{}\n\n{}\n",
        l.t_args(
            "pr-proposed",
            &args!("head" => branch.clone(), "base" => base.clone(), "forge" => client.remote.kind.id())
        ),
        desc.title,
        desc.body
    );
    let first_push = !forge::has_upstream(root, &branch);
    let question = if first_push {
        l.t_args("pr-push-consent", &args!("branch" => branch.clone()))
    } else {
        l.t("pr-create-confirm")
    };
    if !crate::shell_cmd::confirm(l, &question, yes) {
        return false;
    }
    eprintln!(
        "{}",
        l.t_args("pr-pushing", &args!("branch" => branch.clone()))
    );
    if let Err(e) = forge::push(root, &branch) {
        return fail(l, &e);
    }
    let req = PrRequest {
        head: branch,
        base,
        title: desc.title,
        body: desc.body,
        draft,
    };
    match rt.block_on(client.create(&req)) {
        Ok(created) => {
            println!(
                "{}",
                l.t_args(
                    "pr-created",
                    &args!("number" => created.number, "url" => created.url)
                )
            );
            true
        }
        Err(e) => fail(l, &e),
    }
}

fn persona(l: &Localizer, root: &Path, name: Option<&str>) -> Option<review::Persona> {
    let name = name.unwrap_or(review::DEFAULT_PERSONA);
    let p = review::persona(root, name);
    if p.is_none() {
        let all: Vec<String> = review::personas(root).into_iter().map(|p| p.name).collect();
        eprintln!(
            "{}",
            l.t_args(
                "review-unknown-persona",
                &args!("name" => name.to_owned(), "list" => all.join(", "))
            )
        );
    }
    p
}

pub(crate) fn pr_review(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    target: &str,
    persona_name: Option<&str>,
    yes: bool,
    model_arg: Option<&str>,
) -> bool {
    let l = &ctx.l;
    let (Some(num), Some(chosen)) = (number(l, target), persona(l, root, persona_name)) else {
        return false;
    };
    let client = match Forge::open(root) {
        Ok(client) => client,
        Err(e) => return fail(l, &e),
    };
    let diff = match rt.block_on(client.diff(num)) {
        Ok(d) => d,
        Err(e) => return fail(l, &e),
    };
    let provider = model(ctx, model_arg);
    let report = rt.block_on(review::review(
        l,
        provider.as_ref().map(|(prov, id)| (prov, id.as_str())),
        &chosen,
        &diff,
    ));
    println!("{}", review::markdown(l, &report));
    if !crate::shell_cmd::confirm(
        l,
        &l.t_args("pr-review-confirm", &args!("number" => num)),
        yes,
    ) {
        return false;
    }
    let (body, notes) = review::to_notes(l, &report);
    match rt.block_on(client.post_review(num, &body, &notes)) {
        Ok(posted) => {
            println!(
                "{}",
                l.t_args(
                    "pr-review-posted",
                    &args!("inline" => posted.inline, "folded" => posted.folded)
                )
            );
            true
        }
        Err(e) => fail(l, &e),
    }
}

/// `[a]ccept / [e]dit / [r]eject` for one proposal; the edited text when `e`.
fn decide(l: &Localizer, p: &kor_agent::Proposal, yes: bool) -> Option<String> {
    let after = p.after.clone().unwrap_or_default();
    let diff = kor_agent::port::unified_diff(&p.path, p.before.as_deref().unwrap_or(""), &after);
    println!("{diff}");
    if yes {
        return Some(after);
    }
    print!(
        "{} ",
        l.t_args("perm-file", &args!("path" => p.path.clone()))
    );
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return None;
    }
    match line.trim() {
        "a" | "A" => Some(after),
        "e" | "E" => match crate::tui::review::edit_in_editor(&p.path, &after) {
            Ok(text) => Some(text),
            Err(error) => {
                eprintln!(
                    "{}",
                    l.t_args("perm-editor-failed", &args!("error" => error))
                );
                None
            }
        },
        _ => None,
    }
}

fn respond(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    root: &Path,
    target: &str,
    yes: bool,
    model_arg: Option<&str>,
) -> bool {
    let l = &ctx.l;
    let Some(num) = number(l, target) else {
        return false;
    };
    let client = match Forge::open(root) {
        Ok(client) => client,
        Err(e) => return fail(l, &e),
    };
    let fetched = rt.block_on(async {
        Ok::<_, ForgeError>((client.get(num).await?, client.comments(num).await?))
    });
    let (pr, comments) = match fetched {
        Ok(both) => both,
        Err(e) => return fail(l, &e),
    };
    let branch = match forge::current_branch(root) {
        Ok(name) => name,
        Err(e) => return fail(l, &e),
    };
    if branch != pr.head {
        eprintln!(
            "{}",
            l.t_args("pr-respond-wrong-branch", &args!("head" => pr.head.clone()))
        );
        return false;
    }
    if comments.is_empty() {
        println!("{}", l.t("pr-respond-none"));
        return true;
    }
    let Some((provider, model_id)) = model(ctx, model_arg) else {
        return false;
    };
    let resp = match rt.block_on(forge::respond::draft(
        &provider, &model_id, root, &pr, &comments,
    )) {
        Ok(drafted) => drafted,
        Err(e) => return fail(l, &e),
    };
    // ADR 0030: every file is shown and decided before anything is written.
    let mut accepted = Vec::new();
    for proposal in &resp.edits {
        if let Some(text) = decide(l, proposal, yes) {
            accepted.push((proposal.path.clone(), text));
        }
    }
    println!("{}", l.t("pr-respond-replies-title"));
    for reply in &resp.replies {
        let comment = comments.iter().find(|c| c.id == reply.id);
        let at = comment
            .and_then(|c| {
                c.path
                    .as_ref()
                    .map(|path| format!("{path}:{}", c.line.unwrap_or(0)))
            })
            .unwrap_or_default();
        println!(
            "\n> {} {at}\n{}",
            comment.map_or("", |c| c.body.as_str()).replace('\n', " "),
            reply.body
        );
    }
    let question = l.t_args(
        "pr-respond-confirm",
        &args!("files" => accepted.len(), "replies" => resp.replies.len(), "number" => num),
    );
    if !crate::shell_cmd::confirm(l, &question, yes) {
        return false;
    }
    if !accepted.is_empty() {
        let files: Vec<String> = accepted.iter().map(|(path, _)| path.clone()).collect();
        let done = forge::respond::write(root, &accepted)
            .and_then(|()| {
                forge::respond::commit(
                    root,
                    &files,
                    &format!("fix: address review comments on #{num}"),
                )
            })
            .and_then(|commit| forge::push(root, &branch).map(|_| commit));
        match done {
            Ok(commit) => println!("{commit}"),
            Err(e) => return fail(l, &e),
        }
    }
    let mut sent = 0usize;
    for reply in &resp.replies {
        if let Some(comment) = comments.iter().find(|c| c.id == reply.id) {
            match rt.block_on(client.reply(num, comment, &reply.body)) {
                Ok(()) => sent += 1,
                Err(e) => {
                    fail(l, &e);
                }
            }
        }
    }
    println!(
        "{}",
        l.t_args("pr-respond-done", &args!("count" => sent, "number" => num))
    );
    sent == resp.replies.len()
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::PrCmd;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        pr: PrCmd,
    }

    #[test]
    fn parses() {
        Cli::command().debug_assert();
        let c = Cli::try_parse_from(["pr", "create", "--base", "dev", "--draft", "--title", "T"])
            .unwrap();
        assert!(matches!(c.pr, PrCmd::Create { draft: true, base: Some(b), .. } if b == "dev"));
        let c = Cli::try_parse_from(["pr", "review", "12", "--persona", "security"]).unwrap();
        assert!(matches!(c.pr, PrCmd::Review { target, persona: Some(_), .. } if target == "12"));
        assert!(Cli::try_parse_from(["pr", "respond", "https://x/pull/3", "-y"]).is_ok());
        assert!(Cli::try_parse_from(["pr", "status", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["pr", "list"]).is_ok());
        assert!(Cli::try_parse_from(["pr", "review"]).is_err());
    }
}
