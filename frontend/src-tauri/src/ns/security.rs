//! Güvenlik paneli için ince köprü (§6.4, §6.6, §6.7, §12.3). İş mantığı çekirdekte:
//! `kor_secrets::vault` (kasa), `kor_agent::audit` (denetim kaydı), `kor_sandbox` (sandbox).
//! Sözleşme: `frontend/src/desktop/ipc-security.ts`.
//!
//! - `vault.list()` → `{ names, backend: 'keychain'|'file', file?, scope: { ad: string[]|null } }`.
//!   Değerler arayüze hiçbir zaman gönderilmez; değeri gösteren bir yöntem bilerek yok (ADR 0036).
//! - `vault.add(name, value)` → `{ names, replaced }`.
//! - `vault.remove(name)` → `{ names, removed }`.
//! - `vault.import(path?, confirm?)` → `{ path, found, skipped, existing, imported, names }`.
//!   `confirm` olmadan yalnızca önizleme (adlar) döner; hiçbir şey yazılmaz.
//! - `audit.list({ action?, model?, file?, offset?, limit? })` → `{ path, total, entries, actions, models }`.
//! - `audit.verify()` → `{ path, entries, files, ok, line?, reason?, file?, head, trimmed }`.
//! - `audit.report(since?)` → `{ markdown, report }` (KVKK/GDPR, arayüz dilinde; ADR 0036).
//! - `audit.export(path?)` → `{ path, entries, ok, head }`: tüm dosyalar tek JSONL kopyada;
//!   yol boşsa Kor veri klasöründe `audit-export-<zaman>.jsonl`.
//! - `sandbox.status()` → `{ mode, preference, backend, enforcing, files, network, limits, image,
//!   docker, notes }`. Mod `core.configSet('sandbox', …)`, sınırlar `core.configSet('sandbox.<alan>', …)`.
//! - `sandbox.test()` → `{ backend, blocked, target, detected, output }` (ADR 0033 öz-denetimi).
//! - `sandbox.setNetwork(list)` → `sandbox.status()` sonucu; `[sandbox] network` izin listesi.
//! - `sandbox.pull()` → `{ image, ok, output }`: Docker imajı yalnızca bu açık istekle indirilir.

use std::path::{Path, PathBuf};

use kor_secrets::{SecretError, SecretStore, SecretString, vault};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;


/// `.env` dosyaları bundan büyükse okunmaz.
const MAX_DOTENV: u64 = 1024 * 1024;

