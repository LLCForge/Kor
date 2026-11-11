//! Local memory: `MEMORY.md`, session notes, and the auto-dream gate.
//!
//! Files stay on disk. Nothing is uploaded. The background forked extractor in
//! `extractMemories.ts` runs only when a `GrowthBook` gate is on (external default
//! off); the main session still saves and reloads `MEMORY.md` itself.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const ENTRYPOINT: &str = "MEMORY.md";
pub const MAX_ENTRYPOINT_LINES: usize = 200;
pub const MAX_ENTRYPOINT_BYTES: usize = 25_000;
/// The session-notes template. Section headers and the italic lines under them
/// are structure; only the text below an italic line is content.
pub const SESSION_TEMPLATE: &str = "\
# Session Title
_A short and distinctive 5-10 word descriptive title for the session. Super info dense, no filler_

# Current State
_What is actively being worked on right now? Pending tasks not yet completed. Immediate next steps._

# Task specification
_What did the user ask to build? Any design decisions or other explanatory context_

# Files and Functions
_What are the important files? In short, what do they contain and why are they relevant?_

# Workflow
_What bash commands are usually run and in what order? How to interpret their output if not obvious?_

# Errors & Corrections
_Errors encountered and how they were fixed. What did the user correct? What approaches failed and should not be tried again?_

# Codebase and System Documentation
_What are the important system components? How do they work/fit together?_

# Learnings
_What has worked well? What has not? What to avoid? Do not duplicate items from other sections_

# Key results
_If the user asked a specific output such as an answer to a question, a table, or other document, repeat the exact result here_

# Worklog
_Step by step, what was attempted, done? Very terse summary for each step_
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub project_dir: PathBuf,
    pub auto_dir: PathBuf,
    pub entry: PathBuf,
    pub team_dir: PathBuf,
    pub team_entry: PathBuf,
}

/// `sanitizePath`: one implementation shared with the project memory of `kor-memory` (§6.1),
/// so `memory/` and `architecture.md`, `decisions.md`... live in the same project folder.
pub use kor_memory::sanitize_path;

/// `~/.claude/projects/<sanitized>/memory` and the team subdirectory.
pub fn layout(base: &Path, project: &Path) -> Layout {
    let project_dir = base.join("projects").join(sanitize_path(&display(project)));
    let auto_dir = project_dir.join("memory");
    Layout {
        entry: auto_dir.join(ENTRYPOINT),
        team_dir: auto_dir.join("team"),
        team_entry: auto_dir.join("team").join(ENTRYPOINT),
        project_dir,
        auto_dir,
    }
}

