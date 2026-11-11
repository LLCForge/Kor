//! Download, verify and install against a local mock server.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kor_components::{Manifest, Progress, Store};
use sha2::{Digest as _, Sha256};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const T: &str = "test-target";

fn hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .fold(String::new(), |s, b| s + &format!("{b:02x}"))
}

fn manifest(url: &str, data: &[u8], archive: &str, exe: &str) -> Manifest {
    Manifest::parse(&format!(
        r#"
        [[component]]
        name = "tool"
        version = "1.0+1"
        about = "test tool"
        [[component.target]]
        target = "{T}"
        url = "{url}"
        sha256 = "{}"
        size = {}
        archive = "{archive}"
        exe = "{exe}"
        "#,
        hex(data),
        data.len()
    ))
    .unwrap()
}

fn tar_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        // Raw header: `Header::set_path` would refuse the `../` names these tests need.
        let mut h = tar::Header::new_gnu();
        let raw = name.as_bytes();
        h.as_gnu_mut().unwrap().name[..raw.len()].copy_from_slice(raw);
        h.set_size(data.len() as u64);
        h.set_mode(0o755);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        b.append(&h, *data).unwrap();
    }
    b.into_inner().unwrap()
}

fn xz(data: &[u8]) -> Vec<u8> {
    let mut w = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default()).unwrap();
    w.write_all(data).unwrap();
    w.finish().unwrap()
}

fn zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        z.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

async fn serve(body: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tool"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    server
}

fn quiet(_: Progress) {}

fn installed_text(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[tokio::test]
async fn tar_xz_installs_once_and_reports_progress() {
    let archive = xz(&tar_entries(&[("pkg/bin/tool", b"hello")]));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tool"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(
            &format!("{}/tool", server.uri()),
            &archive,
            "tar.xz",
            "pkg/bin/tool",
        ),
    );
    assert!(store.path_for("tool", T).is_none());
    let seen = Mutex::new(Vec::new());
    let report = |p: Progress| seen.lock().unwrap().push(p);
    let exe = store.ensure_for("tool", T, &report).await.unwrap();
    assert_eq!(installed_text(&exe), "hello");
    assert_eq!(
        exe,
        dir.path().join("tool").join("1.0+1").join("pkg/bin/tool")
    );
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.done, archive.len() as u64);
    assert_eq!(last.total, Some(archive.len() as u64));
    // Second call: already installed, no request (the mock expects exactly one).
    assert_eq!(store.ensure_for("tool", T, &quiet).await.unwrap(), exe);
    assert_eq!(store.path_for("tool", T), Some(exe));
    // Nothing else left next to the version folder.
    let names: Vec<_> = std::fs::read_dir(dir.path().join("tool"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["1.0+1"]);
    assert!(store.remove("tool").unwrap());
    assert!(store.path_for("tool", T).is_none());
    assert!(!store.remove("tool").unwrap());
}

#[tokio::test]
async fn zip_installs() {
    let archive = zip_entries(&[("node/node.exe", b"zip-ok")]);
    let server = serve(archive.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(
            &format!("{}/tool", server.uri()),
            &archive,
            "zip",
            "node/node.exe",
        ),
    );
    let exe = store.ensure_for("tool", T, &quiet).await.unwrap();
    assert_eq!(installed_text(&exe), "zip-ok");
}

#[tokio::test]
async fn wrong_checksum_installs_nothing() {
    let archive = zip_entries(&[("a/tool", b"real")]);
    let tampered = zip_entries(&[("a/tool", b"evil")]);
    assert_eq!(archive.len(), tampered.len());
    let server = serve(tampered).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(&format!("{}/tool", server.uri()), &archive, "zip", "a/tool"),
    );
    let err = store.ensure_for("tool", T, &quiet).await.unwrap_err();
    assert!(err.contains("checksum"), "{err}");
    assert!(store.path_for("tool", T).is_none());
    assert!(!dir.path().join("tool").join("1.0+1.part").exists());
    assert!(!dir.path().join("tool").join("1.0+1").exists());
}

#[tokio::test]
async fn oversized_download_is_cut_off() {
    let archive = zip_entries(&[("a/tool", b"x")]);
    let mut bigger = archive.clone();
    bigger.extend_from_slice(&[0u8; 4096]);
    let server = serve(bigger).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(&format!("{}/tool", server.uri()), &archive, "zip", "a/tool"),
    );
    let err = store.ensure_for("tool", T, &quiet).await.unwrap_err();
    assert!(err.contains("larger"), "{err}");
}

