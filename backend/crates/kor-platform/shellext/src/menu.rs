//! What the Windows 11 "Kor" context submenu shows and what each entry launches (§25.1, §27.6).
//!
//! Pure logic, shared by the COM server and its tests. Every entry starts `kor-desktop.exe` with
//! the argument protocol of `kor_runtime::osint` (`--terminal`, `--open`, `--edit`, `--task`);
//! the desktop's single-instance plugin forwards it to a running window.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Menu language: Kor's `ui.language`, else the Windows display language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Tr,
    En,
    Fr,
}

/// Windows primary language ids (`PRIMARYLANGID` of the UI LANGID).
const LANG_TURKISH: u16 = 0x1f;
const LANG_FRENCH: u16 = 0x0c;

impl Lang {
    /// `config` is `ui.language` (`tr`, `en`, `fr`, `auto`/unset); `os_langid` the user's UI LANGID.
    pub fn pick(config: Option<&str>, os_langid: u16) -> Self {
        match config.map(str::trim) {
            Some("tr") => Self::Tr,
            Some("en") => Self::En,
            Some("fr") => Self::Fr,
            _ => match os_langid & 0x3ff {
                LANG_TURKISH => Self::Tr,
                LANG_FRENCH => Self::Fr,
                _ => Self::En,
            },
        }
    }
}

/// One entry of the submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// "Kor CLI ile aç": `kor` in a terminal at the project.
    Terminal,
    /// "Kor Desktop ile aç".
    Desktop,
    /// §27.6 "Kor ile düzenle" (files only, multi-select).
    Edit,
    /// §25.1 extended: "Kor: Projeyi açıkla".
    Explain,
    /// §25.1 extended: "Kor: Hata bul".
    Bugs,
    /// §25.1 extended: "Kor: Bu dosyayı test et" (files only).
    Test,
}

impl Verb {
    pub const ALL: [Self; 6] = [
        Self::Terminal,
        Self::Desktop,
        Self::Edit,
        Self::Explain,
        Self::Bugs,
        Self::Test,
    ];

    pub fn title(self, lang: Lang) -> &'static str {
        use Lang::{En, Fr, Tr};
        match (self, lang) {
            (Self::Terminal, Tr) => "Kor CLI ile aç",
            (Self::Terminal, En) => "Open with Kor CLI",
            (Self::Terminal, Fr) => "Ouvrir avec Kor CLI",
            (Self::Desktop, Tr) => "Kor Desktop ile aç",
            (Self::Desktop, En) => "Open with Kor Desktop",
            (Self::Desktop, Fr) => "Ouvrir avec Kor Desktop",
            (Self::Edit, Tr) => "Kor ile düzenle",
            (Self::Edit, En) => "Edit with Kor",
            (Self::Edit, Fr) => "Modifier avec Kor",
            (Self::Explain, Tr) => "Kor: Projeyi açıkla",
            (Self::Explain, En) => "Kor: Explain the project",
            (Self::Explain, Fr) => "Kor : Expliquer le projet",
            (Self::Bugs, Tr) => "Kor: Hata bul",
            (Self::Bugs, En) => "Kor: Find bugs",
            (Self::Bugs, Fr) => "Kor : Trouver les bogues",
            (Self::Test, Tr) => "Kor: Bu dosyayı test et",
            (Self::Test, En) => "Kor: Test this file",
            (Self::Test, Fr) => "Kor : Tester ce fichier",
        }
    }

    /// Stable id (`IExplorerCommand::GetCanonicalName` is derived from it).
    pub fn index(self) -> u8 {
        match self {
            Self::Terminal => 1,
            Self::Desktop => 2,
            Self::Edit => 3,
            Self::Explain => 4,
            Self::Bugs => 5,
            Self::Test => 6,
        }
    }

    /// Shown for this selection? `extended` is `shell.context_menu.extended`.
    pub fn visible(self, sel: Selection, extended: bool) -> bool {
        if sel.count == 0 {
            return false;
        }
        let files_only = sel.files == sel.count;
        match self {
            Self::Terminal | Self::Desktop => true,
            Self::Edit => files_only,
            Self::Explain | Self::Bugs => extended,
            Self::Test => extended && files_only,
        }
    }

    /// `kor-desktop` arguments for the selected `paths` (`kor_runtime::osint` protocol).
    pub fn args(self, paths: &[PathBuf]) -> Vec<OsString> {
        let mut out: Vec<OsString> = match self {
            Self::Terminal => vec!["--terminal".into()],
            Self::Desktop => vec!["--open".into()],
            Self::Edit => vec!["--edit".into()],
            Self::Explain => vec!["--task".into(), "explain".into()],
            Self::Bugs => vec!["--task".into(), "bugs".into()],
            Self::Test => vec!["--task".into(), "test".into()],
        };
        out.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
        out
    }
}

/// What was right-clicked (for `Directory\Background` Explorer passes the folder itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub count: usize,
    /// How many of them are files (not folders).
    pub files: usize,
}

/// Entries for a selection, in menu order.
pub fn entries(sel: Selection, extended: bool) -> Vec<Verb> {
    Verb::ALL
        .into_iter()
        .filter(|v| v.visible(sel, extended))
        .collect()
}

