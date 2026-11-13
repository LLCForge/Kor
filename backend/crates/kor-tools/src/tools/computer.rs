//! Desktop control: `screenshot`, `left_click`, `right_click`, `double_click`,
//! `mouse_move`, `type`, `key`, `scroll`.
//!
//! Real input on Windows (`SendInput`), macOS (`CGEvent`; needs the Accessibility
//! permission) and Linux (X11, or Wayland through libei where the compositor offers it),
//! via `enigo`. Screen capture via `xcap` (GDI/DXGI, CoreGraphics, X11/Wayland portal).
//!
//! The screenshot is scaled down to at most 1280 px wide and returned to the model as an
//! image. Coordinates the model sends are in that screenshot's pixels and are mapped to
//! the input coordinate space here, so DPI scaling and resolution never leak to the model.
//!
//! Every action goes through the permission engine (desktop tools ask unless the app's tier is
//! `full`) and through `kor_computer::begin` (per-app tier, human-first stop, indicator, audit;
//! ADR 0059). Capture and input live in `kor_computer::live`. Off by default: config
//! `computer.enabled`; `KOR_COMPUTER_USE=1|0` overrides.

use async_trait::async_trait;
use kor_computer::policy::Action;
use kor_core::ToolSpec;
use serde_json::{Value, json};

use crate::{Tool, ToolCtx, ToolKind, ToolOutput};

#[cfg(feature = "computer")]
use kor_computer::keys::parse_combo;

pub const COMPUTER_TOOLS: &[&str] = &[
    "screenshot",
    "left_click",
    "right_click",
    "double_click",
    "mouse_move",
    "type",
    "key",
    "scroll",
    "left_click_drag",
    "undo_last_action",
];

/// One desktop action and the screen as it was just before it (screenshot-backed undo).
#[derive(Debug, Clone)]
pub struct Step {
    pub action: String,
    pub input: Value,
    /// JPEG, base64, as the model sees screenshots.
    pub before: String,
}

/// The last actions, newest last (each holds one scaled screenshot, so the list stays short).
const MAX_STEPS: usize = 20;
static STEPS: std::sync::Mutex<Vec<Step>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn push_step(step: Step) {
    let mut s = STEPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.len() >= MAX_STEPS {
        s.remove(0);
    }
    s.push(step);
}

pub(crate) fn pop_step() -> Option<Step> {
    STEPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
}

/// How an action is reversed: typed text is erased with Backspace, a key combination or a drag
/// with the app's own undo shortcut. Clicks, pointer moves and scrolls have no general reverse;
/// the screenshot from before is what the model restores by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reverse {
    Backspace(usize),
    Shortcut(&'static str),
    ByHand,
}

pub fn reverse_of(action: &str, input: &Value) -> Reverse {
    let undo = if kor_platform::os() == kor_platform::Os::Mac {
        "cmd+z"
    } else {
        "ctrl+z"
    };
    match action {
        // A newline usually submitted something (a form, a chat): erasing would not undo that.
        "type" => match input["text"].as_str() {
            Some(t) if !t.contains('\n') => Reverse::Backspace(t.chars().count()),
            _ => Reverse::Shortcut(undo),
        },
        "key" | "left_click_drag" => Reverse::Shortcut(undo),
        _ => Reverse::ByHand,
    }
}

pub fn is_computer_tool(name: &str) -> bool {
    COMPUTER_TOOLS.contains(&name) || super::desktop::DESKTOP_COMPUTER_TOOLS.contains(&name)
}

/// What a pointer / keyboard tool does, for the app's tier (§19.1), and whether it is destructive
/// (closing apps, deleting: always approved by the user).
pub fn action_of(tool: &str, input: &Value) -> (Action, bool) {
    match tool {
        "screenshot" => (Action::Observe, false),
        "left_click" | "double_click" | "mouse_move" | "scroll" => (Action::Click, false),
        "right_click" | "left_click_drag" => (Action::Pointer, false),
        "key" => (
            Action::Type,
            kor_computer::keys::is_destructive_combo(input["text"].as_str().unwrap_or_default()),
        ),
        _ => (Action::Type, false),
    }
}

fn coord(input: &Value) -> Option<(i64, i64)> {
    let c = &input["coordinate"];
    let x = c
        .get(0)
        .and_then(Value::as_i64)
        .or_else(|| input["x"].as_i64())?;
    let y = c
        .get(1)
        .and_then(Value::as_i64)
        .or_else(|| input["y"].as_i64())?;
    Some((x, y))
}

