//! Install / uninstall into fake roots: an in-memory `HKCU` and temp home directories. Never the
//! real registry, `~/Library` or `~/.local` (§25.4).

use std::collections::BTreeMap;

use super::*;

#[derive(Default)]
struct FakeReg {
    keys: BTreeMap<String, BTreeMap<String, String>>,
    notified: usize,
}

impl Registry for FakeReg {
    fn set(&mut self, key: &str, name: &str, value: &str) -> io::Result<()> {
        // Parents exist implicitly, like RegSetKeyValueW.
        let mut k = String::new();
        for part in key.split('\\') {
            if !k.is_empty() {
                k.push('\\');
            }
            k.push_str(part);
            self.keys.entry(k.to_ascii_lowercase()).or_default();
        }
        self.keys.entry(key.to_ascii_lowercase()).or_default().insert(name.to_owned(), value.to_owned());
        Ok(())
    }
    fn get(&self, key: &str, name: &str) -> Option<String> {
        self.keys.get(&key.to_ascii_lowercase())?.get(name).cloned()
    }
    fn exists(&self, key: &str) -> bool {
        self.keys.contains_key(&key.to_ascii_lowercase())
    }
    fn delete_tree(&mut self, key: &str) -> io::Result<()> {
        let k = key.to_ascii_lowercase();
        self.keys.retain(|x, _| *x != k && !x.starts_with(&format!("{k}\\")));
        Ok(())
    }
    fn notify(&mut self) {
        self.notified += 1;
    }
}

/// Records commands; answers like a machine without the sparse package / Finder extension.
#[derive(Default)]
struct FakeRun(Vec<String>);

impl Runner for FakeRun {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<sparse::Output> {
        self.0.push(format!("{program} {}", args.join(" ")));
        Ok(sparse::Output { code: 0, stdout: String::new(), stderr: String::new() })
    }
}

fn target(root: &Path, os: Os, managers: Vec<Manager>) -> Target {
    Target {
        os,
        home: root.join("home"),
        data: root.join("data"),
        config: root.join("home").join(".config"),
        share: root.join("home").join(".local").join("share"),
        app_config: root.join("appconfig"),
        managers,
    }
}

fn apps(win: bool) -> Apps {
    if win {
        Apps { desktop: Some(PathBuf::from(r"C:\Kor\kor-desktop.exe")), cli: Some(PathBuf::from(r"C:\Kor\kor.exe")) }
    } else {
        Apps { desktop: Some(PathBuf::from("/opt/Kor/kor-desktop")), cli: Some(PathBuf::from("/opt/Kor/kor")) }
    }
}

fn opts(extended: bool) -> Options {
    Options { extended, lang: "tr".into() }
}

