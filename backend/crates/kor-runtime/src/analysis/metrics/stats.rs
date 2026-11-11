//! `kor stats [--week]`: the developer's weekly numbers from git, Kor's task records and the
//! audit log. Lines are split three ways:
//! - **agent**: commits Kor made (author `kor@localhost`, or a commit listed in a `kor task`
//!   checkpoint under `.kor/tasks/`);
//! - **with agent edits**: lines of the developer's own commits in files the agent wrote that
//!   week (audit log `tool` entries of kind `write`);
//! - **human**: the rest of the developer's commits.
//!
//! Fixes and features come from Conventional Commit types, the coverage change from `kor quality`
//! snapshots. Everything stays on this machine (§9.1: visible only to the user); the team ranking
//! appears only with `[stats] team = true`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::git::{self, Commit, LogOpts};

pub const AGENT_EMAIL: &str = "kor@localhost";

static CONVENTIONAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z]+)(?:\([^)]*\))?!?:").expect("valid regex"));

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lines {
    pub added: u64,
    pub deleted: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Week {
    /// Monday 00:00 local time (Unix seconds).
    pub start: i64,
    pub end: i64,
    pub commits: u32,
    pub human: Lines,
    pub assisted: Lines,
    pub agent: Lines,
    pub agent_commits: u32,
    /// Successful agent file writes in this project (audit log).
    pub agent_writes: u32,
    pub fixes: u32,
    pub features: u32,
    /// Other Conventional Commit types.
    pub types: BTreeMap<String, u32>,
    pub active_days: u32,
    /// Line coverage at the first and last `kor quality` snapshot of the week.
    pub coverage: Option<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamRow {
    pub author: String,
    pub commits: u32,
    pub added: u64,
    pub deleted: u64,
    pub fixes: u32,
    pub features: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub author: String,
    /// Newest first.
    pub weeks: Vec<Week>,
    /// Consecutive days with a commit, ending today (or yesterday when today has none yet).
    pub streak: u32,
    pub longest_streak: u32,
    pub sources: Vec<String>,
    /// Only with `[stats] team = true`.
    pub team: Option<Vec<TeamRow>>,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Default: `git config user.email`.
    pub author: Option<String>,
    /// Weeks back including the current one (1 = this week).
    pub weeks: u32,
    /// `[stats] team = true`.
    pub team: bool,
    /// The audit log (`<kor home>/audit.jsonl`).
    pub audit: Option<std::path::PathBuf>,
}

fn week_start(now: chrono::DateTime<Local>) -> i64 {
    let monday =
        now.date_naive() - chrono::Days::new(u64::from(now.weekday().num_days_from_monday()));
    local_midnight(monday)
}

fn local_midnight(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .and_then(|t| Local.from_local_datetime(&t).earliest())
        .map_or(0, |t| t.timestamp())
}

fn day_of(ts: i64) -> Option<NaiveDate> {
    Local.timestamp_opt(ts, 0).single().map(|t| t.date_naive())
}

/// Conventional Commit type of a subject, lowercase.
pub fn commit_type(subject: &str) -> Option<String> {
    CONVENTIONAL
        .captures(subject.trim())
        .map(|c| c[1].to_lowercase())
}

/// Commits made by Kor's multi-agent tasks (full or abbreviated hashes).
fn task_commits(root: &Path) -> BTreeSet<String> {
    crate::task::list(root)
        .into_iter()
        .flat_map(|t| t.subtasks.into_iter().filter_map(|s| s.commit))
        .collect()
}

struct Write {
    ts: i64,
    target: String,
}

fn same_root(a: &str, b: &Path) -> bool {
    let norm = |s: &str| {
        s.trim_start_matches(r"\\?\")
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    let b = b.canonicalize().unwrap_or_else(|_| b.to_owned());
    norm(a) == norm(&b.to_string_lossy())
}

/// Successful agent writes in `root` from the audit log.
fn agent_writes(audit: &Path, root: &Path, since: i64) -> Vec<Write> {
    let mut out = Vec::new();
    let _ = kor_agent::audit::each(audit, |v| {
        if v["action"] != "tool" || v["kind"] != "write" || v["ok"] == false {
            return;
        }
        let Some(ts) = v["ts"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp())
        else {
            return;
        };
        if ts < since || !v["root"].as_str().is_some_and(|r| same_root(r, root)) {
            return;
        }
        out.push(Write {
            ts,
            target: v["target"].as_str().unwrap_or_default().replace('\\', "/"),
        });
    });
    out
}

fn is_agent(c: &Commit, tasks: &BTreeSet<String>) -> bool {
    c.email == AGENT_EMAIL
        || tasks
            .iter()
            .any(|t| t.len() >= 7 && (c.hash.starts_with(t.as_str()) || t.starts_with(&c.hash)))
}

/// Longest run and the run ending today/yesterday over a set of active days.
fn streaks(days: &BTreeSet<NaiveDate>, today: NaiveDate) -> (u32, u32) {
    let mut longest = 0;
    let mut run = 0;
    let mut prev: Option<NaiveDate> = None;
    for d in days {
        run = match prev {
            Some(p) if p.succ_opt() == Some(*d) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(*d);
    }
    let mut cur = 0;
    let mut d = if days.contains(&today) {
        Some(today)
    } else {
        today.pred_opt()
    };
    while let Some(x) = d.filter(|x| days.contains(x)) {
        cur += 1;
        d = x.pred_opt();
    }
    (cur, longest)
}

/// The developer's statistics for the last `o.weeks` weeks.
#[allow(clippy::too_many_lines)] // week loop with one bucket per counter
pub fn stats(root: &Path, o: &Options) -> Result<Stats, String> {
    if !git::is_repo(root) {
        return Err("not a git repository".into());
    }
    let author = match &o.author {
        Some(a) => a.trim().to_lowercase(),
        None => git::user_email(root).ok_or("git user.email is not set; pass --author <email>")?,
    };
    let now = Local::now();
    let weeks = o.weeks.clamp(1, 52);
    let first = week_start(now) - i64::from(weeks - 1) * 7 * 86_400;
    let year_ago = now.timestamp() - 366 * 86_400;
    let all = git::log(
        root,
        &LogOpts {
            since: Some(first.min(year_ago)),
            ..LogOpts::default()
        },
    )?;
    let tasks = task_commits(root);
    let writes = o
        .audit
        .as_deref()
        .map(|a| agent_writes(a, root, first))
        .unwrap_or_default();
    let snaps = super::quality::history(root);
    let mut sources = vec![
        format!(
            "git log: {} commits since {}",
            all.len(),
            fmt_day(first.min(year_ago))
        ),
        format!("kor task records: {} agent commits", tasks.len()),
    ];
    sources.push(match &o.audit {
        Some(a) if a.exists() => format!("audit log: {} agent file writes", writes.len()),
        _ => "audit log: not found".into(),
    });
    sources.push(format!("kor quality snapshots: {}", snaps.len()));
    let mut out = Vec::new();
    for i in 0..weeks {
        let start = first + i64::from(i) * 7 * 86_400;
        let end = start + 7 * 86_400;
        let in_week = |t: i64| t >= start && t < end;
        let mut w = Week {
            start,
            end,
            commits: 0,
            human: Lines::default(),
            assisted: Lines::default(),
            agent: Lines::default(),
            agent_commits: 0,
            agent_writes: 0,
            fixes: 0,
            features: 0,
            types: BTreeMap::new(),
            active_days: 0,
            coverage: None,
        };
        let week_writes: Vec<&Write> = writes.iter().filter(|x| in_week(x.ts)).collect();
        w.agent_writes = u32::try_from(week_writes.len()).unwrap_or(u32::MAX);
        let mut days = BTreeSet::new();
        for c in all.iter().filter(|c| in_week(c.time)) {
            if is_agent(c, &tasks) {
                w.agent_commits += 1;
                w.agent.added += c.added();
                w.agent.deleted += c.deleted();
                continue;
            }
            if c.email != author {
                continue;
            }
            w.commits += 1;
            if let Some(d) = day_of(c.time) {
                days.insert(d);
            }
            for f in &c.files {
                let assisted = week_writes.iter().any(|x| x.target.ends_with(&f.path));
                let bucket = if assisted {
                    &mut w.assisted
                } else {
                    &mut w.human
                };
                bucket.added += u64::from(f.added.unwrap_or(0));
                bucket.deleted += u64::from(f.deleted.unwrap_or(0));
            }
            match commit_type(&c.subject).as_deref() {
                Some("fix") => w.fixes += 1,
                Some("feat") => w.features += 1,
                Some(t) => *w.types.entry(t.to_owned()).or_default() += 1,
                None => {}
            }
        }
        w.active_days = u32::try_from(days.len()).unwrap_or(u32::MAX);
        let cov: Vec<f64> = snaps
            .iter()
            .filter(|s| in_week(s.taken_at))
            .filter_map(|s| s.coverage)
            .collect();
        if let (Some(a), Some(b)) = (cov.first(), cov.last()) {
            w.coverage = Some((*a, *b));
        }
        out.push(w);
    }
    out.reverse();
    let active: BTreeSet<NaiveDate> = all
        .iter()
        .filter(|c| c.email == author && !is_agent(c, &tasks))
        .filter_map(|c| day_of(c.time))
        .collect();
    let (streak, longest_streak) = streaks(&active, now.date_naive());
    let team = o.team.then(|| team(&all, &tasks, first));
    Ok(Stats {
        author,
        weeks: out,
        streak,
        longest_streak,
        sources,
        team,
    })
}

fn team(all: &[Commit], tasks: &BTreeSet<String>, since: i64) -> Vec<TeamRow> {
    let mut rows: HashMap<String, TeamRow> = HashMap::new();
    for c in all
        .iter()
        .filter(|c| c.time >= since && !is_agent(c, tasks))
    {
        let r = rows.entry(c.email.clone()).or_insert_with(|| TeamRow {
            author: c.author.clone(),
            commits: 0,
            added: 0,
            deleted: 0,
            fixes: 0,
            features: 0,
        });
        r.commits += 1;
        r.added += c.added();
        r.deleted += c.deleted();
        match commit_type(&c.subject).as_deref() {
            Some("fix") => r.fixes += 1,
            Some("feat") => r.features += 1,
            _ => {}
        }
    }
    let mut v: Vec<TeamRow> = rows.into_values().collect();
    v.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then(b.added.cmp(&a.added))
            .then(a.author.cmp(&b.author))
    });
    v
}

pub fn fmt_day(ts: i64) -> String {
    day_of(ts)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::metrics::fixture;

    #[test]
    fn human_agent_and_assisted_lines() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fixture::init(d);
        fixture::write(d, "a.rs", "fn a() {}\nfn b() {}\n");
        fixture::commit(d, "feat: a", "Ayşe Dev", "ayse@example.com", 0);
        fixture::write(d, "b.rs", "fn c() {}\n");
        fixture::commit(d, "chore: agent work", "Kor", AGENT_EMAIL, 0);
        fixture::write(d, "c.rs", "fn d() {}\nfn e() {}\nfn f() {}\n");
        fixture::commit(d, "fix(c): thing", "Ayşe Dev", "ayse@example.com", 0);
        fixture::write(d, "z.rs", "fn z() {}\n");
        fixture::commit(d, "feat: z", "Can Dev", "can@example.com", 0);

        let home = tempfile::tempdir().unwrap();
        let audit = home.path().join("audit.jsonl");
        kor_agent::audit::append(
            &audit,
            "tool",
            serde_json::json!({"tool": "Edit", "kind": "write", "target": "c.rs", "decision": "allow",
                "root": d.canonicalize().unwrap().to_string_lossy(), "ok": true}),
        )
        .unwrap();

        let s = stats(
            d,
            &Options {
                weeks: 1,
                audit: Some(audit),
                ..Options::default()
            },
        )
        .unwrap();
        let w = &s.weeks[0];
        assert_eq!(w.commits, 2);
        assert_eq!(w.human.added, 2);
        assert_eq!(w.assisted.added, 3);
        assert_eq!((w.agent.added, w.agent_commits), (1, 1));
        assert_eq!(w.agent_writes, 1);
        assert_eq!((w.fixes, w.features), (1, 1));
        assert_eq!(s.streak, 1);
        assert!(s.team.is_none());

        let t = stats(
            d,
            &Options {
                weeks: 1,
                team: true,
                ..Options::default()
            },
        )
        .unwrap();
        let team = t.team.unwrap();
        assert_eq!(team.len(), 2);
        assert_eq!(team[0].author, "Ayşe Dev");
    }

    #[test]
    fn streak_counts_back_from_today() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let days: BTreeSet<NaiveDate> = [8, 9, 10, 1, 2, 3, 4]
            .into_iter()
            .map(|d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap())
            .collect();
        assert_eq!(streaks(&days, today), (3, 4));
        let yesterday_only: BTreeSet<NaiveDate> =
            [NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()].into();
        assert_eq!(streaks(&yesterday_only, today).0, 1);
        assert_eq!(commit_type("feat(ui)!: x").as_deref(), Some("feat"));
        assert_eq!(commit_type("Merge branch"), None);
    }
}
