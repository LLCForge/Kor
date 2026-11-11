//! This process's memory and CPU (§32.1 budgets, §32.3 overlay; ADR 0062): resident set now and
//! at its peak (bytes), CPU time, and the same summed over the processes it started (a desktop
//! app's `WebView`). Windows `GetProcessMemoryInfo` / `GetProcessTimes` / Toolhelp, Linux `/proc`,
//! macOS `getrusage` and `ps`.

use std::time::Duration;

/// Resident memory of this process now, bytes.
pub fn rss() -> Option<u64> {
    imp::rss()
}

/// Highest resident memory this process has had, bytes.
pub fn peak_rss() -> Option<u64> {
    imp::peak_rss()
}

/// CPU time (user + system) this process has used.
pub fn cpu_time() -> Option<Duration> {
    imp::cpu_time()
}

/// This process and every process it started (a desktop app's `WebView` processes): resident
/// memory, CPU time and how many processes were counted. On macOS `WebKit`'s XPC services are not
/// children of the app and are not counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tree {
    pub rss: u64,
    pub cpu: Duration,
    pub processes: usize,
}

pub fn tree() -> Option<Tree> {
    imp::tree()
}

/// `root` and its descendants in a `(pid, parent)` list.
#[cfg_attr(not(any(windows, unix)), allow(dead_code))]
fn descendants(root: u32, procs: &[(u32, u32)]) -> Vec<u32> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        for (c, parent) in procs {
            if *parent == p && *c != p && !out.contains(c) {
                out.push(*c);
            }
        }
        i += 1;
    }
    out
}

/// `ps` CPU time: `1:02.50`, `01:02:03.25`, `2-01:02:03` → duration.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn ps_time(t: &str) -> Option<Duration> {
    let (days, rest) = match t.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, t),
    };
    let mut secs = 0.0_f64;
    for part in rest.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(Duration::from_secs_f64(secs) + Duration::from_secs(days * 86_400))
}

