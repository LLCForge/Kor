//! `privacy.*`: sağlayıcı gizlilik kartı (§31.2, ADR 0057). Veri `kor_providers::privacy_card`
//! (kaynak bağlantısı ve okunma tarihiyle); bilinmeyen alanlar "bilinmiyor" kalır.
//!
//! - `privacy.cards()` → `{ require, providers: PrivacyRow[] }`: etkin şartlar (`privacy.require`,
//!   gizlilik modunda en sıkı küme) ve her sağlayıcının kartı, yerelleştirilmiş metinleri ve
//!   karşılamadığı şartlar. Model seçici rozetleri ve soluklaştırma buradan.
//! - `privacy.setRequire(list)` → `string[]`: kullanıcı yapılandırmasına `privacy.require` yazar.

use kor_core::PrivacyRequirement;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root};
use crate::state::AppState;

pub const NS: &str = "privacy";

fn localizer(state: &AppState) -> kor_i18n::Localizer {
    let lang = root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map_or(kor_i18n::Lang::Tr, |l| l.config.ui.language.resolve());
    kor_i18n::Localizer::new(lang)
}

#[allow(clippy::unused_async)] // the shape every ns module has (`ns::call` awaits it)
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let r = match name {
        "cards" => cards(state),
        "setRequire" => set_require(state, args),
        _ => return None,
    };
    Some(r)
}

fn cards(state: &AppState) -> Res {
    // `load_config` applies `privacy.require` and privacy mode to the process (ADR 0035).
    if let Ok(r) = root(state) {
        crate::bridge::load_config(&r)?;
    }
    Ok(kor_runtime::privacycard::payload(&localizer(state)))
}

fn set_require(state: &AppState, args: &[Value]) -> Res {
    let list: Vec<String> = args
        .first()
        .and_then(Value::as_array)
        .ok_or("`privacy.setRequire`: liste gerekli")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let parsed = PrivacyRequirement::parse_list(&list.join(","))?;
    let names: Vec<String> = parsed.iter().map(|r| r.as_str().to_owned()).collect();
    let file = kor_config::user_dir()
        .ok_or("Kullanıcı yapılandırma klasörü yok")?
        .join("config.toml");
    kor_config::set_string_list(&file, "privacy.require", &names).map_err(|e| e.to_string())?;
    if let Ok(r) = root(state) {
        crate::bridge::load_config(&r)?;
    }
    Ok(json!(names))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_requirements_are_refused_before_writing() {
        assert!(PrivacyRequirement::parse_list("no-training,mars-only").is_err());
        let ok = PrivacyRequirement::parse_list("eu-only,no-training").unwrap();
        assert_eq!(ok.len(), 2);
    }
}
