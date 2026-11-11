use std::collections::BTreeMap;

use kor_secrets::{ExposeSecret, MemoryStore, SecretStore};

use super::*;

fn hosts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(h, k)| ((*h).to_owned(), (*k).to_owned()))
        .collect()
}

#[test]
fn remotes_of_every_forge_and_url_form() {
    let none = BTreeMap::new();
    let r = Remote::parse("git@github.com:octo/kor.git", &none).unwrap();
    assert_eq!(
        (r.kind, r.owner.as_str(), r.repo.as_str()),
        (Kind::Github, "octo", "kor")
    );
    assert_eq!(r.api, "https://api.github.com");
    let r = Remote::parse("https://gitlab.com/group/sub/proj.git", &none).unwrap();
    assert_eq!((r.kind, r.owner.as_str()), (Kind::Gitlab, "group/sub"));
    assert_eq!(r.api, "https://gitlab.com/api/v4");
    let r = Remote::parse("ssh://git@codeberg.org:22/me/x", &none).unwrap();
    assert_eq!(
        (r.kind, r.api.as_str()),
        (Kind::Gitea, "https://codeberg.org/api/v1")
    );
    let r = Remote::parse("https://user@bitbucket.org/ws/repo.git", &none).unwrap();
    assert_eq!(
        (r.kind, r.api.as_str()),
        (Kind::Bitbucket, "https://api.bitbucket.org/2.0")
    );
    let cfg = hosts(&[("git.corp.example", "forgejo"), ("ghe.corp", "github")]);
    let r = Remote::parse("https://git.corp.example/a/b", &cfg).unwrap();
    assert_eq!(
        (r.kind, r.api.as_str()),
        (Kind::Gitea, "https://git.corp.example/api/v1")
    );
    let r = Remote::parse("git@ghe.corp:a/b.git", &cfg).unwrap();
    assert_eq!(r.api, "https://ghe.corp/api/v3");
    let r = Remote::parse(
        "http://127.0.0.1:8080/a/b.git",
        &hosts(&[("127.0.0.1", "gitlab")]),
    )
    .unwrap();
    assert_eq!(r.api, "http://127.0.0.1:8080/api/v4");
    assert!(Remote::parse("https://gitlab.example.org/a/b", &none).is_ok());
    assert!(matches!(
        Remote::parse("https://example.org/a/b", &none),
        Err(ForgeError::Unsupported(h)) if h == "example.org"
    ));
    assert!(matches!(
        Remote::parse("/srv/git/repo.git", &none),
        Err(ForgeError::NoRemote)
    ));
    assert!(matches!(
        Remote::parse(r"C:\repos\x.git", &none),
        Err(ForgeError::NoRemote)
    ));
}

#[test]
fn pr_numbers_from_text_and_urls() {
    assert_eq!(pr_number("12"), Some(12));
    assert_eq!(pr_number("#7"), Some(7));
    assert_eq!(pr_number("https://github.com/o/r/pull/42/files"), Some(42));
    assert_eq!(
        pr_number("https://gitlab.com/g/p/-/merge_requests/9?tab=diffs"),
        Some(9)
    );
    assert_eq!(
        pr_number("https://bitbucket.org/w/r/pull-requests/3"),
        Some(3)
    );
    assert_eq!(pr_number("https://github.com/o/r"), None);
}

#[test]
fn tokens_from_env_then_keychain() {
    let store = MemoryStore::default();
    assert!(matches!(
        token(Kind::Gitea, |_| None, &store),
        Err(ForgeError::NoToken(k)) if k == "gitea"
    ));
    store
        .set(
            &account_for("gitea").unwrap(),
            &SecretString::from("kc".to_owned()),
        )
        .unwrap();
    assert_eq!(
        token(Kind::Gitea, |_| None, &store)
            .unwrap()
            .expose_secret(),
        "kc"
    );
    let env = |v: &str| (v == "FORGEJO_TOKEN").then(|| "envtok".to_owned());
    assert_eq!(
        token(Kind::Gitea, env, &store).unwrap().expose_secret(),
        "envtok"
    );
    assert_eq!(Kind::for_auth_id("bitbucket"), Some(Kind::Bitbucket));
    assert_eq!(Kind::for_auth_id("openai"), None);
}

#[test]
fn descriptions_parse_and_fall_back() {
    let d = parse_description("```\n# Add forge support.\n\n## Summary\nWhy.\n```").unwrap();
    assert_eq!(d.title, "Add forge support");
    assert!(d.body.starts_with("## Summary"));
    assert!(parse_description("  \n").is_none());
    let e = Entry {
        hash: "abc".into(),
        kind: Some("feat".into()),
        scope: None,
        subject: "add pr create".into(),
        breaking: None,
    };
    let f = fallback_description("feat/pr-create", std::slice::from_ref(&e));
    assert_eq!(f.title, "add pr create");
    assert!(
        f.body.contains("### Added") && !f.body.contains("## [Changes]"),
        "{}",
        f.body
    );
    let two = fallback_description("feat/pr-create", &[e.clone(), e]);
    assert_eq!(two.title, "feat pr create");
}

#[test]
fn local_push_targets_skip_the_guard() {
    assert!(check_push("/srv/git/x.git").is_ok());
    assert!(check_push(r"C:\git\x.git").is_ok());
    assert!(check_push("file:///srv/git/x.git").is_ok());
}