#[cfg(feature = "computer")]
fn start(input: &Value) -> Option<(i64, i64)> {
    let c = &input["start_coordinate"];
    Some((c.get(0)?.as_i64()?, c.get(1)?.as_i64()?))
}

static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// From config `computer.enabled` (on by default, ADR 0059), set once when a session is prepared.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// `KOR_COMPUTER_USE=1|0` overrides the config either way.
pub(crate) fn disabled() -> bool {
    match std::env::var("KOR_COMPUTER_USE").ok().as_deref() {
        Some("1" | "true") => false,
        Some("0" | "false") => true,
        _ => !ENABLED.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// Desktop control is on for this process.
pub fn enabled() -> bool {
    !disabled()
}

#[cfg(feature = "computer")]
fn act(action: &str, input: &Value) -> ToolOutput {
    use kor_computer::live;
    // Human first between the low-level events of a long action, not only between actions.
    let human = || kor_computer::step(action);
    let r: Result<ToolOutput, String> = (|| match action {
        "screenshot" => {
            let shot = live::screenshot()?;
            let ((w, h), (rw, rh)) = (shot.size, shot.real);
            let mut out = ToolOutput::ok(format!(
                "Screenshot {w}x{h} (screen {rw}x{rh}). Give coordinates in this image's pixels."
            ));
            out.image = Some(kor_core::ImageSource::Base64 {
                media_type: "image/jpeg".into(),
                data: shot.jpeg_base64,
            });
            Ok(out)
        }
        "left_click" | "right_click" | "double_click" | "mouse_move" => {
            let (x, y) = coord(input).ok_or("`coordinate` [x, y] is required")?;
            let (ix, iy) = live::map_point(x, y)?;
            if action == "mouse_move" {
                live::move_to(ix, iy)?;
            } else {
                live::click(ix, iy, action == "right_click", action == "double_click")?;
            }
            Ok(ToolOutput::ok(format!("{action} at ({x}, {y})")))
        }
        "left_click_drag" => {
            let (x1, y1) = start(input).ok_or("`start_coordinate` [x, y] is required")?;
            let (x2, y2) = coord(input).ok_or("`coordinate` [x, y] is required")?;
            let from = live::map_point(x1, y1)?;
            let to = live::map_point(x2, y2)?;
            live::drag(from, to, &human)?;
            Ok(ToolOutput::ok(format!(
                "dragged ({x1}, {y1}) -> ({x2}, {y2})"
            )))
        }
        "type" => {
            let text = input["text"].as_str().ok_or("`text` is required")?;
            live::type_text(text, &human)?;
            Ok(ToolOutput::ok(format!(
                "typed {} characters",
                text.chars().count()
            )))
        }
        "key" => {
            let combo = input["text"]
                .as_str()
                .ok_or("`text` (e.g. ctrl+s) is required")?;
            live::combo(&parse_combo(combo)?)?;
            Ok(ToolOutput::ok(format!("pressed {combo}")))
        }
        "scroll" => {
            if let Some((x, y)) = coord(input) {
                let (ix, iy) = live::map_point(x, y)?;
                live::move_to(ix, iy)?;
            }
            let amount =
                i32::try_from(input["amount"].as_u64().unwrap_or(3).clamp(1, 50)).unwrap_or(3);
            let dir = input["direction"].as_str().unwrap_or("down");
            let (n, vertical) = match dir {
                "up" => (-amount, true),
                "left" => (-amount, false),
                "right" => (amount, false),
                _ => (amount, true),
            };
            live::scroll(n, vertical, &human)?;
            Ok(ToolOutput::ok(format!("scrolled {dir} {amount}")))
        }
        "undo_last_action" => {
            let step = pop_step()
                .ok_or("nothing to undo: no desktop action was recorded in this session")?;
            let done = match reverse_of(&step.action, &step.input) {
                Reverse::Backspace(n) => {
                    let times = n.min(2_000);
                    live::repeat_combo(&parse_combo("backspace")?, times, &human)?;
                    format!("erased the {n} typed characters with Backspace")
                }
                Reverse::Shortcut(k) => {
                    live::combo(&parse_combo(k)?)?;
                    format!("pressed {k} (the app's undo)")
                }
                Reverse::ByHand => {
                    "this action has no automatic reverse; nothing was pressed".to_owned()
                }
            };
            let mut out = ToolOutput::ok(format!(
                "Undo of `{}`: {done}. The attached image is the screen just before that action; take a \
                 screenshot and compare. If they differ, restore it by hand (click, scroll back) or tell the user.",
                step.action
            ));
            out.image = Some(kor_core::ImageSource::Base64 {
                media_type: "image/jpeg".into(),
                data: step.before,
            });
            Ok(out)
        }
        other => Err(format!("unknown desktop action `{other}`")),
    })();
    r.unwrap_or_else(ToolOutput::err)
}

#[cfg(feature = "computer")]
fn before_shot() -> Option<String> {
    kor_computer::live::screenshot().ok().map(|s| s.jpeg_base64)
}

#[cfg(not(feature = "computer"))]
fn before_shot() -> Option<String> {
    None
}

#[cfg(not(feature = "computer"))]
fn act(_action: &str, _input: &Value) -> ToolOutput {
    ToolOutput::err("this build of Kor has no desktop control (feature `computer` is off)")
}

fn schema(required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": {
            "coordinate": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2,
                "description": "[x, y] in pixels of the latest screenshot"},
            "start_coordinate": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2,
                "description": "Drag start [x, y] in pixels of the latest screenshot"},
            "text": {"type": "string"},
            "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
            "amount": {"type": "integer", "minimum": 1, "maximum": 50}
        },
        "required": required,
        "additionalProperties": false
    })
}

