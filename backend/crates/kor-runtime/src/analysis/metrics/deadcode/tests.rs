use super::*;
use crate::analysis::metrics::fixture;

fn dead_names(r: &DeadReport) -> Vec<&str> {
    r.dead.iter().map(|d| d.name.as_str()).collect()
}

fn ambiguous_names(r: &DeadReport) -> Vec<&str> {
    r.ambiguous.iter().map(|d| d.name.as_str()).collect()
}

fn analyse(dir: &Path) -> DeadReport {
    let g = crate::analysis::metrics::graph(dir).expect("graph builds");
    find(dir, &g)
}

#[test]
fn rust_library_crate() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fixture::write(
        d,
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    );
    fixture::write(
        d,
        "src/lib.rs",
        "mod inner;\nuse std::fmt;\n\npub fn api() -> u32 { inner::helper() }\n\n/// Never called.\nfn forgotten() -> u32 { 7 }\n\npub struct Shown;\n\nimpl fmt::Display for Shown {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, \"x\") }\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn checks() { assert_eq!(super::api(), 1); }\n}\n",
    );
    fixture::write(
        d,
        "src/inner.rs",
        "pub(crate) fn helper() -> u32 { 1 }\npub(crate) fn orphan_helper() -> u32 { 2 }\n",
    );
    let r = analyse(d);
    let dead = dead_names(&r);
    assert!(dead.contains(&"forgotten"), "{dead:?}");
    assert!(dead.contains(&"orphan_helper"), "{dead:?}");
    for alive in ["api", "helper", "fmt", "Shown", "checks"] {
        assert!(!dead.contains(&alive), "{alive} reported dead: {dead:?}");
    }
    assert!(r.entry_rules.contains_key("rust-public-api"));
    // Outside git there is no history: medium confidence at most.
    assert!(r.dead.iter().all(|x| x.confidence == Confidence::Medium));
}

#[test]
fn nextjs_django_and_go_entry_rules() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fixture::write(
        d,
        "web/package.json",
        "{\"dependencies\": {\"next\": \"15.0.0\"}}",
    );
    fixture::write(
        d,
        "web/app/page.tsx",
        "import { usedFn } from '../lib/a'\nexport default function Page() { return usedFn() }\n",
    );
    fixture::write(
        d,
        "web/lib/a.ts",
        "export function usedFn() { return 1 }\nexport function unusedFn() { return 2 }\n",
    );
    fixture::write(d, "api/manage.py", "import sys\n");
    fixture::write(
        d,
        "api/shop/views.py",
        "from shop.util import price\n\ndef index(request):\n    return price()\n",
    );
    fixture::write(
        d,
        "api/shop/util.py",
        "def price():\n    return 1\n\ndef stale_price():\n    return 2\n",
    );
    fixture::write(d, "svc/go.mod", "module example.com/svc\n");
    fixture::write(
        d,
        "svc/main.go",
        "package main\n\nfunc main() { helper() }\n\nfunc helper() {}\n\nfunc unusedThing() {}\n",
    );
    let r = analyse(d);
    let dead = dead_names(&r);
    for x in ["unusedFn", "stale_price", "unusedThing"] {
        assert!(dead.contains(&x), "{x} not dead: {dead:?}");
    }
    for x in ["Page", "usedFn", "index", "price", "main", "helper"] {
        assert!(!dead.contains(&x), "{x} reported dead: {dead:?}");
    }
    assert!(r.entry_rules.contains_key("nextjs-routes"));
    assert!(r.entry_rules.contains_key("django-views"));
}

