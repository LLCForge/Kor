//! "Kor ile Aç" in the file manager (Master Build Prompt §25.1–25.2, §27.6, ADR 0052).
//!
//! Right-click on a folder, inside a folder (background) or on files shows **Kor CLI ile aç**,
//! **Kor Desktop ile aç** and, for files (multi-select too), **Kor ile düzenle**; with
//! `shell.context_menu.extended` also "Kor: Projeyi açıkla / Hata bul / Bu dosyayı test et".
//! Installed only on the user's consent (`kor integrate install`, desktop settings toggle,
//! installers), idempotent, and reversible: every key / file Kor writes is recorded in
//! `<data>/shell-integration.txt` and uninstall removes exactly those.
//!
//! - Windows: `HKCU\Software\Classes\{Directory, Directory\Background, *}\shell\Kor*` (`MUIVerb`,
//!   `Icon`, `MultiSelectModel`, cascading "Kor" submenu for the extended tasks), the `kor://`
//!   URL protocol when no installer registered it, and on Windows 11 the signed sparse package
//!   with the `IExplorerCommand` handler ([`sparse`]) so Kor appears in the new top-level menu.
//! - macOS: Finder Quick Actions (`~/Library/Services/*.workflow`, Automator "Run Shell Script")
//!   plus the Finder Sync extension shipped inside Kor.app ([`finder_sync`]). `kor://` is in the
//!   app's `Info.plist`.
//! - Linux: only for the file managers that are installed — Nautilus scripts (+ the
//!   nautilus-python extension when available), Caja scripts, Nemo actions, Dolphin service
//!   menus (KF5 and KF6), Thunar custom actions (`uca.xml`, merged), `PCManFM` / `FileManager`-Actions;
//!   and a hidden `.desktop` entry handling `x-scheme-handler/kor` and folders (dock drops),
//!   registered as the `kor://` default in `~/.config/mimeapps.list` when nothing else owns it.
//!
//! - Editors (§25.3, every OS): "Kor'da aç" in VS Code user tasks, Zed global tasks and
//!   `JetBrains` External Tools, for the editors whose settings folder exists.
//!
//! Every entry starts the desktop with the `kor_runtime::osint` argument protocol (`--open`,
//! `--edit`, `--task`, `--terminal`); without the desktop it starts `kor integrate …`.
//! Failures are kept in `<data>/shell-integration-error.txt` for `kor doctor` (§25.2).

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Os;

pub mod finder_sync;
pub mod sparse;

#[cfg(windows)]
mod win;

use sparse::Runner;

/// File of the Linux `.desktop` entry (shared with `jumplist`).
pub const LINUX_ENTRY_NAME: &str = crate::jumplist::LINUX_ENTRY_NAME;
const RECORD: &str = "shell-integration.txt";
const ERROR_FILE: &str = "shell-integration-error.txt";
const CLASSES: &str = r"Software\Classes";
const THUNAR_MARK: &str = "kor-integration";
const ICON_PNG: &[u8] = include_bytes!("../../../../frontend/src-tauri/icons/128x128.png");

/// The programs the entries start.
#[derive(Debug, Clone, Default)]
pub struct Apps {
    /// `kor-desktop` executable.
    pub desktop: Option<PathBuf>,
    /// `kor` CLI executable.
    pub cli: Option<PathBuf>,
}

/// Install options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// §25.1 extended submenu (`shell.context_menu.extended`).
    pub extended: bool,
    /// Menu language: `tr`, `en` or `fr` (Kor's `ui.language`).
    pub lang: String,
}

/// Where things stand.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Status {
    /// Kor's entries are in place.
    pub installed: bool,
    /// The extended submenu is installed.
    pub extended: bool,
    /// What was written (registry keys, files), as recorded.
    pub entries: Vec<String>,
    /// Linux file managers found on this machine.
    pub file_managers: Vec<String>,
    /// Windows 11 top-level menu package (`None`: not applicable).
    pub modern_menu: Option<bool>,
    /// macOS Finder Sync extension enabled (`None`: not applicable).
    pub finder_sync: Option<bool>,
    /// `kor://` opens Kor.
    pub scheme: bool,
    /// The last install / uninstall error (`kor doctor`).
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------------------------

/// Menu labels in one language.
#[derive(Debug, Clone, Copy)]
pub struct Labels {
    pub cli: &'static str,
    pub desktop: &'static str,
    pub edit: &'static str,
    pub submenu: &'static str,
    pub explain: &'static str,
    pub bugs: &'static str,
    pub test: &'static str,
}

/// Labels for `lang` (`tr` default).
pub fn labels(lang: &str) -> Labels {
    match lang {
        "en" => Labels {
            cli: "Open with Kor CLI",
            desktop: "Open with Kor Desktop",
            edit: "Edit with Kor",
            submenu: "Kor",
            explain: "Kor: Explain the project",
            bugs: "Kor: Find bugs",
            test: "Kor: Test this file",
        },
        "fr" => Labels {
            cli: "Ouvrir avec Kor CLI",
            desktop: "Ouvrir avec Kor Desktop",
            edit: "Modifier avec Kor",
            submenu: "Kor",
            explain: "Kor : Expliquer le projet",
            bugs: "Kor : Trouver des bogues",
            test: "Kor : Tester ce fichier",
        },
        _ => Labels {
            cli: "Kor CLI ile aç",
            desktop: "Kor Desktop ile aç",
            edit: "Kor ile düzenle",
            submenu: "Kor",
            explain: "Kor: Projeyi açıkla",
            bugs: "Kor: Hata bul",
            test: "Kor: Bu dosyayı test et",
        },
    }
}

// ---------------------------------------------------------------------------------------------
// Actions and their command lines
// ---------------------------------------------------------------------------------------------

/// What a menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Cli,
    Desktop,
    Edit,
    Task(&'static str),
}

/// Which selections an entry applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    /// Folders (and the inside of a folder).
    Folders,
    /// Files.
    Files,
    /// Both.
    Any,
}

impl Action {
    fn id(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Desktop => "desktop",
            Self::Edit => "edit",
            Self::Task(t) => t,
        }
    }
    fn label(self, l: &Labels) -> &'static str {
        match self {
            Self::Cli => l.cli,
            Self::Desktop => l.desktop,
            Self::Edit => l.edit,
            Self::Task("explain") => l.explain,
            Self::Task("bugs") => l.bugs,
            Self::Task(_) => l.test,
        }
    }
    fn applies(self) -> Applies {
        match self {
            Self::Edit | Self::Task("test") => Applies::Files,
            Self::Task("explain") => Applies::Folders,
            _ => Applies::Any,
        }
    }
}

/// Token replaced by the selected paths in [`argv`].
pub const PATHS: &str = "\u{0}PATHS\u{0}";

/// The entries to install.
pub fn actions(apps: &Apps, extended: bool) -> Vec<Action> {
    let mut v = vec![Action::Cli];
    if apps.desktop.is_some() {
        v.push(Action::Desktop);
    }
    v.push(Action::Edit);
    if extended {
        v.extend(["explain", "bugs", "test"].map(Action::Task));
    }
    v
}