macro_rules! computer_tool {
    ($ty:ident, $name:literal, $kind:expr, $required:expr, $about:literal) => {
        #[derive(Debug)]
        pub(crate) struct $ty;

        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &'static str {
                $name
            }
            fn rule_name(&self) -> &'static str {
                $name
            }
            fn kind(&self) -> ToolKind {
                $kind
            }
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: $name.into(),
                    description: $about.into(),
                    input_schema: schema($required),
                }
            }
            fn target(&self, input: &Value) -> Option<String> {
                input["text"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| coord(input).map(|(x, y)| format!("{x},{y}")))
                    .or_else(|| Some($name.to_owned()))
            }
            async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
                if disabled() {
                    return ToolOutput::err("desktop control is off; turn it on with `kor config set computer.enabled true`");
                }
                if cfg!(test) {
                    // Unit tests must never move the real pointer or read the real screen.
                    return ToolOutput::err("desktop actions are not run in tests");
                }
                let action = $name;
                tokio::task::spawn_blocking(move || {
                    // §19.1: the app's tier, the human-first stop, the indicator and the audit log.
                    let (kind, destructive) = action_of(action, &input);
                    let ticket = match kor_computer::begin(action, kind, kor_computer::target_app(None), destructive) {
                        Ok(t) => t,
                        Err(e) => return ToolOutput::err(e),
                    };
                    // The screen before any action that changes something, for `undo_last_action`.
                    let before = (kind != Action::Observe && action != "undo_last_action")
                        .then(before_shot)
                        .flatten();
                    let out = act(action, &input);
                    if let (Some(before), false) = (before, out.is_error) {
                        push_step(Step { action: action.to_owned(), input: input.clone(), before });
                    }
                    ticket.finish(!out.is_error);
                    out
                })
                .await
                .unwrap_or_else(|e| ToolOutput::err(e.to_string()))
            }
        }
    };
}

