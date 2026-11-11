//! Provider privacy cards (spec §31.2, ADR 0057): training use, retention, region, sub-processor,
//! zero-data-retention need and a KVKK/GDPR note per provider and model, each with the policy
//! page it came from (`backend/providers/privacy/cards.toml`, url + retrieved date).
//!
//! A field the cited page does not state is [`Training::Unknown`] etc. and shown as "unknown"
//! ("bilinmiyor"); nothing is inferred. A requirement ([`PrivacyRequirement`]) is met only by a
//! known value: unknown never passes (fail closed).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use kor_core::PrivacyRequirement;
use serde::{Deserialize, Serialize};

use crate::{Category, ProviderManifest};

const DATA: &str = include_str!("../../../providers/privacy/cards.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Training {
    No,
    OptOut,
    Yes,
    #[default]
    Unknown,
}

/// Default retention of API inputs and outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Retention {
    Days(u32),
    Indefinite,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    Eu,
    Us,
    Tr,
    Cn,
    Other,
    Local,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Zdr {
    /// Zero retention only by agreement or approval.
    Required,
    /// Zero retention by default or as a self-serve setting.
    NotNeeded,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    /// `YYYY-MM-DD` the page was read.
    pub retrieved: String,
    /// Short verbatim quote that supports the card.
    #[serde(default)]
    pub quote: String,
}

/// The card of one provider, or of one model behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub provider: String,
    pub model: Option<String>,
    pub training: Training,
    pub retention: Retention,
    pub region: Region,
    /// An EU processing option exists (may need a plan or approval); `None` = not stated.
    pub eu_residency: Option<bool>,
    pub zdr: Zdr,
    /// The provider routes to other hosts (its model ids name the vendor).
    pub gateway: bool,
    /// The real processor behind a gateway (`openrouter/anthropic/…` → `anthropic`).
    pub sub_processor: Option<String>,
    pub notes: Vec<String>,
    pub sources: Vec<Source>,
    /// Runs on this machine: every value holds by construction, no source needed.
    pub local: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    #[serde(default)]
    training: Training,
    retention_days: Option<u32>,
    retention: Option<String>,
    #[serde(default)]
    zdr: Zdr,
    #[serde(default)]
    region: Region,
    eu_residency: Option<bool>,
    #[serde(default)]
    gateway: bool,
    #[serde(default)]
    notes: Vec<String>,
    sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    provider: Vec<Entry>,
}

fn table() -> &'static BTreeMap<String, Entry> {
    static T: LazyLock<BTreeMap<String, Entry>> = LazyLock::new(|| {
        // Checked by `embedded_data_is_valid_and_cited`; a broken file would fail that test.
        toml::from_str::<File>(DATA)
            .map(|f| f.provider.into_iter().map(|e| (e.id.clone(), e)).collect())
            .unwrap_or_default()
    });
    &T
}

/// Every provider id with a card in the data file.
pub fn cited_providers() -> Vec<&'static str> {
    table().keys().map(String::as_str).collect()
}

/// The card of `provider` (and `model`, a model id without the provider prefix).
pub fn card(manifest: &ProviderManifest, model: Option<&str>) -> Card {
    card_for(
        &manifest.id,
        manifest.category == Category::Local,
        model,
    )
}

/// [`card`] without a manifest (`local` = the provider runs on this machine).
pub fn card_for(provider: &str, local: bool, model: Option<&str>) -> Card {
    let model = model.map(str::to_owned).filter(|m| !m.is_empty());
    if local {
        return Card {
            provider: provider.to_owned(),
            model,
            training: Training::No,
            retention: Retention::Days(0),
            region: Region::Local,
            eu_residency: None,
            zdr: Zdr::NotNeeded,
            gateway: false,
            sub_processor: None,
            notes: Vec::new(),
            sources: Vec::new(),
            local: true,
        };
    }
    let Some(e) = table().get(provider) else {
        return Card {
            provider: provider.to_owned(),
            model,
            training: Training::Unknown,
            retention: Retention::Unknown,
            region: Region::Unknown,
            eu_residency: None,
            zdr: Zdr::Unknown,
            gateway: false,
            sub_processor: None,
            notes: Vec::new(),
            sources: Vec::new(),
            local: false,
        };
    };
    let retention = match (e.retention_days, e.retention.as_deref()) {
        (Some(d), _) => Retention::Days(d),
        (None, Some("indefinite")) => Retention::Indefinite,
        _ => Retention::Unknown,
    };
    // A gateway names the vendor in the model id (`anthropic/claude-…`); the vendor's own card
    // does not apply, because the gateway picks the host.
    let sub_processor = e
        .gateway
        .then(|| model.as_deref().and_then(|m| m.split_once('/')).map(|(v, _)| v.to_owned()))
        .flatten();
    Card {
        provider: provider.to_owned(),
        model,
        training: e.training,
        retention,
        region: e.region,
        eu_residency: e.eu_residency,
        zdr: e.zdr,
        gateway: e.gateway,
        sub_processor,
        notes: e.notes.clone(),
        sources: e.sources.clone(),
        local: false,
    }
}

