use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `provider/model`, e.g. `anthropic/claude-sonnet-4-5` or `huggingface/org/model:groq`.
///
/// Only the first `/` separates provider from model; the model part may contain more.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelRef {
    provider: String,
    model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelRefError {
    #[error("model reference must look like `provider/model`")]
    MissingSeparator,
    #[error("invalid provider id `{0}` (allowed: a-z, 0-9, `-`, `_`; 1-64 chars)")]
    InvalidProvider(String),
    #[error("model name is empty or longer than 256 characters")]
    InvalidModel,
}

/// Provider ids are used in env var names, keychain entries and file names.
pub fn is_valid_provider_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl ModelRef {
    pub fn new(provider: &str, model: &str) -> Result<Self, ModelRefError> {
        if !is_valid_provider_id(provider) {
            return Err(ModelRefError::InvalidProvider(provider.to_owned()));
        }
        if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
            return Err(ModelRefError::InvalidModel);
        }
        Ok(Self {
            provider: provider.to_owned(),
            model: model.to_owned(),
        })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

impl FromStr for ModelRef {
    type Err = ModelRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (p, m) = s.split_once('/').ok_or(ModelRefError::MissingSeparator)?;
        Self::new(p, m)
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

impl Serialize for ModelRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ModelRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_model_names() {
        let r: ModelRef = "huggingface/meta-llama/Llama-3:groq".parse().unwrap();
        assert_eq!(r.provider(), "huggingface");
        assert_eq!(r.model(), "meta-llama/Llama-3:groq");
        assert_eq!(r.to_string(), "huggingface/meta-llama/Llama-3:groq");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            "nomodel".parse::<ModelRef>(),
            Err(ModelRefError::MissingSeparator)
        );
        assert!(matches!(
            "Bad Id/x".parse::<ModelRef>(),
            Err(ModelRefError::InvalidProvider(_))
        ));
        assert_eq!(
            "openai/".parse::<ModelRef>(),
            Err(ModelRefError::InvalidModel)
        );
        assert!("../etc/x".parse::<ModelRef>().is_err());
    }

    #[test]
    fn serde_uses_string_form() {
        let r: ModelRef = serde_json::from_str("\"openai/gpt-5\"").unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"openai/gpt-5\"");
        assert!(serde_json::from_str::<ModelRef>("\"x\"").is_err());
    }
}
