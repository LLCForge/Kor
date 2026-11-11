//! Ad alanları: `g9` (sağlayıcılar, anahtar profilleri, uç noktalar, sağlık, maliyet, yerel Ollama), `g35` (Venice
//! web hesabı girişi: Kor yalnızca resmi API anahtarını kullanır, reddedilir), `preview` (dev sunucusu) ve
//! `abrowser` (ajan tarayıcısı paneli). Sözleşmeler: `ipc-g9.ts`, `ipc-g35.ts`, `ipc-preview.ts`, `ipc-browser.ts`.
//!
//! Sağlayıcılar Kor'un bildirimlerinden (`kor_runtime::registry`) gelir; etkin anahtar `kor auth login <id>` ile aynı
//! anahtarlık hesabındadır (`provider:<id>`). Ek anahtar profilleri ayrı anahtarlık hesaplarında durur ve "etkin"
//! seçilince o hesaba kopyalanır. Gömülü web görünümleri (önizleme, ajan tarayıcısı) Tauri alt webview'leridir
//! (`tauri` `unstable` özelliği); güvenlik notları "Gömülü web görünümleri" bölümünde.

use kor_platform::NoWindow as _;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use kor_providers::{CancellationToken, Category, ProviderManifest, Registry};
use kor_secrets::{ExposeSecret, KeychainStore, SecretStore, SecretString};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, WebviewUrl};
use tauri_plugin_opener::OpenerExt;

use crate::api::{Res, opt_s, s};
use crate::state::AppState;

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    match ns {
        "g9" => g9(name, args, app).await,
        "g35" => g35(name),
        "preview" => preview(name, args, app, state).await,
        "abrowser" => abrowser(name, args, app).await,
        _ => None,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Klasör oluşturulamadı: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("Dosya yazılamadı: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Dosya yazılamadı: {e}"))
}

fn is_loopback(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    )
}

// =============================================================================================
// g9: sağlayıcılar
// =============================================================================================

const MAX_PROFILES: usize = 20;
/// Yalnızca `kor auth login` ile kaydedilmiş (profil kaydı olmayan) anahtarın sanal profil kimliği öneki.
const CLI_PREFIX: &str = "cli-";
/// Bu dosyanın yazdığı uç nokta bildirimlerinin ilk satırı; elle yazılmış bildirimlere dokunulmaz.
const OVERRIDE_MARKER: &str = "# kor-desktop: uç nokta ayarı (Ayarlar > Sağlayıcılar)";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Store {
    profiles: Vec<Profile>,
    /// `G9Settings` (uç noktalar hariç; onlar sağlayıcı bildirimlerinde).
    settings: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String,
    provider: String,
    label: String,
    hint: String,
    created_at: u64,
}

fn store_path() -> Result<PathBuf, String> {
    kor_runtime::kor_home()
        .map(|d| d.join("desktop-g9.json"))
        .ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

fn store_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(Mutex::default)
}

fn load_store() -> Result<Store, String> {
    let path = store_path()?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| format!("Sağlayıcı ayar dosyası bozuk: {}", path.to_string_lossy())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(format!("Dosya okunamadı: {e}")),
    }
}

/// Oku-değiştir-yaz tek kilit altında; dosya atomik yazılır.
fn update_store<T>(f: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
    let _g = lock(store_lock());
    let mut st = load_store()?;
    let out = f(&mut st)?;
    let bytes = serde_json::to_vec_pretty(&st).map_err(|e| e.to_string())?;
    write_atomic(&store_path()?, &bytes)?;
    Ok(out)
}

fn registry() -> Result<Registry, String> {
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    kor_runtime::registry(&l, &mut Vec::new())
}

fn manifest(reg: &Registry, id: &str) -> Result<ProviderManifest, String> {
    reg.get(id)
        .cloned()
        .ok_or_else(|| format!("Bilinmeyen sağlayıcı: {id}"))
}

fn main_account(provider: &str) -> Result<String, String> {
    kor_secrets::account_for(provider).map_err(|e| e.to_string())
}

fn profile_account(provider: &str, profile: &str) -> String {
    format!("desktop-profile:{provider}:{profile}")
}

fn ks_get(account: &str) -> Result<Option<SecretString>, String> {
    KeychainStore.get(account).map_err(|e| e.to_string())
}

fn ks_set(account: &str, secret: &SecretString) -> Result<(), String> {
    KeychainStore
        .set(account, secret)
        .map_err(|e| e.to_string())
}

fn ks_delete(account: &str) -> Result<(), String> {
    KeychainStore
        .delete(account)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Anahtarın kendisi asla dönmez: kısa anahtar tamamen gizlenir, değilse yalnızca son 4 karakter.
fn hint(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() < 12 {
        return "••••".into();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("…{tail}")
}

fn valid_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn same(a: &SecretString, b: &SecretString) -> bool {
    a.expose_secret() == b.expose_secret()
}

/// Etkin hesaptaki anahtar hiçbir profile ait değilse (ör. `kor auth login` ile girildi) onu profile alır;
/// böylece etkin anahtar değiştirilince eskisi kaybolmaz.
fn adopt_orphan(st: &mut Store, provider: &str) -> Result<(), String> {
    let Some(main) = ks_get(&main_account(provider)?)? else {
        return Ok(());
    };
    for p in st.profiles.iter().filter(|p| p.provider == provider) {
        if ks_get(&profile_account(provider, &p.id))?.is_some_and(|k| same(&k, &main)) {
            return Ok(());
        }
    }
    let id = crate::meta::new_uuid();
    ks_set(&profile_account(provider, &id), &main)?;
    st.profiles.push(Profile {
        id,
        provider: provider.to_owned(),
        label: "kor auth login".into(),
        hint: hint(main.expose_secret()),
        created_at: crate::meta::now_ms(),
    });
    Ok(())
}

/// Failover havuzunun yedek yuvaları (`provider:<id>#2…`, §4.5): (yuva, anahtar).
fn backup_slots(provider: &str) -> Result<Vec<(usize, SecretString)>, String> {
    let mut out = Vec::new();
    for slot in 2..=kor_secrets::MAX_POOL_KEYS {
        let acct = kor_secrets::pool_account(provider, slot).map_err(|e| e.to_string())?;
        if let Some(k) = ks_get(&acct)? {
            out.push((slot, k));
        }
    }
    Ok(out)
}

/// Profil anahtarını failover havuzuna koy ya da havuzdan çıkar; çekirdek (`build_pool`) 429/5xx/
/// zaman aşımında sıradaki anahtara kendisi geçer.
fn set_backup(provider: &str, key: &SecretString, on: bool) -> Res {
    let slots = backup_slots(provider)?;
    let present: Vec<usize> = slots
        .iter()
        .filter(|(_, k)| same(k, key))
        .map(|(s, _)| *s)
        .collect();
    if on {
        if present.is_empty() {
            // Yuva 1 etkin anahtardır; yedekler 2'den başlar.
            let free = (2..=kor_secrets::MAX_POOL_KEYS)
                .find(|s| !slots.iter().any(|(u, _)| u == s))
                .ok_or("Yedek anahtar yuvalarının hepsi dolu")?;
            let acct = kor_secrets::pool_account(provider, free).map_err(|e| e.to_string())?;
            ks_set(&acct, key)?;
        }
    } else {
        for slot in present {
            ks_delete(&kor_secrets::pool_account(provider, slot).map_err(|e| e.to_string())?)?;
        }
    }
    Ok(Value::Null)
}

fn profiles_view(st: &Store, provider: &str) -> Result<Vec<Value>, String> {
    let main = ks_get(&main_account(provider)?)?;
    let backups = backup_slots(provider)?;
    let mut out = Vec::new();
    let mut matched = false;
    for p in st.profiles.iter().filter(|p| p.provider == provider) {
        let key = ks_get(&profile_account(provider, &p.id))?;
        let active = !matched
            && match (&main, &key) {
                (Some(m), Some(k)) => same(k, m),
                _ => false,
            };
        matched |= active;
        let backup = key
            .as_ref()
            .is_some_and(|k| backups.iter().any(|(_, b)| same(b, k)));
        out.push(json!({
            "id": p.id, "label": p.label, "hint": p.hint, "backup": backup,
            "active": active, "createdAt": p.created_at,
        }));
    }
    if let Some(m) = main
        && !matched
    {
        out.insert(
            0,
            json!({
                "id": format!("{CLI_PREFIX}{provider}"), "label": "kor auth login",
                "hint": hint(m.expose_secret()), "backup": false, "active": true, "createdAt": 0,
            }),
        );
    }
    Ok(out)
}

fn ollama_enabled(st: &Store) -> bool {
    st.settings
        .get("ollamaEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn usable(m: &ProviderManifest, st: &Store) -> bool {
    if !m.needs_key() {
        return m.adapter != kor_providers::AdapterKind::Ollama || ollama_enabled(st);
    }
    matches!(kor_runtime::key_for(m), Ok(Some(_)))
}

fn env_set(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| !v.trim().is_empty())
}

// --- uç nokta bildirimleri ------------------------------------------------------------------

fn override_path(id: &str) -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("providers").join(format!("{id}.toml")))
        .ok_or_else(|| "Kullanıcı ayar klasörü bulunamadı".into())
}

/// `Some(true)`: bu dosyanın yazdığı bildirim var; `Some(false)`: elle yazılmış bildirim var; `None`: dosya yok.
fn override_state(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.starts_with(OVERRIDE_MARKER))
}

