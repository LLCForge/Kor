//! `compat.*`: `kor compat conflicts`, `kor compat export --to <target>` and the MCP servers
//! other tools define (part of `kor compat status`; `core.compat` carries the rest). Thin JSON
//! wrappers over `kor-compat`; rule texts are not sent. `compat.reveal(path)` shows a loaded
//! source file in the system file manager (only paths Kor actually loaded; nothing is opened or run).

use std::path::PathBuf;

use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::api::{Res, root, s};
use crate::state::AppState;

/// Unknown method: `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let src = |s: &kor_compat::Source| json!({ "eco": s.eco, "scope": s.scope, "path": s.path.to_string_lossy() });
    Some(match name {
        "conflicts" => {
            let c = kor_runtime::load_compat(&root);
            Ok(Value::Array(
                kor_compat::conflicts(&c)
                    .iter()
                    .map(|x| {
                        json!({
                            "key": x.key,
                            "rules": x.rules.iter().map(src).collect::<Vec<_>>(),
                            "winner": x.winner.to_string_lossy(),
                        })
                    })
                    .collect(),
            ))
        }
        "export" => s(args, 0, "compat.export").and_then(|to| {
            let target: kor_compat::export::Target = to.parse()?;
            let out = kor_compat::export::export(&root, target)
                .map_err(|e| format!("Dışa aktarılamadı: {e}"))?;
            Ok(json!({
                "dir": out.dir.to_string_lossy(),
                "files": out.files.iter().map(|f| f.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>(),
                "rules": out.rules,
                "warnings": out.warnings,
            }))
        }),
        "mcp" => {
            let c = kor_runtime::load_compat(&root);
            Ok(Value::Array(
                c.mcp_servers
                    .iter()
                    .map(|m| json!({ "name": m.name, "source": src(&m.source) }))
                    .collect(),
            ))
        }
        "reveal" => s(args, 0, "compat.reveal").and_then(|p| {
            let c = kor_runtime::load_compat(&root);
            let path = std::path::Path::new(p);
            if !loaded_paths(&c).any(|l| l == path) {
                return Err("Bu dosya Kor'un yüklediği kaynaklardan biri değil".into());
            }
            app.opener()
                .reveal_item_in_dir(path)
                .map_err(|e| format!("Gösterilemedi: {e}"))?;
            Ok(Value::Null)
        }),
        _ => return None,
    })
}

/// Every file Kor read for `c` (the allowlist of `compat.reveal`).
fn loaded_paths(c: &kor_compat::Compat) -> impl Iterator<Item = &std::path::Path> {
    let p = &c.permissions;
    c.instructions
        .iter()
        .map(|i| &i.source)
        .chain(
            p.allow
                .iter()
                .chain(&p.ask)
                .chain(&p.deny)
                .map(|r| &r.source),
        )
        .chain(p.mode.iter().map(|(_, s)| s))
        .chain(c.hooks.iter().map(|h| &h.source))
        .chain(c.commands.iter().map(|x| &x.source))
        .chain(c.agents.iter().map(|x| &x.source))
        .chain(c.skills.iter().map(|x| &x.source))
        .chain(c.mcp_servers.iter().map(|m| &m.source))
        .map(|s| s.path.as_path())
        .chain(c.ignore_files.iter().map(PathBuf::as_path))
}
