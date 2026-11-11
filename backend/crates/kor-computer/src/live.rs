//! Real screen capture and input (feature `live`): `xcap` (GDI/DXGI, CoreGraphics, X11/Wayland
//! portal) and `enigo` (`SendInput`, `CGEvent` with the Accessibility permission, X11 / libei).

use std::io::Cursor;

use base64::Engine as _;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use serde::{Deserialize, Serialize};
use xcap::Monitor;

use crate::keys::{KeyName, shot_size, to_input};

/// One monitor, physical pixels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub primary: bool,
}

/// Every monitor (empty when capture is unavailable, e.g. a headless session).
pub fn displays() -> Vec<Display> {
    Monitor::all()
        .map(|all| {
            all.iter()
                .map(|m| Display {
                    name: m.name().unwrap_or_default(),
                    x: m.x().unwrap_or(0),
                    y: m.y().unwrap_or(0),
                    width: m.width().unwrap_or(0),
                    height: m.height().unwrap_or(0),
                    scale: m.scale_factor().unwrap_or(1.0),
                    primary: m.is_primary().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn primary() -> Result<Monitor, String> {
    let all = Monitor::all().map_err(|e| format!("screen capture unavailable: {e}"))?;
    let idx = all
        .iter()
        .position(|m| m.is_primary().unwrap_or(false))
        .unwrap_or(0);
    all.into_iter()
        .nth(idx)
        .ok_or_else(|| "no monitor found".into())
}

pub fn screen() -> Result<(u32, u32), String> {
    let m = primary()?;
    Ok((
        m.width().map_err(|e| e.to_string())?,
        m.height().map_err(|e| e.to_string())?,
    ))
}

/// A JPEG of the primary screen (base64), its size, and the real screen size.
#[derive(Debug, Clone)]
pub struct Shot {
    pub jpeg_base64: String,
    pub size: (u32, u32),
    pub real: (u32, u32),
}

pub fn screenshot() -> Result<Shot, String> {
    let m = primary()?;
    let img = m
        .capture_image()
        .map_err(|e| format!("capture failed: {e}"))?;
    let real = (img.width(), img.height());
    let (w, h) = shot_size(real.0, real.1);
    let img = if (w, h) == real {
        img
    } else {
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
    };
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok(Shot {
        jpeg_base64: base64::engine::general_purpose::STANDARD.encode(buf.into_inner()),
        size: (w, h),
        real,
    })
}

fn enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| {
        format!(
            "cannot control input: {e} (macOS: allow Kor under Privacy & Security → Accessibility)"
        )
    })
}

/// Pointer position in input coordinates (the human-first check compares it between actions).
pub fn cursor() -> Option<(i32, i32)> {
    enigo().ok()?.location().ok()
}

/// Screenshot pixels → input coordinates.
pub fn map_point(x: i64, y: i64) -> Result<(i32, i32), String> {
    let (sw, sh) = screen()?;
    let e = enigo()?;
    let input = e.main_display().map_err(|e| e.to_string())?;
    Ok(to_input(x, y, shot_size(sw, sh), input))
}

pub fn move_to(x: i32, y: i32) -> Result<(), String> {
    enigo()?
        .move_mouse(x, y, Coordinate::Abs)
        .map_err(|e| e.to_string())
}

pub fn click(x: i32, y: i32, right: bool, double: bool) -> Result<(), String> {
    let mut e = enigo()?;
    e.move_mouse(x, y, Coordinate::Abs)
        .map_err(|e| e.to_string())?;
    let b = if right { Button::Right } else { Button::Left };
    for _ in 0..if double { 2 } else { 1 } {
        e.button(b, Direction::Click).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Press at `from`, move there in small steps (apps track the motion), release at `to`.
pub fn drag(from: (i32, i32), to: (i32, i32)) -> Result<(), String> {
    let mut e = enigo()?;
    e.move_mouse(from.0, from.1, Coordinate::Abs)
        .map_err(|e| e.to_string())?;
    e.button(Button::Left, Direction::Press)
        .map_err(|e| e.to_string())?;
    let steps = 12;
    for i in 1..=steps {
        let x = from.0 + (to.0 - from.0) * i / steps;
        let y = from.1 + (to.1 - from.1) * i / steps;
        let _ = e.move_mouse(x, y, Coordinate::Abs);
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    e.button(Button::Left, Direction::Release)
        .map_err(|e| e.to_string())
}

pub fn type_text(text: &str) -> Result<(), String> {
    enigo()?.text(text).map_err(|e| e.to_string())
}

fn key_of(k: &KeyName) -> Key {
    match k {
        KeyName::Char(c) => Key::Unicode(*c),
        KeyName::Named(n) => match *n {
            "Control" => Key::Control,
            "Shift" => Key::Shift,
            "Alt" => Key::Alt,
            "Meta" => Key::Meta,
            "Return" => Key::Return,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Space" => Key::Space,
            "UpArrow" => Key::UpArrow,
            "DownArrow" => Key::DownArrow,
            "LeftArrow" => Key::LeftArrow,
            "RightArrow" => Key::RightArrow,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "CapsLock" => Key::CapsLock,
            "F1" => Key::F1,
            "F2" => Key::F2,
            "F3" => Key::F3,
            "F4" => Key::F4,
            "F5" => Key::F5,
            "F6" => Key::F6,
            "F7" => Key::F7,
            "F8" => Key::F8,
            "F9" => Key::F9,
            "F10" => Key::F10,
            "F11" => Key::F11,
            _ => Key::F12,
        },
    }
}

/// Press modifiers, tap the last key, release modifiers in reverse.
pub fn combo(keys: &[KeyName]) -> Result<(), String> {
    let mut e = enigo()?;
    let (last, mods) = keys.split_last().ok_or("empty key combination")?;
    let mut pressed = Vec::new();
    let result = (|| {
        for m in mods {
            e.key(key_of(m), Direction::Press)
                .map_err(|x| x.to_string())?;
            pressed.push(key_of(m));
        }
        e.key(key_of(last), Direction::Click)
            .map_err(|x| x.to_string())
    })();
    // Never leave a modifier held down, even after an error.
    for k in pressed.into_iter().rev() {
        let _ = e.key(k, Direction::Release);
    }
    result
}

pub fn scroll(amount: i32, vertical: bool) -> Result<(), String> {
    let axis = if vertical {
        Axis::Vertical
    } else {
        Axis::Horizontal
    };
    enigo()?.scroll(amount, axis).map_err(|e| e.to_string())
}
