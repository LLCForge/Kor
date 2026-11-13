//! `core.*`: masaüstünün CLI ile aynı çekirdek işlevlere ince bağları (parity.toml).
//! Gizlilik modu, `kor undo`, `kor compat status`, `kor config explain/paths/set`,
//! `kor plugin marketplace add/list/remove`, `kor plugin doctor`. İş mantığı burada değil,
//! çağrılan crate'lerde; burası yalnızca JSON'a çevirir. Sözleşme: `ipc-core.ts`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root, s};
use crate::state::AppState;

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("core.{name}");
    let m = m.as_str();
    // `kor doctor --ipc-trace` (§17.2, ADR 0061): needs no open project.
    if name == "ipcTrace" {
        return Some(ipc_trace(args.first().and_then(Value::as_bool)));
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "privacy" => loaded(&root).map(
            |l| json!({ "enabled": l.config.privacy.mode, "offline": l.config.offline.enabled }),
        ),
        "setPrivacy" => set(
            &root,
            "privacy.mode",
            if b(args, 0) { "true" } else { "false" },
        ),
        "undoList" => Ok(undo_list()),
        "undo" => s(args, 0, m).and_then(undo),
        "compat" => Ok(compat(&root)),
        "configExplain" => config_explain(&root),
        "configSet" => s(args, 0, m).and_then(|k| set(&root, k, s(args, 1, m)?)),
        "marketplaces" => Ok(marketplaces()),
        "addMarketplace" => plugins_root()
            .and_then(|r| {
                kor_agent::plugins::add_marketplace(&r, s(args, 0, m)?, s(args, 1, m)?)
                    .map_err(|e| format!("Pazar eklenemedi: {e}"))
            })
            .map(|_| marketplaces()),
        "removeMarketplace" => plugins_root()
            .and_then(|r| {
                kor_agent::plugins::remove_marketplace(&r, s(args, 0, m)?)
                    .map_err(|e| format!("Pazar kaldırılamadı: {e}"))
            })
            .map(|()| marketplaces()),
        "pluginDoctor" => s(args, 0, m).and_then(|id| plugin_doctor(&root, id)),
        // §12.1: WASM plugins run in the CLI (wasmtime lives there, not in the desktop).
        "pluginSearch" => match s(args, 0, m) {
            Ok(q) => {
                let mut a = vec!["search", "--", q];
                if let Some(i) = opt(args, 1) {
                    a.splice(1..1, ["--index", i]);
                }
                kor_plugin(&root, &a).await
            }
            Err(e) => Err(e),
        },
        "pluginCreate" => match s(args, 0, m) {
            Ok(name) => {
                let mut a = vec!["create", name];
                if let Some(f) = opt(args, 1) {
                    a.extend(["--from-skill", f]);
                }
                kor_plugin(&root, &a).await
            }
            Err(e) => Err(e),
        },
        "pluginRevoke" => match s(args, 0, m) {
            Ok(id) => {
                let mut a = vec!["revoke", id];
                a.extend(opt(args, 1));
                kor_plugin(&root, &a).await
            }
            Err(e) => Err(e),
        },
        "pluginRun" => match (s(args, 0, m), s(args, 1, m)) {
            (Ok(id), Ok(input)) => kor_plugin(&root, &["run", id, "--", input]).await,
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        // `kor update channel <name>`
        "updateChannel" => s(args, 0, m).and_then(|name| {
            kor_runtime::update::Channel::parse(name)
                .ok_or("Kanal stable, beta ya da nightly olmalı")?;
            set(&root, "update.channel", name)
        }),
        _ => return None,
    })
}

fn paths(root: &Path) -> kor_config::Paths {
    kor_config::Paths::discover(root)
}

fn loaded(root: &Path) -> Result<kor_config::Loaded, String> {
    crate::bridge::load_config(root)
}

