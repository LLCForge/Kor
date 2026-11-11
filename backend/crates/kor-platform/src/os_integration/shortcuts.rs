//! Shortcuts.app actions for macOS (§27.10): "Kor'a Gönder", "Görev Başlat", "Son Yanıtı Al",
//! "Onayla". Each is an unsigned `WFWorkflow` property list (XML); `shortcuts sign` signs it and
//! `open` shows Shortcuts.app's "Add Shortcut" sheet. Siri and automations can then run them.
//! The two task actions open `kor://task?prompt=…` (the desktop app puts the text in the composer);
//! the other two run the `kor` CLI and show its output.

/// Which shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Share-sheet input → `kor://task`.
    Send,
    /// Ask for text → `kor://task`.
    Start,
    /// `kor os-integration last-reply`.
    LastReply,
    /// `kor os-integration approve`.
    Approve,
}

/// A shortcut file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    /// File stem (ASCII).
    pub file: &'static str,
    pub action: Action,
}

pub const ALL: [Shortcut; 4] = [
    Shortcut {
        file: "kor-send",
        action: Action::Send,
    },
    Shortcut {
        file: "kor-start-task",
        action: Action::Start,
    },
    Shortcut {
        file: "kor-last-reply",
        action: Action::LastReply,
    },
    Shortcut {
        file: "kor-approve",
        action: Action::Approve,
    },
];

/// The question "Görev Başlat" asks (localized by the caller; Turkish by default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texts {
    pub ask: String,
}

