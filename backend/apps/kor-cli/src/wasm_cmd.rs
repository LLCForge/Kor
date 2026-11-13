//! `kor plugin install <folder>|search|create|revoke|run` and the install-time permission prompt
//! (§12.1). Logic: `kor_agent::wasm_plugins`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use kor_agent::wasm_plugins::{self as wp, EVENT_EMIT, FS_READ, Host, Manifest, TOOL_PREFIX};
use kor_i18n::{Localizer, args};

/// The capability list and signer, then `[y/N]` (`--yes` skips the question).
pub(crate) fn approve(l: &Localizer, yes: bool) -> impl FnMut(&Manifest) -> bool + '_ {
    move |m| {
        println!(
            "{}",
            l.t_args(
                "plugin-perm-title",
                &args!("name" => m.name.clone(), "version" => m.version.clone())
            )
        );
        if !m.description.is_empty() {
            println!("  {}", m.description);
        }
        let signer = match &m.public_key {
            Some(k) => l.t_args("plugin-perm-key", &args!("key" => k.clone())),
            None => l.t("plugin-perm-local"),
        };
        println!("  {signer}");
        if m.capabilities.is_empty() {
            println!("  {}", l.t("plugin-perm-none"));
        }
        for c in &m.capabilities {
            println!("  - {c}: {}", describe(l, c));
        }
        crate::shell_cmd::confirm(l, &l.t("plugin-perm-ask"), yes)
    }
}

fn describe(l: &Localizer, cap: &str) -> String {
    match cap {
        FS_READ => l.t("plugin-cap-fs-read"),
        EVENT_EMIT => l.t("plugin-cap-emit"),
        _ => l.t_args(
            "plugin-cap-tool",
            &args!("tool" => cap.trim_start_matches(TOOL_PREFIX).to_owned()),
        ),
    }
}

fn fail(e: impl std::fmt::Display) -> bool {
    eprintln!("{e}");
    false
}

/// `kor plugin install <folder>`: a signed WASM plugin or a Claude Code skill folder.
pub(crate) fn install_dir(l: &Localizer, root: &Path, src: &Path, yes: bool) -> bool {
    match wp::install(root, src, &mut approve(l, yes)) {
        Ok(id) => {
            println!("{}", l.t_args("plugin-installed", &args!("id" => id)));
            true
        }
        Err(e) => fail(e),
    }
}

/// After a marketplace copy: a WASM plugin is verified and asked for; refused = removed again.
pub(crate) fn admit(
    l: &Localizer,
    root: &Path,
    market: &str,
    name: &str,
    dir: &Path,
    yes: bool,
) -> bool {
    let id = format!("{name}@{market}");
    match wp::admit(root, &id, dir, &mut approve(l, yes)) {
        Ok(_) => true,
        Err(e) => {
            let _ = kor_agent::plugins::remove(root, market, name);
            fail(e)
        }
    }
}

/// `kor plugin uninstall <name>` of a folder install, and its grant.
pub(crate) fn uninstall_direct(root: &Path, name: &str) -> bool {
    let dir = root.join(name);
    // Only a direct child folder: never the root itself, `..`, or the marketplace/cache folders.
    if name.is_empty()
        || name.contains(['/', '\\', ':', '\0'])
        || name.starts_with('.')
        || matches!(name, "cache" | "marketplaces")
        || !dir.is_dir()
    {
        return fail("plugin is not installed");
    }
    match std::fs::remove_dir_all(&dir).map_err(|e| e.to_string()) {
        Ok(()) => wp::forget(root, name).map_or_else(fail, |()| true),
        Err(e) => fail(e),
    }
}

/// `kor plugin list`: the folder, then each installed plugin and what it was granted.
pub(crate) fn list(root: &Path) {
    println!("{}", root.display());
    let grants = wp::grants(root);
    for (id, _) in kor_agent::plugins::installed(root) {
        match grants.get(&id) {
            Some(g) if g.capabilities.is_empty() => println!("{id}  -"),
            Some(g) => println!("{id}  {}", g.capabilities.join(", ")),
            None => println!("{id}"),
        }
    }
}

/// `kor plugin search`: the index is `--index` or `KOR_PLUGIN_INDEX`; Kor has no default one.
pub(crate) fn search(l: &Localizer, query: &str, index: Option<&str>) -> bool {
    let Some(src) = index
        .map(str::to_owned)
        .or_else(|| std::env::var("KOR_PLUGIN_INDEX").ok())
        .filter(|s| !s.trim().is_empty())
    else {
        return fail(l.t("plugin-index-missing"));
    };
    let hits = load_index(&src).and_then(|text| wp::search(&text, query));
    match hits {
        Ok(hits) if hits.is_empty() => {
            println!("{}", l.t("plugin-search-none"));
            true
        }
        Ok(hits) => {
            for h in hits {
                println!("{}  {}  {}", h.name, h.version, h.description);
                if !h.source.is_empty() {
                    println!("    {}", h.source);
                }
                if !h.capabilities.is_empty() {
                    println!("    {}", h.capabilities.join(", "));
                }
            }
            true
        }
        Err(e) => fail(l.t_args("plugin-index-bad", &args!("error" => e))),
    }
}

const MAX_INDEX: usize = 16 << 20;

fn load_index(src: &str) -> Result<String, String> {
    if !(src.starts_with("https://") || src.starts_with("http://")) {
        return std::fs::read_to_string(src).map_err(|e| format!("{src}: {e}"));
    }
    // Privacy/offline mode refuses this like any other outbound call.
    kor_core::net::check_url(src).map_err(|b| b.message)?;
    let rt = crate::providers::runtime().map_err(|e| e.to_string())?;
    let body = rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        client
            .get(src)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
    });
    let body = body.map_err(|e| e.to_string())?;
    if body.len() > MAX_INDEX {
        return Err("index is larger than 16 MiB".into());
    }
    String::from_utf8(body.to_vec()).map_err(|e| e.to_string())
}