/// `kor config set` ile aynı: kullanıcı ayar dosyasına doğrulanarak yazar. Etkisi yeni sohbetlerde.
fn set(root: &Path, key: &str, value: &str) -> Res {
    if key.len() > 100 || value.len() > 4000 {
        return Err("Geçersiz ayar".into());
    }
    let file = paths(root)
        .user
        .ok_or("Kullanıcı ayar klasörü bulunamadı")?;
    kor_config::set_scalar(&file, key, value).map_err(|e| e.to_string())?;
    // §28.14: every settings change is logged (hash-chained audit log).
    kor_runtime::prefs::log_config_change(key, value, &file);
    config_explain(root)
}

fn layer(l: kor_config::Layer) -> &'static str {
    match l {
        kor_config::Layer::Default => "default",
        kor_config::Layer::System => "system",
        kor_config::Layer::User => "user",
        kor_config::Layer::Project => "project",
        kor_config::Layer::Env => "env",
        kor_config::Layer::Cli => "cli",
    }
}

/// `kor config explain` + `kor config paths`.
fn config_explain(root: &Path) -> Res {
    let p = paths(root);
    let show = |p: &Option<PathBuf>| p.as_ref().map(|p| p.to_string_lossy().into_owned());
    let l = loaded(root)?;
    let settings: Vec<Value> = l
        .settings
        .iter()
        .map(|(k, s)| {
            json!({
                "key": k,
                "value": s.value.to_string(),
                "layer": layer(s.layer),
                "origin": s.origin,
            })
        })
        .collect();
    Ok(json!({
        "paths": { "system": show(&p.system), "user": show(&p.user), "project": show(&p.project) },
        "settings": settings,
    }))
}

/// `kor undo --list`, en yenisi önce.
fn undo_list() -> Value {
    let Some(base) = kor_runtime::undo_dir() else {
        return json!([]);
    };
    Value::Array(
        kor_tools::undo::list(&base)
            .into_iter()
            .map(|j| {
                let ms = j
                    .modified
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64);
                json!({ "session": j.session, "paths": j.paths, "modifiedAt": ms })
            })
            .collect(),
    )
}

/// `kor undo --session <id>`.
fn undo(session: &str) -> Res {
    let base = kor_runtime::undo_dir().ok_or("Kullanıcı veri klasörü bulunamadı")?;
    let j = kor_tools::undo::list(&base)
        .into_iter()
        .find(|j| j.session == session)
        .ok_or("Geri alınacak bir şey yok")?;
    let r = kor_tools::undo::undo(&j.dir).map_err(|e| e.to_string())?;
    let list = |v: &[PathBuf]| {
        v.iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    Ok(json!({
        "restored": list(&r.restored),
        "deleted": list(&r.deleted),
        "skipped": list(&r.skipped),
        "failed": r.failed.iter().map(|(p, e)| json!({ "path": p.to_string_lossy(), "error": e })).collect::<Vec<_>>(),
    }))
}

/// `kor compat status`: kaynaklar ve sayılar; kural/istem metinleri gönderilmez (yalnızca uzunluk).
fn compat(root: &Path) -> Value {
    let c = kor_runtime::load_compat(root);
    let src = |s: &kor_compat::Source| json!({ "eco": s.eco, "scope": s.scope, "path": s.path.to_string_lossy() });
    let rules = |v: &[kor_compat::RuleEntry]| {
        v.iter()
            .map(|r| json!({ "rule": r.rule, "source": src(&r.source) }))
            .collect::<Vec<_>>()
    };
    json!({
        "trusted": c.trusted,
        "instructions": c.instructions.iter().map(|i| json!({ "source": src(&i.source), "applies": i.applies, "chars": i.text.chars().count() })).collect::<Vec<_>>(),
        "permissions": {
            "allow": rules(&c.permissions.allow),
            "ask": rules(&c.permissions.ask),
            "deny": rules(&c.permissions.deny),
            "mode": c.permissions.mode.as_ref().map(|(m, s)| json!({ "mode": m.to_string(), "source": src(s) })),
        },
        "hooks": c.hooks.iter().map(|h| json!({ "event": h.event, "matcher": h.matcher, "command": h.command, "source": src(&h.source) })).collect::<Vec<_>>(),
        "commands": c.commands.iter().map(|x| json!({ "name": x.name, "source": src(&x.source) })).collect::<Vec<_>>(),
        "agents": c.agents.iter().map(|x| json!({ "name": x.name, "source": src(&x.source) })).collect::<Vec<_>>(),
        "skills": c.skills.iter().map(|x| json!({ "name": x.name, "source": src(&x.source) })).collect::<Vec<_>>(),
        "ignoreFiles": c.ignore_files.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "untrustedSkipped": c.untrusted_skipped,
        "warnings": c.warnings,
    })
}

fn plugins_root() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| kor_agent::plugins::plugins_root(&h))
        .ok_or_else(|| "Ev klasörü bulunamadı".into())
}

