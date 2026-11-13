//! Plugin marketplaces. Cache layout:
//! `$CLAUDE_CODE_PLUGIN_CACHE_DIR` or `~/.claude/plugins`, with clones under
//! `marketplaces/` and installs under `cache/<marketplace>/<plugin>`.
//! Manifests are `marketplace.json` and `.claude-plugin/plugin.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marketplace {
    pub name: String,
    pub path: PathBuf,
    pub plugins: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MarketFile {
    plugins: Option<Vec<PluginEntry>>,
}

#[derive(Debug, Deserialize)]
struct PluginEntry {
    name: Option<String>,
    source: Option<String>,
}

pub fn plugins_root(home: &Path) -> PathBuf {
    if let Some(over) = crate::env_compat("KOR_PLUGIN_CACHE_DIR", "CLAUDE_CODE_PLUGIN_CACHE_DIR") {
        if let Some(rest) = over.strip_prefix("~/") {
            return home.join(rest);
        }
        return PathBuf::from(over);
    }
    // Kor installs into its own directory and never writes into another tool's.
    home.join(".kor").join("plugins")
}

/// Plugin metadata directory: `.kor-plugin/`, or Claude Code's `.claude-plugin/`.
pub fn meta_dir(dir: &Path) -> PathBuf {
    let kor = dir.join(".kor-plugin");
    if kor.is_dir() {
        kor
    } else {
        dir.join(".claude-plugin")
    }
}

fn market_dir(root: &Path, name: &str) -> PathBuf {
    root.join("marketplaces").join(name)
}

fn install_dir(root: &Path, market: &str, plugin: &str) -> PathBuf {
    root.join("cache").join(market).join(plugin)
}

pub(crate) fn safe(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', '\0']) && !name.contains("..") && name.len() < 80
}

