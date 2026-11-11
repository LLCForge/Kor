//! Screenshot scaling, point mapping and key combinations (moved from `kor-tools`'s computer tool).

/// Screenshots wider than this are scaled down before they reach the model.
pub const MAX_SHOT_WIDTH: u32 = 1280;

/// Size the model sees for a screen of `w`×`h`.
pub fn shot_size(w: u32, h: u32) -> (u32, u32) {
    if w <= MAX_SHOT_WIDTH || w == 0 {
        return (w, h);
    }
    let sh = u32::try_from(u64::from(h) * u64::from(MAX_SHOT_WIDTH) / u64::from(w)).unwrap_or(h);
    (MAX_SHOT_WIDTH, sh.max(1))
}

/// Map a point from screenshot pixels to the input space (`input_w`×`input_h`),
/// clamped to the screen.
pub fn to_input(x: i64, y: i64, shot: (u32, u32), input: (i32, i32)) -> (i32, i32) {
    let map = |v: i64, from: u32, to: i32| -> i32 {
        if from == 0 || to <= 0 {
            return 0;
        }
        let v = v.clamp(0, i64::from(from) - 1);
        let mapped = v * i64::from(to) / i64::from(from);
        i32::try_from(mapped.clamp(0, i64::from(to) - 1)).unwrap_or(0)
    };
    (map(x, shot.0, input.0), map(y, shot.1, input.1))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyName {
    Named(&'static str),
    Char(char),
}

/// `ctrl+shift+t`, `cmd+c`, `Return`, `alt+F4`, `a` → modifiers + key, normalised.
pub fn parse_combo(combo: &str) -> Result<Vec<KeyName>, String> {
    let mut keys = Vec::new();
    for raw in combo.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        let lower = raw.to_ascii_lowercase();
        let named = match lower.as_str() {
            "ctrl" | "control" => "Control",
            "shift" => "Shift",
            "alt" | "option" | "opt" => "Alt",
            "cmd" | "command" | "meta" | "super" | "win" | "windows" => "Meta",
            "enter" | "return" => "Return",
            "tab" => "Tab",
            "esc" | "escape" => "Escape",
            "backspace" => "Backspace",
            "delete" | "del" => "Delete",
            "space" => "Space",
            "up" | "arrowup" => "UpArrow",
            "down" | "arrowdown" => "DownArrow",
            "left" | "arrowleft" => "LeftArrow",
            "right" | "arrowright" => "RightArrow",
            "home" => "Home",
            "end" => "End",
            "pageup" | "page_up" => "PageUp",
            "pagedown" | "page_down" => "PageDown",
            "capslock" => "CapsLock",
            f if f.len() <= 3
                && f.starts_with('f')
                && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) =>
            {
                [
                    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
                ][usize::from(f[1..].parse::<u8>().unwrap_or(1)) - 1]
            }
            _ => {
                let mut chars = raw.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => {
                        keys.push(KeyName::Char(c.to_ascii_lowercase()));
                        continue;
                    }
                    _ => return Err(format!("unknown key `{raw}`")),
                }
            }
        };
        keys.push(KeyName::Named(named));
    }
    if keys.is_empty() {
        return Err("empty key combination".into());
    }
    Ok(keys)
}

/// Combinations that close apps, end the session or delete (§19.1): always approved by the user,
/// whatever the app's tier. Unparseable text counts as destructive.
pub fn is_destructive_combo(combo: &str) -> bool {
    let Ok(keys) = parse_combo(combo) else {
        return true;
    };
    let has = |n: &str| {
        keys.iter()
            .any(|k| matches!(k, KeyName::Named(x) if *x == n))
    };
    let ch = |c: char| keys.contains(&KeyName::Char(c));
    (has("Alt") && has("F4"))
        || (has("Meta") && (ch('q') || ch('w') || has("Backspace") || ch('l')))
        || (has("Control") && has("Alt") && has("Delete"))
        || has("Delete")
        || (has("Control") && ch('w'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_scaling_and_point_mapping() {
        assert_eq!(shot_size(1920, 1080), (1280, 720));
        assert_eq!(shot_size(1280, 800), (1280, 800));
        assert_eq!(shot_size(3840, 2160), (1280, 720));
        // A 4K screen with 150% scaling exposes a 2560x1440 input space.
        assert_eq!(to_input(640, 360, (1280, 720), (2560, 1440)), (1280, 720));
        assert_eq!(to_input(0, 0, (1280, 720), (1920, 1080)), (0, 0));
        assert_eq!(to_input(5000, -3, (1280, 720), (1920, 1080)), (1918, 0));
    }

    #[test]
    fn key_combos() {
        assert_eq!(
            parse_combo("ctrl+shift+T").unwrap(),
            [
                KeyName::Named("Control"),
                KeyName::Named("Shift"),
                KeyName::Char('t')
            ]
        );
        assert_eq!(parse_combo("cmd+c").unwrap()[0], KeyName::Named("Meta"));
        assert_eq!(parse_combo("Return").unwrap(), [KeyName::Named("Return")]);
        assert_eq!(parse_combo("alt+F4").unwrap()[1], KeyName::Named("F4"));
        assert!(parse_combo("hyper+q").is_err());
        assert!(parse_combo("").is_err());
    }

    #[test]
    fn destructive_combos_are_recognised() {
        for c in [
            "alt+F4",
            "cmd+q",
            "ctrl+alt+del",
            "shift+Delete",
            "Delete",
            "cmd+backspace",
            "ctrl+w",
            "bogus+x",
        ] {
            assert!(is_destructive_combo(c), "{c}");
        }
        for c in ["ctrl+s", "Return", "ctrl+c", "cmd+shift+t", "a"] {
            assert!(!is_destructive_combo(c), "{c}");
        }
    }
}
