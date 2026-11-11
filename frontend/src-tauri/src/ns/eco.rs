//! Ekosistem ayar ekranları: MCP sunucuları, eklentiler, ajanlar, dosya komutları ve takımlar.
//! Sözleşmeler: `ipc-mcp.ts`, `ipc-plugins.ts`, `ipc-agents.ts`, `ipc-commands.ts`, `ipc-teams.ts`.
//!
//! MCP: arayüz Kor'un kendi `<config>/kor/mcp.json` dosyasını düzenler; ajan aynı dosyayı okur
//! (`kor_agent::mcp_tools::discover`). `~/.claude.json` ve güvenilen projenin `.mcp.json` sunucuları
//! listede görünür ama oraya yazılmaz: kapatmak/kaldırmak Kor dosyasına `"disabled": true` yazar,
//! düzenlemek Kor dosyasında onu gölgeleyen bir kopya oluşturur.

use kor_platform::NoWindow as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncReadExt as _;

use crate::api::{Res, b, root, s};
use crate::state::AppState;

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    let method = format!("{ns}.{name}");
    let m = method.as_str();
    Some(match (ns, name) {
        ("mcp", "list") => mcp_views(&root, false).await,
        ("mcp", "reconnect") => mcp_views(&root, true).await,
        ("mcp", "save") => match args.first() {
            Some(cfg) => {
                async {
                    mcp_save(&root, cfg)?;
                    mcp_views(&root, false).await
                }
                .await
            }
            None => Err("Sunucu yapılandırması eksik".into()),
        },
        ("mcp", "remove") => {
            async {
                mcp_remove(&root, s(args, 0, m)?)?;
                mcp_views(&root, false).await
            }
            .await
        }
        ("mcp", "setEnabled") => {
            async {
                mcp_set_enabled(&root, s(args, 0, m)?, b(args, 1))?;
                mcp_views(&root, false).await
            }
            .await
        }

        ("plugins", "list") => Ok(Value::Array(plugins(&root))),
        ("plugins", "catalog") => catalog(&root),
        ("plugins", "setEnabled") => {
            async {
                let p = find_plugin(&root, s(args, 0, m)?)?;
                kor_agent::plugins::set_enabled(&p.base, &p.id, b(args, 1))?;
                Ok(Value::Array(plugins(&root)))
            }
            .await
        }
        ("plugins", "install") => {
            async {
                let name = s(args, 0, m)?;
                let entry = CATALOG
                    .iter()
                    .find(|c| c.kind == Kind::Plugin && c.id == name)
                    .ok_or_else(|| format!("Katalogda böyle bir eklenti yok: {name}"))?;
                install(app, name, entry.source).await?;
                Ok(Value::Array(plugins(&root)))
            }
            .await
        }
        ("plugins", "installRepo") => {
            async {
                let url = s(args, 0, m)?;
                let name = install_name(url)?;
                install(app, &name, url).await?;
                Ok(Value::Array(plugins(&root)))
            }
            .await
        }
        ("plugins", "update") => {
            async {
                let p = find_plugin(&root, s(args, 0, m)?)?;
                if !p.dir.join(".git").exists() {
                    return Err("Bu eklenti git ile kurulmamış; güncellenemez".into());
                }
                git(&p.dir, &["pull", "--ff-only", "--quiet"], 120).await?;
                Ok(Value::Array(plugins(&root)))
            }
            .await
        }
        ("plugins", "uninstall") => {
            async {
                let p = find_plugin(&root, s(args, 0, m)?)?;
                crate::api_files::to_recycle_bin(&p.dir)?;
                Ok(Value::Array(plugins(&root)))
            }
            .await
        }
        ("plugins", "checkUpdates") => check_updates(&root).await,
        ("plugins", "addMcp") => {
            async {
                add_catalog_mcp(s(args, 0, m)?)?;
                catalog(&root)
            }
            .await
        }
        ("plugins", "storeSearch") => {
            store_search(args.first().and_then(Value::as_str).unwrap_or("")).await
        }
        ("plugins", "openFolder") => {
            let dir = user_plugins_root().and_then(|d| {
                std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                Ok(d)
            });
            dir.and_then(|d| {
                app.opener()
                    .open_path(d.to_string_lossy(), None::<&str>)
                    .map(|()| Value::Null)
                    .map_err(|e| e.to_string())
            })
        }

        ("agents", "list") => Ok(agents(&root)),
        ("commands", "list") => Ok(commands(&root)),
        ("teams", "list") => Ok(Value::Array(teams(&root))),
        ("teams", "saveUser") => match args.first() {
            Some(v) => save_teams(v).map(|()| Value::Array(teams(&root))),
            None => Err("Takım listesi eksik".into()),
        },
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Ortak dosya yardımcıları
// ---------------------------------------------------------------------------------------------

use kor_agent::mcp_config::{
    self as mcpc, Entry, config_of, disabled, is_http, is_stub, read_object, servers_of,
    string_map, strings, valid_id, write_atomic,
};

fn user_config_dir() -> Result<PathBuf, String> {
    kor_config::user_dir().ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

// ---------------------------------------------------------------------------------------------
// MCP (düzenleme mantığı çekirdekte: `kor_agent::mcp_config`, `kor mcp add/remove/...` ile aynı)
// ---------------------------------------------------------------------------------------------

fn kor_mcp_file() -> Result<PathBuf, String> {
    Ok(user_config_dir()?.join("mcp.json"))
}

fn mcp_loc(root: &Path) -> Result<mcpc::Locations, String> {
    Ok(mcpc::Locations {
        kor_file: kor_mcp_file()?,
        home: dirs::home_dir(),
        root: root.to_owned(),
        trusted: kor_runtime::is_trusted(root),
    })
}

fn mcp_entries(root: &Path) -> Result<Vec<Entry>, String> {
    mcpc::entries(&mcp_loc(root)?)
}

/// Son bağlantı denemesinin sonucu; yapılandırma değişince geçersizleşir.
struct Probe {
    signature: String,
    result: Result<usize, String>,
}

fn probes() -> &'static Mutex<BTreeMap<String, Probe>> {
    static P: OnceLock<Mutex<BTreeMap<String, Probe>>> = OnceLock::new();
    P.get_or_init(Mutex::default)
}

fn signature(raw: &Value) -> String {
    raw.to_string()
}

/// Etkin stdio sunucularını başlatıp araçlarını listeler, sonra kapatır (istemciler düşünce süreç ölür).
async fn probe(root: &Path, entries: &[&Entry]) {
    let env = |k: &str| std::env::var(k).ok();
    let x = |s: &str| kor_agent::mcp_tools::expand(s, &env);
    let specs: Vec<_> = entries
        .iter()
        .map(|e| kor_agent::mcp_tools::ServerSpec {
            name: e.id.clone(),
            command: x(e.raw.get("command").and_then(Value::as_str).unwrap_or("")),
            args: strings(e.raw.get("args")).iter().map(|a| x(a)).collect(),
            env: string_map(e.raw.get("env"))
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), x(v))))
                .collect(),
            origin: e.external.unwrap_or("mcp.json").to_owned(),
        })
        .collect();
    let (tools, report) = kor_agent::mcp_tools::connect_all(&specs, root).await;
    drop(tools);
    let mut p = probes()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for c in report {
        if let Some(e) = entries.iter().find(|e| e.id == c.server) {
            p.insert(
                c.server,
                Probe {
                    signature: signature(&e.raw),
                    result: c.result,
                },
            );
        }
    }
}

