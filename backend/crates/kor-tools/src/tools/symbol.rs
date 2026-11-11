//! `EditSymbol` (Master Build Prompt §8.1): structural edit by name. The model says "replace the
//! body of `Cart::total`" instead of quoting old text; tree-sitter finds the definition
//! (`kor_graph::symbols`), the new text goes exactly over its body or the whole definition, and
//! a result that no longer parses is refused. Same rules as `Edit`: inside the project, the file
//! must have been read, the original is kept for `kor undo`, the verified edit loop checks it.

use std::path::Path;

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_graph::lang::Lang;
use kor_graph::symbols;
use serde::Deserialize;
use serde_json::{Value, json};

use super::fs::{
    mtime_ms, read_text, remember, require_fresh_read, resolve_in, writable, write_text,
};
use crate::jail::rel;
use crate::{ReadStamp, Tool, ToolCtx, ToolKind, ToolOutput, parse};

#[derive(Debug)]
pub(crate) struct EditSymbol;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Part {
    #[default]
    Body,
    Whole,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct In {
    #[serde(alias = "path")]
    file_path: String,
    symbol: String,
    #[serde(default)]
    part: Part,
    new_text: String,
    /// 1-based, when the name is defined more than once.
    #[serde(default)]
    occurrence: Option<usize>,
}

fn lang_of(path: &Path) -> Option<Lang> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(Lang::from_ext)
}

