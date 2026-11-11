//! `bridge.rs`'nin bilmediği arayüz yöntemleri: kenar çubuğu ve oturum yönetimi, ayarlar, hesap ve model
//! bilgisi, bağlam/sıkıştırma, bellek, izin kuralları, güven, gizlilik, güncelleme/çökme/yapılandırma.
//! Dosya, kod çalıştırma ve proje işleri `api_files.rs`'de. Dönüş şekilleri `frontend/src/desktop/ipc*.ts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use kor_core::{Message, Role, StreamEvent};
use kor_i18n::Localizer;
use kor_providers::ChatRequest;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::meta::{self, Section};
use crate::state::AppState;

pub type Res = Result<Value, String>;

pub(crate) fn s<'a>(args: &'a [Value], i: usize, method: &str) -> Result<&'a str, String> {
    args.get(i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{method}`: {}. bağımsız değişken metin olmalı", i + 1))
}

pub(crate) fn opt_s(args: &[Value], i: usize) -> Option<&str> {
    args.get(i).and_then(Value::as_str)
}

pub(crate) fn b(args: &[Value], i: usize) -> bool {
    args.get(i).and_then(Value::as_bool).unwrap_or(false)
}

fn changed(app: &AppHandle) {
    let _ = app.emit("kor:sessionsChanged", ());
}

fn valid_id(id: &str) -> Result<&str, String> {
    if kor_runtime::valid_session_id(id) {
        Ok(id)
    } else {
        Err("Geçersiz oturum kimliği".into())
    }
}

/// Sohbetlerin çalıştığı klasör: açık klasör, yoksa varsayılan (`Belgeler\Kor`). Ajanın kökü gibi
/// `\\?\` öneksiz; dosya değişiklikleri bu yola göre tutulur.
pub(crate) fn root(state: &AppState) -> Result<PathBuf, String> {
    Ok(kor_tools::without_verbatim(&state.workspace()?))
}

fn localizer(state: &AppState) -> Localizer {
    let cwd = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let lang = crate::bridge::load_config(&cwd)
        .map_or(kor_i18n::Lang::En, |l| l.config.ui.language.resolve());
    Localizer::new(lang)
}

/// Kullanıcı `config.toml` yolu.
/// The user removed an integration: record `key = false` (`integrations.context_menu`,
/// `integrations.os.<id>`) so no later first run installs it again, even without the marker file.
pub(crate) fn remember_off(key: &str) {
    if let Err(e) = user_config().and_then(|f| kor_config::set_scalar(&f, key, "false").map_err(|e| e.to_string())) {
        eprintln!("Kor: {key} kaydedilemedi: {e}");
    }
}

fn user_config() -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("config.toml"))
        .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

/// Kaydetme penceresi; seçilen yol ya da iptalde `None`.
pub(crate) async fn save_dialog(
    app: &AppHandle,
    dir: Option<&Path>,
    name: &str,
) -> Option<PathBuf> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut d = app.dialog().file().set_file_name(name);
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.save_file(move |p| {
        let _ = tx.send(p);
    });
    tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten()
        .and_then(|p| p.into_path().ok())
}

/// Açma penceresi (tek dosya).
pub(crate) async fn open_dialog(app: &AppHandle, dir: Option<&Path>) -> Option<PathBuf> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut d = app.dialog().file();
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.pick_file(move |p| {
        let _ = tx.send(p);
    });
    tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten()
        .and_then(|p| p.into_path().ok())
}

/// Yerel onay penceresi (kod çalıştırma, içe aktarma).
pub(crate) async fn confirm(app: &AppHandle, title: &str, message: &str, ok: &str) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .message(message)
        .title(title)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            ok.to_owned(),
            "Vazgeç".into(),
        ))
        .show(move |ok| {
            let _ = tx.send(ok);
        });
    tauri::async_runtime::spawn_blocking(move || rx.recv().unwrap_or(false))
        .await
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------------------------
// Venice: katalog ve hesap (önbellekli)
// ---------------------------------------------------------------------------------------------

struct Cached {
    at: Instant,
    value: Value,
}

fn cache() -> &'static Mutex<BTreeMap<&'static str, Cached>> {
    static C: OnceLock<Mutex<BTreeMap<&'static str, Cached>>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

/// Venice `/models?type=text` verisi; 10 dakika önbellek. Anahtar yoksa boş.
async fn venice_models(state: &AppState, force: bool) -> Vec<Value> {
    if !force
        && let Some(c) = cache().lock().ok().and_then(|m| {
            m.get("models")
                .filter(|c| c.at.elapsed() < Duration::from_secs(600))
                .map(|c| c.value.clone())
        })
    {
        return c.as_array().cloned().unwrap_or_default();
    }
    let Some(client) = kor_runtime::venice_client(&localizer(state)) else {
        return Vec::new();
    };
    let Ok(v) = client.text_models().await else {
        return Vec::new();
    };
    let data = v["data"].clone();
    if let Ok(mut m) = cache().lock() {
        m.insert(
            "models",
            Cached {
                at: Instant::now(),
                value: data.clone(),
            },
        );
    }
    data.as_array().cloned().unwrap_or_default()
}

fn spec(m: &Value) -> &Value {
    &m["model_spec"]
}

fn price(m: &Value) -> Option<(f64, f64)> {
    let p = &spec(m)["pricing"];
    Some((p["input"]["usd"].as_f64()?, p["output"]["usd"].as_f64()?))
}

/// `CatalogModel` (Venice modeli, `venice/` önekli kimlik).
fn catalog_model(m: &Value) -> Value {
    let sp = spec(m);
    let caps = &sp["capabilities"];
    let flag = |k: &str| caps[k].as_bool().unwrap_or(false);
    let p = &sp["pricing"];
    let price = price(m).map(|(i, o)| {
        json!({ "input": i, "output": o, "cacheInput": p["cache_input"]["usd"], "cacheWrite": Value::Null })
    });
    json!({
        "id": format!("venice/{}", m["id"].as_str().unwrap_or("")),
        "name": sp["name"].as_str().or(m["id"].as_str()).unwrap_or(""),
        "description": sp["description"].as_str().unwrap_or(""),
        "uncensored": sp["traits"].as_array().is_some_and(|t| t.iter().any(|x| x.as_str().is_some_and(|s| s.contains("uncensored"))))
            || m["id"].as_str().is_some_and(|i| i.contains("uncensored")),
        "supportsTools": flag("supportsFunctionCalling"),
        "vision": flag("supportsVision"),
        "reasoning": flag("supportsReasoning"),
        "codeOptimized": flag("optimizedForCode"),
        "webSearch": flag("supportsWebSearch"),
        "contextTokens": sp["availableContextTokens"].as_u64().or(m["context_length"].as_u64()),
        "maxCompletionTokens": sp["maxCompletionTokens"],
        "privacy": sp["privacy"],
        "traits": sp["traits"].as_array().cloned().unwrap_or_default(),
        "beta": sp["beta"].as_bool().unwrap_or(false),
        "deprecatedAt": sp["deprecation"]["date"],
        "price": price,
        "aa": Value::Null,
        "provider": "venice",
    })
}

/// Venice modelinin Artificial Analysis puanları, yalnızca yerel olarak saklanan merkezi katalogdan
/// (`catalog.json`, CI'daki tek AA anahtarıyla üretilir; ADR-0003). AA verisi yoksa `null`.
fn aa_scores(models: &[kor_providers::model_catalog::ModelData], m: &Value) -> Value {
    use kor_providers::model_catalog::{SRC_AA, lookup};
    let id = m["id"].as_str().unwrap_or("");
    let name = spec(m)["name"].as_str().unwrap_or(id);
    let Some(d) = lookup(models, id)
        .or_else(|| lookup(models, name))
        .filter(|d| d.source.values().any(|s| s == SRC_AA))
    else {
        return Value::Null;
    };
    json!({
        "id": d.model_id,
        "name": d.model_id,
        "exact": d.model_id == id || d.model_id.rsplit('/').next() == Some(id),
        "intelligence": d.aa_intelligence_index,
        "coding": d.aa_coding_index,
        "math": d.aa_math_index,
        "tokensPerSecond": d.speed_tps,
        "ttftSeconds": d.ttft_ms.map(|ms| ms / 1000.0),
    })
}

/// `venice/<model>` → (girdi, çıktı) USD / 1M token; Venice kataloğundan (önbellekli).
pub(crate) async fn price_table(state: &AppState) -> BTreeMap<String, (f64, f64)> {
    venice_models(state, false)
        .await
        .iter()
        .filter_map(|m| Some((format!("venice/{}", m["id"].as_str()?), price(m)?)))
        .collect()
}

fn find_model<'a>(models: &'a [Value], id: &str) -> Option<&'a Value> {
    let bare = id.strip_prefix("venice/").unwrap_or(id);
    models.iter().find(|m| m["id"].as_str() == Some(bare))
}

/// `ModelCapabilities` (Venice bilgisi; bilinmeyen model için temkinli varsayılan).
fn capabilities(models: &[Value], id: &str) -> Value {
    match find_model(models, id) {
        Some(m) => {
            let caps = &spec(m)["capabilities"];
            let flag = |k: &str| caps[k].as_bool().unwrap_or(false);
            json!({
                "tools": flag("supportsFunctionCalling"),
                "vision": flag("supportsVision"),
                "pdf": false,
                "audio": flag("supportsAudioInput"),
                "contextTokens": spec(m)["availableContextTokens"],
                "responseSchema": flag("supportsResponseSchema"),
            })
        }
        None => {
            json!({ "tools": "unknown", "vision": false, "pdf": false, "audio": false, "contextTokens": Value::Null })
        }
    }
}

/// Ekin türü ve modele nasıl gideceği (`G22FileSupport`).
fn attachment_plan(path: &str, vision: bool) -> Value {
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let kind = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => "image",
        "pdf" => "pdf",
        "docx" => "docx",
        "xlsx" => "xlsx",
        "mp3" | "wav" | "m4a" | "ogg" | "flac" => "audio",
        _ => "other",
    };
    // Kor ekleri yol olarak verir; ajan `Read` ile okur (görseli yalnızca görsel destekli model görür).
    let plan = match kind {
        "image" if vision => "native",
        "image" | "audio" => "unsupported",
        _ => "text",
    };
    json!({ "path": path, "kind": kind, "plan": plan })
}

// ---------------------------------------------------------------------------------------------
// Model testi (g7RunProbe)
// ---------------------------------------------------------------------------------------------

fn probes() -> &'static Mutex<BTreeMap<String, kor_providers::CancellationToken>> {
    static P: OnceLock<Mutex<BTreeMap<String, kor_providers::CancellationToken>>> = OnceLock::new();
    P.get_or_init(Mutex::default)
}

async fn run_probe(state: &AppState, run_id: &str, model: &str, prompt: &str) -> Res {
    if prompt.trim().is_empty() || prompt.len() > 20_000 {
        return Err("İstem 1-20000 karakter olmalı".into());
    }
    let l = localizer(state);
    let mut notes = Vec::new();
    let (provider, mref, _key) = kor_runtime::build_provider(&l, model, &mut notes)?;
    let cancel = kor_providers::CancellationToken::new();
    if let Ok(mut p) = probes().lock() {
        p.insert(run_id.to_owned(), cancel.clone());
    }
    let start = Instant::now();
    let req = ChatRequest {
        model: mref.model().to_owned(),
        system: None,
        messages: vec![Message::text(Role::User, prompt)],
        tools: Vec::new(),
        max_tokens: Some(1024),
        temperature: None,
        reasoning: None,
    };
    let result = async {
        let mut stream = provider
            .complete(req, cancel.clone())
            .await
            .map_err(|e| e.to_string())?;
        let (mut text, mut first, mut usage, mut finish) = (String::new(), None, None, None);
        while let Some(ev) = stream.next().await {
            match ev.map_err(|e| e.to_string())? {
                StreamEvent::TextDelta { text: t } => {
                    first.get_or_insert_with(|| start.elapsed().as_millis());
                    text.push_str(&t);
                }
                StreamEvent::Usage { usage: u } => usage = Some(u),
                StreamEvent::Finish { reason } => finish = Some(format!("{reason:?}")),
                _ => {}
            }
        }
        Ok::<_, String>((text, first, usage, finish))
    }
    .await;
    if let Ok(mut p) = probes().lock() {
        p.remove(run_id);
    }
    let (text, first, usage, finish) = result?;
    let models = venice_models(state, false).await;
    let cost = match (&usage, find_model(&models, model).and_then(price)) {
        (Some(u), Some((i, o))) => Some((u.input as f64 * i + u.output as f64 * o) / 1_000_000.0),
        _ => None,
    };
    Ok(json!({
        "model": model,
        "text": text,
        "durationMs": start.elapsed().as_millis(),
        "firstTokenMs": first,
        "promptTokens": usage.as_ref().map(|u| u.input),
        "completionTokens": usage.as_ref().map(|u| u.output),
        "costUsd": cost,
        "finishReason": finish,
    }))
}

// ---------------------------------------------------------------------------------------------
// Oturumlar ve kenar çubuğu
// ---------------------------------------------------------------------------------------------

fn session_config(state: &AppState, id: &str) -> Res {
    let m = meta::read()?;
    let own = m.sessions.get(id).cloned().unwrap_or_default();
    let cwd = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let cfg = crate::bridge::load_config(&cwd)?.config;
    let model = own
        .model
        .filter(|x| !x.is_empty())
        .or_else(|| state.model())
        .unwrap_or(cfg.agent.model);
    let mode = own.permission_mode.unwrap_or_else(|| {
        crate::bridge::ui_mode(state.mode().unwrap_or(cfg.permissions.mode)).to_owned()
    });
    Ok(json!({ "model": model, "permissionMode": mode }))
}

fn export_markdown(dir: &Path, id: &str) -> Result<(String, String), String> {
    let transcript = crate::store::open_session(dir, id)?;
    let mut out = String::new();
    let mut title = String::new();
    for item in transcript.as_array().into_iter().flatten() {
        match item["kind"].as_str() {
            Some("user") => {
                let t = item["text"].as_str().unwrap_or("");
                if title.is_empty() {
                    title = t.lines().next().unwrap_or("").chars().take(60).collect();
                }
                out.push_str("## Kullanıcı\n\n");
                out.push_str(t);
                out.push_str("\n\n");
            }
            Some("assistant") => {
                out.push_str("## Kor\n\n");
                out.push_str(item["text"].as_str().unwrap_or(""));
                out.push_str("\n\n");
            }
            Some("tool") => {
                out.push_str(&format!(
                    "> `{}` {}\n\n",
                    item["name"].as_str().unwrap_or(""),
                    item["summary"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(200)
                        .collect::<String>()
                ));
            }
            _ => {}
        }
    }
    if title.is_empty() {
        title = "sohbet".into();
    }
    Ok((format!("# {title}\n\n{out}"), title))
}

fn safe_file_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let n = n.trim().to_owned();
    if n.is_empty() {
        "kor".into()
    } else {
        n.chars().take(60).collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Kullanım geçmişi
// ---------------------------------------------------------------------------------------------

fn day_of(ms: u64) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp_millis(i64::try_from(ms).unwrap_or(0))
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_default()
}

fn usage_history(m: &meta::Meta, models: &[Value], period: &str) -> Value {
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    let (count, step) = if period == "week" { (12, 7) } else { (30, 1) };
    let bucket_start = |d: chrono::NaiveDate| {
        if step == 7 {
            d - chrono::Duration::days(i64::from(d.weekday().num_days_from_monday()))
        } else {
            d
        }
    };
    let last = bucket_start(today);
    let first = last - chrono::Duration::days((count - 1) * step);
    let mut buckets: BTreeMap<chrono::NaiveDate, f64> = (0..count)
        .map(|i| (first + chrono::Duration::days(i * step), 0.0))
        .collect();
    let mut per_model: BTreeMap<String, f64> = BTreeMap::new();
    for u in &m.usage {
        let start = bucket_start(day_of(u.at));
        if start < first {
            continue;
        }
        let usd = find_model(models, &u.model)
            .and_then(price)
            .map_or(0.0, |(i, o)| {
                (u.prompt as f64 * i + u.completion as f64 * o) / 1_000_000.0
            });
        *buckets.entry(start).or_default() += usd;
        *per_model.entry(u.model.clone()).or_default() += usd;
    }
    let total: f64 = buckets.values().sum();
    let mut top: Vec<(String, f64)> = per_model.into_iter().collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    json!({
        "period": period,
        "buckets": buckets.iter().map(|(d, usd)| json!({ "start": d.format("%Y-%m-%d").to_string(), "usd": usd, "diem": 0.0 })).collect::<Vec<_>>(),
        "totalUsd": total,
        "totalDiem": 0.0,
        "topModels": top.into_iter().take(5).map(|(name, usd)| json!({ "name": name, "usd": usd, "diem": 0.0 })).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------------------------
// İzinler, güven, gizlilik
// ---------------------------------------------------------------------------------------------

/// Kullanıcı `config.toml`'undaki izin listesi (yalnız o dosya; proje katmanı karışmaz).
fn user_rules(list: &str) -> Result<Vec<String>, String> {
    let file = user_config()?;
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    Ok(doc
        .get("permissions")
        .and_then(|p| p.get(list))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

fn permission_rules(state: &AppState) -> Res {
    let cwd = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let cfg = crate::bridge::load_config(&cwd)?.config;
    let allow = user_rules("allow")?;
    let deny = user_rules("deny")?;
    let project_deny: Vec<String> = cfg
        .permissions
        .deny
        .iter()
        .filter(|r| !deny.contains(r))
        .cloned()
        .collect();
    let tools: Vec<Value> = kor_tools::default_tools()
        .iter()
        .map(|t| {
            let kind = match t.kind() {
                kor_tools::ToolKind::Read => "read",
                kor_tools::ToolKind::Edit => "edit",
                kor_tools::ToolKind::Execute => "exec",
            };
            json!({ "name": t.rule_name(), "kind": kind })
        })
        .collect();
    Ok(json!({
        "allow": allow,
        "deny": deny,
        "projectDeny": project_deny,
        "defaultMode": crate::bridge::ui_mode(cfg.permissions.mode),
        "tools": tools,
        "userFile": user_config()?.to_string_lossy(),
    }))
}

fn edit_rule(list: &str, rule: &str, add: bool) -> Res {
    if !matches!(list, "allow" | "deny") {
        return Err("Liste allow ya da deny olmalı".into());
    }
    let rule = rule.trim();
    kor_agent::permissions::Rule::parse(rule).map_err(|e| e.to_string())?;
    let mut rules = user_rules(list)?;
    if add {
        if !rules.iter().any(|r| r == rule) {
            rules.push(rule.to_owned());
        }
    } else {
        rules.retain(|r| r != rule);
    }
    kor_config::set_string_list(&user_config()?, &format!("permissions.{list}"), &rules)
        .map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

const PROJECT_FILES: &[&str] = &[
    ".kor/config.toml",
    ".kor/settings.json",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".mcp.json",
    "AGENTS.md",
    "CLAUDE.md",
];

fn trust_info(state: &AppState) -> Res {
    let Some(root) = state.root() else {
        return Ok(Value::Null);
    };
    let trusted = kor_runtime::is_trusted(&root);
    let files: Vec<&str> = PROJECT_FILES
        .iter()
        .copied()
        .filter(|f| root.join(f).exists())
        .collect();
    Ok(json!({
        "root": root.to_string_lossy(),
        "decision": if trusted { json!(true) } else { Value::Null },
        "projectFiles": files,
        "trusted": trusted,
    }))
}

fn dir_stats(dir: &Path, ext: &str) -> (usize, u64) {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .filter_map(|e| e.metadata().ok())
        .fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
}

fn privacy_info(state: &AppState) -> Res {
    let home = kor_runtime::kor_home().unwrap_or_default();
    let sessions = kor_runtime::sessions_dir().unwrap_or_default();
    let (count, _) = dir_stats(&sessions, "jsonl");
    let memory = state
        .root()
        .and_then(|r| kor_runtime::memory_layout(&r))
        .map(|l| l.entry);
    let config_dir = kor_config::user_dir().unwrap_or_default();
    // §6.7 denetim kaydı: etkin dosya ve döndürülmüş arşivler (ADR 0036).
    let audit = kor_runtime::secure::audit_path().unwrap_or_else(|| home.join(kor_agent::audit::FILE));
    let audit_files = kor_agent::audit::files(&audit).unwrap_or_default();
    let audit_bytes: u64 = audit_files
        .iter()
        .filter_map(|f| f.metadata().ok())
        .map(|m| m.len())
        .sum();
    Ok(json!({
        "configDir": config_dir.to_string_lossy(),
        "sessionsDir": sessions.to_string_lossy(),
        "sessionCount": count,
        "memoryFile": memory.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        "memoryExists": memory.as_ref().is_some_and(|p| p.is_file()),
        "auditDir": audit.parent().unwrap_or(&home).to_string_lossy(),
        "auditFiles": audit_files.len(),
        "auditBytes": audit_bytes,
        "settingsFile": meta::path().unwrap_or_default().to_string_lossy(),
        "trustFile": kor_runtime::trust_store().unwrap_or_default().to_string_lossy(),
        "aaCacheFile": kor_platform::cache_dir()
            .map(|d| d.join("model-catalog.json").to_string_lossy().into_owned())
            .unwrap_or_default(),
        "promptFile": meta::path().unwrap_or_default().to_string_lossy(),
        "userDataDir": home.to_string_lossy(),
        // API anahtarları işletim sisteminin anahtarlığında (Windows Kimlik Bilgisi Yöneticisi).
        "keyEncryption": true,
    }))
}

/// Klasördeki dosyaları Geri Dönüşüm Kutusu'na ya da kalıcı olarak siler.
fn clear_dir(dir: &Path, ext: &str, trash: bool) -> Result<(), String> {
    for e in std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let p = e.path();
        if p.extension().is_none_or(|x| x != ext) {
            continue;
        }
        if trash {
            crate::api_files::to_recycle_bin(&p)?;
        } else {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Bellek (MEMORY.md ve g11)
// ---------------------------------------------------------------------------------------------

/// Konu dosyasının arayüz kimliği: `k` + FNV-1a özetinin 8 onaltılık hanesi.
fn record_id(file: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in file.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("k{h:08x}")
}

fn memory_records(root: &Path) -> Vec<(String, Value)> {
    let Some(lay) = kor_runtime::memory_layout(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in std::fs::read_dir(&lay.auto_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") || name == "MEMORY.md" {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        let field = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}: ")))
                .map(str::to_owned)
        };
        let body = text
            .splitn(3, "---")
            .nth(2)
            .unwrap_or(&text)
            .trim()
            .to_owned();
        let kind = match field("type").as_deref() {
            Some("project") => "decision",
            Some("feedback") => "constraint",
            Some("reference") => "term",
            _ => "note",
        };
        out.push((
            name.clone(),
            json!({
                "id": record_id(&name),
                "layer": "project",
                "text": body,
                "kind": kind,
                "subject": field("name").unwrap_or(name),
            }),
        ));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Güncelleme, çökme, yapılandırma
// ---------------------------------------------------------------------------------------------

// Güncelleme (imzalı Tauri updater, `kor update` ile aynı kanal) ve CLI kurulumu: crate::packaging (ADR 0032).

fn logs_dir() -> PathBuf {
    kor_runtime::kor_home().unwrap_or_default().join("logs")
}

/// Ayar dışa aktarma: kullanıcı `config.toml` ve masaüstü tercihleri (oturumlar, kullanım, anahtarlar yok).
fn export_config() -> Result<String, String> {
    let m = meta::read()?;
    let config = std::fs::read_to_string(user_config()?).unwrap_or_default();
    let out = json!({
        "kor": "settings-export",
        "version": 1,
        "configToml": config,
        "desktop": {
            "settings": m.settings,
            "outputStyle": m.output_style,
            "compact": m.compact,
            "systemPrompt": m.system_prompt,
            "sections": m.sections,
        },
    });
    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
}

/// İçe aktarma: yalnızca güvenli anahtarlar uygulanır; izin genişleten kurallar (`allow`) atlanır.
fn plan_import(text: &str) -> Result<(Value, Vec<String>, Vec<String>), String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "Kor ayar dosyası değil".to_owned())?;
    if v["kor"] != "settings-export" {
        return Err("Kor ayar dosyası değil".into());
    }
    let mut applied = Vec::new();
    let mut skipped = Vec::new();
    let d = &v["desktop"];
    for k in ["settings", "outputStyle", "compact", "systemPrompt"] {
        if !d[k].is_null() {
            applied.push(k.to_owned());
        }
    }
    if let Some(cfg) = v["configToml"].as_str() {
        let doc: toml_edit::DocumentMut = cfg
            .parse()
            .map_err(|e: toml_edit::TomlError| e.to_string())?;
        if doc.get("ui").and_then(|u| u.get("language")).is_some() {
            applied.push("ui.language".into());
        }
        if doc.get("agent").and_then(|u| u.get("model")).is_some() {
            applied.push("agent.model".into());
        }
        if doc.get("permissions").and_then(|p| p.get("deny")).is_some() {
            applied.push("permissions.deny".into());
        }
        for k in ["allow", "ask", "mode"] {
            if doc.get("permissions").and_then(|p| p.get(k)).is_some() {
                skipped.push(format!("permissions.{k}"));
            }
        }
        for t in ["mcp", "hooks", "sandbox", "providers"] {
            if doc.get(t).is_some() {
                skipped.push(t.to_owned());
            }
        }
    }
    Ok((v, applied, skipped))
}

fn apply_import(v: &Value, applied: &[String]) -> Result<(), String> {
    let d = &v["desktop"];
    meta::update(|m| {
        if applied.iter().any(|a| a == "settings")
            && let Ok(s) = serde_json::from_value(d["settings"].clone())
        {
            m.settings = s;
        }
        if let Some(st) = d["outputStyle"]
            .as_str()
            .filter(|s| meta::OUTPUT_STYLES.contains(s))
        {
            m.output_style = st.to_owned();
        }
        if let Ok(c) = serde_json::from_value(d["compact"].clone()) {
            m.compact = c;
        }
        if applied.iter().any(|a| a == "systemPrompt") {
            m.system_prompt = d["systemPrompt"].as_str().map(str::to_owned);
        }
        Ok(())
    })?;
    if let Some(cfg) = v["configToml"].as_str() {
        let doc: toml_edit::DocumentMut = cfg
            .parse()
            .map_err(|e: toml_edit::TomlError| e.to_string())?;
        let file = user_config()?;
        if let Some(l) = doc
            .get("ui")
            .and_then(|u| u.get("language"))
            .and_then(|x| x.as_str())
        {
            kor_config::set_string(&file, "ui.language", l).map_err(|e| e.to_string())?;
        }
        if let Some(mo) = doc
            .get("agent")
            .and_then(|u| u.get("model"))
            .and_then(|x| x.as_str())
        {
            kor_config::set_string(&file, "agent.model", mo).map_err(|e| e.to_string())?;
        }
        if let Some(arr) = doc
            .get("permissions")
            .and_then(|p| p.get("deny"))
            .and_then(|x| x.as_array())
        {
            let mut deny = user_rules("deny")?;
            for r in arr.iter().filter_map(|x| x.as_str()) {
                if kor_agent::permissions::Rule::parse(r).is_ok() && !deny.iter().any(|d| d == r) {
                    deny.push(r.to_owned());
                }
            }
            kor_config::set_string_list(&file, "permissions.deny", &deny)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Yönlendirici
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
pub async fn call(method: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let arg = |i: usize| s(args, i, method);
    match method {
        // --- oturumlar ve kenar çubuğu ---
        "pinSession" => {
            let (id, pinned) = (valid_id(arg(0)?)?.to_owned(), b(args, 1));
            meta::update(|m| {
                m.session(&id).pinned = pinned;
                m.tidy(&id);
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        "renameSession" => {
            let id = valid_id(arg(0)?)?.to_owned();
            let title = opt_s(args, 1)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned);
            if title
                .as_ref()
                .is_some_and(|t| t.chars().count() > meta::MAX_TITLE)
            {
                return Err(format!(
                    "Başlık en fazla {} karakter olabilir",
                    meta::MAX_TITLE
                ));
            }
            meta::update(|m| {
                m.session(&id).custom_title = title;
                m.tidy(&id);
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        "setSessionArchived" => {
            let (id, archived) = (valid_id(arg(0)?)?.to_owned(), b(args, 1));
            meta::update(|m| {
                m.session(&id).archived = archived;
                m.tidy(&id);
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        "moveSessionToSection" => {
            let id = valid_id(arg(0)?)?.to_owned();
            let section = opt_s(args, 1).map(str::to_owned);
            meta::update(|m| {
                if let Some(sid) = &section
                    && !m.sections.iter().any(|x| &x.id == sid)
                {
                    return Err("Bölüm bulunamadı".into());
                }
                m.session(&id).section_id = section;
                m.tidy(&id);
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        "listSections" => Ok(json!(meta::read()?.sections)),
        "createSection" => {
            let name = meta::valid_section_name(arg(0)?)?;
            let sec = meta::update(|m| {
                if m.sections.len() >= meta::MAX_SECTIONS {
                    return Err(format!("En fazla {} bölüm olabilir", meta::MAX_SECTIONS));
                }
                let sec = Section {
                    id: meta::new_uuid(),
                    name,
                    created_at: meta::now_ms(),
                };
                m.sections.push(sec.clone());
                Ok(sec)
            })?;
            changed(app);
            Ok(json!(sec))
        }
        "renameSection" => {
            let (id, name) = (arg(0)?.to_owned(), meta::valid_section_name(arg(1)?)?);
            meta::update(|m| {
                let sec = m
                    .sections
                    .iter_mut()
                    .find(|x| x.id == id)
                    .ok_or("Bölüm bulunamadı")?;
                sec.name = name;
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        "deleteSection" => {
            let id = arg(0)?.to_owned();
            meta::update(|m| {
                m.sections.retain(|x| x.id != id);
                let ids: Vec<String> = m
                    .sessions
                    .iter()
                    .filter(|(_, s)| s.section_id.as_deref() == Some(id.as_str()))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in ids {
                    m.session(&k).section_id = None;
                    m.tidy(&k);
                }
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        "getLastSession" => {
            let last = meta::read()?.last_session;
            let dir = kor_runtime::sessions_dir().unwrap_or_default();
            Ok(match last {
                Some(id) if dir.join(format!("{id}.jsonl")).is_file() => json!(id),
                _ => Value::Null,
            })
        }
        "setLastSession" => {
            let id = opt_s(args, 0).map(valid_id).transpose()?.map(str::to_owned);
            meta::update(|m| {
                m.last_session = id;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "getSessionConfig" => session_config(state, valid_id(arg(0)?)?),
        "setSessionConfig" => {
            let id = valid_id(arg(0)?)?.to_owned();
            let cfg = args.get(1).cloned().unwrap_or(Value::Null);
            let model = cfg["model"].as_str().map(str::to_owned);
            if let Some(m) = &model {
                m.parse::<kor_core::ModelRef>()
                    .map_err(|_| format!("Model `sağlayıcı/model` biçiminde olmalı: {m}"))?;
            }
            let mode = cfg["permissionMode"].as_str().map(str::to_owned);
            if let Some(md) = &mode {
                crate::bridge::from_ui_mode(md)?;
            }
            meta::update(|m| {
                let s = m.session(&id);
                if model.is_some() {
                    s.model = model;
                }
                if mode.is_some() {
                    s.permission_mode = mode;
                }
                Ok(())
            })?;
            changed(app);
            Ok(Value::Null)
        }
        // Kor sohbetleri tek süreçte; pencere odaklanınca yeniden kurulacak bir şey yok.
        "activateSession" => Ok(Value::Bool(false)),
        "openSessionWindow" => {
            let id = valid_id(arg(0)?)?;
            let label = format!("session-{id}");
            if let Some(w) = app.get_webview_window(&label) {
                let _ = w.set_focus();
                return Ok(Value::Null);
            }
            tauri::WebviewWindowBuilder::new(
                app,
                &label,
                tauri::WebviewUrl::App(format!("index.html?session={id}").into()),
            )
            .title("Kor")
            .inner_size(1100.0, 800.0)
            .min_inner_size(640.0, 480.0)
            .decorations(false)
            .build()
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "exportSession" => {
            let id = valid_id(arg(0)?)?;
            let dir = kor_runtime::sessions_dir().ok_or("Oturum klasörü yok")?;
            let (md, title) = export_markdown(&dir, id)?;
            let start = state.root();
            match save_dialog(
                app,
                start.as_deref(),
                &format!("{}.md", safe_file_name(&title)),
            )
            .await
            {
                Some(p) => {
                    std::fs::write(&p, md).map_err(|e| e.to_string())?;
                    Ok(json!(p.to_string_lossy()))
                }
                None => Ok(Value::Null),
            }
        }
        "getFileChange" => {
            let (id, path) = (valid_id(arg(0)?)?, arg(1)?);
            Ok(crate::changes::file_change(id, &root(state)?, path).unwrap_or(Value::Null))
        }
        "revertFile" => {
            let (id, path) = (valid_id(arg(0)?)?, arg(1)?);
            crate::changes::revert(id, &root(state)?, path)?;
            Ok(Value::Null)
        }

        // --- hesap ve model ---
        "setApiKey" => {
            let key = arg(0)?.trim();
            if key.is_empty() {
                return Err("Anahtar boş olamaz".into());
            }
            kor_secrets::store_key(
                "venice",
                &kor_secrets::SecretString::from(key.to_owned()),
                &kor_secrets::KeychainStore,
            )
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "getUsage" => {
            let session = match opt_s(args, 0) {
                Some(id) => crate::chat::session_usage(app, id).await,
                None => None,
            }
            .unwrap_or_default();
            let models = venice_models(state, false).await;
            let model = opt_s(args, 0)
                .and_then(|id| meta::read().ok()?.sessions.get(id)?.model.clone())
                .or_else(|| state.model())
                .unwrap_or_default();
            let cost = find_model(&models, &model)
                .and_then(price)
                .map(|(i, o)| (session.input as f64 * i + session.output as f64 * o) / 1_000_000.0);
            let mut out = json!({
                "accountError": Value::Null,
                "tier": Value::Null,
                "balances": Value::Null,
                "nextEpochBegins": Value::Null,
                "keyExpiration": Value::Null,
                "modelLimits": Value::Null,
                "live": Value::Null,
                "session": { "promptTokens": session.input, "completionTokens": session.output, "costUsd": cost },
            });
            match kor_runtime::venice_client(&localizer(state)) {
                None => {
                    out["accountError"] =
                        json!("Venice anahtarı tanımlı değil (kor auth login venice)")
                }
                Some(c) => match c.rate_limits().await {
                    Err(e) => out["accountError"] = json!(e.to_string()),
                    Ok(v) => {
                        let d = &v["data"];
                        out["tier"] = d["apiTier"]["id"].clone();
                        out["balances"] = json!({
                            "usd": d["balances"]["USD"].as_f64().unwrap_or(0.0),
                            "diem": d["balances"]["DIEM"].as_f64().unwrap_or(0.0),
                            "bundled": d["balances"]["BUNDLED_CREDITS"].as_f64().unwrap_or(0.0),
                        });
                        out["nextEpochBegins"] = d["nextEpochBegins"].clone();
                        out["keyExpiration"] = d["keyExpiration"].clone();
                        let bare = model.strip_prefix("venice/").unwrap_or(&model);
                        if let Some(lim) = d["rateLimits"]
                            .as_array()
                            .and_then(|a| a.iter().find(|x| x["apiModelId"] == bare))
                        {
                            let get = |t: &str| {
                                lim["rateLimits"]
                                    .as_array()
                                    .and_then(|a| a.iter().find(|x| x["type"] == t))
                                    .and_then(|x| x["amount"].as_u64())
                            };
                            out["modelLimits"] = json!({ "rpm": get("RPM"), "tpm": get("TPM") });
                        }
                    }
                },
            }
            Ok(out)
        }
        "getModelPricing" => {
            let models = venice_models(state, false).await;
            let map: BTreeMap<String, Value> = models
                .iter()
                .filter_map(|m| {
                    let (i, o) = price(m)?;
                    Some((
                        format!("venice/{}", m["id"].as_str()?),
                        json!({ "input": i, "output": o }),
                    ))
                })
                .collect();
            Ok(json!(map))
        }
        "g2ModelCaps" => {
            let models = venice_models(state, false).await;
            let map: BTreeMap<String, Value> = models
                .iter()
                .filter_map(|m| {
                    let caps = &spec(m)["capabilities"];
                    Some((
                        format!("venice/{}", m["id"].as_str()?),
                        json!({
                            "vision": caps["supportsVision"].as_bool().unwrap_or(false),
                            // Kor akıl yürütme düzeyini modele iletmiyor; seçici gizli kalır.
                            "reasoningEfforts": [],
                            "defaultReasoningEffort": Value::Null,
                        }),
                    ))
                })
                .collect();
            Ok(json!(map))
        }
        "g22Support" => {
            let model = arg(0)?;
            let models = venice_models(state, false).await;
            let caps = capabilities(&models, model);
            let vision = caps["vision"].as_bool().unwrap_or(false);
            let files: Vec<Value> = args
                .get(1)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|p| attachment_plan(p, vision))
                .collect();
            let mode = meta::read()?
                .tool_modes
                .get(model)
                .cloned()
                .unwrap_or_else(|| "auto".into());
            Ok(json!({ "model": model, "caps": caps, "toolMode": mode, "files": files }))
        }
        "g22SetToolMode" => {
            let (model, mode) = (arg(0)?.to_owned(), arg(1)?.to_owned());
            if !matches!(mode.as_str(), "auto" | "native" | "text") {
                return Err("Kor araç modu auto, native ya da text olabilir".into());
            }
            meta::update(|m| {
                if mode == "auto" {
                    m.tool_modes.remove(&model);
                } else {
                    m.tool_modes.insert(model, mode);
                }
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "g7GetCatalog" => {
            let force = b(args, 0);
            let models = venice_models(state, force).await;
            let m = meta::read()?;
            // Puanlar merkezi katalogdan (yerel kopya, 24 saat); uygulama AA'ya hiç istek atmaz.
            let central = crate::ns::catalog::data(state, force).await;
            let stored = central
                .as_ref()
                .map_or(&[][..], |(c, _, _)| c.catalog.models.as_slice());
            let out: Vec<Value> = models
                .iter()
                .map(|v| {
                    let mut j = catalog_model(v);
                    j["aa"] = aa_scores(stored, v);
                    j
                })
                .collect();
            let matched = out.iter().filter(|j| !j["aa"].is_null()).count();
            let aa = match &central {
                Ok((c, from_cache, notices)) => json!({
                    "status": if *from_cache && !notices.is_empty() { "stale" } else { "ok" },
                    "fetchedAt": c.fetched_at.saturating_mul(1000),
                    "message": if notices.is_empty() { Value::Null } else { json!(notices.join("; ")) },
                    "matched": matched,
                }),
                Err(e) => json!({ "status": "error", "fetchedAt": Value::Null, "message": e, "matched": 0 }),
            };
            Ok(json!({ "models": out, "aliases": m.settings.model_aliases, "aa": aa }))
        }
        "g7RunProbe" => run_probe(state, arg(0)?, arg(1)?, arg(2)?).await,
        "g7AbortProbe" => {
            if let Some(c) = probes()
                .lock()
                .ok()
                .and_then(|mut p| p.remove(arg(0).unwrap_or("")))
            {
                c.cancel();
            }
            Ok(Value::Null)
        }
        "g7CheckOnline" => {
            let ok = match kor_runtime::venice_client(&localizer(state)) {
                Some(c) => tokio::time::timeout(Duration::from_secs(8), c.text_models())
                    .await
                    .is_ok_and(|r| r.is_ok()),
                None => false,
            };
            Ok(Value::Bool(ok))
        }

        // --- ayarlar ---
        "g7GetSettings" => {
            let m = meta::read()?;
            let s = m.settings;
            Ok(json!({
                "modelAliases": s.model_aliases,
                "taskModels": s.task_models,
                "theme": s.theme,
                "notifications": s.notifications,
                "git": s.git,
                "editor": s.editor,
                "scripts": s.scripts,
                "profiles": s.profiles,
                "logs": { "keepDays": s.logs_keep_days },
                "app": {
                    "version": app.package_info().version.to_string(),
                    "tauri": tauri::VERSION,
                    "webview": tauri::webview_version().unwrap_or_default(),
                    "webviewName": if cfg!(windows) { "WebView2" } else if cfg!(target_os = "macos") { "WKWebView" } else { "WebKitGTK" },
                    "platform": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                },
            }))
        }
        "g7SetModelAliases" | "g7SetTaskModels" => {
            let map: BTreeMap<String, String> =
                serde_json::from_value(args.first().cloned().unwrap_or(Value::Null))
                    .map_err(|_| "Geçersiz model eşlemesi".to_owned())?;
            for v in map.values() {
                v.parse::<kor_core::ModelRef>()
                    .map_err(|_| format!("Model `sağlayıcı/model` biçiminde olmalı: {v}"))?;
            }
            let aliases = method == "g7SetModelAliases";
            meta::update(|m| {
                if aliases {
                    m.settings.model_aliases = map;
                } else {
                    m.settings.task_models = map;
                }
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "g7SetTheme" => {
            let theme = arg(0)?;
            if !matches!(theme, "system" | "light" | "dark") {
                return Err("Tema system, light ya da dark olmalı".into());
            }
            let t = theme.to_owned();
            meta::update(|m| {
                m.settings.theme = t;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "g7SetNotifications" | "g7SetGit" | "g7SetEditor" | "g7SetScripts" => {
            let v = args
                .first()
                .cloned()
                .filter(Value::is_object)
                .ok_or("Geçersiz ayar")?;
            meta::update(|m| {
                let slot = match method {
                    "g7SetNotifications" => &mut m.settings.notifications,
                    "g7SetGit" => &mut m.settings.git,
                    "g7SetEditor" => &mut m.settings.editor,
                    _ => &mut m.settings.scripts,
                };
                *slot = v;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "g7SetProfiles" => {
            let list = args
                .first()
                .and_then(Value::as_array)
                .cloned()
                .ok_or("Geçersiz profil listesi")?;
            if list.len() > 50 {
                return Err("En fazla 50 profil".into());
            }
            meta::update(|m| {
                m.settings.profiles = list;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "g7SetLogs" => {
            let days = args
                .first()
                .and_then(|v| v["keepDays"].as_u64())
                .ok_or("Geçersiz gün")?;
            let days = u32::try_from(days.clamp(1, 3650)).unwrap_or(30);
            meta::update(|m| {
                m.settings.logs_keep_days = days;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "g3GetOutputStyle" => Ok(json!(meta::read()?.output_style)),
        "g3SetOutputStyle" => {
            let st = arg(0)?;
            if !meta::OUTPUT_STYLES.contains(&st) {
                return Err(format!("Bilinmeyen çıktı stili: {st}"));
            }
            let st = st.to_owned();
            meta::update(|m| {
                m.output_style = st;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "getSystemPrompt" => {
            let cwd = state
                .root()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            let compat = kor_runtime::load_compat(&cwd);
            let shell = kor_tools::Shell::detect();
            let default = kor_agent::prompt::build(
                &cwd,
                shell.name,
                None,
                &compat,
                kor_agent::prompt::Surface::Desktop,
            );
            Ok(json!({ "custom": meta::read()?.system_prompt, "defaultInstructions": default }))
        }
        "setSystemPrompt" => {
            let text = arg(0)?;
            if text.len() > 50_000 {
                return Err("Sistem istemi en fazla 50000 karakter olabilir".into());
            }
            let t = (!text.trim().is_empty()).then(|| text.to_owned());
            meta::update(|m| {
                m.system_prompt = t;
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "getMemory" => {
            let Some(lay) = state.root().and_then(|r| kor_runtime::memory_layout(&r)) else {
                return Ok(json!(""));
            };
            Ok(json!(
                std::fs::read_to_string(&lay.entry).unwrap_or_default()
            ))
        }
        "setMemory" => {
            let text = arg(0)?;
            if text.len() > kor_agent::memory::MAX_ENTRYPOINT_BYTES * 4 {
                return Err("Bellek dizini çok büyük".into());
            }
            let lay = kor_runtime::memory_layout(&root(state)?).ok_or("Bellek klasörü yok")?;
            std::fs::create_dir_all(&lay.auto_dir).map_err(|e| e.to_string())?;
            let tmp = lay.entry.with_extension("md.tmp");
            std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &lay.entry).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        // --- bağlam ve sıkıştırma ---
        "getContextInfo" => Ok(crate::chat::context_info(app, opt_s(args, 0)).await),
        "getCompactSettings" => {
            let c = meta::read()?.compact;
            Ok(json!({ "maxTokens": c.max_tokens, "ratio": c.ratio }))
        }
        "setCompactSettings" => {
            let p = args.first().cloned().unwrap_or(Value::Null);
            if let Some(r) = p["ratio"].as_f64()
                && !(0.3..=0.95).contains(&r)
            {
                return Err("Oran 0.3 ile 0.95 arasında olmalı".into());
            }
            if let Some(t) = p["maxTokens"].as_u64()
                && t < 10_000
            {
                return Err("Tavan en az 10000 token olmalı".into());
            }
            meta::update(|m| {
                if p.get("ratio").is_some() {
                    m.compact.ratio = p["ratio"].as_f64();
                }
                if p.get("maxTokens").is_some() {
                    m.compact.max_tokens = p["maxTokens"].as_u64();
                }
                Ok(())
            })?;
            Ok(Value::Null)
        }
        "compactSession" => crate::chat::compact(app, valid_id(arg(0)?)?).await,
        "editCompactionSummary" => {
            crate::chat::edit_compaction_summary(app, valid_id(arg(0)?)?, arg(1)?)
                .await
                .map(Value::Bool)
        }

        // --- mesaj düzenleme / yeniden üretme ---
        "g2EditMessage" => {
            let id = valid_id(arg(0)?)?.to_owned();
            let p = args.get(1).cloned().unwrap_or(Value::Null);
            let original = p["originalText"].as_str().unwrap_or_default();
            let new_text = p["newText"].as_str().unwrap_or_default();
            if new_text.trim().is_empty() {
                return Err("Mesaj boş olamaz".into());
            }
            let ordinal = p["ordinal"].as_u64().and_then(|n| usize::try_from(n).ok());
            crate::chat::rewind(app, &id, ordinal, original).await?;
            crate::chat::send(
                app,
                crate::chat::SendArgs {
                    session_id: id,
                    text: new_text.to_owned(),
                    attachments: Vec::new(),
                    open_file: None,
                },
            )?;
            Ok(Value::Null)
        }
        "g2Regenerate" => {
            let id = valid_id(arg(0)?)?.to_owned();
            let text = args
                .get(1)
                .and_then(|p| p["text"].as_str())
                .unwrap_or_default()
                .to_owned();
            crate::chat::rewind(app, &id, None, &text).await?;
            crate::chat::send(
                app,
                crate::chat::SendArgs {
                    session_id: id,
                    text,
                    attachments: Vec::new(),
                    open_file: None,
                },
            )?;
            Ok(Value::Null)
        }

        // --- bellek katmanları (g11) ---
        "g11GetOverview" => {
            let m = meta::read()?;
            let project = match state.root() {
                Some(r) => {
                    let records: Vec<Value> =
                        memory_records(&r).into_iter().map(|(_, v)| v).collect();
                    json!({ "root": r.to_string_lossy(), "trusted": kor_runtime::is_trusted(&r), "records": records })
                }
                None => Value::Null,
            };
            Ok(json!({
                "session": [],
                "project": project,
                "global": [],
                // Kor bellek kayıtları serbest metin; çakışma tespiti yapılmıyor.
                "conflicts": [],
                "confidenceScore": m.confidence_score,
            }))
        }
        "g11Forget" => {
            let id = arg(1)?;
            let r = root(state)?;
            let (file, _) = memory_records(&r)
                .into_iter()
                .find(|(_, v)| v["id"] == id)
                .ok_or("Kayıt bulunamadı")?;
            let lay = kor_runtime::memory_layout(&r).ok_or("Bellek klasörü yok")?;
            kor_agent::memory::forget_memory(&lay, &file).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "g11KeepBoth" => Ok(Value::Null),
        "g11SetConfidence" => {
            let on = b(args, 0);
            meta::update(|m| {
                m.confidence_score = on;
                Ok(())
            })?;
            Ok(Value::Null)
        }

        // --- izinler, güven, gizlilik ---
        "g7GetPermissionRules" => permission_rules(state),
        "g7AddPermissionRule" => edit_rule(arg(0)?, arg(1)?, true),
        "g7RemovePermissionRule" => edit_rule(arg(0)?, arg(1)?, false),
        "g7GetTrust" => trust_info(state),
        "g7SetTrust" => {
            let r = root(state)?;
            let store = kor_runtime::trust_store().ok_or("Güven deposu yok")?;
            kor_compat::trust::set(&store, &r, b(args, 0)).map_err(|e| e.to_string())?;
            // Open chats pick up the new trust (AGENTS.md, project agents, permission mode) on their next turn.
            state.chats.reload_idle_under(&r);
            Ok(Value::Null)
        }
        "g7GetPrivacyInfo" => privacy_info(state),
        "g7OpenFolder" => {
            let dir = match arg(0)? {
                "config" => kor_config::user_dir().unwrap_or_default(),
                "sessions" => kor_runtime::sessions_dir().unwrap_or_default(),
                "audit" | "userData" => kor_runtime::kor_home().unwrap_or_default(),
                other => return Err(format!("Bilinmeyen klasör: {other}")),
            };
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            app.opener()
                .open_path(dir.to_string_lossy(), None::<&str>)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "g7ClearData" => {
            let trash = arg(1)? == "trash";
            match arg(0)? {
                "sessions" => {
                    for id in state.chats.running_ids() {
                        state.chats.abort(&id);
                    }
                    let dir = kor_runtime::sessions_dir().ok_or("Oturum klasörü yok")?;
                    clear_dir(&dir, "jsonl", trash)?;
                    meta::update(|m| {
                        m.sessions.clear();
                        m.last_session = None;
                        Ok(())
                    })?;
                    changed(app);
                }
                // Kor ayrı denetim günlüğü tutmuyor: silinecek bir şey yok.
                "audit" => {}
                other => return Err(format!("Bilinmeyen hedef: {other}")),
            }
            Ok(Value::Null)
        }
        "g7GetUsageHistory" => {
            let period = if arg(0)? == "week" { "week" } else { "day" };
            let models = venice_models(state, false).await;
            Ok(usage_history(&meta::read()?, &models, period))
        }

        // --- güvenlik paneli (g12) ---
        "g12GetSandbox" => crate::ns::g12_sandbox(state, None),
        "g12SetSandbox" => crate::ns::g12_sandbox(state, Some(b(args, 0))),
        "g12GetToolScopes" => Ok(crate::chat::tool_scopes(app, valid_id(arg(0)?)?)
            .await
            .unwrap_or(Value::Null)),
        "g12SetToolScope" => {
            crate::chat::set_tool_scope(app, valid_id(arg(0)?)?, arg(1)?, arg(2)?).await
        }
        // Kor toplu silmeyi ayrıca sormaz (Delete aracı ve komutlar zaten izin ister).
        "g12AnswerBulkDelete" => Ok(Value::Null),

        // --- güncelleme, çökme, yapılandırma (g16) ---
        "g16UpdateStatus" => Ok(crate::packaging::status(app, state)),
        "g16UpdateCheck" => Ok(crate::packaging::check(app, state).await),
        // İmzalı paket: indir → (onayla) kur ve yeniden başlat; imzasız derleme / .deb / .rpm: sürüm sayfası.
        "g16UpdateInstall" => crate::packaging::install(app, state).await,
        // Paketle gelen `kor` CLI'ı PATH'e ekle / kaldır (yalnızca arayüzdeki açık onayla).
        "g16CliStatus" => Ok(crate::packaging::cli_status()),
        "g16CliInstall" => crate::packaging::cli_install(),
        "g16CliRemove" => crate::packaging::cli_remove(),
        "g16CrashList" => {
            let dumps: Vec<Value> = std::fs::read_dir(logs_dir())
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
                    Some(json!({ "name": e.file_name().to_string_lossy(), "size": m.len(), "mtimeMs": mtime }))
                })
                .collect();
            Ok(json!({ "events": [], "dumps": dumps, "uploadHost": Value::Null }))
        }
        "g16CrashExport" => {
            let report = format!(
                "Kor tanılama raporu\nsürüm: {}\nişletim sistemi: {} {}\nwebview: {}\nayar dosyası: {}\noturum klasörü: {}\n",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                tauri::webview_version().unwrap_or_default(),
                user_config()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                kor_runtime::sessions_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            );
            match save_dialog(app, None, "kor-tanilama.txt").await {
                Some(p) => {
                    std::fs::write(&p, report).map_err(|e| e.to_string())?;
                    Ok(json!(p.to_string_lossy()))
                }
                None => Ok(Value::Null),
            }
        }
        // Kor hiçbir yere rapor göndermez (sıfır telemetri).
        "g16CrashSend" => Ok(Value::Bool(false)),
        "g16CrashOpenFolder" => {
            let dir = logs_dir();
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            app.opener()
                .open_path(dir.to_string_lossy(), None::<&str>)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "g16ConfigExport" => {
            let text = export_config()?;
            match save_dialog(app, None, "kor-ayarlar.json").await {
                Some(p) => {
                    std::fs::write(&p, text).map_err(|e| e.to_string())?;
                    Ok(json!(p.to_string_lossy()))
                }
                None => Ok(Value::Null),
            }
        }
        "g16ConfigImport" => {
            let Some(p) = open_dialog(app, None).await else {
                return Ok(Value::Null);
            };
            let meta_len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if meta_len > 2 * 1024 * 1024 {
                return Err("Dosya çok büyük".into());
            }
            let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let (v, applied, skipped) = plan_import(&text)?;
            let msg = format!(
                "Uygulanacak: {}\nGüvenlik nedeniyle atlanacak: {}",
                if applied.is_empty() {
                    "-".into()
                } else {
                    applied.join(", ")
                },
                if skipped.is_empty() {
                    "-".into()
                } else {
                    skipped.join(", ")
                }
            );
            if !confirm(app, "Ayarları içe aktar", &msg, "İçe aktar").await {
                return Ok(Value::Null);
            }
            apply_import(&v, &applied)?;
            Ok(json!({
                "applied": applied,
                "skipped": skipped,
                "systemPrompt": v["desktop"]["systemPrompt"].is_string(),
                "outputStyle": v["desktop"]["outputStyle"].is_string(),
            }))
        }

        _ => crate::api_files::call(method, args, app, state).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn venice(id: &str, vision: bool) -> Value {
        json!({ "id": id, "model_spec": { "name": "N", "pricing": { "input": { "usd": 2.0 }, "output": { "usd": 10.0 } },
            "availableContextTokens": 1000, "capabilities": { "supportsVision": vision, "supportsFunctionCalling": true } } })
    }

    #[test]
    fn catalog_and_caps_come_from_venice_specs() {
        let models = vec![venice("a", true)];
        let c = catalog_model(&models[0]);
        assert_eq!(c["id"], "venice/a");
        assert_eq!(c["price"]["output"], 10.0);
        assert_eq!(capabilities(&models, "venice/a")["vision"], true);
        assert_eq!(capabilities(&models, "venice/zzz")["tools"], "unknown");
        assert_eq!(attachment_plan("x.png", false)["plan"], "unsupported");
        assert_eq!(attachment_plan("x.PNG", true)["plan"], "native");
        assert_eq!(attachment_plan("doc.pdf", false)["kind"], "pdf");
    }

    #[test]
    fn usage_history_prices_tokens() {
        let mut m = meta::Meta::default();
        m.record_usage(meta::UsageEntry {
            at: meta::now_ms(),
            model: "venice/a".into(),
            prompt: 1_000_000,
            completion: 100_000,
        });
        let h = usage_history(&m, &[venice("a", false)], "day");
        assert_eq!(h["buckets"].as_array().unwrap().len(), 30);
        assert!((h["totalUsd"].as_f64().unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(h["topModels"][0]["name"], "venice/a");
    }

    #[test]
    fn import_skips_rules_that_widen_permissions() {
        let file = json!({ "kor": "settings-export", "version": 1,
            "configToml": "[permissions]\nallow = [\"Bash\"]\ndeny = [\"Bash(rm *)\"]\n[ui]\nlanguage = \"en\"\n",
            "desktop": { "outputStyle": "concise", "settings": null } });
        let (_, applied, skipped) = plan_import(&file.to_string()).unwrap();
        assert!(
            applied.contains(&"permissions.deny".to_owned())
                && applied.contains(&"ui.language".to_owned())
        );
        assert!(skipped.contains(&"permissions.allow".to_owned()));
        assert!(plan_import("{}").is_err());
    }

    #[test]
    fn record_ids_are_stable() {
        assert_eq!(record_id("a.md"), record_id("a.md"));
        assert_ne!(record_id("a.md"), record_id("b.md"));
        assert!(record_id("a.md").starts_with('k') && record_id("a.md").len() == 9);
    }
}