async fn mcp_views(root: &Path, reconnect: bool) -> Res {
    let entries = mcp_entries(root)?;
    let stale: Vec<&Entry> = {
        let p = probes()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .iter()
            .filter(|e| !disabled(&e.raw) && !is_http(&e.raw))
            .filter(|e| {
                reconnect
                    || p.get(&e.id)
                        .is_none_or(|x| x.signature != signature(&e.raw))
            })
            .collect()
    };
    if !stale.is_empty() {
        probe(root, &stale).await;
    }
    let p = probes()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let views = entries
        .iter()
        .map(|e| {
            let config = config_of(e);
            let target = if is_http(&e.raw) {
                config["url"].as_str().unwrap_or_default().to_owned()
            } else {
                std::iter::once(config["command"].as_str().unwrap_or_default().to_owned())
                    .chain(strings(e.raw.get("args")))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let (state, tools, error) = if disabled(&e.raw) {
                ("disabled", 0, None)
            } else if is_http(&e.raw) {
                (
                    "error",
                    0,
                    Some(
                        "HTTP MCP sunucuları henüz bağlanmıyor; yalnızca stdio sunucuları çalışır"
                            .to_owned(),
                    ),
                )
            } else {
                match p.get(&e.id).map(|x| &x.result) {
                    Some(Ok(n)) => ("connected", *n, None),
                    Some(Err(err)) => ("error", 0, Some(err.clone())),
                    None => ("error", 0, Some("Bağlantı denenmedi".to_owned())),
                }
            };
            json!({
                "config": config,
                "status": {
                    "id": e.id,
                    "label": config.get("label"),
                    "transport": config["transport"],
                    "target": target,
                    "enabled": !disabled(&e.raw),
                    "state": state,
                    "toolCount": tools,
                    "error": error,
                }
            })
        })
        .collect();
    Ok(Value::Array(views))
}

fn edit_kor_servers(
    f: impl FnOnce(&mut Map<String, Value>) -> Result<(), String>,
) -> Result<(), String> {
    mcpc::edit_servers(&kor_mcp_file()?, f)
}

fn mcp_save(root: &Path, cfg: &Value) -> Result<(), String> {
    mcpc::save(&mcp_loc(root)?, cfg)
}

fn mcp_remove(root: &Path, id: &str) -> Result<(), String> {
    mcpc::remove(&mcp_loc(root)?, id)?;
    probes()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(id);
    Ok(())
}

fn mcp_set_enabled(root: &Path, id: &str, enabled: bool) -> Result<(), String> {
    mcpc::set_enabled(&mcp_loc(root)?, id, enabled)
}

// ---------------------------------------------------------------------------------------------
// Eklentiler
// ---------------------------------------------------------------------------------------------

#[derive(PartialEq, Eq)]
enum Kind {
    Plugin,
    Mcp,
}

struct CatalogEntry {
    kind: Kind,
    /// Eklenti klasör adı ya da MCP kimliği.
    id: &'static str,
    label: &'static str,
    /// Git deposu ya da npm paketi.
    source: &'static str,
    description: &'static str,
    author: &'static str,
}

// `frontend/src/core/plugin-catalog.ts` ile aynı liste (arayüz çevirileri bu metinlerle eşleşir).
const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        kind: Kind::Mcp,
        id: "rea",
        label: "rea — tersine mühendislik",
        source: "rea-agents@6.8.0",
        description: "Binary, uygulama ve çalışma zamanı tersine mühendisliği için MCP sunucusu (Node gerektirir).",
        author: "",
    },
    CatalogEntry {
        kind: Kind::Plugin,
        id: "ecc",
        label: "ecc",
        source: "https://github.com/affaan-m/ecc.git",
        description: "Ajan harness optimizasyonu: komut, skill, ajan ve kancalar.",
        author: "affaan-m",
    },
    CatalogEntry {
        kind: Kind::Plugin,
        id: "claude-mem",
        label: "claude-mem",
        source: "https://github.com/thedotmack/claude-mem.git",
        description: "Hafıza sıkıştırma: oturumlar arası bağlamı korur.",
        author: "Alex Newman",
    },
    CatalogEntry {
        kind: Kind::Plugin,
        id: "ponytail",
        label: "ponytail",
        source: "https://github.com/DietrichGebert/ponytail.git",
        description: "Tembel kıdemli geliştirici modu: en yalın, en kısa çözüm.",
        author: "Dietrich Gebert",
    },
    CatalogEntry {
        kind: Kind::Plugin,
        id: "karpathy-guidelines",
        label: "karpathy-guidelines",
        source: "https://github.com/forrestchang/andrej-karpathy-skills.git",
        description: "Yaygın LLM kodlama hatalarını azaltan davranış kuralları.",
        author: "forrestchang",
    },
    CatalogEntry {
        kind: Kind::Plugin,
        id: "superpowers",
        label: "superpowers",
        source: "https://github.com/obra/superpowers.git",
        description: "Çekirdek skill kütüphanesi: TDD, hata ayıklama, işbirliği desenleri.",
        author: "Jesse Vincent",
    },
];