fn endpoint_view(m: &ProviderManifest, builtin: &Registry) -> Value {
    let ours = override_path(&m.id)
        .ok()
        .and_then(|p| override_state(&p))
        .unwrap_or(false);
    let Some(base) = builtin.get(&m.id).filter(|_| ours) else {
        return json!({});
    };
    let mut out = Map::new();
    if m.base_url != base.base_url {
        out.insert("baseURL".into(), json!(m.base_url));
    }
    let headers: Map<String, Value> = m
        .quirks
        .extra_headers
        .iter()
        .filter(|(k, v)| base.quirks.extra_headers.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    if !headers.is_empty() {
        out.insert("headers".into(), Value::Object(headers));
    }
    Value::Object(out)
}

const FORBIDDEN_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
    "host",
    "content-length",
    "content-type",
    "connection",
    "transfer-encoding",
    "te",
    "upgrade",
    "anthropic-version",
];

fn validate_base_url(raw: &str) -> Result<String, String> {
    let bad = || {
        "Temel URL https olmalı; http yalnızca bu makine (localhost) için kabul edilir".to_owned()
    };
    if raw.chars().count() > 300 {
        return Err("Temel URL en fazla 300 karakter olmalı".into());
    }
    let url = Url::parse(raw).map_err(|_| "Geçerli bir URL girin".to_owned())?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&url) => {}
        _ => return Err(bad()),
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Temel URL kullanıcı adı, parola, sorgu veya # içeremez".into());
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

/// `ProviderEndpoint` → (temel URL, başlıklar). Kor'da sağlayıcı başına zaman aşımı/eşzamanlılık yok.
fn parse_endpoint(v: &Value) -> Result<(Option<String>, BTreeMap<String, String>), String> {
    let o = v.as_object().ok_or("Geçersiz uç nokta ayarı")?;
    let mut base = None;
    let mut headers = BTreeMap::new();
    for (k, val) in o {
        match k.as_str() {
            "baseURL" => {
                base = Some(validate_base_url(
                    val.as_str().ok_or("Geçerli bir URL girin")?,
                )?)
            }
            "headers" => {
                let h = val.as_object().ok_or("Geçersiz uç nokta ayarı")?;
                if h.len() > 10 {
                    return Err("En fazla 10 başlık".into());
                }
                for (name, value) in h {
                    if name.is_empty()
                        || name.len() > 64
                        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    {
                        return Err("Başlık adı yalnızca harf, rakam ve - içerebilir".into());
                    }
                    if FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                        return Err("Bu başlık ayarlanamaz (kimlik veya taşıma başlığı)".into());
                    }
                    let value = value.as_str().ok_or("Geçersiz uç nokta ayarı")?;
                    if value.chars().count() > 500 || value.contains(['\r', '\n']) {
                        return Err(
                            "Başlık değeri en fazla 500 karakter olmalı ve satır sonu içeremez"
                                .into(),
                        );
                    }
                    headers.insert(name.clone(), value.to_owned());
                }
            }
            "timeoutMs" | "maxConcurrent" => {
                return Err("Kor'da sağlayıcı başına zaman aşımı ve eşzamanlılık sınırı yok; yalnızca temel URL ve başlıklar ayarlanabilir.".into());
            }
            _ => return Err("Geçersiz uç nokta ayarı".into()),
        }
    }
    Ok((base, headers))
}

fn toml_value(v: &Value) -> Option<toml_edit::Value> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64()?.into(),
        },
        Value::String(s) => s.as_str().into(),
        Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for x in a.iter().filter_map(toml_value) {
                arr.push(x);
            }
            arr.into()
        }
        Value::Object(o) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, x) in o {
                if let Some(x) = toml_value(x) {
                    t.insert(k, x);
                }
            }
            t.into()
        }
    })
}

fn toml_table(o: &Map<String, Value>) -> toml_edit::Table {
    let mut t = toml_edit::Table::new();
    for (k, v) in o {
        match v {
            Value::Object(inner) => {
                t.insert(k, toml_edit::Item::Table(toml_table(inner)));
            }
            other => {
                if let Some(x) = toml_value(other) {
                    t.insert(k, toml_edit::Item::Value(x));
                }
            }
        }
    }
    t
}

/// Bildirimi TOML'a çevirir ve yazmadan önce Kor'un kendi ayrıştırıcısıyla doğrular.
fn manifest_toml(m: &ProviderManifest) -> Result<String, String> {
    let v = serde_json::to_value(m).map_err(|e| e.to_string())?;
    let obj = v.as_object().ok_or("Geçersiz bildirim")?;
    let doc = toml_edit::DocumentMut::from(toml_table(obj));
    let text = format!("{OVERRIDE_MARKER}\n{doc}");
    ProviderManifest::parse(&text, "kor-desktop").map_err(|e| e.message)?;
    Ok(text)
}

fn set_endpoint(provider: &str, endpoint: Option<&Value>) -> Res {
    let builtin = Registry::builtin();
    let Some(base) = builtin.get(provider) else {
        return Err(
            "Bu sağlayıcı bir bildirim dosyasıyla tanımlı; uç noktayı o dosyada değiştirin.".into(),
        );
    };
    let path = override_path(provider)?;
    if override_state(&path) == Some(false) {
        return Err(format!(
            "Bu sağlayıcı için elle yazılmış bir bildirim var; onu düzenleyin: {}",
            path.to_string_lossy()
        ));
    }
    let Some(ep) = endpoint.filter(|v| !v.is_null()) else {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("Dosya silinemedi: {e}"))?;
        }
        return Ok(Value::Null);
    };
    let (url, headers) = parse_endpoint(ep)?;
    let mut m = base.clone();
    if let Some(u) = url {
        m.base_url = u;
    }
    m.quirks.extra_headers.extend(headers);
    let text = manifest_toml(&m)?;
    write_atomic(&path, text.as_bytes())?;
    // Aynı kimliği tanımlayan başka bir dosya sonra yükleniyorsa ayar etkisiz kalır: söyle.
    if registry()?.get(provider).map(|x| &x.base_url) != Some(&m.base_url) {
        return Err("Ayar yazıldı ama aynı sağlayıcıyı tanımlayan başka bir bildirim dosyası onu geçersiz kılıyor.".into());
    }
    Ok(Value::Null)
}

// --- ayarlar ----------------------------------------------------------------------------------

fn num_in(v: &Value, lo: f64, hi: f64) -> bool {
    v.as_f64().is_some_and(|x| (lo..=hi).contains(&x))
}

fn int_in(v: &Value, lo: u64, hi: u64) -> bool {
    v.as_u64().is_some_and(|x| (lo..=hi).contains(&x))
}

fn preset_name_ok(s: &str) -> bool {
    let n = s.chars().count();
    (1..=40).contains(&n) && s.chars().all(|c| c.is_alphanumeric() || " _.-".contains(c))
}

fn model_id_ok(s: &str) -> bool {
    (1..=200).contains(&s.chars().count()) && !s.chars().any(char::is_control)
}

/// `G9SettingsPatch` doğrulaması (ayar dosyası şemasının Rust karşılığı). Kor'un uygulamadığı güvenceler
/// (maliyet tavanı, yanıt önbelleği) açılamaz: kullanıcı korunduğunu sanmasın.
fn validate_patch(patch: &Map<String, Value>) -> Result<(), String> {
    let bad = |k: &str| format!("Geçersiz ayar: {k}");
    for (k, v) in patch {
        let ok = match k.as_str() {
            "ollamaEnabled" | "lowCostMode" => v.is_boolean(),
            "costCapUsd" => {
                if !v.is_null() {
                    return Err("Kor görev maliyet tavanını uygulamıyor; tavan ayarlanamaz.".into());
                }
                true
            }
            "responseCache" => {
                let o = v.as_object().ok_or_else(|| bad(k))?;
                if o.get("enabled").and_then(Value::as_bool) == Some(true) {
                    return Err("Kor'da yerel yanıt önbelleği yok; açılamaz.".into());
                }
                o.iter().all(|(f, x)| match f.as_str() {
                    "enabled" => x.is_boolean(),
                    "ttlHours" => int_in(x, 1, 720),
                    "exclude" => x.as_array().is_some_and(|a| {
                        a.len() <= 50
                            && a.iter().all(|s| {
                                s.as_str()
                                    .is_some_and(|s| (2..=100).contains(&s.chars().count()))
                            })
                    }),
                    _ => false,
                })
            }
            "presets" => v.as_object().is_some_and(|o| {
                o.len() <= 30
                    && o.iter().all(|(name, p)| {
                        preset_name_ok(name)
                            && p.as_object().is_some_and(|p| {
                                p.iter().all(|(f, x)| match f.as_str() {
                                    "temperature" => num_in(x, 0.0, 2.0),
                                    "top_p" => num_in(x, 0.0, 1.0),
                                    "maxTokens" => int_in(x, 1, 1_000_000),
                                    _ => false,
                                })
                            })
                    })
            }),
            "modelPresets" => v.as_object().is_some_and(|o| {
                o.len() <= 300
                    && o.iter()
                        .all(|(m, p)| model_id_ok(m) && p.as_str().is_some_and(preset_name_ok))
            }),
            "prices" => v.as_object().is_some_and(|o| {
                o.len() <= 500
                    && o.iter().all(|(m, p)| {
                        model_id_ok(m)
                            && p.as_object().is_some_and(|p| {
                                p.get("input").is_some()
                                    && p.get("output").is_some()
                                    && p.iter().all(|(f, x)| match f.as_str() {
                                        "input" | "output" | "cacheInput" => {
                                            num_in(x, 0.0, 10_000.0)
                                        }
                                        "contextTokens" => int_in(x, 1, 20_000_000),
                                        _ => false,
                                    })
                            })
                    })
            }),
            _ => false,
        };
        if !ok {
            return Err(bad(k));
        }
    }
    Ok(())
}

