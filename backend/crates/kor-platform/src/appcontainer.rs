//! Windows `AppContainer` confinement for commands the agent runs (Master Build Prompt §6.4).
//!
//! A confined process runs with a low-privilege `AppContainer` token: it can read what Windows
//! grants to "ALL APPLICATION PACKAGES" (system files, Program Files), read and write the project
//! root (an explicit ACL grant held while any confined command of that root runs) and a private
//! temp folder, and nothing else of the user's. No network unless `network` is set. A job object
//! caps memory and CPU and kills the whole process tree when the [`Child`] is dropped.
//!
//! Elsewhere [`available`] is `false` and [`spawn`] returns `Unsupported`.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Output;

#[cfg(windows)]
mod win;

/// What to run and how to confine it.
#[derive(Debug, Clone, Copy)]
pub struct Spec<'a> {
    /// Program name (looked up on `PATH` from `env`) or path.
    pub program: &'a OsStr,
    pub args: &'a [OsString],
    pub cwd: &'a Path,
    /// The child's complete environment (`TEMP`/`TMP` are replaced with the container's own).
    pub env: &'a [(OsString, OsString)],
    /// Project root the command may read and write (absolute, existing directory).
    pub root: &'a Path,
    pub network: bool,
    /// Memory cap for the whole process tree.
    pub memory_bytes: u64,
    /// CPU cap for the whole process tree in percent of the machine; 0 or 100 means none.
    pub cpu_percent: u8,
}

/// Whether this machine can run confined commands (Windows 8 or later).
pub fn available() -> bool {
    #[cfg(windows)]
    return win::available();
    #[cfg(not(windows))]
    false
}

/// Start `spec` confined, stdin empty, stdout and stderr captured.
pub fn spawn(spec: &Spec<'_>) -> io::Result<Child> {
    #[cfg(windows)]
    return win::spawn(spec).map(|inner| Child { inner });
    #[cfg(not(windows))]
    {
        let _ = spec;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "AppContainer is only available on Windows",
        ))
    }
}

/// A confined process. Dropping it kills every process it started.
#[derive(Debug)]
pub struct Child {
    #[cfg(windows)]
    inner: win::Child,
    #[cfg(not(windows))]
    never: std::convert::Infallible,
}

impl Child {
    pub fn id(&self) -> u32 {
        #[cfg(windows)]
        return self.inner.pid;
        #[cfg(not(windows))]
        match self.never {}
    }

    /// Wait for exit and collect the output. Dropping the future kills the process tree.
    pub async fn wait_with_output(self) -> io::Result<Output> {
        #[cfg(windows)]
        return self.inner.wait_with_output().await;
        #[cfg(not(windows))]
        match self.never {}
    }
}

/// Profile name for a project root: stable across runs, so a grant left by a crash is reused and
/// then removed by the next run.
#[cfg_attr(not(windows), allow(dead_code))]
fn profile_name(root: &Path) -> String {
    // FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
    let hash = root
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("kor.sandbox.{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_is_stable_short_and_case_insensitive() {
        let a = profile_name(Path::new(r"C:\Work\Proj"));
        assert_eq!(a, profile_name(Path::new(r"c:\work\proj")));
        assert_ne!(a, profile_name(Path::new(r"C:\Work\Other")));
        // CreateAppContainerProfile allows at most 64 characters.
        assert!(a.len() <= 64 && a.starts_with("kor.sandbox."));
    }

    #[test]
    fn unavailable_off_windows() {
        if !cfg!(windows) {
            assert!(!available());
        }
    }
}