fn user_plugins_root() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| kor_agent::plugins::plugins_root(&h))
        .ok_or_else(|| "Ev klasörü bulunamadı".into())
}

struct Plugin {
    base: PathBuf,
    id: String,
    dir: PathBuf,
    source: &'static str,
}

fn installed_plugins(root: &Path) -> Vec<Plugin> {
    let mut bases = Vec::new();
    if let Ok(user) = user_plugins_root() {
        bases.push((user, "user"));
    }
    if kor_runtime::is_trusted(root) {
        bases.push((root.join(".kor").join("plugins"), "project"));
    }
    bases
        .into_iter()
        .flat_map(|(base, source)| {
            kor_agent::plugins::installed(&base)
                .into_iter()
                .map(move |(id, dir)| Plugin {
                    base: base.clone(),
                    id,
                    dir,
                    source,
                })
        })
        .collect()
}

fn find_plugin(root: &Path, dir_name: &str) -> Result<Plugin, String> {
    installed_plugins(root)
        .into_iter()
        .find(|p| p.id == dir_name)
        .ok_or_else(|| format!("Eklenti bulunamadı: {dir_name}"))
}

fn manifest(dir: &Path) -> Map<String, Value> {
    let candidates = [
        kor_agent::plugins::meta_dir(dir).join("plugin.json"),
        dir.join("plugin.json"),
    ];
    candidates
        .iter()
        .find_map(|p| read_object(p).ok().filter(|o| !o.is_empty()))
        .unwrap_or_default()
}

/// Klasördeki (alt klasörler dahil, sınırlı) dosyalar.
fn count_files(dir: &Path, ext: Option<&str>, depth: usize) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .take(2000)
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                if depth < 4 {
                    count_files(&p, ext, depth + 1)
                } else {
                    0
                }
            } else {
                usize::from(ext.is_none_or(|x| p.extension().is_some_and(|e| e == x)))
            }
        })
        .sum()
}

