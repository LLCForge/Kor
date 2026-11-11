//! Per-application tiers (§19.1): what desktop control may do while an app is in front.
//!
//! Built-in defaults by app class, overridden per app by `computer.apps.<name>` in the config:
//! terminals, password managers, banking / wallet apps and Kor itself are `blocked`; browsers are
//! `read` (the Chrome extension, §18, drives the web); IDEs and editors are `click`; anything else
//! `ask` (every step approved). `full` skips the approval except for destructive key combinations.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

pub use kor_config::ComputerTier as Tier;
use serde::Serialize;

/// The kind of thing a desktop tool does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Screenshot, accessibility tree, window list, clipboard read.
    Observe,
    /// Left / double click, scroll, pointer move, focusing a window.
    Click,
    /// Right click, drag.
    Pointer,
    /// Typing, keys, setting values, clipboard write.
    Type,
    /// Move, resize, minimize, close windows.
    Window,
    /// Start a program.
    Launch,
}

impl Action {
    pub fn key(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Click => "click",
            Self::Pointer => "pointer",
            Self::Type => "type",
            Self::Window => "window",
            Self::Launch => "launch",
        }
    }
}

/// Why an app has its tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Kor,
    Terminal,
    PasswordManager,
    Banking,
    Browser,
    Ide,
    Other,
}

impl Class {
    pub fn key(self) -> &'static str {
        match self {
            Self::Kor => "kor",
            Self::Terminal => "terminal",
            Self::PasswordManager => "password_manager",
            Self::Banking => "banking",
            Self::Browser => "browser",
            Self::Ide => "ide",
            Self::Other => "other",
        }
    }

    pub fn default_tier(self) -> Tier {
        match self {
            Self::Kor | Self::Terminal | Self::PasswordManager | Self::Banking => Tier::Blocked,
            Self::Browser => Tier::Read,
            Self::Ide => Tier::Click,
            Self::Other => Tier::Ask,
        }
    }
}

const KOR: &[&str] = &["kor", "kor-desktop"];
const TERMINALS: &[&str] = &[
    "cmd",
    "powershell",
    "pwsh",
    "windowsterminal",
    "wt",
    "conhost",
    "openconsole",
    "mintty",
    "putty",
    "terminal",
    "iterm",
    "iterm2",
    "alacritty",
    "kitty",
    "wezterm",
    "wezterm-gui",
    "hyper",
    "warp",
    "ghostty",
    "tabby",
    "gnome-terminal",
    "gnome-terminal-server",
    "kgx",
    "konsole",
    "xterm",
    "tilix",
    "terminator",
    "xfce4-terminal",
    "foot",
    "st",
    "urxvt",
    "rxvt",
    "terminology",
    "termius",
];
const PASSWORD_MANAGERS: &[&str] = &[
    "1password",
    "keepass",
    "keepassxc",
    "bitwarden",
    "lastpass",
    "dashlane",
    "enpass",
    "keeper",
    "nordpass",
    "proton pass",
    "passwords",
    "keychain access",
    "seahorse",
    "kwalletmanager5",
    "roboform",
    "authy",
    "authy desktop",
];
const BANKING_WORDS: &[&str] = &[
    "bank",
    "banka",
    "ziraat",
    "isbank",
    "garanti",
    "akbank",
    "yapikredi",
    "halkbank",
    "vakifbank",
    "denizbank",
    "enpara",
    "papara",
    "paypal",
    "revolut",
    "wise",
    "n26",
    "monzo",
    "coinbase",
    "binance",
    "kraken",
    "ledger live",
    "trezor suite",
    "exodus",
    "electrum",
    "metamask",
    "wallet",
];
const BROWSERS: &[&str] = &[
    "chrome",
    "google chrome",
    "google-chrome",
    "chromium",
    "chromium-browser",
    "msedge",
    "microsoft edge",
    "firefox",
    "safari",
    "brave",
    "brave browser",
    "opera",
    "vivaldi",
    "arc",
    "librewolf",
    "waterfox",
    "zen",
    "yandex",
    "orion",
];
const IDES: &[&str] = &[
    "code",
    "code - insiders",
    "visual studio code",
    "cursor",
    "windsurf",
    "zed",
    "idea",
    "idea64",
    "intellij idea",
    "pycharm",
    "pycharm64",
    "webstorm",
    "webstorm64",
    "clion",
    "clion64",
    "goland",
    "goland64",
    "rider",
    "rider64",
    "rustrover",
    "rustrover64",
    "phpstorm",
    "phpstorm64",
    "devenv",
    "xcode",
    "sublime_text",
    "sublime text",
    "notepad++",
    "studio64",
    "android studio",
    "eclipse",
    "fleet",
    "nvim-qt",
    "neovide",
    "emacs",
    "gvim",
];

