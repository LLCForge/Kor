//! Editing MCP server configuration, shared by `kor mcp add/remove/enable/disable` and the
//! desktop's MCP screen.
//!
//! Kor writes only its own file (`<config>/kor/mcp.json`). Servers from `~/.claude.json` and a
//! trusted project's `.mcp.json` are listed but never written: turning one off or removing it
//! writes a `{"disabled": true}` stub into Kor's file, and editing one writes a full copy into
//! Kor's file that shadows it (see [`crate::mcp_tools::discover`] for the load order).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

const MAX_JSON_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_SERVERS: usize = 50;

/// Where configuration lives for one project.
#[derive(Debug, Clone)]
pub struct Locations {
    /// Kor's own `mcp.json`.
    pub kor_file: PathBuf,
    pub home: Option<PathBuf>,
    pub root: PathBuf,
    /// The project's `.mcp.json` is read only when the project is trusted.
    pub trusted: bool,
}

/// A configured server and where it came from (`None`: Kor's file).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub raw: Value,
    pub external: Option<&'static str>,
}

/// A JSON object file; empty when missing. A damaged file is an error, never overwritten.
///
/// # Errors
/// If the file is too large, unreadable or not a JSON object.
pub fn read_object(path: &Path) -> Result<Map<String, Value>, String> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(Map::new());
    };
    if meta.len() > MAX_JSON_BYTES {
        return Err(format!("{} çok büyük", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(o)) => Ok(o),
        Ok(_) => Err(format!(
            "{} bozuk, düzeltin ya da silin: {}",
            path.display(),
            "nesne değil"
        )),
        Err(e) => Err(format!(
            "{} bozuk, düzeltin ya da silin: {}",
            path.display(),
            e
        )),
    }
}

