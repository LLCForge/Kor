//! `webview.*`: WebView kilidinin (§21, ADR 0053) arayüze açtığı küçük köprü.
//!
//! - `webview.devtools()` → `bool` (geliştirici build'i + `KOR_DEVTOOLS=1` / `--devtools`)
//! - `webview.closeNow()` → `null`: Kor'un "çalışan görev var" diyaloğu onaylandı; pencere soru sormadan kapanır
//! - `webview.saveImage(src: string)` → kaydedilen yol ya da `null` (iptal). `src`: `http://asset.localhost/…`,
//!   `asset://localhost/…` (yerel dosya) ya da `data:image/…;base64,…`. Başka kaynak reddedilir.
//! - `webview.sampleProject()` → örnek projenin yolu (`~/.kor/ornek-proje`; yoksa oluşturulur, varsa dokunulmaz).
//!   Karşılama ekranındaki "Örnek projeyle dene" çipi (§23.3) bunu açar.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use base64::Engine as _;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::api::{Res, s, save_dialog};
use crate::state::AppState;
use crate::webview_lock::{FORCE_CLOSE, devtools_allowed};

pub const NS: &str = "webview";

/// Kaydedilecek görsel en çok 64 MiB (veri URL'si arayüzden gelir: güven sınırı).
const MAX_IMAGE: usize = 64 * 1024 * 1024;

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "devtools" => Ok(json!(devtools_allowed())),
        "closeNow" => {
            FORCE_CLOSE.store(true, Ordering::SeqCst);
            match app.get_webview_window("main") {
                Some(w) => w.close().map(|()| Value::Null).map_err(|e| e.to_string()),
                None => Ok(Value::Null),
            }
        }
        "sampleProject" => sample_project().map(|p| json!(p.to_string_lossy())),
        "saveImage" => match s(args, 0, "webview.saveImage") {
            Ok(src) => save_image(app, state, src).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Source {
    File(PathBuf),
    Bytes { bytes: Vec<u8>, ext: &'static str },
}

/// Arayüzden gelen görsel kaynağını çözer; yalnızca yerel varlık ve görsel veri URL'si.
fn parse_source(src: &str) -> Result<Source, String> {
    if let Some(rest) = src.strip_prefix("data:image/") {
        let (mime, data) = rest
            .split_once(";base64,")
            .ok_or("Desteklenmeyen görsel verisi")?;
        let ext = match mime {
            "png" => "png",
            "jpeg" | "jpg" => "jpg",
            "gif" => "gif",
            "webp" => "webp",
            "svg+xml" => "svg",
            _ => return Err(format!("Desteklenmeyen görsel türü: {mime}")),
        };
        if data.len() > MAX_IMAGE / 3 * 4 {
            return Err("Görsel çok büyük".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| e.to_string())?;
        return Ok(Source::Bytes { bytes, ext });
    }
    let encoded = src
        .strip_prefix("http://asset.localhost/")
        .or_else(|| src.strip_prefix("https://asset.localhost/"))
        .or_else(|| src.strip_prefix("asset://localhost/"))
        .ok_or("Yalnızca yerel görseller kaydedilebilir")?;
    let path = PathBuf::from(percent_decode(encoded)?);
    if !path.is_absolute() {
        return Err("Geçersiz görsel yolu".into());
    }
    Ok(Source::File(path))
}

fn percent_decode(s: &str) -> Result<String, String> {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or("Geçersiz yol kodlaması")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "Geçersiz yol kodlaması")?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "Geçersiz yol kodlaması".into())
}

async fn save_image(app: &AppHandle, state: &AppState, src: &str) -> Res {
    let source = parse_source(src)?;
    let name = match &source {
        Source::File(p) => p.file_name().map_or_else(
            || "gorsel.png".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        ),
        Source::Bytes { ext, .. } => format!("gorsel.{ext}"),
    };
    let Some(dest) = save_dialog(app, state.root().as_deref(), &name).await else {
        return Ok(Value::Null);
    };
    match source {
        Source::File(p) => std::fs::copy(&p, &dest).map(|_| ()),
        Source::Bytes { bytes, .. } => std::fs::write(&dest, bytes),
    }
    .map_err(|e| e.to_string())?;
    Ok(json!(dest.to_string_lossy()))
}

/// Örnek proje: küçük, bilinçli bir hatası olan TypeScript modülü ve testi (ajan "Hata bul" / "Test yaz" ile denenir).
const SAMPLE: &[(&str, &str)] = &[
    ("README.md", include_str!("../../sample-project/README.md")),
    (
        "package.json",
        include_str!("../../sample-project/package.json"),
    ),
    (
        "src/sepet.ts",
        include_str!("../../sample-project/src/sepet.ts"),
    ),
    (
        "src/sepet.test.ts",
        include_str!("../../sample-project/src/sepet.test.ts"),
    ),
];

fn sample_project() -> Result<PathBuf, String> {
    let dir = kor_config::user_dir()
        .ok_or("Kullanıcı ayar klasörü bulunamadı")?
        .join("ornek-proje");
    for (rel, body) in SAMPLE {
        let path = dir.join(rel);
        // Kullanıcının düzenlemeleri korunur: var olan dosyaya yazılmaz.
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, body).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_project_has_a_bug_and_a_test() {
        let src = SAMPLE
            .iter()
            .find(|(p, _)| *p == "src/sepet.ts")
            .expect("sepet.ts")
            .1;
        assert!(src.contains("(1 - indirim) * 100"));
        assert!(
            SAMPLE
                .iter()
                .any(|(p, b)| *p == "src/sepet.test.ts" && b.contains("node:test"))
        );
        assert!(
            SAMPLE
                .iter()
                .all(|(p, _)| !p.starts_with('/') && !p.contains(".."))
        );
    }

    #[test]
    fn data_urls_decode_only_images() {
        assert_eq!(
            parse_source("data:image/png;base64,iVBORw0K"),
            Ok(Source::Bytes {
                bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a],
                ext: "png"
            })
        );
        assert!(parse_source("data:text/html;base64,PGI+").is_err());
        assert!(parse_source("data:image/bmp;base64,AAAA").is_err());
    }

    #[test]
    fn asset_urls_map_to_absolute_paths_only() {
        let abs = std::env::temp_dir().join("b c.png");
        let enc: String = abs
            .to_string_lossy()
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b == b'.' {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        assert_eq!(
            parse_source(&format!("http://asset.localhost/{enc}")),
            Ok(Source::File(abs))
        );
        assert!(parse_source("https://example.com/x.png").is_err());
        assert!(parse_source("http://asset.localhost/rel%2Fx.png").is_err());
        assert!(parse_source("http://asset.localhost/%ZZ").is_err());
    }
}
