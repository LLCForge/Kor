//! Threshold watcher (§19.3): disk fill (the project's disk first), RAM / swap pressure, CPU and
//! memory of Kor and its children, runaway child processes, CPU temperature, critical battery,
//! huge build caches, Docker disk use and Kor's own cache / index size. Thresholds and muted kinds
//! come from `[health]`; everything is local, nothing is sent anywhere.
//!
//! [`evaluate`] is pure (a [`Sample`] in, alerts out); [`Sampler`] reads the real machine and
//! remembers since when each child process has been busy.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use kor_config::Health;
use kor_i18n::{Localizer, args};
use kor_platform::desktop_probe::Battery;
use serde::Serialize;

use crate::clean::{self, Candidate, human};
use crate::system::{self, Disk, Memory};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Warn,
    Critical,
}

/// One alert, ready for the notification card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    /// Stable for the condition (`disk:C:\`), used to show it once a day.
    pub id: String,
    /// `disk`, `memory`, `swap`, `kor`, `runaway`, `temperature`, `battery`, `cache`, `docker`,
    /// `kor_cache`.
    pub kind: String,
    pub level: Level,
    pub title: String,
    pub body: String,
    /// The card offers "clean up" (`kor clean --suggest`).
    pub clean: bool,
}

/// A process of Kor's tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    /// Percent of one core.
    pub cpu: f32,
    pub mem_mb: u64,
}

/// What the watcher looked at.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Sample {
    pub disks: Vec<Disk>,
    pub memory: Memory,
    pub battery: Option<Battery>,
    /// Kor and every process it started.
    pub kor: Vec<Proc>,
    /// Logical CPUs.
    pub cpus: usize,
    /// Children busy above `runaway_cpu` for at least `runaway_minutes`.
    pub runaway: Vec<Proc>,
    /// Sensor temperatures (°C).
    pub temps: Vec<f32>,
    /// Build / tool caches measured this time (may be empty: they are measured less often).
    pub caches: Vec<Candidate>,
    /// Docker's total disk use.
    pub docker_bytes: Option<u64>,
    /// Kor's own cache and index folders.
    pub kor_cache_bytes: u64,
}

/// The disk holding `path`: the longest mount point that is a prefix of it.
pub fn disk_of<'a>(disks: &'a [Disk], path: &Path) -> Option<&'a Disk> {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let s = p.to_string_lossy().replace("\\\\?\\", "").to_lowercase();
    disks
        .iter()
        .filter(|d| s.starts_with(&d.mount.to_lowercase()))
        .max_by_key(|d| d.mount.len())
}

const GB: u64 = 1024 * 1024 * 1024;

