//! First-run installation of the default-on integrations (ADR 0052, user decision 11 Nov 2026):
//! the right-click menu, the Kor block in the default shell's startup file and this OS's §27.10
//! items. It runs once per user; `<data dir>/integrations-first-run.json` records that it ran and
//! what it set up, so an item the user later turns off is never installed again. Every item stays
//! reversible exactly as when installed by hand (`kor integrate uninstall`, `kor shell uninstall`,
//! `kor os-integration uninstall`).
//!
//! [`guard`] keeps it away from isolated and test runs (`KOR_HOME` / `KOR_CONFIG_DIR` /
//! `KOR_CACHE_DIR` set, `tauri dev`, `cargo test` / `cargo run`), so a developer's real registry
//! and shell profiles are only touched by an installed release build.

use std::path::{Path, PathBuf};

use kor_agent::shellint::ShellKind;
use serde::{Deserialize, Serialize};

/// Marker file name in Kor's data directory.
pub const MARKER: &str = "integrations-first-run.json";

/// Why the first run must not happen in this process, or `None` when it may.
/// `env` reads an environment variable; `dev` is `tauri::is_dev()` in the desktop.
pub fn guard(env: impl Fn(&str) -> Option<String>, dev: bool) -> Option<&'static str> {
    // Set at all (even empty) counts: fail closed.
    if ["KOR_HOME", "KOR_CONFIG_DIR", "KOR_CACHE_DIR"]
        .iter()
        .any(|k| env(k).is_some())
    {
        return Some("isolated");
    }
    if dev {
        return Some("dev");
    }
    // Cargo sets `CARGO_MANIFEST_DIR` for the programs it runs (`cargo test`, `cargo run`).
    if cfg!(test) || env("CARGO_MANIFEST_DIR").is_some() {
        return Some("cargo");
    }
    None
}

/// [`guard`] for the current process.
pub fn guard_here(dev: bool) -> Option<&'static str> {
    guard(|k| std::env::var(k).ok(), dev)
}

/// One thing the first run installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// "Kor ile Aç" right-click entries (`kor integrate install`).
    ContextMenu,
    /// Kor's block in this shell's startup file (`kor shell install`).
    Shell(ShellKind),
    /// A §27.10 item (`kor os-integration install <id>`).
    Os(String),
}

impl Item {
    /// Stable id in the marker: `context-menu`, `shell:powershell`, `powertoys-run`.
    pub fn id(&self) -> String {
        match self {
            Self::ContextMenu => "context-menu".into(),
            Self::Shell(k) => format!("shell:{}", k.name()),
            Self::Os(id) => id.clone(),
        }
    }
}

/// What the first run did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marker {
    /// Unix seconds.
    pub at: u64,
    /// Item ids set up.
    pub installed: Vec<String>,
    /// Item id → error text, for items that failed (not retried).
    pub failed: Vec<(String, String)>,
}

/// The marker path in `data`.
pub fn marker_path(data: &Path) -> PathBuf {
    data.join(MARKER)
}