/// Program and arguments for `action` (`PATHS` marks the selection), or `None` when neither app
/// can do it.
pub fn argv(os: Os, apps: &Apps, action: Action) -> Option<Vec<String>> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let desktop = apps.desktop.as_deref().map(s);
    let cli = apps.cli.as_deref().map(s);
    let with = |prog: String, mid: &[&str]| -> Vec<String> {
        std::iter::once(prog)
            .chain(mid.iter().map(|m| (*m).to_owned()))
            .chain(std::iter::once(PATHS.to_owned()))
            .collect()
    };
    match action {
        Action::Desktop => desktop.map(|d| with(d, &["--open"])),
        Action::Edit => desktop
            .map(|d| with(d, &["--edit"]))
            .or_else(|| cli.map(|c| with(c, &["integrate", "edit"]))),
        Action::Task(t) => desktop
            .map(|d| with(d, &["--task", t]))
            .or_else(|| cli.map(|c| with(c, &["integrate", "open-terminal", "--task", t]))),
        Action::Cli => {
            // Windows: through the GUI desktop binary (no console flash); elsewhere the CLI.
            let (first, second) = if os == Os::Windows {
                (desktop.clone(), cli.clone())
            } else {
                (cli.clone(), desktop.clone())
            };
            match (first, second) {
                (Some(p), _) if os == Os::Windows => Some(with(p, &["--terminal"])),
                (Some(p), _) => Some(with(p, &["integrate", "open-terminal"])),
                (None, Some(p)) if os == Os::Windows => {
                    Some(with(p, &["integrate", "open-terminal"]))
                }
                (None, Some(p)) => Some(with(p, &["--terminal"])),
                (None, None) => None,
            }
        }
    }
}

/// Windows command line with the selection as `token` (`"%V"`, `"%1"`).
pub fn windows_command(argv: &[String], token: &str) -> String {
    argv.iter()
        .map(|a| {
            if a == PATHS {
                format!("\"{token}\"")
            } else {
                crate::jumplist::quote_windows_arg(a)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `.desktop` `Exec=` value with the selection as `%F`.
pub fn desktop_exec(argv: &[String]) -> String {
    let (prog, rest) = argv
        .split_first()
        .map_or(("", &[][..]), |(p, r)| (p.as_str(), r));
    let mut s = crate::jumplist::desktop_exec(
        Path::new(prog),
        &rest
            .iter()
            .filter(|a| *a != PATHS)
            .cloned()
            .collect::<Vec<_>>(),
    );
    if argv.iter().any(|a| a == PATHS) {
        s.push_str(" %F");
    }
    s
}

/// POSIX `sh` line with the selection as `"$@"` (`${@:-$PWD}` style: the current folder when
/// nothing is selected, as in a folder background).
pub fn sh_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a == PATHS {
                "\"$@\"".to_owned()
            } else {
                crate::terminal::sh_quote(a)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn script(argv: &[String]) -> String {
    format!(
        "#!/bin/sh\n# Kor (kor integrate) — removed by `kor integrate uninstall`.\n[ $# -eq 0 ] && set -- \"$PWD\"\nexec {} >/dev/null 2>&1\n",
        sh_line(argv)
    )
}

// ---------------------------------------------------------------------------------------------
// Plan: what to write
// ---------------------------------------------------------------------------------------------

/// One thing to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A registry key under `HKCU` with values (`""` = default value). Created with its parents;
    /// removed as a tree.
    Reg {
        key: String,
        values: Vec<(String, String)>,
    },
    /// A file (executable when `exec`).
    File {
        path: PathBuf,
        body: Vec<u8>,
        exec: bool,
    },
    /// A directory removed as a whole on uninstall (macOS `.workflow` bundles).
    Dir { path: PathBuf },
    /// Kor's actions merged into Thunar's `uca.xml`.
    Thunar { path: PathBuf, actions: String },
    /// Kor's "Kor'da aç" tasks merged into an editor's `tasks.json`.
    JsonTasks {
        path: PathBuf,
        tasks: Vec<serde_json::Value>,
    },
    /// `x-scheme-handler/kor` default in `mimeapps.list` (only when unset).
    Mime { path: PathBuf, entry: String },
}

/// Linux file managers Kor writes entries for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Manager {
    Nautilus,
    NautilusPython,
    Caja,
    Nemo,
    Dolphin,
    Thunar,
    Pcmanfm,
}

impl Manager {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nautilus => "nautilus",
            Self::NautilusPython => "nautilus-python",
            Self::Caja => "caja",
            Self::Nemo => "nemo",
            Self::Dolphin => "dolphin",
            Self::Thunar => "thunar",
            Self::Pcmanfm => "pcmanfm",
        }
    }
}

/// The machine the plan is for (injected in tests).
#[derive(Debug, Clone)]
pub struct Target {
    pub os: Os,
    pub home: PathBuf,
    /// Kor's data dir (the record).
    pub data: PathBuf,
    /// XDG config home (`~/.config`).
    pub config: PathBuf,
    /// XDG data home (`~/.local/share`).
    pub share: PathBuf,
    /// Per-user application settings (`%APPDATA%`, `~/Library/Application Support`, `~/.config`):
    /// VS Code, Zed and `JetBrains` live here (§25.3 editor link).
    pub app_config: PathBuf,
    pub managers: Vec<Manager>,
}

impl Target {
    /// This machine.
    ///
    /// # Errors
    /// Without a home directory.
    pub fn current() -> io::Result<Self> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, "home directory not found");
        let home = dirs::home_dir().ok_or_else(missing)?;
        let os = crate::os();
        Ok(Self {
            config: dirs::config_dir()
                .filter(|_| os == Os::Linux)
                .unwrap_or_else(|| home.join(".config")),
            share: dirs::data_dir()
                .filter(|_| os == Os::Linux)
                .unwrap_or_else(|| home.join(".local").join("share")),
            data: crate::data_dir().ok_or_else(missing)?,
            app_config: dirs::config_dir().unwrap_or_else(|| home.join(".config")),
            managers: if os == Os::Linux {
                detect_managers()
            } else {
                Vec::new()
            },
            os,
            home,
        })
    }
}

