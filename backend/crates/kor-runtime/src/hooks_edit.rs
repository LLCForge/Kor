//! Hook editor backend (Master Build Prompt §28.12, ADR 0063): list, add, edit, remove and dry-run
//! hooks in Kor's `hooks.toml` (user and project) and Claude Code `settings.json` files, for the
//! desktop's visual editor and `kor hooks add|edit|rm|test|log`.
//!
//! Kor files are edited with `toml_edit`, so comments and formatting stay; Claude Code files keep
//! every other key. Claude Code hooks stay plain commands (that format has nothing else), so they
//! keep working in Claude Code unchanged; conditions, MCP calls and notifications are Kor-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_agent::hooks::{HookRunner, KOR_EVENTS};
use kor_compat::{Eco, Hook, KorHook, Scope, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Claude Code hook events Kor runs.
pub const CLAUDE_EVENTS: &[&str] = &[
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    "Stop",
    "SubagentStop",
    "Notification",
    "SessionStart",
    "SessionEnd",
    "PreCompact",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Kor,
    Claude,
}

/// One hook as the editor shows and edits it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    /// `<index>` within its event (Kor) or `<group>.<hook>` (Claude); set by [`list`].
    pub id: String,
    pub file: String,
    pub format: Option<Format>,
    /// `user`, `project` or `local`.
    pub scope: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: Option<String>,
    pub timeout: Option<u64>,
    pub files: Vec<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub mcp: Option<String>,
    /// JSON object text.
    pub args: Option<String>,
    pub notify: Option<String>,
    pub disabled: bool,
}

/// The hook files of a project, in the order they apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookFile {
    pub path: PathBuf,
    pub format: Format,
    pub scope: String,
    pub exists: bool,
}

/// Every file the editor can write: Kor user/project, Claude user/project/local.
pub fn files(root: &Path, home: Option<&Path>, user_dir: Option<&Path>) -> Vec<HookFile> {
    let mut out = Vec::new();
    let mut push = |path: PathBuf, format, scope: &str| {
        out.push(HookFile {
            exists: path.is_file(),
            path,
            format,
            scope: scope.into(),
        });
    };
    if let Some(u) = user_dir {
        push(u.join("hooks.toml"), Format::Kor, "user");
    }
    push(root.join(".kor").join("hooks.toml"), Format::Kor, "project");
    if let Some(h) = home {
        push(
            h.join(".claude").join("settings.json"),
            Format::Claude,
            "user",
        );
    }
    push(
        root.join(".claude").join("settings.json"),
        Format::Claude,
        "project",
    );
    push(
        root.join(".claude").join("settings.local.json"),
        Format::Claude,
        "local",
    );
    out
}