pub async fn call(
    ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("{ns}.{name}");
    let m = m.as_str();
    Some(match (ns, name) {
        ("vault", "list") => vault_list(state),
        ("vault", "add") => s(args, 0, m).and_then(|n| add(state, n, s(args, 1, m)?)),
        ("vault", "remove") => s(args, 0, m).and_then(|n| remove(state, n)),
        ("vault", "import") => {
            root(state).and_then(|r| import(&r, state, opt_s(args, 0), b(args, 1)))
        }
        ("audit", "list") => audit_list(args.first()),
        ("audit", "verify") => audit_verify(),
        ("audit", "report") => audit_report(state, opt_s(args, 0)),
        ("audit", "export") => audit_export(opt_s(args, 0)),
        ("sandbox", "status") => root(state).and_then(|r| sandbox(&r)),
        ("sandbox", "test") => match root(state) {
            Ok(r) => sandbox_test(&r).await,
            Err(e) => Err(e),
        },
        ("sandbox", "setNetwork") => root(state).and_then(|r| set_network(&r, args.first())),
        ("sandbox", "pull") => match root(state) {
            Ok(r) => sandbox_pull(&r).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

fn tr(e: &SecretError) -> String {
    match e {
        SecretError::InvalidName(n) => format!(
            "Geçersiz ad: {n}. Harf, rakam ve _ kullanın; rakamla başlamasın (en çok 128 karakter)."
        ),
        SecretError::InvalidSecret => {
            "Değer boş olamaz, satır sonu içeremez ve 8192 baytı geçemez.".into()
        }
        SecretError::InvalidProvider(p) => format!("Geçersiz ad: {p}"),
        SecretError::Keychain(e) => format!("İşletim sistemi anahtarlığı: {e}"),
        SecretError::VaultFile(e) => format!(
            "Kasa dosyası: {e}. Masaüstü uygulaması parolayı {} ortam değişkeninden okur.",
            kor_runtime::secure::PASSPHRASE_ENV
        ),
    }
}

/// `[vault]` ayarı (açık proje ya da çalışma klasörüne göre katmanlı yapılandırma).
fn vault_cfg(state: &AppState) -> kor_config::Vault {
    let r = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    crate::bridge::load_config(&r)
        .map(|l| l.config.vault)
        .unwrap_or_default()
}

/// Ayarlı kasa: işletim sistemi anahtarlığı ya da şifreli dosya (ADR 0036).
fn store(state: &AppState) -> Result<Box<dyn SecretStore>, String> {
    kor_runtime::secure::vault_store(&vault_cfg(state), None).map_err(|e| tr(&e))
}

fn names(store: &dyn SecretStore) -> Result<Vec<String>, String> {
    vault::names(store).map_err(|e| tr(&e))
}

/// Adlar, arka uç ve her sırrın hangi komutlara verildiği (`[vault.scope]`; `null` = tümü).
fn vault_list(state: &AppState) -> Res {
    let cfg = vault_cfg(state);
    let file = cfg.backend == kor_config::VaultBackend::File;
    let n = names(store(state)?.as_ref())?;
    let scope: serde_json::Map<String, Value> = n
        .iter()
        .map(|k| (k.clone(), cfg.scope.get(k).map_or(Value::Null, |p| json!(p))))
        .collect();
    Ok(json!({
        "names": n,
        "backend": if file { "file" } else { "keychain" },
        "file": file.then(|| kor_runtime::secure::vault_file().map(|p| kor_tools::display_path(&p))).flatten(),
        "scope": scope,
    }))
}

fn add(state: &AppState, name: &str, value: &str) -> Res {
    let st = store(state)?;
    let replaced = vault::add(
        st.as_ref(),
        name.trim(),
        &SecretString::from(value.to_owned()),
    )
    .map_err(|e| tr(&e))?;
    Ok(json!({ "names": names(st.as_ref())?, "replaced": replaced }))
}

fn remove(state: &AppState, name: &str) -> Res {
    let st = store(state)?;
    let removed = vault::remove(st.as_ref(), name).map_err(|e| tr(&e))?;
    Ok(json!({ "names": names(st.as_ref())?, "removed": removed }))
}

/// Göreli yol proje köküne göredir; boşsa `<kök>/.env`.
fn dotenv_path(root: &Path, path: Option<&str>) -> PathBuf {
    match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => root.join(p),
        None => root.join(".env"),
    }
}

fn import(root: &Path, state: &AppState, path: Option<&str>, confirm: bool) -> Res {
    let p = dotenv_path(root, path);
    let shown = kor_tools::display_path(&p);
    let meta = std::fs::metadata(&p).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_DOTENV {
        return Err(format!("{shown}: 1 MB'tan küçük bir dosya olmalı"));
    }
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{shown}: {e}"))?;
    let parsed = vault::parse_dotenv(&text);
    let st = store(state)?;
    let current = names(st.as_ref())?;
    let found: Vec<&str> = parsed.entries.iter().map(|(k, _)| k.as_str()).collect();
    let existing: Vec<&str> = found
        .iter()
        .copied()
        .filter(|k| current.iter().any(|c| c == k))
        .collect();
    let mut imported = false;
    if confirm && !parsed.entries.is_empty() {
        for (k, v) in &parsed.entries {
            vault::add(st.as_ref(), k, v).map_err(|e| format!("{k}: {}", tr(&e)))?;
        }
        imported = true;
    }
    Ok(json!({
        "path": shown,
        "found": found,
        "skipped": parsed.skipped,
        "existing": existing,
        "imported": imported,
        "names": if imported { names(st.as_ref())? } else { current },
    }))
}

fn audit_path() -> Result<PathBuf, String> {
    kor_runtime::secure::audit_path().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

fn audit_list(q: Option<&Value>) -> Res {
    let path = audit_path()?;
    let field = |k: &str| {
        q.and_then(|q| q.get(k))
            .and_then(Value::as_str)
            .map(|v| v.chars().take(200).collect::<String>())
    };
    let num = |k: &str, d: usize| {
        q.and_then(|q| q.get(k))
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(d)
    };
    let query = kor_agent::audit::Query {
        action: field("action"),
        model: field("model"),
        file: field("file"),
        offset: num("offset", 0),
        limit: num("limit", 50),
    };
    let page = kor_agent::audit::read(&path, &query)
        .map_err(|e| format!("{}: {e}", kor_tools::display_path(&path)))?;
    let mut v = serde_json::to_value(page).map_err(|e| e.to_string())?;
    v["path"] = json!(kor_tools::display_path(&path));
    Ok(v)
}

fn audit_verify() -> Res {
    let path = audit_path()?;
    let r = kor_agent::audit::verify(&path)
        .map_err(|e| format!("{}: {e}", kor_tools::display_path(&path)))?;
    Ok(json!({
        "path": kor_tools::display_path(&path),
        "entries": r.entries,
        "files": r.files,
        "ok": r.ok(),
        "line": r.broken.map(|(l, _)| l),
        "reason": r.broken.map(|(_, why)| why),
        "file": r.file,
        "head": r.head,
        "trimmed": r.trimmed,
    }))
}

/// KVKK/GDPR raporu (§6.7) arayüz dilinde Markdown ve ham veri olarak. İçerik (istem, dosya
/// içeriği) içermez; yalnızca sayılar, sağlayıcılar ve dosya yolları.
fn audit_report(state: &AppState, since: Option<&str>) -> Res {
    let since = since.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = since
        && !kor_runtime::secure::valid_since(s)
    {
        return Err(format!("Geçersiz tarih: {s} (YYYY-AA-GG kullanın)"));
    }
    let r = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let lang = crate::bridge::load_config(&r)
        .map(|l| l.config.ui.language.resolve())
        .unwrap_or(kor_i18n::Lang::Tr);
    let l = kor_i18n::Localizer::new(lang);
    let report = kor_runtime::secure::audit_report(&l, since)?;
    Ok(json!({
        "markdown": kor_runtime::secure::report_markdown(&report, &l),
        "report": report,
    }))
}

/// Tüm denetim dosyalarını (eskiden yeniye) tek bir JSONL kopyasına yazar.
fn audit_export(target: Option<&str>) -> Res {
    let path = audit_path()?;
    let shown = |p: &Path| kor_tools::display_path(p);
    let v = kor_agent::audit::verify(&path).map_err(|e| format!("{}: {e}", shown(&path)))?;
    let out = match target.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if Path::new(t).is_absolute() => PathBuf::from(t),
        Some(_) => return Err("Dışa aktarma yolu mutlak olmalı".into()),
        None => path.with_file_name(format!(
            "audit-export-{}.jsonl",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        )),
    };
    let mut body = String::new();
    for f in kor_agent::audit::files(&path).map_err(|e| e.to_string())? {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", shown(&f)))?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            body.push_str(line);
            body.push('\n');
        }
    }
    std::fs::write(&out, body).map_err(|e| format!("{}: {e}", shown(&out)))?;
    Ok(json!({
        "path": shown(&out),
        "entries": v.entries,
        "ok": v.ok(),
        "head": v.head,
    }))
}

/// `[sandbox]` ve bu makinede `mode = "on"` ile neyin gerçekten uygulandığı (ADR 0033).
fn sandbox(root: &Path) -> Res {
    let loaded = crate::bridge::load_config(root)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let st = kor_runtime::sandbox::status(&l, &loaded.config.sandbox);
    let (s, e) = (&st.setup, &st.enforcement);
    Ok(json!({
        "mode": st.mode.as_str(),
        "preference": kor_runtime::sandbox::preference_str(st.preference),
        "backend": s.kind.as_str(),
        "enforcing": s.kind.is_enforcing(),
        "files": e.files,
        "network": {
            "on": s.network,
            "allow": s.options.allow,
            "filter": if e.filter == kor_sandbox::Filter::Ports { "ports" } else { "none" },
        },
        "limits": {
            "memoryMb": s.options.memory_mb,
            "memoryEnforced": e.memory,
            "cpuPercent": s.options.cpu_percent,
            "cpuEnforced": e.cpu,
        },
        "image": s.options.image,
        "docker": st.docker.map(|d| json!({
            "installed": d.installed, "daemon": d.daemon, "imagePresent": d.image_present,
        })),
        "notes": s.notes,
    }))
}

/// `sandbox.test()`: güvenli öz-denetim; geçici bir projenin dışına tek dosya yazmayı dener.
async fn sandbox_test(root: &Path) -> Res {
    let loaded = crate::bridge::load_config(root)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let st = kor_runtime::sandbox::status(&l, &loaded.config.sandbox);
    if !st.setup.kind.is_enforcing() {
        return Err("Bu makinede sandbox arka ucu yok; test edilecek bir şey yok.".into());
    }
    let p = kor_sandbox::probe(st.setup.kind, &st.setup.options)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "backend": p.kind.as_str(),
        "blocked": p.blocked,
        "target": kor_tools::display_path(&p.target),
        "detected": p.detected.iter().map(|d| d.line.clone()).collect::<Vec<_>>(),
        "output": kor_agent::audit::clip(&p.output),
    }))
}