fn skill_dirs(dir: &Path, declared: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let standard = ["skills", ".agents/skills", ".claude/skills"];
    for rel in declared.iter().map(String::as_str).chain(standard) {
        let rel = rel.trim_start_matches("./");
        if rel.contains("..") || Path::new(rel).is_absolute() {
            continue;
        }
        let d = dir.join(rel);
        if d.is_dir() && !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn plugin_view(p: &Plugin) -> Value {
    let m = manifest(&p.dir);
    let text = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_owned);
    let author = match m.get("author") {
        Some(Value::String(a)) => Some(a.clone()),
        Some(Value::Object(o)) => o.get("name").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    };
    let declared = match m.get("skills") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(v @ Value::Array(_)) => strings(Some(v)),
        _ => Vec::new(),
    };
    let skills: usize = skill_dirs(&p.dir, &declared)
        .iter()
        .map(|d| {
            std::fs::read_dir(d)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.path().join("SKILL.md").is_file())
                        .count()
                })
                .unwrap_or(0)
        })
        .sum();
    let name = text("name").unwrap_or_else(|| p.id.clone());
    let mut manifest = json!({
        "name": name,
        "version": text("version"),
        "description": text("description").unwrap_or_default(),
        "author": author,
    });
    if let Some(sk) = m.get("skills") {
        manifest["skills"] = sk.clone();
    }
    json!({
        "name": name,
        "dirName": p.id,
        "dir": p.dir.to_string_lossy(),
        "source": p.source,
        "manifest": manifest,
        "enabled": kor_agent::plugins::is_enabled(&p.base, &p.id),
        "contributions": {
            "commands": count_files(&p.dir.join("commands"), Some("md"), 0),
            "skills": skills,
            "agents": count_files(&p.dir.join("agents"), Some("md"), 0),
            "tools": count_files(&p.dir.join("tools"), None, 0),
        },
        "managed": p.dir.join(".git").exists(),
    })
}

fn plugins(root: &Path) -> Vec<Value> {
    installed_plugins(root).iter().map(plugin_view).collect()
}

fn catalog(root: &Path) -> Res {
    let ids: Vec<String> = installed_plugins(root).into_iter().map(|p| p.id).collect();
    let mcp = servers_of(&read_object(&kor_mcp_file()?)?);
    Ok(Value::Array(
        CATALOG
            .iter()
            .map(|c| match c.kind {
                Kind::Mcp => json!({
                    "kind": "mcp",
                    "id": c.id,
                    "label": c.label,
                    "npmPackage": c.source,
                    "mcpArgs": ["mcp"],
                    "description": c.description,
                    "installed": mcp.get(c.id).is_some_and(|r| !is_stub(r)),
                }),
                Kind::Plugin => json!({
                    "kind": "plugin",
                    "name": c.id,
                    "repo": c.source,
                    "description": c.description,
                    "author": c.author,
                    "installed": ids.iter().any(|i| i == c.id),
                }),
            })
            .collect(),
    ))
}

fn add_catalog_mcp(id: &str) -> Result<(), String> {
    let c = CATALOG
        .iter()
        .find(|c| c.kind == Kind::Mcp && c.id == id)
        .ok_or_else(|| format!("Katalogda böyle bir MCP sunucusu yok: {id}"))?;
    edit_kor_servers(|servers| {
        servers.insert(
            c.id.to_owned(),
            json!({ "label": c.label, "command": "npx", "args": ["-y", c.source, "mcp"], "env": {} }),
        );
        Ok(())
    })
}

/// `https://host/owner/repo(.git)` → güvenli klasör adı (`^[a-z0-9][a-z0-9_-]{0,63}$`).
fn install_name(url: &str) -> Result<String, String> {
    let u = reqwest::Url::parse(url.trim()).map_err(|_| "Geçersiz adres".to_owned())?;
    let ok = u.scheme() == "https"
        && u.host_str().is_some()
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none()
        && url.len() <= 500;
    if !ok {
        return Err("Yalnızca kimlik bilgisi içermeyen https depo adresleri kabul edilir".into());
    }
    let last = u
        .path_segments()
        .and_then(|mut s| s.rfind(|x| !x.is_empty()))
        .unwrap_or("");
    let mut name: String = last
        .trim_end_matches(".git")
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    name = name.trim_matches(['-', '_']).chars().take(64).collect();
    if valid_id(&name, 64) {
        Ok(name)
    } else {
        Err("Depo adından klasör adı türetilemedi".into())
    }
}

fn git_command() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("git");
    cmd.kill_on_drop(true).env("GIT_TERMINAL_PROMPT", "0");
    cmd.no_window();
    cmd
}

fn git_missing(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "Git bulunamadı".into()
    } else {
        e.to_string()
    }
}