impl Default for Texts {
    fn default() -> Self {
        Self {
            ask: "Kor ne yapsın?".into(),
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn key(k: &str, v: &str) -> String {
    format!("<key>{}</key>{v}", esc(k))
}

fn string(s: &str) -> String {
    format!("<string>{}</string>", esc(s))
}

fn dict(items: &[String]) -> String {
    format!("<dict>{}</dict>", items.concat())
}

fn action(id: &str, params: &[String]) -> String {
    dict(&[
        key("WFWorkflowActionIdentifier", &string(id)),
        key("WFWorkflowActionParameters", &dict(params)),
    ])
}

/// `prefix` followed by the output of the action `uuid` (a text token string).
fn token(prefix: &str, uuid: &str, output: &str) -> String {
    let at = prefix.encode_utf16().count();
    dict(&[
        key(
            "Value",
            &dict(&[
                key(
                    "attachmentsByRange",
                    &dict(&[key(
                        &format!("{{{at}, 1}}"),
                        &dict(&[
                            key("OutputName", &string(output)),
                            key("OutputUUID", &string(uuid)),
                            key("Type", &string("ActionOutput")),
                        ]),
                    )]),
                ),
                key("string", &string(&format!("{prefix}\u{FFFC}"))),
            ]),
        ),
        key("WFSerializationType", &string("WFTextTokenString")),
    ])
}

/// Single-quote for `sh`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

const ENCODED: &str = "6B6F7201-0000-4000-8000-000000000001";
const SCRIPT: &str = "6B6F7201-0000-4000-8000-000000000002";

/// The unsigned workflow plist for `s`; `kor` is the CLI the script actions run.
pub fn workflow(s: Shortcut, kor: &str) -> String {
    workflow_with(s, kor, &Texts::default())
}

/// [`workflow`] with localized texts.
pub fn workflow_with(s: Shortcut, kor: &str, t: &Texts) -> String {
    let uuid = |u: &str| key("UUID", &string(u));
    let to_task = |first: Option<String>| {
        let mut v: Vec<String> = first.into_iter().collect();
        v.push(action(
            "is.workflow.actions.urlencode",
            &[uuid(ENCODED), key("WFEncodeMode", &string("Encode"))],
        ));
        v.push(action(
            "is.workflow.actions.gettext",
            &[key(
                "WFTextActionText",
                &token("kor://task?prompt=", ENCODED, "URL Encoded Text"),
            )],
        ));
        v.push(action("is.workflow.actions.openurl", &[]));
        v
    };
    let run = |sub: &str| {
        vec![
            action(
                "is.workflow.actions.runshellscript",
                &[
                    uuid(SCRIPT),
                    key(
                        "Script",
                        &string(&format!("{} os-integration {sub}", sh_quote(kor))),
                    ),
                    key("Shell", &string("/bin/zsh")),
                    key("InputMode", &string("to stdin")),
                ],
            ),
            action(
                "is.workflow.actions.showresult",
                &[key("Text", &token("", SCRIPT, "Shell Script Result"))],
            ),
        ]
    };
    let (actions, inputs, types): (Vec<String>, &[&str], &[&str]) = match s.action {
        Action::Send => (
            to_task(None),
            &["WFStringContentItem", "WFURLContentItem"],
            &["ActionExtension", "QuickActions"],
        ),
        Action::Start => (
            to_task(Some(action(
                "is.workflow.actions.ask",
                &[key("WFAskActionPrompt", &string(&t.ask))],
            ))),
            &[],
            &[],
        ),
        Action::LastReply => (run("last-reply"), &[], &[]),
        Action::Approve => (run("approve"), &[], &[]),
    };
    let array = |v: &[String]| format!("<array>{}</array>", v.concat());
    let strings = |v: &[&str]| array(&v.iter().map(|x| string(x)).collect::<Vec<_>>());
    let body = dict(&[
        key("WFWorkflowActions", &array(&actions)),
        key("WFWorkflowClientVersion", &string("2302.0.4")),
        key(
            "WFWorkflowIcon",
            &dict(&[
                key("WFWorkflowIconGlyphNumber", "<integer>59511</integer>"),
                key("WFWorkflowIconStartColor", "<integer>4271458815</integer>"),
            ]),
        ),
        key("WFWorkflowImportQuestions", "<array/>"),
        key("WFWorkflowInputContentItemClasses", &strings(inputs)),
        key("WFWorkflowMinimumClientVersion", "<integer>900</integer>"),
        key("WFWorkflowMinimumClientVersionString", &string("900")),
        key("WFWorkflowOutputContentItemClasses", "<array/>"),
        key("WFWorkflowTypes", &strings(types)),
    ]);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">{body}</plist>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_shortcuts_open_an_encoded_kor_link() {
        let x = workflow(ALL[1], "/Applications/Kor.app/Contents/MacOS/kor");
        assert!(x.starts_with("<?xml") && x.trim_end().ends_with("</plist>"));
        let ask = x.find("is.workflow.actions.ask").unwrap();
        let enc = x.find("is.workflow.actions.urlencode").unwrap();
        let open = x.find("is.workflow.actions.openurl").unwrap();
        assert!(ask < enc && enc < open);
        // The attachment sits right after the 18-character prefix.
        assert!(x.contains("<key>{18, 1}</key>"));
        assert!(x.contains("<string>kor://task?prompt=\u{FFFC}</string>"));
        assert!(x.contains(&format!("<string>{ENCODED}</string>")));
        let send = workflow(ALL[0], "kor");
        assert!(!send.contains("actions.ask"));
        assert!(
            send.contains("<string>ActionExtension</string>") && send.contains("WFURLContentItem")
        );
    }

    #[test]
    fn cli_shortcuts_quote_the_path_and_show_the_result() {
        let x = workflow(ALL[3], "/Users/o'neil/kor & co/kor");
        assert!(x.contains(
            r"<string>'/Users/o'\''neil/kor &amp; co/kor' os-integration approve</string>"
        ));
        assert!(x.contains("is.workflow.actions.showresult"));
        assert!(x.contains("<key>{0, 1}</key>"));
        let t = Texts {
            ask: "What should Kor do?".into(),
        };
        assert!(workflow_with(ALL[1], "kor", &t).contains("What should Kor do?"));
        assert!(workflow(ALL[2], "kor").contains("os-integration last-reply"));
    }
}
