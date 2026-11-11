//! `env.status`: `kor env status` için ince köprü (§15.7, ADR 0043). Algılanan ortam (devcontainer,
//! compose, nix, mise/asdf), komutların nasıl çalıştığı ve beklenen/bulunan sürümler; sürüm yoklaması
//! bir oturumun ilk komutuyla aynı sarmalayıcı ve sandbox içinde çalışır. İş mantığı
//! `kor_runtime::devenv` içinde.

use serde_json::Value;
use tauri::AppHandle;

use crate::api::{Res, root};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `devenv::call`'a yönlendirir).
pub const NS: &str = "env";

async fn status(state: &AppState) -> Res {
    let root = root(state)?;
    let loaded = crate::bridge::load_config(&root)?;
    serde_json::to_value(kor_runtime::devenv::status(&loaded.config, &root).await)
        .map_err(|e| e.to_string())
}

pub async fn call(
    _ns: &str,
    name: &str,
    _args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "status" => status(state).await,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn namespace_is_env() {
        assert_eq!(super::NS, "env");
    }
}