/// Remove the common indentation of `text`, then put `indent` before every line but the first
/// (the first lands where the replaced text started, after its own indentation).
fn reindent(text: &str, indent: &str) -> String {
    let lines: Vec<&str> = text.trim_matches('\n').lines().collect();
    let common = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let l = if l.trim().is_empty() {
                ""
            } else {
                &l[common.min(l.len())..]
            };
            if i == 0 || l.is_empty() {
                l.to_owned()
            } else {
                format!("{indent}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The new file text, or why the edit cannot be made.
fn apply(file: &str, lang: Lang, a: &In) -> Result<String, String> {
    let found = symbols::find(lang, file, &a.symbol);
    let s = match (found.len(), a.occurrence) {
        (0, _) => {
            let names: Vec<String> = symbols::definitions(lang, file)
                .into_iter()
                .map(|s| s.qualified)
                .take(40)
                .collect();
            return Err(format!(
                "no function, method or type named `{}` in this file. Defined here: {}",
                a.symbol,
                if names.is_empty() {
                    "(none)".into()
                } else {
                    names.join(", ")
                }
            ));
        }
        (1, None | Some(1)) => &found[0],
        (n, Some(k)) if (1..=n).contains(&k) => &found[k - 1],
        (n, _) => {
            let lines: Vec<String> = found
                .iter()
                .map(|s| format!("{} (line {})", s.qualified, s.line))
                .collect();
            return Err(format!(
                "`{}` matches {n} definitions: {}. Qualify the name (`Type::name`) or pass occurrence (1-{n}).",
                a.symbol,
                lines.join(", ")
            ));
        }
    };
    let (start, end) = match a.part {
        Part::Whole => (s.start, s.end),
        Part::Body => s.body.ok_or_else(|| {
            format!(
                "`{}` has no body to replace; use part \"whole\"",
                s.qualified
            )
        })?,
    };
    let line_start = file[..start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = file[line_start..start]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let mut new = reindent(&a.new_text, &indent);
    if file.contains("\r\n") {
        new = new.replace('\n', "\r\n");
    }
    let out = format!("{}{new}{}", &file[..start], &file[end..]);
    if symbols::parses(lang, file) && !symbols::parses(lang, &out) {
        return Err(format!(
            "the edited `{}` does not parse; nothing was written. Check brackets and indentation of new_text.",
            s.qualified
        ));
    }
    Ok(out)
}

#[async_trait]
impl Tool for EditSymbol {
    fn name(&self) -> &'static str {
        "EditSymbol"
    }
    fn rule_name(&self) -> &'static str {
        "Edit"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Replace a function, method or type by name instead of quoting old text (Rust, Python, TypeScript/JavaScript, Go, Java, C#, C/C++, PHP, Ruby). part \"body\" (default) replaces the body (with its braces in brace languages; for Python the indented block), part \"whole\" the entire definition. Qualify ambiguous names as Type::name. The file must have been Read first; a result that does not parse is refused. Prefer this over Edit for changing whole functions.".into(),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "symbol": {"type": "string", "description": "Name, optionally qualified: total, Cart::total, Cart.total"},
                "part": {"type": "string", "enum": ["body", "whole"]},
                "new_text": {"type": "string"},
                "occurrence": {"type": "integer", "minimum": 1, "description": "Which definition when the name appears more than once"}
            }, "required": ["file_path", "symbol", "new_text"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input
            .get("file_path")
            .or_else(|| input.get("path"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
    fn summary(&self, input: &Value) -> String {
        let f = self.target(input).unwrap_or_default();
        let s = input
            .get("symbol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        format!("{f} :: {s}")
    }
    async fn propose(&self, input: &Value, ctx: &ToolCtx) -> Option<Vec<super::review::Proposal>> {
        let a: In = parse(input.clone()).ok()?;
        let path = writable(ctx, &a.file_path, &a.new_text)?;
        let lang = lang_of(&path)?;
        let before = tokio::fs::read_to_string(&path).await.ok()?;
        require_fresh_read(ctx, &path, &before).ok()?;
        let after = apply(&before, lang, &a).ok()?;
        Some(vec![super::review::proposal(
            ctx,
            &path,
            Some(before),
            Some(after),
        )])
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: In = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let path = match resolve_in(ctx, &a.file_path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let Some(lang) = lang_of(&path) else {
            return ToolOutput::err(
                "EditSymbol supports Rust, Python, TypeScript/JavaScript, Go, Java, C#, C/C++, PHP and Ruby files; use Edit for this file.",
            );
        };
        let file = match read_text(ctx, &path).await {
            Ok(f) => f,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        if let Err(e) = require_fresh_read(ctx, &path, &file) {
            return ToolOutput::err(e);
        }
        let new = match apply(&file, lang, &a) {
            Ok(n) => n,
            Err(e) => return ToolOutput::err(e),
        };
        match write_text(ctx, &path, &new).await {
            Ok(()) => {
                // Like `Edit`: the content is known, but it does not count as a fresh `Read`.
                remember(
                    ctx,
                    &path,
                    ReadStamp {
                        mtime_ms: mtime_ms(&path),
                        partial: false,
                        from_read: false,
                        content: new,
                        offset: 1,
                        limit: None,
                    },
                );
                ToolOutput::ok(format!("edited {} in {}", a.symbol, rel(&ctx.root, &path)))
            }
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(symbol: &str, part: Part, new_text: &str, occurrence: Option<usize>) -> In {
        In {
            file_path: "x".into(),
            symbol: symbol.into(),
            part,
            new_text: new_text.into(),
            occurrence,
        }
    }

    #[test]
    fn replaces_bodies_and_whole_definitions() {
        let src = "struct Cart;\nimpl Cart {\n    fn total(&self) -> u32 {\n        1\n    }\n}\nfn total() -> u32 { 0 }\n";
        let out = apply(
            src,
            Lang::Rust,
            &input(
                "Cart::total",
                Part::Body,
                "{\n    self.items() * 2\n}",
                None,
            ),
        )
        .unwrap();
        assert!(
            out.contains("    fn total(&self) -> u32 {\n        self.items() * 2\n    }\n"),
            "{out}"
        );
        assert!(
            out.ends_with("fn total() -> u32 { 0 }\n"),
            "the free function stays"
        );
        // Ambiguous without a qualifier or an occurrence.
        let e = apply(src, Lang::Rust, &input("total", Part::Body, "{ 3 }", None)).unwrap_err();
        assert!(e.contains("matches 2") && e.contains("line 3"), "{e}");
        let out = apply(
            src,
            Lang::Rust,
            &input("total", Part::Whole, "fn total() -> u32 { 9 }", Some(2)),
        )
        .unwrap();
        assert!(out.ends_with("fn total() -> u32 { 9 }\n"));
        // A result that does not parse is refused.
        assert!(
            apply(
                src,
                Lang::Rust,
                &input("Cart::total", Part::Body, "{ 1 + ", None)
            )
            .unwrap_err()
            .contains("does not parse")
        );
        let e = apply(src, Lang::Rust, &input("Nope", Part::Body, "{}", None)).unwrap_err();
        assert!(e.contains("Cart::total"), "lists what exists: {e}");
    }

    #[test]
    fn python_bodies_are_reindented_and_crlf_kept() {
        let src = "class Cart:\r\n    def total(self):\r\n        return 1\r\n";
        let out = apply(
            src,
            Lang::Python,
            &input("Cart.total", Part::Body, "x = self.n\nreturn x * 2", None),
        )
        .unwrap();
        assert_eq!(
            out,
            "class Cart:\r\n    def total(self):\r\n        x = self.n\r\n        return x * 2\r\n"
        );
        assert_eq!(reindent("  a\n    b\n", "  "), "a\n    b");
    }
}