impl Card {
    /// Requirements in `required` this card does not meet (unknown values never meet one).
    pub fn unmet(&self, required: &[PrivacyRequirement]) -> Vec<PrivacyRequirement> {
        required
            .iter()
            .copied()
            .filter(|r| !self.meets(*r))
            .collect()
    }

    pub fn meets(&self, r: PrivacyRequirement) -> bool {
        match r {
            PrivacyRequirement::NoTraining => self.training == Training::No,
            PrivacyRequirement::EuOnly => matches!(self.region, Region::Eu | Region::Local),
            PrivacyRequirement::TrOnly => matches!(self.region, Region::Tr | Region::Local),
            PrivacyRequirement::LocalOnly => self.region == Region::Local,
            PrivacyRequirement::ZeroRetention => {
                self.retention == Retention::Days(0) && self.zdr != Zdr::Required
            }
        }
    }

    /// Fluent key of the KVKK/GDPR note: where the data goes decides which transfer rules apply.
    pub fn law_note_key(&self) -> &'static str {
        match self.region {
            Region::Local => "privcard-law-local",
            Region::Eu => "privcard-law-eu",
            Region::Tr => "privcard-law-tr",
            Region::Us | Region::Cn | Region::Other => "privcard-law-abroad",
            Region::Unknown => "privcard-law-unknown",
        }
    }
}

/// `--privacy-mode` pulls the filter to the strictest set; otherwise `configured`.
pub fn effective_requirements(
    configured: &[PrivacyRequirement],
    privacy_mode: bool,
) -> Vec<PrivacyRequirement> {
    let mut v: Vec<PrivacyRequirement> = configured.to_vec();
    if privacy_mode {
        v.extend(PrivacyRequirement::STRICTEST);
    }
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_data_is_valid_and_cited() {
        let f: File = toml::from_str(DATA).expect("privacy/cards.toml parses");
        assert!(!f.provider.is_empty());
        for e in &f.provider {
            assert!(!e.sources.is_empty(), "{}: every card cites a page", e.id);
            for s in &e.sources {
                assert!(s.url.starts_with("https://"), "{}: {}", e.id, s.url);
                assert!(
                    chrono_like_date(&s.retrieved),
                    "{}: retrieved {}",
                    e.id,
                    s.retrieved
                );
                assert!(s.quote.split_whitespace().count() < 15, "{}: quote too long", e.id);
            }
            assert!(
                e.retention.is_none() || e.retention.as_deref() == Some("indefinite"),
                "{}",
                e.id
            );
        }
        // Every id is a shipped provider manifest.
        let reg = crate::Registry::builtin();
        for e in &f.provider {
            assert!(reg.get(&e.id).is_some(), "unknown provider {}", e.id);
        }
    }

    fn chrono_like_date(s: &str) -> bool {
        let b = s.as_bytes();
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && s.chars().filter(char::is_ascii_digit).count() == 8
    }

    #[test]
    fn unknown_fields_never_meet_a_requirement() {
        let c = card_for("no-such-provider", false, Some("m"));
        assert_eq!(c.training, Training::Unknown);
        assert_eq!(c.region, Region::Unknown);
        assert_eq!(c.unmet(&PrivacyRequirement::ALL).len(), PrivacyRequirement::ALL.len());
        assert!(c.sources.is_empty());
    }

    #[test]
    fn cited_values_and_local_cards() {
        let a = card_for("anthropic", false, Some("claude-x"));
        assert_eq!(a.training, Training::No);
        assert_eq!(a.retention, Retention::Days(30));
        assert_eq!(a.zdr, Zdr::Required);
        assert_eq!(a.region, Region::Unknown);
        assert!(a.meets(PrivacyRequirement::NoTraining));
        assert!(!a.meets(PrivacyRequirement::EuOnly));
        assert!(!a.meets(PrivacyRequirement::ZeroRetention));

        let l = card_for("local", true, Some("qwen3"));
        assert!(l.unmet(&PrivacyRequirement::ALL).is_empty());
        assert_eq!(l.law_note_key(), "privcard-law-local");

        let o = card_for("openrouter", false, Some("anthropic/claude-sonnet"));
        assert_eq!(o.sub_processor.as_deref(), Some("anthropic"));
        assert_eq!(o.training, Training::Unknown);

        let f = card_for("fireworks", false, None);
        assert!(f.meets(PrivacyRequirement::ZeroRetention));
    }

    #[test]
    fn privacy_mode_is_the_strictest_filter() {
        let e = effective_requirements(&[PrivacyRequirement::EuOnly], true);
        assert!(e.contains(&PrivacyRequirement::LocalOnly));
        assert!(e.contains(&PrivacyRequirement::EuOnly));
        assert_eq!(
            effective_requirements(&[PrivacyRequirement::NoTraining], false),
            vec![PrivacyRequirement::NoTraining]
        );
    }
}
