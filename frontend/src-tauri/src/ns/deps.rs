//! `deps.*`: `kor deps list|check` ve `kor audit --licenses` (§9, ADR 0046) için ince köprü.
//! Kilit dosyası okuma, lisans sınıflandırma, osv.dev ve kayıt sorguları
//! `kor_runtime::analysis::deps` içinde; burası yalnızca JSON'a çevirir.
//!
//! - `deps.list()` → `{ packages: [{ ecosystem, name, version, direct, dev, license?, license_from?,
//!   file, line, via?, class }], sources, errors }`
//! - `deps.licenses()` → `{ report }` (`[audit.licenses] deny` politikasıyla)
//! - `deps.check(offline?: bool)` → `{ report }`; gizlilik/çevrimdışı modda ağ kısmı reddedilir ve
//!   notlarda yerelleştirilmiş mesaj olur, `offline: true` son önbelleği yaşıyla kullanır.
//!
//! Raporlar `analysis.export("licenses" | "deps", …)` ile dışa aktarılabilsin diye
//! `analysis::remember` ile saklanır.

use std::path::Path;

use kor_runtime::analysis::deps::{self, check, license};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, root};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `deps::call`'a yönlendirir).
pub const NS: &str = "deps";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let dir = match root(state) {
        Ok(d) => d,
        Err(e) => return Some(Err(e)),
    };
    let offline = b(args, 0);
    Some(match name {
        "list" => blocking(move || Ok(list_at(&dir))).await,
        "licenses" => blocking(move || Ok(licenses_at(&dir))).await,
        "check" => blocking(move || Ok(check_at(&dir, offline))).await,
        _ => return None,
    })
}

async fn blocking(f: impl FnOnce() -> Res + Send + 'static) -> Res {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Bağımlılık analizi çalıştırılamadı: {e}"))?
}

/// Proje (ve kullanıcı) ayarları: arayüz dili ve lisans yasak listesi.
fn settings(dir: &Path) -> (kor_i18n::Localizer, Vec<String>) {
    match crate::bridge::load_config(dir) {
        Ok(l) => (
            kor_i18n::Localizer::new(l.config.ui.language.resolve()),
            l.config.audit.licenses.deny,
        ),
        Err(_) => (kor_i18n::Localizer::new(kor_i18n::Lang::Tr), Vec::new()),
    }
}

pub(crate) fn list_at(dir: &Path) -> Value {
    let inv = deps::scan(dir);
    json!({
        "packages": license::classified(&inv),
        "sources": inv.sources,
        "errors": inv.errors,
    })
}

pub(crate) fn licenses_at(dir: &Path) -> Value {
    let (l, deny) = settings(dir);
    let r = license::licenses_report(&deps::scan(dir), &deny, &l);
    super::analysis::remember(&r);
    json!({ "report": r })
}

pub(crate) fn check_at(dir: &Path, offline: bool) -> Value {
    let (l, _) = settings(dir);
    let inv = deps::scan(dir);
    let opts = check::Options {
        offline,
        endpoints: check::Endpoints::default(),
        cache_dir: deps::cache_dir(),
    };
    let r = tauri::async_runtime::block_on(check::check_report(dir, &inv, &opts, &l));
    super::analysis::remember(&r);
    json!({ "report": r })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("package.json"),
            "{\n  \"dependencies\": {\n    \"agpl-lib\": \"^1.0.0\"\n  }\n}\n",
        )
        .unwrap();
        std::fs::write(
            d.path().join("package-lock.json"),
            r#"{ "lockfileVersion": 3, "packages": {
  "": { "dependencies": { "agpl-lib": "^1.0.0" } },
  "node_modules/agpl-lib": { "version": "1.0.0", "license": "AGPL-3.0-only" } } }"#,
        )
        .unwrap();
        d
    }

    #[test]
    fn list_and_licenses_have_file_line() {
        let d = project();
        let v = list_at(d.path());
        let p = &v["packages"][0];
        assert_eq!(p["name"], "agpl-lib");
        assert_eq!(p["class"], "strong-copyleft");
        assert_eq!(
            (p["file"].as_str(), p["line"].as_u64()),
            (Some("package.json"), Some(3))
        );
        let r = &licenses_at(d.path())["report"];
        assert_eq!(r["tool"], "licenses");
        assert_eq!(r["findings"][0]["rule"], "license-strong-copyleft");
        assert_eq!(r["findings"][0]["source"], "spdx");
    }

    #[test]
    fn offline_check_without_cache_runs_local_checks_only() {
        let d = project();
        let r = &check_at(d.path(), true)["report"];
        assert_eq!(r["tool"], "deps");
        assert!(
            r["findings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["source"] != "osv")
        );
    }
}
