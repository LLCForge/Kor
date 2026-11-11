//! One trigger run: fresh worktree → the normal agent loop (trigger model, budget, approval
//! policy; approvals go out as notifications and pause the run) → transcript and cost saved →
//! optionally commit on a `kor/…` branch, push and open a pull request (`forge`) → notify.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::{Agent, AgentEvent, Approver, PermissionReply, PermissionRequest, StopReason};
use kor_config::{Approval, Trigger};
use kor_core::{ContentPart, Role};
use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;

use super::notify::{Ask, Hub, Note, Notifier, broadcast};
use super::{Defined, Job, Status, save_job};
use crate::forge::{PrRequest, create_pull_request};

/// An agent ready to run, with the session's cost control and transcript file when it has them.
pub struct Assembled {
    pub agent: Agent,
    pub cost: Option<Arc<crate::cost::Control>>,
    pub transcript: Option<PathBuf>,
}

impl std::fmt::Debug for Assembled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assembled")
            .field("transcript", &self.transcript)
            .finish_non_exhaustive()
    }
}

/// Builds the agent for a run in `dir` (the worktree). The daemon uses [`Prepare`]; tests
/// script the model.
#[async_trait]
pub trait AgentFactory: Send + Sync {
    async fn build(
        &self,
        dir: &Path,
        project: &Path,
        t: &Trigger,
        job: &str,
    ) -> Result<Assembled, String>;
}

/// The normal runtime ([`crate::prepare`]) with the project's config, the trigger's model, MCP
/// servers attached. Notifier secrets never reach agent commands.
#[derive(Debug)]
pub struct Prepare {
    pub lang: kor_i18n::Lang,
}

#[async_trait]
impl AgentFactory for Prepare {
    async fn build(
        &self,
        dir: &Path,
        project: &Path,
        t: &Trigger,
        job: &str,
    ) -> Result<Assembled, String> {
        let l = Localizer::new(self.lang);
        let paths = kor_config::Paths::discover(project);
        let loaded = kor_config::load(&paths, std::env::vars(), &[]).map_err(|e| e.to_string())?;
        let opts = crate::Options {
            root: dir.to_owned(),
            model: (!t.model.trim().is_empty()).then(|| t.model.trim().to_owned()),
            mode: None,
            resume: None,
            new_id: Some(format!("trigger-{job}")),
            surface: kor_agent::prompt::Surface::default(),
        };
        let mut notes = Vec::new();
        let mut p = crate::prepare(&l, &loaded, &opts, &mut notes)?;
        p.agent
            .ctx
            .env
            .retain(|(k, _)| !super::notify::NOTIFY_SECRETS.contains(&k.as_str()));
        p.agent
            .ctx
            .scoped_env
            .retain(|(k, _, _)| !super::notify::NOTIFY_SECRETS.contains(&k.as_str()));
        let _ = crate::attach_mcp(&l, &mut p.agent, &p.compat).await;
        Ok(Assembled {
            agent: p.agent,
            cost: Some(p.cost),
            transcript: p.path,
        })
    }
}