#[tokio::test]
async fn partial_download_resumes_with_range() {
    let content = vec![7u8; 20_000];
    let archive = xz(&tar_entries(&[("pkg/tool", &content)]));
    let half = archive.len() / 2;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tool"))
        .and(header("range", format!("bytes={half}-").as_str()))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header(
                    "content-range",
                    format!("bytes {half}-{}/{}", archive.len() - 1, archive.len()).as_str(),
                )
                .set_body_bytes(archive[half..].to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(
            &format!("{}/tool", server.uri()),
            &archive,
            "tar.xz",
            "pkg/tool",
        ),
    );
    std::fs::create_dir_all(dir.path().join("tool")).unwrap();
    std::fs::write(dir.path().join("tool").join("1.0+1.part"), &archive[..half]).unwrap();
    let exe = store.ensure_for("tool", T, &quiet).await.unwrap();
    assert_eq!(std::fs::read(exe).unwrap(), content);
}

async fn rejected(archive: Vec<u8>, kind: &str) -> (String, PathBuf, tempfile::TempDir) {
    let server = serve(archive.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("components");
    let store = Store::new(
        &root,
        manifest(
            &format!("{}/tool", server.uri()),
            &archive,
            kind,
            "pkg/tool",
        ),
    );
    let err = store.ensure_for("tool", T, &quiet).await.unwrap_err();
    assert!(store.path_for("tool", T).is_none());
    (err, root, dir)
}

#[tokio::test]
async fn path_traversal_in_tar_is_rejected() {
    let archive = xz(&tar_entries(&[
        ("pkg/tool", b"ok"),
        ("../../evil.txt", b"pwned"),
    ]));
    let (err, root, dir) = rejected(archive, "tar.xz").await;
    assert!(err.contains("escapes"), "{err}");
    assert!(!dir.path().join("evil.txt").exists());
    assert!(!root.join("evil.txt").exists());
    // The temporary unpack folder is gone too.
    let left: Vec<_> = std::fs::read_dir(root.join("tool"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn escaping_symlink_in_tar_is_rejected() {
    let mut b = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_size(0);
    h.set_path("pkg/tool").unwrap();
    h.set_link_name("../../../outside").unwrap();
    h.set_cksum();
    b.append(&h, std::io::empty()).unwrap();
    let archive = xz(&b.into_inner().unwrap());
    let (err, _, _dir) = rejected(archive, "tar.xz").await;
    assert!(err.contains("escapes"), "{err}");
}

#[tokio::test]
async fn path_traversal_in_zip_is_rejected() {
    let archive = zip_entries(&[("pkg/tool", b"ok"), ("../evil.txt", b"pwned")]);
    let (err, root, dir) = rejected(archive, "zip").await;
    assert!(err.contains("escapes"), "{err}");
    assert!(!dir.path().join("evil.txt").exists());
    assert!(!root.join("evil.txt").exists());
}

#[tokio::test]
async fn privacy_mode_refuses_download_but_offline_file_works() {
    let archive = zip_entries(&[("pkg/tool", b"offline")]);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path().join("c"),
        manifest(
            &format!("{}/tool", server.uri()),
            &archive,
            "zip",
            "pkg/tool",
        ),
    )
    .offline(true);
    let err = store.ensure_for("tool", T, &quiet).await.unwrap_err();
    assert!(err.contains("--from"), "{err}");
    // Air-gapped install: same checksum, no network.
    let file = dir.path().join("tool.zip");
    std::fs::write(&file, &archive).unwrap();
    let exe = store.add_from("tool", T, &file).unwrap();
    assert_eq!(installed_text(&exe), "offline");
    assert!(file.exists(), "the user's archive is left alone");
    std::fs::write(&file, b"not the archive").unwrap();
    assert!(
        store
            .add_from("tool", T, &file)
            .unwrap_err()
            .contains("checksum")
    );
}

#[tokio::test]
async fn single_file_component_and_missing_target() {
    let model = b"traineddata-bytes".to_vec();
    let server = serve(model.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(
            &format!("{}/tool", server.uri()),
            &model,
            "file",
            "tur.traineddata",
        ),
    );
    let p = store.ensure_for("tool", T, &quiet).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), model);
    let err = store
        .ensure_for("tool", "other-target", &quiet)
        .await
        .unwrap_err();
    assert!(err.contains("no published build"), "{err}");
    assert!(
        store
            .ensure_for("nope", T, &quiet)
            .await
            .unwrap_err()
            .contains("unknown")
    );
}

#[tokio::test]
async fn artifact_file_for_a_bundle_is_verified_and_cached() {
    let body = zip_entries(&[("bin/tool", b"#!/bin/sh\n")]);
    let server = serve(body.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path().join("c"),
        manifest(&format!("{}/tool", server.uri()), &body, "zip", "bin/tool"),
    );
    let cache = dir.path().join("cache");
    let p = store
        .artifact_file("tool", T, &cache, &quiet)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), body);
    assert!(store.path_for("tool", T).is_none(), "not installed");
    // A cached, verified file is reused without a request.
    drop(server);
    let again = store
        .artifact_file("tool", T, &cache, &quiet)
        .await
        .unwrap();
    assert_eq!(again, p);
    // A damaged cache file is not used (and with privacy on, not re-downloaded either).
    std::fs::write(&p, b"x").unwrap();
    let offline = store.clone().offline(true);
    let err = offline
        .artifact_file("tool", T, &cache, &quiet)
        .await
        .unwrap_err();
    assert!(err.contains("privacy"), "{err}");
}