/// `kor plugin create <dir> [--from-skill <skill folder>]`.
pub(crate) fn create(l: &Localizer, name: &str, from_skill: Option<&Path>) -> bool {
    let dir = PathBuf::from(name);
    let made = match from_skill {
        Some(_) if dir.exists() => Err(format!("{} already exists", dir.display())),
        Some(skill) => wp::pack_skill(skill, &dir).map(|_| ()),
        None => {
            let base = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            wp::scaffold(&dir, &base)
        }
    };
    match made {
        Ok(()) => {
            println!(
                "{}",
                l.t_args("plugin-created", &args!("dir" => dir.display().to_string()))
            );
            true
        }
        Err(e) => fail(e),
    }
}

/// `kor plugin revoke <id> [capability]`.
pub(crate) fn revoke(l: &Localizer, root: &Path, id: &str, cap: Option<&str>) -> bool {
    match wp::revoke(root, id, cap) {
        Ok(left) => {
            let caps = if left.is_empty() {
                "-".to_owned()
            } else {
                left.join(", ")
            };
            println!(
                "{}",
                l.t_args(
                    "plugin-revoked",
                    &args!("id" => id.to_owned(), "caps" => caps)
                )
            );
            true
        }
        Err(e) => fail(e),
    }
}

/// `kor plugin run <id> [input]`: `kor_run` with the granted capabilities, in this project.
pub(crate) fn run(l: &Localizer, root: &Path, id: &str, input: &str) -> bool {
    let Some((_, dir)) = kor_agent::plugins::installed(root)
        .into_iter()
        .find(|(i, _)| i == id)
    else {
        return fail(l.t_args("plugin-not-found", &args!("id" => id.to_owned())));
    };
    let out = wp::read_manifest(&dir).and_then(|m| {
        let grant = wp::grants(root).get(id).cloned().unwrap_or_default();
        // Checked again here: the files on disk may have changed since the install.
        if grant.public_key.is_some() {
            if grant.public_key != m.public_key {
                return Err("the plugin's key changed since it was installed".into());
            }
            wp::verify_signature(&dir, &m)?;
        }
        let project = kor_runtime::project_root().unwrap_or_default();
        wp::run(
            &dir,
            &m,
            &grant.capabilities,
            host(&project, id)?,
            input.as_bytes(),
        )
    });
    match out {
        Ok(bytes) => {
            println!("{}", String::from_utf8_lossy(&bytes));
            true
        }
        Err(e) => fail(e),
    }
}

/// Project files through the same root jail and ignore files as the agent's tools; only
/// read-only tools, since no session is here to approve an edit or a command.
fn host(project: &Path, id: &str) -> Result<Host, String> {
    let mut ctx = kor_tools::ToolCtx::new(project).map_err(|e| e.to_string())?;
    let _ = ctx.load_ai_ignore();
    let ctx = Rc::new(ctx);
    let rt = crate::providers::runtime().map_err(|e| e.to_string())?;
    let tools = kor_tools::default_tools();
    let files = Rc::clone(&ctx);
    let id = id.to_owned();
    Ok(Host {
        read_file: Box::new(move |p| {
            let path = files.resolve(p)?;
            let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
            if len > 8 << 20 {
                return Err("file is larger than 8 MiB".into());
            }
            std::fs::read(path).map_err(|e| e.to_string())
        }),
        call_tool: Box::new(move |name, input| {
            let tool = tools
                .iter()
                .find(|t| t.name() == name || t.rule_name() == name)
                .ok_or_else(|| format!("unknown tool {name}"))?;
            if tool.kind() != kor_tools::ToolKind::Read {
                return Err(format!("{name} is not a read-only tool"));
            }
            let input = serde_json::from_str(input).map_err(|e| e.to_string())?;
            let out = rt.block_on(tool.call(input, &ctx));
            if out.is_error {
                Err(out.content)
            } else {
                Ok(out.content)
            }
        }),
        emit: Box::new(move |e| eprintln!("[{id}] {e}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_install_list_revoke_run_and_uninstall() {
        let t = tempfile::tempdir().unwrap();
        let l = Localizer::new(kor_i18n::Lang::En);
        let root = t.path().join("plugins");
        let skill = t.path().join("review-checklist");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: review-checklist\ndescription: Review steps\n---\nCheck tests.\n",
        )
        .unwrap();
        assert!(install_dir(&l, &root, &skill, true));
        assert!(
            root.join("review-checklist/skills/review-checklist/SKILL.md")
                .is_file()
        );
        assert!(!revoke(&l, &root, "review-checklist", Some("fs.read")));
        assert!(revoke(&l, &root, "review-checklist", None));
        // A skill plugin has no `kor_run`.
        assert!(!run(&l, &root, "review-checklist", ""));
        assert!(!run(&l, &root, "missing", ""));
        assert!(uninstall_direct(&root, "review-checklist"));
        assert!(wp::grants(&root).is_empty());
        assert!(!uninstall_direct(&root, "review-checklist"));
        for bad in ["", "..", "cache", "a/.."] {
            assert!(!uninstall_direct(&root, bad), "{bad:?}");
        }
        assert!(root.is_dir());
        assert!(!search(
            &l,
            "x",
            Some(&t.path().join("none.json").display().to_string())
        ));
        let index = t.path().join("index.json");
        std::fs::write(&index, r#"[{"name":"demo","description":"Demo"}]"#).unwrap();
        assert!(search(&l, "demo", Some(&index.display().to_string())));
    }
}