/// Write via a temporary file and a rename, so a crash never leaves half a file.
///
/// # Errors
/// On any I/O error.
pub fn write_atomic(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// `^[A-Za-z0-9][A-Za-z0-9_-]*$`, at most `max` bytes.
pub fn valid_id(id: &str, max: usize) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.len() <= max
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn servers_of(file: &Map<String, Value>) -> Map<String, Value> {
    file.get("mcpServers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

pub fn is_http(raw: &Value) -> bool {
    raw.get("url").is_some()
        || matches!(
            raw.get("type").and_then(Value::as_str),
            Some("http" | "sse" | "streamable-http")
        )
}

/// A `{"disabled": true}` entry without a command or URL: switches off a server defined elsewhere.
pub fn is_stub(raw: &Value) -> bool {
    raw.get("command").is_none() && !is_http(raw)
}

pub fn disabled(raw: &Value) -> bool {
    raw.get("disabled").and_then(Value::as_bool) == Some(true)
}

/// `~/.claude.json` keys projects by the path Claude Code wrote; compare loosely.
fn same_path(a: &str, b: &Path) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let b = b.to_string_lossy();
    norm(a) == norm(b.strip_prefix(r"\\?\").unwrap_or(&b))
}

/// MCP servers other tools define for the user: Cursor, Windsurf, Claude Desktop, VS Code. Read
/// only, never written; they load first, so Claude Code, Kor and the project shadow them. VS Code
/// entries that prompt for `${input:…}` values are left out (only VS Code can answer them).
/// `app_config` is the OS config folder (`%APPDATA%`, `~/Library/Application Support`, `~/.config`).
pub fn other_tools(
    home: &Path,
    app_config: Option<&Path>,
) -> Vec<(&'static str, Map<String, Value>)> {
    let mut files: Vec<(&'static str, PathBuf, &str)> = vec![
        (
            "Cursor",
            home.join(".cursor").join("mcp.json"),
            "mcpServers",
        ),
        (
            "Windsurf",
            home.join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            "mcpServers",
        ),
    ];
    if let Some(c) = app_config {
        files.push((
            "Claude Desktop",
            c.join("Claude").join("claude_desktop_config.json"),
            "mcpServers",
        ));
        files.push((
            "VS Code",
            c.join("Code").join("User").join("mcp.json"),
            "servers",
        ));
    }
    files
        .into_iter()
        .filter_map(|(label, path, key)| {
            let file = read_object(&path).ok()?;
            let servers: Map<String, Value> = file
                .get(key)?
                .as_object()?
                .iter()
                .filter(|(id, raw)| valid_id(id, 64) && !raw.to_string().contains("${input:"))
                .map(|(id, raw)| (id.clone(), raw.clone()))
                .collect();
            (!servers.is_empty()).then_some((label, servers))
        })
        .collect()
}

/// [`other_tools`] for the real user only: a test or sandbox with its own `home` sees none.
pub fn other_tools_for(home: &Path) -> Vec<(&'static str, Map<String, Value>)> {
    if dirs::home_dir().as_deref() != Some(home) {
        return Vec::new();
    }
    other_tools(home, dirs::config_dir().as_deref())
}

/// Every configured server in the order the agent loads them (later ones shadow earlier ones).
///
/// # Errors
/// If Kor's own file is damaged.
pub fn entries(loc: &Locations) -> Result<Vec<Entry>, String> {
    let mut out: BTreeMap<String, Entry> = BTreeMap::new();
    let put = |servers: &Map<String, Value>,
               origin: Option<&'static str>,
               out: &mut BTreeMap<String, Entry>| {
        for (id, raw) in servers {
            if origin.is_some() && is_stub(raw) {
                continue;
            }
            out.insert(
                id.clone(),
                Entry {
                    id: id.clone(),
                    raw: raw.clone(),
                    external: origin,
                },
            );
        }
    };
    if let Some(home) = &loc.home {
        for (label, servers) in other_tools_for(home) {
            put(&servers, Some(label), &mut out);
        }
    }
    if let Some(home) = &loc.home
        && let Ok(v) = read_object(&home.join(".claude.json"))
    {
        put(&servers_of(&v), Some("~/.claude.json"), &mut out);
        if let Some(projects) = v.get("projects").and_then(Value::as_object) {
            for (key, p) in projects {
                if same_path(key, &loc.root)
                    && let Some(o) = p.get("mcpServers").and_then(Value::as_object)
                {
                    put(o, Some("~/.claude.json"), &mut out);
                }
            }
        }
    }
    for (id, raw) in servers_of(&read_object(&loc.kor_file)?) {
        if is_stub(&raw) {
            if let Some(e) = out.get_mut(&id)
                && disabled(&raw)
                && let Some(o) = e.raw.as_object_mut()
            {
                o.insert("disabled".into(), Value::Bool(true));
            }
            continue;
        }
        out.insert(
            id.clone(),
            Entry {
                id,
                raw,
                external: None,
            },
        );
    }
    if loc.trusted
        && let Ok(v) = read_object(&loc.root.join(".mcp.json"))
    {
        put(&servers_of(&v), Some(".mcp.json"), &mut out);
    }
    Ok(out.into_values().collect())
}

pub fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn string_map(v: Option<&Value>) -> Map<String, Value> {
    v.and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter(|(_, v)| v.is_string())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The desktop's `McpServerConfig` shape for an entry.
pub fn config_of(e: &Entry) -> Value {
    let raw = &e.raw;
    let mut c = Map::new();
    c.insert("id".into(), json!(e.id));
    let label = raw.get("label").and_then(Value::as_str).map(str::to_owned);
    let label = match (label, e.external) {
        (Some(l), _) => Some(l),
        (None, Some(origin)) => Some(format!("{} ({origin})", e.id)),
        (None, None) => None,
    };
    if let Some(l) = label {
        c.insert("label".into(), json!(l));
    }
    c.insert("enabled".into(), json!(!disabled(raw)));
    if is_http(raw) {
        c.insert("transport".into(), json!("http"));
        c.insert(
            "url".into(),
            json!(raw.get("url").and_then(Value::as_str).unwrap_or("")),
        );
        c.insert(
            "headers".into(),
            Value::Object(string_map(raw.get("headers"))),
        );
    } else {
        c.insert("transport".into(), json!("stdio"));
        c.insert(
            "command".into(),
            json!(raw.get("command").and_then(Value::as_str).unwrap_or("")),
        );
        c.insert("args".into(), json!(strings(raw.get("args"))));
        c.insert("env".into(), Value::Object(string_map(raw.get("env"))));
    }
    Value::Object(c)
}

fn bounded_map(v: Option<&Value>, what: &str) -> Result<Map<String, Value>, String> {
    let Some(v) = v else { return Ok(Map::new()) };
    let o = v
        .as_object()
        .ok_or_else(|| format!("{what} bir nesne olmalı"))?;
    if o.len() > 100 {
        return Err(format!("{what}: en fazla 100 girdi"));
    }
    for (k, v) in o {
        let ok = !k.is_empty() && k.len() <= 100 && v.as_str().is_some_and(|s| s.len() <= 8000);
        if !ok {
            return Err(format!("{what}: geçersiz girdi `{k}`"));
        }
    }
    Ok(o.clone())
}

/// Validate a `McpServerConfig` and turn it into an `mcpServers` entry. Unknown fields of the
/// entry it replaces (`timeout`…) are kept.
///
/// # Errors
/// On an invalid id, label, command, argument list, URL or env/header map.
pub fn raw_from_config(cfg: &Value, previous: Option<&Value>) -> Result<(String, Value), String> {
    let id = cfg.get("id").and_then(Value::as_str).unwrap_or_default();
    if !valid_id(id, 60) {
        return Err(
            "Sunucu kimliği harf/rakamla başlamalı; yalnızca harf, rakam, _ ve - içerebilir".into(),
        );
    }
    let mut raw = previous
        .filter(|p| !is_stub(p))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for k in [
        "command", "args", "env", "url", "headers", "type", "label", "disabled",
    ] {
        raw.remove(k);
    }
    match cfg.get("label").and_then(Value::as_str).map(str::trim) {
        Some(l) if l.chars().count() > 80 => {
            return Err("Etiket en fazla 80 karakter olabilir".into());
        }
        Some(l) if !l.is_empty() => {
            raw.insert("label".into(), json!(l));
        }
        _ => {}
    }
    match cfg.get("transport").and_then(Value::as_str) {
        Some("stdio") => {
            let command = cfg
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if command.is_empty() || command.len() > 500 {
                return Err("Komut 1-500 karakter olmalı".into());
            }
            let args = cfg
                .get("args")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if args.len() > 100
                || !args
                    .iter()
                    .all(|a| a.as_str().is_some_and(|s| s.len() <= 4000))
            {
                return Err("Geçersiz argüman listesi".into());
            }
            raw.insert("command".into(), json!(command));
            raw.insert("args".into(), Value::Array(args));
            raw.insert(
                "env".into(),
                Value::Object(bounded_map(cfg.get("env"), "env")?),
            );
        }
        Some("http") => {
            let url = cfg
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let ok = url.len() <= 2048
                && reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
            if !ok {
                return Err("Geçersiz adres".into());
            }
            raw.insert("type".into(), json!("http"));
            raw.insert("url".into(), json!(url));
            raw.insert(
                "headers".into(),
                Value::Object(bounded_map(cfg.get("headers"), "headers")?),
            );
        }
        _ => return Err("Bağlantı türü stdio ya da http olmalı".into()),
    }
    if cfg.get("enabled").and_then(Value::as_bool) != Some(true) {
        raw.insert("disabled".into(), Value::Bool(true));
    }
    Ok((id.to_owned(), Value::Object(raw)))
}

/// Read Kor's file, change its `mcpServers`, write it back atomically (other keys are kept).
///
/// # Errors
/// If the file is damaged, `f` fails, or there would be more than [`MAX_SERVERS`].
pub fn edit_servers(
    kor_file: &Path,
    f: impl FnOnce(&mut Map<String, Value>) -> Result<(), String>,
) -> Result<(), String> {
    let mut file = read_object(kor_file)?;
    let mut servers = servers_of(&file);
    f(&mut servers)?;
    if servers.len() > MAX_SERVERS {
        return Err(format!("En fazla {MAX_SERVERS} MCP sunucusu olabilir"));
    }
    file.insert("mcpServers".into(), Value::Object(servers));
    write_atomic(kor_file, &Value::Object(file))
}

/// Add or replace a server from a `McpServerConfig`.
///
/// # Errors
/// See [`raw_from_config`] and [`edit_servers`].
pub fn save(loc: &Locations, cfg: &Value) -> Result<(), String> {
    let id = cfg
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // Keep unknown fields (timeout…) of a shadowed external server.
    let external = entries(loc)?
        .into_iter()
        .find(|e| e.id == id)
        .map(|e| e.raw);
    edit_servers(&loc.kor_file, |servers| {
        // A `{"disabled": true}` stub is not a previous definition: fall back to the external one.
        let previous = servers
            .get(&id)
            .filter(|p| !is_stub(p))
            .cloned()
            .or(external);
        let (id, raw) = raw_from_config(cfg, previous.as_ref())?;
        servers.insert(id, raw);
        Ok(())
    })
}

/// Remove a server. One defined by another tool is switched off in Kor's file instead.
///
/// # Errors
/// If the server is unknown or the file cannot be written.
pub fn remove(loc: &Locations, id: &str) -> Result<(), String> {
    let entries = entries(loc)?;
    let entry = entries
        .iter()
        .find(|e| e.id == id)
        .ok_or("Sunucu bulunamadı")?;
    let external = entry.external.is_some();
    let generated = !external && entry.raw.get("kor").is_some();
    edit_servers(&loc.kor_file, |servers| {
        servers.remove(id);
        if external {
            servers.insert(id.to_owned(), json!({ "disabled": true }));
        }
        Ok(())
    })?;
    // A `kor mcp add <source>` server also leaves its definition (`mcp-gen/<id>.json`).
    if generated && let Some(p) = crate::mcp_gen::definition_path(&loc.kor_file, id) {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

/// Turn a server on or off, wherever it is defined.
///
/// # Errors
/// If the server is unknown or the file cannot be written.
pub fn set_enabled(loc: &Locations, id: &str, enabled: bool) -> Result<(), String> {
    if !entries(loc)?.iter().any(|e| e.id == id) {
        return Err("Sunucu bulunamadı".into());
    }
    edit_servers(&loc.kor_file, |servers| {
        match servers.get_mut(id) {
            Some(raw) if is_stub(raw) => {
                if enabled {
                    servers.remove(id);
                }
            }
            Some(Value::Object(o)) => {
                if enabled {
                    o.remove("disabled");
                } else {
                    o.insert("disabled".into(), Value::Bool(true));
                }
            }
            _ => {
                if !enabled {
                    servers.insert(id.to_owned(), json!({ "disabled": true }));
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_other_tools_servers_without_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let (home, cfg) = (dir.path().join("h"), dir.path().join("c"));
        let w = |p: PathBuf, t: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            home.join(".cursor/mcp.json"),
            r#"{"mcpServers":{"gh":{"command":"gh-mcp"}}}"#,
        );
        w(
            cfg.join("Code/User/mcp.json"),
            r#"{"servers":{"db":{"type":"stdio","command":"db-mcp"},"ask":{"command":"x","env":{"K":"${input:key}"}}}}"#,
        );
        w(cfg.join("Claude/claude_desktop_config.json"), "not json");
        let got = other_tools(&home, Some(&cfg));
        let names: Vec<(&str, Vec<&String>)> =
            got.iter().map(|(l, s)| (*l, s.keys().collect())).collect();
        assert_eq!(
            names,
            [
                ("Cursor", vec![&"gh".to_owned()]),
                ("VS Code", vec![&"db".to_owned()])
            ]
        );
        assert!(
            other_tools_for(&home).is_empty(),
            "a test home never sees the real user's tools"
        );
    }

    fn loc(dir: &Path) -> Locations {
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"ext":{"command":"ext-mcp","timeout":9}}}"#,
        )
        .unwrap();
        Locations {
            kor_file: dir.join("cfg").join("mcp.json"),
            home: Some(home),
            root: dir.join("proj"),
            trusted: false,
        }
    }

    #[test]
    fn edits_only_kor_file_and_stubs_external_servers() {
        let dir = tempfile::tempdir().unwrap();
        let l = loc(dir.path());
        let cfg = json!({"transport":"stdio","id":"docs","enabled":true,"command":"npx","args":["-y","d"],"env":{}});
        save(&l, &cfg).unwrap();
        set_enabled(&l, "ext", false).unwrap();
        let all = entries(&l).unwrap();
        assert_eq!(all.len(), 2);
        assert!(disabled(&all.iter().find(|e| e.id == "ext").unwrap().raw));
        // Editing the external server keeps its unknown fields in Kor's shadow copy.
        save(
            &l,
            &json!({"transport":"stdio","id":"ext","enabled":true,"command":"ext2"}),
        )
        .unwrap();
        let ext = entries(&l)
            .unwrap()
            .into_iter()
            .find(|e| e.id == "ext")
            .unwrap();
        assert_eq!((ext.external, ext.raw["timeout"].as_i64()), (None, Some(9)));
        remove(&l, "docs").unwrap();
        assert!(remove(&l, "nope").is_err());
        // Claude Code's file is untouched.
        let claude = std::fs::read_to_string(dir.path().join("home/.claude.json")).unwrap();
        assert!(claude.contains("ext-mcp") && !claude.contains("disabled"));
    }
}
