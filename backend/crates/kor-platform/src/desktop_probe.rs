//! Desktop facts that need the OS (§19.1, §19.2, ADR 0059): time since the user's last input (the
//! "human first" stop of computer use), the app in front (per-app permission tiers), GPUs, battery
//! and the keyboard layout (system profile). Every function answers `None` / empty when the
//! platform cannot tell; nothing here changes the system.
//!
//! - Windows: `GetLastInputInfo`, `GetForegroundWindow` + `QueryFullProcessImageNameW`,
//!   `GetKeyboardLayoutNameW`, `GetSystemPowerStatus`, `Win32_VideoController` (CIM).
//! - macOS: `ioreg` (`HIDIdleTime`), System Events (front process), `defaults` (HIToolbox),
//!   `pmset -g batt`, `system_profiler SPDisplaysDataType -json`.
//! - Linux (X11): `xprintidle`, `xprop` (`_NET_ACTIVE_WINDOW` → `_NET_WM_PID` → `/proc`),
//!   `setxkbmap -query`, `/sys/class/power_supply`, `lspci` + `/sys/class/drm` (AMD VRAM).
//!
//! `nvidia-smi` is tried first for GPUs on every OS (it knows the real VRAM).

use serde::{Deserialize, Serialize};

/// A graphics adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gpu {
    pub name: String,
    /// Dedicated video memory, when the platform reports it.
    pub vram_mb: Option<u64>,
}

/// Battery state; `None` from [`battery`] on machines without one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// Milliseconds since the last keyboard or mouse input on this desktop session (injected input
/// counts too: callers compare against their own last action).
pub fn idle_ms() -> Option<u64> {
    imp::idle_ms()
}

/// Executable name of the app owning the foreground window, lower case, without `.exe`
/// (`chrome`, `code`, `windowsterminal`, `safari`).
pub fn foreground_app() -> Option<String> {
    imp::foreground_app().map(|n| app_name(&n)).filter(|n| !n.is_empty())
}

/// Graphics adapters (empty when unknown).
pub fn gpus() -> Vec<Gpu> {
    let nvidia = run("nvidia-smi", &["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .map(|o| parse_nvidia_smi(&o))
        .unwrap_or_default();
    let mut all = imp::gpus();
    // nvidia-smi's VRAM is exact; the OS list adds integrated / other vendors' adapters.
    for g in nvidia {
        all.retain(|o| !same_gpu(&o.name, &g.name));
        all.push(g);
    }
    all
}

/// Battery charge, `None` without a battery.
pub fn battery() -> Option<Battery> {
    imp::battery()
}

/// Active keyboard layout as the OS names it: a Windows KLID (`0001041F`), a macOS input source
/// (`com.apple.keylayout.Turkish`), or `layout[:variant]` on Linux (`tr:f`).
pub fn keyboard_layout() -> Option<String> {
    imp::keyboard_layout()
}

/// `C:\Program Files\Google\Chrome\Application\chrome.exe` → `chrome`.
pub fn app_name(path_or_name: &str) -> String {
    let base = path_or_name
        .trim()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default();
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .or_else(|| base.strip_suffix(".app"))
        .unwrap_or(base);
    base.to_lowercase()
}

fn same_gpu(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().replace("nvidia", "").trim().to_owned();
    norm(a) == norm(b)
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    use crate::NoWindow as _;
    let out = std::process::Command::new(program)
        .args(args)
        .no_window()
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `NVIDIA GeForce RTX 4070, 12282` lines.
fn parse_nvidia_smi(out: &str) -> Vec<Gpu> {
    out.lines()
        .filter_map(|l| {
            let (name, mem) = l.rsplit_once(',')?;
            Some(Gpu {
                name: name.trim().to_owned(),
                vram_mb: mem.trim().parse().ok(),
            })
        })
        .filter(|g| !g.name.is_empty())
        .collect()
}

/// `"HIDIdleTime" = 123456789` (nanoseconds) in `ioreg -c IOHIDSystem` output.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_hid_idle(out: &str) -> Option<u64> {
    out.lines()
        .find(|l| l.contains("\"HIDIdleTime\""))
        .and_then(|l| l.rsplit('=').next())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|ns| ns / 1_000_000)
}

/// `pmset -g batt`: `-InternalBattery-0 (id=…)	87%; charging; 1:02 remaining`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_pmset(out: &str) -> Option<Battery> {
    let line = out.lines().find(|l| l.contains("InternalBattery"))?;
    let pct = line.split_whitespace().find_map(|w| w.strip_suffix("%;"))?;
    Some(Battery {
        percent: pct.parse().ok()?,
        charging: line.contains("; charging") || line.contains("; charged") || line.contains("AC attached"),
    })
}

