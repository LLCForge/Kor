//! System profile, health alerts and cache cleanup (spec §19.2–19.3, ADR 0059).
//!
//! - [`system`]: `<kor home>/system.json`, refreshed on start and every `health.refresh_minutes`,
//!   `system.changed` when stable facts change, a budgeted summary for the agent's prompt;
//! - [`health`]: threshold watcher (disk, memory, swap, Kor's processes, runaway children,
//!   temperature, battery, caches, Docker, Kor's own cache) and the write stop on a full disk;
//! - [`clean`]: `kor clean --suggest` and confirmed deletion of recognised caches only.

pub mod clean;
pub mod health;
pub mod system;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{PoisonError, RwLock};

static CONFIG: RwLock<Option<kor_config::Health>> = RwLock::new(None);

/// `[health]` for this process (set when a session is prepared; defaults until then).
pub fn set_config(h: kor_config::Health) {
    *CONFIG.write().unwrap_or_else(PoisonError::into_inner) = Some(h);
}

pub fn config() -> kor_config::Health {
    CONFIG
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// Whether this process enforces the full-disk write stop (§19.3). Off until a surface turns it
/// on (`kor_runtime::prepare` does, for the CLI, TUI and desktop): an agent built directly, as in
/// the agent crate's tests, never depends on the real disk of the machine running it.
static WRITE_GUARD: AtomicBool = AtomicBool::new(false);

/// Turn the full-disk write stop on for this process.
pub fn enforce_write_guard() {
    WRITE_GUARD.store(true, Ordering::Relaxed);
}

/// [`health::write_block`] with this process's `[health]`, when the guard is on.
pub fn write_block_here(root: &Path) -> Option<String> {
    if !WRITE_GUARD.load(Ordering::Relaxed) {
        return None;
    }
    health::write_block(root, &config())
}

/// Kor's home (`KOR_HOME`, else the local data folder), as `kor_runtime::kor_home`.
pub fn kor_home() -> Option<PathBuf> {
    std::env::var_os("KOR_HOME")
        .map(PathBuf::from)
        .or_else(kor_platform::data_dir)
}

/// The prompt section from the stored profile (no probing: start-up stays fast). When the profile
/// is missing or stale a refresh runs in the background for the next session.
pub fn prompt_section(home: &Path, refresh_minutes: u32, max_chars: usize) -> String {
    if system::stale(home, refresh_minutes) {
        let home = home.to_owned();
        std::thread::spawn(move || {
            let _ = system::refresh(&home, &system::RealProbe);
        });
    }
    system::load(home).map_or_else(String::new, |p| system::prompt_section(&p, max_chars))
}

/// A full health check of the real machine for one surface run (CLI task start, `kor system
/// health`, desktop timer): takes two CPU readings ~250 ms apart; measures caches under `project`
/// and Docker when `deep`.
pub fn check_now(
    t: &kor_config::Health,
    project: Option<&Path>,
    deep: bool,
    l: &kor_i18n::Localizer,
) -> Vec<health::Alert> {
    let mut s = health::Sampler::new();
    let _ = s.sample(t);
    std::thread::sleep(std::time::Duration::from_millis(250));
    let mut sample = s.sample(t);
    if deep {
        fill_slow(&mut sample, project);
    }
    health::evaluate(&sample, t, project, l)
}

/// One line for a terminal task start (§19.3): the most serious of the fast checks (disks,
/// memory, battery; no CPU sampling, no cache walk), `None` when all is well.
pub fn quick_line(
    t: &kor_config::Health,
    project: &Path,
    l: &kor_i18n::Localizer,
) -> Option<String> {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let sample = health::Sample {
        disks: system::disks(&sysinfo::Disks::new_with_refreshed_list()),
        memory: system::Memory {
            total_mb: sys.total_memory() / (1024 * 1024),
            available_mb: sys.available_memory() / (1024 * 1024),
            swap_total_mb: sys.total_swap() / (1024 * 1024),
            swap_used_mb: sys.used_swap() / (1024 * 1024),
        },
        battery: kor_platform::desktop_probe::battery(),
        ..health::Sample::default()
    };
    let mut alerts = health::evaluate(&sample, t, Some(project), l);
    alerts.sort_by(|a, b| b.level.cmp(&a.level));
    alerts.first().map(|a| {
        l.t_args(
            "health-cli-line",
            &kor_i18n::args!("title" => a.title.clone(), "more" => alerts.len() - 1),
        )
    })
}

/// Caches, Docker and Kor's cache size (seconds, not milliseconds: run off the UI thread).
pub fn fill_slow(sample: &mut health::Sample, project: Option<&Path>) {
    let scan = clean::Scan {
        docker: false,
        ..clean::Scan::standard(health::cache_roots(project))
    };
    sample.caches = clean::suggest(&scan);
    sample.docker_bytes = clean::docker_usage().map(|(total, _)| total);
    sample.kor_cache_bytes = kor_home().map_or(0, |h| health::kor_cache_bytes(&h));
}

#[cfg(test)]
mod tests {
    #[test]
    fn write_guard_is_off_until_a_surface_turns_it_on() {
        // A full disk can't be faked here; what matters is that nothing is consulted before the opt-in.
        let root = std::env::temp_dir();
        assert_eq!(super::write_block_here(&root), None);
        super::enforce_write_guard();
        assert!(super::WRITE_GUARD.load(std::sync::atomic::Ordering::Relaxed));
    }
}
