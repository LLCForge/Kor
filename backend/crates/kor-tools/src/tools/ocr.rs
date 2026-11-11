//! `OCR`: text and line positions from an image, on every OS without a manual install:
//! Windows uses its built-in engine (`Windows.Media.Ocr`, driven through Windows `PowerShell` 5.1),
//! macOS the built-in Vision framework (through `osascript -l JavaScript`), Linux `tesseract` from
//! `PATH` or, if missing, the `tesseract` component Kor downloads on first use. No network
//! otherwise. All three give the same output: one `[x,y wxh] text` line per text line.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use kor_core::{ImageSource, ToolSpec};
use kor_platform::Os;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::fs::resolve_in;
use super::winps;
use crate::jail::rel;
use crate::{Tool, ToolCtx, ToolKind, ToolOutput, field, parse};

const SCRIPT: &str = include_str!("ocr.ps1");
const MAC_SCRIPT: &str = include_str!("ocr_mac.js");
const TIMEOUT: Duration = Duration::from_secs(60);
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff"];

#[derive(Debug)]
pub(crate) struct Ocr;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcrIn {
    #[serde(alias = "path")]
    file_path: String,
    language: Option<String>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(default)]
    text: String,
    #[serde(default)]
    x: i64,
    #[serde(default)]
    y: i64,
    #[serde(default)]
    w: i64,
    #[serde(default)]
    h: i64,
}

#[derive(Deserialize)]
struct Recognized {
    #[serde(default)]
    lang: String,
    // ConvertTo-Json writes a one-item array as the item and an empty one as null.
    #[serde(default)]
    lines: Value,
}

/// Lines as `[x,y wxh] text`, so a result can guide a click on a screenshot.
fn render(lang: &str, lines: &[Line]) -> String {
    if lines.is_empty() {
        return format!("No text found (OCR language: {lang}).");
    }
    let mut out = format!("OCR language: {lang}\n");
    for l in lines {
        let _ = writeln!(out, "[{},{} {}x{}] {}", l.x, l.y, l.w, l.h, l.text);
    }
    out
}

/// The JSON the Windows and macOS scripts print.
fn format(raw: &str) -> Result<String, String> {
    let r: Recognized = serde_json::from_str(raw.trim())
        .map_err(|_| "OCR output could not be parsed".to_owned())?;
    let lines: Vec<Line> = match r.lines {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        Value::Null => Vec::new(),
        one => serde_json::from_value(one).into_iter().collect(),
    };
    Ok(render(&r.lang, &lines))
}

/// `tesseract … tsv`: one row per word; words of one (page, block, paragraph, line) become a line.
fn format_tsv(lang: &str, tsv: &str) -> String {
    let mut lines: Vec<Line> = Vec::new();
    let mut key: Option<[&str; 4]> = None;
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        let num = |i: usize| cols[i].trim().parse::<i64>().unwrap_or(0);
        let word = Line {
            text: text.to_owned(),
            x: num(6),
            y: num(7),
            w: num(8),
            h: num(9),
        };
        let line_key = [cols[1], cols[2], cols[3], cols[4]];
        match lines.last_mut() {
            Some(line) if key == Some(line_key) => {
                let right = (line.x + line.w).max(word.x + word.w);
                let bottom = (line.y + line.h).max(word.y + word.h);
                line.x = line.x.min(word.x);
                line.y = line.y.min(word.y);
                line.w = right - line.x;
                line.h = bottom - line.y;
                line.text.push(' ');
                line.text.push_str(&word.text);
            }
            _ => lines.push(word),
        }
        key = Some(line_key);
    }
    render(lang, &lines)
}

fn valid_language(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 35 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// BCP-47 tag → tesseract language code.
fn tesseract_code(tag: &str) -> Option<&'static str> {
    let lower = tag.to_ascii_lowercase();
    let primary = lower.split('-').next().unwrap_or("");
    Some(match primary {
        "en" => "eng",
        "tr" => "tur",
        "fr" => "fra",
        "de" => "deu",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "pl" => "pol",
        "ru" => "rus",
        "uk" => "ukr",
        "ar" => "ara",
        "fa" => "fas",
        "el" => "ell",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" if lower.contains("hant") || lower.ends_with("-tw") || lower.ends_with("-hk") => {
            "chi_tra"
        }
        "zh" => "chi_sim",
        _ => return None,
    })
}