// --- sağlık ve test ---------------------------------------------------------------------------

/// `kor provider status`/`test` ile aynı çekirdek denetimi (`kor_providers::probe::check`);
/// sonuç da aynı sağlık kaydına (`health`) düşer.
async fn probe(m: &ProviderManifest, key: Option<SecretString>) -> Value {
    if let Some(d) = kor_platform::cache_dir() {
        kor_providers::health::set_store(d.join("provider-health.json"));
    }
    let c = match kor_providers::build(m, key) {
        Ok(p) => kor_providers::probe::check(p.as_ref()).await,
        Err(e) => kor_providers::probe::Check {
            provider: m.id.clone(),
            ok: false,
            latency_ms: 0,
            models: 0,
            error: Some(e),
        },
    };
    let (status, message) = match &c.error {
        None => (Some(200), "Erişilebilir".to_owned()),
        Some(e) => {
            use kor_core::ProviderErrorKind as K;
            let msg = match e.kind {
                K::AuthFailed => "Kimlik doğrulanamadı".to_owned(),
                K::RateLimited => "Hız sınırına takıldı".to_owned(),
                K::Timeout => "Erişilemiyor: zaman aşımı".to_owned(),
                _ => format!("Erişilemiyor: {}", e.message),
            };
            (e.http_status, msg)
        }
    };
    json!({
        "provider": m.id,
        "ok": c.ok,
        "status": status,
        "latencyMs": c.ok.then_some(c.latency_ms),
        "message": message,
    })
}

/// Profil kimliği → (sağlayıcı, anahtar).
fn profile_secret(st: &Store, profile_id: &str) -> Result<(String, SecretString), String> {
    if !valid_profile_id(profile_id) {
        return Err("Geçersiz profil kimliği".into());
    }
    if let Some(provider) = profile_id.strip_prefix(CLI_PREFIX) {
        let k = ks_get(&main_account(provider)?)?.ok_or("Anahtar bulunamadı")?;
        return Ok((provider.to_owned(), k));
    }
    let p = st
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or("Profil bulunamadı")?;
    let k = ks_get(&profile_account(&p.provider, &p.id))?.ok_or("Anahtar bulunamadı")?;
    Ok((p.provider.clone(), k))
}

// --- maliyet ----------------------------------------------------------------------------------

/// Oturum maliyeti (USD). OpenAI uyumlu sağlayıcılar önbellekten gelen token'ı girdinin içinde sayar,
/// Anthropic ayrı sayar.
fn session_usd(u: &kor_core::Usage, price: &Value, cached_separate: bool) -> f64 {
    let input = price["input"].as_f64().unwrap_or(0.0);
    let output = price["output"].as_f64().unwrap_or(0.0);
    let cache = price["cacheInput"].as_f64().unwrap_or(input);
    let uncached = if cached_separate {
        u.input
    } else {
        u.input.saturating_sub(u.cache_read)
    };
    (uncached as f64 * input + u.cache_read as f64 * cache + u.output as f64 * output) / 1e6
}

type Timed = Option<(Instant, Vec<Value>)>;

/// Venice metin modelleri (fiyat için); 10 dakika önbellek, anahtar yoksa boş.
async fn venice_models() -> Vec<Value> {
    static C: OnceLock<Mutex<Timed>> = OnceLock::new();
    let cache = C.get_or_init(Mutex::default);
    if let Some((at, v)) = lock(cache).as_ref()
        && at.elapsed() < Duration::from_secs(600)
    {
        return v.clone();
    }
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let Some(client) = kor_runtime::venice_client(&l) else {
        return Vec::new();
    };
    let Ok(v) = client.text_models().await else {
        return Vec::new();
    };
    let data = v["data"].as_array().cloned().unwrap_or_default();
    *lock(cache) = Some((Instant::now(), data.clone()));
    data
}

async fn model_price(st: &Store, model: &str) -> Option<Value> {
    if let Some(p) = st
        .settings
        .get("prices")
        .and_then(|p| p.get(model))
        .and_then(Value::as_object)
    {
        return Some(json!({
            "input": p.get("input"), "output": p.get("output"),
            "cacheInput": p.get("cacheInput").cloned().unwrap_or(Value::Null), "cacheWrite": Value::Null,
        }));
    }
    let bare = model.strip_prefix("venice/")?;
    let models = venice_models().await;
    let m = models.iter().find(|m| m["id"].as_str() == Some(bare))?;
    let p = &m["model_spec"]["pricing"];
    Some(json!({
        "input": p["input"]["usd"].as_f64()?, "output": p["output"]["usd"].as_f64()?,
        "cacheInput": p["cache_input"]["usd"], "cacheWrite": Value::Null,
    }))
}

// --- Ollama -----------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Pull {
    status: String,
    completed: Option<u64>,
    total: Option<u64>,
    error: Option<String>,
    done: bool,
    cancel: CancellationToken,
}

fn pulls() -> &'static Mutex<BTreeMap<String, Pull>> {
    static P: OnceLock<Mutex<BTreeMap<String, Pull>>> = OnceLock::new();
    P.get_or_init(Mutex::default)
}

fn pull_json(name: &str, p: &Pull) -> Value {
    json!({
        "name": name, "status": p.status, "completed": p.completed, "total": p.total,
        "error": p.error, "done": p.done,
    })
}

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        kor_core::net::client_builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default()
    })
}

fn valid_ollama_name(name: &str) -> bool {
    let mut it = name.chars();
    name.len() <= 200
        && it.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && it.all(|c| c.is_ascii_alphanumeric() || "._-/:".contains(c))
        && !name.contains("..")
}

/// Yerel Ollama kökü (`local` bildiriminin adresi); uzak sunucuda yönetim reddedilir.
fn ollama_root() -> Result<String, String> {
    let reg = registry()?;
    let m = reg
        .iter()
        .find(|m| m.adapter == kor_providers::AdapterKind::Ollama)
        .ok_or("Ollama sağlayıcısı tanımlı değil")?;
    let url = Url::parse(&m.base_url).map_err(|e| e.to_string())?;
    if !is_loopback(&url) {
        return Err(
            "Yerel model yönetimi yalnızca bu makinedeki (localhost) Ollama için kullanılabilir."
                .into(),
        );
    }
    Ok(url.origin().ascii_serialization())
}

async fn ollama_req(
    path: &str,
    body: Option<Value>,
    timeout: Option<Duration>,
) -> Result<reqwest::Response, String> {
    let url = format!("{}{path}", ollama_root()?);
    let mut rb = match &body {
        Some(b) => http()
            .post(url)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(b).map_err(|e| e.to_string())?),
        None => http().get(url),
    };
    if let Some(t) = timeout {
        rb = rb.timeout(t);
    }
    let res = rb
        .send()
        .await
        .map_err(|_| "Ollama'ya bağlanılamadı; çalıştığından emin olun.".to_owned())?;
    if !res.status().is_success() {
        let code = res.status().as_u16();
        let text: String = res
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect();
        return Err(format!("Ollama isteği başarısız (HTTP {code}): {text}"));
    }
    Ok(res)
}

async fn ollama_json(path: &str) -> Result<Value, String> {
    let res = ollama_req(path, None, Some(Duration::from_secs(15))).await?;
    let text = res.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|_| "Ollama yanıtı okunamadı".to_owned())
}

fn ollama_view(tags: &Value, ps: &Value) -> Value {
    let arr = |v: &Value| v["models"].as_array().cloned().unwrap_or_default();
    let models: Vec<Value> = arr(tags)
        .iter()
        .take(1000)
        .map(|m| {
            let d = &m["details"];
            json!({
                "name": m["name"].as_str().unwrap_or(""), "sizeBytes": m["size"].as_u64().unwrap_or(0),
                "family": d["family"].as_str(), "parameterSize": d["parameter_size"].as_str(),
                "quantization": d["quantization_level"].as_str(),
            })
        })
        .collect();
    let running: Vec<Value> = arr(ps)
        .iter()
        .take(1000)
        .map(|m| {
            json!({
                "name": m["name"].as_str().unwrap_or(""), "sizeBytes": m["size"].as_u64().unwrap_or(0),
                "vramBytes": m["size_vram"].as_u64().unwrap_or(0), "expiresAt": m["expires_at"].as_str(),
            })
        })
        .collect();
    json!({ "models": models, "running": running })
}

type Progress = (String, Option<u64>, Option<u64>);

/// NDJSON ilerleme satırı → (durum, tamamlanan, toplam) ya da Ollama hatası.
fn pull_line(line: &str) -> Result<Option<Progress>, String> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(line).map_err(|_| "Ollama yanıtı okunamadı".to_owned())?;
    if let Some(e) = v["error"].as_str() {
        return Err(format!(
            "Ollama: {}",
            e.chars().take(2000).collect::<String>()
        ));
    }
    Ok(Some((
        v["status"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(500)
            .collect(),
        v["completed"].as_u64(),
        v["total"].as_u64(),
    )))
}

