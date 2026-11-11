//! `kor deps` / `kor audit --licenses` (ADR 0046): one fixture project per lockfile format,
//! osv.dev and the npm registry mocked with wiremock, the offline cache, and the guard refusal.

use std::path::{Path, PathBuf};

use kor_i18n::{Lang, Localizer};
use kor_runtime::analysis::Severity;
use kor_runtime::analysis::deps::check::{Endpoints, Options, check_report};
use kor_runtime::analysis::deps::license::licenses_report;
use kor_runtime::analysis::deps::{Ecosystem, Inventory, Package, scan};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/deps")
        .join(name)
}

fn find<'a>(inv: &'a Inventory, name: &str) -> &'a Package {
    inv.packages
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{name} not in {:#?}", inv.packages))
}

fn at(p: &Package) -> (String, u32) {
    (p.file.to_string_lossy().replace('\\', "/"), p.line)
}

#[test]
fn cargo_workspace_lock() {
    let inv = scan(&fixture("cargo"));
    assert!(inv.errors.is_empty(), "{:?}", inv.errors);
    assert_eq!(
        inv.packages.len(),
        4,
        "the member itself is not a dependency"
    );
    let serde = find(&inv, "serde");
    assert!(serde.direct);
    assert_eq!(serde.ecosystem, Ecosystem::Cargo);
    assert_eq!(at(serde), ("Cargo.toml".into(), 6));
    let derive = find(&inv, "serde_derive");
    assert!(!derive.direct);
    assert_eq!(derive.via, ["serde"]);
    assert_eq!(at(derive).0, "Cargo.lock");
    assert_eq!(derive.version, "1.0.190");
}

#[test]
fn npm_lock_v3_with_licenses() {
    let inv = scan(&fixture("npm"));
    let gpl = find(&inv, "gpl-helper");
    assert!(!gpl.direct);
    assert_eq!(gpl.via, ["dayjs"]);
    assert_eq!(gpl.license.as_deref(), Some("GPL-3.0-only"));
    assert_eq!(gpl.license_from.as_deref(), Some("lockfile"));
    assert_eq!(at(gpl), ("package-lock.json".into(), 16));
    let jest = find(&inv, "jest");
    assert!(jest.direct && jest.dev);
    assert_eq!(at(jest), ("package.json".into(), 10));
    assert_eq!(at(find(&inv, "moment")), ("package.json".into(), 5));
}

#[test]
fn pnpm_v9_with_installed_license() {
    let inv = scan(&fixture("pnpm"));
    let n = find(&inv, "is-number");
    assert!(n.direct);
    assert_eq!(n.version, "7.0.0");
    assert_eq!(n.license.as_deref(), Some("MIT"));
    assert!(
        n.license_from
            .as_deref()
            .unwrap()
            .ends_with("is-number/package.json"),
        "{:?}",
        n.license_from
    );
    let k = find(&inv, "kind-of");
    assert_eq!(k.via, ["is-number"]);
    assert_eq!(k.license, None, "not installed: unknown, never guessed");
}

#[test]
fn yarn_v1_and_berry() {
    let v1 = scan(&fixture("yarn1"));
    let frame = find(&v1, "@babel/code-frame");
    assert!(frame.direct);
    assert_eq!(frame.version, "7.22.13");
    assert_eq!(find(&v1, "chalk").via, ["@babel/code-frame"]);
    let berry = scan(&fixture("berry"));
    assert_eq!(berry.packages.len(), 1, "workspace entry skipped");
    let lodash = find(&berry, "lodash");
    assert!(lodash.direct);
    assert_eq!(lodash.version, "4.17.21");
}

#[test]
fn python_lockfiles() {
    let poetry = scan(&fixture("poetry"));
    assert!(find(&poetry, "requests").direct);
    assert_eq!(find(&poetry, "certifi").via, ["requests"]);
    let pytest = find(&poetry, "pytest");
    assert!(pytest.direct && pytest.dev);

    let uv = scan(&fixture("uv"));
    assert_eq!(
        uv.packages.len(),
        2,
        "the editable project is not a dependency"
    );
    assert!(find(&uv, "httpx").direct);
    assert_eq!(find(&uv, "anyio").via, ["httpx"]);

    let pip = scan(&fixture("pipfile"));
    assert!(find(&pip, "flask").direct);
    assert!(!find(&pip, "werkzeug").direct);
    assert_eq!(find(&pip, "flask").version, "3.0.0");

    let req = scan(&fixture("requirements"));
    let r = find(&req, "requests");
    assert_eq!(
        r.license.as_deref(),
        Some("Apache 2.0"),
        "verbatim, classified unknown"
    );
    let mupdf = find(&req, "PyMuPDF");
    assert_eq!(mupdf.license.as_deref(), Some("AGPL-3.0-only"));
    assert_eq!(find(&req, "flask").version, "", "unpinned");
}