/// `tesseract --list-langs` prints a header line, then one code per line.
fn parse_langs(out: &str) -> Vec<String> {
    out.lines()
        .skip_while(|l| !l.contains("anguages"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Languages to ask for: the requested one, else English plus Turkish when it is installed.
fn pick_langs(requested: Option<&str>, installed: &[String]) -> Result<String, String> {
    let have = |c: &str| installed.iter().any(|l| l == c);
    if let Some(tag) = requested {
        let code =
            tesseract_code(tag).ok_or_else(|| format!("OCR language '{tag}' is not supported"))?;
        if !have(code) {
            return Err(format!(
                "OCR language '{tag}' ({code}) is not installed. Installed: {}",
                installed.join(", ")
            ));
        }
        return Ok(code.to_owned());
    }
    let mut langs: Vec<&str> = ["eng", "tur"].into_iter().filter(|c| have(c)).collect();
    if langs.is_empty() {
        langs = installed
            .iter()
            .map(String::as_str)
            .filter(|l| *l != "osd")
            .take(1)
            .collect();
    }
    if langs.is_empty() {
        return Err("tesseract has no language data installed".into());
    }
    Ok(langs.join("+"))
}

fn on_path(name: &str) -> Option<PathBuf> {
    let exe = kor_platform::exe(name);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// Arguments for one recognition run.
fn tesseract_args(image: &Path, langs: &str, tessdata: Option<&Path>) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = vec![image.into(), "stdout".into()];
    if let Some(dir) = tessdata {
        a.push("--tessdata-dir".into());
        a.push(dir.into());
    }
    for s in ["-l", langs, "--psm", "3", "tsv"] {
        a.push(s.into());
    }
    a
}

/// Directory holding a downloaded language's `.traineddata`.
fn lang_dir(component: &Path) -> PathBuf {
    if component.is_file() {
        component
            .parent()
            .map_or_else(|| component.to_owned(), Path::to_owned)
    } else {
        component.to_owned()
    }
}

async fn tesseract(
    image: &Path,
    language: Option<&str>,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let quiet = |_: kor_components::Progress| {};
    let (exe, tessdata, installed) = if let Some(exe) = on_path("tesseract") {
        let out = winps::exec(
            &exe.clone().into_os_string(),
            &["--list-langs".into()],
            &[],
            None,
            TIMEOUT,
            cancel,
        )
        .await?;
        (exe, None, parse_langs(&out))
    } else {
        let exe = kor_components::ensure("tesseract", &quiet).await?;
        let eng = lang_dir(&kor_components::ensure("tesseract-lang-eng", &quiet).await?);
        let mut installed = vec!["eng".to_owned()];
        let wants_tur = language.is_none_or(|t| tesseract_code(t) == Some("tur"));
        if wants_tur
            && let Ok(tur) = kor_components::ensure("tesseract-lang-tur", &quiet).await
            && lang_dir(&tur) == eng
        {
            installed.push("tur".into());
        }
        (exe, Some(eng), installed)
    };
    let langs = pick_langs(language, &installed)?;
    let args = tesseract_args(image, &langs, tessdata.as_deref());
    let tsv = winps::exec(&exe.into_os_string(), &args, &[], None, TIMEOUT, cancel).await?;
    Ok(format_tsv(&langs, &tsv))
}

/// Text lines of an image file with the OS's OCR engine.
pub(crate) async fn ocr_file(
    path: &Path,
    language: Option<&str>,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let path_text = path.to_string_lossy();
    let env = [
        ("KOR_OCR_PATH", path_text.as_ref()),
        ("KOR_OCR_LANG", language.unwrap_or("")),
    ];
    match kor_platform::os() {
        Os::Windows => format(&winps::run(SCRIPT, &env, TIMEOUT, cancel).await?),
        Os::Mac => {
            let (exe, args) = winps::osascript(MAC_SCRIPT);
            format(&winps::exec(&exe, &args, &env, None, TIMEOUT, cancel).await?)
        }
        Os::Linux => tesseract(path, language, cancel).await,
        Os::Other => Err("OCR is not available on this operating system".into()),
    }
}

/// OCR text of an image the model cannot see (a model without vision). `Url` images are not
/// fetched.
///
/// # Errors
/// When the image cannot be decoded or no OCR engine is available.
pub async fn image_text(
    source: &ImageSource,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let ImageSource::Base64 { media_type, data } = source else {
        return Err("image is a URL; OCR reads inline images only".into());
    };
    let ext = match media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tif",
        other => return Err(format!("OCR cannot read {other} images")),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|_| "image data is not valid base64".to_owned())?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = std::env::temp_dir().join(format!("kor-ocr-{}-{nanos}.{ext}", std::process::id()));
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| format!("could not write a temporary image: {e}"))?;
    let text = ocr_file(&tmp, None, cancel).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    text
}

#[async_trait]
impl Tool for Ocr {
    fn name(&self) -> &'static str {
        "OCR"
    }
    fn rule_name(&self) -> &'static str {
        "Read"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: "Extract text from an image file (png, jpg, bmp, gif, tiff) with the OCR engine of the OS (Windows OCR, macOS Vision, tesseract on Linux). Returns one line per text line with its pixel box `[x,y wxh]`. Use it for screenshots, scanned pages, or when you need exact text and positions rather than a visual description.".into(),
            input_schema: json!({"type": "object", "properties": {
                "file_path": {"type": "string", "description": "Absolute path, or a path relative to the project root"},
                "language": {"type": "string", "description": "BCP-47 tag such as en-US or tr-TR; default: the user's languages"}
            }, "required": ["file_path"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        field(input, &["file_path", "path"]).map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: OcrIn = match parse(input) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let path = match resolve_in(ctx, &a.file_path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !EXTENSIONS.contains(&ext.as_str()) {
            return ToolOutput::err(format!("OCR reads image files: {}", EXTENSIONS.join(", ")));
        }
        if !path.is_file() {
            return ToolOutput::err(format!("{} does not exist", rel(&ctx.root, &path)));
        }
        if let Some(tag) = &a.language
            && !valid_language(tag)
        {
            return ToolOutput::err("language must be a BCP-47 tag such as en-US");
        }
        match ocr_file(&path, a.language.as_deref(), &ctx.cancel).await {
            Ok(text) => ToolOutput::ok(text),
            Err(e) => ToolOutput::err(format!("OCR: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_powershell_and_vision_shapes() {
        let many = r#"{"lang":"en-US","lines":[{"text":"Hello","x":1,"y":2,"w":30,"h":10},{"text":"World","x":1,"y":14,"w":31,"h":10}]}"#;
        assert_eq!(
            format(many).unwrap(),
            "OCR language: en-US\n[1,2 30x10] Hello\n[1,14 31x10] World\n"
        );
        let one = r#"{"lang":"tr-TR","lines":{"text":"Merhaba","x":0,"y":0,"w":5,"h":5}}"#;
        assert!(format(one).unwrap().ends_with("[0,0 5x5] Merhaba\n"));
        assert!(
            format(r#"{"lang":"en-US","lines":null}"#)
                .unwrap()
                .starts_with("No text found")
        );
        assert!(format("not json").is_err());
        let vision = include_str!("fixtures/ocr_vision.json");
        assert_eq!(
            format(vision).unwrap(),
            "OCR language: auto\n[12,8 260x22] Kor reads this line\n[12,40 140x20] İkinci satır\n"
        );
    }

    #[test]
    fn groups_tesseract_words_into_lines() {
        let tsv = include_str!("fixtures/ocr_tesseract.tsv");
        assert_eq!(
            format_tsv("eng+tur", tsv),
            "OCR language: eng+tur\n[10,20 205x30] Kor reads this\n[10,60 90x25] İkinci satır\n"
        );
        assert!(format_tsv("eng", "level\tpage_num\n").starts_with("No text found"));
    }

    #[test]
    fn tesseract_languages_and_arguments() {
        let langs = parse_langs(
            "List of available languages in \"/usr/share/tesseract-ocr/5/tessdata/\" (3):\neng\nosd\ntur\n",
        );
        assert_eq!(langs, ["eng", "osd", "tur"]);
        assert_eq!(pick_langs(None, &langs).unwrap(), "eng+tur");
        assert_eq!(pick_langs(Some("tr-TR"), &langs).unwrap(), "tur");
        assert!(
            pick_langs(Some("de-DE"), &langs)
                .unwrap_err()
                .contains("not installed")
        );
        assert!(
            pick_langs(Some("xx"), &langs)
                .unwrap_err()
                .contains("not supported")
        );
        assert_eq!(
            pick_langs(None, &["osd".into(), "fra".into()]).unwrap(),
            "fra"
        );
        assert_eq!(tesseract_code("zh-TW"), Some("chi_tra"));
        let args = tesseract_args(Path::new("a.png"), "eng", Some(Path::new("/td")));
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "a.png",
                "stdout",
                "--tessdata-dir",
                "/td",
                "-l",
                "eng",
                "--psm",
                "3",
                "tsv"
            ]
        );
    }

    #[test]
    fn vision_script_reads_inputs_from_the_environment_only() {
        assert!(
            MAC_SCRIPT.contains("KOR_OCR_PATH") && MAC_SCRIPT.contains("VNRecognizeTextRequest")
        );
        assert!(MAC_SCRIPT.contains("function run()"));
    }

    #[tokio::test]
    async fn image_text_rejects_urls_and_bad_data() {
        let cancel = CancellationToken::new();
        let url = ImageSource::Url {
            url: "https://x/y.png".into(),
        };
        assert!(image_text(&url, &cancel).await.is_err());
        let bad = ImageSource::Base64 {
            media_type: "image/png".into(),
            data: "%%%".into(),
        };
        assert!(
            image_text(&bad, &cancel)
                .await
                .unwrap_err()
                .contains("base64")
        );
        let webp = ImageSource::Base64 {
            media_type: "image/webp".into(),
            data: String::new(),
        };
        assert!(image_text(&webp, &cancel).await.is_err());
    }

    /// Draws text with System.Drawing and reads it back. `cargo test -p kor-tools -- --ignored ocr`
    #[tokio::test]
    #[ignore = "needs the Windows OCR engine and an installed OCR language"]
    async fn windows_ocr_reads_rendered_text() {
        if kor_platform::os() != Os::Windows {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let png = ctx.root.join("t.png");
        let draw = "Add-Type -AssemblyName System.Drawing; $b=New-Object Drawing.Bitmap 520,90; $g=[Drawing.Graphics]::FromImage($b); $g.Clear([Drawing.Color]::White); $g.DrawString('Kor reads this line', (New-Object Drawing.Font 'Arial',28), [Drawing.Brushes]::Black, 10, 20); $b.Save($env:P)";
        let ok = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", draw])
            .env("P", &png)
            .status()
            .unwrap();
        assert!(ok.success());
        let out = Ocr.call(json!({"file_path": "t.png"}), &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("Kor reads this line"),
            "{}",
            out.content
        );
        let bytes = std::fs::read(&png).unwrap();
        let src = ImageSource::Base64 {
            media_type: "image/png".into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        let text = image_text(&src, &ctx.cancel).await.unwrap();
        assert!(text.contains("Kor reads this line"), "{text}");
    }

    #[test]
    fn language_tags_are_checked() {
        assert!(valid_language("tr-TR") && valid_language("en"));
        assert!(!valid_language("en'; Remove-Item") && !valid_language(""));
    }
}
