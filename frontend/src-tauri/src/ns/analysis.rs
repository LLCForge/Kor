//! `analysis.*`: `kor audit --security|--compliance` ve rapor dışa aktarma/imzalama (§9, §9.1,
//! ADR 0045) için ince köprü. Tarama `kor_runtime::analysis` içinde; burası yalnızca JSON'a çevirir.
//!
//! - `analysis.security(noExternal?: bool)` → `{ report }`
//! - `analysis.compliance(regime: "kvkk" | "gdpr")` → `{ report }`
//! - `analysis.export(tool: string, format: "md" | "json" | "sarif", sign?: bool)` →
//!   `{ path, sha256, signature }` (kaydetme penceresi; iptalde `path: null`). Dışa aktarılan,
//!   arayüzün gönderdiği veri değil, bu süreçte son üretilen rapordur (imza ona atılır).
//! - `analysis.verifyReport(path?: string)` → `{ path, result }` (yol yoksa açma penceresi)
//!
//! `report` alanı `Report` sözleşmesidir: `{ tool, findings: [{ rule, source, severity, file, line,
//! column?, message, fix?, cwe? }], notes }`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use kor_runtime::analysis::{self, Format, Report, compliance::Regime};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `analysis::call`'a yönlendirir).
pub const NS: &str = "analysis";

/// Son raporlar (araç adına göre); `export` yalnızca bunları yazar.
static LAST: Mutex<Option<HashMap<String, Report>>> = Mutex::new(None);

pub(crate) fn remember(r: &Report) {
    LAST.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(r.tool.clone(), r.clone());
}

fn last(tool: &str) -> Option<Report> {
    LAST.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(tool).cloned())
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "security" => match root(state) {
            Ok(dir) => {
                let no_external = b(args, 0);
                let l = localizer(&dir);
                blocking(move || security_at(&dir, no_external, &l)).await
            }
            Err(e) => Err(e),
        },
        "compliance" => match (regime(args), root(state)) {
            (Ok(rg), Ok(dir)) => {
                let l = localizer(&dir);
                blocking(move || compliance_at(&dir, rg, &l)).await
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "export" => export(args, app, state).await,
        "verifyReport" => verify(args, app, state).await,
        _ => return None,
    })
}

fn localizer(dir: &Path) -> kor_i18n::Localizer {
    let lang = crate::bridge::load_config(dir)
        .map_or(kor_i18n::Lang::Tr, |l| l.config.ui.language.resolve());
    kor_i18n::Localizer::new(lang)
}

async fn blocking(f: impl FnOnce() -> Res + Send + 'static) -> Res {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Analiz çalıştırılamadı: {e}"))?
}

fn regime(args: &[Value]) -> Result<Regime, String> {
    s(args, 0, "analysis.compliance")
        .ok()
        .and_then(Regime::parse)
        .ok_or_else(|| "Uyum rejimi \"kvkk\" ya da \"gdpr\" olmalı".into())
}

fn format(args: &[Value]) -> Result<Format, String> {
    s(args, 1, "analysis.export")
        .ok()
        .and_then(Format::parse)
        .ok_or_else(|| "Biçim \"md\", \"json\" ya da \"sarif\" olmalı".into())
}

pub(crate) fn security_at(dir: &Path, no_external: bool, l: &kor_i18n::Localizer) -> Res {
    let r = analysis::security::scan(dir, &analysis::security::Options { no_external }, l);
    remember(&r);
    Ok(json!({ "report": r }))
}

pub(crate) fn compliance_at(dir: &Path, rg: Regime, l: &kor_i18n::Localizer) -> Res {
    let r = analysis::compliance::scan(dir, rg, l);
    remember(&r);
    Ok(json!({ "report": r }))
}

/// Rapor metni: `tool` için son rapor, istenen biçimde.
pub(crate) fn rendered(
    tool: &str,
    fmt: Format,
    l: &kor_i18n::Localizer,
) -> Result<(Report, String), String> {
    let r = last(tool).ok_or("Önce analizi çalıştırın")?;
    let text = analysis::render(&r, fmt, l);
    Ok((r, text))
}

async fn export(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let tool = s(args, 0, "analysis.export")?.to_owned();
    let fmt = format(args)?;
    let sign = b(args, 2);
    let dir = root(state)?;
    let l = localizer(&dir);
    let (report, text) = rendered(&tool, fmt, &l)?;
    let name = format!("kor-audit-{tool}.{}", fmt.ext());
    let Some(path) = crate::api::save_dialog(app, Some(&dir), &name).await else {
        return Ok(json!({ "path": null }));
    };
    std::fs::write(&path, text).map_err(|e| format!("Dosya yazılamadı: {e}"))?;
    if !sign {
        return Ok(json!({ "path": path.to_string_lossy(), "sha256": null, "signature": null }));
    }
    let out = analysis::sign_and_link(&report, &path)?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "sha256": out.sha256,
        "auditHash": out.audit_hash,
        "signature": out.signature.map(|p| p.to_string_lossy().into_owned()),
    }))
}

async fn verify(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let path = match opt_s(args, 0).filter(|p| !p.trim().is_empty()) {
        Some(p) => p.into(),
        None => {
            let dir = root(state).ok();
            match crate::api::open_dialog(app, dir.as_deref()).await {
                Some(p) => p,
                None => return Ok(json!({ "path": null })),
            }
        }
    };
    let result = analysis::verify_report(&path)?;
    Ok(json!({ "path": path.to_string_lossy(), "result": result, "ok": result.ok() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_renders_the_last_report() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("app.py"),
            "import subprocess\nsubprocess.run(cmd, shell=True)\nlogging.info(user.email)\n",
        )
        .unwrap();
        let l = kor_i18n::Localizer::new(kor_i18n::Lang::Tr);
        let v = security_at(d.path(), true, &l).unwrap();
        let f = &v["report"]["findings"][0];
        assert_eq!(f["rule"], "kor.command-injection.python-shell");
        assert_eq!(
            (f["file"].as_str(), f["line"].as_u64()),
            (Some("app.py"), Some(2))
        );
        let (_, sarif) = rendered("security", Format::Sarif, &l).unwrap();
        assert!(sarif.contains("\"2.1.0\""));
        let c = compliance_at(d.path(), Regime::Gdpr, &l).unwrap();
        assert_eq!(c["report"]["tool"], "compliance-gdpr");
        assert!(rendered("nope", Format::Json, &l).is_err());
    }

    #[test]
    fn argument_validation() {
        assert!(regime(&[json!("kvkk")]).is_ok());
        assert!(regime(&[json!("ccpa")]).is_err() && regime(&[]).is_err());
        assert!(format(&[json!("security"), json!("sarif")]).is_ok());
        assert!(format(&[json!("security"), json!("pdf")]).is_err());
    }
}