#[test]
fn go_composer_nuget_ruby_jvm() {
    let go = scan(&fixture("go"));
    let gin = find(&go, "github.com/gin-gonic/gin");
    assert!(gin.direct);
    assert_eq!((gin.version.as_str(), gin.line), ("1.9.1", 6));
    assert!(!find(&go, "golang.org/x/net").direct);
    assert!(find(&go, "github.com/BurntSushi/toml").direct);

    let composer = scan(&fixture("composer"));
    let mpdf = find(&composer, "mpdf/mpdf");
    assert!(mpdf.direct);
    assert_eq!(at(mpdf), ("composer.json".into(), 4));
    assert_eq!(mpdf.license.as_deref(), Some("GPL-2.0-only"));
    assert_eq!(find(&composer, "psr/log").via, ["mpdf/mpdf"]);

    let nuget = scan(&fixture("nuget"));
    let nj = find(&nuget, "Newtonsoft.Json");
    assert!(nj.direct);
    assert_eq!(at(nj), ("App.csproj".into(), 3));
    assert!(!find(&nuget, "System.Buffers").direct);

    let ruby = scan(&fixture("ruby"));
    let rack = find(&ruby, "rack");
    assert!(rack.direct);
    assert_eq!(at(rack), ("Gemfile".into(), 2));
    assert_eq!(find(&ruby, "nokogiri").version, "1.15.4");

    let maven = scan(&fixture("maven"));
    assert_eq!(
        maven.packages.len(),
        2,
        "dependencyManagement is not a dependency"
    );
    let itext = find(&maven, "com.itextpdf:itextpdf");
    assert_eq!(itext.version, "5.5.13.3");
    assert!(find(&maven, "junit:junit").dev);

    let gradle = scan(&fixture("gradle"));
    let guava = find(&gradle, "com.google.guava:guava");
    assert!(guava.direct);
    assert_eq!(at(guava), ("build.gradle".into(), 2));
    assert!(!find(&gradle, "com.google.guava:failureaccess").direct);
    assert!(find(&gradle, "junit:junit").dev);
}

#[test]
fn licenses_report_has_file_line_rule_and_source() {
    let l = Localizer::new(Lang::En);
    let r = licenses_report(&scan(&fixture("npm")), &["GPL-3.0*".into()], &l);
    let denied = &r.findings[0];
    assert_eq!(denied.rule, "license-denied");
    assert_eq!(denied.location(), "package-lock.json:16");
    assert!(denied.message.contains("dayjs"), "{}", denied.message);
    assert!(r.findings.iter().all(|f| f.source == "spdx" && f.line > 0));

    let req = licenses_report(&scan(&fixture("requirements")), &[], &l);
    let agpl = req
        .findings
        .iter()
        .find(|f| f.rule == "license-strong-copyleft")
        .unwrap();
    assert_eq!(agpl.location(), "requirements.txt:3");
    assert!(agpl.fix.as_deref().unwrap().contains("pypdf"));
    assert!(
        req.findings
            .iter()
            .any(|f| f.rule == "license-unknown" && f.line == 2)
    );
}

async fn mock_services() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {}, {}, {}, {},
                { "vulns": [{ "id": "GHSA-8hfj-j24r-96c4", "modified": "2023-01-01T00:00:00Z" }] }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/vulns/GHSA-8hfj-j24r-96c4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "GHSA-8hfj-j24r-96c4",
            "summary": "Path Traversal: 'dir/../../filename' in moment.locale",
            "aliases": ["CVE-2022-24785"],
            "database_specific": { "severity": "HIGH", "cwe_ids": ["CWE-22"] },
            "affected": [{
                "package": { "ecosystem": "npm", "name": "moment" },
                "ranges": [{ "type": "SEMVER", "events": [{ "introduced": "1.0.1" }, { "fixed": "2.29.2" }] }]
            }]
        })))
        .mount(&server)
        .await;
    for (name, body) in [
        ("moment", serde_json::json!({ "version": "2.29.4" })),
        ("dayjs", serde_json::json!({ "version": "1.11.10" })),
        (
            "left-pad",
            serde_json::json!({ "version": "1.3.0", "deprecated": "use String.prototype.padStart()" }),
        ),
        ("jest", serde_json::json!({ "version": "30.0.0" })),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/{name}/latest")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    server
}

