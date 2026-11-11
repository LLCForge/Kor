//! The intent catalog: built-in `assets/intents/intent.toml` merged with `parity.toml` (every
//! command's description, TUI slash path and desktop method), plugin `intent.toml` files and the
//! user's own patterns.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::matcher;

const BUILTIN: &str = include_str!("../../../../../assets/intents/intent.toml");
const PARITY: &str = include_str!("../../../../../parity.toml");
/// Plugin catalogs larger than this are ignored.
const MAX_PLUGIN_FILE: u64 = 256 * 1024;

/// What routing may do with a command without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Run {
    /// Local and read-only: runs without asking.
    Auto,
    /// Writes, costs money or uses the network: shown with the exact command and confirmed.
    Confirm,
    /// Never chosen from free text (interactive entry points, internal commands).
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Path,
    Text,
    Number,
    Url,
    Domain,
}

/// One argument of a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub required: bool,
    /// `--flag value` instead of a positional argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    command: String,
    run: Run,
    #[serde(default)]
    tr: Vec<String>,
    #[serde(default)]
    en: Vec<String>,
    #[serde(default)]
    fr: Vec<String>,
    #[serde(default)]
    params: Vec<Param>,
    tokens: Option<u64>,
    eta_s: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct File {
    #[serde(default)]
    intent: Vec<Raw>,
}

