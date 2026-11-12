//! `kor comment --lang <dil>` (Master Build Prompt §12.2): translate a file's code comments into
//! the target language and, with `--missing`, write one-line doc comments for functions that have
//! none. Comments come from tree-sitter (`kor_graph::comments`), so strings that look like
//! comments are never touched; the model only sees comment text and signatures. Every reply is
//! checked before it reaches the file: same number of items, same comment markers, line comments
//! stay one line. Anything that fails a check keeps its original text.

use std::sync::Arc;

use kor_graph::comments::{self, Comment};
use kor_graph::lang::Lang;
use kor_providers::Provider;

/// Comments per model call (keeps each prompt and reply small).
const BATCH: usize = 40;

/// What one file run changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    pub translated: usize,
    /// Comments whose translation failed a check and stayed as they were.
    pub kept: usize,
    pub added: usize,
}

/// `tr` → `Turkish`; anything else is passed to the model as written.
pub fn language_name(code: &str) -> String {
    match code.to_ascii_lowercase().as_str() {
        "tr" => "Turkish",
        "en" => "English",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "ar" => "Arabic",
        "ja" => "Japanese",
        "zh" => "Chinese",
        _ => return code.to_owned(),
    }
    .to_owned()
}

/// Leading marker (`//`, `///`, `/**`, `#`, `--`…) and, for block comments, the closing one.
fn markers(c: &str) -> (&str, &str) {
    let head_len = c
        .find(|ch: char| ch.is_alphanumeric() || ch.is_whitespace())
        .unwrap_or(c.len());
    let head = &c[..head_len];
    let tail = if head.starts_with("/*") && c.ends_with("*/") {
        "*/"
    } else {
        ""
    };
    (head, tail)
}

/// The model's JSON string array (the first `[` … last `]` of the reply).
fn parse_list(reply: &str) -> Option<Vec<String>> {
    let (a, b) = (reply.find('[')?, reply.rfind(']')?);
    serde_json::from_str(reply.get(a..=b)?).ok()
}

/// A translation is used only when it keeps the original's markers and line shape.
fn acceptable(orig: &str, new: &str) -> bool {
    let (h, t) = markers(orig);
    let (nh, nt) = markers(new);
    let one_line = !orig.contains('\n');
    !new.trim().is_empty()
        && h == nh
        && t == nt
        && (!one_line || !new.contains('\n'))
        && new.len() <= orig.len() * 4 + 200
}

fn translate_prompt(batch: &[&Comment], lang: &str) -> String {
    let items: Vec<&str> = batch.iter().map(|c| c.text.as_str()).collect();
    format!(
        "Translate these source code comments into {lang}. Rules: return ONLY a JSON array of strings, \
         one per input, same order and count. Keep every comment marker exactly (//, ///, //!, /* */, /** */, #, etc.), \
         keep line breaks and leading `*` of block comments, keep code identifiers, paths, URLs and text in backticks \
         unchanged. A comment already in {lang} is returned unchanged.\n\n{}",
        serde_json::to_string(&items).unwrap_or_default()
    )
}

fn missing_prompt(sigs: &[&str], lang: &str) -> String {
    format!(
        "For each function signature below, write ONE short line in {lang} describing what it does (no comment markers, \
         no line breaks, at most 120 characters). Return ONLY a JSON array of strings, same order and count.\n\n{}",
        serde_json::to_string(sigs).unwrap_or_default()
    )
}

async fn ask(provider: &Arc<dyn Provider>, model: &str, prompt: String) -> Result<String, String> {
    kor_agent::gitflow::ask(provider, model, prompt, 4000)
        .await
        .map_err(|e| e.to_string())
}