/// `kor plugin marketplace list`.
fn marketplaces() -> Value {
    let Ok(r) = plugins_root() else {
        return json!([]);
    };
    Value::Array(
        kor_agent::plugins::list_marketplaces(&r)
            .into_iter()
            .map(|m| json!({ "name": m.name, "path": m.path.to_string_lossy(), "plugins": m.plugins }))
            .collect(),
    )
}

fn opt(args: &[Value], i: usize) -> Option<&str> {
    args.get(i)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
}

/// `kor plugin <args>` açık projede; çıktı metni, hata durumunda stderr. Kurulum burada yok (onay
/// istemi terminal ister); `revoke`/`run`/`search`/`create` soru sormaz.
async fn kor_plugin(root: &Path, args: &[&str]) -> Res {
    use kor_platform::NoWindow as _;
    let out = tokio::process::Command::new(crate::kor_cli::path())
        .arg("plugin")
        .args(args)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .no_window()
        .output()
        .await
        .map_err(|e| format!("{} ({e})", crate::kor_cli::MISSING))?;
    if out.status.success() {
        Ok(Value::String(
            String::from_utf8_lossy(&out.stdout).trim_end().to_owned(),
        ))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// `kor plugin doctor`: kurulu eklentinin manifest denetimi (kullanıcı ya da güvenilen proje).
fn plugin_doctor(root: &Path, id: &str) -> Res {
    let mut bases = vec![plugins_root()?];
    if kor_runtime::is_trusted(root) {
        bases.push(root.join(".kor").join("plugins"));
    }
    let dir = bases
        .iter()
        .flat_map(|b| kor_agent::plugins::installed(b))
        .find(|(i, _)| i == id)
        .map(|(_, d)| d)
        .ok_or_else(|| format!("Eklenti bulunamadı: {id}"))?;
    Ok(match kor_agent::plugins::doctor(&dir) {
        Ok(()) => json!({ "ok": true, "error": null }),
        Err(e) => json!({ "ok": false, "error": e }),
    })
}

/// `core.ipcTrace(on?)`: starts (`true`) or stops (`false`) the IPC trace of this app, or reports it.
/// The file is `<kor home>/logs/ipc-trace-<unix ms>.ndjson`; `KOR_IPC_TRACE` picks it at start instead.
fn ipc_trace(on: Option<bool>) -> Res {
    use kor_core::ipc_trace as trace;
    match on {
        Some(true) if !trace::enabled() => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let file = kor_runtime::kor_home()
                .ok_or("Kor veri klasörü bulunamadı")?
                .join("logs")
                .join(format!("ipc-trace-{ms}.ndjson"));
            trace::enable(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        }
        Some(false) => {
            trace::disable();
        }
        _ => {}
    }
    Ok(
        json!({ "enabled": trace::enabled(), "path": trace::path().map(|p| p.display().to_string()) }),
    )
}
