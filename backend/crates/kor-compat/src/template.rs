//! Slash-command bodies: `$ARGUMENTS`, `$1`…`$9`, `` !`command` `` and `@file`.
//!
//! Rendering is split so the caller can run shell parts through the permission system
//! and read file parts through the project jail.

use crate::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// `` !`cmd` ``: replaced by the command's output.
    Shell(String),
    /// `@path`: replaced by the file's contents.
    File(String),
}

fn substitute(body: &str, args: &str) -> String {
    let positional: Vec<&str> = args.split_whitespace().collect();
    let mut s = body.replace("$ARGUMENTS", args);
    // Highest index first so `$1` does not eat the start of `$10`-like text.
    for i in (1..=9).rev() {
        s = s.replace(
            &format!("${i}"),
            positional.get(i - 1).copied().unwrap_or(""),
        );
    }
    if !body.contains("$ARGUMENTS") && !body.contains("$1") && !args.trim().is_empty() {
        s.push_str("\n\nARGUMENTS: ");
        s.push_str(args.trim());
    }
    s
}

pub fn parts(cmd: &Command, args: &str) -> Vec<Part> {
    let text = substitute(&cmd.body, args);
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("!`")
            && let Some(end) = after.find('`')
        {
            out.push(Part::Text(std::mem::take(&mut buf)));
            out.push(Part::Shell(after[..end].to_owned()));
            rest = &after[end + 1..];
            continue;
        }
        let ch = rest.chars().next().unwrap_or_default();
        let at_boundary = buf.chars().last().is_none_or(char::is_whitespace);
        if ch == '@' && at_boundary {
            let tok: String = rest[1..]
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            let tok = tok.trim_end_matches(['.', ',', ';', ':', ')']).to_owned();
            if !tok.is_empty() && (tok.contains('.') || tok.contains('/')) {
                out.push(Part::Text(std::mem::take(&mut buf)));
                rest = &rest[1 + tok.len()..];
                out.push(Part::File(tok));
                continue;
            }
        }
        buf.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out.push(Part::Text(buf));
    out.retain(|p| !matches!(p, Part::Text(t) if t.is_empty()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Eco, Scope, Source};

    fn cmd(body: &str) -> Command {
        Command {
            name: "t".into(),
            description: None,
            argument_hint: None,
            allowed_tools: Vec::new(),
            body: body.into(),
            source: Source {
                eco: Eco::Claude,
                scope: Scope::Project,
                path: "x".into(),
            },
        }
    }

    #[test]
    fn arguments_shell_and_files() {
        let p = parts(
            &cmd(
                "Fix issue $1 ($ARGUMENTS).\nStatus: !`git status`\nSee @src/main.rs please, mail a@b.c",
            ),
            "42 high",
        );
        assert_eq!(
            p,
            [
                Part::Text("Fix issue 42 (42 high).\nStatus: ".into()),
                Part::Shell("git status".into()),
                Part::Text("\nSee ".into()),
                Part::File("src/main.rs".into()),
                Part::Text(" please, mail a@b.c".into()),
            ]
        );
    }

    #[test]
    fn arguments_are_appended_when_not_referenced() {
        assert_eq!(
            parts(&cmd("Review"), "carefully"),
            [Part::Text("Review\n\nARGUMENTS: carefully".into())]
        );
        assert_eq!(parts(&cmd("Review"), ""), [Part::Text("Review".into())]);
    }
}