/// Clone a git URL or copy a local directory into the marketplace cache.
///
/// # Errors
/// If the name is unsafe, git fails, or the manifest is missing.
pub fn add_marketplace(root: &Path, name: &str, source: &str) -> Result<PathBuf, String> {
    if !safe(name) {
        return Err("invalid marketplace name".into());
    }
    let dest = market_dir(root, name);
    if dest.exists() {
        return Err("marketplace already exists".into());
    }
    fs::create_dir_all(dest.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    let src = Path::new(source);
    if src.is_dir() {
        copy_dir(src, &dest)?;
    } else {
        // Privacy/offline mode: a remote clone is refused like any other outbound call.
        kor_core::net::check_url(source).map_err(|b| b.message)?;
        let status = Command::new("git")
            .args(["clone", "--depth", "1", source])
            .arg(&dest)
            .status()
            .map_err(|e| format!("git: {e}"))?;
        if !status.success() {
            return Err(format!("git clone failed ({status})"));
        }
    }
    if manifest(&dest).is_none() {
        let _ = fs::remove_dir_all(&dest);
        return Err("no marketplace.json".into());
    }
    Ok(dest)
}

fn manifest(dir: &Path) -> Option<PathBuf> {
    let a = dir.join("marketplace.json");
    if a.is_file() {
        return Some(a);
    }
    let b = meta_dir(dir).join("marketplace.json");
    b.is_file().then_some(b)
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for ent in fs::read_dir(from).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let name = ent.file_name();
        if name == ".git" {
            continue;
        }
        let dest = to.join(&name);
        if ent.path().is_dir() {
            copy_dir(&ent.path(), &dest)?;
        } else {
            fs::copy(ent.path(), dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn list_marketplaces(root: &Path) -> Vec<Marketplace> {
    let dir = root.join("marketplaces");
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        if !ent.path().is_dir() {
            continue;
        }
        let Some(path) = manifest(&ent.path()) else {
            continue;
        };
        let name = ent.file_name().to_string_lossy().into_owned();
        let plugins = read_plugins(&path);
        out.push(Marketplace {
            name,
            path: ent.path(),
            plugins,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn read_plugins(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_str::<MarketFile>(&text) else {
        return Vec::new();
    };
    file.plugins
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.name.or(p.source))
        .collect()
}

/// Copy one plugin from a marketplace into the install cache.
///
/// # Errors
/// If the marketplace or plugin directory is missing.
pub fn install(root: &Path, market: &str, plugin: &str) -> Result<PathBuf, String> {
    if !safe(market) || !safe(plugin) {
        return Err("invalid name".into());
    }
    let src = market_dir(root, market).join(plugin);
    let plugin_json = meta_dir(&src).join("plugin.json");
    if !src.is_dir() || !plugin_json.is_file() {
        return Err(
            "plugin directory has no .kor-plugin/plugin.json or .claude-plugin/plugin.json".into(),
        );
    }
    let dest = install_dir(root, market, plugin);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    copy_dir(&src, &dest)?;
    Ok(dest)
}

pub fn remove(root: &Path, market: &str, plugin: &str) -> Result<(), String> {
    if !safe(market) || !safe(plugin) {
        return Err("invalid name".into());
    }
    let dest = install_dir(root, market, plugin);
    if !dest.exists() {
        return Err("plugin is not installed".into());
    }
    fs::remove_dir_all(dest).map_err(|e| e.to_string())
}

pub fn remove_marketplace(root: &Path, name: &str) -> Result<(), String> {
    if !safe(name) {
        return Err("invalid marketplace name".into());
    }
    let dest = market_dir(root, name);
    if !dest.exists() {
        return Err("marketplace is not added".into());
    }
    fs::remove_dir_all(dest).map_err(|e| e.to_string())
}

fn enabled_path(root: &Path) -> PathBuf {
    root.join("enabled.json")
}

/// `enabledPlugins` flag file. Missing entries are enabled.
pub fn set_enabled(root: &Path, id: &str, enabled: bool) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = enabled_path(root);
    let mut map: serde_json::Map<String, serde_json::Value> = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    map.insert(id.to_owned(), serde_json::Value::Bool(enabled));
    fs::write(
        &path,
        serde_json::to_string(&map).unwrap_or_else(|_| "{}".into()),
    )
    .map_err(|e| e.to_string())
}

pub fn is_enabled(root: &Path, id: &str) -> bool {
    let Ok(text) = fs::read_to_string(enabled_path(root)) else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get(id).and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// Installed plugins as `(id, dir)`: direct installs `<root>/<name>` (id `name`, the desktop's
/// git installs) and marketplace installs `cache/<market>/<plugin>` (id `plugin@market`).
pub fn installed(root: &Path) -> Vec<(String, PathBuf)> {
    let subdirs = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(d)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let mut out = Vec::new();
    for d in subdirs(root) {
        match name(&d).as_deref() {
            Some("marketplaces") => {}
            Some("cache") => {
                for m in subdirs(&d) {
                    for p in subdirs(&m) {
                        if let (Some(pn), Some(mn)) = (name(&p), name(&m)) {
                            out.push((format!("{pn}@{mn}"), p));
                        }
                    }
                }
            }
            Some(n) if !n.starts_with('.') => out.push((n.to_owned(), d)),
            _ => {}
        }
    }
    out
}

/// Directories of the installed plugins that are enabled.
pub fn enabled_dirs(root: &Path) -> Vec<PathBuf> {
    installed(root)
        .into_iter()
        .filter(|(id, _)| is_enabled(root, id))
        .map(|(_, d)| d)
        .collect()
}

/// First `skills/<name>/SKILL.md` in an installed plugin directory.
pub fn plugin_skill(plugin_dir: &Path) -> Option<String> {
    let rd = fs::read_dir(plugin_dir.join("skills")).ok()?;
    for ent in rd.flatten() {
        let file = ent.path().join("SKILL.md");
        if let Ok(text) = fs::read_to_string(file)
            && !text.trim().is_empty()
        {
            return Some(text);
        }
    }
    None
}

pub fn doctor(plugin_dir: &Path) -> Result<(), String> {
    let manifest = meta_dir(plugin_dir).join("plugin.json");
    if manifest.is_file() {
        Ok(())
    } else {
        Err(format!("missing {}", manifest.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_disable_toggles_the_flag_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_enabled(dir.path(), "demo@local"));
        set_enabled(dir.path(), "demo@local", false).unwrap();
        assert!(!is_enabled(dir.path(), "demo@local"));
        set_enabled(dir.path(), "demo@local", true).unwrap();
        assert!(is_enabled(dir.path(), "demo@local"));
        let plug = dir.path().join("demo");
        fs::create_dir_all(plug.join("skills").join("ship")).unwrap();
        fs::write(plug.join("skills").join("ship").join("SKILL.md"), "ship it").unwrap();
        assert_eq!(plugin_skill(&plug).as_deref(), Some("ship it"));
        assert!(doctor(&plug).is_err());
    }

    #[test]
    fn installed_lists_direct_and_marketplace_plugins_and_filters_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["ponytail", "cache/mk/demo", "marketplaces/mk", ".tmp"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let ids: Vec<String> = installed(root).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["demo@mk", "ponytail"]);
        set_enabled(root, "ponytail", false).unwrap();
        assert_eq!(enabled_dirs(root), vec![root.join("cache/mk/demo")]);
    }

    #[test]
    fn local_git_repo_installs_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        let plug = origin.join("demo");
        fs::create_dir_all(plug.join(".claude-plugin")).unwrap();
        fs::write(
            origin.join("marketplace.json"),
            r#"{"name":"local","plugins":[{"name":"demo","source":"./demo"}]}"#,
        )
        .unwrap();
        fs::write(
            plug.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"demo"}"#,
        )
        .unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&origin)
                .env("GIT_AUTHOR_NAME", "kor")
                .env("GIT_AUTHOR_EMAIL", "kor@example.com")
                .env("GIT_COMMITTER_NAME", "kor")
                .env("GIT_COMMITTER_EMAIL", "kor@example.com")
                .status()
                .unwrap();
            assert!(status.success(), "{args:?}");
        };
        git(&["init"]);
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);
        let cache = dir.path().join("plugins");
        let url = origin.display().to_string();
        add_marketplace(&cache, "local", &url).unwrap();
        let listed = list_marketplaces(&cache);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].plugins.iter().any(|p| p == "demo"));
        let installed = install(&cache, "local", "demo").unwrap();
        assert!(
            installed
                .join(".claude-plugin")
                .join("plugin.json")
                .is_file()
        );
        remove(&cache, "local", "demo").unwrap();
        assert!(!installed.exists());
        remove_marketplace(&cache, "local").unwrap();
        assert!(list_marketplaces(&cache).is_empty());
    }
}