/// Translate (and with `missing`, add) comments of `src` written in `lang` into `target`.
///
/// # Errors
/// When the model call fails.
pub async fn run(
    provider: &Arc<dyn Provider>,
    model: &str,
    lang: Lang,
    src: &str,
    target: &str,
    translate: bool,
    missing: bool,
) -> Result<Outcome, String> {
    let target = language_name(target);
    // (byte position, bytes to drop, replacement)
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let (mut translated, mut kept, mut added) = (0, 0, 0);
    if translate {
        let found = comments::comments(lang, src);
        let todo: Vec<&Comment> = found
            .iter()
            .filter(|c| c.text.chars().any(char::is_alphabetic))
            .collect();
        for batch in todo.chunks(BATCH) {
            let reply = ask(provider, model, translate_prompt(batch, &target)).await?;
            let list = parse_list(&reply).filter(|l| l.len() == batch.len());
            for (i, c) in batch.iter().enumerate() {
                match list.as_ref().map(|l| l[i].clone()) {
                    Some(new) if acceptable(&c.text, &new) => {
                        if new != c.text {
                            edits.push((c.start, c.end - c.start, new));
                            translated += 1;
                        }
                    }
                    _ => kept += 1,
                }
            }
        }
    }
    if let Some(marker) = comments::doc_marker(lang).filter(|_| missing) {
        let undoc = comments::undocumented(lang, src);
        for batch in undoc.chunks(BATCH) {
            let sigs: Vec<&str> = batch.iter().map(|u| u.signature.as_str()).collect();
            let reply = ask(provider, model, missing_prompt(&sigs, &target)).await?;
            let Some(list) = parse_list(&reply).filter(|l| l.len() == batch.len()) else {
                continue;
            };
            for (u, line) in batch.iter().zip(list) {
                let line: String = line
                    .replace(['\r', '\n'], " ")
                    .trim()
                    .trim_start_matches(['/', '#', '*', ' '])
                    .chars()
                    .take(160)
                    .collect();
                if line.is_empty() || line.contains("*/") {
                    continue;
                }
                let nl = if src.contains("\r\n") { "\r\n" } else { "\n" };
                edits.push((u.line_start, 0, format!("{}{marker} {line}{nl}", u.indent)));
                added += 1;
            }
        }
    }
    // Back to front, so earlier byte positions stay valid.
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut text = src.to_owned();
    for (at, len, with) in edits {
        text.replace_range(at..at + len, &with);
    }
    Ok(Outcome {
        text,
        translated,
        kept,
        added,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use kor_core::{ProviderError, StreamEvent};
    use kor_providers::{
        CancellationToken, ChatRequest, EventStream, ModelInfo, ProviderManifest, Registry,
    };

    use super::*;

    struct Scripted {
        manifest: ProviderManifest,
        replies: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Provider for Scripted {
        fn manifest(&self) -> &ProviderManifest {
            &self.manifest
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(Vec::new())
        }
        async fn complete(
            &self,
            _req: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, ProviderError> {
            let text = self.replies.lock().unwrap().remove(0);
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                StreamEvent::TextDelta { text },
            )])))
        }
    }

    fn scripted(replies: &[&str]) -> Arc<dyn Provider> {
        Arc::new(Scripted {
            manifest: Registry::builtin().get("local").unwrap().clone(),
            replies: Mutex::new(replies.iter().map(|s| (*s).to_owned()).collect()),
        })
    }

    #[tokio::test]
    async fn translates_checked_comments_and_adds_missing_docs() {
        let src = "// read the file\nfn a() {\n    let s = \"// keep\"; // ---\n}\n\n/* block\n * two */\nfn b() {}\n\nfn c() {}\n";
        // Two comments with letters; the second reply drops its marker and must be refused.
        let p = scripted(&[
            "Here:\n[\"// dosyayı oku\", \"block iki\"]",
            "[\"İkinci fonksiyon\"]",
        ]);
        let out = run(&p, "m", Lang::Rust, src, "tr", true, true)
            .await
            .unwrap();
        assert_eq!(out.translated, 1);
        assert_eq!(out.kept, 1);
        assert_eq!(out.added, 1);
        assert!(
            out.text.starts_with("// dosyayı oku\nfn a()"),
            "{}",
            out.text
        );
        assert!(out.text.contains("\"// keep\"; // ---"));
        // `b` already has the block comment above it; only `c` gets one.
        assert!(
            out.text
                .contains("/* block\n * two */\nfn b() {}\n\n/// İkinci fonksiyon\nfn c() {}"),
            "{}",
            out.text
        );
    }

    #[test]
    fn markers_and_shapes_are_checked() {
        assert!(acceptable("// hello", "// merhaba"));
        assert!(!acceptable("// hello", "merhaba"));
        assert!(!acceptable("/// doc", "// doc"));
        assert!(!acceptable("// one", "// bir\n// iki"));
        assert!(acceptable("/* a\n b */", "/* x\n y */"));
        assert!(!acceptable("/* a */", "/* x"));
        assert!(acceptable("# py", "# piton"));
        assert_eq!(language_name("TR"), "Turkish");
        assert_eq!(language_name("Klingon"), "Klingon");
        assert_eq!(parse_list("x [\"a\"] y"), Some(vec!["a".to_owned()]));
        assert_eq!(parse_list("nope"), None);
    }
}
