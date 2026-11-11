//! How busy this machine is, for the Kor-to-Kor peer list and "run on the least busy machine"
//! (spec §31.1, ADR 0057): logical CPUs and the share of them in use. Linux `/proc/loadavg`,
//! macOS `sysctl -n vm.loadavg` (1-minute load average ÷ CPUs), Windows `GetSystemTimes` sampled
//! over 250 ms (busy share of all CPUs). `None` when the OS does not say.

/// Logical CPUs.
pub fn cpus() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// Share of the CPUs in use, `0.0` idle … `1.0` all busy (load averages can exceed 1).
pub fn busy() -> Option<f64> {
    imp::busy()
}

/// `0.52 0.40 0.31 1/123 4567` → 0.52.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn first_load(text: &str) -> Option<f64> {
    text.trim()
        .trim_start_matches('{')
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
mod imp {
    #[allow(clippy::cast_precision_loss)] // CPU counts are small
    pub fn busy() -> Option<f64> {
        let load = super::first_load(&std::fs::read_to_string("/proc/loadavg").ok()?)?;
        Some(load / super::cpus() as f64)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    #[allow(clippy::cast_precision_loss)] // CPU counts are small
    pub fn busy() -> Option<f64> {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output()
            .ok()?;
        let load = super::first_load(&String::from_utf8_lossy(&out.stdout))?;
        Some(load / super::cpus() as f64)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;

    fn ticks(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }

    fn sample() -> Option<(u64, u64)> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut idle, mut kernel, mut user) = (zero, zero, zero);
        // SAFETY: three valid, writable FILETIME out-pointers.
        let ok = unsafe { GetSystemTimes(&raw mut idle, &raw mut kernel, &raw mut user) };
        // Kernel time includes idle time.
        (ok != 0).then(|| (ticks(idle), ticks(kernel) + ticks(user)))
    }

    #[allow(clippy::cast_precision_loss)] // tick deltas over 250 ms are small
    pub fn busy() -> Option<f64> {
        let (i0, t0) = sample()?;
        std::thread::sleep(std::time::Duration::from_millis(250));
        let (i1, t1) = sample()?;
        let total = t1.checked_sub(t0)?;
        if total == 0 {
            return None;
        }
        let idle = i1.saturating_sub(i0).min(total);
        Some((total - idle) as f64 / total as f64)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    pub fn busy() -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_load_lines_and_reports_a_share() {
        assert_eq!(first_load("0.52 0.40 0.31 1/123 4567\n"), Some(0.52));
        assert_eq!(first_load("{ 1.25 1.10 0.90 }"), Some(1.25));
        assert_eq!(first_load(""), None);
        assert!(cpus() >= 1);
        if let Some(b) = busy() {
            assert!(b >= 0.0, "{b}");
        }
    }
}