#[test]
fn spring_and_laravel_entry_rules() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fixture::write(
        d,
        "src/main/java/app/HomeController.java",
        "package app;\n\n@RestController\npublic class HomeController {\n    @GetMapping(\"/\")\n    public String home() { return Fmt.label(); }\n}\n",
    );
    fixture::write(
        d,
        "src/main/java/app/Fmt.java",
        "package app;\n\npublic class Fmt {\n    public static String label() { return \"x\"; }\n    public static String orphanLabel() { return \"y\"; }\n}\n",
    );
    fixture::write(
        d,
        "shop/composer.json",
        "{\"require\": {\"laravel/framework\": \"^11\"}}",
    );
    fixture::write(
        d,
        "shop/app/Http/Controllers/HomeController.php",
        "<?php\nclass ShopController { public function show() { return shop_total(); } }\n",
    );
    fixture::write(
        d,
        "shop/app/Support/helpers.php",
        "<?php\nfunction shop_total() { return 1; }\nfunction unused_helper() { return 2; }\n",
    );
    let r = analyse(d);
    let dead = dead_names(&r);
    assert!(dead.contains(&"orphanLabel"), "{dead:?}");
    assert!(dead.contains(&"unused_helper"), "{dead:?}");
    for x in ["home", "HomeController", "label", "show", "shop_total"] {
        assert!(!dead.contains(&x), "{x} reported dead: {dead:?}");
    }
}

#[test]
fn ambiguous_user_rules_and_git_confidence() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fixture::init(d);
    fixture::write(
        d,
        "app/main.py",
        "import handlers\n\nHANDLER = \"dyn_handler\"\n\nif __name__ == \"__main__\":\n    handlers.run()\n",
    );
    fixture::write(
        d,
        "app/handlers.py",
        "def run():\n    return 0\n\ndef dyn_handler():\n    return 1\n\ndef old_unused():\n    return 2\n\ndef plugin_hook():\n    return 3\n",
    );
    fixture::write(
        d,
        "app/plugins.py",
        "def loader(n):\n    return getattr(object, n)\n\ndef by_name():\n    return 4\n",
    );
    fixture::write(
        d,
        ".kor/dead-code.toml",
        "[[rule]]\nname = \"plugins\"\nframework = \"own\"\nkind = \"callback\"\nnames = '^plugin_'\n",
    );
    fixture::commit(d, "feat: app", "Ayşe Dev", "ayse@example.com", 400);
    let r = analyse(d);
    assert!(
        ambiguous_names(&r).contains(&"dyn_handler"),
        "{:?}",
        ambiguous_names(&r)
    );
    assert!(
        ambiguous_names(&r).contains(&"by_name"),
        "reflection: {:?}",
        ambiguous_names(&r)
    );
    let old = r
        .dead
        .iter()
        .find(|x| x.name == "old_unused")
        .expect("old_unused dead");
    assert_eq!(old.confidence, Confidence::High);
    assert!(old.unchanged_days.is_some_and(|x| x >= 399));
    assert!(!dead_names(&r).contains(&"plugin_hook"));
    assert!(!dead_names(&r).contains(&"run"));
    let rep = report(&r, &Localizer::new(kor_i18n::Lang::En));
    assert!(
        rep.findings
            .iter()
            .any(|f| f.rule == "kor.dead-code.ambiguous" && f.line == 4)
    );
    assert!(
        rep.findings
            .iter()
            .any(|f| f.rule == "kor.dead-code.unreachable" && f.line == 7)
    );
}

#[test]
fn removal_takes_docs_and_attributes() {
    let text = "fn keep() {}\n\n/// Doc.\n#[inline]\nfn gone() {\n    1;\n}\n\nfn also_keep() {}\n";
    let item = DeadItem {
        id: "a.rs#gone".into(),
        kind: "function".into(),
        name: "gone".into(),
        file: "a.rs".into(),
        line: 5,
        end: 7,
        confidence: Confidence::High,
        reason: Reason::Unreachable,
        unchanged_days: Some(400),
    };
    assert_eq!(
        remove_items(text, &[&item]),
        "fn keep() {}\n\nfn also_keep() {}\n"
    );
}