fn display(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Priority matches `isAutoMemoryEnabled`: disable env, simple mode, remote
/// without a memory dir, then an explicit settings bool, then on.
pub fn auto_memory_enabled(settings: Option<bool>) -> bool {
    auto_memory_enabled_from(
        crate::env_compat("KOR_DISABLE_AUTO_MEMORY", "CLAUDE_CODE_DISABLE_AUTO_MEMORY").as_deref(),
        truthy_var("KOR_SIMPLE", "CLAUDE_CODE_SIMPLE"),
        truthy_var("KOR_REMOTE", "CLAUDE_CODE_REMOTE")
            && crate::env_compat("KOR_REMOTE_MEMORY_DIR", "CLAUDE_CODE_REMOTE_MEMORY_DIR").is_none(),
        settings,
    )
}

/// Same gate with the environment already read, so tests do not touch process state.
pub fn auto_memory_enabled_from(
    disable: Option<&str>,
    simple: bool,
    remote_without_dir: bool,
    settings: Option<bool>,
) -> bool {
    if let Some(v) = disable {
        if truthy(v) {
            return false;
        }
        if falsy(v) {
            return true;
        }
    }
    if simple || remote_without_dir {
        return false;
    }
    settings.unwrap_or(true)
}

fn truthy_var(kor: &str, claude: &str) -> bool {
    crate::env_compat(kor, claude).is_some_and(|v| truthy(&v))
}

fn truthy(v: &str) -> bool {
    matches!(v, "1" | "true" | "TRUE" | "yes")
}

fn falsy(v: &str) -> bool {
    matches!(v, "0" | "false" | "FALSE" | "no")
}

/// Line cap first, then a byte cap on a newline boundary. A warning is appended
/// when either cap fires, matching `truncateEntrypointContent`.
pub fn truncate_entrypoint(raw: &str) -> String {
    let trimmed = raw.trim();
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let line_hit = lines.len() > MAX_ENTRYPOINT_LINES;
    let byte_hit = trimmed.len() > MAX_ENTRYPOINT_BYTES;
    if !line_hit && !byte_hit {
        return trimmed.to_owned();
    }
    let mut truncated = if line_hit {
        lines[..MAX_ENTRYPOINT_LINES].join("\n")
    } else {
        trimmed.to_owned()
    };
    if truncated.len() > MAX_ENTRYPOINT_BYTES {
        let cut = truncated[..MAX_ENTRYPOINT_BYTES].rfind('\n').unwrap_or(0);
        truncated.truncate(if cut > 0 { cut } else { MAX_ENTRYPOINT_BYTES });
    }
    let reason = match (line_hit, byte_hit) {
        (true, false) => format!("{} lines (limit: {MAX_ENTRYPOINT_LINES})", lines.len()),
        (false, true) => format!(
            "{} bytes (limit: {MAX_ENTRYPOINT_BYTES}) — index entries are too long",
            trimmed.len()
        ),
        _ => format!("{} lines and {} bytes", lines.len(), trimmed.len()),
    };
    format!(
        "{truncated}\n\n> WARNING: {ENTRYPOINT} is {reason}. Only part of it was loaded. \
         Keep index entries to one line under ~200 chars; move detail into topic files."
    )
}

/// System-prompt section. Empty `entrypoint` still tells the model where to write.
pub fn prompt_section(entrypoint: &str, auto_dir: &Path) -> String {
    let body = if entrypoint.trim().is_empty() {
        "(no memories yet)".to_owned()
    } else {
        truncate_entrypoint(entrypoint)
    };
    format!(
        "\n# Auto memory\n\
         Directory: {}\n\
         This directory is for durable notes that are NOT derivable from the repo \
         (user preferences, feedback, project facts, references). Do not save code \
         patterns you can recover with Grep or git.\n\
         Types: user, feedback, project, reference.\n\
         Prefer the `remember` and `forget` tools; without them, save in two steps: write a topic file (frontmatter `name`, `description`, `type`), \
         then add one index line to `{ENTRYPOINT}`: `- [Title](file.md) — one-line hook`. \
         `{ENTRYPOINT}` is an index, not the memory itself. Lines after {MAX_ENTRYPOINT_LINES} \
         are dropped.\n\
         Team notes live in `team/` and must never contain API keys or other secrets.\n\n\
         {body}\n",
        display(auto_dir)
    )
}

/// Write a topic file and an index line. Reloading [`prompt_section`] then shows it.
///
/// # Errors
/// If the filename escapes the memory directory or the disk write fails.
pub fn save_memory(lay: &Layout, filename: &str, title: &str, body: &str) -> io::Result<()> {
    save_memory_as(lay, filename, title, "user", body)
}

/// [`save_memory`] with a memory type (`user`, `feedback`, `project`, `reference`).
///
/// # Errors
/// If the filename escapes the memory directory or the disk write fails.
pub fn save_memory_as(
    lay: &Layout,
    filename: &str,
    title: &str,
    kind: &str,
    body: &str,
) -> io::Result<()> {
    let name = safe_filename(filename)?;
    fs::create_dir_all(&lay.auto_dir)?;
    let topic = lay.auto_dir.join(&name);
    let desc = body.lines().next().unwrap_or(title);
    let file = format!("---\nname: {title}\ndescription: {desc}\ntype: {kind}\n---\n\n{body}\n");
    fs::write(&topic, file)?;
    let mut index = fs::read_to_string(&lay.entry).unwrap_or_default();
    let hook: String = desc.chars().take(120).collect();
    let line = format!("- [{title}]({name}) — {hook}");
    if !index.contains(&name) {
        if !index.is_empty() && !index.ends_with('\n') {
            index.push('\n');
        }
        index.push_str(&line);
        index.push('\n');
        fs::write(&lay.entry, index)?;
    }
    Ok(())
}

/// Remove a topic file and its index line. `Ok(false)` when neither existed.
///
/// # Errors
/// If the filename escapes the memory directory or a disk operation fails.
pub fn forget_memory(lay: &Layout, filename: &str) -> io::Result<bool> {
    let name = safe_filename(filename)?;
    let removed = match fs::remove_file(lay.auto_dir.join(&name)) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e),
    };
    let link = format!("]({name})");
    let Ok(index) = fs::read_to_string(&lay.entry) else {
        return Ok(removed);
    };
    let kept: Vec<&str> = index.lines().filter(|l| !l.contains(&link)).collect();
    if kept.len() == index.lines().count() {
        return Ok(removed);
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    fs::write(&lay.entry, out)?;
    Ok(true)
}