async fn git(dir: &Path, args: &[&str], secs: u64) -> Result<String, String> {
    // Gizlilik/çevrimdışı mod: uzak depoya giden git işlemleri çalıştırılmaz (ADR 0035).
    if matches!(args.first(), Some(&("fetch" | "pull" | "push" | "clone")))
        && let Some(b) = kor_core::net::refusal("git remote")
    {
        return Err(b.message);
    }
    let out = tokio::time::timeout(
        Duration::from_secs(secs),
        git_command().arg("-C").arg(dir).args(args).output(),
    )
    .await
    .map_err(|_| "Git zaman aşımına uğradı".to_owned())?
    .map_err(|e| git_missing(&e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .last()
            .unwrap_or("git başarısız")
            .trim()
            .to_owned())
    }
}

/// `git clone --progress` çıktısındaki son indirme yüzdesi (yalnızca "Receiving objects": sonraki
/// aşamalar da 0'dan başladığı için çubuk geri sıçramasın; bitişte 100 ayrıca gönderilir).
fn last_percent(chunk: &str) -> Option<u8> {
    chunk
        .split(['\r', '\n'])
        .filter(|l| l.contains("Receiving objects"))
        .filter_map(|l| {
            let pct = l.split('%').next()?;
            pct.rsplit(' ').next()?.trim().parse::<u8>().ok()
        })
        .next_back()
}

/// Depoyu kullanıcı eklenti klasörüne klonlar (kod çalıştırmaz) ve pasif bırakır.
async fn install(app: &AppHandle, name: &str, url: &str) -> Result<(), String> {
    if !valid_id(name, 64) {
        return Err("Geçersiz eklenti adı".into());
    }
    install_name(url)?;
    kor_core::net::check_url(url).map_err(|b| b.message)?;
    let base = user_plugins_root()?;
    let dest = base.join(name);
    if dest.exists() {
        return Err(format!("{name} zaten kurulu"));
    }
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let mut child = git_command()
        .args(["clone", "--depth", "1", "--progress", "--"])
        .arg(url)
        .arg(&dest)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| git_missing(&e))?;
    let mut stderr = child.stderr.take().ok_or("git çıktısı alınamadı")?;
    let progress = async {
        let mut buf = [0u8; 4096];
        let mut tail = String::new();
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buf[..n]);
            if let Some(p) = last_percent(&text) {
                let _ = app.emit(
                    "kor:plugins/installProgress",
                    json!({ "name": name, "percent": p }),
                );
            }
            tail.push_str(&text);
            if tail.len() > 8000 {
                tail.drain(..tail.len() - 4000);
            }
        }
        tail
    };
    let run = async {
        let tail = progress.await;
        (child.wait().await, tail)
    };
    let result = tokio::time::timeout(Duration::from_secs(600), run).await;
    let failed = match result {
        Ok((Ok(st), _)) if st.success() => None,
        Ok((Ok(_), tail)) => Some(
            tail.split(['\r', '\n'])
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("git clone başarısız")
                .trim()
                .to_owned(),
        ),
        Ok((Err(e), _)) => Some(e.to_string()),
        Err(_) => Some("Git zaman aşımına uğradı".into()),
    };
    if let Some(err) = failed {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(format!("{name} kurulamadı: {err}"));
    }
    kor_agent::plugins::set_enabled(&base, name, false)?;
    let _ = app.emit(
        "kor:plugins/installProgress",
        json!({ "name": name, "percent": 100 }),
    );
    Ok(())
}

async fn check_updates(root: &Path) -> Res {
    let managed: Vec<Plugin> = installed_plugins(root)
        .into_iter()
        .filter(|p| p.dir.join(".git").exists())
        .collect();
    let checks = managed.iter().map(|p| async move {
        let behind = async {
            git(&p.dir, &["fetch", "--quiet"], 60).await?;
            let n = git(&p.dir, &["rev-list", "--count", "HEAD..@{u}"], 20).await?;
            Ok::<bool, String>(n.trim().parse::<u64>().unwrap_or(0) > 0)
        }
        .await;
        (p.id.clone(), behind.unwrap_or(false))
    });
    let out: Map<String, Value> = futures::future::join_all(checks)
        .await
        .into_iter()
        .map(|(k, v)| (k, Value::Bool(v)))
        .collect();
    Ok(Value::Object(out))
}

