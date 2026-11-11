//! Notification sounds for the CLI and TUI (§28.4): the same `assets/sounds/*.wav` files the
//! desktop plays, through the OS's own player in the background (Windows `SoundPlayer`, macOS
//! `afplay`, Linux `paplay` then `aplay`). Nothing is played when no player exists.

use std::path::PathBuf;

/// The bundled sounds: `soft`, `double`, `bell`, `chime`.
pub fn bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "soft" => Some(include_bytes!("../../../../assets/sounds/soft.wav")),
        "double" => Some(include_bytes!("../../../../assets/sounds/double.wav")),
        "bell" => Some(include_bytes!("../../../../assets/sounds/bell.wav")),
        "chime" => Some(include_bytes!("../../../../assets/sounds/chime.wav")),
        _ => None,
    }
}

/// The sound written once to Kor's cache folder (players take a file path).
fn file(name: &str) -> Option<PathBuf> {
    let data = bytes(name)?;
    let path = crate::cache_dir()?.join("sounds").join(format!("{name}.wav"));
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(data.len() as u64) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, data).ok()?;
    }
    Some(path)
}

/// Commands that play `path`, tried in order (the first that starts wins).
fn players(path: &std::path::Path) -> Vec<std::process::Command> {
    let p = path.to_string_lossy().into_owned();
    match crate::os() {
        crate::Os::Windows => {
            let mut c = std::process::Command::new("powershell");
            // The path is ours (cache folder); single quotes are doubled for PowerShell.
            let script = format!("(New-Object Media.SoundPlayer '{}').PlaySync()", p.replace('\'', "''"));
            c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            vec![c]
        }
        crate::Os::Mac => {
            let mut c = std::process::Command::new("afplay");
            c.arg(&p);
            vec![c]
        }
        _ => ["paplay", "aplay"]
            .into_iter()
            .map(|b| {
                let mut c = std::process::Command::new(b);
                c.arg(&p);
                c
            })
            .collect(),
    }
}

/// Play `name` in the background; returns at once.
pub fn play(name: &str) {
    let Some(path) = file(name) else { return };
    std::thread::spawn(move || {
        use crate::NoWindow as _;
        for mut c in players(&path) {
            let started = c
                .no_window()
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if started.is_ok() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_named_sound_is_a_wav() {
        for n in ["soft", "double", "bell", "chime"] {
            let b = super::bytes(n).unwrap();
            assert_eq!(&b[..4], b"RIFF", "{n}");
            assert_eq!(&b[8..12], b"WAVE", "{n}");
        }
        assert!(super::bytes("nope").is_none());
        assert!(!super::players(std::path::Path::new("x.wav")).is_empty());
    }
}