#[test]
fn windows_registry_install_is_idempotent_and_leaves_no_trace() {
    let d = tempfile::tempdir().unwrap();
    let t = target(d.path(), Os::Windows, vec![]);
    let mut reg = FakeReg::default();
    // Someone else's verb stays.
    reg.set(r"Software\Classes\Directory\shell\Other\command", "", "x").unwrap();
    let before = reg.keys.clone();
    let mut run = FakeRun::default();
    let s = install_with(&t, &mut reg, &mut run, &apps(true), &opts(true)).unwrap();
    assert!(s.installed && s.extended && s.scheme, "{s:?}");
    let cmd = reg.get(r"Software\Classes\Directory\shell\Kor.desktop\command", "").unwrap();
    assert_eq!(cmd, r#"C:\Kor\kor-desktop.exe --open "%V""#);
    assert_eq!(reg.get(r"Software\Classes\Directory\Background\shell\Kor.cli\command", "").unwrap(), r#"C:\Kor\kor-desktop.exe --terminal "%V""#);
    assert_eq!(reg.get(r"Software\Classes\*\shell\Kor.edit", "MultiSelectModel").as_deref(), Some("Player"));
    assert_eq!(reg.get(r"Software\Classes\*\shell\Kor.edit", "MUIVerb").as_deref(), Some("Kor ile düzenle"));
    assert_eq!(reg.get(r"Software\Classes\*\shell\Kor.desktop", "Icon").as_deref(), Some(r"C:\Kor\kor-desktop.exe,0"));
    assert_eq!(
        reg.get(r"Software\Classes\*\shell\Kor.tasks\shell\test\command", "").unwrap(),
        r#"C:\Kor\kor-desktop.exe --task test "%1""#
    );
    // Explain is for folders only, edit for files only.
    assert!(!reg.exists(r"Software\Classes\*\shell\Kor.tasks\shell\explain"));
    assert!(!reg.exists(r"Software\Classes\Directory\shell\Kor.edit"));
    assert_eq!(reg.get(r"Software\Classes\kor", "URL Protocol").as_deref(), Some(""));
    let once = reg.keys.clone();
    install_with(&t, &mut reg, &mut run, &apps(true), &opts(true)).unwrap();
    assert_eq!(reg.keys, once, "idempotent");
    // Turning the extended menu off removes it.
    install_with(&t, &mut reg, &mut run, &apps(true), &opts(false)).unwrap();
    assert!(!reg.exists(r"Software\Classes\Directory\shell\Kor.tasks"));
    let s = uninstall_with(&t, &mut reg, &mut run, &apps(true)).unwrap();
    assert!(!s.installed);
    // Parents created implicitly remain as empty keys like on Windows; nothing of Kor's.
    assert!(reg.keys.keys().all(|k| !k.contains("kor")), "{:?}", reg.keys.keys().collect::<Vec<_>>());
    assert_eq!(reg.get(r"Software\Classes\Directory\shell\Other\command", ""), before.get(r"software\classes\directory\shell\other\command").and_then(|m| m.get("").cloned()));
    assert!(!record_path(&t).exists());
    assert!(reg.notified > 0);
}

#[test]
fn windows_leaves_an_installer_owned_url_protocol_alone() {
    let d = tempfile::tempdir().unwrap();
    let t = target(d.path(), Os::Windows, vec![]);
    let mut reg = FakeReg::default();
    reg.set(r"Software\Classes\kor\shell\open\command", "", r#""C:\Program Files\Kor\kor-desktop.exe" "%1""#).unwrap();
    install_with(&t, &mut reg, &mut FakeRun::default(), &apps(true), &opts(false)).unwrap();
    assert_eq!(reg.get(r"Software\Classes\kor\shell\open\command", "").unwrap(), r#""C:\Program Files\Kor\kor-desktop.exe" "%1""#);
    uninstall_with(&t, &mut reg, &mut FakeRun::default(), &apps(true)).unwrap();
    assert!(reg.exists(r"Software\Classes\kor\shell\open\command"));
}

#[test]
fn cli_only_installs_use_kor_integrate() {
    let only_cli = Apps { desktop: None, cli: Some(PathBuf::from("/usr/bin/kor")) };
    assert_eq!(actions(&only_cli, false), vec![Action::Cli, Action::Edit]);
    assert_eq!(argv(Os::Linux, &only_cli, Action::Cli).unwrap(), vec!["/usr/bin/kor", "integrate", "open-terminal", PATHS]);
    assert_eq!(argv(Os::Linux, &only_cli, Action::Edit).unwrap(), vec!["/usr/bin/kor", "integrate", "edit", PATHS]);
    assert_eq!(
        argv(Os::Linux, &only_cli, Action::Task("bugs")).unwrap(),
        vec!["/usr/bin/kor", "integrate", "open-terminal", "--task", "bugs", PATHS]
    );
    let win = Apps { desktop: None, cli: Some(PathBuf::from(r"C:\K\kor.exe")) };
    assert_eq!(windows_command(&argv(Os::Windows, &win, Action::Cli).unwrap(), "%V"), r#"C:\K\kor.exe integrate open-terminal "%V""#);
    assert!(argv(Os::Linux, &Apps::default(), Action::Cli).is_none());
}

#[test]
fn linux_writes_only_for_installed_file_managers_and_cleans_up() {
    let d = tempfile::tempdir().unwrap();
    let all = vec![Manager::Nautilus, Manager::NautilusPython, Manager::Caja, Manager::Nemo, Manager::Dolphin, Manager::Thunar, Manager::Pcmanfm];
    let t = target(d.path(), Os::Linux, all);
    let uca = t.config.join("Thunar").join("uca.xml");
    std::fs::create_dir_all(uca.parent().unwrap()).unwrap();
    let user_uca = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n<action><name>Mine</name><unique-id>1-1</unique-id></action>\n</actions>\n";
    std::fs::write(&uca, user_uca).unwrap();
    let mime = t.config.join("mimeapps.list");
    std::fs::write(&mime, "[Default Applications]\ntext/plain=gedit.desktop\n").unwrap();
    let mut reg = FakeReg::default();
    let mut run = FakeRun::default();
    let s = install_with(&t, &mut reg, &mut run, &apps(false), &opts(true)).unwrap();
    assert!(s.installed && s.scheme);
    let script = std::fs::read_to_string(t.share.join("nautilus/scripts/Kor CLI ile aç")).unwrap();
    assert!(script.contains("exec /opt/Kor/kor integrate open-terminal \"$@\""), "{script}");
    assert!(t.config.join("caja/scripts/Kor ile düzenle").is_file());
    let py = std::fs::read_to_string(t.share.join("nautilus-python/extensions/kor_integration.py")).unwrap();
    assert!(py.contains("('Kor::desktop', 'Kor Desktop ile aç', ['/opt/Kor/kor-desktop', '--open'])"), "{py}");
    let nemo = std::fs::read_to_string(t.share.join("nemo/actions/kor-edit.nemo_action")).unwrap();
    assert!(nemo.contains("Exec=/opt/Kor/kor-desktop --edit %F") && nemo.contains("Extensions=nodirs;"), "{nemo}");
    let dolphin = std::fs::read_to_string(t.share.join("kio/servicemenus/kor.desktop")).unwrap();
    assert!(dolphin.contains("[Desktop Action kor-bugs]\nName=Kor: Hata bul"), "{dolphin}");
    assert!(t.share.join("kservices5/ServiceMenus/kor.desktop").is_file());
    let uca_now = std::fs::read_to_string(&uca).unwrap();
    assert!(uca_now.contains("<unique-id>1-1</unique-id>") && uca_now.contains("kor-integration-cli"), "{uca_now}");
    assert!(t.share.join("file-manager/actions/kor-desktop.desktop").is_file());
    let entry = std::fs::read_to_string(t.share.join("applications").join(LINUX_ENTRY_NAME)).unwrap();
    assert!(entry.contains("MimeType=x-scheme-handler/kor;inode/directory;") && entry.contains("Exec=/opt/Kor/kor-desktop %U"));
    assert!(std::fs::read_to_string(&mime).unwrap().contains("x-scheme-handler/kor=dev.kor.desktop-integration.desktop"));
    assert!(run.0.iter().any(|c| c.starts_with("update-desktop-database")));
    // Twice: same files, Thunar actions not duplicated.
    install_with(&t, &mut reg, &mut run, &apps(false), &opts(true)).unwrap();
    assert_eq!(std::fs::read_to_string(&uca).unwrap().matches("kor-integration-cli").count(), 1);
    uninstall_with(&t, &mut reg, &mut run, &apps(false)).unwrap();
    assert_eq!(std::fs::read_to_string(&uca).unwrap(), user_uca);
    assert_eq!(std::fs::read_to_string(&mime).unwrap(), "[Default Applications]\ntext/plain=gedit.desktop\n");
    assert!(!t.share.join("nautilus/scripts/Kor CLI ile aç").exists());
    assert!(!t.share.join("applications").join(LINUX_ENTRY_NAME).exists());
    assert!(!record_path(&t).exists());
}

#[test]
fn linux_skips_missing_file_managers_and_foreign_scheme_defaults() {
    let d = tempfile::tempdir().unwrap();
    let t = target(d.path(), Os::Linux, vec![Manager::Dolphin]);
    let mime = t.config.join("mimeapps.list");
    std::fs::create_dir_all(mime.parent().unwrap()).unwrap();
    std::fs::write(&mime, "[Default Applications]\nx-scheme-handler/kor=other.desktop\n").unwrap();
    install_with(&t, &mut FakeReg::default(), &mut FakeRun::default(), &apps(false), &opts(false)).unwrap();
    assert!(!t.share.join("nautilus").exists() && !t.share.join("nemo").exists());
    assert!(t.share.join("kio/servicemenus/kor.desktop").is_file());
    assert_eq!(std::fs::read_to_string(&mime).unwrap(), "[Default Applications]\nx-scheme-handler/kor=other.desktop\n");
}

#[test]
fn macos_quick_actions_are_bundles_and_removed_whole() {
    let d = tempfile::tempdir().unwrap();
    let t = target(d.path(), Os::Mac, vec![]);
    let a = Apps { desktop: Some(PathBuf::from("/Applications/Kor.app/Contents/MacOS/kor-desktop")), cli: Some(PathBuf::from("/Applications/Kor.app/Contents/MacOS/kor")) };
    let mut run = FakeRun::default();
    install_with(&t, &mut FakeReg::default(), &mut run, &a, &Options { extended: false, lang: "en".into() }).unwrap();
    let wf = t.home.join("Library/Services/Open with Kor Desktop.workflow/Contents");
    let info = std::fs::read_to_string(wf.join("Info.plist")).unwrap();
    assert!(info.contains("<string>Open with Kor Desktop</string>") && info.contains("public.folder"));
    let doc = std::fs::read_to_string(wf.join("document.wflow")).unwrap();
    assert!(doc.contains("/Applications/Kor.app/Contents/MacOS/kor-desktop --open &quot;$@&quot; &gt;/dev/null 2&gt;&amp;1 &amp;"), "{doc}");
    assert!(doc.contains("com.apple.Automator.servicesMenu"));
    let edit = std::fs::read_to_string(t.home.join("Library/Services/Edit with Kor.workflow/Contents/Info.plist")).unwrap();
    assert!(edit.contains("public.item") && !edit.contains("public.folder"));
    assert!(run.0.iter().any(|c| c.contains("pbs -update")));
    uninstall_with(&t, &mut FakeReg::default(), &mut run, &a).unwrap();
    assert_eq!(std::fs::read_dir(t.home.join("Library/Services")).unwrap().count(), 0);
}

#[test]
fn text_merges_are_exact() {
    let empty = thunar_merge("", "<action><unique-id>kor-integration-x</unique-id></action>\n");
    assert!(empty.contains("<actions>\n<action><unique-id>kor-integration-x"));
    assert_eq!(thunar_merge(&empty, ""), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n");
    assert_eq!(mime_merge("", "e.desktop", true), "[Default Applications]\nx-scheme-handler/kor=e.desktop\n");
    assert_eq!(mime_merge("[Added Associations]\n", "e.desktop", true), "[Added Associations]\n[Default Applications]\nx-scheme-handler/kor=e.desktop\n");
    let added = mime_merge("[Default Applications]\na=b\n", "e.desktop", true);
    assert_eq!(mime_merge(&added, "e.desktop", false), "[Default Applications]\na=b\n");
    assert_eq!(top_key(r"Software\Classes\*\shell\Kor.tasks\shell\bugs\command"), r"Software\Classes\*\shell\Kor.tasks");
    assert_eq!(top_key(r"Software\Classes\kor\shell\open\command"), r"Software\Classes\kor");
    assert_eq!(labels("fr").edit, "Modifier avec Kor");
}

#[test]
fn editors_get_open_in_kor_and_lose_it_on_uninstall() {
    let d = tempfile::tempdir().unwrap();
    let t = target(d.path(), Os::Windows, vec![]);
    let code = t.app_config.join("Code").join("User");
    let idea = t.app_config.join("JetBrains").join("IntelliJIdea2025.2");
    let zed = t.app_config.join("Zed");
    for p in [&code, &idea, &zed] {
        std::fs::create_dir_all(p).unwrap();
    }
    let mine = "{\n  \"version\": \"2.0.0\",\n  \"tasks\": [\n    {\n      \"label\": \"mine\"\n    }\n  ]\n}\n";
    std::fs::write(code.join("tasks.json"), mine).unwrap();
    let mut reg = FakeReg::default();
    install_with(&t, &mut reg, &mut FakeRun::default(), &apps(true), &opts(false)).unwrap();
    let vs = std::fs::read_to_string(code.join("tasks.json")).unwrap();
    assert!(vs.contains("\"label\": \"mine\"") && vs.contains("Kor'da aç") && vs.contains("${workspaceFolder}"), "{vs}");
    let z = std::fs::read_to_string(zed.join("tasks.json")).unwrap();
    assert!(z.trim_start().starts_with('[') && z.contains("$ZED_WORKTREE_ROOT"), "{z}");
    let jb = std::fs::read_to_string(idea.join("tools").join("Kor.xml")).unwrap();
    assert!(jb.contains("$ProjectFileDir$") && jb.contains(r"C:\Kor\kor-desktop.exe"), "{jb}");
    install_with(&t, &mut reg, &mut FakeRun::default(), &apps(true), &opts(false)).unwrap();
    assert_eq!(std::fs::read_to_string(code.join("tasks.json")).unwrap().matches("\"label\": \"Kor'da aç\"").count(), 1);
    uninstall_with(&t, &mut reg, &mut FakeRun::default(), &apps(true)).unwrap();
    let back: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(code.join("tasks.json")).unwrap()).unwrap();
    assert_eq!(back, serde_json::from_str::<serde_json::Value>(mine).unwrap());
    assert!(!idea.join("tools").join("Kor.xml").exists());
    // A tasks.json with comments is left alone.
    assert!(tasks_merge("// c\n{}", &[]).is_none());
}