computer_tool!(
    Screenshot,
    "screenshot",
    ToolKind::Read,
    &[],
    "Capture the primary screen and see it as an image. Take one before clicking; coordinates refer to the latest screenshot."
);
computer_tool!(
    LeftClick,
    "left_click",
    ToolKind::Execute,
    &["coordinate"],
    "Left-click at [x, y] of the latest screenshot."
);
computer_tool!(
    RightClick,
    "right_click",
    ToolKind::Execute,
    &["coordinate"],
    "Right-click at [x, y] of the latest screenshot."
);
computer_tool!(
    DoubleClick,
    "double_click",
    ToolKind::Execute,
    &["coordinate"],
    "Double-click at [x, y] of the latest screenshot."
);
computer_tool!(
    MouseMove,
    "mouse_move",
    ToolKind::Execute,
    &["coordinate"],
    "Move the pointer to [x, y] of the latest screenshot."
);
computer_tool!(
    TypeText,
    "type",
    ToolKind::Execute,
    &["text"],
    "Type text into the focused window."
);
computer_tool!(
    KeyPress,
    "key",
    ToolKind::Execute,
    &["text"],
    "Press a key or combination, e.g. Return, ctrl+s, cmd+shift+t, alt+F4."
);
computer_tool!(
    LeftClickDrag,
    "left_click_drag",
    ToolKind::Execute,
    &["start_coordinate", "coordinate"],
    "Press the left button at start_coordinate, drag to coordinate and release (move, resize, select a range). Both in pixels of the latest screenshot."
);
computer_tool!(
    Scroll,
    "scroll",
    ToolKind::Execute,
    &["direction"],
    "Scroll up/down/left/right by `amount` notches, optionally at [x, y]."
);
computer_tool!(
    UndoLastAction,
    "undo_last_action",
    ToolKind::Execute,
    &[],
    "Undo the last desktop action: typed text is erased, a key or drag gets the app's undo shortcut; returns the screenshot from just before it so you can check and restore the rest by hand."
);

#[cfg(test)]
mod undo_tests {
    use super::*;

    #[test]
    fn reverses_typing_keys_and_drags_and_keeps_order() {
        assert_eq!(
            reverse_of("type", &json!({"text": "héllo"})),
            Reverse::Backspace(5)
        );
        let undo = if kor_platform::os() == kor_platform::Os::Mac {
            "cmd+z"
        } else {
            "ctrl+z"
        };
        assert_eq!(
            reverse_of("type", &json!({"text": "a\nb"})),
            Reverse::Shortcut(undo)
        );
        assert_eq!(
            reverse_of("key", &json!({"text": "ctrl+x"})),
            Reverse::Shortcut(undo)
        );
        assert_eq!(reverse_of("left_click", &json!({})), Reverse::ByHand);
        for i in 0..(MAX_STEPS + 2) {
            push_step(Step {
                action: format!("a{i}"),
                input: json!({}),
                before: String::new(),
            });
        }
        assert_eq!(
            pop_step().map(|s| s.action),
            Some(format!("a{}", MAX_STEPS + 1))
        );
        let mut n = 1;
        while pop_step().is_some() {
            n += 1;
        }
        assert_eq!(n, MAX_STEPS, "only the last steps are kept");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_map_to_tiers() {
        assert_eq!(
            action_of("screenshot", &json!({})),
            (Action::Observe, false)
        );
        assert_eq!(
            action_of("double_click", &json!({})),
            (Action::Click, false)
        );
        assert_eq!(
            action_of("left_click_drag", &json!({})),
            (Action::Pointer, false)
        );
        assert_eq!(
            action_of("key", &json!({"text": "alt+F4"})),
            (Action::Type, true)
        );
        assert_eq!(
            action_of("key", &json!({"text": "ctrl+s"})),
            (Action::Type, false)
        );
        assert_eq!(
            action_of("type", &json!({"text": "hi"})),
            (Action::Type, false)
        );
    }

    /// Reads the real screen (no input is sent). Run with `--ignored` on a desktop session.
    #[cfg(feature = "computer")]
    #[test]
    #[ignore = "needs a desktop session"]
    fn live_screenshot_and_mapping() {
        use kor_computer::keys::MAX_SHOT_WIDTH;
        use kor_computer::live;
        let shot = live::screenshot().unwrap();
        let ((w, h), (rw, rh)) = (shot.size, shot.real);
        eprintln!(
            "screen {rw}x{rh} -> shot {w}x{h}, {} KB",
            shot.jpeg_base64.len() / 1024
        );
        assert!(w <= MAX_SHOT_WIDTH && w > 0 && h > 0 && shot.jpeg_base64.len() > 1000);
        let (x, y) = live::map_point(i64::from(w) - 1, i64::from(h) - 1).unwrap();
        eprintln!("bottom-right of shot maps to input ({x}, {y})");
        assert!(x > 0 && y > 0);
    }

    #[tokio::test]
    async fn tools_never_act_in_tests_and_schemas_are_exposed() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        assert!(
            LeftClick
                .call(json!({"coordinate": [1, 1]}), &ctx)
                .await
                .is_error
        );
        assert!(is_computer_tool("double_click") && !is_computer_tool("Read"));
        assert_eq!(KeyPress.spec().input_schema["required"][0], "text");
    }
}