/// Alerts for `s` under thresholds `t`; the project's disk comes first.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn evaluate(s: &Sample, t: &Health, project: Option<&Path>, l: &Localizer) -> Vec<Alert> {
    if !t.enabled {
        return Vec::new();
    }
    let muted = |k: &str| t.muted.iter().any(|m| m == k);
    let mut out = Vec::new();
    let cleanable: Vec<&Candidate> = s.caches.iter().take(3).collect();
    let clean_hint = || {
        let mut parts: Vec<String> = cleanable
            .iter()
            .map(|c| {
                let label = c
                    .path
                    .as_ref()
                    .map_or_else(|| c.kind.clone(), |p| p.display().to_string());
                format!("{label} {}", human(c.bytes))
            })
            .collect();
        if let Some(b) = s.docker_bytes.filter(|b| *b >= GB) {
            parts.push(format!("Docker {}", human(b)));
        }
        parts.join(", ")
    };

    if !muted("disk") {
        let project_disk = project.and_then(|p| disk_of(&s.disks, p)).map(|d| d.mount.clone());
        let mut disks: Vec<&Disk> = s.disks.iter().filter(|d| !d.removable || Some(&d.mount) == project_disk.as_ref()).collect();
        disks.sort_by_key(|d| Some(&d.mount) != project_disk.as_ref());
        for d in disks {
            let used = d.used_percent();
            if used < t.disk_warn {
                continue;
            }
            let level = if used >= t.disk_critical { Level::Critical } else { Level::Warn };
            let hint = clean_hint();
            out.push(Alert {
                id: format!("disk:{}", d.mount),
                kind: "disk".into(),
                level,
                title: l.t_args("health-disk-title", &args!("disk" => d.mount.clone(), "percent" => u32::from(used))),
                body: if hint.is_empty() {
                    l.t_args("health-disk-body", &args!("free" => human(d.free_mb * 1024 * 1024)))
                } else {
                    l.t_args("health-disk-body-clean", &args!("free" => human(d.free_mb * 1024 * 1024), "items" => hint))
                },
                clean: true,
            });
        }
    }
    if s.memory.total_mb > 0 {
        let used = (s.memory.total_mb.saturating_sub(s.memory.available_mb)) * 100 / s.memory.total_mb;
        if !muted("memory") && used >= u64::from(t.memory_warn) {
            out.push(Alert {
                id: "memory".into(),
                kind: "memory".into(),
                level: if used >= 97 { Level::Critical } else { Level::Warn },
                title: l.t_args("health-memory-title", &args!("percent" => used)),
                body: l.t_args("health-memory-body", &args!("free" => human(s.memory.available_mb * 1024 * 1024))),
                clean: false,
            });
        }
    }
    if s.memory.swap_total_mb > 0 && !muted("swap") {
        let used = s.memory.swap_used_mb * 100 / s.memory.swap_total_mb;
        if used >= u64::from(t.swap_warn) {
            out.push(Alert {
                id: "swap".into(),
                kind: "swap".into(),
                level: Level::Warn,
                title: l.t_args("health-swap-title", &args!("percent" => used)),
                body: l.t("health-swap-body"),
                clean: false,
            });
        }
    }
    if !muted("kor") && !s.kor.is_empty() {
        let cpu: f32 = s.kor.iter().map(|p| p.cpu).sum::<f32>() / s.cpus.max(1) as f32;
        let mem: u64 = s.kor.iter().map(|p| p.mem_mb).sum();
        if cpu >= f32::from(t.kor_cpu_warn) || mem >= u64::from(t.kor_memory_mb) {
            let top = s
                .kor
                .iter()
                .max_by(|a, b| a.cpu.total_cmp(&b.cpu))
                .map(|p| format!("{} ({})", p.name, p.pid))
                .unwrap_or_default();
            out.push(Alert {
                id: "kor".into(),
                kind: "kor".into(),
                level: Level::Warn,
                title: l.t_args("health-kor-title", &args!("cpu" => cpu.round() as u32, "memory" => human(mem * 1024 * 1024))),
                body: l.t_args("health-kor-body", &args!("top" => top)),
                clean: false,
            });
        }
    }
    if !muted("runaway") {
        for p in &s.runaway {
            out.push(Alert {
                id: format!("runaway:{}", p.pid),
                kind: "runaway".into(),
                level: Level::Warn,
                title: l.t_args("health-runaway-title", &args!("name" => p.name.clone(), "pid" => p.pid)),
                body: l.t_args("health-runaway-body", &args!("cpu" => p.cpu.round() as u32, "minutes" => t.runaway_minutes)),
                clean: false,
            });
        }
    }
    if !muted("temperature")
        && let Some(max) = s.temps.iter().copied().filter(|v| v.is_finite()).reduce(f32::max)
        && max >= f32::from(t.temp_warn)
    {
        out.push(Alert {
            id: "temperature".into(),
            kind: "temperature".into(),
            level: Level::Warn,
            title: l.t_args("health-temp-title", &args!("celsius" => max.round() as u32)),
            body: l.t("health-temp-body"),
            clean: false,
        });
    }
    if !muted("battery")
        && let Some(b) = s.battery
        && !b.charging
        && b.percent <= t.battery_critical
    {
        out.push(Alert {
            id: "battery".into(),
            kind: "battery".into(),
            level: Level::Critical,
            title: l.t_args("health-battery-title", &args!("percent" => u32::from(b.percent))),
            body: l.t("health-battery-body"),
            clean: false,
        });
    }
    if !muted("cache") {
        let big: Vec<&Candidate> = s
            .caches
            .iter()
            .filter(|c| c.path.is_some() && c.bytes >= u64::from(t.cache_warn_gb) * GB)
            .collect();
        if !big.is_empty() {
            out.push(Alert {
                id: format!("cache:{}", big[0].id),
                kind: "cache".into(),
                level: Level::Warn,
                title: l.t_args("health-cache-title", &args!("size" => human(big.iter().map(|c| c.bytes).sum()))),
                body: big
                    .iter()
                    .take(3)
                    .map(|c| format!("{} {}{}", c.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(), if c.approx { "≥ " } else { "" }, human(c.bytes)))
                    .collect::<Vec<_>>()
                    .join(", "),
                clean: true,
            });
        }
    }
    if !muted("docker")
        && let Some(b) = s.docker_bytes
        && b >= u64::from(t.docker_warn_gb) * GB
    {
        out.push(Alert {
            id: "docker".into(),
            kind: "docker".into(),
            level: Level::Warn,
            title: l.t_args("health-docker-title", &args!("size" => human(b))),
            body: l.t("health-docker-body"),
            clean: true,
        });
    }
    if !muted("kor_cache") && s.kor_cache_bytes >= u64::from(t.kor_cache_warn_gb) * GB {
        out.push(Alert {
            id: "kor_cache".into(),
            kind: "kor_cache".into(),
            level: Level::Warn,
            title: l.t_args("health-kor-cache-title", &args!("size" => human(s.kor_cache_bytes))),
            body: l.t("health-kor-cache-body"),
            clean: true,
        });
    }
    out
}

