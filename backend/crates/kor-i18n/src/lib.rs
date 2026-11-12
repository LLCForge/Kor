//! User-facing text for Kor in Turkish, English and French (Fluent).
//!
//! Translations are compiled into the binary; English is the fallback for any
//! message id missing from the active locale.

use std::fmt;
use std::str::FromStr;

use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;
use serde::{Deserialize, Serialize};
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::{FluentArgs, FluentValue};

/// A supported interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Tr,
    Fr,
}

impl Lang {
    pub const ALL: [Self; 3] = [Self::En, Self::Tr, Self::Fr];

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Tr => "tr",
            Self::Fr => "fr",
        }
    }

    /// Accepts BCP 47 and POSIX forms: `tr`, `tr-TR`, `fr_CA.UTF-8`, `EN`.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag.split(['-', '_', '.', '@']).next()?;
        Self::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(primary))
    }

    /// The OS locale, if it is one we support.
    pub fn from_system() -> Option<Self> {
        sys_locale::get_locales().find_map(|l| Self::from_tag(&l))
    }

    fn source(self) -> &'static str {
        match self {
            Self::En => include_str!("../locales/en/kor.ftl"),
            Self::Tr => include_str!("../locales/tr/kor.ftl"),
            Self::Fr => include_str!("../locales/fr/kor.ftl"),
        }
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Lang {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|l| l.code() == s)
            .ok_or_else(|| format!("unsupported language `{s}` (expected tr, en or fr)"))
    }
}

/// A language setting that may defer to the environment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LangPref {
    #[default]
    Auto,
    En,
    Tr,
    Fr,
}

impl LangPref {
    pub fn fixed(self) -> Option<Lang> {
        match self {
            Self::Auto => None,
            Self::En => Some(Lang::En),
            Self::Tr => Some(Lang::Tr),
            Self::Fr => Some(Lang::Fr),
        }
    }

    /// Fixed choice, else the OS locale, else English.
    pub fn resolve(self) -> Lang {
        self.fixed().or_else(Lang::from_system).unwrap_or(Lang::En)
    }
}

impl From<Lang> for LangPref {
    fn from(l: Lang) -> Self {
        match l {
            Lang::En => Self::En,
            Lang::Tr => Self::Tr,
            Lang::Fr => Self::Fr,
        }
    }
}

fn bundle(lang: Lang) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = lang.code().parse().expect("static language code");
    let mut b = FluentBundle::new_concurrent(vec![id]);
    // Unicode isolation marks garble terminal output.
    b.set_use_isolating(false);
    let res = FluentResource::try_new(lang.source().to_owned())
        .unwrap_or_else(|(_, e)| panic!("{lang}: invalid .ftl: {e:?}"));
    b.add_resource(res)
        .unwrap_or_else(|e| panic!("{lang}: duplicate .ftl ids: {e:?}"));
    b
}

/// Formats messages for one language with English fallback.
pub struct Localizer {
    lang: Lang,
    primary: FluentBundle<FluentResource>,
    fallback: Option<FluentBundle<FluentResource>>,
}

impl fmt::Debug for Localizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Localizer")
            .field("lang", &self.lang)
            .finish_non_exhaustive()
    }
}

impl Localizer {
    pub fn new(lang: Lang) -> Self {
        Self {
            lang,
            primary: bundle(lang),
            fallback: (lang != Lang::En).then(|| bundle(Lang::En)),
        }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    pub fn t(&self, id: &str) -> String {
        self.format(id, None)
    }

    /// Whether `id` exists in this language (the English fallback does not count).
    pub fn has(&self, id: &str) -> bool {
        self.primary.has_message(id)
    }

    pub fn t_args(&self, id: &str, args: &FluentArgs<'_>) -> String {
        self.format(id, Some(args))
    }

    fn format(&self, id: &str, args: Option<&FluentArgs<'_>>) -> String {
        for b in std::iter::once(&self.primary).chain(&self.fallback) {
            if let Some(pattern) = b.get_message(id).and_then(|m| m.value()) {
                let mut errors = Vec::new();
                return b.format_pattern(pattern, args, &mut errors).into_owned();
            }
        }
        // Visible but harmless; the key-integrity test keeps this from shipping.
        id.to_owned()
    }
}

/// Builds `FluentArgs` from `(name, value)` pairs.
#[macro_export]
macro_rules! args {
    ($($k:literal => $v:expr),* $(,)?) => {{
        let mut a = $crate::FluentArgs::new();
        $( a.set($k, $v); )*
        a
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn ids(src: &str) -> BTreeSet<&str> {
        src.lines()
            .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()))
            .filter_map(|l| l.split_once(" =").map(|(k, _)| k.trim()))
            .collect()
    }

    #[test]
    fn every_locale_has_the_same_keys() {
        let en = ids(Lang::En.source());
        assert!(!en.is_empty());
        for lang in [Lang::Tr, Lang::Fr] {
            let other = ids(lang.source());
            let missing: Vec<_> = en.difference(&other).collect();
            let extra: Vec<_> = other.difference(&en).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{lang}: missing {missing:?}, extra {extra:?}"
            );
        }
    }

    #[test]
    fn all_bundles_parse_and_format() {
        for lang in Lang::ALL {
            let l = Localizer::new(lang);
            for id in ids(lang.source()) {
                let out = l.t_args(id, &args!("count" => 2, "failures" => 0));
                assert!(!out.is_empty(), "{lang}:{id}");
            }
        }
    }

    #[test]
    fn plurals_follow_locale_rules() {
        let en = Localizer::new(Lang::En);
        let fr = Localizer::new(Lang::Fr);
        let tr = Localizer::new(Lang::Tr);
        assert_eq!(
            en.t_args("doctor-summary", &args!("failures" => 0)),
            "All checks passed."
        );
        assert!(
            en.t_args("doctor-summary", &args!("failures" => 2))
                .contains('2')
        );
        assert!(
            fr.t_args("doctor-summary", &args!("failures" => 1))
                .contains("1 vérification")
        );
        assert!(
            tr.t_args("doctor-summary", &args!("failures" => 3))
                .contains("3 kontrol")
        );
    }

    #[test]
    fn tag_parsing() {
        assert_eq!(Lang::from_tag("tr-TR"), Some(Lang::Tr));
        assert_eq!(Lang::from_tag("fr_CA.UTF-8"), Some(Lang::Fr));
        assert_eq!(Lang::from_tag("EN"), Some(Lang::En));
        assert_eq!(Lang::from_tag("de-DE"), None);
        assert_eq!(LangPref::Tr.resolve(), Lang::Tr);
    }

    #[test]
    fn unknown_id_falls_back_to_id() {
        assert_eq!(Localizer::new(Lang::Tr).t("no-such-id"), "no-such-id");
    }
}