/// The recorded first run, if it happened.
pub fn load(data: &Path) -> Option<Marker> {
    let text = std::fs::read_to_string(marker_path(data)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Readable names of marker ids, joined with ", " (notice, `kor integrate status`, `kor doctor`).
pub fn names(l: &kor_i18n::Localizer, ids: &[String]) -> String {
    ids.iter()
        .map(|id| match id.strip_prefix("shell:") {
            Some(sh) => l.t_args(
                "autoint-item-shell",
                &kor_i18n::args!("shell" => sh.to_owned()),
            ),
            None if id == "context-menu" => l.t("autoint-item-context-menu"),
            None => l.t(&format!("osx-item-{id}")),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One line on the first run for `kor integrate status`, `kor os-integration list` and `kor doctor`.
pub fn status_line(l: &kor_i18n::Localizer, cfg: &kor_config::Integrations) -> String {
    match kor_platform::data_dir().and_then(|d| load(&d)) {
        Some(m) if m.installed.is_empty() => l.t("autoint-ran-none"),
        Some(m) => l.t_args(
            "autoint-ran",
            &kor_i18n::args!("list" => names(l, &m.installed)),
        ),
        None if !cfg.auto => l.t("autoint-off"),
        None => l.t("autoint-pending"),
    }
}

/// Items the first run would install. `default_shell` is `Some` only when that shell exists here;
/// `os_available` lists the installable §27.10 items of this OS that are available.
pub fn plan(
    cfg: &kor_config::Integrations,
    default_shell: Option<ShellKind>,
    os_available: &[&str],
) -> Vec<Item> {
    if !cfg.auto {
        return Vec::new();
    }
    let mut v = Vec::new();
    if cfg.context_menu {
        v.push(Item::ContextMenu);
    }
    if cfg.shell
        && let Some(k) = default_shell
    {
        v.push(Item::Shell(k));
    }
    v.extend(
        os_available
            .iter()
            .filter(|id| cfg.os_on(id))
            .map(|id| Item::Os((*id).to_owned())),
    );
    v
}

/// Result of [`run_once`].
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not run here ([`guard`] reason); nothing installed, nothing recorded.
    Blocked(&'static str),
    /// Ran before (or `auto` is off and the plan is empty): nothing done.
    AlreadyDone,
    /// Ran now.
    Ran(Marker),
}

impl Marker {
    /// Items this machine already tried (installed or failed): never tried again, so an item the
    /// user removed stays removed.
    pub fn attempted(&self, id: &str) -> bool {
        self.installed.iter().any(|i| i == id) || self.failed.iter().any(|(i, _)| i == id)
    }
}

/// Install each of `items` that was never attempted on this machine (the CLI and the desktop each
/// set up what they can; whichever runs first does not stop the other). Errors are recorded, not
/// retried; the marker keeps every attempt.
///
/// # Errors
/// When the marker cannot be written (the items stay installed and reversible).
pub fn run_once(
    data: &Path,
    blocked: Option<&'static str>,
    items: &[Item],
    mut install: impl FnMut(&Item) -> Result<(), String>,
) -> std::io::Result<Outcome> {
    if let Some(r) = blocked {
        return Ok(Outcome::Blocked(r));
    }
    let mut m = load(data).unwrap_or_default();
    let todo: Vec<&Item> = items.iter().filter(|it| !m.attempted(&it.id())).collect();
    if todo.is_empty() {
        return Ok(Outcome::AlreadyDone);
    }
    m.at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut now = Marker {
        at: m.at,
        ..Marker::default()
    };
    for it in todo {
        match install(it) {
            Ok(()) => now.installed.push(it.id()),
            Err(e) => now.failed.push((it.id(), e)),
        }
    }
    m.installed.extend(now.installed.iter().cloned());
    m.failed.extend(now.failed.iter().cloned());
    std::fs::create_dir_all(data)?;
    std::fs::write(
        marker_path(data),
        serde_json::to_string_pretty(&m).map_err(std::io::Error::other)?,
    )?;
    Ok(Outcome::Ran(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_blocks_isolated_dev_and_cargo_runs() {
        for k in ["KOR_HOME", "KOR_CONFIG_DIR", "KOR_CACHE_DIR"] {
            assert_eq!(
                guard(|v| (v == k).then(String::new), false),
                Some("isolated"),
                "{k}"
            );
        }
        assert_eq!(guard(|_| None, true), Some("dev"));
        assert_eq!(
            guard(|v| (v == "CARGO_MANIFEST_DIR").then(|| "/x".into()), false),
            Some("cargo")
        );
        // `cargo test` of this crate: blocked even with a clean environment.
        assert_eq!(guard(|_| None, false), Some("cargo"));
        assert!(guard_here(false).is_some());
    }

    #[test]
    fn blocked_run_installs_and_records_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let out = run_once(d.path(), guard_here(false), &[Item::ContextMenu], |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert!(matches!(out, Outcome::Blocked(_)));
        assert_eq!(calls, 0);
        assert!(load(d.path()).is_none());
    }

    #[test]
    fn runs_once_and_respects_later_removal() {
        let d = tempfile::tempdir().unwrap();
        let cfg = kor_config::Integrations::default();
        let items = plan(
            &cfg,
            Some(ShellKind::PowerShell),
            &["powertoys-run", "portals", "widgets"],
        );
        assert_eq!(
            items.iter().map(Item::id).collect::<Vec<_>>(),
            [
                "context-menu",
                "shell:powershell",
                "powertoys-run",
                "widgets"
            ]
        );
        let out = run_once(d.path(), None, &items, |i| {
            if i.id() == "widgets" {
                Err("no-bundle".into())
            } else {
                Ok(())
            }
        })
        .unwrap();
        let Outcome::Ran(m) = out else {
            panic!("{out:?}")
        };
        assert_eq!(
            m.installed,
            ["context-menu", "shell:powershell", "powertoys-run"]
        );
        assert_eq!(m.failed, [("widgets".to_owned(), "no-bundle".to_owned())]);
        assert_eq!(load(d.path()), Some(m));
        // Second launch (the user may have removed items meanwhile): nothing is installed again.
        assert_eq!(
            run_once(d.path(), None, &items, |_| panic!("ran twice")).unwrap(),
            Outcome::AlreadyDone
        );
    }

    #[test]
    fn cli_first_then_desktop_adds_only_what_it_can() {
        let d = tempfile::tempdir().unwrap();
        let cfg = kor_config::Integrations::default();
        // The CLI can set up the menu and the shell, not the desktop-only OS items.
        let cli = plan(&cfg, Some(ShellKind::Bash), &[]);
        let Outcome::Ran(first) = run_once(d.path(), None, &cli, |_| Ok(())).unwrap() else {
            panic!()
        };
        assert_eq!(first.installed, ["context-menu", "shell:bash"]);
        // Later the desktop runs: only the items never attempted are installed.
        let desk = plan(&cfg, Some(ShellKind::Bash), &["windows-terminal"]);
        let mut seen = Vec::new();
        let Outcome::Ran(second) = run_once(d.path(), None, &desk, |i| {
            seen.push(i.id());
            Ok(())
        })
        .unwrap() else {
            panic!()
        };
        assert_eq!(seen, ["windows-terminal"]);
        assert_eq!(second.installed, ["windows-terminal"]);
        assert_eq!(
            load(d.path()).unwrap().installed,
            ["context-menu", "shell:bash", "windows-terminal"]
        );
    }

    #[test]
    fn switches_leave_items_out() {
        let mut cfg = kor_config::Integrations::default();
        cfg.os.insert("widgets".into(), false);
        cfg.shell = false;
        let ids: Vec<String> = plan(
            &cfg,
            Some(ShellKind::Bash),
            &["widgets", "windows-terminal"],
        )
        .iter()
        .map(Item::id)
        .collect();
        assert_eq!(ids, ["context-menu", "windows-terminal"]);
        cfg.auto = false;
        assert!(plan(&cfg, None, &["windows-terminal"]).is_empty());
    }
}