/// Where an intent came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "lowercase")]
pub enum Source {
    Builtin,
    Plugin(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Phrase {
    /// `tr`, `en`, `fr`, `parity` (the command's parity.toml description) or `user`.
    pub lang: String,
    pub text: String,
    #[serde(skip)]
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Intent {
    /// `kor cost`, `kor sandbox test`, or a plugin's `/command`.
    pub command: String,
    pub run: Run,
    /// parity.toml description (Turkish), or the command itself.
    pub name: String,
    pub tui: Option<String>,
    pub desktop: Option<String>,
    pub phrases: Vec<Phrase>,
    pub params: Vec<Param>,
    pub tokens: Option<u64>,
    pub eta_s: Option<u32>,
    pub source: Source,
    /// Every content word of the command's phrases (for parameter slots: what is left is a value).
    #[serde(skip)]
    pub vocab: Vec<String>,
    /// Words in at least two of its phrases or in the command itself: never an argument value
    /// (a word in one phrase only may be an example value, `chromium` in "install chromium").
    #[serde(skip)]
    pub strong: Vec<String>,
}

impl Intent {
    fn phrase(lang: &str, text: &str) -> Phrase {
        Phrase {
            lang: lang.to_owned(),
            text: text.to_owned(),
            tokens: matcher::tokens(text),
        }
    }

    pub(crate) fn add_phrase(&mut self, lang: &str, text: &str) {
        let p = Self::phrase(lang, text);
        for t in &p.tokens {
            if self.vocab.contains(t) {
                if !self.strong.contains(t) && self.phrases.iter().any(|q| q.tokens.contains(t)) {
                    self.strong.push(t.clone());
                }
            } else {
                self.vocab.push(t.clone());
            }
        }
        self.phrases.push(p);
    }

    fn from_raw(r: Raw, source: Source) -> Self {
        let mut i = Self {
            name: r.command.clone(),
            command: r.command,
            run: r.run,
            tui: None,
            desktop: None,
            phrases: Vec::new(),
            params: r.params,
            tokens: r.tokens,
            eta_s: r.eta_s,
            source,
            vocab: Vec::new(),
            strong: Vec::new(),
        };
        for (lang, list) in [("tr", r.tr), ("en", r.en), ("fr", r.fr)] {
            for p in list {
                i.add_phrase(lang, &p);
            }
        }
        // Words of the command itself (`sandbox`, `test`) name it too.
        let words = i.command.trim_start_matches("kor").replace(['-', '/'], " ");
        for t in matcher::tokens(&words) {
            if !i.vocab.contains(&t) {
                i.vocab.push(t.clone());
            }
            if !i.strong.contains(&t) {
                i.strong.push(t);
            }
        }
        i
    }
}

/// parity.toml rows: `cli`, `name`, `tui`, `desktop`, `internal`.
#[derive(Debug, Clone, Deserialize)]
pub struct Feature {
    pub name: String,
    pub cli: String,
    #[serde(default)]
    pub tui: Option<String>,
    #[serde(default)]
    pub desktop: Option<String>,
    #[serde(default)]
    pub internal: bool,
}

#[derive(Debug, Deserialize)]
struct Parity {
    #[serde(default)]
    feature: Vec<Feature>,
}

/// Every feature of the embedded parity manifest.
///
/// # Panics
/// Never at runtime: the embedded file is checked by the tests.
pub fn parity() -> Vec<Feature> {
    toml::from_str::<Parity>(PARITY)
        .map(|p| p.feature)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Catalog {
    pub intents: Vec<Intent>,
    /// Plugin files that could not be read (shown by `kor intents list`).
    pub errors: Vec<String>,
}

impl Catalog {
    /// Built-in intents, generated from parity.toml plus the hand-written phrases.
    pub fn builtin() -> Self {
        let mut cat = Self::parse(BUILTIN, &Source::Builtin).unwrap_or_default();
        for f in parity() {
            let gap = |v: &Option<String>| {
                v.clone()
                    .filter(|s| s.trim() != "-" && !s.trim().is_empty())
            };
            if let Some(i) = cat.intents.iter_mut().find(|i| i.command == f.cli) {
                i.name.clone_from(&f.name);
                i.tui = gap(&f.tui).filter(|t| t.starts_with('/'));
                i.desktop = gap(&f.desktop);
                if f.internal {
                    i.run = Run::Never;
                }
                // The description before its parenthesis is one more Turkish phrase.
                let short = f.name.split('(').next().unwrap_or_default().trim();
                if i.run != Run::Never && !short.is_empty() {
                    i.add_phrase("parity", short);
                }
            }
        }
        cat
    }

    /// A catalog file in the `intent.toml` format.
    ///
    /// # Errors
    /// When the TOML is invalid.
    pub fn parse(text: &str, source: &Source) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        Ok(Self {
            intents: file
                .intent
                .into_iter()
                .map(|r| Intent::from_raw(r, source.clone()))
                .collect(),
            errors: Vec::new(),
        })
    }

    /// Add a plugin's `intent.toml`. Plugin intents always ask first (they cannot declare
    /// `auto`), and a plugin cannot redefine a built-in command: its phrases are added to it.
    pub fn add_plugin(&mut self, id: &str, file: &Path) {
        let text = match std::fs::metadata(file) {
            Ok(m) if m.len() <= MAX_PLUGIN_FILE => {
                std::fs::read_to_string(file).map_err(|e| e.to_string())
            }
            Ok(_) => Err("file too large".into()),
            Err(e) => Err(e.to_string()),
        };
        match text.and_then(|t| Self::parse(&t, &Source::Plugin(id.to_owned()))) {
            Ok(c) => {
                for mut i in c.intents {
                    if let Some(existing) = self.intents.iter_mut().find(|x| x.command == i.command)
                    {
                        for p in i.phrases {
                            existing.add_phrase(&p.lang, &p.text);
                        }
                        continue;
                    }
                    if i.run == Run::Auto {
                        i.run = Run::Confirm;
                    }
                    self.intents.push(i);
                }
            }
            Err(e) => self.errors.push(format!("{}: {e}", file.display())),
        }
    }

    pub fn get(&self, command: &str) -> Option<&Intent> {
        self.intents.iter().find(|i| i.command == command)
    }

    pub(crate) fn get_mut(&mut self, command: &str) -> Option<&mut Intent> {
        self.intents.iter_mut().find(|i| i.command == command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_parity_command_has_an_intent_with_phrases() {
        // §30.2: a capability without intents is rejected by CI.
        let cat = Catalog::builtin();
        let mut missing = Vec::new();
        let mut thin = Vec::new();
        for f in parity() {
            match cat.get(&f.cli) {
                None => missing.push(f.cli.clone()),
                Some(i) if i.run != Run::Never => {
                    let n = |l: &str| i.phrases.iter().filter(|p| p.lang == l).count();
                    if n("tr") < 3 || n("en") < 3 || n("fr") < 2 {
                        thin.push(f.cli.clone());
                    }
                }
                Some(_) => {}
            }
        }
        assert!(
            missing.is_empty(),
            "add these commands to assets/intents/intent.toml: {missing:?}"
        );
        assert!(
            thin.is_empty(),
            "need ≥3 tr, ≥3 en, ≥2 fr phrases: {thin:?}"
        );
        let mut seen = std::collections::HashSet::new();
        for i in &cat.intents {
            assert!(seen.insert(&i.command), "duplicate intent {}", i.command);
            assert!(i.command.starts_with("kor"), "{}", i.command);
        }
        let cost = cat.get("kor cost").unwrap();
        assert_eq!(cost.run, Run::Auto);
        assert_eq!(cost.tui.as_deref(), Some("/cost"));
        assert!(cost.desktop.is_some());
        assert!(
            cost.vocab
                .iter()
                .any(|w| w == "maliyetimi" || w.starts_with("maliyet"))
        );
    }

    #[test]
    fn plugins_cannot_run_without_asking_or_replace_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("intent.toml");
        std::fs::write(
            &f,
            "[[intent]]\ncommand = \"/seo-quick\"\nrun = \"auto\"\ntr = [\"siteye bak\"]\n\n[[intent]]\ncommand = \"kor cost\"\nrun = \"never\"\nen = [\"money gone\"]\n",
        )
        .unwrap();
        let mut cat = Catalog::builtin();
        cat.add_plugin("seo@market", &f);
        let seo = cat.get("/seo-quick").unwrap();
        assert_eq!(seo.run, Run::Confirm);
        assert_eq!(seo.source, Source::Plugin("seo@market".into()));
        let cost = cat.get("kor cost").unwrap();
        assert_eq!(cost.run, Run::Auto);
        assert!(cost.phrases.iter().any(|p| p.text == "money gone"));
        std::fs::write(&f, "not toml [").unwrap();
        cat.add_plugin("broken", &f);
        assert_eq!(cat.errors.len(), 1);
    }
}
