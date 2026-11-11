use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How much a provider can be trusted with prompt data, weakest first.
///
/// The ordering is meaningful: `privacy.min_tier = "no_train"` admits
/// `NoTrain`, `NoLog` and `Local`, and nothing below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyTier {
    /// No published data policy.
    Unknown,
    /// Data may be retained and possibly used for training.
    Standard,
    /// Provider states data is not used for training.
    NoTrain,
    /// Provider states prompts and responses are not stored.
    NoLog,
    /// Never leaves this machine.
    Local,
}

impl PrivacyTier {
    pub const ALL: [Self; 5] = [
        Self::Unknown,
        Self::Standard,
        Self::NoTrain,
        Self::NoLog,
        Self::Local,
    ];

    pub fn satisfies(self, minimum: Self) -> bool {
        self >= minimum
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Standard => "standard",
            Self::NoTrain => "no_train",
            Self::NoLog => "no_log",
            Self::Local => "local",
        }
    }
}

impl fmt::Display for PrivacyTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PrivacyTier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| format!("unknown privacy tier `{s}`"))
    }
}

/// One condition of `privacy.require` (spec §31.2): a model that does not meet it — or whose
/// provider card leaves the field unknown — is dimmed in the picker and skipped by the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyRequirement {
    /// Inputs are not used for training by default (opt-out does not count).
    NoTraining,
    /// Processed in the EU (or on this machine).
    EuOnly,
    /// Processed in Türkiye (or on this machine).
    TrOnly,
    /// Never leaves this machine.
    LocalOnly,
    /// Nothing retained by default, without a special agreement.
    ZeroRetention,
}

impl PrivacyRequirement {
    pub const ALL: [Self; 5] = [
        Self::NoTraining,
        Self::EuOnly,
        Self::TrOnly,
        Self::LocalOnly,
        Self::ZeroRetention,
    ];

    /// What `--privacy-mode` enforces: the strictest set.
    pub const STRICTEST: [Self; 3] = [Self::LocalOnly, Self::NoTraining, Self::ZeroRetention];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoTraining => "no-training",
            Self::EuOnly => "eu-only",
            Self::TrOnly => "tr-only",
            Self::LocalOnly => "local-only",
            Self::ZeroRetention => "zero-retention",
        }
    }

    /// `"no-training, eu-only"` (what `kor config set privacy.require …` writes) or one token.
    ///
    /// # Errors
    /// On an unknown token, naming it.
    pub fn parse_list(s: &str) -> Result<Vec<Self>, String> {
        let mut out: Vec<Self> = Vec::new();
        for t in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let r: Self = t.parse()?;
            if !out.contains(&r) {
                out.push(r);
            }
        }
        out.sort();
        Ok(out)
    }
}

impl fmt::Display for PrivacyRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PrivacyRequirement {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == s.trim())
            .ok_or_else(|| {
                let known: Vec<&str> = Self::ALL.iter().map(|r| r.as_str()).collect();
                format!(
                    "unknown privacy requirement `{s}` (known: {})",
                    known.join(", ")
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_lists_parse_dedupe_and_reject_unknown() {
        assert_eq!(
            PrivacyRequirement::parse_list("eu-only, no-training,eu-only").unwrap(),
            vec![PrivacyRequirement::NoTraining, PrivacyRequirement::EuOnly]
        );
        assert!(PrivacyRequirement::parse_list("").unwrap().is_empty());
        assert!(PrivacyRequirement::parse_list("no-training,mars-only").is_err());
    }

    #[test]
    fn ordering_drives_policy() {
        assert!(PrivacyTier::Local.satisfies(PrivacyTier::NoTrain));
        assert!(PrivacyTier::NoTrain.satisfies(PrivacyTier::NoTrain));
        assert!(!PrivacyTier::Standard.satisfies(PrivacyTier::NoTrain));
        assert!(!PrivacyTier::Unknown.satisfies(PrivacyTier::Standard));
    }

    #[test]
    fn string_round_trip() {
        for t in PrivacyTier::ALL {
            assert_eq!(t.as_str().parse::<PrivacyTier>().unwrap(), t);
        }
    }
}