async fn run_pull(app: AppHandle, name: String, cancel: CancellationToken) {
    let set = |f: &dyn Fn(&mut Pull)| {
        let mut g = lock(pulls());
        if let Some(p) = g.get_mut(&name) {
            f(p);
            let _ = app.emit("kor:g9/pullProgress", pull_json(&name, p));
        }
    };
    let result: Result<(), String> = async {
        let mut res = tokio::select! {
            r = ollama_req("/api/pull", Some(json!({ "model": name, "stream": true })), None) => r?,
            () = cancel.cancelled() => return Err("İndirme durduruldu".into()),
        };
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let chunk = tokio::select! {
                c = res.chunk() => c.map_err(|e| e.to_string())?,
                () = cancel.cancelled() => return Err("İndirme durduruldu".into()),
            };
            let Some(chunk) = chunk else { break };
            buf.extend_from_slice(&chunk);
            while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=i).collect();
                if let Some((status, completed, total)) =
                    pull_line(&String::from_utf8_lossy(&line))?
                {
                    set(&|p| {
                        p.status.clone_from(&status);
                        p.completed = completed;
                        p.total = total;
                    });
                }
            }
            if buf.len() > 1_000_000 {
                return Err("Ollama yanıtı okunamadı".into());
            }
        }
        if let Some((status, completed, total)) = pull_line(&String::from_utf8_lossy(&buf))? {
            set(&|p| {
                p.status.clone_from(&status);
                p.completed = completed;
                p.total = total;
            });
        }
        Ok(())
    }
    .await;
    set(&|p| {
        p.done = true;
        p.error = result.clone().err();
    });
}

// --- g9 yönlendirici --------------------------------------------------------------------------

async fn g9(name: &str, args: &[Value], app: &AppHandle) -> Option<Res> {
    let m = |n: &str| format!("g9.{n}");
    Some(match name {
        "getProviders" => get_providers(),
        "addKey" => (|| {
            let provider = s(args, 0, &m(name))?;
            let label = s(args, 1, &m(name))?.trim();
            let key = s(args, 2, &m(name))?.trim();
            let backup = crate::api::b(args, 3);
            add_key(provider, label, key, backup)
        })(),
        "removeKey" => match s(args, 0, &m(name)) {
            Ok(id) => remove_key(id),
            Err(e) => Err(e),
        },
        "setActiveKey" => (|| {
            let provider = s(args, 0, &m(name))?;
            let id = s(args, 1, &m(name))?;
            set_active(provider, id)
        })(),
        "setKeyBackup" => match s(args, 0, &m(name))
            .and_then(|id| Ok((load_store()?, id)))
            .and_then(|(st, id)| profile_secret(&st, id))
        {
            Ok((provider, key)) => set_backup(&provider, &key, crate::api::b(args, 1)),
            Err(e) => Err(e),
        },
        "testKey" => match s(args, 0, &m(name))
            .and_then(|id| Ok((load_store()?, id)))
            .and_then(|(st, id)| profile_secret(&st, id))
            .and_then(|(p, k)| Ok((manifest(&registry()?, &p)?, k)))
        {
            Ok((mf, k)) => Ok(probe(&mf, Some(k)).await),
            Err(e) => Err(e),
        },
        "setEndpoint" => match s(args, 0, &m(name)) {
            Ok(p) => set_endpoint(p, args.get(1)),
            Err(e) => Err(e),
        },
        "saveSettings" => (|| {
            let patch = args
                .first()
                .and_then(Value::as_object)
                .ok_or("Geçersiz ayar")?
                .clone();
            if serde_json::to_vec(&patch).map_or(true, |b| b.len() > 256 * 1024) {
                return Err("Ayar çok büyük".to_owned());
            }
            validate_patch(&patch)?;
            update_store(|st| {
                for (k, v) in patch {
                    if v.is_null() {
                        st.settings.remove(&k);
                    } else {
                        st.settings.insert(k, v);
                    }
                }
                Ok(Value::Null)
            })
        })(),
        "health" => health().await,
        "costInfo" => cost_info(args, app).await,
        // Kor'da yanıt önbelleği yok; temizlenecek bir şey de yok.
        "clearResponseCache" => Ok(Value::Null),
        "ollamaList" => {
            let (tags, ps) = futures::join!(ollama_json("/api/tags"), ollama_json("/api/ps"));
            let pulls: Vec<Value> = lock(pulls()).iter().map(|(n, p)| pull_json(n, p)).collect();
            match (tags, ps) {
                (Ok(t), Ok(p)) => {
                    let mut v = ollama_view(&t, &p);
                    v["pulls"] = Value::Array(pulls);
                    Ok(v)
                }
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        "ollamaPull" => (|| {
            let n = s(args, 0, &m(name))?.trim().to_owned();
            if !valid_ollama_name(&n) {
                return Err(format!("Geçersiz model adı: {n}"));
            }
            ollama_root()?;
            let cancel = CancellationToken::new();
            {
                let mut g = lock(pulls());
                if g.get(&n).is_some_and(|p| !p.done) {
                    return Err("Bu model zaten indiriliyor".into());
                }
                // Bitmiş kayıtlar sınırsız birikmesin.
                if g.len() >= 20 {
                    g.retain(|_, p| !p.done);
                }
                g.insert(
                    n.clone(),
                    Pull {
                        status: "başlıyor".into(),
                        completed: None,
                        total: None,
                        error: None,
                        done: false,
                        cancel: cancel.clone(),
                    },
                );
            }
            tauri::async_runtime::spawn(run_pull(app.clone(), n, cancel));
            Ok(Value::Null)
        })(),
        "ollamaCancelPull" => match s(args, 0, &m(name)) {
            Ok(n) => {
                if let Some(p) = lock(pulls()).get(n) {
                    p.cancel.cancel();
                }
                Ok(Value::Null)
            }
            Err(e) => Err(e),
        },
        "ollamaUnload" => match s(args, 0, &m(name)) {
            Ok(n) if valid_ollama_name(n) => ollama_req(
                "/api/generate",
                Some(json!({ "model": n, "keep_alive": 0 })),
                Some(Duration::from_secs(15)),
            )
            .await
            .map(|_| Value::Null),
            Ok(n) => Err(format!("Geçersiz model adı: {n}")),
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

fn get_providers() -> Res {
    let reg = registry()?;
    let builtin = Registry::builtin();
    let st = load_store()?;
    let mut providers = Vec::new();
    let mut endpoints = Map::new();
    for m in reg.iter() {
        let endpoint = endpoint_view(m, &builtin);
        if endpoint.as_object().is_some_and(|o| !o.is_empty()) {
            endpoints.insert(m.id.clone(), endpoint.clone());
        }
        let profiles = if m.needs_key() {
            profiles_view(&st, &m.id)?
        } else {
            Vec::new()
        };
        providers.push(json!({
            "id": m.id,
            "label": m.name,
            "defaultBaseURL": builtin.get(&m.id).map_or(&m.base_url, |b| &b.base_url),
            "baseURL": m.base_url,
            "keyless": !m.needs_key(),
            "uncensored": m.category == Category::Uncensored,
            "usable": usable(m, &st),
            "profiles": profiles,
            "endpoint": endpoint,
            "catalogError": Value::Null,
        }));
    }
    let mut settings = st.settings.clone();
    settings.insert("endpoints".into(), Value::Object(endpoints));
    let env_venice = reg.get("venice").is_some_and(|m| env_set(&m.key_env()));
    Ok(json!({
        "providers": providers,
        "persisted": true,
        "envVenice": env_venice,
        "settings": settings,
        "responseCache": { "enabled": false, "entries": 0 },
    }))
}

fn add_key(provider: &str, label: &str, key: &str, backup: bool) -> Res {
    let m = manifest(&registry()?, provider)?;
    if !m.needs_key() {
        return Err("Bu sağlayıcı anahtar gerektirmiyor".into());
    }
    let n = label.chars().count();
    if !(1..=60).contains(&n) || label.chars().any(char::is_control) {
        return Err("Profil adı 1-60 karakter olmalı".into());
    }
    if key.len() < 8
        || key.len() > kor_secrets::MAX_SECRET_LEN
        || key.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("Geçersiz API anahtarı".into());
    }
    let secret = SecretString::from(key.to_owned());
    update_store(|st| {
        if st
            .profiles
            .iter()
            .filter(|p| p.provider == provider)
            .count()
            >= MAX_PROFILES
        {
            return Err(format!(
                "Sağlayıcı başına en fazla {MAX_PROFILES} anahtar profili"
            ));
        }
        adopt_orphan(st, provider)?;
        let id = crate::meta::new_uuid();
        ks_set(&profile_account(provider, &id), &secret)?;
        let has_main = ks_get(&main_account(provider)?)?.is_some();
        // "Yedek" işaretli profil etkin yapılmaz, failover havuzuna girer (429/5xx/zaman aşımında
        // çekirdek ona geçer); ilk anahtar her zaman etkin olur.
        if !backup || !has_main {
            kor_secrets::store_key(provider, &secret, &KeychainStore).map_err(|e| e.to_string())?;
        } else {
            set_backup(provider, &secret, true)?;
        }
        st.profiles.push(Profile {
            id,
            provider: provider.to_owned(),
            label: label.to_owned(),
            hint: hint(key),
            created_at: crate::meta::now_ms(),
        });
        Ok(Value::Null)
    })
}

fn remove_key(profile_id: &str) -> Res {
    if !valid_profile_id(profile_id) {
        return Err("Geçersiz profil kimliği".into());
    }
    if let Some(provider) = profile_id.strip_prefix(CLI_PREFIX) {
        ks_delete(&main_account(provider)?)?;
        return Ok(Value::Null);
    }
    update_store(|st| {
        let idx = st
            .profiles
            .iter()
            .position(|p| p.id == profile_id)
            .ok_or("Profil bulunamadı")?;
        let p = st.profiles.remove(idx);
        let acct = profile_account(&p.provider, &p.id);
        let main_acct = main_account(&p.provider)?;
        let key = ks_get(&acct)?;
        let was_active = match (&key, ks_get(&main_acct)?) {
            (Some(a), Some(b)) => same(a, &b),
            _ => false,
        };
        if let Some(k) = &key {
            set_backup(&p.provider, k, false)?;
        }
        ks_delete(&acct)?;
        if was_active {
            ks_delete(&main_acct)?;
            // Sıradaki profil etkin olur; yoksa sağlayıcının anahtarı kalmaz.
            if let Some(next) = st.profiles.iter().find(|x| x.provider == p.provider)
                && let Some(k) = ks_get(&profile_account(&p.provider, &next.id))?
            {
                ks_set(&main_acct, &k)?;
            }
        }
        Ok(Value::Null)
    })
}

fn set_active(provider: &str, profile_id: &str) -> Res {
    if !valid_profile_id(profile_id) {
        return Err("Geçersiz profil kimliği".into());
    }
    if profile_id == format!("{CLI_PREFIX}{provider}") {
        return Ok(Value::Null);
    }
    update_store(|st| {
        if !st
            .profiles
            .iter()
            .any(|p| p.id == profile_id && p.provider == provider)
        {
            return Err("Profil bulunamadı".into());
        }
        adopt_orphan(st, provider)?;
        let k = ks_get(&profile_account(provider, profile_id))?.ok_or("Anahtar bulunamadı")?;
        ks_set(&main_account(provider)?, &k)?;
        Ok(Value::Null)
    })
}

async fn health() -> Res {
    let reg = registry()?;
    let st = load_store()?;
    let jobs: Vec<_> = reg
        .iter()
        .filter(|m| usable(m, &st))
        .map(|m| {
            let key = kor_runtime::key_for(m).ok().flatten().map(|(k, _)| k);
            probe(m, key)
        })
        .collect();
    Ok(Value::Array(futures::future::join_all(jobs).await))
}

async fn cost_info(args: &[Value], app: &AppHandle) -> Res {
    let session = opt_s(args, 0);
    if let Some(id) = session
        && !kor_runtime::valid_session_id(id)
    {
        return Err("Geçersiz oturum kimliği".into());
    }
    let model = s(args, 1, "g9.costInfo")?;
    if !model_id_ok(model) {
        return Err("Geçersiz model kimliği".into());
    }
    let st = load_store()?;
    let price = model_price(&st, model).await;
    let usage = match session {
        Some(id) => crate::chat::session_usage(app, id)
            .await
            .unwrap_or_default(),
        None => kor_core::Usage::default(),
    };
    let context = crate::chat::context_info(app, session).await["usedTokens"]
        .as_u64()
        .unwrap_or(0);
    let used = usage.input > 0 || usage.output > 0;
    let usd = price.as_ref().map_or(0.0, |p| {
        session_usd(&usage, p, model.starts_with("anthropic/"))
    });
    Ok(json!({
        "price": price,
        "contextTokens": context,
        "promptTokens": usage.input,
        "cachedTokens": usage.cache_read,
        "sessionUsd": usd,
        "unpriced": price.is_none() && used,
        "capUsd": Value::Null,
    }))
}

// =============================================================================================
// g35: Venice web hesabı (reddedilir)
// =============================================================================================

const G35_DECLINED: &str = "Kor, Venice web hesabıyla (e-posta + parola) giriş yapmaz; resmi Venice API anahtarını kullanır. Anahtarı Ayarlar > Sağlayıcılar bölümünden ekleyin veya `kor auth login venice` çalıştırın.";

fn g35(name: &str) -> Option<Res> {
    let no_session = || {
        json!({
            "hasSession": false, "ready": false, "email": Value::Null, "savedAt": Value::Null,
            "pendingSecondFactor": false, "pendingHint": Value::Null, "pendingTarget": Value::Null,
            "transport": "api-key", "lastEmail": Value::Null, "lastError": Value::Null,
        })
    };
    Some(match name {
        "status" | "restore" | "logout" => Ok(no_session()),
        // Parola ve kod bağımsız değişkenlerine hiç dokunulmaz.
        "login" | "submitCode" => Err(G35_DECLINED.into()),
        _ => return None,
    })
}

// =============================================================================================
// preview: dev sunucusu (gömülü görünüm yok)
// =============================================================================================

const MAX_LOG: usize = 200;

#[derive(Debug, Default)]
struct Dev {
    running: bool,
    pid: Option<u32>,
    url: Option<String>,
    exit_code: Option<i32>,
    log: VecDeque<String>,
    /// Her başlatmada artar; eski sürecin çıkışı yeni çalışmanın durumunu ezmesin.
    run: u64,
}

fn dev() -> &'static Mutex<Dev> {
    static D: OnceLock<Mutex<Dev>> = OnceLock::new();
    D.get_or_init(Mutex::default)
}

fn dev_command() -> Option<String> {
    crate::meta::read()
        .ok()?
        .settings
        .scripts
        .get("dev")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
}

fn dev_status() -> Value {
    let d = lock(dev());
    json!({
        "running": d.running, "command": dev_command(), "url": d.url, "pid": d.pid,
        "exitCode": d.exit_code, "log": d.log,
    })
}

fn emit_dev(app: &AppHandle) {
    let _ = app.emit(
        "kor:preview/event",
        json!({ "type": "dev", "status": dev_status() }),
    );
}

/// Komut satırını argv'ye böler (kabuk yok): boşluk ayırır, "..." ve '...' tırnakları tek parça tutar.
fn split_args(cmd: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in cmd.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err("Dev betiğinde kapanmamış tırnak var".into());
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn ansi_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("static pattern"))
}

/// Dev sunucusu çıktısındaki ilk yerel adres (0.0.0.0 / [::] → localhost).
fn local_url(line: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(r#"https?://(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\])(?::\d{1,5})?(?:/[^\s"'<>]*)?"#)
            .expect("static pattern")
    });
    let found = re.find(line)?.as_str();
    Some(
        found
            .replacen("0.0.0.0", "localhost", 1)
            .replacen("[::]", "localhost", 1)
            .trim_end_matches(['.', ',', ')'])
            .to_owned(),
    )
}

/// Windows'ta `npm`, `npx` gibi komutlar `.cmd`; `PATH` + `PATHEXT` ile kabuğun yaptığı gibi bulunur.
fn resolve_program(name: &str, root: &Path) -> PathBuf {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return root.join(p);
    }
    if !cfg!(windows) || p.extension().is_some() {
        return p.to_owned();
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let c = dir.join(format!("{name}{}", ext.to_lowercase()));
            if c.is_file() {
                return c;
            }
        }
    }
    p.to_owned()
}