/// `sandbox.setNetwork(["host:port", …])`: `[sandbox] network` kullanıcı ayar dosyasına yazılır.
/// Geçersiz bir girdi varsa hiçbir şey yazılmaz (genişletmek yerine reddet).
fn set_network(root: &Path, list: Option<&Value>) -> Res {
    let items = list
        .and_then(Value::as_array)
        .ok_or("Liste bekleniyor")?;
    if items.len() > 64 {
        return Err("En çok 64 girdi".into());
    }
    let mut entries = Vec::new();
    for v in items {
        let e = v.as_str().map(str::trim).unwrap_or_default();
        if !kor_sandbox::valid_allow(e) {
            return Err(format!("Geçersiz girdi: {e}. host:port ya da * kullanın."));
        }
        entries.push(e.to_owned());
    }
    let file = kor_config::Paths::discover(root)
        .user
        .ok_or("Kullanıcı ayar klasörü bulunamadı")?;
    kor_config::set_string_list(&file, "sandbox.network", &entries).map_err(|e| e.to_string())?;
    sandbox(root)
}

/// `sandbox.pull()`: Docker imajını kullanıcı isteğiyle indirir (Kor kendiliğinden indirmez).
async fn sandbox_pull(root: &Path) -> Res {
    let loaded = crate::bridge::load_config(root)?;
    let image = loaded.config.sandbox.image.trim().to_owned();
    let shown = image.clone();
    let out = tokio::task::spawn_blocking(move || kor_sandbox::docker_pull(&image, false))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(json!({ "image": shown, "ok": out.status.success(), "output": kor_agent::audit::clip(&text) }))
}

