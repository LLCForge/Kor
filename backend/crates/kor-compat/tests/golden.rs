//! Golden tests: `CLAUDE.md`, Codex, and Cursor setups must load into the
//! same model every time. Regenerate with `UPDATE_GOLDEN=1 cargo test -p kor-compat`.

use std::path::Path;

use kor_compat::{LoadOptions, load};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn snapshot(trusted: bool) -> String {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    copy_dir(&fixtures.join("home"), &base.join("home"));
    copy_dir(&fixtures.join("project"), &base.join("repo"));
    std::fs::create_dir(base.join("repo/.git")).unwrap();
    std::fs::write(base.join("outside.md"), "OUTSIDE SECRET").unwrap();

    let c = load(&LoadOptions {
        root: base.join("repo"),
        home: Some(base.join("home")),
        codex_home: Some(base.join("home/.codex")),
        trusted,
    });
    let json = serde_json::to_string_pretty(&c).unwrap();
    let escaped = serde_json::to_string(&base.display().to_string()).unwrap();
    let escaped = escaped.trim_matches('"');
    json.replace(escaped, "<tmp>")
        .replace(&base.display().to_string(), "<tmp>")
        .replace("\\\\", "/")
        + "\n"
}

fn check(name: &str, trusted: bool) {
    let actual = snapshot(trusted);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(
        actual, expected,
        "golden mismatch for {name}; rerun with UPDATE_GOLDEN=1 and review the diff"
    );
}

#[test]
fn untrusted_project() {
    check("untrusted.json", false);
}

#[test]
fn trusted_project() {
    check("trusted.json", true);
}