fn push_log(app: &AppHandle, run: u64, raw: &[u8]) {
    let text = String::from_utf8_lossy(raw);
    let clean = ansi_re().replace_all(text.trim_end(), "");
    let line: String = clean.chars().take(2000).collect();
    {
        let mut d = lock(dev());
        if d.run != run {
            return;
        }
        if d.url.is_none() {
            d.url = local_url(&line);
        }
        d.log.push_back(line);
        while d.log.len() > MAX_LOG {
            d.log.pop_front();
        }
    }
    emit_dev(app);
}

async fn read_lines(app: AppHandle, run: u64, r: impl tokio::io::AsyncRead + Unpin) {
    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => push_log(&app, run, &buf),
        }
    }
}

async fn dev_start(app: &AppHandle, state: &AppState) -> Res {
    if lock(dev()).running {
        return Ok(dev_status());
    }
    let cmd =
        dev_command().ok_or("Dev sunucu betiği tanımlı değil (Ayarlar > Genel > Betikler).")?;
    if cmd.chars().count() > 2000 {
        return Err("Dev betiği çok uzun".into());
    }
    let root = crate::api::root(state)?;
    if !kor_runtime::is_trusted(&root) {
        return Err(
            "Bu klasör güvenilir değil; dev betiği yalnızca güvenilen klasörde çalışır.".into(),
        );
    }
    let argv = split_args(&cmd)?;
    let (prog, rest) = argv.split_first().ok_or("Dev betiği boş")?;
    let mut c = tokio::process::Command::new(resolve_program(prog, &root));
    c.args(rest)
        .current_dir(&root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    c.no_window();
    #[cfg(unix)]
    c.process_group(0); // durdururken tüm süreç grubu öldürülür
    let mut child = c
        .spawn()
        .map_err(|e| format!("Dev betiği başlatılamadı: {e}"))?;
    let run = {
        let mut d = lock(dev());
        d.run += 1;
        d.running = true;
        d.pid = child.id();
        d.url = None;
        d.exit_code = None;
        d.log.clear();
        d.run
    };
    if let Some(out) = child.stdout.take() {
        tauri::async_runtime::spawn(read_lines(app.clone(), run, out));
    }
    if let Some(err) = child.stderr.take() {
        tauri::async_runtime::spawn(read_lines(app.clone(), run, err));
    }
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        let code = child.wait().await.ok().and_then(|s| s.code());
        {
            let mut d = lock(dev());
            if d.run != run {
                return;
            }
            d.running = false;
            d.pid = None;
            d.exit_code = code;
        }
        emit_dev(&a);
    });
    emit_dev(app);
    Ok(dev_status())
}