/// `VmRSS:   12345 kB` → bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn status_kb(status: &str, key: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with(key))?;
    let kb: u64 = line[key.len()..]
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    kb.checked_mul(1024)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::time::Duration;

    fn field(key: &str) -> Option<u64> {
        super::status_kb(&std::fs::read_to_string("/proc/self/status").ok()?, key)
    }

    pub fn rss() -> Option<u64> {
        field("VmRSS:")
    }

    pub fn peak_rss() -> Option<u64> {
        field("VmHWM:")
    }

    /// `/proc/<pid>/stat`: (pid, ppid, utime + stime at 100 Hz).
    fn stat(pid: &str) -> Option<(u32, u32, Duration)> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Fields after the `(comm)`, which may hold spaces: ppid is the 4th, utime and stime the
        // 14th and 15th of the whole line.
        let rest = stat.rsplit_once(')')?.1;
        let f: Vec<&str> = rest.split_whitespace().collect();
        let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
        Some((
            pid.parse().ok()?,
            f.get(1)?.parse().ok()?,
            Duration::from_millis(ticks * 10),
        ))
    }

    /// `/proc/self/schedstat` (nanoseconds on a CPU) when the kernel has it, else `stat` ticks.
    pub fn cpu_time() -> Option<Duration> {
        if let Some(ns) = std::fs::read_to_string("/proc/self/schedstat")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok())
        {
            return Some(Duration::from_nanos(ns));
        }
        stat("self").map(|s| s.2)
    }

    pub fn tree() -> Option<super::Tree> {
        let all: Vec<(u32, u32, Duration)> = std::fs::read_dir("/proc")
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| stat(&e.file_name().to_string_lossy()))
            .collect();
        let pairs: Vec<(u32, u32)> = all.iter().map(|(p, pp, _)| (*p, *pp)).collect();
        let mine = super::descendants(std::process::id(), &pairs);
        let mut t = super::Tree::default();
        for (pid, _, cpu) in all.iter().filter(|(p, ..)| mine.contains(p)) {
            t.rss += std::fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|s| super::status_kb(&s, "VmRSS:"))
                .unwrap_or(0);
            t.cpu += *cpu;
            t.processes += 1;
        }
        Some(t)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn counters(h: HANDLE) -> Option<PROCESS_MEMORY_COUNTERS> {
        let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
        // SAFETY: a zeroed PROCESS_MEMORY_COUNTERS is a valid value and `cb` is its size; `h` is
        // a process handle with query rights.
        let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        c.cb = size;
        let ok = unsafe { GetProcessMemoryInfo(h, &raw mut c, size) };
        (ok != 0).then_some(c)
    }

    fn times(h: HANDLE) -> Option<Duration> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut create, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: four valid out pointers to FILETIMEs; `h` has query rights.
        let ok = unsafe {
            GetProcessTimes(
                h,
                &raw mut create,
                &raw mut exit,
                &raw mut kernel,
                &raw mut user,
            )
        };
        let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
        // FILETIME counts 100 ns units.
        (ok != 0).then(|| Duration::from_nanos((ticks(kernel) + ticks(user)) * 100))
    }

    fn me() -> HANDLE {
        // SAFETY: the pseudo handle of the current process is always valid and needs no closing.
        unsafe { GetCurrentProcess() }
    }

    pub fn rss() -> Option<u64> {
        counters(me()).and_then(|c| u64::try_from(c.WorkingSetSize).ok())
    }

    pub fn peak_rss() -> Option<u64> {
        counters(me()).and_then(|c| u64::try_from(c.PeakWorkingSetSize).ok())
    }

    pub fn cpu_time() -> Option<Duration> {
        times(me())
    }

    /// Every process as (pid, parent pid).
    fn processes() -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        // SAFETY: the snapshot handle is checked and closed; the entry's `dwSize` is set before
        // the first call.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0);
            let mut ok = Process32FirstW(snap, &raw mut e);
            while ok != 0 {
                out.push((e.th32ProcessID, e.th32ParentProcessID));
                ok = Process32NextW(snap, &raw mut e);
            }
            CloseHandle(snap);
        }
        out
    }

    #[allow(clippy::unnecessary_wraps)] // the same signature on every OS
    pub fn tree() -> Option<super::Tree> {
        let mine = super::descendants(std::process::id(), &processes());
        let mut t = super::Tree::default();
        for pid in mine {
            // SAFETY: the handle is checked before use and closed after.
            let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if h.is_null() {
                continue;
            }
            if let Some(c) = counters(h) {
                t.rss += u64::try_from(c.WorkingSetSize).unwrap_or(0);
            }
            t.cpu += times(h).unwrap_or_default();
            t.processes += 1;
            unsafe { CloseHandle(h) };
        }
        Some(t)
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use std::time::Duration;

    fn usage() -> Option<libc::rusage> {
        // SAFETY: getrusage only writes the struct it is given.
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut u) };
        (ok == 0).then_some(u)
    }

    /// `ps -A -o pid=,ppid=,rss=,time=` rows: (pid, ppid, rss bytes, cpu).
    fn ps() -> Vec<(u32, u32, u64, Duration)> {
        let Ok(out) = std::process::Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,rss=,time="])
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                Some((
                    f.first()?.parse().ok()?,
                    f.get(1)?.parse().ok()?,
                    f.get(2)?.parse::<u64>().ok()?.checked_mul(1024)?,
                    super::ps_time(f.get(3)?)?,
                ))
            })
            .collect()
    }

    pub fn rss() -> Option<u64> {
        let me = std::process::id();
        ps().into_iter().find(|r| r.0 == me).map(|r| r.2)
    }

    pub fn peak_rss() -> Option<u64> {
        // macOS reports ru_maxrss in bytes.
        usage().and_then(|u| u64::try_from(u.ru_maxrss).ok())
    }

    pub fn cpu_time() -> Option<Duration> {
        let tv = |t: libc::timeval| {
            Duration::from_secs(u64::try_from(t.tv_sec).unwrap_or(0))
                + Duration::from_micros(u64::try_from(t.tv_usec).unwrap_or(0))
        };
        usage().map(|u| tv(u.ru_utime) + tv(u.ru_stime))
    }

    pub fn tree() -> Option<super::Tree> {
        let rows = ps();
        let pairs: Vec<(u32, u32)> = rows.iter().map(|r| (r.0, r.1)).collect();
        let mine = super::descendants(std::process::id(), &pairs);
        let mut t = super::Tree::default();
        for r in rows.iter().filter(|r| mine.contains(&r.0)) {
            t.rss += r.2;
            t.cpu += r.3;
            t.processes += 1;
        }
        (t.processes > 0).then_some(t)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    pub fn rss() -> Option<u64> {
        None
    }

    pub fn peak_rss() -> Option<u64> {
        None
    }

    pub fn cpu_time() -> Option<std::time::Duration> {
        None
    }

    pub fn tree() -> Option<super::Tree> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_parse_to_bytes() {
        let s = "Name:\tkor\nVmHWM:\t   20480 kB\nVmRSS:\t   10240 kB\n";
        assert_eq!(status_kb(s, "VmRSS:"), Some(10_485_760));
        assert_eq!(status_kb(s, "VmHWM:"), Some(20_971_520));
        assert_eq!(status_kb(s, "VmSwap:"), None);
    }

    #[test]
    fn descendants_follow_parents_and_ps_times_parse() {
        let procs = [(1, 0), (10, 1), (11, 10), (12, 11), (20, 1), (13, 10)];
        let mut d = descendants(10, &procs);
        d.sort_unstable();
        assert_eq!(d, [10, 11, 12, 13]);
        assert_eq!(ps_time("1:02.50"), Some(Duration::from_millis(62_500)));
        assert_eq!(ps_time("01:00:00"), Some(Duration::from_secs(3600)));
        assert_eq!(ps_time("2-00:00:01"), Some(Duration::from_secs(172_801)));
        assert_eq!(ps_time("x"), None);
    }

    #[test]
    fn this_process_has_resident_memory_and_cpu_time() {
        let block = vec![1u8; 8 << 20];
        let now = rss().unwrap();
        let peak = peak_rss().unwrap();
        assert!(now > 1 << 20, "{now}");
        assert!(peak >= now / 2, "{peak} {now}");
        drop(block);
        let before = cpu_time().unwrap();
        let t = std::time::Instant::now();
        let mut x = 0u64;
        while t.elapsed() < Duration::from_millis(60) {
            x = std::hint::black_box(x.wrapping_add(1));
        }
        assert!(cpu_time().unwrap() > before);
        let t = tree().unwrap();
        assert!(t.processes >= 1 && t.rss > 1 << 20, "{t:?}");
    }
}