/// Settings the menu reads from Kor's user `config.toml`: (`ui.language`,
/// `shell.context_menu.extended`). A missing or broken file gives the defaults.
pub fn settings(config_toml: &str) -> (Option<String>, bool) {
    let Ok(doc) = config_toml.parse::<toml::Table>() else {
        return (None, false);
    };
    let lang = doc
        .get("ui")
        .and_then(|u| u.get("language"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    let extended = doc
        .get("shell")
        .and_then(|s| s.get("context_menu"))
        .and_then(|c| c.get("extended"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    (lang, extended)
}

/// Kor's user `config.toml` (`%APPDATA%\kor\config.toml`, same as `kor_config::user_dir`).
pub fn user_config() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kor").join("config.toml"))
}

/// Where `kor-desktop.exe` is relative to this DLL: next to it, or one folder up (the installer
/// puts the shell extension in `<install>\shellext\`).
pub fn desktop_candidates(module_dir: &Path) -> [PathBuf; 2] {
    [
        module_dir.join("kor-desktop.exe"),
        module_dir
            .parent()
            .unwrap_or(module_dir)
            .join("kor-desktop.exe"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_comes_from_config_then_windows() {
        assert_eq!(Lang::pick(Some("fr"), 0x041f), Lang::Fr);
        assert_eq!(Lang::pick(Some("auto"), 0x041f), Lang::Tr);
        assert_eq!(Lang::pick(None, 0x040c), Lang::Fr);
        assert_eq!(Lang::pick(None, 0x0409), Lang::En);
        assert_eq!(Lang::pick(Some(" tr "), 0x0409), Lang::Tr);
    }

    #[test]
    fn entries_follow_selection_and_extended_setting() {
        use Verb::{Bugs, Desktop, Edit, Explain, Terminal, Test};
        let folder = Selection { count: 1, files: 0 };
        let files = Selection { count: 3, files: 3 };
        let mixed = Selection { count: 2, files: 1 };
        assert_eq!(entries(folder, false), [Terminal, Desktop]);
        assert_eq!(entries(files, false), [Terminal, Desktop, Edit]);
        assert_eq!(entries(mixed, false), [Terminal, Desktop]);
        assert_eq!(entries(folder, true), [Terminal, Desktop, Explain, Bugs]);
        assert_eq!(
            entries(files, true),
            [Terminal, Desktop, Edit, Explain, Bugs, Test]
        );
        assert!(entries(Selection::default(), true).is_empty());
    }

    #[test]
    fn args_follow_the_osint_protocol() {
        let p = [PathBuf::from(r"C:\a b\x.txt"), PathBuf::from(r"C:\y")];
        assert_eq!(
            Verb::Edit.args(&p),
            [r"--edit", r"C:\a b\x.txt", r"C:\y"].map(OsString::from)
        );
        assert_eq!(
            Verb::Test.args(&p[..1]),
            ["--task", "test", r"C:\a b\x.txt"].map(OsString::from)
        );
        assert_eq!(Verb::Terminal.args(&p[1..])[0], "--terminal");
        assert_eq!(Verb::Desktop.args(&p[1..])[0], "--open");
        let ids: std::collections::BTreeSet<u8> = Verb::ALL.iter().map(|v| v.index()).collect();
        assert_eq!(ids.len(), Verb::ALL.len());
        for v in Verb::ALL {
            for l in [Lang::Tr, Lang::En, Lang::Fr] {
                assert!(v.title(l).contains("Kor"));
            }
        }
    }

    #[test]
    fn finder_extension_uses_the_same_titles_and_protocol() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../frontend/src-tauri/macos/KorFinderSync");
        let swift = std::fs::read_to_string(dir.join("Menu.swift")).unwrap();
        for v in Verb::ALL {
            for l in [Lang::Tr, Lang::En, Lang::Fr] {
                assert!(
                    swift.contains(&format!("return \"{}\"", v.title(l))),
                    "{v:?} {l:?}"
                );
            }
            let flag = v
                .args(&[])
                .into_iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            assert!(swift.contains(&format!("[\"{}\"", flag[0])), "{flag:?}");
        }
        let plist = std::fs::read_to_string(dir.join("Info.plist")).unwrap();
        let glue = std::fs::read_to_string(dir.join("FinderSync.swift")).unwrap();
        assert!(
            plist.contains("<string>KorFinderSync</string>")
                && glue.contains("@objc(KorFinderSync)")
        );
        assert!(
            plist.contains("<string>dev.kor.desktop.findersync</string>"),
            "must be inside the app's id"
        );
    }

    #[test]
    fn settings_are_read_leniently() {
        let (l, e) = settings("[ui]\nlanguage = \"fr\"\n[shell.context_menu]\nextended = true\n");
        assert_eq!((l.as_deref(), e), (Some("fr"), true));
        assert_eq!(settings("not toml ["), (None, false));
        assert_eq!(settings(""), (None, false));
    }

    #[test]
    fn desktop_is_next_to_or_above_the_dll() {
        // Built from the host's separator: the test also runs on Linux and macOS.
        let root = std::env::temp_dir().join("Kor");
        let c = desktop_candidates(&root.join("shellext"));
        assert_eq!(c[0], root.join("shellext").join("kor-desktop.exe"));
        assert_eq!(c[1], root.join("kor-desktop.exe"));
    }
}
