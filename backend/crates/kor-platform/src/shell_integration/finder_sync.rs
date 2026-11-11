//! macOS Finder Sync extension switch (§25.2, ADR 0052).
//!
//! `Kor.app/Contents/PlugIns/KorFinderSync.appex` ships inside the app (built by
//! `scripts/build-finder-sync.sh`). macOS discovers it with the app but leaves it off until it is
//! enabled; `kor integrate install` turns it on with `pluginkit -e use`, uninstall with
//! `pluginkit -e ignore` (both per user, no administrator rights). Commands go through
//! [`Runner`] so tests never touch the system.

use std::io;

use super::sparse::{Output, Runner};

/// Bundle id of the extension (`Info.plist`).
pub const BUNDLE_ID: &str = "dev.kor.desktop.findersync";

fn pluginkit(runner: &mut dyn Runner, args: &[&str]) -> io::Result<Output> {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let out = runner.run("pluginkit", &args)?;
    if out.code == 0 {
        Ok(out)
    } else {
        Err(io::Error::other(format!(
            "pluginkit: {}",
            out.stderr.trim()
        )))
    }
}

/// Is the extension known to macOS (app installed), and is it enabled?
///
/// # Errors
/// When `pluginkit` cannot run.
pub fn status(runner: &mut dyn Runner) -> io::Result<(bool, bool)> {
    // `pluginkit -m -i <id>` prints `+    dev.kor…` (enabled), `-    …` (disabled) or nothing.
    let out = pluginkit(runner, &["-m", "-i", BUNDLE_ID])?;
    let line = out.stdout.lines().find(|l| l.contains(BUNDLE_ID));
    Ok((
        line.is_some(),
        line.is_some_and(|l| l.trim_start().starts_with('+')),
    ))
}

/// Enable the Finder menu. Idempotent.
///
/// # Errors
/// When Kor.app (with the extension) is not installed, or `pluginkit` fails.
pub fn enable(runner: &mut dyn Runner) -> io::Result<()> {
    if !status(runner)?.0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Kor Finder extension not found: install Kor.app in /Applications and open it once",
        ));
    }
    pluginkit(runner, &["-e", "use", "-i", BUNDLE_ID]).map(drop)
}

/// Disable the Finder menu. Idempotent; nothing to do when the app is gone.
///
/// # Errors
/// When `pluginkit` fails.
pub fn disable(runner: &mut dyn Runner) -> io::Result<()> {
    if !status(runner)?.0 {
        return Ok(());
    }
    pluginkit(runner, &["-e", "ignore", "-i", BUNDLE_ID]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        known: bool,
        on: bool,
        calls: Vec<String>,
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[String]) -> io::Result<Output> {
            self.calls.push(format!("{program} {}", args.join(" ")));
            let stdout = match args.first().map(String::as_str) {
                Some("-m") if self.known => format!(
                    "{}    {BUNDLE_ID}(0.8.0)\n",
                    if self.on { '+' } else { '-' }
                ),
                Some("-e") => {
                    self.on = args[1] == "use";
                    String::new()
                }
                _ => String::new(),
            };
            Ok(Output {
                code: 0,
                stdout,
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn enable_and_disable_through_pluginkit() {
        let mut f = Fake {
            known: true,
            on: false,
            calls: Vec::new(),
        };
        assert_eq!(status(&mut f).unwrap(), (true, false));
        enable(&mut f).unwrap();
        assert_eq!(status(&mut f).unwrap(), (true, true));
        assert!(
            f.calls
                .contains(&format!("pluginkit -e use -i {BUNDLE_ID}"))
        );
        disable(&mut f).unwrap();
        assert_eq!(status(&mut f).unwrap(), (true, false));

        let mut missing = Fake {
            known: false,
            on: false,
            calls: Vec::new(),
        };
        assert_eq!(
            enable(&mut missing).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        disable(&mut missing).unwrap();
        assert!(!missing.calls.iter().any(|c| c.contains("-e")));
    }
}
