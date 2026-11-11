//! The `kor` CLI the desktop starts (intent routing, MCP servers, triggers, peers, OS integration):
//! the one shipped next to the desktop executable, else, in a debug build, the workspace's own
//! `backend/target/{debug,release}/kor`, else the one on `PATH`.

use std::path::PathBuf;

/// `kor` (`kor.exe`) next to the desktop executable: what installers ship.
pub fn bundled() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|e| e.with_file_name(kor_platform::exe("kor")))
        .filter(|p| p.is_file())
}

/// Debug builds (`tauri dev`) have no bundled CLI: use the one this workspace built.
fn workspace_build() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../backend/target");
    ["debug", "release"]
        .into_iter()
        .map(|p| target.join(p).join(kor_platform::exe("kor")))
        .find(|p| p.is_file())
}

fn on_path() -> Option<PathBuf> {
    let name = kor_platform::exe("kor");
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&name)).find(|p| p.is_file())
}

/// The CLI to run, if there is one.
pub fn find() -> Option<PathBuf> {
    bundled().or_else(workspace_build).or_else(on_path)
}

/// [`find`], else the bare name (the OS reports "not found" with the usual message).
pub fn path() -> PathBuf {
    find().unwrap_or_else(|| PathBuf::from(kor_platform::exe("kor")))
}

/// Error text when the CLI is needed and missing.
pub const MISSING: &str = "Kor CLI (kor) bulunamadı: masaüstüyle gelen kopya yok ve PATH'te değil. Ayarlar > Kor > \"CLI'yı PATH'e ekle\" ya da `cargo build -p kor-cli`.";

#[cfg(test)]
mod tests {
    #[test]
    fn debug_builds_find_the_workspace_cli_when_it_was_built() {
        let built = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../backend/target/debug")
            .join(kor_platform::exe("kor"));
        if built.is_file() {
            assert!(super::find().is_some());
        }
        assert!(!super::path().as_os_str().is_empty());
    }

    use std::path::PathBuf;
}