/// Süreç ağacını öldürür: Windows'ta `taskkill /T`, Unix'te süreç grubu.
fn kill_tree(pid: u32) {
    kor_platform::kill_tree(pid);
}

/// Uygulama kapanırken çağrılır: çalışan dev sunucusu (süreç ağacıyla) durdurulur.
pub fn shutdown() {
    let pid = lock(dev()).pid;
    if let Some(pid) = pid {
        kill_tree(pid);
    }
}

async fn dev_stop(app: &AppHandle) -> Res {
    let pid = lock(dev()).pid;
    if let Some(pid) = pid {
        let _ = tauri::async_runtime::spawn_blocking(move || kill_tree(pid)).await;
        for _ in 0..30 {
            if !lock(dev()).running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    emit_dev(app);
    Ok(dev_status())
}

async fn preview(name: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Option<Res> {
    Some(match name {
        "devStart" => dev_start(app, state).await,
        "devStop" => dev_stop(app).await,
        "devStatus" => Ok(dev_status()),
        "openExternal" => (|| {
            let current = lock(pv()).url.clone();
            let url = current
                .filter(|u| !u.is_empty())
                .or_else(|| lock(dev()).url.clone())
                .ok_or("Açık önizleme adresi yok")?;
            let parsed = Url::parse(&url).map_err(|_| "Geçersiz adres".to_owned())?;
            if !preview_url_ok(&parsed) {
                return Err("Geçersiz adres".to_owned());
            }
            app.opener()
                .open_url(parsed.as_str(), None::<&str>)
                .map(|()| Value::Null)
                .map_err(|e| e.to_string())
        })(),
        "show" => preview_show(app, opt_s(args, 0), args.get(1)).await,
        "hide" => {
            lock(pv()).visible = false;
            sync_preview(app)
        }
        "setBounds" => match parse_bounds(args.first()) {
            Ok(b) => {
                lock(pv()).bounds = Some(b);
                sync_preview(app)
            }
            Err(e) => Err(e),
        },
        "navigate" => match s(args, 0, "preview.navigate").and_then(|r| parse_input(r, "http")) {
            Ok(u) if preview_url_ok(&u) => match app.get_webview(PREVIEW_LABEL) {
                Some(wv) => wv.navigate(u).map(|()| Value::Null).map_err(err_s),
                None => preview_show(app, Some(u.as_str()), None).await,
            },
            Ok(_) => Err(PREVIEW_ONLY_LOCAL.into()),
            Err(e) => Err(e),
        },
        "reload" => with_view(app, PREVIEW_LABEL, |wv| wv.reload()),
        "back" => {
            lock(pv()).nav.back_pending = true;
            with_view(app, PREVIEW_LABEL, |wv| wv.eval("history.back()"))
        }
        "setInspect" => set_inspect(app, crate::api::b(args, 0)),
        _ => return None,
    })
}

// =============================================================================================
// Gömülü web görünümleri (Tauri alt webview'leri)
// =============================================================================================
//
// Güvenlik: alt webview'ler yalnızca http(s) yükler (`on_navigation` her gezinmeyi denetler); `*.localhost`
// (Windows'ta uygulamanın kendi kökeni `http://tauri.localhost`) engellenir, böylece bir sayfa uygulama
// arayüzünü yerel köken olarak açıp `kor_call` çağıramaz. Yetkiler (`capabilities/default.json`) yalnızca `main`
// webview'ine verilir. Pencere açma isteği reddedilir (ajan tarayıcısında yeni sekme olur), indirme reddedilir,
// oturum verisi saklanmaz (gizli mod). Konum renderer'ın CSS pikselleri = mantıksal pikseller (yakınlaştırma yok).

const PREVIEW_LABEL: &str = "kor-preview";
const TAB_PREFIX: &str = "kor-abrowser-";
const MAX_TABS: usize = 20;
const PREVIEW_ONLY_LOCAL: &str = "Önizleme yalnızca bu makinedeki adresleri (localhost) açar.";
const BLOCKED_URL: &str = "Bu adres açılamaz: yalnızca http(s) sayfalarına izin var.";

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn parse_bounds(v: Option<&Value>) -> Result<Bounds, String> {
    let bad = || "Geçersiz görünüm sınırları".to_owned();
    let o = v.and_then(Value::as_object).ok_or_else(bad)?;
    let f = |k: &str| {
        o.get(k)
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && x.abs() <= 100_000.0)
    };
    match (f("x"), f("y"), f("width"), f("height")) {
        (Some(x), Some(y), Some(w), Some(h)) if w >= 0.0 && h >= 0.0 => Ok(Bounds { x, y, w, h }),
        _ => Err(bad()),
    }
}

fn err_s(e: tauri::Error) -> String {
    e.to_string()
}

/// Uygulamanın kendi kökenleri (`tauri.localhost`, `ipc.localhost`, `asset.localhost`…) hiçbir zaman yüklenmez.
fn app_host(u: &Url) -> bool {
    u.host_str().is_some_and(|h| h.ends_with(".localhost"))
}

fn preview_url_ok(u: &Url) -> bool {
    matches!(u.scheme(), "http" | "https") && is_loopback(u) && !app_host(u)
}

fn browser_url_ok(u: &Url) -> bool {
    u.as_str() == "about:blank"
        || (matches!(u.scheme(), "http" | "https") && u.host_str().is_some() && !app_host(u))
}

/// Adres çubuğu girdisi: şemasız ise `default_scheme://` eklenir.
fn parse_input(raw: &str, default_scheme: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 4096 {
        return Err("Geçersiz adres".into());
    }
    let full = if raw == "about:blank" || raw.contains("://") {
        raw.to_owned()
    } else {
        format!("{default_scheme}://{raw}")
    };
    Url::parse(&full).map_err(|_| "Geçersiz adres".to_owned())
}

fn place(wv: &tauri::Webview, b: Bounds) -> Result<(), String> {
    wv.set_position(LogicalPosition::new(b.x, b.y))
        .map_err(err_s)?;
    wv.set_size(LogicalSize::new(b.w.max(1.0), b.h.max(1.0)))
        .map_err(err_s)
}

fn with_view(
    app: &AppHandle,
    label: &str,
    f: impl FnOnce(&tauri::Webview) -> tauri::Result<()>,
) -> Res {
    let wv = app.get_webview(label).ok_or("Açık sayfa yok")?;
    f(&wv).map(|()| Value::Null).map_err(err_s)
}

/// Alt webview oluşturur. `add_child` ana iş parçacığını bekler; async görevi kilitlememek için ayrı iş parçacığında.
async fn add_view(
    app: &AppHandle,
    builder: WebviewBuilder<tauri::Wry>,
    b: Option<Bounds>,
) -> Result<tauri::Webview, String> {
    let window = app.get_window("main").ok_or("Ana pencere bulunamadı")?;
    let b = b.unwrap_or(Bounds {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    });
    let builder = builder
        .incognito(true)
        .devtools(cfg!(debug_assertions))
        .on_download(|_, _| false);
    tauri::async_runtime::spawn_blocking(move || {
        window.add_child(
            builder,
            LogicalPosition::new(b.x, b.y),
            LogicalSize::new(b.w.max(1.0), b.h.max(1.0)),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err_s)
}

/// Gezinme geçmişi yaklaşığı: yüklenen farklı adresler sayılır, `history.back()` bir azaltır.
/// (SPA içi `pushState` gezinmeleri sayfa yükü üretmez; geri düğmesi o durumda temkinli kalır.)
#[derive(Debug, Default, Clone)]
struct Nav {
    url: String,
    title: String,
    loading: bool,
    depth: u32,
    back_pending: bool,
}

impl Nav {
    fn started(&mut self, url: &str) {
        self.loading = true;
        url.clone_into(&mut self.url);
    }

    fn finished(&mut self, url: &str, last: &mut String) {
        self.loading = false;
        url.clone_into(&mut self.url);
        if self.back_pending {
            self.depth = self.depth.saturating_sub(1).max(1);
            self.back_pending = false;
        } else if last != url {
            self.depth += 1;
        }
        url.clone_into(last);
    }

    fn can_go_back(&self) -> bool {
        self.depth > 1
    }
}

// --- preview görünümü -------------------------------------------------------------------------

#[derive(Debug, Default)]
struct PreviewView {
    nav: Nav,
    last: String,
    url: Option<String>,
    bounds: Option<Bounds>,
    visible: bool,
}

fn pv() -> &'static Mutex<PreviewView> {
    static P: OnceLock<Mutex<PreviewView>> = OnceLock::new();
    P.get_or_init(Mutex::default)
}

fn emit_preview_nav(app: &AppHandle) {
    let ev = {
        let p = lock(pv());
        json!({ "type": "nav", "url": p.nav.url, "title": p.nav.title, "canGoBack": p.nav.can_go_back(), "loading": p.nav.loading })
    };
    let _ = app.emit("kor:preview/event", ev);
}

fn sync_preview(app: &AppHandle) -> Res {
    let (bounds, visible) = {
        let p = lock(pv());
        (p.bounds, p.visible)
    };
    let Some(wv) = app.get_webview(PREVIEW_LABEL) else {
        return Ok(Value::Null);
    };
    match bounds.filter(|b| visible && b.w >= 1.0 && b.h >= 1.0) {
        Some(b) => {
            place(&wv, b)?;
            wv.show().map_err(err_s)?;
        }
        None => wv.hide().map_err(err_s)?,
    }
    Ok(Value::Null)
}

fn preview_builder(app: &AppHandle, url: Url) -> WebviewBuilder<tauri::Wry> {
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    WebviewBuilder::new(PREVIEW_LABEL, WebviewUrl::External(url))
        .on_navigation(move |u| {
            let ok = preview_url_ok(u) || u.as_str() == "about:blank";
            if !ok {
                let _ = a1.emit(
                    "kor:preview/event",
                    json!({ "type": "loadError", "url": u.as_str(), "message": PREVIEW_ONLY_LOCAL }),
                );
            }
            ok
        })
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_page_load(move |_, payload| {
            let url = payload.url().as_str();
            {
                let mut p = lock(pv());
                match payload.event() {
                    PageLoadEvent::Started => p.nav.started(url),
                    PageLoadEvent::Finished => {
                        let mut last = std::mem::take(&mut p.last);
                        p.nav.finished(url, &mut last);
                        p.last = last;
                        p.url = Some(url.to_owned());
                    }
                }
            }
            emit_preview_nav(&a2);
        })
        .on_document_title_changed(move |_, title| {
            lock(pv()).nav.title = title.chars().take(300).collect();
            emit_preview_nav(&a3);
        })
}

fn create_lock() -> &'static tokio::sync::Mutex<()> {
    static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    L.get_or_init(tokio::sync::Mutex::default)
}

async fn preview_show(app: &AppHandle, url: Option<&str>, bounds: Option<&Value>) -> Res {
    let url = match url {
        Some(raw) => {
            let u = parse_input(raw, "http")?;
            if !preview_url_ok(&u) {
                return Err(PREVIEW_ONLY_LOCAL.into());
            }
            Some(u)
        }
        None => None,
    };
    {
        let mut p = lock(pv());
        if bounds.is_some() {
            p.bounds = Some(parse_bounds(bounds)?);
        }
        p.visible = true;
    }
    let _g = create_lock().lock().await;
    match (app.get_webview(PREVIEW_LABEL), url) {
        (Some(wv), Some(u)) => {
            let same = wv.url().is_ok_and(|cur| cur == u);
            if !same {
                wv.navigate(u).map_err(err_s)?;
            }
        }
        (Some(_), None) => {}
        (None, Some(u)) => {
            let b = lock(pv()).bounds;
            add_view(app, preview_builder(app, u), b).await?;
        }
        (None, None) => {}
    }
    sync_preview(app)
}

fn set_inspect(app: &AppHandle, enabled: bool) -> Res {
    if !enabled {
        return Ok(Value::Null);
    }
    #[cfg(debug_assertions)]
    {
        with_view(app, PREVIEW_LABEL, |wv| {
            wv.open_devtools();
            Ok(())
        })
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = app;
        Err("Öğe seçme bu sürümde yok; geliştirici araçları yalnızca geliştirme derlemesinde açılır.".into())
    }
}

// =============================================================================================
// abrowser: ajan tarayıcısı paneli (sekme başına bir alt webview, yalnızca etkin olan görünür)
// =============================================================================================

#[derive(Debug, Clone)]
struct Tab {
    id: String,
    nav: Nav,
    last: String,
}

#[derive(Debug, Default)]
struct Browser {
    tabs: Vec<Tab>,
    active: Option<String>,
    bounds: Option<Bounds>,
    visible: bool,
    next: u64,
    logs: VecDeque<Value>,
}

fn br() -> &'static Mutex<Browser> {
    static B: OnceLock<Mutex<Browser>> = OnceLock::new();
    B.get_or_init(Mutex::default)
}

fn tabs_json(b: &Browser) -> Value {
    Value::Array(
        b.tabs
            .iter()
            .map(|t| {
                json!({
                    "id": t.id, "url": t.nav.url, "title": t.nav.title, "loading": t.nav.loading,
                    "active": b.active.as_deref() == Some(t.id.as_str()), "canGoBack": t.nav.can_go_back(),
                })
            })
            .collect(),
    )
}

fn emit_tabs(app: &AppHandle) {
    let tabs = tabs_json(&lock(br()));
    let _ = app.emit(
        "kor:abrowser/event",
        json!({ "type": "tabs", "tabs": tabs }),
    );
}

fn browser_log(app: &AppHandle, tab: &str, level: &str, text: String) {
    {
        let mut b = lock(br());
        b.logs
            .push_back(json!({ "level": level, "text": text, "at": crate::meta::now_ms() }));
        while b.logs.len() > MAX_LOG {
            b.logs.pop_front();
        }
    }
    let _ = app.emit(
        "kor:abrowser/event",
        json!({ "type": "console", "level": level, "text": text, "tabId": tab }),
    );
}

/// Yalnızca etkin sekme, panel görünürken ve geçerli sınırlarla gösterilir.
fn sync_browser(app: &AppHandle) -> Res {
    let (ids, active, bounds) = {
        let b = lock(br());
        let shown = b.bounds.filter(|x| b.visible && x.w >= 1.0 && x.h >= 1.0);
        (
            b.tabs.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            b.active.clone(),
            shown,
        )
    };
    for id in ids {
        let Some(wv) = app.get_webview(&id) else {
            continue;
        };
        match bounds.filter(|_| active.as_deref() == Some(id.as_str())) {
            Some(b) => {
                place(&wv, b)?;
                wv.show().map_err(err_s)?;
            }
            None => wv.hide().map_err(err_s)?,
        }
    }
    Ok(Value::Null)
}

fn tab_mut<'a>(b: &'a mut Browser, id: &str) -> Option<&'a mut Tab> {
    b.tabs.iter_mut().find(|t| t.id == id)
}

