//! Auto-compact. The buffer is `13_000` tokens. The summary replaces the
//! transcript and is marked with a compact boundary.

use futures::StreamExt;
use kor_core::{ContentPart, Message};
use kor_providers::{CancellationToken, ChatRequest};
use serde_json::json;

use crate::agent::Agent;

pub const AUTOCOMPACT_BUFFER_TOKENS: u64 = 13_000;
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
pub const BOUNDARY: &str = "[compact boundary]";
pub const SESSION_BOUNDARY: &str = "[session memory compact]";

/// Opening of `buildConsolidationPrompt` in `consolidationPrompt.ts`.
pub const DREAM_PROMPT: &str = "You are performing a dream — a reflective pass over your memory files. \
Synthesize what you've learned recently into durable, well-organized memories so that future sessions can orient quickly.\n\
Respond with FILE, TITLE, and BODY lines.";

/// Wording from `getCompactUserSummaryMessage` when session notes are the summary.
pub fn session_memory_summary(notes: &str, transcript: &str) -> String {
    format!(
        "This session is being continued from a previous conversation that ran out of context. \
         The summary below covers the earlier portion of the conversation.\n\n{notes}\n\n\
         If you need specific details from before compaction (like exact code snippets, error \
         messages, or content you generated), read the full transcript at: {transcript}\n\n\
         Recent messages are preserved verbatim.\n\
         Continue the conversation from where it left off without asking the user any further \
         questions. Resume directly — do not acknowledge the summary, do not recap what was \
         happening, do not preface with \"I'll continue\" or similar. Pick up the last task as \
         if the break never happened."
    )
}

pub fn estimate_tokens(messages: &[Message]) -> u64 {
    messages
        .iter()
        .map(|m| {
            m.parts
                .iter()
                .map(|p| match p {
                    ContentPart::Text { text } | ContentPart::Thinking { text, .. } => text.len(),
                    ContentPart::ToolResult { content, .. } => content.len(),
                    ContentPart::ToolUse { name, input, .. } => {
                        name.len() + input.to_string().len()
                    }
                    ContentPart::Image { .. } | ContentPart::File { .. } => 256,
                })
                .sum::<usize>()
                / 4
                + 1
        })
        .sum::<usize>() as u64
}

pub fn threshold(context_window: u64) -> u64 {
    context_window.saturating_sub(AUTOCOMPACT_BUFFER_TOKENS)
}

/// [`threshold`], lowered to `auto_percent` of the window when the user set one (`[context]`).
pub fn auto_threshold(context_window: u64, auto_percent: u8) -> u64 {
    let t = threshold(context_window);
    if auto_percent == 0 {
        t
    } else {
        t.min(context_window.saturating_mul(u64::from(auto_percent)) / 100)
    }
}

/// Summarize when the estimate is past the threshold. Returns whether a boundary was written.
pub async fn maybe_compact(
    agent: &mut Agent,
    cancel: CancellationToken,
) -> Result<bool, kor_core::ProviderError> {
    if !agent.auto_compact || agent.messages.is_empty() {
        return Ok(false);
    }
    if estimate_tokens(&agent.messages)
        <= auto_threshold(agent.context_window, agent.context.auto_percent)
    {
        return Ok(false);
    }
    compact_now(agent, cancel, "auto").await
}