/// The class of an app by its (normalised) name.
pub fn classify(app: &str) -> Class {
    let a = kor_platform::desktop_probe::app_name(app);
    if KOR.contains(&a.as_str()) {
        Class::Kor
    } else if TERMINALS.contains(&a.as_str()) {
        Class::Terminal
    } else if PASSWORD_MANAGERS.contains(&a.as_str()) {
        Class::PasswordManager
    } else if BANKING_WORDS.iter().any(|w| a.contains(w)) {
        Class::Banking
    } else if BROWSERS.contains(&a.as_str()) {
        Class::Browser
    } else if IDES.contains(&a.as_str()) {
        Class::Ide
    } else {
        Class::Other
    }
}

/// Where an app's tier came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Config,
    Default,
}

/// The configured overrides (`computer.apps`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub apps: BTreeMap<String, Tier>,
}

impl Policy {
    pub fn from_config(c: &kor_config::Computer) -> Self {
        Self {
            apps: c
                .apps
                .iter()
                .map(|(k, v)| (kor_platform::desktop_probe::app_name(k), *v))
                .collect(),
        }
    }

    pub fn tier(&self, app: &str) -> (Tier, Source) {
        let a = kor_platform::desktop_probe::app_name(app);
        match self.apps.get(&a) {
            Some(t) => (*t, Source::Config),
            None => (classify(&a).default_tier(), Source::Default),
        }
    }
}

/// The config key for an app's tier (`computer.apps.<app>`); app names are letters, digits,
/// spaces, `-`, `_`, `+` (no dots: they would split the key).
pub fn config_key(app: &str) -> Result<String, String> {
    let a = kor_platform::desktop_probe::app_name(app);
    if a.is_empty()
        || a.len() > 64
        || !a
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '+'))
    {
        return Err(format!("invalid app name `{app}`"));
    }
    Ok(format!("computer.apps.{a}"))
}

/// Whether a tier permits an action at all.
pub fn permits(tier: Tier, action: Action) -> bool {
    match tier {
        Tier::Blocked => false,
        Tier::Read => action == Action::Observe,
        Tier::Click => matches!(action, Action::Observe | Action::Click),
        Tier::Ask | Tier::Full => true,
    }
}

/// The decision for one desktop action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run without asking (`full` tier, nothing destructive).
    Allow,
    /// Run after the user approves it.
    Ask,
    /// Not allowed in this app; the message is for the model and the audit log.
    Refuse(String),
}

/// `app`: the app in front (or the target window's / program's app); `None` when unknown.
pub fn verdict(policy: &Policy, app: Option<&str>, action: Action, destructive: bool) -> Verdict {
    let Some(app) = app else {
        return Verdict::Ask;
    };
    let (tier, source) = policy.tier(app);
    if !permits(tier, action) {
        let why = match source {
            Source::Config => "your setting".to_owned(),
            Source::Default => format!("default for a {}", classify(app).key().replace('_', " ")),
        };
        return Verdict::Refuse(format!(
            "desktop control refused: `{app}` is `{}` for Kor ({why}); `{}` actions are not allowed there. Do not retry; ask the user, who can change it with `kor computer allow {app} <tier>`.",
            tier.key(),
            action.key()
        ));
    }
    if tier == Tier::Full && !destructive {
        Verdict::Allow
    } else {
        Verdict::Ask
    }
}

static POLICY: RwLock<Option<Policy>> = RwLock::new(None);

/// Set from the config when a session is prepared (and when the desktop changes a tier).
pub fn set_policy(p: Policy) {
    *POLICY.write().unwrap_or_else(PoisonError::into_inner) = Some(p);
}

