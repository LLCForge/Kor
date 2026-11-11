//! Everything running on this machine, for `kor status`, the tray and the status window: desktop
//! chats (from the app's snapshot), `kor daemon` jobs, `kor remote-serve` tasks and the approval
//! queue over all of them.

use serde::Serialize;

use super::desktop::{self, Running};
use super::eta::{self, Eta, Sample};
use super::queue::{self, Group, Item};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonJob {
    pub id: String,
    pub trigger: String,
    pub status: String,
    pub root: String,
    pub started: Option<u64>,
    pub cost_usd: f64,
    pub cost_estimated: bool,
    pub eta: Option<Eta>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    pub running: bool,
    pub queued: u64,
    pub jobs: Vec<DaemonJob>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTask {
    pub id: String,
    pub state: String,
    pub project: String,
    pub prompt: String,
    pub started: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// The desktop app runs (fresh snapshot).
    pub desktop: bool,
    pub sessions: Vec<Running>,
    pub daemon: DaemonInfo,
    pub remote: Vec<RemoteTask>,
    pub approvals: Vec<Item>,
    pub groups: Vec<Group>,
    pub paused: bool,
    pub now: u64,
}

/// Durations of finished jobs as ETA samples keyed by trigger name.
pub fn job_samples(jobs: &[crate::triggers::Job]) -> Vec<Sample> {
    jobs.iter()
        .filter(|j| j.status == crate::triggers::Status::Done)
        .filter_map(|j| {
            let (s, f) = (j.started?, j.finished?);
            Some(Sample {
                key: j.trigger.clone(),
                secs: f.saturating_sub(s) / 1000,
                at: f,
            })
        })
        .collect()
}

pub fn daemon_jobs(jobs: &[crate::triggers::Job], now: u64) -> Vec<DaemonJob> {
    let samples = job_samples(jobs);
    jobs.iter()
        .filter(|j| {
            matches!(
                j.status,
                crate::triggers::Status::Running
                    | crate::triggers::Status::Waiting
                    | crate::triggers::Status::Queued
            )
        })
        .map(|j| DaemonJob {
            id: j.id.clone(),
            trigger: j.trigger.clone(),
            status: format!("{:?}", j.status).to_ascii_lowercase(),
            root: j.root.to_string_lossy().into_owned(),
            started: j.started,
            cost_usd: j.cost_usd,
            cost_estimated: j.cost_estimated,
            eta: j
                .started
                .and_then(|s| eta::estimate(&samples, &j.trigger, s, now)),
        })
        .collect()
}

/// Collect everything (reads files; asks a running daemon over its loopback control API).
pub async fn collect() -> Overview {
    let now = super::now_ms();
    let snap = super::dir().and_then(|d| desktop::read_live(&d, now));
    let mut daemon = DaemonInfo::default();
    if let Some(d) = crate::triggers::dir() {
        let st = crate::triggers::daemon::status(&d).await;
        daemon.running = st["running"] == true;
        daemon.queued = st["queued"].as_u64().unwrap_or(0);
        let hist = crate::triggers::history(&d, None, 200);
        daemon.jobs = daemon_jobs(&hist, now);
        if !daemon.running {
            // A stopped daemon runs nothing, whatever its last job files say.
            daemon.jobs.clear();
        }
    }
    let remote = crate::remote::TaskStore::standard()
        .map(|s| s.list())
        .unwrap_or_default()
        .into_iter()
        .filter(|t| matches!(t.state.as_str(), "queued" | "running" | "waiting"))
        .map(|t| RemoteTask {
            id: t.id,
            state: t.state,
            project: t.project,
            prompt: t.prompt,
            started: t.started,
        })
        .collect();
    let approvals = queue::collect().await;
    let groups = queue::groups(&approvals);
    Overview {
        desktop: snap.is_some(),
        paused: snap.as_ref().is_some_and(|s| s.paused),
        sessions: snap.map(|s| s.sessions).unwrap_or_default(),
        daemon,
        remote,
        approvals,
        groups,
        now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{Job, Status};

    fn job(id: &str, trigger: &str, status: Status, started: u64, finished: Option<u64>) -> Job {
        Job {
            id: id.into(),
            trigger: trigger.into(),
            status,
            started: Some(started),
            finished,
            ..Job::default()
        }
    }

    #[test]
    fn running_jobs_get_an_eta_from_finished_ones() {
        let jobs = vec![
            job("1", "ci", Status::Done, 0, Some(60_000)),
            job("2", "ci", Status::Done, 0, Some(120_000)),
            job("3", "ci", Status::Done, 0, Some(90_000)),
            job("4", "ci", Status::Failed, 0, Some(5_000)),
            job("5", "ci", Status::Running, 1_000_000, None),
        ];
        assert_eq!(job_samples(&jobs).len(), 3);
        let v = daemon_jobs(&jobs, 1_010_000);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].status, "running");
        let e = v[0].eta.as_ref().unwrap();
        assert_eq!(e.finish_ms, Some(1_000_000 + 90_000));
    }
}