fn read_toml(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(json!({})),
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("kor-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn s(t: &toml_edit::Table, k: &str) -> Option<String> {
    t.get(k).and_then(|v| v.as_str()).map(str::to_owned)
}

fn kor_items(f: &HookFile) -> Result<Vec<Item>, String> {
    let doc = read_toml(&f.path)?;
    let mut out = Vec::new();
    for (event, v) in doc.iter() {
        let Some(arr) = v.as_array_of_tables() else {
            continue;
        };
        for (i, t) in arr.iter().enumerate() {
            let files = match t.get("files") {
                Some(v) if v.is_str() => v.as_str().map(|x| vec![x.to_owned()]).unwrap_or_default(),
                Some(v) => v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            let args = t.get("args").map(|a| {
                let text = format!("x = {a}");
                toml::from_str::<toml::Table>(&text)
                    .ok()
                    .and_then(|tb| {
                        tb.get("x")
                            .map(|v| serde_json::to_value(v).unwrap_or(Value::Null).to_string())
                    })
                    .unwrap_or_default()
            });
            out.push(Item {
                id: i.to_string(),
                file: f.path.to_string_lossy().into_owned(),
                format: Some(Format::Kor),
                scope: f.scope.clone(),
                event: event.to_owned(),
                matcher: s(t, "matcher"),
                command: s(t, "command"),
                timeout: t
                    .get("timeout")
                    .and_then(|v| v.as_integer())
                    .and_then(|n| u64::try_from(n).ok()),
                files,
                branch: s(t, "branch"),
                model: s(t, "model"),
                mcp: s(t, "mcp"),
                args,
                notify: s(t, "notify"),
                disabled: t.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }
    Ok(out)
}

fn claude_items(f: &HookFile) -> Result<Vec<Item>, String> {
    let v = read_json(&f.path)?;
    let mut out = Vec::new();
    for (event, groups) in v["hooks"].as_object().into_iter().flatten() {
        for (g, group) in groups.as_array().into_iter().flatten().enumerate() {
            for (h, hook) in group["hooks"].as_array().into_iter().flatten().enumerate() {
                out.push(Item {
                    id: format!("{g}.{h}"),
                    file: f.path.to_string_lossy().into_owned(),
                    format: Some(Format::Claude),
                    scope: f.scope.clone(),
                    event: event.clone(),
                    matcher: group["matcher"]
                        .as_str()
                        .filter(|m| !m.is_empty())
                        .map(str::to_owned),
                    command: hook["command"].as_str().map(str::to_owned),
                    timeout: hook["timeout"].as_u64(),
                    ..Item::default()
                });
            }
        }
    }
    Ok(out)
}

/// Every hook in `files`; unreadable files become warnings.
pub fn list(files: &[HookFile]) -> (Vec<Item>, Vec<String>) {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    for f in files.iter().filter(|f| f.exists) {
        let r = match f.format {
            Format::Kor => kor_items(f),
            Format::Claude => claude_items(f),
        };
        match r {
            Ok(v) => items.extend(v),
            Err(e) => warnings.push(e),
        }
    }
    (items, warnings)
}

/// Problems with `it` for its file format; empty when it can be saved.
pub fn validate(it: &Item) -> Vec<String> {
    let mut e = Vec::new();
    let format = it.format.unwrap_or(Format::Kor);
    match format {
        Format::Kor if !KOR_EVENTS.contains(&it.event.as_str()) => {
            e.push(format!("unknown event `{}`", it.event))
        }
        Format::Claude if !CLAUDE_EVENTS.contains(&it.event.as_str()) => {
            e.push(format!("unknown event `{}`", it.event))
        }
        _ => {}
    }
    let command = it
        .command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let actions = usize::from(command.is_some())
        + usize::from(it.mcp.is_some())
        + usize::from(it.notify.is_some());
    if actions != 1 {
        e.push("exactly one of command, mcp or notify".into());
    }
    if format == Format::Claude
        && (it.mcp.is_some()
            || it.notify.is_some()
            || !it.files.is_empty()
            || it.branch.is_some()
            || it.model.is_some()
            || it.disabled)
    {
        e.push("Claude Code hooks are plain commands: conditions, MCP calls and notifications need hooks.toml".into());
    }
    if let Some(m) = &it.matcher
        && regex::Regex::new(&format!("^(?:{m})$")).is_err()
    {
        e.push(format!("matcher `{m}` is not a valid pattern"));
    }
    if it.timeout.is_some_and(|t| !(1..=600).contains(&t)) {
        e.push("timeout must be 1-600 seconds".into());
    }
    if it.mcp.as_deref().is_some_and(|m| {
        m.split_once('/')
            .is_none_or(|(a, b)| a.is_empty() || b.is_empty())
    }) {
        e.push("mcp must be `server/tool`".into());
    }
    if let Some(a) = &it.args
        && !serde_json::from_str::<Value>(a).is_ok_and(|v| v.is_object())
    {
        e.push("args must be a JSON object".into());
    }
    for g in it.files.iter().chain(&it.branch).chain(&it.model) {
        if globset::Glob::new(g).is_err() {
            e.push(format!("`{g}` is not a valid pattern"));
        }
    }
    if command.is_some_and(|c| c.len() > 4_000)
        || it.notify.as_deref().is_some_and(|n| n.len() > 1_000)
    {
        e.push("too long".into());
    }
    e
}

fn kor_table(it: &Item) -> toml_edit::Table {
    use toml_edit::value;
    let mut t = toml_edit::Table::new();
    if let Some(c) = it
        .command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        t["command"] = value(c);
    }
    if let Some(m) = &it.mcp {
        t["mcp"] = value(m.as_str());
    }
    if let Some(a) = it
        .args
        .as_deref()
        .and_then(|a| serde_json::from_str::<Value>(a).ok())
    {
        // JSON → TOML inline table through the toml crate.
        if let Ok(tv) = toml::Value::try_from(a)
            && let Ok(doc) = format!("x = {tv}").parse::<toml_edit::DocumentMut>()
            && let Some(item) = doc.get("x")
        {
            t["args"] = item.clone();
        }
    }
    if let Some(n) = &it.notify {
        t["notify"] = value(n.as_str());
    }
    if let Some(m) = &it.matcher {
        t["matcher"] = value(m.as_str());
    }
    if let Some(n) = it.timeout {
        t["timeout"] = value(i64::try_from(n).unwrap_or(600));
    }
    if !it.files.is_empty() {
        let mut a = toml_edit::Array::new();
        for f in &it.files {
            a.push(f.as_str());
        }
        t["files"] = value(a);
    }
    if let Some(b) = &it.branch {
        t["branch"] = value(b.as_str());
    }
    if let Some(m) = &it.model {
        t["model"] = value(m.as_str());
    }
    if it.disabled {
        t["disabled"] = value(true);
    }
    t
}

/// Add `it` to `file` (`replace = Some(id)` edits that hook; the event may change). Returns the
/// saved item as listed afterwards.
///
/// # Errors
/// Validation errors, an unreadable file, an unknown id.
pub fn save(file: &HookFile, it: &Item, replace: Option<(&str, &str)>) -> Result<(), String> {
    let mut it = it.clone();
    it.format = Some(file.format);
    let errs = validate(&it);
    if !errs.is_empty() {
        return Err(errs.join("; "));
    }
    match file.format {
        Format::Kor => {
            let mut doc = read_toml(&file.path)?;
            if let Some((event, id)) = replace {
                remove_kor(&mut doc, event, id)?;
            }
            let entry = doc
                .entry(&it.event)
                .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
            let arr = entry
                .as_array_of_tables_mut()
                .ok_or_else(|| format!("`{}` is not a list of hooks", it.event))?;
            arr.push(kor_table(&it));
            write_atomic(&file.path, &doc.to_string())
        }
        Format::Claude => {
            let mut v = read_json(&file.path)?;
            if let Some((event, id)) = replace {
                remove_claude(&mut v, event, id)?;
            }
            if !v.is_object() {
                v = json!({});
            }
            if !v["hooks"].is_object() {
                v["hooks"] = json!({});
            }
            let mut hook = json!({ "type": "command", "command": it.command.as_deref().unwrap_or_default().trim() });
            if let Some(t) = it.timeout {
                hook["timeout"] = json!(t);
            }
            let groups = v["hooks"]
                .as_object_mut()
                .expect("object")
                .entry(it.event.clone())
                .or_insert_with(|| json!([]));
            if !groups.is_array() {
                *groups = json!([]);
            }
            let groups = groups.as_array_mut().expect("array");
            let matcher = it.matcher.clone().unwrap_or_default();
            match groups
                .iter_mut()
                .find(|g| g["matcher"].as_str().unwrap_or("") == matcher)
            {
                Some(g) if g["hooks"].is_array() => {
                    g["hooks"].as_array_mut().expect("array").push(hook)
                }
                _ => groups.push(json!({ "matcher": matcher, "hooks": [hook] })),
            }
            write_atomic(
                &file.path,
                &(serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n"),
            )
        }
    }
}

fn remove_kor(doc: &mut toml_edit::DocumentMut, event: &str, id: &str) -> Result<(), String> {
    let i: usize = id
        .parse()
        .map_err(|_| format!("unknown hook `{event}:{id}`"))?;
    let arr = doc
        .get_mut(event)
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .filter(|a| i < a.len())
        .ok_or_else(|| format!("unknown hook `{event}:{id}`"))?;
    arr.remove(i);
    if arr.is_empty() {
        doc.remove(event);
    }
    Ok(())
}

fn remove_claude(v: &mut Value, event: &str, id: &str) -> Result<(), String> {
    let bad = || format!("unknown hook `{event}:{id}`");
    let (g, h) = id.split_once('.').ok_or_else(bad)?;
    let (g, h): (usize, usize) = (g.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    let groups = v["hooks"][event].as_array_mut().ok_or_else(bad)?;
    let hooks = groups
        .get_mut(g)
        .and_then(|x| x["hooks"].as_array_mut())
        .filter(|a| h < a.len())
        .ok_or_else(bad)?;
    hooks.remove(h);
    if hooks.is_empty() {
        groups.remove(g);
    }
    if groups.is_empty()
        && let Some(o) = v["hooks"].as_object_mut()
    {
        o.remove(event);
    }
    Ok(())
}

/// Remove hook `id` of `event` from `file`.
///
/// # Errors
/// Unknown hook or unreadable file.
pub fn remove(file: &HookFile, event: &str, id: &str) -> Result<(), String> {
    match file.format {
        Format::Kor => {
            let mut doc = read_toml(&file.path)?;
            remove_kor(&mut doc, event, id)?;
            write_atomic(&file.path, &doc.to_string())
        }
        Format::Claude => {
            let mut v = read_json(&file.path)?;
            remove_claude(&mut v, event, id)?;
            write_atomic(
                &file.path,
                &(serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n"),
            )
        }
    }
}

/// A sample moment to dry-run a hook against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sample {
    pub tool: Option<String>,
    pub file: Option<String>,
    pub model: Option<String>,
    /// Branch to assume; the work tree's own when `None`.
    pub branch: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRun {
    /// The hook would run at this moment.
    pub matches: bool,
    /// Why it would not run.
    pub skipped: Vec<String>,
    /// Kor events the Claude event stands for at this moment.
    pub kor_events: Vec<String>,
    /// With `execute`: what the hook returned.
    pub executed: bool,
    pub blocked: Option<String>,
    pub context: Vec<String>,
    pub notices: Vec<String>,
    pub ms: u64,
}

fn hook_of(it: &Item) -> Hook {
    let kor = KorHook {
        files: it.files.clone(),
        branch: it.branch.clone(),
        model: it.model.clone(),
        mcp: it.mcp.clone(),
        args: it.args.clone(),
        notify: it.notify.clone(),
        disabled: it.disabled,
    };
    Hook {
        event: it.event.clone(),
        matcher: it.matcher.clone(),
        command: it
            .command
            .clone()
            .or_else(|| it.mcp.as_ref().map(|m| format!("mcp:{m}")))
            .or_else(|| it.notify.as_ref().map(|n| format!("notify:{n}")))
            .unwrap_or_default(),
        timeout_secs: it.timeout,
        source: Source {
            eco: if it.format == Some(Format::Claude) {
                Eco::Claude
            } else {
                Eco::Kor
            },
            scope: Scope::Project,
            path: PathBuf::from(&it.file),
        },
        kor: (kor != KorHook::default()).then_some(kor),
    }
}

/// Would `it` run at `sample`, and (with `execute`) what does it return. Running executes the
/// hook's real command or MCP call with a sample payload.
pub async fn dry_run(
    root: &Path,
    shell: kor_tools::Shell,
    it: &Item,
    sample: &Sample,
    execute: bool,
) -> DryRun {
    let mut d = DryRun::default();
    let h = hook_of(it);
    let tool = sample.tool.as_deref();
    let mut ti = json!({});
    if let Some(f) = &sample.file {
        ti["file_path"] = json!(root.join(f).to_string_lossy());
    }
    let fields = json!({ "tool_name": tool, "tool_input": ti, "prompt": sample.prompt });
    let input = json!({ "tool_input": ti });
    if it.disabled {
        d.skipped.push("disabled".into());
    }
    if let Some(m) = &it.matcher
        && let Some(t) = tool
        && !regex::Regex::new(&format!("^(?:{m})$")).is_ok_and(|r| r.is_match(t))
    {
        d.skipped.push(format!("matcher `{m}` does not match {t}"));
    }
    if let Some(k) = &h.kor {
        let branch = sample.branch.clone().or_else(|| {
            kor_agent::gitflow::git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
                .ok()
                .map(|b| b.trim().to_owned())
        });
        if !k.disabled
            && !kor_agent::hooks::conditions_met(
                k,
                root,
                &input,
                branch.as_deref(),
                sample.model.as_deref(),
            )
        {
            d.skipped
                .push("files / branch / model condition not met".into());
        }
    }
    if CLAUDE_EVENTS.contains(&it.event.as_str()) {
        d.kor_events = kor_agent::hooks::kor_events(&it.event, tool, &fields)
            .into_iter()
            .map(str::to_owned)
            .collect();
    }
    d.matches = d.skipped.is_empty();
    if !(execute && d.matches) {
        return d;
    }
    let runner = HookRunner {
        hooks: vec![h],
        shell,
        root: root.to_path_buf(),
        session_id: "dry-run".into(),
        transcript: None,
        model: sample.model.clone(),
    };
    let started = std::time::Instant::now();
    let o = tokio::time::timeout(
        Duration::from_secs(it.timeout.unwrap_or(60).clamp(1, 600) + 5),
        runner.run(&it.event, tool, fields),
    )
    .await;
    d.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    d.executed = true;
    match o {
        Ok(o) => {
            d.blocked = o.block;
            d.context = o.context;
            d.notices = o.notices;
        }
        Err(_) => d.notices.push("timed out".into()),
    }
    d
}

/// The last `limit` hook runs, newest first.
pub fn log(limit: usize) -> Vec<Value> {
    let Some(p) = kor_agent::hooks::log_path() else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(p).unwrap_or_default();
    let mut v: Vec<Value> = text
        .lines()
        .rev()
        .take(limit.clamp(1, 1000))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    v.shrink_to_fit();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(event: &str, command: &str) -> Item {
        Item {
            event: event.into(),
            command: Some(command.into()),
            ..Item::default()
        }
    }

    #[test]
    fn kor_file_round_trip_keeps_comments() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join(".kor")).unwrap();
        std::fs::write(
            root.join(".kor/hooks.toml"),
            "# team hooks\n[[pre-commit]]\ncommand = \"cargo fmt --check\"\n",
        )
        .unwrap();
        let fs = files(root, None, None);
        let kor = fs
            .iter()
            .find(|f| f.format == Format::Kor && f.scope == "project")
            .unwrap();
        let it = Item {
            files: vec!["src/**/*.rs".into()],
            branch: Some("main".into()),
            timeout: Some(30),
            ..item("post-edit", "cargo clippy")
        };
        save(kor, &it, None).unwrap();
        let mcp = Item {
            mcp: Some("lint/check".into()),
            args: Some("{\"strict\":true}".into()),
            ..item("on-error", "")
        };
        let mcp = Item {
            command: None,
            ..mcp
        };
        save(kor, &mcp, None).unwrap();
        let text = std::fs::read_to_string(&kor.path).unwrap();
        assert!(text.starts_with("# team hooks"), "{text}");
        assert!(
            text.contains("files = [\"src/**/*.rs\"]") && text.contains("strict = true"),
            "{text}"
        );
        let (items, w) = list(&fs);
        assert!(w.is_empty());
        assert_eq!(items.len(), 3);
        let edited = items.iter().find(|i| i.event == "post-edit").unwrap();
        assert_eq!(edited.branch.as_deref(), Some("main"));
        let m = items.iter().find(|i| i.event == "on-error").unwrap();
        assert_eq!(m.args.as_deref(), Some("{\"strict\":true}"));
        // The file is still what kor-agent parses.
        let mut warns = Vec::new();
        let parsed = kor_agent::hooks::parse_toml(
            &text,
            &Source {
                eco: Eco::Kor,
                scope: Scope::Project,
                path: kor.path.clone(),
            },
            &mut warns,
        );
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(parsed.len(), 3);
        assert_eq!(
            parsed
                .iter()
                .find(|h| h.event == "post-edit")
                .unwrap()
                .kor
                .as_ref()
                .unwrap()
                .files,
            vec!["src/**/*.rs".to_owned()]
        );
        // Edit (event changes) and remove.
        save(
            kor,
            &Item {
                disabled: true,
                ..item("pre-edit", "true")
            },
            Some(("post-edit", "0")),
        )
        .unwrap();
        let (items, _) = list(&fs);
        assert!(items.iter().any(|i| i.event == "pre-edit" && i.disabled));
        assert!(!items.iter().any(|i| i.event == "post-edit"));
        remove(kor, "pre-commit", "0").unwrap();
        assert!(
            !std::fs::read_to_string(&kor.path)
                .unwrap()
                .contains("pre-commit")
        );
        assert!(remove(kor, "pre-commit", "0").is_err());
    }

    #[test]
    fn claude_file_keeps_other_keys_and_refuses_kor_extras() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join(".claude/settings.json"), "{\"model\": \"opus\", \"hooks\": {\"Stop\": [{\"matcher\": \"\", \"hooks\": [{\"type\": \"command\", \"command\": \"say done\"}]}]}}").unwrap();
        let fs = files(root, None, None);
        let c = fs
            .iter()
            .find(|f| f.format == Format::Claude && f.scope == "project")
            .unwrap();
        save(
            c,
            &Item {
                matcher: Some("Edit|Write".into()),
                ..item("PostToolUse", "npm run lint")
            },
            None,
        )
        .unwrap();
        assert!(
            save(
                c,
                &Item {
                    files: vec!["*.ts".into()],
                    ..item("PostToolUse", "x")
                },
                None
            )
            .is_err()
        );
        assert!(save(c, &item("pre-edit", "x"), None).is_err());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&c.path).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "Edit|Write");
        let (items, _) = list(&fs);
        assert_eq!(items.len(), 2);
        remove(c, "Stop", "0.0").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&c.path).unwrap()).unwrap();
        assert!(v["hooks"].get("Stop").is_none());
    }

    #[test]
    fn validation() {
        assert!(validate(&item("pre-edit", "x")).is_empty());
        assert!(!validate(&item("pre-edit", " ")).is_empty());
        assert!(
            !validate(&Item {
                notify: Some("hi".into()),
                ..item("pre-edit", "x")
            })
            .is_empty()
        );
        assert!(
            !validate(&Item {
                matcher: Some("(".into()),
                ..item("pre-edit", "x")
            })
            .is_empty()
        );
        assert!(
            !validate(&Item {
                mcp: Some("nope".into()),
                command: None,
                ..item("pre-edit", "")
            })
            .is_empty()
        );
        assert!(
            !validate(&Item {
                timeout: Some(0),
                ..item("pre-edit", "x")
            })
            .is_empty()
        );
        assert!(
            !validate(&Item {
                args: Some("[1]".into()),
                ..item("pre-edit", "x")
            })
            .is_empty()
        );
    }

    #[tokio::test]
    async fn dry_run_conditions_and_execution() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let it = Item {
            files: vec!["src/**".into()],
            model: Some("anthropic/*".into()),
            ..item("pre-edit", "echo checked")
        };
        let sample = Sample {
            tool: Some("Edit".into()),
            file: Some("src/a.rs".into()),
            model: Some("anthropic/claude".into()),
            ..Sample::default()
        };
        let r = dry_run(&root, kor_tools::Shell::detect(), &it, &sample, false).await;
        assert!(r.matches && !r.executed);
        let other = Sample {
            file: Some("docs/a.md".into()),
            ..sample.clone()
        };
        assert!(
            !dry_run(&root, kor_tools::Shell::detect(), &it, &other, false)
                .await
                .matches
        );
        let n = Item {
            command: None,
            notify: Some("Edit coming".into()),
            ..it.clone()
        };
        let r = dry_run(&root, kor_tools::Shell::detect(), &n, &sample, true).await;
        assert!(r.executed);
        assert_eq!(r.notices, vec!["Edit coming".to_owned()]);
        let c = dry_run(
            &root,
            kor_tools::Shell::detect(),
            &item("PreToolUse", "true"),
            &Sample {
                tool: Some("Edit".into()),
                ..Sample::default()
            },
            false,
        )
        .await;
        assert_eq!(c.kor_events, vec!["pre-edit".to_owned()]);
    }
}
