//! Session wiring of the PATH tool catalog and typed tool definitions (§26.2, ADR 0056).
//! The logic lives in `kor_agent::clitools`; this file decides what a session gets.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kor_agent::clitools::{self, Catalog, defs, search::ToolsSearch};
use kor_tools::{Tool, ToolCtx};

/// `<kor home>/tools`.
pub fn tools_home() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("tools"))
}

/// Where definitions are read from: the user's always, the project's only when trusted.
pub fn dirs(root: &Path, trusted: bool) -> defs::Dirs {
    defs::Dirs {
        home: tools_home(),
        project: trusted.then(|| root.join(".kor").join("tools")),
    }
}

/// `[tools] deny` as permission rules: the shell refuses those commands too.
pub fn deny_rules(cfg: &kor_config::ToolsConfig) -> Vec<String> {
    cfg.deny
        .iter()
        .filter(|n| kor_agent::clitools::help::valid_name(n))
        .flat_map(|n| [format!("Bash({n})"), format!("Bash({n} *)")])
        .collect()
}

/// Rebuild the catalog on a background thread (first run, stale cache); the session does not wait.
pub fn refresh_in_background(home: PathBuf) {
    std::thread::spawn(move || {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            rt.block_on(Catalog::load(&home, false));
        }
    });
}

/// Add the typed tools, `cli_tool`, `tool_define`, `tools_search` and the catalog prompt section.
#[allow(clippy::too_many_arguments)] // one call site, every value is a session property
pub fn attach(
    tools: &mut Vec<Arc<dyn Tool>>,
    system: &mut String,
    tctx: &ToolCtx,
    cfg: &kor_config::ToolsConfig,
    trusted: bool,
    privacy: bool,
    context_window: u64,
) -> Vec<String> {
    let Some(home) = tools_home() else {
        return Vec::new();
    };
    let permits: Arc<dyn Fn(&str) -> bool + Send + Sync> = {
        let cfg = cfg.clone();
        Arc::new(move |n: &str| cfg.permits(n))
    };
    let dirs = dirs(&tctx.root, trusted);
    let (loaded, warnings) = defs::load(&dirs);
    let mut typed = Vec::new();
    for l in loaded {
        let name = l.def.name.clone();
        if !permits(l.def.program())
            || (privacy && l.def.safety == defs::Safety::Network)
            || tools.iter().any(|t| t.name() == name)
        {
            continue;
        }
        let t = defs::DefinedTool::new(l);
        // Loaded on demand (ToolSearch), so a long list of definitions costs no context.
        if let Ok(mut d) = tctx.deferred.lock() {
            d.insert(name.clone());
        }
        if let Ok(mut about) = tctx.deferred_about.lock() {
            about.insert(name.clone(), t.spec().description);
        }
        typed.push(name);
        tools.push(Arc::new(t));
    }
    let embedder: Option<Arc<dyn kor_agent::semantic::Embedder>> =
        kor_agent::semantic::local_embedder()
            .map(|e| Arc::new(e) as Arc<dyn kor_agent::semantic::Embedder>);
    tools.push(Arc::new(ToolsSearch::new(
        home.clone(),
        dirs.clone(),
        privacy,
        permits.clone(),
        embedder,
    )));
    tools.push(Arc::new(defs::CliTool::new(dirs, permits.clone(), privacy)));
    tools.push(Arc::new(defs::ToolDefine::new(home.join("generated"))));
    if cfg.catalog {
        match Catalog::cached(&home) {
            Some(mut c) => {
                if c.stale(clitools::now_ms()) {
                    refresh_in_background(home.clone());
                }
                c.filter(|n| permits(n));
                system.push_str(&clitools::prompt_section(
                    &c,
                    &typed,
                    Some(&tctx.root),
                    clitools::budget_chars(context_window),
                    privacy,
                ));
            }
            None => refresh_in_background(home),
        }
    }
    warnings
}

/// `kor tool add`: write a definition scaffolded from a template to the project's
/// `.kor/tools/` (or the user's tools folder with `user`). Never overwrites.
///
/// # Errors
/// Invalid name/template, the file exists, or it cannot be written.
pub fn add(
    root: &Path,
    name: &str,
    template: &str,
    safety: defs::Safety,
    description: &str,
    user: bool,
) -> Result<PathBuf, String> {
    let def = defs::scaffold(name, template, safety, description)?;
    let dir = if user {
        tools_home().ok_or("no Kor home")?
    } else {
        root.join(".kor").join("tools")
    };
    let file = dir.join(format!("{name}.toml"));
    if file.exists() {
        return Err(format!("{} already exists", file.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(
        &file,
        toml::to_string_pretty(&def).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(file)
}

/// Team server address and token (`[team] server`, keychain `team:token`).
fn team(cfg: &kor_config::Config) -> Result<(String, String), String> {
    use kor_secrets::SecretStore as _;
    let server = cfg.team.server.trim().to_owned();
    if server.is_empty() {
        return Err(
            "no team server: set `[team] server` (kor config set team.server <url>)".into(),
        );
    }
    let token = kor_secrets::KeychainStore
        .get("team:token")
        .map_err(|e| e.to_string())?
        .ok_or("no team token in the keychain (desktop: Settings → Team)")?;
    Ok((
        server,
        kor_secrets::ExposeSecret::expose_secret(&token).to_owned(),
    ))
}

/// `kor tool share`: the user's and the (trusted) project's definitions to the team server.
///
/// # Errors
/// Not connected, a secret in a definition, or the request failed.
pub async fn share(cfg: &kor_config::Config, root: &Path, trusted: bool) -> Result<usize, String> {
    let (server, token) = team(cfg)?;
    let (loaded, _) = defs::load(&dirs(root, trusted));
    let mine: Vec<defs::ToolDef> = loaded
        .into_iter()
        .filter(|l| matches!(l.origin, defs::Origin::User | defs::Origin::Project))
        .map(|l| l.def)
        .collect();
    clitools::team::share(&server, &token, &mine).await
}

/// `kor tool pull`: the team's definitions into `<kor home>/tools/team/`.
///
/// # Errors
/// Not connected or the request failed.
pub async fn pull(cfg: &kor_config::Config) -> Result<(Vec<String>, Vec<String>), String> {
    let (server, token) = team(cfg)?;
    let home = tools_home().ok_or("no Kor home")?;
    clitools::team::pull(&server, &token, &home.join("team")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list_becomes_shell_rules() {
        let cfg = kor_config::ToolsConfig {
            deny: vec!["curl".into(), "../x".into()],
            ..Default::default()
        };
        assert_eq!(deny_rules(&cfg), ["Bash(curl)", "Bash(curl *)"]);
        assert!(!cfg.permits("curl.exe") && cfg.permits("git"));
        let only = kor_config::ToolsConfig {
            allow: vec!["git".into()],
            ..Default::default()
        };
        assert!(only.permits("GIT") && !only.permits("npm"));
    }

    #[test]
    fn untrusted_projects_get_no_project_tools() {
        let d = dirs(Path::new("/p"), false);
        assert!(d.project.is_none());
        assert_eq!(
            dirs(Path::new("/p"), true).project,
            Some(Path::new("/p").join(".kor").join("tools"))
        );
    }
}