/// `system_profiler SPDisplaysDataType -json`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_system_profiler(out: &str) -> Vec<Gpu> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(out) else {
        return Vec::new();
    };
    v["SPDisplaysDataType"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let name = g["sppci_model"].as_str().or_else(|| g["_name"].as_str())?.to_owned();
            let vram = ["spdisplays_vram", "spdisplays_vram_shared"]
                .iter()
                .find_map(|k| g[*k].as_str())
                .and_then(parse_size_mb);
            Some(Gpu { name, vram_mb: vram })
        })
        .collect()
}

/// `8 GB`, `1536 MB`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_size_mb(s: &str) -> Option<u64> {
    let mut it = s.split_whitespace();
    let n: u64 = it.next()?.parse().ok()?;
    match it.next()?.to_ascii_uppercase().as_str() {
        "GB" => Some(n * 1024),
        "MB" => Some(n),
        _ => None,
    }
}

/// `lspci` lines for display controllers: `01:00.0 VGA compatible controller: NVIDIA … (rev a1)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_lspci(out: &str) -> Vec<Gpu> {
    out.lines()
        .filter(|l| {
            l.contains("VGA compatible controller") || l.contains("3D controller") || l.contains("Display controller")
        })
        .filter_map(|l| {
            let name = l.split_once("controller: ")?.1;
            let name = name.split(" (rev").next().unwrap_or(name).trim();
            (!name.is_empty()).then(|| Gpu {
                name: name.to_owned(),
                vram_mb: None,
            })
        })
        .collect()
}

/// `setxkbmap -query`: `layout:     tr` / `variant:    f` → `tr:f`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_setxkbmap(out: &str) -> Option<String> {
    let field = |k: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(k))
            .map(|v| v.trim().split(',').next().unwrap_or_default().to_owned())
            .filter(|v| !v.is_empty())
    };
    let layout = field("layout:")?;
    Some(match field("variant:") {
        Some(v) => format!("{layout}:{v}"),
        None => layout,
    })
}

/// `_NET_WM_PID(CARDINAL) = 4321` → 4321.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_wm_pid(out: &str) -> Option<u32> {
    out.rsplit('=').next()?.trim().parse().ok()
}

