//! `visual.*`: görsel ve çok kipli girdi (§10, ADR 0048) için ince köprü. İş `kor_runtime::visual`
//! içinde; burası yalnızca JSON'a çevirir.
//!
//! - `visual.screenshot(dataUrl, name, target, model?)` / `visual.diagram(dataUrl, name, model?)` →
//!   `{ vision, description, reply, provider, files: [{ path, before, after }] }`. Hiçbir şey yazılmaz;
//!   arayüz dosya dosya onaylatır (ADR 0030) ve kabul edilenleri `visual.write` ile yazar.
//! - `visual.write(files: [{ path, content }])` → `{ written: string[] }` (proje kökü altında, jail ile).
//! - `visual.visualize(path?, format?)` → `{ main, files, counts }`; HTML tarayıcıda açılır.
//! - `visual.voiceStatus()` → `{ ready, whisper, model, missing: [{ name, version, size }] }`.
//! - `visual.voice(seconds?, language?)` → `{ text }` (mikrofon → yerel whisper.cpp; ses dışarı çıkmaz).
//!
//! Sohbete yapıştırılan görseller `bridge.rs` `g2Send` içinde `visual::with_pasted_images` ile yerel
//! OCR metnine çevrilir.

use std::path::{Path, PathBuf};

use kor_providers::CancellationToken;
use kor_runtime::visual::{self, GenFile, Kind, Picture, SystemOcr, Target, map, voice};
use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::api::{Res, opt_s, root, s};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `visual::call`'a yönlendirir).
pub const NS: &str = "visual";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "screenshot" => {
            let target = opt_s(args, 2).unwrap_or("react").parse::<Target>();
            match (picture(args), target, root(state)) {
                (Ok(p), Ok(t), Ok(dir)) => {
                    generate(&dir, &p, Kind::Screenshot(t), opt_s(args, 3)).await
                }
                (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => Err(e),
            }
        }
        "diagram" => match (picture(args), root(state)) {
            (Ok(p), Ok(dir)) => generate(&dir, &p, Kind::Diagram, opt_s(args, 2)).await,
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "write" => match (files(args.first()), root(state)) {
            (Ok(f), Ok(dir)) => visual::write(&dir, &f)
                .await
                .map(|w| json!({ "written": w }))
                .map_err(|e| format!("Dosyalar yazılamadı: {e}")),
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "visualize" => match root(state) {
            Ok(dir) => {
                let r = visualize(&dir, opt_s(args, 0), opt_s(args, 1).unwrap_or("html"));
                if let Ok(v) = &r
                    && v["format"] == "html"
                    && let Some(main) = v["main"].as_str()
                {
                    let _ = app.opener().open_path(main, None::<&str>);
                }
                r
            }
            Err(e) => Err(e),
        },
        "voiceStatus" => Ok(voice_status()),
        "voice" => {
            let secs = args
                .first()
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(10)
                .clamp(1, voice::MAX_SECONDS);
            voice::listen(secs, opt_s(args, 1), &CancellationToken::new())
                .await
                .map(|text| json!({ "text": text }))
                .map_err(|e| format!("Ses metne çevrilemedi: {e}"))
        }
        _ => return None,
    })
}

fn picture(args: &[Value]) -> Result<Picture, String> {
    let url = s(args, 0, "visual")?;
    let name = opt_s(args, 1).unwrap_or("görsel.png");
    Picture::from_data_url(name, url).map_err(|e| format!("Görsel okunamadı: {e}"))
}

fn files(v: Option<&Value>) -> Result<Vec<GenFile>, String> {
    let list = v.and_then(Value::as_array).ok_or("Dosya listesi eksik")?;
    list.iter()
        .map(|f| match (f["path"].as_str(), f["content"].as_str()) {
            (Some(path), Some(content)) => Ok(GenFile {
                path: path.to_owned(),
                content: content.to_owned(),
            }),
            _ => Err("Her dosyanın `path` ve `content` alanı olmalı".to_owned()),
        })
        .collect()
}

async fn generate(dir: &Path, pic: &Picture, kind: Kind, model: Option<&str>) -> Res {
    // Gizlilik modu: yerel olmayan sağlayıcıyı oneshot_provider reddeder.
    let (provider, model) = crate::bridge::load_config(dir).and_then(|loaded| {
        let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
        kor_runtime::oneshot_provider(&l, &loaded, model)
    })?;
    let g = visual::generate(&provider, &model, pic, kind, &SystemOcr, &CancellationToken::new())
        .await
        .map_err(|e| format!("Görselden kod üretilemedi: {e}"))?;
    let files: Vec<Value> = visual::proposals(dir, &g.files)
        .into_iter()
        .map(|p| json!({ "path": p.path, "before": p.before, "after": p.after }))
        .collect();
    Ok(json!({
        "vision": g.vision,
        "description": g.description,
        "reply": g.reply,
        "provider": provider.manifest().name,
        "files": files,
    }))
}

fn visualize(dir: &Path, path: Option<&str>, format: &str) -> Res {
    let format: map::Format = format.parse()?;
    let mut g = kor_graph::Graph::open(dir).map_err(|e| format!("Proje grafı açılamadı: {e}"))?;
    g.update()
        .map_err(|e| format!("Proje grafı güncellenemedi: {e}"))?;
    let scope = map::scope_of(dir, path.filter(|p| !p.is_empty()).map(Path::new));
    let views = map::views(&g, &scope);
    let w = map::write(&views, format, &map::default_dir(dir))?;
    let counts: Vec<Value> = w
        .counts
        .iter()
        .map(|(k, n, e, o)| json!({ "view": k, "title": k.title(), "nodes": n, "edges": e, "omitted": o }))
        .collect();
    Ok(json!({
        "format": format,
        "main": w.main.to_string_lossy(),
        "files": w.files.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        "counts": counts,
    }))
}

fn voice_status() -> Value {
    let st = voice::status();
    let path = |p: &Option<PathBuf>| p.as_ref().map(|p| p.to_string_lossy().into_owned());
    json!({
        "ready": st.ready(),
        "whisper": path(&st.whisper),
        "model": path(&st.model),
        "missing": st.missing.iter().map(|(n, v, size)| json!({ "name": n, "version": v, "size": size })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1x1 PNG.
    const DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";

    #[test]
    fn arguments_are_checked() {
        assert!(picture(&[json!(DATA_URL), json!("a.png")]).is_ok());
        assert!(picture(&[json!("https://x/a.png")]).unwrap_err().contains("Görsel okunamadı"));
        assert!(picture(&[]).is_err());
        let f = files(Some(&json!([{ "path": "src/a.tsx", "content": "x" }]))).unwrap();
        assert_eq!(f[0].path, "src/a.tsx");
        assert!(files(Some(&json!([{ "path": "a" }]))).is_err());
        assert!(files(None).is_err());
    }

    #[test]
    fn visualize_writes_the_views() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("main.rs"), "fn a() { b(); }\nfn b() {}\n").unwrap();
        let v = visualize(d.path(), None, "mermaid").unwrap();
        assert_eq!(v["format"], "mermaid");
        assert!(v["main"].as_str().unwrap().ends_with("visualize.md"));
        assert_eq!(v["counts"].as_array().unwrap().len(), 3);
        assert!(visualize(d.path(), None, "png").is_err());
    }

    #[test]
    fn voice_status_lists_what_is_missing() {
        let v = voice_status();
        assert!(v["ready"].is_boolean() && v["missing"].is_array());
    }
}