fn is_md(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn safe_filename(name: &str) -> io::Result<String> {
    if name.is_empty()
        || name.contains('\0')
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !is_md(name)
        || name == ENTRYPOINT
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "memory filename must be a single .md topic file",
        ));
    }
    Ok(name.to_owned())
}

pub fn read_entrypoint(lay: &Layout) -> Option<String> {
    fs::read_to_string(&lay.entry).ok()
}

pub fn session_summary_path(lay: &Layout, session_id: &str) -> PathBuf {
    lay.project_dir
        .join(sanitize_path(session_id))
        .join("session-memory")
        .join("summary.md")
}

/// Create the session notes file from the template when it is missing.
///
/// # Errors
/// On directory or file write failure.
pub fn ensure_session_notes(path: &Path) -> io::Result<()> {
    if path.is_file() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, SESSION_TEMPLATE)
}

/// Replace the body of `# Current State`, leaving the header and italic line.
///
/// # Errors
/// If the template structure is missing or the write fails.
pub fn set_current_state(path: &Path, text: &str) -> io::Result<()> {
    ensure_session_notes(path)?;
    let raw = fs::read_to_string(path)?;
    let updated = replace_section(&raw, "# Current State", text).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "session notes are missing the Current State section",
        )
    })?;
    fs::write(path, updated)
}

fn replace_section(raw: &str, header: &str, text: &str) -> Option<String> {
    let mut out = String::new();
    let mut lines = raw.lines().peekable();
    let mut found = false;
    while let Some(line) = lines.next() {
        out.push_str(line);
        out.push('\n');
        if line == header {
            found = true;
            if let Some(italic) = lines.next() {
                out.push_str(italic);
                out.push('\n');
            }
            let body = text.trim();
            if !body.is_empty() {
                out.push_str(body);
                out.push('\n');
            }
            for rest in lines.by_ref() {
                if rest.starts_with("# ") {
                    out.push_str(rest);
                    out.push('\n');
                    break;
                }
            }
        }
    }
    found.then_some(out)
}

/// `autoDream.ts` gate: enough hours since the last consolidation and enough
/// sessions touched since then. The defaults are 24 hours and 5 sessions.
/// The `GrowthBook` `enabled` bit defaults off for external builds; callers pass
/// `enabled` from settings (`autoDreamEnabled`) or a local flag file.
pub fn dream_due(
    enabled: bool,
    last_unix: Option<u64>,
    now_unix: u64,
    sessions_since: u64,
    min_hours: u64,
    min_sessions: u64,
) -> bool {
    if !enabled || min_hours == 0 || min_sessions == 0 {
        return false;
    }
    let elapsed = last_unix.map_or(u64::MAX, |t| now_unix.saturating_sub(t));
    elapsed >= min_hours.saturating_mul(3600) && sessions_since >= min_sessions
}

/// Rebuild `MEMORY.md` as one index line per topic file. Returns the new index.
///
/// # Errors
/// If the directory cannot be read or the index cannot be written.
pub fn rebuild_index(auto_dir: &Path) -> io::Result<String> {
    fs::create_dir_all(auto_dir)?;
    let mut names = Vec::new();
    for ent in fs::read_dir(auto_dir)? {
        let ent = ent?;
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if is_md(name) && name != ENTRYPOINT && ent.path().is_file() {
            names.push(name.to_owned());
        }
    }
    names.sort();
    let mut index = String::new();
    for name in &names {
        let title = name.trim_end_matches(".md");
        let _ = writeln_index(&mut index, title, name);
    }
    fs::write(auto_dir.join(ENTRYPOINT), &index)?;
    Ok(index)
}

fn writeln_index(out: &mut String, title: &str, name: &str) -> std::fmt::Result {
    use std::fmt::Write as _;
    writeln!(out, "- [{title}]({name}) — topic file")
}

/// Apply a selector's filename list. `findRelevantMemories` asks the model, then
/// keeps at most five names that exist, skipping `MEMORY.md` and names already shown.
pub fn select_memories(available: &[String], chosen: &[String], already: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for name in chosen {
        if name == "MEMORY.md" || already.iter().any(|a| a == name) {
            continue;
        }
        if !available.iter().any(|a| a == name) || out.iter().any(|a| a == name) {
            continue;
        }
        out.push(name.clone());
        if out.len() == 5 {
            break;
        }
    }
    out
}

pub const COMPACT_WARNING_BUFFER: u64 = 20_000;