/// What a run needs from the daemon.
pub struct Env {
    pub l: Localizer,
    /// `<kor home>/daemon` (jobs, worktrees).
    pub dir: PathBuf,
    pub hub: Arc<Hub>,
    pub notifiers: Arc<Vec<Box<dyn Notifier>>>,
    /// `daemon.public_url`: base of approve links in Slack / e-mail.
    pub public_url: Option<String>,
    pub approval_timeout: Duration,
    pub factory: Arc<dyn AgentFactory>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    use kor_platform::NoWindow as _;
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .no_window()
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn clip(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// A short line-level change view for the Details button.
pub fn change_view(req: &PermissionRequest) -> String {
    let mut out = req.summary.clone();
    for f in &req.files {
        let _ = write!(out, "\n\n{}", f.path);
        let before: Vec<&str> = f.before.as_deref().unwrap_or_default().lines().collect();
        let after: Vec<&str> = f.after.as_deref().unwrap_or_default().lines().collect();
        if f.after.is_none() {
            out.push_str("\n(deleted)");
            continue;
        }
        for l in before.iter().filter(|l| !after.contains(l)).take(20) {
            let _ = write!(out, "\n- {l}");
        }
        for l in after.iter().filter(|l| !before.contains(l)).take(20) {
            let _ = write!(out, "\n+ {l}");
        }
    }
    clip(&out, 3500)
}

/// Answers permission requests of an unattended run by the trigger's policy; asks over the
/// notifiers (and the desktop / `kor triggers approve`) and waits otherwise.
struct DaemonApprover<'a> {
    env: &'a Env,
    policy: Approval,
    job: &'a Mutex<Job>,
}

impl DaemonApprover<'_> {
    fn set(&self, f: impl FnOnce(&mut Job)) {
        let mut j = self
            .job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut j);
        let _ = save_job(&self.env.dir, &j);
    }

    /// The answer and the channel it came from (`telegram`, `link`, `api`).
    async fn ask(&self, req: &PermissionRequest) -> (bool, Option<&'static str>) {
        let l = &self.env.l;
        let (job_id, trigger) = {
            let j = self
                .job
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (j.id.clone(), j.trigger.clone())
        };
        let summary = clip(&format!("{}: {}", req.tool, req.summary), 300);
        let Ok((id, token, rx)) = self.env.hub.open(&job_id, &summary, &change_view(req)) else {
            return (false, None);
        };
        self.set(|j| {
            j.status = Status::Waiting;
            j.approvals += 1;
        });
        let mut text = l.t_args(
            "trigger-approval-text",
            &args!("trigger" => trigger, "action" => summary.clone()),
        );
        if let Some(reason) = &req.destructive {
            text.push('\n');
            text.push_str(&l.t_args(
                "trigger-approval-destructive",
                &args!("reason" => reason.clone()),
            ));
        }
        let note = Note {
            title: l.t("trigger-approval-title"),
            text,
            ask: Some(Ask {
                id: id.clone(),
                link: self
                    .env
                    .public_url
                    .as_ref()
                    .map(|u| format!("{}/approve/{id}?t={token}", u.trim_end_matches('/'))),
                labels: [
                    l.t("trigger-approve"),
                    l.t("trigger-reject"),
                    l.t("trigger-details"),
                ],
            }),
        };
        for e in broadcast(&self.env.notifiers, &note).await {
            eprintln!("kor daemon: {e}");
        }
        let (approved, by) = match tokio::time::timeout(self.env.approval_timeout, rx).await {
            Ok(Ok((yes, by))) => (yes, Some(by)),
            _ => (false, None),
        };
        self.env.hub.forget(&id);
        kor_agent::audit::record(
            "trigger.approval",
            serde_json::json!({ "job": job_id, "tool": req.tool, "approved": approved, "by": by }),
        );
        self.set(|j| j.status = Status::Running);
        (approved, by)
    }
}

#[async_trait]
impl Approver for DaemonApprover<'_> {
    async fn approve(&self, req: &PermissionRequest) -> PermissionReply {
        self.review(req).await.reply
    }

    fn surface(&self) -> &'static str {
        "trigger"
    }

    async fn review(&self, req: &PermissionRequest) -> kor_agent::Review {
        let (ok, by) = match self.policy {
            Approval::Deny => (false, Some("policy")),
            Approval::Auto if req.destructive.is_none() => (true, Some("policy")),
            Approval::Auto | Approval::Notify => self.ask(req).await,
        };
        // Never "always": a trigger's answer covers one call.
        let mut r = kor_agent::Review::from(if ok {
            PermissionReply::Allow
        } else {
            PermissionReply::Deny
        });
        r.by = by;
        r
    }
}

/// What fired, for the prompt and the PR body.
#[derive(Debug, Clone)]
pub struct Fire {
    pub def: Defined,
    pub detail: String,
    /// Push branch (PR base) and pushed commit.
    pub base: Option<String>,
    pub sha: Option<String>,
}

/// A new queued job for `fire`.
///
/// # Errors
/// Without a random source.
pub fn new_job(fire: &Fire) -> Result<Job, String> {
    Ok(Job {
        id: super::new_job_id()?,
        trigger: fire.def.name.clone(),
        event: super::event_name(fire.def.trigger.event).to_owned(),
        detail: fire.detail.clone(),
        root: fire.def.root.clone(),
        status: Status::Queued,
        created: super::now_ms(),
        base: fire.base.clone(),
        sha: fire.sha.clone(),
        ..Job::default()
    })
}

fn prompt(fire: &Fire) -> String {
    let t = &fire.def.trigger;
    let mut p = format!(
        "{}\n\n[Kor trigger `{}` ({}): {}]\nYou run unattended in a fresh git worktree of the project. Do the task, verify it, then stop with a short summary of what you changed and why.",
        t.task.trim(),
        fire.def.name,
        super::event_name(t.event),
        fire.detail
    );
    if t.pr {
        p.push_str(" Do not commit, push or open pull requests yourself: Kor commits your changes on a branch, pushes it and opens the pull request when you finish.");
    }
    p
}