/// Real downloads (~665 MB): `cargo test -p kor-components --test install -- --ignored`.
#[tokio::test]
#[ignore = "downloads llama-server and a 639 MB model from GitHub / Hugging Face"]
async fn real_offline_runtime_and_model_install_verified() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path(), Manifest::builtin());
    let t = kor_components::target();
    for name in ["llama-server", "model-qwen3-0-6b"] {
        let p = store.ensure_for(name, &t, &quiet).await.unwrap();
        assert!(p.is_file(), "{name}: {}", p.display());
    }
}

#[tokio::test]
async fn broken_install_is_found_deleted_and_downloaded_again() {
    use kor_components::Health;
    let archive = xz(&tar_entries(&[
        ("pkg/bin/tool", b"hello"),
        ("pkg/lib/data", b"0123456789"),
    ]));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tool"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive.clone()))
        .expect(3)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest(
            &format!("{}/tool", server.uri()),
            &archive,
            "tar.xz",
            "pkg/bin/tool",
        ),
    );
    assert_eq!(store.health("tool", T, true), Health::Missing);
    let exe = store.ensure_for("tool", T, &quiet).await.unwrap();
    assert_eq!(store.health("tool", T, true), Health::Ok);
    let data = dir
        .path()
        .join("tool")
        .join("1.0+1")
        .join("pkg")
        .join("lib")
        .join("data");

    // Cut short: the cheap check on use sees it and downloads again (request 2).
    std::fs::write(&data, b"0123").unwrap();
    assert_eq!(
        store.health("tool", T, false),
        Health::Broken(vec!["pkg/lib/data".into()])
    );
    assert_eq!(store.ensure_for("tool", T, &quiet).await.unwrap(), exe);
    assert_eq!(installed_text(&data), "0123456789");

    // Same size, other bytes: only the deep check sees it; verify deletes the copy.
    std::fs::write(&data, b"abcdefghij").unwrap();
    assert_eq!(store.health("tool", T, false), Health::Ok);
    assert_eq!(
        store.verify("tool", T).unwrap(),
        Health::Broken(vec!["pkg/lib/data".into()])
    );
    assert!(store.path_for("tool", T).is_none());
    store.ensure_for("tool", T, &quiet).await.unwrap(); // request 3
    assert_eq!(store.verify("tool", T).unwrap(), Health::Ok);
}

#[test]
fn an_older_installed_version_is_outdated() {
    use kor_components::Health;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        dir.path(),
        manifest("http://127.0.0.1:9/tool", b"x", "tar.xz", "pkg/bin/tool"),
    );
    std::fs::create_dir_all(dir.path().join("tool").join("0.9").join("pkg")).unwrap();
    assert_eq!(
        store.health("tool", T, false),
        Health::Outdated {
            installed: "0.9".into()
        }
    );
    assert_eq!(store.outdated(T), ["tool"]);
    // An install from before the receipt existed is still usable, just not checkable.
    let v = dir
        .path()
        .join("tool")
        .join("1.0+1")
        .join("pkg")
        .join("bin");
    std::fs::create_dir_all(&v).unwrap();
    std::fs::write(v.join("tool"), b"old").unwrap();
    assert_eq!(store.health("tool", T, true), Health::Unrecorded);
    assert!(store.outdated(T).is_empty());
}