/// [`maybe_compact`] with the §28.1 preview: the parts the summary would replace go to the
/// approver first (desktop `kor:context/compactPreview`, CLI list); the user may keep items
/// verbatim or cancel. With `[context] compact_confirm = false` the list is only shown.
pub async fn maybe_compact_with(
    agent: &mut Agent,
    events: &tokio::sync::mpsc::UnboundedSender<crate::AgentEvent>,
    approver: &dyn crate::Approver,
    cancel: CancellationToken,
) -> Result<bool, kor_core::ProviderError> {
    if !agent.auto_compact || agent.messages.is_empty() {
        return Ok(false);
    }
    if estimate_tokens(&agent.messages)
        <= auto_threshold(agent.context_window, agent.context.auto_percent)
    {
        return Ok(false);
    }
    if estimate_tokens(&agent.messages) < agent.context.compact_snooze {
        return Ok(false);
    }
    let items = crate::ask::compact_items(&agent.messages);
    let tools = items.iter().filter(|i| i.kind == "tool").count();
    let tokens: u64 = items.iter().map(|i| i.tokens).sum();
    let _ = events.send(crate::AgentEvent::Notice(format!(
        "context: summarizing {} messages ({tools} tool outputs, ≈{tokens} tokens)",
        items.len()
    )));
    let keep = match approver
        .compact(&items, agent.context.compact_confirm)
        .await
    {
        crate::ask::CompactChoice::Cancel => {
            // Asked again only after the conversation grows by another tenth of the window.
            agent.context.compact_snooze =
                estimate_tokens(&agent.messages) + agent.context_window / 10;
            let _ = events.send(crate::AgentEvent::Notice(
                "context: summary cancelled".into(),
            ));
            return Ok(false);
        }
        crate::ask::CompactChoice::Proceed { keep } => keep,
    };
    // Pinned items (§28.1 context breakdown) are kept as well.
    let mut keep = keep;
    for p in std::mem::take(&mut agent.context.pinned) {
        if !keep.contains(&p) {
            keep.push(p);
        }
    }
    keep.sort_unstable();
    let kept = crate::ask::kept_message(&agent.messages, &keep);
    let done = compact_now(agent, cancel, "auto").await?;
    if done && let Some(k) = kept {
        agent.messages.push(k);
    }
    Ok(done)
}

/// Summarize now, whatever the size (`/compact`, the desktop's compact button). `trigger` is
/// `auto` or `manual` for the `PreCompact` hook. Returns whether a boundary was written.
pub async fn compact_now(
    agent: &mut Agent,
    cancel: CancellationToken,
    trigger: &str,
) -> Result<bool, kor_core::ProviderError> {
    if agent.messages.is_empty() {
        return Ok(false);
    }
    if let Some(h) = &agent.hooks {
        let o = h.run("PreCompact", None, json!({"trigger": trigger})).await;
        if o.block.is_some() {
            return Ok(false);
        }
    }
    let session_notes = agent
        .session_notes
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .filter(|t| !t.trim().is_empty());
    let system = if let Some(notes) = session_notes {
        session_memory_summary(&notes, "session transcript")
    } else {
        "Respond with text only. Do not call tools. Summarize the conversation so another \
         assistant can continue. Keep file paths, decisions, errors, and unfinished work."
            .into()
    };
    let boundary = if agent.session_notes.is_some() {
        SESSION_BOUNDARY
    } else {
        BOUNDARY
    };
    // `[context] summarize_model`: a cheap or local model writes the summary when configured.
    let (provider, model) = agent
        .context
        .summarizer
        .clone()
        .unwrap_or_else(|| (agent.provider.clone(), agent.model.clone()));
    let req = ChatRequest {
        model,
        system: Some(system),
        messages: agent.messages.clone(),
        tools: Vec::new(),
        max_tokens: agent.max_tokens,
        temperature: None,
        reasoning: None,
    };
    let mut stream = provider.complete(req, cancel).await?;
    let mut summary = String::new();
    while let Some(ev) = stream.next().await {
        if let kor_core::StreamEvent::TextDelta { text } = ev? {
            summary.push_str(&text);
        }
    }
    if summary.trim().is_empty() {
        return Ok(false);
    }
    agent.messages = vec![Message::text(
        kor_core::Role::User,
        format!("{boundary}\n{summary}"),
    )];
    agent.context.warned = false;
    agent.context.compact_snooze = 0;
    agent.context.pinned.clear();
    Ok(true)
}

/// One memory line from an extraction reply, when the local extract flag is on.
pub fn parse_memory(text: &str) -> Option<(&str, &str, &str)> {
    let mut file = None;
    let mut title = None;
    let mut body = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("FILE:") {
            file = Some(v.trim());
        } else if let Some(v) = line.strip_prefix("TITLE:") {
            title = Some(v.trim());
        } else if let Some(v) = line.strip_prefix("BODY:") {
            body = Some(v.trim());
        }
    }
    Some((file?, title?, body?))
}