#[tokio::test]
async fn check_with_osv_registry_and_offline_cache() {
    let server = mock_services().await;
    let base = server.uri();
    let cache = tempfile::tempdir().unwrap();
    let opts = Options {
        offline: false,
        endpoints: Endpoints {
            osv: base.clone(),
            npm: base.clone(),
            ..Endpoints::default()
        },
        cache_dir: Some(cache.path().to_path_buf()),
    };
    let l = Localizer::new(Lang::En);
    let root = fixture("npm");
    let inv = scan(&root);
    let r = check_report(&root, &inv, &opts, &l).await;
    let vuln = r
        .findings
        .iter()
        .find(|f| f.source == "osv")
        .expect("osv finding");
    assert_eq!(vuln.rule, "GHSA-8hfj-j24r-96c4");
    assert_eq!(vuln.severity, Severity::High);
    assert_eq!(vuln.location(), "package.json:5");
    assert_eq!(vuln.cwe.as_deref(), Some("CWE-22"));
    assert!(vuln.message.contains("CVE-2022-24785"));
    assert!(vuln.fix.as_deref().unwrap().contains("2.29.2"));

    let rules: Vec<&str> = r.findings.iter().map(|f| f.rule.as_str()).collect();
    assert!(
        rules.contains(&"duplicate-purpose"),
        "moment + dayjs: {rules:?}"
    );
    assert!(rules.contains(&"unused-dependency"), "left-pad: {rules:?}");
    let unused: Vec<_> = r
        .findings
        .iter()
        .filter(|f| f.rule == "unused-dependency")
        .collect();
    assert_eq!(
        unused.len(),
        1,
        "moment and dayjs are imported, jest is dev: {unused:?}"
    );
    assert_eq!(unused[0].location(), "package.json:7");
    let patch = r
        .findings
        .iter()
        .find(|f| f.rule == "upgrade-patch")
        .unwrap();
    assert!(
        patch.message.contains("2.29.1 → 2.29.4"),
        "{}",
        patch.message
    );
    assert_eq!(patch.source, "npm");
    let major = r
        .findings
        .iter()
        .find(|f| f.rule == "upgrade-major")
        .unwrap();
    assert!(major.message.contains("jest"));
    let deprecated = r.findings.iter().find(|f| f.rule == "deprecated").unwrap();
    assert!(
        deprecated.message.contains("padStart"),
        "registry note shown: {}",
        deprecated.message
    );
    assert!(
        r.findings
            .iter()
            .all(|f| f.line > 0 && !f.source.is_empty())
    );

    // Offline: the same advisories from the cache, with their age; no request.
    drop(server);
    let offline = Options {
        offline: true,
        ..opts
    };
    let r2 = check_report(&root, &inv, &offline, &l).await;
    assert!(r2.findings.iter().any(|f| f.rule == "GHSA-8hfj-j24r-96c4"));
    assert!(
        r2.notes.iter().any(|n| n.contains("from the cache")),
        "{:?}",
        r2.notes
    );

    // No cache yet: said so, local checks still run.
    let empty = tempfile::tempdir().unwrap();
    let r3 = check_report(
        &root,
        &inv,
        &Options {
            offline: true,
            endpoints: Endpoints::default(),
            cache_dir: Some(empty.path().to_path_buf()),
        },
        &l,
    )
    .await;
    assert!(r3.findings.iter().all(|f| f.source != "osv"));
    assert!(r3.notes.iter().any(|n| n.contains("no cached results")));
    assert!(r3.findings.iter().any(|f| f.rule == "duplicate-purpose"));
}

#[tokio::test]
async fn refused_in_offline_mode_without_connecting() {
    kor_core::net::set_policy(kor_core::net::Policy::Offline);
    let l = Localizer::new(Lang::En);
    let root = fixture("npm");
    let inv = scan(&root);
    let cache = tempfile::tempdir().unwrap();
    let r = check_report(
        &root,
        &inv,
        &Options {
            offline: false,
            endpoints: Endpoints::default(),
            cache_dir: Some(cache.path().to_path_buf()),
        },
        &l,
    )
    .await;
    kor_core::net::set_policy(kor_core::net::Policy::Open);
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("Offline mode") && n.contains("api.osv.dev")),
        "{:?}",
        r.notes
    );
    assert!(r.notes.iter().any(|n| n.contains("--offline")));
    assert!(r.findings.iter().all(|f| f.source != "osv"));
    assert!(
        std::fs::read_dir(cache.path()).unwrap().next().is_none(),
        "nothing cached from a refused run"
    );
    let osv: Vec<_> = kor_core::net::attempts()
        .into_iter()
        .filter(|a| a.host == "api.osv.dev")
        .collect();
    assert!(!osv.is_empty() && osv.iter().all(|a| !a.allowed));
}
