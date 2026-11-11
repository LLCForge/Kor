//! `team.*`: Takım paneli için ince köprü (§11, §12.3). Takım sunucusu (`kor server`) Faz 8'de
//! gelir; bugün yalnızca sunucu adresi (`[team] server`), erişim belirteci (işletim sistemi
//! anahtarlığı) ve adresin yanıt verip vermediği vardır. `kor_agent::team_sync` de
//! `KOR_TEAM_MEMORY_URL` ortam değişkenini okur; ayarlıysa durumda gösterilir.
//! Sözleşme: `frontend/src/desktop/ipc-team.ts`.
//!
//! - `team.status()` → `{ server, source: 'config'|'env'|null, token, privacy }`.
//! - `team.connect(url, token?)` → `status`. Adres doğrulanır; belirteç anahtarlığa yazılır.
//! - `team.disconnect()` → `status`. Adres ve belirteç silinir.
//! - `team.check()` → `{ reachable, status?, error? }`. Adrese tek bir GET; gizlilik modunda
//!   yalnızca bu bilgisayardaki adreslere izin verilir.

use std::path::Path;

use kor_secrets::{KeychainStore, SecretStore as _, SecretString};
use reqwest::Url;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı. (`teams` ayrı: alt ajan takımları, `ns/eco.rs`.)
pub const NS: &str = "team";

const TOKEN_ACCOUNT: &str = "team:token";
const MAX_URL: usize = 2048;

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(match name {
        "status" => status(&root),
        "connect" => match s(args, 0, "team.connect") {
            Ok(url) => connect(&root, url, opt_s(args, 1)),
            Err(e) => Err(e),
        },
        "disconnect" => disconnect(&root),
        "check" => check(&root).await,
        _ => return None,
    })
}

fn is_loopback(u: &Url) -> bool {
    u.host_str().is_some_and(|h| {
        h == "localhost" || h.ends_with(".localhost") || h == "[::1]" || h.starts_with("127.")
    })
}

/// `https://…`, ya da bu bilgisayarda `http://…`; kullanıcı adı/parola ve parça yok.
fn parse_server(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    let bad = || {
        "Sunucu adresi https://alan.adı biçiminde olmalı (yerel sunucu için http://localhost:port)."
            .to_owned()
    };
    if raw.is_empty() || raw.len() > MAX_URL || raw.chars().any(char::is_whitespace) {
        return Err(bad());
    }
    let u = Url::parse(raw).map_err(|_| bad())?;
    let scheme_ok = u.scheme() == "https" || (u.scheme() == "http" && is_loopback(&u));
    if !scheme_ok
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err(bad());
    }
    Ok(u)
}

fn status(root: &Path) -> Res {
    let loaded = crate::bridge::load_config(root)?;
    let configured = loaded.config.team.server.trim().to_owned();
    let (server, source) = if !configured.is_empty() {
        (Some(configured), Some("config"))
    } else if let Some(env) = kor_agent::team_sync::base_url() {
        (Some(env), Some("env"))
    } else {
        (None, None)
    };
    let token = KeychainStore
        .get(TOKEN_ACCOUNT)
        .map_err(|e| format!("İşletim sistemi anahtarlığı: {e}"))?
        .is_some();
    Ok(json!({
        "server": server,
        "source": source,
        "token": token,
        "privacy": loaded.config.privacy.mode,
    }))
}

fn set_server(root: &Path, value: &str) -> Result<(), String> {
    let file = kor_config::Paths::discover(root)
        .user
        .ok_or("Kullanıcı ayar klasörü bulunamadı")?;
    kor_config::set_scalar(&file, "team.server", value).map_err(|e| e.to_string())
}

fn connect(root: &Path, url: &str, token: Option<&str>) -> Res {
    let u = parse_server(url)?;
    if let Some(t) = token.map(str::trim).filter(|t| !t.is_empty()) {
        if t.len() > kor_secrets::MAX_SECRET_LEN || t.chars().any(char::is_control) {
            return Err("Erişim belirteci geçersiz.".into());
        }
        KeychainStore
            .set(TOKEN_ACCOUNT, &SecretString::from(t.to_owned()))
            .map_err(|e| format!("İşletim sistemi anahtarlığı: {e}"))?;
    }
    set_server(root, u.as_str().trim_end_matches('/'))?;
    status(root)
}

fn disconnect(root: &Path) -> Res {
    set_server(root, "")?;
    KeychainStore
        .delete(TOKEN_ACCOUNT)
        .map_err(|e| format!("İşletim sistemi anahtarlığı: {e}"))?;
    status(root)
}

async fn check(root: &Path) -> Res {
    let st = status(root)?;
    let Some(server) = st["server"].as_str() else {
        return Err("Bağlı takım sunucusu yok.".into());
    };
    let u = parse_server(server)?;
    if st["privacy"].as_bool() == Some(true) && !is_loopback(&u) {
        return Err("Gizlilik modu açık: yalnızca bu bilgisayardaki sunuculara bağlanılır.".into());
    }
    let client = kor_core::net::client_builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    Ok(match client.get(u).send().await {
        Ok(r) => json!({ "reachable": true, "status": r.status().as_u16() }),
        Err(e) => json!({ "reachable": false, "error": e.to_string() }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls_are_https_or_local() {
        assert!(parse_server("https://kor.example.com").is_ok());
        assert!(parse_server(" https://kor.example.com/base/ ").is_ok());
        assert!(parse_server("http://localhost:8787").is_ok());
        assert!(parse_server("http://127.0.0.1:8787").is_ok());
        for bad in [
            "",
            "http://kor.example.com",
            "ftp://kor.example.com",
            "https://user:pw@kor.example.com",
            "https://kor.example.com/#x",
            "kor.example.com",
            "https://a b.com",
        ] {
            assert!(parse_server(bad).is_err(), "{bad}");
        }
    }
}