/// Reads the real machine; keep one alive between samples (CPU use needs two readings).
#[derive(Debug)]
pub struct Sampler {
    sys: sysinfo::System,
    components: sysinfo::Components,
    busy_since: HashMap<u32, Instant>,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            sys: sysinfo::System::new(),
            components: sysinfo::Components::new_with_refreshed_list(),
            busy_since: HashMap::new(),
        }
    }

    /// Disks, memory, battery, Kor's processes, temperatures. Caches, Docker and Kor's cache are
    /// filled by the caller when it measures them (they are slow).
    pub fn sample(&mut self, t: &Health) -> Sample {
        use sysinfo::{Disks, ProcessRefreshKind, ProcessesToUpdate};
        self.sys.refresh_memory();
        self.sys.refresh_cpu_all();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        self.components.refresh(true);
        let me = sysinfo::get_current_pid().ok();
        let mut children: BTreeMap<sysinfo::Pid, Vec<sysinfo::Pid>> = BTreeMap::new();
        for (pid, p) in self.sys.processes() {
            if let Some(parent) = p.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }
        let mut tree = Vec::new();
        let mut stack: Vec<sysinfo::Pid> = me.into_iter().collect();
        while let Some(pid) = stack.pop() {
            if tree.len() > 2000 || tree.contains(&pid) {
                break;
            }
            tree.push(pid);
            stack.extend(children.get(&pid).into_iter().flatten().copied());
        }
        let kor: Vec<Proc> = tree
            .iter()
            .filter_map(|pid| {
                let p = self.sys.process(*pid)?;
                Some(Proc {
                    pid: pid.as_u32(),
                    name: p.name().to_string_lossy().into_owned(),
                    cpu: p.cpu_usage(),
                    mem_mb: p.memory() / (1024 * 1024),
                })
            })
            .collect();
        let now = Instant::now();
        let limit = Duration::from_secs(u64::from(t.runaway_minutes) * 60);
        self.busy_since.retain(|pid, _| kor.iter().any(|p| p.pid == *pid));
        let mut runaway = Vec::new();
        for p in kor.iter().skip(1) {
            if p.cpu >= f32::from(t.runaway_cpu) {
                let since = *self.busy_since.entry(p.pid).or_insert(now);
                if now.duration_since(since) >= limit {
                    runaway.push(p.clone());
                }
            } else {
                self.busy_since.remove(&p.pid);
            }
        }
        Sample {
            disks: system::disks(&Disks::new_with_refreshed_list()),
            memory: Memory {
                total_mb: self.sys.total_memory() / (1024 * 1024),
                available_mb: self.sys.available_memory() / (1024 * 1024),
                swap_total_mb: self.sys.total_swap() / (1024 * 1024),
                swap_used_mb: self.sys.used_swap() / (1024 * 1024),
            },
            battery: kor_platform::desktop_probe::battery(),
            kor,
            cpus: self.sys.cpus().len(),
            runaway,
            temps: self.components.iter().filter_map(sysinfo::Component::temperature).collect(),
            caches: Vec::new(),
            docker_bytes: None,
            kor_cache_bytes: 0,
        }
    }
}

/// Size of Kor's own cache and indexes: `<kor home>` without sessions and the audit log, plus the
/// separate cache folder where there is one.
pub fn kor_cache_bytes(home: &Path) -> u64 {
    let budget = Duration::from_secs(3);
    let mut total: u64 = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n != "sessions" && !n.starts_with("audit")
        })
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                clean::dir_size(&p, budget).0
            } else {
                e.metadata().map_or(0, |m| m.len())
            }
        })
        .sum();
    if let (Some(c), d) = (kor_platform::cache_dir(), kor_platform::data_dir())
        && d.as_ref() != Some(&c)
    {
        total += clean::dir_size(&c, budget).0;
    }
    total
}

/// Before a task writes (§19.3): `Some(message)` when the disk holding `root` is at or above
/// `health.disk_critical`; the agent stops writing tasks up front instead of half-writing.
/// Reads the disks at most every 30 s.
pub fn write_block(root: &Path, t: &Health) -> Option<String> {
    static CACHE: Mutex<Option<(Instant, Vec<Disk>)>> = Mutex::new(None);
    if !t.enabled || t.muted.iter().any(|m| m == "disk") {
        return None;
    }
    let disks = {
        let mut c = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        match &*c {
            Some((at, d)) if at.elapsed() < Duration::from_secs(30) => d.clone(),
            _ => {
                let d = system::disks(&sysinfo::Disks::new_with_refreshed_list());
                *c = Some((Instant::now(), d.clone()));
                d
            }
        }
    };
    block_message(&disks, root, t)
}

