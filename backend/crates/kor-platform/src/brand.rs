//! The brand single source (spec §22.2, ADR 0064): the repo-root `branding.toml`, embedded at
//! build time so the binaries, the installers and `just rebrand` read the same values.

use std::sync::OnceLock;

use serde::Deserialize;

/// `branding.toml` as shipped in this build.
pub const SOURCE: &str = include_str!("../../../../branding.toml");

/// Values from `branding.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Brand {
    pub product: Product,
    pub urls: Urls,
    pub chrome: Chrome,
}

/// `[product]`
#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    pub name: String,
    pub slug: String,
    pub identifier: String,
    pub publisher: String,
    pub description: String,
}

/// `[urls]`
#[derive(Debug, Clone, Deserialize)]
pub struct Urls {
    pub repository: String,
    pub components: String,
    pub install: String,
    pub security: String,
}

/// `[chrome]`
#[derive(Debug, Clone, Deserialize)]
pub struct Chrome {
    pub native_host: String,
    pub extension_ids: Vec<String>,
}

/// The embedded brand.
///
/// # Panics
/// Never in a released build: a malformed `branding.toml` fails the `parses` test first.
pub fn brand() -> &'static Brand {
    static BRAND: OnceLock<Brand> = OnceLock::new();
    BRAND.get_or_init(|| {
        toml::from_str(SOURCE).expect("branding.toml is checked by the brand::tests::parses test")
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let b = super::brand();
        assert!(!b.product.name.is_empty() && !b.product.slug.is_empty());
        assert!(
            b.product
                .slug
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert!(
            b.product.identifier.split('.').count() >= 3,
            "reverse-DNS bundle id"
        );
        assert!(b.urls.repository.starts_with("https://"));
        assert!(
            b.chrome
                .extension_ids
                .iter()
                .all(|id| id.len() == 32 && id.chars().all(|c| ('a'..='p').contains(&c)))
        );
    }
}
