//! End-to-end checks against the built `kor` binary.

use std::process::Command;

/// Runs the built `kor` isolated from the developer's own setup: its own home, config and cache
/// under `cwd`, privacy mode (local providers only), so no test reads real keys or calls a paid API.
fn kor(cwd: &std::path::Path, args: &[&str]) -> (bool, String) {
    let iso = cwd.join(".kor-test-home");
    let out = Command::new(env!("CARGO_BIN_EXE_kor"))
        .arg("--privacy-mode")
        .args(args)
        .current_dir(cwd)
        .env_remove("KOR_LANG")
        .env("KOR_HOME", iso.join("home"))
        .env("KOR_CONFIG_DIR", iso.join("config"))
        .env("KOR_CACHE_DIR", iso.join("cache"))
        .env("CLAUDE_CONFIG_DIR", iso.join("claude"))
        .env("CODEX_HOME", iso.join("codex"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run kor");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn help_is_localized() {
    let dir = tempfile::tempdir().unwrap();
    for (lang, needle) in [
        ("en", "Diagnose the local environment"),
        ("tr", "Yerel ortamı teşhis et"),
        ("fr", "Diagnostiquer l’environnement local"),
    ] {
        let (ok, out) = kor(dir.path(), &["--lang", lang, "--help"]);
        assert!(ok, "{lang}: {out}");
        assert!(out.contains(needle), "{lang}: {out}");
    }
}

#[test]
fn project_config_is_explained_and_errors_name_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let kor_dir = dir.path().join(".kor");
    std::fs::create_dir(&kor_dir).unwrap();
    std::fs::write(
        kor_dir.join("config.toml"),
        "[privacy]\nmin_tier = \"local\"\n",
    )
    .unwrap();

    let (ok, out) = kor(dir.path(), &["--lang", "fr", "config", "explain"]);
    assert!(ok, "{out}");
    assert!(out.contains("privacy.min_tier = \"local\""), "{out}");
    assert!(out.contains("config.toml"), "{out}");

    std::fs::write(
        kor_dir.join("config.toml"),
        "[privacy]\nmin_tier = \"nope\"\n",
    )
    .unwrap();
    let (ok, out) = kor(dir.path(), &["--lang", "en", "config", "explain"]);
    assert!(!ok);
    assert!(out.contains("Invalid configuration in"), "{out}");
    assert!(out.contains("config.toml"), "{out}");
}

#[test]
fn invalid_provider_id_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out) = kor(dir.path(), &["--lang", "en", "auth", "logout", "../etc"]);
    assert!(!ok);
    assert!(out.contains("Invalid provider id"), "{out}");
}

#[test]
fn project_config_cannot_grant_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let kor_dir = dir.path().join(".kor");
    std::fs::create_dir(&kor_dir).unwrap();
    std::fs::write(
        kor_dir.join("config.toml"),
        "[permissions]\nmode = \"accept-edits\"\nallow = [\"Bash\"]\ndeny = [\"Read(.env)\"]\n",
    )
    .unwrap();
    let (ok, out) = kor(dir.path(), &["--lang", "en", "hello"]);
    assert!(!ok);
    assert!(
        out.contains("Ignored permissions.allow, permissions.mode"),
        "{out}"
    );
}