/// [`write_block`] without the disk reading (tests).
pub fn block_message(disks: &[Disk], root: &Path, t: &Health) -> Option<String> {
    let d = disk_of(disks, root)?;
    let used = d.used_percent();
    (used >= t.disk_critical).then(|| {
        format!(
            "Stopped before writing: the disk holding this project ({}) is {used}% full ({} free; `health.disk_critical` = {}%). Free space first (`kor clean --suggest`), then retry. Nothing was written.",
            d.mount,
            human(d.free_mb * 1024 * 1024),
            t.disk_critical
        )
    })
}

/// Project roots worth scanning for caches: `root` and nothing above the home folder.
pub fn cache_roots(root: Option<&Path>) -> Vec<PathBuf> {
    root.filter(|r| dirs::home_dir().as_deref() != Some(*r) && r.parent().is_some())
        .map(|r| vec![r.to_owned()])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l() -> Localizer {
        Localizer::new(kor_i18n::Lang::En)
    }

    fn disk(mount: &str, total: u64, free: u64) -> Disk {
        Disk { mount: mount.into(), total_mb: total, free_mb: free, ..Disk::default() }
    }

    #[test]
    fn thresholds_mutes_and_project_disk_first() {
        let t = Health::default();
        let d = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(d.path()).unwrap().to_string_lossy().replace("\\\\?\\", "");
        let s = Sample {
            disks: vec![disk("/aaa-other", 1000, 100), disk(&mount, 1000, 40)],
            memory: Memory { total_mb: 1000, available_mb: 50, swap_total_mb: 100, swap_used_mb: 90 },
            battery: Some(Battery { percent: 5, charging: false }),
            kor: vec![
                Proc { pid: 1, name: "kor".into(), cpu: 50.0, mem_mb: 100 },
                Proc { pid: 2, name: "cargo".into(), cpu: 700.0, mem_mb: 5000 },
            ],
            cpus: 8,
            runaway: vec![Proc { pid: 2, name: "cargo".into(), cpu: 700.0, mem_mb: 5000 }],
            temps: vec![55.0, 96.4, f32::NAN],
            caches: vec![Candidate { id: "abc".into(), kind: "cargo".into(), path: Some("/p/target".into()), bytes: 18 * GB, approx: false }],
            docker_bytes: Some(24 * GB),
            kor_cache_bytes: 3 * GB,
        };
        let a = evaluate(&s, &t, Some(d.path()), &l());
        let kinds: Vec<&str> = a.iter().map(|x| x.kind.as_str()).collect();
        assert_eq!(kinds, ["disk", "disk", "memory", "swap", "kor", "runaway", "temperature", "battery", "cache", "docker", "kor_cache"]);
        assert_eq!(a[0].id, format!("disk:{mount}"), "the project's disk comes first");
        assert_eq!(a[0].level, Level::Critical);
        assert_eq!(a[1].level, Level::Warn);
        assert!(a[0].body.contains("/p/target") && a[0].body.contains("Docker 24.0 GB"), "{}", a[0].body);
        assert!(a[0].clean && !a[2].clean);

        let quiet = Health { muted: kinds.iter().map(|k| (*k).to_owned()).collect(), ..Health::default() };
        assert!(evaluate(&s, &quiet, None, &l()).is_empty());
        let off = Health { enabled: false, ..Health::default() };
        assert!(evaluate(&s, &off, None, &l()).is_empty());
        assert!(evaluate(&Sample::default(), &t, None, &l()).is_empty());
    }

    #[test]
    fn writes_stop_on_a_critical_project_disk() {
        let d = tempfile::tempdir().unwrap();
        let mount = std::fs::canonicalize(d.path()).unwrap().to_string_lossy().replace("\\\\?\\", "");
        let t = Health::default();
        assert!(block_message(&[disk(&mount, 1000, 30)], d.path(), &t).unwrap().contains("97% full"));
        assert!(block_message(&[disk(&mount, 1000, 300)], d.path(), &t).is_none());
        assert!(block_message(&[], d.path(), &t).is_none());
    }

    #[test]
    fn sampler_reads_this_process() {
        let mut s = Sampler::new();
        let first = s.sample(&Health::default());
        assert!(first.memory.total_mb > 0 && first.cpus > 0);
        assert!(first.kor.first().is_some_and(|p| p.pid == std::process::id()));
    }
}