/// Kenar çubuğu Güvenlik düğmesi (`g12GetSandbox` / `g12SetSandbox(enabled)`): aynı gerçek ayar.
/// Açmak `sandbox = on`, kapatmak `off` yazar (kullanıcı ayar dosyası; proje bunu değiştiremez).
pub(crate) fn g12_sandbox(state: &AppState, set: Option<bool>) -> Res {
    let r = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    if let Some(on) = set {
        let file = kor_config::Paths::discover(&r)
            .user
            .ok_or("Kullanıcı ayar klasörü bulunamadı")?;
        kor_config::set_scalar(&file, "sandbox", if on { "on" } else { "off" })
            .map_err(|e| e.to_string())?;
    }
    let st = sandbox(&r)?;
    let enforcing = st["enforcing"].as_bool() == Some(true);
    let message = match (st["mode"].as_str(), enforcing) {
        (Some("on"), true) => format!(
            "Komutlar {} ile izole çalışır: {}, yalnızca proje klasörüne yazılır.",
            st["backend"].as_str().unwrap_or_default(),
            if st["network"]["on"] == true { "ağ izin listesine göre açık" } else { "ağ kapalı" }
        ),
        (Some("on"), false) => {
            "Bu makinede sandbox arka ucu bulunamadı: Kor her komuttan önce onay ister.".into()
        }
        (Some("ask"), _) => "Sandbox kapalı; her komut onay ister.".into(),
        _ => "Sandbox kapalı; komutlar izin moduna göre çalışır.".into(),
    };
    Ok(json!({
        "enabled": st["mode"] == "on",
        "available": enforcing,
        "mechanism": Value::Null,
        "message": message,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_path_defaults_to_project_env() {
        let root = Path::new("/p");
        assert_eq!(dotenv_path(root, None), root.join(".env"));
        assert_eq!(dotenv_path(root, Some("  ")), root.join(".env"));
        assert_eq!(
            dotenv_path(root, Some("cfg/.env.local")),
            root.join("cfg/.env.local")
        );
    }
}
