//! Where shell completions for `kor` go, and installing them (Master Build Prompt §26.1, ADR 0050).
//!
//! Shells that load completions from a folder get a file and nothing else (bash-completion,
//! fish, nushell autoload). zsh, `PowerShell` and elvish need one line in their profile; that line
//! is marked, added only once and only when the caller asked the user (`kor doctor --fix`,
//! `kor completions <shell> --install`), and removed again by [`remove`].

use std::io;
use std::path::{Path, PathBuf};

const MARK: &str = "# added by Kor (kor completions)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Elvish,
    Nushell,
}

impl Shell {
    pub const ALL: [Self; 6] = [
        Self::Bash,
        Self::Zsh,
        Self::Fish,
        Self::PowerShell,
        Self::Elvish,
        Self::Nushell,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
            Self::Elvish => "elvish",
            Self::Nushell => "nushell",
        }
    }

    /// Program that shows the shell is installed.
    pub fn program(self) -> &'static str {
        match self {
            Self::PowerShell => "pwsh",
            Self::Nushell => "nu",
            s => s.name(),
        }
    }
}

impl std::str::FromStr for Shell {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|sh| sh.name() == s || (s == "pwsh" && *sh == Self::PowerShell) || (s == "nu" && *sh == Self::Nushell))
            .ok_or_else(|| format!("unknown shell `{s}`"))
    }
}

/// Folders the plan is computed from (real ones in [`Dirs::current`], temp ones in tests).
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    /// `dirs::config_dir()`.
    pub config: PathBuf,
    /// `dirs::data_dir()` (XDG data home on Linux).
    pub data: PathBuf,
    /// Documents folder (`PowerShell` profile on Windows).
    pub documents: PathBuf,
    pub windows: bool,
}

impl Dirs {
    pub fn current() -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self {
            config: dirs::config_dir().unwrap_or_else(|| home.join(".config")),
            data: dirs::data_dir().unwrap_or_else(|| home.join(".local").join("share")),
            documents: dirs::document_dir().unwrap_or_else(|| home.join("Documents")),
            windows: crate::os() == crate::Os::Windows,
            home,
        })
    }
}

/// Where the script goes and which profile line loads it (if one is needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub shell: Shell,
    pub file: PathBuf,
    pub profile: Option<(PathBuf, String)>,
}

pub fn plan(shell: Shell, d: &Dirs) -> Plan {
    let kor_dir = d.data.join("kor").join("completions");
    let quoted = |p: &Path| p.display().to_string().replace('"', "\\\"");
    match shell {
        // bash-completion loads `$XDG_DATA_HOME/bash-completion/completions/<cmd>` on demand.
        Shell::Bash => Plan {
            shell,
            file: d.home.join(".local/share/bash-completion/completions/kor"),
            profile: None,
        },
        Shell::Fish => Plan {
            shell,
            file: d.home.join(".config/fish/completions/kor.fish"),
            profile: None,
        },
        Shell::Nushell => Plan {
            shell,
            file: d.config.join("nushell").join("autoload").join("kor.nu"),
            profile: None,
        },
        Shell::Zsh => {
            let file = kor_dir.join("_kor");
            let line = format!("(( $+functions[compdef] )) && source \"{}\" {MARK}", quoted(&file));
            Plan { shell, profile: Some((d.home.join(".zshrc"), line)), file }
        }
        Shell::PowerShell => {
            let file = kor_dir.join("kor.ps1");
            let profile = if d.windows {
                d.documents.join("PowerShell").join("Microsoft.PowerShell_profile.ps1")
            } else {
                d.home.join(".config/powershell/Microsoft.PowerShell_profile.ps1")
            };
            let line = format!(". \"{}\" {MARK}", quoted(&file));
            Plan { shell, profile: Some((profile, line)), file }
        }
        Shell::Elvish => {
            let base = if d.windows { d.config.join("elvish") } else { d.home.join(".config/elvish") };
            Plan {
                shell,
                file: base.join("lib").join("kor.elv"),
                profile: Some((base.join("rc.elv"), format!("use kor {MARK}"))),
            }
        }
    }
}

fn has_line(profile: &Path) -> bool {
    std::fs::read_to_string(profile).is_ok_and(|t| t.lines().any(|l| l.ends_with(MARK)))
}

/// Script present and, where needed, loaded from the profile.
pub fn installed(p: &Plan) -> bool {
    p.file.is_file() && p.profile.as_ref().is_none_or(|(f, _)| has_line(f))
}

/// Write `script` and add the profile line once. Idempotent.
///
/// # Errors
/// If a file cannot be written.
pub fn install(p: &Plan, script: &str) -> io::Result<()> {
    if let Some(dir) = p.file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p.file, script)?;
    if let Some((profile, line)) = &p.profile
        && !has_line(profile)
    {
        if let Some(dir) = profile.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = std::fs::read_to_string(profile).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
        text.push('\n');
        std::fs::write(profile, text)?;
    }
    Ok(())
}

/// Undo [`install`]: the script and Kor's marked line only.
///
/// # Errors
/// If a file cannot be written.
pub fn remove(p: &Plan) -> io::Result<()> {
    match std::fs::remove_file(&p.file) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some((profile, _)) = &p.profile
        && has_line(profile)
    {
        let text = std::fs::read_to_string(profile)?;
        let kept: Vec<&str> = text.lines().filter(|l| !l.ends_with(MARK)).collect();
        std::fs::write(profile, format!("{}\n", kept.join("\n")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path, windows: bool) -> Dirs {
        Dirs {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            documents: root.join("docs"),
            windows,
        }
    }

    #[test]
    fn folder_shells_need_no_profile_and_profile_shells_get_one_marked_line() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path(), false);
        let bash = plan(Shell::Bash, &d);
        assert!(bash.profile.is_none());
        assert!(!installed(&bash));
        install(&bash, "complete -F _kor kor").unwrap();
        assert!(installed(&bash));

        let zsh = plan(Shell::Zsh, &d);
        let (rc, _) = zsh.profile.clone().unwrap();
        std::fs::create_dir_all(rc.parent().unwrap()).unwrap();
        std::fs::write(&rc, "export A=1").unwrap();
        install(&zsh, "#compdef kor").unwrap();
        install(&zsh, "#compdef kor").unwrap();
        let text = std::fs::read_to_string(&rc).unwrap();
        assert_eq!(text.matches(MARK).count(), 1, "{text}");
        assert!(text.starts_with("export A=1\n"));
        assert!(installed(&zsh));
        remove(&zsh).unwrap();
        assert!(!installed(&zsh));
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), "export A=1\n");

        let ps = plan(Shell::PowerShell, &dirs(t.path(), true));
        assert!(ps.profile.unwrap().0.starts_with(t.path().join("docs")));
        assert_eq!("pwsh".parse::<Shell>().unwrap(), Shell::PowerShell);
        assert!("tcsh".parse::<Shell>().is_err());
    }
}