#[cfg(windows)]
#[allow(unsafe_code)] // Win32 calls with out-parameters; each is commented.
mod imp {
    use super::{Battery, Gpu, run};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayoutNameW, GetLastInputInfo, LASTINPUTINFO,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    pub fn idle_ms() -> Option<u64> {
        let mut info = LASTINPUTINFO {
            cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>()).ok()?,
            dwTime: 0,
        };
        // SAFETY: `info` is a valid LASTINPUTINFO with cbSize set, writable for the call.
        if unsafe { GetLastInputInfo(&raw mut info) } == 0 {
            return None;
        }
        // SAFETY: no arguments; returns the tick count. Both wrap at 49.7 days together.
        let now = unsafe { GetTickCount() };
        Some(u64::from(now.wrapping_sub(info.dwTime)))
    }

    pub fn foreground_app() -> Option<String> {
        // SAFETY: no arguments; a null handle when no window is in front.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        // SAFETY: `hwnd` came from the OS; `pid` is a valid writable u32.
        unsafe { GetWindowThreadProcessId(hwnd, &raw mut pid) };
        if pid == 0 {
            return None;
        }
        // SAFETY: plain call; the handle is closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = u32::try_from(buf.len()).unwrap_or(0);
        // SAFETY: `buf` holds `len` u16s; `len` is updated to the written length.
        let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &raw mut len) };
        // SAFETY: `h` is an open process handle owned here.
        unsafe { CloseHandle(h) };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    pub fn keyboard_layout() -> Option<String> {
        let mut buf = [0u16; 9]; // KL_NAMELENGTH
        // SAFETY: `buf` has room for KL_NAMELENGTH characters including the NUL.
        if unsafe { GetKeyboardLayoutNameW(buf.as_mut_ptr()) } == 0 {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    pub fn battery() -> Option<Battery> {
        // SAFETY: SYSTEM_POWER_STATUS is plain data; zeroed is a valid value.
        let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        // SAFETY: `s` is valid and writable for the call.
        if unsafe { GetSystemPowerStatus(&raw mut s) } == 0 {
            return None;
        }
        // BatteryFlag 128: no system battery; 255: unknown.
        if s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
            return None;
        }
        Some(Battery {
            percent: s.BatteryLifePercent,
            charging: s.ACLineStatus == 1,
        })
    }

    pub fn gpus() -> Vec<Gpu> {
        let script = "Get-CimInstance Win32_VideoController | ForEach-Object { \"$($_.Name)|$($_.AdapterRAM)\" }";
        run("powershell", &["-NoProfile", "-NonInteractive", "-Command", script])
            .map(|o| {
                o.lines()
                    .filter_map(|l| {
                        let (name, ram) = l.trim().rsplit_once('|')?;
                        // AdapterRAM is a 32-bit field: values at 4 GB are capped, so unknown.
                        let vram = ram
                            .trim()
                            .parse::<u64>()
                            .ok()
                            .filter(|b| *b > 0 && *b < 0xFFF0_0000)
                            .map(|b| b / (1024 * 1024));
                        (!name.trim().is_empty()).then(|| Gpu {
                            name: name.trim().to_owned(),
                            vram_mb: vram,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Battery, Gpu, parse_hid_idle, parse_pmset, parse_system_profiler, run};

    pub fn idle_ms() -> Option<u64> {
        parse_hid_idle(&run("ioreg", &["-c", "IOHIDSystem", "-d", "4"])?)
    }

    pub fn foreground_app() -> Option<String> {
        let out = run(
            "osascript",
            &["-e", "tell application \"System Events\" to get name of first process whose frontmost is true"],
        )?;
        let name = out.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }

    pub fn keyboard_layout() -> Option<String> {
        let out = run("defaults", &["read", "com.apple.HIToolbox", "AppleCurrentKeyboardLayoutInputSourceID"])?;
        let id = out.trim();
        (!id.is_empty()).then(|| id.to_owned())
    }

    pub fn battery() -> Option<Battery> {
        parse_pmset(&run("pmset", &["-g", "batt"])?)
    }

    pub fn gpus() -> Vec<Gpu> {
        run("system_profiler", &["SPDisplaysDataType", "-json"])
            .map(|o| parse_system_profiler(&o))
            .unwrap_or_default()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Battery, Gpu, parse_lspci, parse_setxkbmap, parse_wm_pid, run};

    pub fn idle_ms() -> Option<u64> {
        run("xprintidle", &[])?.trim().parse().ok()
    }

    pub fn foreground_app() -> Option<String> {
        let active = run("xprop", &["-root", "_NET_ACTIVE_WINDOW"])?;
        let id = crate::foreground::parse_active_window(&active)?;
        let pid = parse_wm_pid(&run("xprop", &["-id", &id, "_NET_WM_PID"])?)?;
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_owned())
    }

    pub fn keyboard_layout() -> Option<String> {
        parse_setxkbmap(&run("setxkbmap", &["-query"])?)
    }

    pub fn battery() -> Option<Battery> {
        let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
        for e in dir.flatten() {
            let p = e.path();
            if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery") {
                let percent = std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse().ok()?;
                let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
                return Some(Battery {
                    percent,
                    charging: matches!(status.trim(), "Charging" | "Full"),
                });
            }
        }
        None
    }

    pub fn gpus() -> Vec<Gpu> {
        let mut list = run("lspci", &[]).map(|o| parse_lspci(&o)).unwrap_or_default();
        // AMD (amdgpu) exposes VRAM size in bytes; attach it to the AMD adapter.
        if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
            for e in rd.flatten() {
                let f = e.path().join("device/mem_info_vram_total");
                if let Some(mb) = std::fs::read_to_string(&f)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(|b| b / (1024 * 1024))
                    && let Some(g) = list
                        .iter_mut()
                        .find(|g| g.vram_mb.is_none() && (g.name.contains("AMD") || g.name.contains("ATI")))
                {
                    g.vram_mb = Some(mb);
                }
            }
        }
        list
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    use super::{Battery, Gpu};

    pub fn idle_ms() -> Option<u64> {
        None
    }
    pub fn foreground_app() -> Option<String> {
        None
    }
    pub fn keyboard_layout() -> Option<String> {
        None
    }
    pub fn battery() -> Option<Battery> {
        None
    }
    pub fn gpus() -> Vec<Gpu> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_are_normalised() {
        assert_eq!(app_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), "chrome");
        assert_eq!(app_name("/usr/bin/gnome-terminal"), "gnome-terminal");
        assert_eq!(app_name("Safari"), "safari");
        assert_eq!(app_name("WindowsTerminal.EXE"), "windowsterminal");
        assert_eq!(app_name("Visual Studio Code.app"), "visual studio code");
    }

    #[test]
    fn tool_outputs_parse() {
        let n = parse_nvidia_smi("NVIDIA GeForce RTX 4070, 12282\nbad line\n");
        assert_eq!(n, [Gpu { name: "NVIDIA GeForce RTX 4070".into(), vram_mb: Some(12282) }]);
        assert!(same_gpu("NVIDIA GeForce RTX 4070", "GeForce RTX 4070 "));

        assert_eq!(parse_hid_idle("  |   \"HIDIdleTime\" = 2500000000\n"), Some(2500));
        assert_eq!(parse_hid_idle("nothing"), None);

        let b = parse_pmset("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1234)\t87%; charging; 1:02 remaining present: true");
        assert_eq!(b, Some(Battery { percent: 87, charging: true }));
        let b = parse_pmset(" -InternalBattery-0 (id=1)\t12%; discharging; 0:30 remaining");
        assert_eq!(b, Some(Battery { percent: 12, charging: false }));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'"), None);

        let sp = r#"{"SPDisplaysDataType":[{"_name":"kHW","sppci_model":"Apple M2 Pro","spdisplays_vram_shared":"16 GB"},{"sppci_model":"AMD Radeon Pro 5500M","spdisplays_vram":"8 GB"}]}"#;
        let g = parse_system_profiler(sp);
        assert_eq!(g[0].vram_mb, Some(16 * 1024));
        assert_eq!(g[1].name, "AMD Radeon Pro 5500M");
        assert_eq!(parse_size_mb("1536 MB"), Some(1536));

        let l = parse_lspci("00:02.0 VGA compatible controller: Intel Corporation UHD Graphics 620 (rev 07)\n01:00.0 3D controller: NVIDIA Corporation GP108M [GeForce MX150] (rev a1)\n00:1f.3 Audio device: Intel");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "Intel Corporation UHD Graphics 620");

        assert_eq!(
            parse_setxkbmap("rules:      evdev\nmodel:      pc105\nlayout:     tr,us\nvariant:    f,\n").as_deref(),
            Some("tr:f")
        );
        assert_eq!(parse_setxkbmap("layout:     us\n").as_deref(), Some("us"));
        assert_eq!(parse_wm_pid("_NET_WM_PID(CARDINAL) = 4321"), Some(4321));
    }

    #[test]
    fn live_queries_do_not_panic() {
        let _ = idle_ms();
        let _ = foreground_app();
        let _ = keyboard_layout();
        let _ = battery();
    }
}
