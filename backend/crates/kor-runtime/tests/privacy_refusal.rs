//! ADR 0035: privacy mode refuses a cloud provider with a clear message before any connection;
//! a request that gets built anyway stops in the network guard (no DNS lookup, no socket).

use std::time::{Duration, Instant};

use kor_core::ProviderErrorKind;
use kor_core::net;
use kor_i18n::{Lang, Localizer};
use kor_providers::Registry;
use kor_secrets::SecretString;

#[tokio::test]
async fn privacy_mode_refuses_a_cloud_provider_before_any_connection() {
    let l = Localizer::new(Lang::En);
    let mut config = kor_config::Config::default();
    config.privacy.mode = true;
    kor_runtime::apply_network_policy(&l, &config);
    assert!(kor_runtime::privacy_mode());
    assert_ne!(net::policy(), net::Policy::Open);

    let openai = Registry::builtin().get("openai").unwrap().clone();
    let err = kor_runtime::check_provider_allowed(&l, &openai).unwrap_err();
    assert!(
        err.contains("Privacy mode") && err.contains("openai"),
        "{err}"
    );
    let local = Registry::builtin().get("local").unwrap().clone();
    assert!(kor_runtime::check_provider_allowed(&l, &local).is_ok());

    net::clear_attempts();
    let p = kor_providers::build(
        &openai,
        Some(SecretString::from("sk-test-0123456789abcdef")),
    )
    .unwrap();
    let started = Instant::now();
    let err = p.list_models().await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(err.kind, ProviderErrorKind::InvalidRequest, "{err}");
    assert!(!err.is_retryable() && !err.allows_fallback());
    assert!(err.message.contains("api.openai.com"), "{}", err.message);
    let attempts = net::attempts();
    assert!(
        !attempts.is_empty() && attempts.iter().all(|a| !a.allowed),
        "{attempts:?}"
    );
}