async fn mount_pr(server: &wiremock::MockServer) {
    use wiremock::matchers::{body_string_contains, method, path};
    wiremock::Mock::given(method("POST"))
        .and(path("/api/v3/repos/o/r/pulls"))
        .and(body_string_contains("old_unused"))
        .and(body_string_contains("unreachable from every entry point"))
        .respond_with(
            wiremock::ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 9, "title": "t", "html_url": "https://gh/o/r/pull/9",
                "head": { "ref": "x" }, "base": { "ref": "main" }, "user": { "login": "me" },
                "state": "open", "draft": false
            })),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn prune_verifies_commits_and_opens_a_pr() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fixture::init(d);
    fixture::write(
        d,
        "app/main.py",
        "import util\n\nif __name__ == \"__main__\":\n    util.used()\n",
    );
    fixture::write(
        d,
        "app/util.py",
        "def used():\n    return 1\n\n\ndef old_unused():\n    return 2\n",
    );
    fixture::commit(d, "feat: app", "Ayşe Dev", "ayse@example.com", 400);
    let bare = tempfile::tempdir().unwrap();
    fixture::git(bare.path(), &["init", "-q", "--bare"]);
    fixture::git(
        d,
        &["remote", "add", "origin", &bare.path().to_string_lossy()],
    );

    let r = analyse(d);
    assert_eq!(dead_names(&r), ["old_unused"]);
    let mut config = kor_config::Config::default();
    config.sandbox.mode = kor_config::SandboxMode::Off;
    let l = Localizer::new(kor_i18n::Lang::En);
    let plan = prune(
        d,
        &r,
        &PruneOptions {
            include_medium: false,
            check: Some("git --version".into()),
            test: Some("git status --short".into()),
        },
        &config,
        &l,
    )
    .await
    .unwrap();
    assert!(plan.ready, "{plan:?}");
    assert_eq!(plan.removed.len(), 1);
    assert!(plan.check.as_ref().is_some_and(|c| c.ok));
    assert!(plan.tests.as_ref().is_some_and(|c| c.ok));
    // The user's checkout is untouched; the branch holds the deletion.
    assert!(
        std::fs::read_to_string(d.join("app/util.py"))
            .unwrap()
            .contains("old_unused")
    );
    let on_branch = fixture::git(d, &["show", &format!("{}:app/util.py", plan.branch)]);
    assert_eq!(on_branch, "def used():\n    return 1\n");
    assert!(
        !d.join(".kor/worktrees")
            .read_dir()
            .is_ok_and(|mut x| x.next().is_some())
    );

    let server = wiremock::MockServer::start().await;
    mount_pr(&server).await;
    let hosts: std::collections::BTreeMap<String, String> =
        [("127.0.0.1".to_owned(), "github".to_owned())].into();
    let remote = forge::Remote::parse(&format!("{}/o/r.git", server.uri()), &hosts).unwrap();
    let client =
        forge::Forge::new(remote, kor_secrets::SecretString::from("tok".to_owned())).unwrap();
    let pr = publish_with(d, &plan, |req| async move { client.create(&req).await })
        .await
        .unwrap();
    assert_eq!(pr.number, 9);
    let pushed = fixture::git(bare.path(), &["branch", "--list", "kor/dead-code/*"]);
    assert!(pushed.contains(&plan.branch), "{pushed}");
}

#[tokio::test]
async fn prune_without_a_checker_commits_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fixture::init(d);
    fixture::write(d, "a.py", "def old_unused():\n    return 2\n");
    fixture::commit(d, "feat: a", "Ayşe Dev", "ayse@example.com", 400);
    let r = analyse(d);
    let mut config = kor_config::Config::default();
    config.sandbox.mode = kor_config::SandboxMode::Off;
    let l = Localizer::new(kor_i18n::Lang::En);
    let failing = prune(
        d,
        &r,
        &PruneOptions {
            include_medium: true,
            check: Some("git definitely-not-a-command".into()),
            test: None,
        },
        &config,
        &l,
    )
    .await
    .unwrap();
    assert!(!failing.ready);
    assert!(
        fixture::git(d, &["branch", "--list", "kor/dead-code/*"])
            .trim()
            .is_empty()
    );
}