/// Installed Linux file managers.
pub fn detect_managers() -> Vec<Manager> {
    let on = |n: &str| crate::on_path::which(n).is_some();
    let mut v = Vec::new();
    if on("nautilus") {
        v.push(Manager::Nautilus);
        let libs = [
            "/usr/lib",
            "/usr/lib64",
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
        ];
        let py = libs.iter().any(|l| {
            ["nautilus/extensions-4", "nautilus/extensions-3.0"]
                .iter()
                .any(|e| Path::new(l).join(e).join("libnautilus-python.so").is_file())
        });
        if py {
            v.push(Manager::NautilusPython);
        }
    }
    for (bin, m) in [
        ("caja", Manager::Caja),
        ("nemo", Manager::Nemo),
        ("dolphin", Manager::Dolphin),
        ("thunar", Manager::Thunar),
    ] {
        if on(bin) {
            v.push(m);
        }
    }
    if on("pcmanfm") || on("pcmanfm-qt") {
        v.push(Manager::Pcmanfm);
    }
    v
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Everything to write for `apps` on `t`.
pub fn plan(t: &Target, apps: &Apps, opts: &Options) -> Vec<Item> {
    let l = labels(&opts.lang);
    let acts: Vec<(Action, Vec<String>)> = actions(apps, opts.extended)
        .into_iter()
        .filter_map(|a| argv(t.os, apps, a).map(|v| (a, v)))
        .collect();
    let mut items = match t.os {
        Os::Windows => windows_plan(apps, &acts, &l),
        Os::Mac => mac_plan(t, &acts, &l),
        Os::Linux => linux_plan(t, apps, &acts, &l),
        Os::Other => Vec::new(),
    };
    items.extend(editor_items(t, apps, &opts.lang));
    items
}

/// One editor command: label, program, arguments with `{dir}` / `{file}` placeholders.
type EditorCommand = (String, String, Vec<String>);

/// "Kor'da aç" for an editor (§25.3): the project folder and the current file.
fn editor_commands(apps: &Apps, lang: &str) -> Vec<EditorCommand> {
    let (folder, file) = match lang {
        "en" => ("Open in Kor", "Open this file in Kor"),
        "fr" => ("Ouvrir dans Kor", "Ouvrir ce fichier dans Kor"),
        _ => ("Kor'da aç", "Bu dosyayı Kor'da aç"),
    };
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let (prog, pre): (String, Vec<String>) = match (&apps.desktop, &apps.cli) {
        (Some(d), _) => (s(d), vec!["--open".into()]),
        (None, Some(c)) => (s(c), vec!["integrate".into(), "open-terminal".into()]),
        (None, None) => return Vec::new(),
    };
    let with = |x: &str| {
        pre.iter()
            .cloned()
            .chain(std::iter::once(x.to_owned()))
            .collect::<Vec<_>>()
    };
    vec![
        (folder.to_owned(), prog.clone(), with("{dir}")),
        (file.to_owned(), prog, with("{file}")),
    ]
}

/// VS Code user tasks, Zed global tasks and `JetBrains` External Tools for every editor whose
/// settings folder exists.
fn editor_items(t: &Target, apps: &Apps, lang: &str) -> Vec<Item> {
    let cmds = editor_commands(apps, lang);
    if cmds.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let code = t.app_config.join("Code").join("User");
    if code.is_dir() {
        let tasks = cmds
            .iter()
            .map(|(label, prog, args)| {
                let args: Vec<String> =
                    args.iter().map(|a| a.replace("{dir}", "${workspaceFolder}").replace("{file}", "${file}")).collect();
                serde_json::json!({ "label": label, "type": "process", "command": prog, "args": args,
                    "presentation": { "reveal": "never" }, "problemMatcher": [], "detail": EDITOR_MARK })
            })
            .collect();
        out.push(Item::JsonTasks {
            path: code.join("tasks.json"),
            tasks,
        });
    }
    let zed = if t.os == Os::Windows {
        t.app_config.join("Zed")
    } else {
        t.home.join(".config").join("zed")
    };
    if zed.is_dir() {
        let tasks = cmds
            .iter()
            .map(|(label, prog, args)| {
                let args: Vec<String> =
                    args.iter().map(|a| a.replace("{dir}", "$ZED_WORKTREE_ROOT").replace("{file}", "$ZED_FILE")).collect();
                serde_json::json!({ "label": label, "command": prog, "args": args, "reveal": "never",
                    "hide": "always", "tags": [EDITOR_MARK] })
            })
            .collect();
        out.push(Item::JsonTasks {
            path: zed.join("tasks.json"),
            tasks,
        });
    }
    if let Ok(rd) = std::fs::read_dir(t.app_config.join("JetBrains")) {
        let mut products: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        products.sort();
        for p in products {
            out.push(Item::File {
                path: p.join("tools").join("Kor.xml"),
                body: jetbrains_tools(&cmds).into_bytes(),
                exec: false,
            });
        }
    }
    out
}

/// Marks the tasks Kor added to an editor's `tasks.json`.
const EDITOR_MARK: &str = "kor-integration";

fn jetbrains_tools(cmds: &[EditorCommand]) -> String {
    let mut s = String::from("<toolSet name=\"Kor\">\n");
    for (label, prog, args) in cmds {
        let params = args
            .iter()
            .map(|a| {
                a.replace("{dir}", "$ProjectFileDir$")
                    .replace("{file}", "$FilePath$")
            })
            .map(|a| {
                if a.contains(' ') {
                    format!("\"{a}\"")
                } else {
                    a
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            s,
            "  <tool name=\"{}\" showInMainMenu=\"true\" showInEditor=\"true\" showInProject=\"true\" showInSearchPopup=\"true\" disabled=\"false\" useConsole=\"false\" showConsoleOnStdOut=\"false\" showConsoleOnStdErr=\"false\" synchronizeAfterRun=\"false\">\n    <exec>\n      <option name=\"COMMAND\" value=\"{}\" />\n      <option name=\"PARAMETERS\" value=\"{}\" />\n      <option name=\"WORKING_DIRECTORY\" value=\"$ProjectFileDir$\" />\n    </exec>\n  </tool>",
            xml(label),
            xml(prog),
            xml(&params)
        );
    }
    s.push_str("</toolSet>\n");
    s
}

/// `text` (an editor `tasks.json`: VS Code object with `tasks`, or Zed array) with Kor's tasks
/// replaced by `tasks` (empty: removed). `None` when the file is not plain JSON (comments): it is
/// then left alone.
pub fn tasks_merge(text: &str, tasks: &[serde_json::Value]) -> Option<String> {
    use serde_json::Value;
    let ours = |v: &Value| {
        v.get("detail").and_then(Value::as_str) == Some(EDITOR_MARK)
            || v.get("tags")
                .and_then(Value::as_array)
                .is_some_and(|t| t.iter().any(|x| x.as_str() == Some(EDITOR_MARK)))
    };
    let mut doc: Value = if text.trim().is_empty() {
        if tasks.first().is_some_and(|t| t.get("tags").is_some()) {
            Value::Array(Vec::new())
        } else {
            serde_json::json!({ "version": "2.0.0", "tasks": [] })
        }
    } else {
        serde_json::from_str(text).ok()?
    };
    let list = match &mut doc {
        Value::Array(a) => a,
        Value::Object(o) => o
            .entry("tasks")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()?,
        _ => return None,
    };
    list.retain(|v| !ours(v));
    list.extend(tasks.iter().cloned());
    serde_json::to_string_pretty(&doc).ok().map(|s| s + "\n")
}

fn windows_plan(apps: &Apps, acts: &[(Action, Vec<String>)], l: &Labels) -> Vec<Item> {
    let icon = apps
        .desktop
        .as_deref()
        .or(apps.cli.as_deref())
        .map(|p| format!("{},0", p.display()))
        .unwrap_or_default();
    let mut out = Vec::new();
    let bases: [(&str, &str, Applies); 3] = [
        (r"Directory\shell", "%V", Applies::Folders),
        (r"Directory\Background\shell", "%V", Applies::Folders),
        (r"*\shell", "%1", Applies::Files),
    ];
    for (base, token, kind) in bases {
        let fits = |a: Action| a.applies() == Applies::Any || a.applies() == kind;
        let (tasks, plain): (Vec<_>, Vec<_>) = acts
            .iter()
            .filter(|(a, _)| fits(*a))
            .partition(|(a, _)| matches!(a, Action::Task(_)));
        for (a, v) in plain {
            let key = format!(r"{CLASSES}\{base}\Kor.{}", a.id());
            let mut values = vec![
                ("MUIVerb".to_owned(), a.label(l).to_owned()),
                ("Icon".to_owned(), icon.clone()),
            ];
            if *a == Action::Edit {
                values.push(("MultiSelectModel".into(), "Player".into()));
            }
            out.push(Item::Reg {
                key: key.clone(),
                values,
            });
            out.push(Item::Reg {
                key: format!(r"{key}\command"),
                values: vec![(String::new(), windows_command(v, token))],
            });
        }
        if !tasks.is_empty() {
            let key = format!(r"{CLASSES}\{base}\Kor.tasks");
            out.push(Item::Reg {
                key: key.clone(),
                values: vec![
                    ("MUIVerb".into(), l.submenu.into()),
                    ("Icon".into(), icon.clone()),
                    ("SubCommands".into(), String::new()),
                ],
            });
            for (a, v) in tasks {
                let sub = format!(r"{key}\shell\{}", a.id());
                out.push(Item::Reg {
                    key: sub.clone(),
                    values: vec![("MUIVerb".into(), a.label(l).into())],
                });
                out.push(Item::Reg {
                    key: format!(r"{sub}\command"),
                    values: vec![(String::new(), windows_command(v, token))],
                });
            }
        }
    }
    if let Some(d) = &apps.desktop {
        out.extend(windows_scheme(d));
    }
    out
}

/// `HKCU\Software\Classes\kor`: the `kor://` URL protocol opening the desktop.
pub fn windows_scheme(desktop: &Path) -> Vec<Item> {
    let key = format!(r"{CLASSES}\kor");
    vec![
        Item::Reg {
            key: key.clone(),
            values: vec![
                (String::new(), "URL:Kor".into()),
                ("URL Protocol".into(), String::new()),
            ],
        },
        Item::Reg {
            key: format!(r"{key}\DefaultIcon"),
            values: vec![(String::new(), format!("{},0", desktop.display()))],
        },
        Item::Reg {
            key: format!(r"{key}\shell\open\command"),
            values: vec![(String::new(), format!("\"{}\" \"%1\"", desktop.display()))],
        },
    ]
}

fn mac_plan(t: &Target, acts: &[(Action, Vec<String>)], l: &Labels) -> Vec<Item> {
    let services = t.home.join("Library").join("Services");
    let mut out = Vec::new();
    for (a, v) in acts {
        let name = format!("{}.workflow", a.label(l).replace([':', '/'], " ").trim());
        let dir = services.join(name);
        let types: &[&str] = match a.applies() {
            Applies::Files => &["public.item"],
            Applies::Folders => &["public.folder"],
            Applies::Any => &["public.item", "public.folder"],
        };
        out.push(Item::Dir { path: dir.clone() });
        let contents = dir.join("Contents");
        out.push(Item::File {
            path: contents.join("Info.plist"),
            body: workflow_info(a.label(l), types).into_bytes(),
            exec: false,
        });
        out.push(Item::File {
            path: contents.join("document.wflow"),
            body: workflow_document(&format!("{} >/dev/null 2>&1 &", sh_line(v))).into_bytes(),
            exec: false,
        });
    }
    out
}

const PLIST_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n";

/// `Contents/Info.plist` of a Quick Action shown for `types` in Finder.
pub fn workflow_info(label: &str, types: &[&str]) -> String {
    let types: String = types.iter().fold(String::new(), |mut o, t| {
        let _ = write!(o, "<string>{t}</string>");
        o
    });
    format!(
        "{PLIST_HEAD}<dict>\n<key>NSServices</key>\n<array>\n<dict>\n<key>NSBackgroundColorName</key><string>background</string>\n\
<key>NSIconName</key><string>NSActionTemplate</string>\n<key>NSMenuItem</key><dict><key>default</key><string>{}</string></dict>\n\
<key>NSMessage</key><string>runWorkflowAsService</string>\n\
<key>NSRequiredContext</key><dict><key>NSApplicationIdentifier</key><string>com.apple.finder</string></dict>\n\
<key>NSSendFileTypes</key><array>{types}</array>\n</dict>\n</array>\n</dict>\n</plist>\n",
        xml(label)
    )
}

/// `Contents/document.wflow`: one "Run Shell Script" action receiving the files as arguments.
pub fn workflow_document(script: &str) -> String {
    let uuid = |n: u8| format!("4B4F5200-0000-4000-8000-0000000000{n:02X}");
    format!(
        "{PLIST_HEAD}<dict>\n<key>AMApplicationBuild</key><string>523</string>\n<key>AMApplicationVersion</key><string>2.10</string>\n\
<key>AMDocumentVersion</key><string>2</string>\n<key>actions</key>\n<array>\n<dict>\n<key>action</key>\n<dict>\n\
<key>AMAccepts</key><dict><key>Container</key><string>List</string><key>Optional</key><true/><key>Types</key><array><string>com.apple.cocoa.string</string></array></dict>\n\
<key>AMActionVersion</key><string>2.0.3</string>\n<key>AMApplication</key><array><string>Automator</string></array>\n\
<key>AMParameterProperties</key><dict><key>COMMAND_STRING</key><dict/><key>CheckedForUserDefaultShell</key><dict/><key>inputMethod</key><dict/><key>shell</key><dict/><key>source</key><dict/></dict>\n\
<key>AMProvides</key><dict><key>Container</key><string>List</string><key>Types</key><array><string>com.apple.cocoa.string</string></array></dict>\n\
<key>ActionBundlePath</key><string>/System/Library/Automator/Run Shell Script.action</string>\n<key>ActionName</key><string>Run Shell Script</string>\n\
<key>ActionParameters</key><dict><key>COMMAND_STRING</key><string>{}</string><key>CheckedForUserDefaultShell</key><true/><key>inputMethod</key><integer>1</integer><key>shell</key><string>/bin/zsh</string><key>source</key><string></string></dict>\n\
<key>BundleIdentifier</key><string>com.apple.RunShellScript</string>\n<key>CFBundleVersion</key><string>2.0.3</string>\n\
<key>CanShowSelectedItemsWhenRun</key><false/>\n<key>CanShowWhenRun</key><true/>\n<key>Category</key><array><string>AMCategoryUtilities</string></array>\n\
<key>Class Name</key><string>RunShellScriptAction</string>\n<key>InputUUID</key><string>{}</string>\n<key>Keywords</key><array><string>Shell</string><string>Script</string></array>\n\
<key>OutputUUID</key><string>{}</string>\n<key>UUID</key><string>{}</string>\n<key>UnlocalizedApplications</key><array><string>Automator</string></array>\n\
<key>isViewVisible</key><integer>1</integer>\n</dict>\n<key>isViewVisible</key><integer>1</integer>\n</dict>\n</array>\n\
<key>connectors</key><dict/>\n<key>workflowMetaData</key>\n<dict>\n<key>applicationBundleIDsByPath</key><dict/>\n<key>applicationPaths</key><array/>\n\
<key>inputTypeIdentifier</key><string>com.apple.Automator.fileSystemObject</string>\n<key>outputTypeIdentifier</key><string>com.apple.Automator.nothing</string>\n\
<key>presentationMode</key><integer>15</integer>\n<key>processesInput</key><integer>0</integer>\n\
<key>serviceInputTypeIdentifier</key><string>com.apple.Automator.fileSystemObject</string>\n<key>serviceOutputTypeIdentifier</key><string>com.apple.Automator.nothing</string>\n\
<key>serviceProcessesInput</key><integer>0</integer>\n<key>systemImageName</key><string>NSActionTemplate</string>\n<key>useAutomaticInputType</key><integer>0</integer>\n\
<key>workflowTypeIdentifier</key><string>com.apple.Automator.servicesMenu</string>\n</dict>\n</dict>\n</plist>\n",
        xml(script),
        uuid(1),
        uuid(2),
        uuid(3)
    )
}

fn linux_plan(t: &Target, apps: &Apps, acts: &[(Action, Vec<String>)], l: &Labels) -> Vec<Item> {
    let mut out = Vec::new();
    let icon_path = t
        .share
        .join("icons")
        .join("hicolor")
        .join("128x128")
        .join("apps")
        .join("dev.kor.desktop.png");
    out.push(Item::File {
        path: icon_path.clone(),
        body: ICON_PNG.to_vec(),
        exec: false,
    });
    let icon = icon_path.to_string_lossy().into_owned();
    let has = |m: Manager| t.managers.contains(&m);
    let file_name = |a: Action| a.label(l).replace([':', '/'], " ").trim().to_owned();
    for m in [Manager::Nautilus, Manager::Caja] {
        if !has(m) {
            continue;
        }
        let dir = if m == Manager::Nautilus {
            t.share.join("nautilus").join("scripts")
        } else {
            t.config.join("caja").join("scripts")
        };
        for (a, v) in acts {
            out.push(Item::File {
                path: dir.join(file_name(*a)),
                body: script(v).into_bytes(),
                exec: true,
            });
        }
    }
    if has(Manager::NautilusPython) {
        out.push(Item::File {
            path: t
                .share
                .join("nautilus-python")
                .join("extensions")
                .join("kor_integration.py"),
            body: nautilus_python(acts, l).into_bytes(),
            exec: false,
        });
    }
    if has(Manager::Nemo) {
        for (a, v) in acts {
            let sel = match a.applies() {
                Applies::Files => "Selection=notnone\nExtensions=nodirs;\n",
                Applies::Folders => "Selection=any\nExtensions=dir;none;\n",
                Applies::Any => "Selection=any\nExtensions=any;\n",
            };
            out.push(Item::File {
                path: t.share.join("nemo").join("actions").join(format!("kor-{}.nemo_action", a.id())),
                body: format!("[Nemo Action]\nActive=true\nName={}\nComment=Kor\nExec={}\nIcon-Name={icon}\n{sel}", a.label(l), desktop_exec(v)).into_bytes(),
                exec: false,
            });
        }
    }
    if has(Manager::Dolphin) {
        let body = dolphin_menu(acts, l, &icon);
        for dir in [
            t.share.join("kio").join("servicemenus"),
            t.share.join("kservices5").join("ServiceMenus"),
        ] {
            out.push(Item::File {
                path: dir.join("kor.desktop"),
                body: body.clone().into_bytes(),
                exec: true,
            });
        }
    }
    if has(Manager::Thunar) {
        out.push(Item::Thunar {
            path: t.config.join("Thunar").join("uca.xml"),
            actions: thunar_actions(acts, l),
        });
    }
    if has(Manager::Pcmanfm) {
        for (a, v) in acts {
            let cond = match a.applies() {
                Applies::Files => "MimeTypes=all/allfiles;\n",
                Applies::Folders => "MimeTypes=inode/directory;\n",
                Applies::Any => "MimeTypes=all/all;\n",
            };
            out.push(Item::File {
                path: t.share.join("file-manager").join("actions").join(format!("kor-{}.desktop", a.id())),
                body: format!(
                    "[Desktop Entry]\nType=Action\nName={}\nIcon={icon}\nProfiles=kor;\n\n[X-Action-Profile kor]\nExec={}\nSelectionCount>0\n{cond}",
                    a.label(l),
                    desktop_exec(v)
                )
                .into_bytes(),
                exec: false,
            });
        }
    }
    if let Some(d) = &apps.desktop {
        let entry = t.share.join("applications").join(LINUX_ENTRY_NAME);
        out.push(Item::File {
            path: entry,
            body: format!(
                "[Desktop Entry]\nType=Application\nName=Kor\nComment=Kor ile aç\nIcon={icon}\nExec={} %U\nMimeType=x-scheme-handler/kor;inode/directory;\nNoDisplay=true\nTerminal=false\n",
                crate::jumplist::desktop_exec(d, &[])
            )
            .into_bytes(),
            exec: false,
        });
        out.push(Item::Mime {
            path: t.config.join("mimeapps.list"),
            entry: LINUX_ENTRY_NAME.into(),
        });
    }
    out
}

fn dolphin_menu(acts: &[(Action, Vec<String>)], l: &Labels, icon: &str) -> String {
    let ids: String = acts.iter().fold(String::new(), |mut o, (a, _)| {
        let _ = write!(o, "kor-{};", a.id());
        o
    });
    let mut s = format!(
        "[Desktop Entry]\nType=Service\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\nMimeType=all/all;\nActions={ids}\nX-KDE-Submenu={}\nIcon={icon}\n",
        l.submenu
    );
    for (a, v) in acts {
        let _ = write!(
            s,
            "\n[Desktop Action kor-{}]\nName={}\nIcon={icon}\nExec={}\n",
            a.id(),
            a.label(l),
            desktop_exec(v)
        );
    }
    s
}

fn thunar_actions(acts: &[(Action, Vec<String>)], l: &Labels) -> String {
    let mut s = String::new();
    for (a, v) in acts {
        let (dirs, files) = match a.applies() {
            Applies::Files => (
                "",
                "<audio-files/><image-files/><other-files/><text-files/><video-files/>",
            ),
            Applies::Folders => ("<directories/>", ""),
            Applies::Any => (
                "<directories/>",
                "<audio-files/><image-files/><other-files/><text-files/><video-files/>",
            ),
        };
        let _ = writeln!(
            s,
            "<action><icon>dev.kor.desktop</icon><name>{}</name><submenu></submenu><unique-id>{THUNAR_MARK}-{}</unique-id><command>{}</command><description>Kor</description><range></range><patterns>*</patterns>{dirs}{files}</action>",
            xml(a.label(l)),
            a.id(),
            xml(&desktop_exec(v))
        );
    }
    s
}

fn nautilus_python(acts: &[(Action, Vec<String>)], l: &Labels) -> String {
    let py = |s: &str| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"));
    let mut items = String::new();
    for (a, v) in acts {
        let argv: Vec<String> = v.iter().filter(|x| *x != PATHS).map(|x| py(x)).collect();
        let _ = writeln!(
            items,
            "    ({}, {}, [{}]),",
            py(&format!("Kor::{}", a.id())),
            py(a.label(l)),
            argv.join(", ")
        );
    }
    let kinds: String = acts.iter().fold(String::new(), |mut o, (a, _)| {
        let kind = match a.applies() {
            Applies::Files => "files",
            Applies::Folders => "folders",
            Applies::Any => "any",
        };
        let _ = writeln!(o, "    {}: {},", py(&format!("Kor::{}", a.id())), py(kind));
        o
    });
    format!(
        "# Kor (kor integrate) — removed by `kor integrate uninstall`.\nimport subprocess\nfrom urllib.parse import unquote, urlparse\n\nimport gi\ntry:\n    gi.require_version('Nautilus', '4.0')\nexcept ValueError:\n    gi.require_version('Nautilus', '3.0')\nfrom gi.repository import GObject, Nautilus\n\nITEMS = [\n{items}]\nKINDS = {{\n{kinds}}}\n\n\ndef _path(info):\n    uri = info.get_uri()\n    return unquote(urlparse(uri).path) if uri.startswith('file://') else None\n\n\ndef _run(argv, paths):\n    subprocess.Popen(argv + paths, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)\n\n\nclass KorMenu(GObject.GObject, Nautilus.MenuProvider):\n    def _items(self, paths, folders):\n        out = []\n        for ident, label, argv in ITEMS:\n            kind = KINDS[ident]\n            if (kind == 'files' and folders) or (kind == 'folders' and not folders):\n                continue\n            item = Nautilus.MenuItem(name=ident, label=label)\n            item.connect('activate', lambda _m, a=argv: _run(a, paths))\n            out.append(item)\n        return out\n\n    def get_file_items(self, *args):\n        files = args[-1]\n        paths = [p for p in (_path(f) for f in files) if p]\n        if not paths:\n            return []\n        return self._items(paths, all(f.is_directory() for f in files))\n\n    def get_background_items(self, *args):\n        path = _path(args[-1])\n        return self._items([path], True) if path else []\n"
    )
}

// ---------------------------------------------------------------------------------------------
// Applying the plan
// ---------------------------------------------------------------------------------------------

/// The per-user registry (`HKCU`); faked in tests.
pub trait Registry {
    /// Set a string value, creating the key and its parents.
    ///
    /// # Errors
    /// When the registry refuses.
    fn set(&mut self, key: &str, name: &str, value: &str) -> io::Result<()>;
    /// A string value.
    fn get(&self, key: &str, name: &str) -> Option<String>;
    fn exists(&self, key: &str) -> bool;
    /// Remove `key` and everything under it (nothing when missing).
    ///
    /// # Errors
    /// When the registry refuses.
    fn delete_tree(&mut self, key: &str) -> io::Result<()>;
    /// Tell Explorer that associations changed.
    fn notify(&mut self) {}
}

/// The real `HKCU` (Windows only; elsewhere every call is `Unsupported`).
#[derive(Debug, Default)]
pub struct SystemRegistry;

#[cfg(windows)]
impl Registry for SystemRegistry {
    fn set(&mut self, key: &str, name: &str, value: &str) -> io::Result<()> {
        win::set(key, name, value)
    }
    fn get(&self, key: &str, name: &str) -> Option<String> {
        win::get(key, name)
    }
    fn exists(&self, key: &str) -> bool {
        win::exists(key)
    }
    fn delete_tree(&mut self, key: &str) -> io::Result<()> {
        win::delete_tree(key)
    }
    fn notify(&mut self) {
        win::notify();
    }
}

#[cfg(not(windows))]
impl Registry for SystemRegistry {
    fn set(&mut self, _: &str, _: &str, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn get(&self, _: &str, _: &str) -> Option<String> {
        None
    }
    fn exists(&self, _: &str) -> bool {
        false
    }
    fn delete_tree(&mut self, _: &str) -> io::Result<()> {
        Ok(())
    }
}

fn record_path(t: &Target) -> PathBuf {
    t.data.join(RECORD)
}

fn read_record(t: &Target) -> Vec<String> {
    std::fs::read_to_string(record_path(t))
        .map(|r| r.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// `uca.xml` with Kor's actions replaced by `actions` (empty: removed).
pub fn thunar_merge(text: &str, actions: &str) -> String {
    let base = if text.trim().is_empty() {
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n"
    } else {
        text
    };
    let mut kept = String::new();
    let mut rest = base;
    while let Some(start) = rest.find("<action>") {
        let Some(len) = rest[start..].find("</action>") else {
            break;
        };
        let end = start + len + "</action>".len();
        kept.push_str(&rest[..start]);
        let block = &rest[start..end];
        if !block.contains(&format!("<unique-id>{THUNAR_MARK}-")) {
            kept.push_str(block);
        }
        rest = &rest[end..];
        if !block.contains(THUNAR_MARK) {
            continue;
        }
        rest = rest.strip_prefix('\n').unwrap_or(rest);
    }
    kept.push_str(rest);
    match kept.rfind("</actions>") {
        Some(i) => format!("{}{actions}{}", &kept[..i], &kept[i..]),
        None => kept,
    }
}

/// `mimeapps.list` with `x-scheme-handler/kor=<entry>` added under `[Default Applications]`
/// when no default exists (`add`), or removed when it is Kor's (`!add`).
pub fn mime_merge(text: &str, entry: &str, add: bool) -> String {
    const KEY: &str = "x-scheme-handler/kor=";
    let ours = format!("{KEY}{entry}");
    if !add {
        return text
            .split_inclusive('\n')
            .filter(|l| l.trim() != ours)
            .collect();
    }
    if text.lines().any(|l| l.trim().starts_with(KEY)) {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut done = false;
    for line in text.split_inclusive('\n') {
        out.push_str(line);
        if !done && line.trim() == "[Default Applications]" {
            if !line.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&ours);
            out.push('\n');
            done = true;
        }
    }
    if !done {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let _ = write!(out, "[Default Applications]\n{ours}\n");
    }
    out
}

fn write_file(path: &Path, body: &[u8], exec: bool) -> io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, body)?;
    if exec {
        crate::make_executable(path)?;
    }
    Ok(())
}

/// Write `items`, recording what was created. Undoes an earlier install first (so toggling the
/// extended menu or a moved app never leaves stale entries).
///
/// # Errors
/// On the first failing write; what was written so far stays recorded (and removable).
pub fn apply(t: &Target, reg: &mut dyn Registry, items: &[Item], extended: bool) -> io::Result<()> {
    remove(t, reg)?;
    let mut done: Vec<String> = Vec::new();
    let result = (|| -> io::Result<()> {
        for it in items {
            match it {
                Item::Reg { key, values } => {
                    // The URL protocol may belong to an installer: leave a foreign `kor` key alone.
                    if key.ends_with(r"Classes\kor") && reg.exists(key) {
                        continue;
                    }
                    if key.contains(r"Classes\kor\")
                        && !done.iter().any(|d| d == &format!(r"reg {CLASSES}\kor"))
                    {
                        continue;
                    }
                    for (n, v) in values {
                        reg.set(key, n, v)?;
                    }
                    let top = top_key(key);
                    let line = format!("reg {top}");
                    if !done.contains(&line) {
                        done.push(line);
                    }
                }
                Item::File { path, body, exec } => {
                    // Files inside a recorded bundle directory go with it.
                    let ours = done
                        .iter()
                        .any(|d| d.strip_prefix("dir ").is_some_and(|p| path.starts_with(p)));
                    write_file(path, body, *exec)?;
                    if !ours {
                        done.push(format!("file {}", path.display()));
                    }
                }
                Item::Dir { path } => {
                    std::fs::create_dir_all(path)?;
                    done.push(format!("dir {}", path.display()));
                }
                Item::Thunar { path, actions } => {
                    let text = std::fs::read_to_string(path).unwrap_or_default();
                    write_file(path, thunar_merge(&text, actions).as_bytes(), false)?;
                    done.push(format!("thunar {}", path.display()));
                }
                Item::JsonTasks { path, tasks } => {
                    let text = std::fs::read_to_string(path).unwrap_or_default();
                    // A tasks.json with comments is the user's to edit: skipped, not rewritten.
                    if let Some(next) = tasks_merge(&text, tasks) {
                        write_file(path, next.as_bytes(), false)?;
                        done.push(format!("tasks {}", path.display()));
                    }
                }
                Item::Mime { path, entry } => {
                    let text = std::fs::read_to_string(path).unwrap_or_default();
                    let next = mime_merge(&text, entry, true);
                    if next != text {
                        write_file(path, next.as_bytes(), false)?;
                        done.push(format!("mime {} {entry}", path.display()));
                    }
                }
            }
        }
        Ok(())
    })();
    if extended {
        done.push("extended".into());
    }
    reg.notify();
    write_record(t, &done)?;
    result
}

/// The Kor-owned key that contains `key` (`…\shell\Kor.cli`, `…\Kor.tasks`, `…\Classes\kor`).
fn top_key(key: &str) -> String {
    let parts: Vec<&str> = key.split('\\').collect();
    let n = parts
        .iter()
        .position(|p| p.starts_with("Kor.") || *p == "kor")
        .map_or(parts.len(), |i| i + 1);
    parts[..n].join("\\")
}

fn write_record(t: &Target, lines: &[String]) -> io::Result<()> {
    let path = record_path(t);
    if lines.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    write_file(&path, (lines.join("\n") + "\n").as_bytes(), false)
}

/// Remove everything recorded. Idempotent; never touches what Kor did not write.
///
/// # Errors
/// When a recorded entry cannot be removed (it stays recorded).
pub fn remove(t: &Target, reg: &mut dyn Registry) -> io::Result<()> {
    let mut left = Vec::new();
    let mut first_err = None;
    for line in read_record(t) {
        let r: io::Result<()> = if let Some(key) = line.strip_prefix("reg ") {
            reg.delete_tree(key)
        } else if let Some(p) = line.strip_prefix("file ") {
            match std::fs::remove_file(p) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else if let Some(p) = line.strip_prefix("dir ") {
            match std::fs::remove_dir_all(p) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else if let Some(p) = line.strip_prefix("thunar ") {
            match std::fs::read_to_string(p) {
                Ok(text) => std::fs::write(p, thunar_merge(&text, "")),
                Err(_) => Ok(()),
            }
        } else if let Some(p) = line.strip_prefix("tasks ") {
            match std::fs::read_to_string(p) {
                Ok(text) => match tasks_merge(&text, &[]) {
                    Some(next) => std::fs::write(p, next),
                    None => Ok(()),
                },
                Err(_) => Ok(()),
            }
        } else if let Some(rest) = line.strip_prefix("mime ") {
            let (p, entry) = rest.rsplit_once(' ').unwrap_or((rest, LINUX_ENTRY_NAME));
            match std::fs::read_to_string(p) {
                Ok(text) => std::fs::write(p, mime_merge(&text, entry, false)),
                Err(_) => Ok(()),
            }
        } else {
            Ok(())
        };
        if let Err(e) = r {
            first_err.get_or_insert(e);
            left.push(line);
        }
    }
    reg.notify();
    write_record(t, &left)?;
    first_err.map_or(Ok(()), Err)
}

/// Status from the record and the machine.
pub fn status_of(t: &Target, reg: &dyn Registry) -> Status {
    let lines = read_record(t);
    let entries: Vec<String> = lines.iter().filter(|l| *l != "extended").cloned().collect();
    let scheme = match t.os {
        Os::Windows => reg
            .get(r"Software\Classes\kor\shell\open\command", "")
            .is_some_and(|c| c.to_ascii_lowercase().contains("kor")),
        Os::Linux => std::fs::read_to_string(t.config.join("mimeapps.list"))
            .is_ok_and(|m| m.contains("x-scheme-handler/kor=")),
        // Kor.app's Info.plist declares the scheme.
        Os::Mac => true,
        Os::Other => false,
    };
    Status {
        installed: !entries.is_empty(),
        extended: lines.iter().any(|l| l == "extended"),
        entries,
        file_managers: t.managers.iter().map(|m| m.name().to_owned()).collect(),
        modern_menu: None,
        finder_sync: None,
        scheme,
        last_error: std::fs::read_to_string(t.data.join(ERROR_FILE))
            .ok()
            .filter(|e| !e.trim().is_empty()),
    }
}

// ---------------------------------------------------------------------------------------------
// Public API on this machine
// ---------------------------------------------------------------------------------------------

fn remember_error(t: &Target, r: &io::Result<()>) {
    let path = t.data.join(ERROR_FILE);
    match r {
        Ok(()) => {
            let _ = std::fs::remove_file(path);
        }
        Err(e) => {
            let _ = write_file(&path, e.to_string().as_bytes(), false);
        }
    }
}

/// The install folder's sparse menu package, when the desktop ships one.
fn sparse_dir(apps: &Apps) -> Option<PathBuf> {
    let dir = apps.desktop.as_deref()?.parent()?.to_path_buf();
    sparse::Kind::Menu.msix(&dir).is_file().then_some(dir)
}

/// The desktop and CLI installed with `cli` (the running `kor`): `kor-desktop` next to it, in the
/// macOS bundle (`Kor.app/Contents/MacOS`, `/Applications/Kor.app`), or on `PATH` (`.deb`/`.rpm`).
pub fn detect_apps(cli: Option<PathBuf>) -> Apps {
    let name = crate::exe("kor-desktop");
    let beside = cli.as_deref().and_then(Path::parent).map(|d| d.join(&name));
    let desktop = beside
        .filter(|p| p.is_file())
        .or_else(|| {
            (crate::os() == Os::Mac)
                .then(|| PathBuf::from("/Applications/Kor.app/Contents/MacOS").join(&name))
                .filter(|p| p.is_file())
        })
        .or_else(|| crate::on_path::which("kor-desktop"));
    Apps { desktop, cli }
}

/// Install with injected registry and runner (tests, installers that pass their own runner).
///
/// # Errors
/// When a write or a system command fails (recorded for `kor doctor`).
pub fn install_with(
    t: &Target,
    reg: &mut dyn Registry,
    runner: &mut dyn Runner,
    apps: &Apps,
    opts: &Options,
) -> io::Result<Status> {
    let items = plan(t, apps, opts);
    let mut r = apply(t, reg, &items, opts.extended);
    if r.is_ok() {
        r = native_on(t, runner, apps);
    }
    remember_error(t, &r);
    r.map(|()| full_status(t, reg, runner, apps))
}

/// Windows 11 sparse menu / macOS Finder Sync on; Services and desktop databases refreshed.
fn native_on(t: &Target, runner: &mut dyn Runner, apps: &Apps) -> io::Result<()> {
    match t.os {
        Os::Windows => {
            if let Some(dir) = sparse_dir(apps)
                && sparse::supported(runner)
            {
                sparse::register(runner, sparse::Kind::Menu, &dir)?;
                append_record(t, "sparse menu")?;
            }
            Ok(())
        }
        Os::Mac => {
            let _ = runner.run("/System/Library/CoreServices/pbs", &["-update".to_owned()]);
            if apps.desktop.is_some() && finder_sync::status(runner).is_ok_and(|s| s.0) {
                finder_sync::enable(runner)?;
                append_record(t, "finder-sync")?;
            }
            Ok(())
        }
        Os::Linux => {
            let apps_dir = t.share.join("applications");
            if apps_dir.is_dir() {
                let _ = runner.run(
                    "update-desktop-database",
                    &[apps_dir.to_string_lossy().into_owned()],
                );
            }
            let _ = runner.run(
                "gtk-update-icon-cache",
                &[
                    "-f".to_owned(),
                    "-t".to_owned(),
                    t.share
                        .join("icons")
                        .join("hicolor")
                        .to_string_lossy()
                        .into_owned(),
                ],
            );
            Ok(())
        }
        Os::Other => Ok(()),
    }
}

fn append_record(t: &Target, line: &str) -> io::Result<()> {
    let mut lines = read_record(t);
    if !lines.iter().any(|l| l == line) {
        lines.push(line.to_owned());
    }
    write_record(t, &lines)
}

/// Uninstall with injected registry and runner.
///
/// # Errors
/// When something recorded cannot be removed.
pub fn uninstall_with(
    t: &Target,
    reg: &mut dyn Registry,
    runner: &mut dyn Runner,
    apps: &Apps,
) -> io::Result<Status> {
    let lines = read_record(t);
    let mut r = Ok(());
    if lines.iter().any(|l| l == "sparse menu") {
        r = sparse::unregister(runner, sparse::Kind::Menu).map(drop);
    }
    if r.is_ok() && lines.iter().any(|l| l == "finder-sync") {
        r = finder_sync::disable(runner);
    }
    if r.is_ok() {
        r = remove(t, reg);
    }
    if r.is_ok() && t.os == Os::Mac {
        let _ = runner.run("/System/Library/CoreServices/pbs", &["-update".to_owned()]);
    }
    remember_error(t, &r);
    r.map(|()| full_status(t, reg, runner, apps))
}

fn full_status(t: &Target, reg: &dyn Registry, runner: &mut dyn Runner, apps: &Apps) -> Status {
    let mut s = status_of(t, reg);
    match t.os {
        Os::Windows => {
            s.modern_menu = sparse_dir(apps)
                .map(|_| sparse::status(runner, sparse::Kind::Menu).is_ok_and(|p| p.is_some()));
        }
        Os::Mac => {
            s.finder_sync = finder_sync::status(runner)
                .ok()
                .filter(|k| k.0)
                .map(|k| k.1)
        }
        _ => {}
    }
    s
}

/// Install on this machine (only on the user's consent).
///
/// # Errors
/// Without a home directory, or when a write fails.
pub fn install(apps: &Apps, opts: &Options) -> io::Result<Status> {
    install_with(
        &Target::current()?,
        &mut SystemRegistry,
        &mut sparse::System,
        apps,
        opts,
    )
}

/// Remove everything Kor installed.
///
/// # Errors
/// Without a home directory, or when a removal fails.
pub fn uninstall() -> io::Result<Status> {
    uninstall_with(
        &Target::current()?,
        &mut SystemRegistry,
        &mut sparse::System,
        &Apps::default(),
    )
}

/// Status on this machine.
pub fn status(apps: &Apps) -> Status {
    match Target::current() {
        Ok(t) => full_status(&t, &SystemRegistry, &mut sparse::System, apps),
        Err(e) => Status {
            last_error: Some(e.to_string()),
            ..Status::default()
        },
    }
}

/// [`status`] from the record only, without running system tools (`kor doctor`).
pub fn recorded_status() -> Status {
    match Target::current() {
        Ok(t) => status_of(&t, &SystemRegistry),
        Err(e) => Status {
            last_error: Some(e.to_string()),
            ..Status::default()
        },
    }
}

/// Register only the `kor://` scheme (§27.10 "xdg-open kor:// kaydı"; Windows URL protocol).
/// macOS: the app bundle declares it, nothing to do.
///
/// # Errors
/// Without the desktop app, or when a write fails.
pub fn register_scheme(apps: &Apps) -> io::Result<()> {
    let t = Target::current()?;
    let desktop = apps
        .desktop
        .as_deref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Kor desktop not found"))?;
    let mut reg = SystemRegistry;
    // The context-menu install already includes the scheme; do not clear it.
    if !read_record(&t).is_empty() {
        return Ok(());
    }
    apply(&t, &mut reg, &scheme_items(&t, desktop), false)
}

fn scheme_items(t: &Target, desktop: &Path) -> Vec<Item> {
    match t.os {
        Os::Windows => windows_scheme(desktop),
        Os::Linux => linux_plan(
            t,
            &Apps {
                desktop: Some(desktop.to_path_buf()),
                cli: None,
            },
            &[],
            &labels("tr"),
        ),
        _ => Vec::new(),
    }
}

/// Undo [`register_scheme`] (the whole integration: the scheme entry is shared with the menus).
///
/// # Errors
/// When a removal fails.
pub fn unregister_scheme() -> io::Result<()> {
    let t = Target::current()?;
    let menus = read_record(&t).iter().any(|l| {
        l.contains(r"\shell\Kor.")
            || l.starts_with("dir ")
            || l.contains("scripts")
            || l.contains("actions")
    });
    if menus {
        return Err(io::Error::other(
            "kor:// is part of the context-menu integration: use `kor integrate uninstall`",
        ));
    }
    uninstall().map(drop)
}

/// Is `kor://` registered for this user?
pub fn scheme_registered() -> bool {
    status(&Apps::default()).scheme
}

#[cfg(test)]
mod tests;
