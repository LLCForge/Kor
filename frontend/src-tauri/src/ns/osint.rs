//! `osint.*`: işletim sistemi entegrasyonu köprüsü (§25, §27.6; ADR 0052). Mantık `crate::osint`'te.
//!
//! - `osint.pending()` → arayüz dinlemeye başlamadan gelen `Request` dizisi (sonrası `kor:osint/request`).
//! - `osint.drop(paths: string[])` → pencereye bırakılan tek klasör için `Request` (open) ya da `null`.
//! - `osint.recent()` → son projeler (atlama listesi / Dock menüsü kaynağı).
//! - `osint.hotkeyGet()` / `osint.hotkeySet(value)` → `{ hotkey, default, active }`; boş = kapalı.
//! - `osint.menuStatus()` / `osint.menuInstall(extended: bool)` / `osint.menuUninstall()` → sağ tık menüsü
//!   durumu (`kor integrate status|install|uninstall` ile aynı); kurma/kaldırma yalnızca arayüzdeki onaydan sonra.

use serde_json::Value;
use tauri::AppHandle;

use crate::api::{Res, b, s};
use crate::state::AppState;

/// Köprü ad alanı.
pub const NS: &str = "osint";

pub async fn call(_ns: &str, name: &str, args: &[Value], app: &AppHandle, _state: &AppState) -> Option<Res> {
    Some(match name {
        "pending" => Ok(crate::osint::pending()),
        "drop" => {
            let paths: Vec<String> = args
                .first()
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            Ok(crate::osint::dropped(&paths))
        }
        "recent" => Ok(crate::osint::recent()),
        "hotkeyGet" => Ok(crate::osint::hotkey_get()),
        "hotkeySet" => match s(args, 0, "osint.hotkeySet") {
            Ok(v) => crate::osint::hotkey_set(app, v),
            Err(e) => Err(e),
        },
        "menuStatus" => crate::osint::menu_status(),
        "menuInstall" => crate::osint::menu_install(b(args, 0)),
        "menuUninstall" => crate::osint::menu_uninstall(),
        _ => return None,
    })
}