/// The current policy (built-in defaults until [`set_policy`]).
pub fn current() -> Policy {
    POLICY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// One row per app with a configured tier plus the built-in classes, for status screens.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub app: String,
    pub tier: Tier,
    pub source: Source,
    pub class: Class,
}

pub fn rows(p: &Policy) -> Vec<Row> {
    p.apps
        .iter()
        .map(|(app, tier)| Row {
            app: app.clone(),
            tier: *tier,
            source: Source::Config,
            class: classify(app),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_restrict_terminals_browsers_and_secrets() {
        let p = Policy::default();
        assert_eq!(p.tier("WindowsTerminal.exe").0, Tier::Blocked);
        assert_eq!(p.tier("gnome-terminal-server").0, Tier::Blocked);
        assert_eq!(p.tier("KeePassXC").0, Tier::Blocked);
        assert_eq!(p.tier("Ziraat Mobil").0, Tier::Blocked);
        assert_eq!(p.tier("kor-desktop").0, Tier::Blocked);
        assert_eq!(p.tier(r"C:\x\chrome.exe").0, Tier::Read);
        assert_eq!(p.tier("Code").0, Tier::Click);
        assert_eq!(p.tier("mspaint").0, Tier::Ask);
    }

    #[test]
    fn config_overrides_and_verdicts() {
        let mut c = kor_config::Computer::default();
        c.apps.insert("Chrome.exe".into(), Tier::Full);
        c.apps.insert("mspaint".into(), Tier::Blocked);
        let p = Policy::from_config(&c);
        assert_eq!(p.tier("chrome"), (Tier::Full, Source::Config));
        assert_eq!(
            verdict(&p, Some("chrome"), Action::Type, false),
            Verdict::Allow
        );
        assert_eq!(
            verdict(&p, Some("chrome"), Action::Type, true),
            Verdict::Ask
        );
        assert!(
            matches!(verdict(&p, Some("mspaint"), Action::Observe, false), Verdict::Refuse(m) if m.contains("your setting"))
        );

        let d = Policy::default();
        assert_eq!(
            verdict(&d, Some("firefox"), Action::Observe, false),
            Verdict::Ask
        );
        assert!(
            matches!(verdict(&d, Some("firefox"), Action::Click, false), Verdict::Refuse(m) if m.contains("default for a browser"))
        );
        assert_eq!(
            verdict(&d, Some("code"), Action::Click, false),
            Verdict::Ask
        );
        assert!(matches!(
            verdict(&d, Some("code"), Action::Type, false),
            Verdict::Refuse(_)
        ));
        assert!(matches!(
            verdict(&d, Some("pwsh"), Action::Observe, false),
            Verdict::Refuse(_)
        ));
        assert_eq!(verdict(&d, None, Action::Type, false), Verdict::Ask);
        assert_eq!(rows(&p).len(), 2);
    }

    #[test]
    fn tiers_round_trip_through_the_config_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("config.toml");
        let key = config_key("Visual Studio Code.app").unwrap();
        assert_eq!(key, "computer.apps.visual studio code");
        assert!(config_key("a.b").is_err() && config_key("").is_err());
        kor_config::set_string(&f, &key, "full").unwrap();
        kor_config::set_string(&f, &config_key("mspaint").unwrap(), "blocked").unwrap();
        assert!(kor_config::set_string(&f, &config_key("x").unwrap(), "sometimes").is_err());
        let paths = kor_config::Paths {
            system: None,
            user: Some(f.clone()),
            project: None,
        };
        let c = kor_config::load(&paths, std::iter::empty(), &[])
            .unwrap()
            .config;
        let p = Policy::from_config(&c.computer);
        assert_eq!(p.tier("visual studio code"), (Tier::Full, Source::Config));
        assert_eq!(p.tier("mspaint").0, Tier::Blocked);
        assert_eq!(c.health, kor_config::Health::default());
        kor_config::unset(&f, &config_key("mspaint").unwrap()).unwrap();
        kor_config::unset(&f, "computer.apps.never-set").unwrap();
        let c = kor_config::load(&paths, std::iter::empty(), &[])
            .unwrap()
            .config;
        assert_eq!(
            Policy::from_config(&c.computer).tier("mspaint"),
            (Tier::Ask, Source::Default)
        );
    }
}