pub async fn maybe_extract(
    agent: &mut Agent,
    cancel: CancellationToken,
) -> Result<(), kor_core::ProviderError> {
    if !agent.memory_extract {
        return Ok(());
    }
    let Some(base) = &agent.memory_base else {
        return Ok(());
    };
    let req = ChatRequest {
        model: agent.model.clone(),
        system: Some(
            "Respond with exactly three lines: FILE: <name>.md, TITLE: <short>, BODY: <one sentence>. \
             No tools. Only a durable user preference, or FILE: skip.md TITLE: skip BODY: none."
                .into(),
        ),
        messages: agent.messages.clone(),
        tools: Vec::new(),
        max_tokens: Some(200),
        temperature: None,
        reasoning: None,
    };
    let mut stream = agent.provider.complete(req, cancel).await?;
    let mut text = String::new();
    while let Some(ev) = stream.next().await {
        if let kor_core::StreamEvent::TextDelta { text: t } = ev? {
            text.push_str(&t);
        }
    }
    let Some((file, title, body)) = parse_memory(&text) else {
        return Ok(());
    };
    if file == "skip.md" || title.eq_ignore_ascii_case("skip") {
        return Ok(());
    }
    let lay = crate::memory::layout(base, &agent.ctx.root);
    let _ = crate::memory::save_memory(&lay, file, title, body);
    Ok(())
}

/// One consolidation call when the dream gate is open. Closed gate does not call the provider.
pub async fn maybe_dream(
    agent: &mut Agent,
    cancel: CancellationToken,
) -> Result<bool, kor_core::ProviderError> {
    let Some(base) = agent.memory_base.clone() else {
        return Ok(false);
    };
    if !crate::memory::dream_due(true, None, crate::memory::unix_now(), 5, 24, 5) {
        return Ok(false);
    }
    let req = ChatRequest {
        model: agent.model.clone(),
        system: Some(DREAM_PROMPT.into()),
        messages: agent.messages.clone(),
        tools: Vec::new(),
        max_tokens: Some(400),
        temperature: None,
        reasoning: None,
    };
    let mut stream = agent.provider.complete(req, cancel).await?;
    let mut summary = String::new();
    while let Some(ev) = stream.next().await {
        if let kor_core::StreamEvent::TextDelta { text } = ev? {
            summary.push_str(&text);
        }
    }
    let lay = crate::memory::layout(&base, &agent.ctx.root);
    if let Some((file, title, body)) = parse_memory(&summary) {
        let _ = crate::memory::save_memory(&lay, file, title, body);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_reply_writes_the_topic_and_the_index() {
        let flags = crate::flags::Flags {
            memory_extract: true,
            ..crate::flags::Flags::default()
        };
        assert!(flags.memory_extract);
        assert!(!crate::flags::Flags::default().memory_extract);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let (file, title, body) =
            parse_memory("FILE: user_role.md\nTITLE: Role\nBODY: likes rust").unwrap();
        let lay = crate::memory::layout(dir.path(), &project);
        if flags.memory_extract {
            crate::memory::save_memory(&lay, file, title, body).unwrap();
        }
        let index = crate::memory::read_entrypoint(&lay).unwrap();
        assert!(index.contains("[Role](user_role.md)"), "{index}");
        let topic = std::fs::read_to_string(lay.auto_dir.join(file)).unwrap();
        assert!(topic.contains("likes rust"), "{topic}");
    }

    #[test]
    fn user_percent_lowers_the_auto_threshold() {
        assert_eq!(auto_threshold(200_000, 0), 187_000);
        assert_eq!(auto_threshold(200_000, 50), 100_000);
        assert_eq!(auto_threshold(20_000, 99), 7_000);
    }

    #[test]
    fn session_memory_prompt_is_distinct() {
        let text = session_memory_summary("notes body", "/tmp/transcript.jsonl");
        assert!(text.contains("Recent messages are preserved verbatim."));
        assert!(text.contains("notes body"));
        assert!(text.contains("/tmp/transcript.jsonl"));
        assert!(!text.contains("Respond with text only"));
    }
}