/// GitHub depo araması (anahtarsız). Yanıt güvenilmez veridir: yalnızca github.com https adresleri.
async fn store_search(query: &str) -> Res {
    let q = query.trim();
    if q.chars().count() > 200 {
        return Err("Arama en fazla 200 karakter olabilir".into());
    }
    let q = if q.is_empty() {
        "topic:claude-code-plugin"
    } else {
        q
    };
    let url = reqwest::Url::parse_with_params(
        "https://api.github.com/search/repositories",
        &[
            ("q", q),
            ("sort", "stars"),
            ("order", "desc"),
            ("per_page", "30"),
        ],
    )
    .map_err(|e| e.to_string())?;
    let res = kor_core::net::client_builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Kor-PluginStore")
        .send()
        .await
        .map_err(|e| format!("Mağaza alınamadı: {e}"))?;
    let status = res.status().as_u16();
    if status == 403 || status == 429 {
        return Err("GitHub arama hız sınırı aşıldı; birkaç dakika sonra tekrar deneyin.".into());
    }
    if !res.status().is_success() {
        return Err(format!("Mağaza alınamadı (HTTP {status})."));
    }
    let text = res.text().await.map_err(|e| e.to_string())?;
    Ok(Value::Array(map_store(&text, &installed_ids())))
}

fn installed_ids() -> Vec<String> {
    user_plugins_root()
        .map(|b| {
            kor_agent::plugins::installed(&b)
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        })
        .unwrap_or_default()
}

