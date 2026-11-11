//! PDF page rendering for `Read`: page ranges, the system `pdftoppm` when present, otherwise the
//! `PDFium` library from Kor's own components (downloaded once, verified; ADR 0007).

use kor_platform::NoWindow as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use pdfium_render::prelude::{PdfRenderConfig, Pdfium};

/// Pages rendered when no range is given (same cap as one ranged request).
const MAX_PAGES: u32 = 20;
/// Longest rendered side in pixels; huge page boxes would otherwise allocate gigabytes.
const MAX_SIDE: i32 = 4000;

/// `1-5`, `3`, or `10-20`. Pages are 1-indexed. At most 20 pages.
pub fn parse_range(spec: &str) -> Result<(u32, u32), String> {
    let spec = spec.trim();
    let (first, last) = if let Some((a, b)) = spec.split_once('-') {
        (
            a.trim()
                .parse::<u32>()
                .map_err(|_| format!("invalid pages parameter: \"{spec}\""))?,
            b.trim()
                .parse::<u32>()
                .map_err(|_| format!("invalid pages parameter: \"{spec}\""))?,
        )
    } else {
        let n = spec
            .parse::<u32>()
            .map_err(|_| format!("invalid pages parameter: \"{spec}\""))?;
        (n, n)
    };
    if first == 0 || last < first {
        return Err(format!(
            "invalid pages parameter: \"{spec}\". Use formats like \"1-5\", \"3\", or \"10-20\". Pages are 1-indexed."
        ));
    }
    if last - first + 1 > MAX_PAGES {
        return Err(format!(
            "Page range \"{spec}\" exceeds maximum of 20 pages per request. Please use a smaller range."
        ));
    }
    Ok((first, last))
}

/// Arguments passed to `pdftoppm`, excluding the program name.
pub fn pdftoppm_args(pdf: &Path, prefix: &Path, range: Option<(u32, u32)>) -> Vec<String> {
    let mut args = vec!["-jpeg".into(), "-r".into(), "100".into()];
    if let Some((first, last)) = range {
        args.push("-f".into());
        args.push(first.to_string());
        args.push("-l".into());
        args.push(last.to_string());
    }
    args.push(pdf.display().to_string());
    args.push(prefix.display().to_string());
    args
}

pub fn pdftoppm_on_path() -> bool {
    Command::new("pdftoppm")
        .no_window()
        .arg("-v")
        .output()
        .is_ok_and(|o| o.status.success() || !o.stderr.is_empty())
}

/// One `PDFium` binding per process; `PDFium` itself is not thread-safe, so renders take the lock.
static PDFIUM: OnceLock<Result<Mutex<Pdfium>, String>> = OnceLock::new();

fn render_pdfium(
    lib: &Path,
    pdf: &Path,
    out: &Path,
    range: Option<(u32, u32)>,
) -> Result<usize, String> {
    let pdfium = PDFIUM
        .get_or_init(|| {
            Pdfium::bind_to_library(lib)
                .map(|b| Mutex::new(Pdfium::new(b)))
                .map_err(|e| format!("PDFium could not be loaded: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let pdfium = pdfium
        .lock()
        .map_err(|_| "PDFium is unavailable after an earlier failure".to_owned())?;
    let doc = pdfium
        .load_pdf_from_file(pdf, None)
        .map_err(|e| format!("PDF could not be opened: {e}"))?;
    let count = u32::try_from(doc.pages().len()).unwrap_or(0);
    let (first, last) = range.unwrap_or((1, count.min(MAX_PAGES)));
    // pdftoppm -r 100: points (1/72 in) to 100 DPI.
    let config = PdfRenderConfig::new()
        .scale_page_by_factor(100.0 / 72.0)
        .set_maximum_width(MAX_SIDE)
        .set_maximum_height(MAX_SIDE);
    let mut n = 0;
    for number in first..=last.min(count) {
        let index = i32::try_from(number - 1).map_err(|_| "page number too large".to_owned())?;
        let page = doc.pages().get(index).map_err(|e| e.to_string())?;
        let image = page
            .render_with_config(&config)
            .and_then(|b| b.as_image())
            .map_err(|e| format!("page {number} could not be rendered: {e}"))?;
        image
            .into_rgb8()
            .save(out.join(format!("page-{number}.jpg")))
            .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// Render pages of `pdf` as 100 DPI JPEGs into `out`. Uses `pdftoppm` when installed, otherwise
/// the `PDFium` component (fetched on first use; refused in privacy mode). Returns the page count.
pub async fn render(pdf: &Path, out: &Path, range: Option<(u32, u32)>) -> Result<usize, String> {
    // Pages of an earlier PDF must not be counted or shown again.
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    if pdftoppm_on_path() {
        let args = pdftoppm_args(pdf, &out.join("page"), range);
        let o = tokio::process::Command::new("pdftoppm")
            .no_window()
            .args(&args)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err(format!(
                "pdftoppm failed: {}",
                String::from_utf8_lossy(&o.stderr)
            ));
        }
        return Ok(std::fs::read_dir(out).map_or(0, |rd| rd.filter_map(Result::ok).count()));
    }
    // A Read the user's session already allowed counts as consent for this ~4 MB library (ADR 0007).
    let lib = kor_components::ensure("pdfium", &|_| {}).await?;
    let (pdf, out): (PathBuf, PathBuf) = (pdf.to_owned(), out.to_owned());
    tokio::task::spawn_blocking(move || render_pdfium(&lib, &pdf, &out, range))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_command_line() {
        assert_eq!(parse_range("3").unwrap(), (3, 3));
        assert_eq!(parse_range("1-5").unwrap(), (1, 5));
        assert!(parse_range("0").is_err());
        assert!(parse_range("1-30").unwrap_err().contains("20"));
        let args = pdftoppm_args(
            Path::new("a.pdf"),
            &Path::new("out").join("page"),
            Some((1, 2)),
        );
        assert_eq!(&args[..6], ["-jpeg", "-r", "100", "-f", "1", "-l"]);
        assert_eq!(args[6], "2");
        assert!(args[7].ends_with("a.pdf"), "{args:?}");
        assert!(args[8].ends_with("page"), "{args:?}");
    }

    /// Renders a real one-page PDF when the `PDFium` component is already installed
    /// (`kor component add pdfium`); skipped otherwise so tests never download.
    #[tokio::test]
    async fn renders_with_installed_pdfium() {
        let Some(lib) = kor_components::path("pdfium") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, ONE_PAGE).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let n = render_pdfium(&lib, &pdf, &out, None).unwrap();
        assert_eq!(n, 1);
        let jpg = std::fs::read(out.join("page-1.jpg")).unwrap();
        assert_eq!(&jpg[..2], [0xFF, 0xD8]);
    }

    const ONE_PAGE: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
}