/// `tokenUsage >= autoCompactThreshold - WARNING_THRESHOLD_BUFFER_TOKENS` (`20_000`).
pub fn compact_warning(token_usage: u64, auto_compact_threshold: u64) -> bool {
    token_usage >= auto_compact_threshold.saturating_sub(COMPACT_WARNING_BUFFER)
}

/// Session-memory gate from `sessionMemoryUtils.ts`: init at `10_000` tokens,
/// then `5_000` since the last update, and either 3 tool calls or a turn with none.
pub fn session_memory_due(
    tokens: u64,
    since_update: u64,
    tool_calls: u64,
    last_turn_has_tools: bool,
    initialized: bool,
) -> bool {
    let initialized = initialized || tokens >= 10_000;
    if !initialized {
        return false;
    }
    let token_ok = since_update >= 5_000;
    token_ok && (tool_calls >= 3 || !last_turn_has_tools)
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_prompt_contains_the_index_line() {
        let dir = tempfile::tempdir().unwrap();
        let lay = layout(dir.path(), Path::new("/work/demo"));
        save_memory(&lay, "user_role.md", "Role", "Builds compilers in Rust.").unwrap();
        let raw = read_entrypoint(&lay).unwrap();
        assert!(raw.contains("user_role.md"), "{raw}");
        let section = prompt_section(&raw, &lay.auto_dir);
        assert!(section.contains("[Role](user_role.md)"), "{section}");
        assert!(section.contains("Builds compilers"), "{section}");
        let topic = fs::read_to_string(lay.auto_dir.join("user_role.md")).unwrap();
        assert!(topic.contains("type: user"), "{topic}");
    }

    #[test]
    fn entrypoint_truncates_after_200_lines() {
        let raw = (0..250)
            .map(|i| format!("- [m{i}](m{i}.md) — hook"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = truncate_entrypoint(&raw);
        assert!(out.contains("WARNING"), "{out}");
        assert!(out.lines().filter(|l| l.starts_with("- [m")).count() <= MAX_ENTRYPOINT_LINES);
    }

    #[test]
    fn session_notes_keep_headers_when_state_changes() {
        let dir = tempfile::tempdir().unwrap();
        let lay = layout(dir.path(), Path::new("C:/proj"));
        let path = session_summary_path(&lay, "sess-1");
        set_current_state(&path, "Porting the memory pipeline.").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Porting the memory pipeline."));
        assert!(text.contains("# Task specification"));
        assert!(text.contains("_What did the user ask to build?"));
        set_current_state(&path, "Reloaded into the next prompt.").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Reloaded into the next prompt."));
        assert!(!text.contains("Porting the memory pipeline."));
    }

    #[test]
    fn dream_gate_and_index_rebuild() {
        assert!(!dream_due(false, None, 100, 10, 24, 5));
        assert!(dream_due(true, None, 100, 5, 24, 5));
        assert!(!dream_due(true, Some(0), 3600, 5, 24, 5));
        assert!(dream_due(true, Some(0), 24 * 3600, 5, 24, 5));
        let dir = tempfile::tempdir().unwrap();
        let auto = dir.path().join("memory");
        fs::create_dir_all(&auto).unwrap();
        fs::write(auto.join("feedback_tests.md"), "body").unwrap();
        let index = rebuild_index(&auto).unwrap();
        assert!(index.contains("feedback_tests.md"), "{index}");
    }

    #[test]
    fn disable_env_turns_auto_memory_off() {
        assert!(!auto_memory_enabled_from(
            Some("1"),
            false,
            false,
            Some(true)
        ));
        assert!(auto_memory_enabled_from(Some("0"), true, true, None));
        assert!(!auto_memory_enabled_from(None, true, false, None));
        assert!(auto_memory_enabled_from(None, false, false, None));
    }

    #[test]
    fn relevant_memories_are_capped_at_five() {
        let available = vec![
            "a.md",
            "b.md",
            "c.md",
            "d.md",
            "e.md",
            "f.md",
            "g.md",
            "MEMORY.md",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let chosen = vec![
            "MEMORY.md",
            "a.md",
            "nope.md",
            "b.md",
            "c.md",
            "d.md",
            "e.md",
            "f.md",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let ranked = select_memories(&available, &chosen, &["c.md".into()]);
        assert_eq!(ranked, vec!["a.md", "b.md", "d.md", "e.md", "f.md"]);
        assert!(compact_warning(80_000, 100_000));
        assert!(!compact_warning(79_999, 100_000));
        assert!(session_memory_due(10_000, 5_000, 0, false, false));
        assert!(!session_memory_due(9_999, 5_000, 3, false, false));
    }
}