fn tab_builder(app: &AppHandle, id: &str, url: Url) -> WebviewBuilder<tauri::Wry> {
    let (a1, a2, a3, a4) = (app.clone(), app.clone(), app.clone(), app.clone());
    let (i1, i2, i3) = (id.to_owned(), id.to_owned(), id.to_owned());
    WebviewBuilder::new(id, WebviewUrl::External(url))
        .on_navigation(move |u| {
            let ok = browser_url_ok(u);
            if !ok {
                let shown: String = u.as_str().chars().take(200).collect();
                browser_log(&a1, &i1, "warn", format!("Engellenen adres: {shown}"));
            }
            ok
        })
        .on_new_window(move |u, _| {
            // Yeni pencere yerine yeni sekme (add_child ana iş parçacığını beklediğinden ayrı görevde).
            if browser_url_ok(&u) {
                let app = a2.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = new_tab(&app, Some(u)).await;
                });
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_page_load(move |_, payload| {
            let url = payload.url().as_str();
            {
                let mut b = lock(br());
                if let Some(t) = tab_mut(&mut b, &i2) {
                    match payload.event() {
                        PageLoadEvent::Started => t.nav.started(url),
                        PageLoadEvent::Finished => {
                            let mut last = std::mem::take(&mut t.last);
                            t.nav.finished(url, &mut last);
                            t.last = last;
                        }
                    }
                }
            }
            emit_tabs(&a3);
        })
        .on_document_title_changed(move |_, title| {
            {
                let mut b = lock(br());
                if let Some(t) = tab_mut(&mut b, &i3) {
                    t.nav.title = title.chars().take(300).collect();
                }
            }
            emit_tabs(&a4);
        })
}

async fn new_tab(app: &AppHandle, url: Option<Url>) -> Res {
    let url = url.unwrap_or_else(|| Url::parse("about:blank").expect("static url"));
    if !browser_url_ok(&url) {
        return Err(BLOCKED_URL.into());
    }
    let (id, bounds) = {
        let mut b = lock(br());
        if b.tabs.len() >= MAX_TABS {
            return Err(format!("En fazla {MAX_TABS} sekme açılabilir"));
        }
        b.next += 1;
        let id = format!("{TAB_PREFIX}{}", b.next);
        b.tabs.push(Tab {
            id: id.clone(),
            nav: Nav {
                url: url.to_string(),
                ..Nav::default()
            },
            last: String::new(),
        });
        (id, b.bounds)
    };
    if let Err(e) = add_view(app, tab_builder(app, &id, url), bounds).await {
        lock(br()).tabs.retain(|t| t.id != id);
        return Err(e);
    }
    lock(br()).active = Some(id);
    sync_browser(app)?;
    emit_tabs(app);
    Ok(Value::Null)
}

fn known_tab(id: &str) -> Result<String, String> {
    if lock(br()).tabs.iter().any(|t| t.id == id) {
        Ok(id.to_owned())
    } else {
        Err("Sekme bulunamadı".into())
    }
}

fn active_tab() -> Result<String, String> {
    lock(br())
        .active
        .clone()
        .ok_or_else(|| "Açık sekme yok".into())
}

async fn abrowser(name: &str, args: &[Value], app: &AppHandle) -> Option<Res> {
    let m = format!("abrowser.{name}");
    Some(match name {
        "tabs" => Ok(tabs_json(&lock(br()))),
        "logs" => Ok(Value::Array(lock(br()).logs.iter().cloned().collect())),
        "show" => match parse_bounds(args.first()) {
            Ok(bd) => {
                {
                    let mut b = lock(br());
                    b.bounds = Some(bd);
                    b.visible = true;
                }
                sync_browser(app)
            }
            Err(e) => Err(e),
        },
        "hide" => {
            lock(br()).visible = false;
            sync_browser(app)
        }
        "setBounds" => match parse_bounds(args.first()) {
            Ok(bd) => {
                lock(br()).bounds = Some(bd);
                sync_browser(app)
            }
            Err(e) => Err(e),
        },
        "activate" => match s(args, 0, &m).and_then(known_tab) {
            Ok(id) => {
                lock(br()).active = Some(id);
                let r = sync_browser(app);
                emit_tabs(app);
                r
            }
            Err(e) => Err(e),
        },
        "close" => match s(args, 0, &m).and_then(known_tab) {
            Ok(id) => {
                if let Some(wv) = app.get_webview(&id) {
                    let _ = wv.close();
                }
                {
                    let mut b = lock(br());
                    b.tabs.retain(|t| t.id != id);
                    if b.active.as_deref() == Some(id.as_str()) {
                        b.active = b.tabs.last().map(|t| t.id.clone());
                    }
                }
                let r = sync_browser(app);
                emit_tabs(app);
                r
            }
            Err(e) => Err(e),
        },
        "navigate" => match s(args, 0, &m).and_then(|r| parse_input(r, "https")) {
            Ok(u) if browser_url_ok(&u) => match active_tab() {
                Ok(id) => with_view(app, &id, |wv| wv.navigate(u)),
                Err(_) => new_tab(app, Some(u)).await,
            },
            Ok(_) => Err(BLOCKED_URL.into()),
            Err(e) => Err(e),
        },
        "back" => active_tab().and_then(|id| {
            if let Some(t) = tab_mut(&mut lock(br()), &id) {
                t.nav.back_pending = true;
            }
            with_view(app, &id, |wv| wv.eval("history.back()"))
        }),
        "forward" => {
            active_tab().and_then(|id| with_view(app, &id, |wv| wv.eval("history.forward()")))
        }
        "reload" => active_tab().and_then(|id| with_view(app, &id, |wv| wv.reload())),
        "newTab" => match opt_s(args, 0).map(|r| parse_input(r, "https")).transpose() {
            Ok(u) => new_tab(app, u).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_never_reveals_more_than_last4() {
        assert_eq!(hint("short-key"), "••••");
        assert_eq!(hint("sk-abcdefghijklmnop1234"), "…1234");
    }

    #[test]
    fn splits_args_without_shell() {
        assert_eq!(split_args("npm run dev").unwrap(), ["npm", "run", "dev"]);
        assert_eq!(
            split_args(r#"node "my server.js"  --port '3000' """#).unwrap(),
            ["node", "my server.js", "--port", "3000", ""]
        );
        assert!(split_args("npm \"run").is_err());
        assert_eq!(split_args("a && b").unwrap(), ["a", "&&", "b"]);
    }

    #[test]
    fn finds_local_url_in_dev_output() {
        let line = ansi_re().replace_all("  \x1b[32m➜\x1b[0m  Local:   http://localhost:5173/", "");
        assert_eq!(local_url(&line).as_deref(), Some("http://localhost:5173/"));
        assert_eq!(
            local_url("ready on http://0.0.0.0:3000.").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(local_url("see https://example.com:3000"), None);
    }

    #[test]
    fn ollama_names() {
        assert!(valid_ollama_name("llama3.1:8b"));
        assert!(valid_ollama_name("library/qwen3:latest"));
        assert!(!valid_ollama_name("../x"));
        assert!(!valid_ollama_name("-rm"));
        assert!(!valid_ollama_name("a b"));
    }

    #[test]
    fn pull_progress_lines() {
        assert_eq!(pull_line("").unwrap(), None);
        assert_eq!(
            pull_line(r#"{"status":"pulling","completed":5,"total":10}"#).unwrap(),
            Some(("pulling".into(), Some(5), Some(10)))
        );
        assert!(pull_line(r#"{"error":"not found"}"#).is_err());
    }

    #[test]
    fn endpoint_validation() {
        let ok = parse_endpoint(
            &json!({ "baseURL": "https://x.example/v1/", "headers": { "X-Org": "a" } }),
        )
        .unwrap();
        assert_eq!(ok.0.as_deref(), Some("https://x.example/v1"));
        assert!(parse_endpoint(&json!({ "baseURL": "http://evil.example/v1" })).is_err());
        assert!(parse_endpoint(&json!({ "baseURL": "http://127.0.0.1:8080/v1" })).is_ok());
        assert!(parse_endpoint(&json!({ "baseURL": "https://u:p@x.example" })).is_err());
        assert!(parse_endpoint(&json!({ "headers": { "Authorization": "x" } })).is_err());
        assert!(parse_endpoint(&json!({ "headers": { "X-A": "a\nb" } })).is_err());
        assert!(parse_endpoint(&json!({ "timeoutMs": 5000 })).is_err());
    }

    #[test]
    fn builtin_manifests_round_trip_through_toml() {
        for m in Registry::builtin().iter() {
            let mut m = m.clone();
            m.quirks.extra_headers.insert("X-Test".into(), "1".into());
            let text = manifest_toml(&m).unwrap();
            assert!(text.starts_with(OVERRIDE_MARKER));
            assert_eq!(ProviderManifest::parse(&text, "t").unwrap(), m);
        }
    }

    #[test]
    fn settings_patch_validation() {
        let p = |v: Value| validate_patch(v.as_object().unwrap());
        assert!(p(json!({ "ollamaEnabled": true })).is_ok());
        assert!(p(json!({ "ollamaEnabled": "yes" })).is_err());
        assert!(p(json!({ "costCapUsd": null })).is_ok());
        assert!(p(json!({ "costCapUsd": 5 })).is_err());
        assert!(
            p(json!({ "responseCache": { "enabled": true, "ttlHours": 24, "exclude": [] } }))
                .is_err()
        );
        assert!(p(json!({ "responseCache": { "enabled": false, "ttlHours": 24, "exclude": ["gizli"] } })).is_ok());
        assert!(p(json!({ "presets": { "Hızlı": { "temperature": 0.2 } } })).is_ok());
        assert!(p(json!({ "presets": { "x": { "temperature": 3 } } })).is_err());
        assert!(p(json!({ "prices": { "openai/gpt": { "input": 1, "output": 2 } } })).is_ok());
        assert!(p(json!({ "prices": { "openai/gpt": { "input": 1 } } })).is_err());
        assert!(p(json!({ "endpoints": {} })).is_err());
    }

    #[test]
    fn child_webview_url_policy() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(preview_url_ok(&u("http://localhost:5173/")));
        assert!(preview_url_ok(&u("http://127.0.0.1:3000/x")));
        assert!(!preview_url_ok(&u("https://example.com/")));
        assert!(!preview_url_ok(&u("http://tauri.localhost/")));
        assert!(browser_url_ok(&u("https://example.com/")));
        assert!(browser_url_ok(&u("about:blank")));
        assert!(browser_url_ok(&u("http://localhost:8080/")));
        for bad in [
            "http://tauri.localhost/index.html",
            "http://ipc.localhost/kor_call",
            "file:///C:/Windows/win.ini",
            "data:text/html,<b>x</b>",
            "javascript:alert(1)",
            "tauri://localhost/",
            "asset://localhost/x",
        ] {
            assert!(!browser_url_ok(&u(bad)), "{bad}");
        }
        assert_eq!(
            parse_input("localhost:3000", "http").unwrap().as_str(),
            "http://localhost:3000/"
        );
        assert_eq!(
            parse_input("example.com", "https").unwrap().as_str(),
            "https://example.com/"
        );
        assert!(!parse_input("javascript:alert(1)", "https").is_ok_and(|x| browser_url_ok(&x)));
        assert!(!parse_input("data:text/html,x", "https").is_ok_and(|x| browser_url_ok(&x)));
    }

    #[test]
    fn bounds_and_history() {
        assert!(
            parse_bounds(Some(
                &json!({ "x": 1, "y": 2.5, "width": 300, "height": 200 })
            ))
            .is_ok()
        );
        assert!(parse_bounds(Some(&json!({ "x": 1, "y": 2, "width": -3, "height": 2 }))).is_err());
        assert!(parse_bounds(Some(&json!({ "x": 1e9, "y": 2, "width": 3, "height": 2 }))).is_err());
        let mut n = Nav::default();
        let mut last = String::new();
        n.finished("http://a/", &mut last);
        assert!(!n.can_go_back());
        n.finished("http://a/b", &mut last);
        assert!(n.can_go_back());
        n.finished("http://a/b", &mut last); // yenileme
        assert_eq!(n.depth, 2);
        n.back_pending = true;
        n.finished("http://a/", &mut last);
        assert!(!n.can_go_back());
    }

    #[test]
    fn session_cost() {
        let u = kor_core::Usage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 500_000,
            ..Default::default()
        };
        let price = json!({ "input": 2.0, "output": 4.0, "cacheInput": 1.0 });
        // OpenAI tarzı: önbellek girdinin içinde.
        assert!((session_usd(&u, &price, false) - (1.0 + 0.5 + 4.0)).abs() < 1e-9);
        // Anthropic tarzı: ayrı.
        assert!((session_usd(&u, &price, true) - (2.0 + 0.5 + 4.0)).abs() < 1e-9);
    }
}