fn map_store(text: &str, installed: &[String]) -> Vec<Value> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for item in v["items"].as_array().into_iter().flatten().take(100) {
        let (Some(full), Some(clone)) = (item["full_name"].as_str(), item["clone_url"].as_str())
        else {
            continue;
        };
        let host_ok = reqwest::Url::parse(clone).is_ok_and(|u| u.host_str() == Some("github.com"));
        let Ok(name) = install_name(clone) else {
            continue;
        };
        if !host_ok || seen.contains(&clone.to_owned()) {
            continue;
        }
        seen.push(clone.to_owned());
        out.push(json!({
            "name": name,
            "fullName": full,
            "description": item["description"].as_str().unwrap_or("").trim(),
            "author": item["owner"]["login"].as_str().or_else(|| full.split('/').next()).unwrap_or(""),
            "repoUrl": clone,
            "stars": item["stargazers_count"].as_u64().unwrap_or(0),
            "topics": strings(item.get("topics")),
            "installed": installed.contains(&name),
        }));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Ajanlar ve komutlar
// ---------------------------------------------------------------------------------------------

/// Kaynağı bir eklenti klasöründeyse eklentinin adı.
fn plugin_origin(path: &Path, plugin_dirs: &[PathBuf]) -> Option<String> {
    plugin_dirs
        .iter()
        .find(|d| path.starts_with(d))
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

fn compat_and_plugins(root: &Path) -> (kor_compat::Compat, Vec<PathBuf>) {
    let compat = kor_runtime::load_compat(root);
    let home = dirs::home_dir();
    let dirs = kor_runtime::plugin_dirs(root, home.as_deref(), compat.trusted);
    (compat, dirs)
}

fn agents(root: &Path) -> Value {
    let (compat, plugin_dirs) = compat_and_plugins(root);
    let mut out: Vec<Value> = [
        (
            "general-purpose",
            "Her türlü çok adımlı görev; tüm araçlar.",
        ),
        (
            "Explore",
            "Kod tabanında salt okunur arama; bulguları raporlar.",
        ),
        (
            "Plan",
            "Projeyi okuyup uygulama planı çıkarır; dosya değiştirmez.",
        ),
    ]
    .iter()
    .map(|(name, description)| {
        json!({
            "name": name, "description": description, "source": "builtin", "origin": null,
            "model": null, "tools": "all", "manualApproval": false,
        })
    })
    .collect();
    let summary = |a: &kor_compat::AgentDef, source: &str, origin: Option<String>| {
        json!({
            "name": a.name,
            "description": a.description,
            "source": source,
            "origin": origin,
            "model": a.model,
            "tools": if a.tools.is_empty() { json!("all") } else { json!(a.tools.len()) },
            "manualApproval": false,
        })
    };
    for a in &compat.agents {
        let source = if a.source.scope == kor_compat::Scope::User {
            "user"
        } else {
            "project"
        };
        out.push(summary(
            a,
            source,
            plugin_origin(&a.source.path, &plugin_dirs),
        ));
    }
    for a in kor_agent::builtin::agents() {
        if !compat.agents.iter().any(|c| c.name == a.name) {
            out.push(summary(&a, "embedded", None));
        }
    }
    Value::Array(out)
}

fn commands(root: &Path) -> Value {
    let (compat, plugin_dirs) = compat_and_plugins(root);
    Value::Array(
        compat
            .commands
            .iter()
            .map(|c| {
                let source = if plugin_origin(&c.source.path, &plugin_dirs).is_some() {
                    "plugin"
                } else if c.source.scope == kor_compat::Scope::User {
                    "user"
                } else {
                    "project"
                };
                let description = c.description.clone().unwrap_or_else(|| {
                    c.body
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .trim()
                        .chars()
                        .take(120)
                        .collect()
                });
                json!({
                    "name": c.name,
                    "description": description,
                    "argumentHint": c.argument_hint,
                    "body": c.body,
                    "source": source,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Takımlar
// ---------------------------------------------------------------------------------------------

// `frontend/src/core/teams.ts` BUILTIN_TEAMS ile aynı.
const BUILTIN_TEAMS: &str = r#"[
  {"name":"ozellik","description":"Yeni özellik: planla → uygula → gözden geçir.","roles":[
    {"role":"planlayıcı","agent":"Plan","instructions":"Görev: {task}\n\nKod tabanını incele ve dosya dosya uygulama planı çıkar: değişecek dosyalar, yeni modüller, testler, riskler. Kod yazma."},
    {"role":"geliştirici","agent":"general-purpose","instructions":"Görev: {task}\n\nÖnceki rolün planı:\n{previous}\n\nPlanı uygula: kodu yaz, testleri ekle/çalıştır, derlemeyi doğrula. Sonunda yapılan değişiklikleri dosya listesiyle özetle."},
    {"role":"gözden geçiren","agent":"code-reviewer","instructions":"Görev: {task}\n\nGeliştiricinin raporu:\n{previous}\n\nDeğişen dosyaları oku; doğruluk, güvenlik ve eksik test açısından incele. Bulduğun sorunları önem sırasıyla listele; kritik olanları düzelt."}]},
  {"name":"hata","description":"Hata düzeltme: keşfet → düzelt → doğrula.","roles":[
    {"role":"kaşif","agent":"Explore","instructions":"Hata: {task}\n\nHatanın kök nedenini bul: ilgili dosyalar, çağrı zinciri, yeniden üretme adımları. Kod değiştirme; bulgularını dosya:satır referanslarıyla raporla."},
    {"role":"geliştirici","agent":"general-purpose","instructions":"Hata: {task}\n\nKaşif raporu:\n{previous}\n\nKök nedeni düzelt, regresyon testi ekle ve testleri çalıştır. Yaptığın değişiklikleri özetle."},
    {"role":"doğrulayıcı","agent":"code-reviewer","instructions":"Hata: {task}\n\nDüzeltme raporu:\n{previous}\n\nDüzeltmenin kök nedeni kapattığını ve yan etki yaratmadığını doğrula; eksik varsa tamamla."}]},
  {"name":"inceleme","description":"Kod incelemesi: bağımsız inceleyici + sadeleştirici.","roles":[
    {"role":"inceleyici","agent":"code-reviewer","instructions":"İnceleme konusu: {task}\n\nDeğişiklikleri doğruluk, güvenlik, performans ve test kapsamı açısından incele; bulguları önem sırasıyla listele."},
    {"role":"sadeleştirici","agent":"code-simplifier","instructions":"İnceleme konusu: {task}\n\nİnceleyici bulguları:\n{previous}\n\nDavranışı değiştirmeden kodu sadeleştir; inceleyicinin kritik bulgularını düzelt. Yaptıklarını özetle."}]}
]"#;

/// Takım listesini doğrular (teams.ts `teamFileSchema`), kaynak alanı olmadan.
fn validate_teams(v: &Value) -> Result<Vec<Value>, String> {
    let list = v.as_array().ok_or("Takım listesi bir dizi olmalı")?;
    if list.len() > 50 {
        return Err("En fazla 50 takım olabilir".into());
    }
    let text = |o: &Value, k: &str, max: usize| -> Option<String> {
        let t = o.get(k)?.as_str()?.trim();
        (!t.is_empty() && t.chars().count() <= max).then(|| t.to_owned())
    };
    list.iter()
        .map(|t| {
            let name = t.get("name").and_then(Value::as_str).unwrap_or_default();
            if !valid_id(name, 40) {
                return Err("Takım adı harf/rakamla başlamalı; harf, rakam, _ ve - (en fazla 40)".into());
            }
            let description = text(t, "description", 300).ok_or("Takım açıklaması 1-300 karakter olmalı")?;
            let roles = t.get("roles").and_then(Value::as_array).ok_or("Takımın rolleri olmalı")?;
            if roles.is_empty() || roles.len() > 12 {
                return Err("Bir takımda 1-12 rol olmalı".into());
            }
            let roles = roles
                .iter()
                .map(|r| {
                    Ok(json!({
                        "role": text(r, "role", 40).ok_or("Rol adı 1-40 karakter olmalı")?,
                        "agent": text(r, "agent", 60).ok_or("Ajan adı 1-60 karakter olmalı")?,
                        "instructions": text(r, "instructions", 8000).ok_or("Rol talimatı 1-8000 karakter olmalı")?,
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(json!({ "name": name, "description": description, "roles": roles }))
        })
        .collect()
}

fn team_file(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| validate_teams(&v).ok())
        .unwrap_or_default()
}

fn teams(root: &Path) -> Vec<Value> {
    let mut by_name: BTreeMap<String, (usize, Value)> = BTreeMap::new();
    let add = |list: Vec<Value>, source: &str, by_name: &mut BTreeMap<String, (usize, Value)>| {
        for mut t in list {
            t["source"] = json!(source);
            let key = t["name"].as_str().unwrap_or_default().to_lowercase();
            let order = by_name.get(&key).map_or(by_name.len(), |(i, _)| *i);
            by_name.insert(key, (order, t));
        }
    };
    let builtin = serde_json::from_str::<Value>(BUILTIN_TEAMS)
        .ok()
        .and_then(|v| validate_teams(&v).ok())
        .unwrap_or_default();
    add(builtin, "builtin", &mut by_name);
    if let Ok(dir) = user_config_dir() {
        add(team_file(&dir.join("teams.json")), "user", &mut by_name);
    }
    if kor_runtime::is_trusted(root) {
        add(
            team_file(&root.join(".kor").join("teams.json")),
            "project",
            &mut by_name,
        );
    }
    let mut out: Vec<(usize, Value)> = by_name.into_values().collect();
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, t)| t).collect()
}

fn save_teams(v: &Value) -> Result<(), String> {
    let list = validate_teams(v)?;
    let mut names: Vec<String> = list
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_lowercase())
        .collect();
    names.sort();
    names.dedup();
    if names.len() != list.len() {
        return Err("Takım adları benzersiz olmalı".into());
    }
    write_atomic(&user_config_dir()?.join("teams.json"), &Value::Array(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_round_trips_and_rejects_bad_input() {
        let cfg = json!({"transport":"stdio","id":"docs","label":"Docs","enabled":false,
            "command":"npx","args":["-y","x"],"env":{"T":"1"}});
        let prev = json!({"command":"old","timeout":5000});
        let (id, raw) = mcpc::raw_from_config(&cfg, Some(&prev)).unwrap();
        assert_eq!(id, "docs");
        assert_eq!(raw["timeout"], 5000, "unknown fields kept");
        assert_eq!(raw["disabled"], true);
        let view = config_of(&Entry {
            id,
            raw,
            external: None,
        });
        assert_eq!(view["command"], "npx");
        assert_eq!(view["enabled"], false);
        assert_eq!(view["label"], "Docs");

        for bad in [
            json!({"transport":"stdio","id":"-x","command":"a"}),
            json!({"transport":"stdio","id":"ok","command":""}),
            json!({"transport":"http","id":"ok","url":"file:///etc/passwd"}),
            json!({"transport":"ftp","id":"ok"}),
        ] {
            assert!(mcpc::raw_from_config(&bad, None).is_err(), "{bad}");
        }
        let http = json!({"transport":"http","id":"h","enabled":true,"url":"https://x.dev/mcp","headers":{"A":"b"}});
        let (_, raw) = mcpc::raw_from_config(&http, None).unwrap();
        assert!(is_http(&raw) && !disabled(&raw));
    }

    #[test]
    fn install_names_are_safe() {
        assert_eq!(
            install_name("https://github.com/obra/superpowers.git").unwrap(),
            "superpowers"
        );
        assert_eq!(
            install_name("https://github.com/a/My.Repo").unwrap(),
            "my-repo"
        );
        assert!(install_name("http://github.com/a/b").is_err());
        assert!(install_name("https://user:pw@github.com/a/b").is_err());
        assert!(install_name("https://github.com/a/b?x=1").is_err());
        assert!(install_name("https://github.com/").is_err());
    }

    #[test]
    fn clone_progress_is_parsed() {
        assert_eq!(
            last_percent("Receiving objects:  12% (1/8)\rReceiving objects:  45% (4/8)\r"),
            Some(45)
        );
        assert_eq!(last_percent("Cloning into 'x'...\n"), None);
    }

    #[test]
    fn store_results_keep_only_github_https() {
        let text = json!({"items":[
            {"full_name":"a/good","clone_url":"https://github.com/a/good.git","owner":{"login":"a"},"stargazers_count":3},
            {"full_name":"b/evil","clone_url":"https://evil.example/b/evil.git"},
            {"full_name":"a/good","clone_url":"https://github.com/a/good.git"},
            {"full_name":"bad"}
        ]})
        .to_string();
        let out = map_store(&text, &["good".to_owned()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["name"], "good");
        assert_eq!(out[0]["installed"], true);
    }

    #[test]
    fn builtin_teams_validate_and_bad_teams_fail() {
        let v: Value = serde_json::from_str(BUILTIN_TEAMS).unwrap();
        assert_eq!(validate_teams(&v).unwrap().len(), 3);
        assert!(validate_teams(&json!([{"name":"x","description":"d","roles":[]}])).is_err());
        assert!(validate_teams(&json!([{"name":"1 x","description":"d","roles":[{"role":"r","agent":"a","instructions":"i"}]}])).is_err());
    }
}