fn last_text(agent: &Agent) -> String {
    agent
        .messages
        .iter()
        .rev()
        .find(|m| {
            m.role == Role::Assistant
                && m.parts
                    .iter()
                    .any(|p| matches!(p, ContentPart::Text { .. }))
        })
        .map(|m| {
            m.parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Fresh worktree at the pushed commit (or HEAD); `None` for a folder that is not a git repo.
fn worktree(env: &Env, job: &Job) -> Result<Option<PathBuf>, String> {
    let root = &job.root;
    if git(root, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Ok(None);
    }
    let rev = match &job.sha {
        Some(sha) => {
            if git(root, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_err() {
                let mut fetch = vec!["fetch", "--quiet", "origin"];
                if let Some(b) = &job.base {
                    fetch.push(b);
                }
                git(root, &fetch)?;
            }
            sha.clone()
        }
        None => "HEAD".into(),
    };
    let wt = env.dir.join("worktrees").join(&job.id);
    std::fs::create_dir_all(wt.parent().unwrap_or(&env.dir)).map_err(|e| e.to_string())?;
    git(
        root,
        &[
            "worktree",
            "add",
            "--detach",
            &wt.display().to_string(),
            &rev,
        ],
    )?;
    Ok(Some(super::canon(&wt).unwrap_or(wt)))
}

/// Commit everything in `wt` on a new branch, push it and open the PR.
async fn publish(
    env: &Env,
    fire: &Fire,
    job: &Job,
    wt: &Path,
) -> Result<(Option<String>, Option<String>), String> {
    if git(wt, &["status", "--porcelain"])?.is_empty() {
        return Ok((None, None));
    }
    let slug: String = fire
        .def
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let branch = format!(
        "kor/{slug}-{}",
        job.id.rsplit('-').next().unwrap_or(&job.id)
    );
    git(wt, &["checkout", "-b", &branch])?;
    git(wt, &["add", "-A"])?;
    let title = clip(
        &format!(
            "kor: {}",
            fire.def
                .trigger
                .task
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
        ),
        72,
    );
    let mut commit: Vec<&str> = Vec::new();
    if git(wt, &["config", "user.email"]).is_err() {
        commit.extend(["-c", "user.name=Kor", "-c", "user.email=kor@localhost"]);
    }
    commit.extend(["commit", "--quiet", "-m", &title]);
    git(wt, &commit)?;
    git(
        wt,
        &[
            "push",
            "--quiet",
            "origin",
            &format!("{branch}:refs/heads/{branch}"),
        ],
    )?;
    let base = match &job.base {
        Some(b) => b.clone(),
        None if !fire.def.trigger.branch.is_empty() => fire.def.trigger.branch.clone(),
        None => git(&job.root, &["rev-parse", "--abbrev-ref", "HEAD"])?,
    };
    let l = &env.l;
    let body = format!(
        "{}\n\n---\n{}\n",
        job.summary.trim(),
        l.t_args(
            "trigger-pr-footer",
            &args!(
                "trigger" => fire.def.name.clone(),
                "event" => job.event.clone(),
                "detail" => fire.detail.clone(),
                "task" => fire.def.trigger.task.trim().to_owned(),
                "cost" => crate::cost::fmt_usd(job.cost_usd, job.cost_estimated),
                "job" => job.id.clone()
            )
        )
    );
    let pr = create_pull_request(
        wt,
        &PrRequest {
            head: branch.clone(),
            base,
            title,
            body,
            draft: false,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok((Some(branch), Some(pr.url)))
}

/// Run `job` to the end. The job file is updated at every step (the checkpoint the desktop and
/// `kor triggers history` read); an interrupted daemon marks it failed on its next start.
#[allow(clippy::too_many_lines)] // one linear run; every step updates the stored job
pub async fn run(env: &Env, fire: &Fire, job: Job) -> Job {
    let l = &env.l;
    let shared = Mutex::new(job);
    let update = |f: &dyn Fn(&mut Job)| {
        let mut j = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut j);
        let _ = save_job(&env.dir, &j);
        j.clone()
    };
    let job = update(&|j| {
        j.status = Status::Running;
        j.started = Some(super::now_ms());
    });
    kor_agent::audit::record(
        "trigger.fire",
        serde_json::json!({ "job": job.id, "trigger": job.trigger, "event": job.event, "detail": clip(&job.detail, 200) }),
    );
    let result = async {
        let wt = worktree(env, &job)?;
        update(&|j| j.worktree.clone_from(&wt));
        let dir = wt.clone().unwrap_or_else(|| job.root.clone());
        let mut built = env
            .factory
            .build(&dir, &job.root, &fire.def.trigger, &job.id)
            .await?;
        update(&|j| j.transcript.clone_from(&built.transcript));
        let cancel = CancellationToken::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let budget = fire.def.trigger.budget_usd;
        let cost = built.cost.clone();
        let watch_cancel = cancel.clone();
        let over = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let over_w = over.clone();
        // Per-trigger budget: stop the run once this session's spend reaches it.
        let watcher = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let (AgentEvent::Usage(_), Some(c)) = (&ev, &cost)
                    && budget > 0.0
                    && c.session_rows().iter().map(|r| r.usd).sum::<f64>() >= budget
                {
                    over_w.store(true, std::sync::atomic::Ordering::Relaxed);
                    watch_cancel.cancel();
                }
            }
        });
        let approver = DaemonApprover {
            env,
            policy: fire.def.trigger.approval,
            job: &shared,
        };
        let stop = built.agent.send(prompt(fire), &tx, &approver, cancel).await;
        drop(tx);
        let _ = watcher.await;
        if let Some(p) = &built.transcript {
            let _ = kor_agent::session::save(p, &built.agent.messages);
        }
        let (usd, est) = built.cost.as_ref().map_or((0.0, false), |c| {
            let rows = c.session_rows();
            (
                rows.iter().map(|r| r.usd).sum(),
                rows.iter().any(|r| r.estimated || r.unpriced > 0),
            )
        });
        let summary = last_text(&built.agent);
        update(&|j| {
            j.cost_usd = usd;
            j.cost_estimated = est;
            j.summary = clip(&summary, 4000);
        });
        if over.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(l.t_args(
                "trigger-over-budget",
                &args!("usd" => format!("{budget:.2}")),
            ));
        }
        match stop.map_err(|e| crate::provider_error(l, &e))? {
            StopReason::Done(_) | StopReason::MaxTurns | StopReason::Budget => {}
            StopReason::Cancelled => return Err(l.t("trigger-cancelled")),
            StopReason::Blocked(why) => return Err(why),
        }
        if fire.def.trigger.pr
            && let Some(wt) = &wt
        {
            let snapshot = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let (branch, pr) = publish(env, fire, &snapshot, wt).await?;
            update(&|j| {
                j.branch.clone_from(&branch);
                j.pr_url.clone_from(&pr);
            });
        }
        if let Some(wt) = &wt {
            let _ = git(
                &job.root,
                &["worktree", "remove", "--force", &wt.display().to_string()],
            );
            update(&|j| j.worktree = None);
        }
        Ok::<(), String>(())
    }
    .await;
    let job = update(&|j| {
        j.finished = Some(super::now_ms());
        match &result {
            Ok(()) => j.status = Status::Done,
            Err(e) => {
                j.status = Status::Failed;
                j.error = Some(clip(e, 2000));
            }
        }
    });
    kor_agent::audit::record(
        "trigger.done",
        serde_json::json!({ "job": job.id, "trigger": job.trigger, "ok": result.is_ok(), "cost_usd": job.cost_usd, "pr": job.pr_url }),
    );
    let mut text = match &job.error {
        None => clip(&job.summary, 1500),
        Some(e) => l.t_args("trigger-failed-text", &args!("error" => e.clone())),
    };
    if let Some(pr) = &job.pr_url {
        let _ = write!(text, "\n{pr}");
    }
    let _ = write!(
        text,
        "\n{}",
        crate::cost::fmt_usd(job.cost_usd, job.cost_estimated)
    );
    let key = if job.error.is_none() {
        "trigger-done-title"
    } else {
        "trigger-failed-title"
    };
    let note = Note {
        title: l.t_args(key, &args!("trigger" => job.trigger.clone())),
        text,
        ask: None,
    };
    for e in broadcast(&env.notifiers, &note).await {
        eprintln!("kor daemon: {e}");
    }
    super::prune(&env.dir);
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use kor_agent::Proposal;

    #[test]
    fn details_show_changed_lines() {
        let req = PermissionRequest {
            id: "1".into(),
            tool: "Edit".into(),
            summary: "src/a.txt".into(),
            files: vec![Proposal {
                path: "src/a.txt".into(),
                before: Some("keep\n41\n".into()),
                after: Some("keep\n42\n".into()),
            }],
            destructive: None,
            rules: Vec::new(),
        };
        let v = change_view(&req);
        assert!(
            v.contains("- 41") && v.contains("+ 42") && !v.contains("keep"),
            "{v}"
        );
    }
}
