//! LLM-free layout of a UI or diagram image (spec §10): the OCR text boxes grouped into rows and
//! cells, plus the dominant colours, written as a plain description any text model can read.

use std::collections::HashMap;
use std::fmt::Write as _;

/// One OCR text line with its pixel box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBox {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub text: String,
}

/// Lines in the shape every Kor OCR backend prints: `[x,y wxh] text` (Windows OCR, macOS
/// Vision, tesseract TSV). Other lines (the language header, "No text found") are skipped.
pub fn parse_ocr(ocr: &str) -> Vec<TextBox> {
    ocr.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            let (geo, text) = rest.split_once("] ")?;
            let (xy, wh) = geo.split_once(' ')?;
            let (x, y) = xy.split_once(',')?;
            let (w, h) = wh.split_once('x')?;
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            Some(TextBox {
                x: x.trim().parse().ok()?,
                y: y.trim().parse().ok()?,
                w: w.trim().parse().ok()?,
                h: h.trim().parse().ok()?,
                text: text.to_owned(),
            })
        })
        .collect()
}

/// Boxes grouped into visual rows (top to bottom): a box joins a row when its vertical centre lies
/// inside the row's band. Each row is sorted left to right.
pub fn rows(boxes: &[TextBox]) -> Vec<Vec<TextBox>> {
    let mut sorted = boxes.to_vec();
    sorted.sort_by_key(|b| (b.y, b.x));
    let mut rows: Vec<Vec<TextBox>> = Vec::new();
    for b in sorted {
        let centre = b.y + b.h / 2;
        match rows.last_mut() {
            Some(row)
                if row
                    .iter()
                    .any(|r| centre >= r.y && centre <= r.y + r.h.max(1)) =>
            {
                row.push(b);
            }
            _ => rows.push(vec![b]),
        }
    }
    for r in &mut rows {
        r.sort_by_key(|b| b.x);
    }
    rows
}

/// Splits a row into cells where the horizontal gap is wider than about two line heights (a new
/// column, button or table cell rather than the next word).
pub fn cells(row: &[TextBox]) -> Vec<Vec<&TextBox>> {
    let mut out: Vec<Vec<&TextBox>> = Vec::new();
    let mut right = i64::MIN;
    for b in row {
        let gap_limit = (b.h * 2).max(24);
        if out.is_empty() || b.x - right > gap_limit {
            out.push(vec![b]);
        } else if let Some(cell) = out.last_mut() {
            cell.push(b);
        }
        right = right.max(b.x + b.w);
    }
    out
}

/// Image size and its most common colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub width: u32,
    pub height: u32,
    /// `#rrggbb` and share in percent, most common first.
    pub colors: Vec<(String, u32)>,
}

/// The most common colours (16 levels per channel on a 64 px thumbnail) with their share in
/// percent, most common first.
///
/// # Errors
/// When the bytes are not a decodable PNG or JPEG.
pub fn dominant_colors(bytes: &[u8], top: usize) -> Result<Palette, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("image could not be decoded: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let thumb = img.thumbnail(64, 64).to_rgb8();
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for p in thumb.pixels() {
        *counts.entry([p[0] >> 4, p[1] >> 4, p[2] >> 4]).or_default() += 1;
    }
    let total: u32 = counts.values().sum::<u32>().max(1);
    let mut list: Vec<([u8; 3], u32)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let colors = list
        .into_iter()
        .take(top)
        .map(|(c, n)| {
            let hex = format!(
                "#{:02x}{:02x}{:02x}",
                (c[0] << 4) | 8,
                (c[1] << 4) | 8,
                (c[2] << 4) | 8
            );
            (hex, n * 100 / total)
        })
        .collect();
    Ok(Palette {
        width: w,
        height: h,
        colors,
    })
}

/// The structured text a model without vision gets instead of the image.
pub fn describe(ocr: &str, size: (u32, u32), colors: &[(String, u32)]) -> String {
    let boxes = parse_ocr(ocr);
    let mut s = format!("Image size: {}x{} px\n", size.0, size.1);
    if let Some(lang) = ocr.lines().find_map(|l| l.strip_prefix("OCR language: ")) {
        let _ = writeln!(s, "OCR language: {lang}");
    }
    if !colors.is_empty() {
        let list: Vec<String> = colors.iter().map(|(c, p)| format!("{c} {p}%")).collect();
        let _ = writeln!(s, "Dominant colours: {}", list.join(", "));
    }
    if boxes.is_empty() {
        s.push_str("No text was found in the image.\n");
        return s;
    }
    s.push_str(
        "Text layout, top to bottom. One line per visual row; cells of a row are separated by ` | `; \
         x/y/w/h are pixels, h is roughly the font size:\n",
    );
    for (i, row) in rows(&boxes).iter().enumerate() {
        let top = row.iter().map(|b| b.y).min().unwrap_or(0);
        let cells: Vec<String> = cells(row)
            .iter()
            .map(|cell| {
                let x = cell.first().map_or(0, |b| b.x);
                let right = cell.iter().map(|b| b.x + b.w).max().unwrap_or(x);
                let h = cell.iter().map(|b| b.h).max().unwrap_or(0);
                let text: Vec<&str> = cell.iter().map(|b| b.text.as_str()).collect();
                format!("[x={x} w={} h={h}] {}", right - x, text.join(" "))
            })
            .collect();
        let _ = writeln!(s, "row {} (y={top}): {}", i + 1, cells.join(" | "));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const OCR: &str = "OCR language: en-US\n[10,10 80x20] Logo\n[400,12 60x18] Login\n[480,11 70x18] Sign up\n[10,80 300x30] Welcome back\n[10,130 120x16] Email\n[140,131 40x16] here\n";

    #[test]
    fn parses_ocr_lines_and_skips_headers() {
        let b = parse_ocr(OCR);
        assert_eq!(b.len(), 6);
        assert_eq!(b[1], TextBox { x: 400, y: 12, w: 60, h: 18, text: "Login".into() });
        assert!(parse_ocr("No text found (OCR language: en).").is_empty());
        assert!(parse_ocr("[1,2 3] x\n[a,b cxd] y").is_empty());
    }

    #[test]
    fn groups_rows_and_cells() {
        let r = rows(&parse_ocr(OCR));
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), ["Logo", "Login", "Sign up"]);
        let c = cells(&r[0]);
        assert_eq!(c.len(), 2, "Logo is far from the two nav links");
        assert_eq!(c[1].len(), 2);
        assert_eq!(cells(&r[2]).len(), 1, "Email and here are one cell");
    }

    #[test]
    fn describes_layout_and_colors() {
        let d = describe(OCR, (640, 200), &[("#f8f8f8".into(), 80)]);
        assert!(d.contains("Image size: 640x200 px"));
        assert!(d.contains("Dominant colours: #f8f8f8 80%"));
        assert!(d.contains("row 1 (y=10): [x=10 w=80 h=20] Logo | [x=400 w=150 h=18] Login Sign up"), "{d}");
        assert!(d.contains("row 3 (y=130): [x=10 w=170 h=16] Email here"), "{d}");
        assert!(describe("", (1, 1), &[]).contains("No text was found"));
    }

    #[test]
    fn dominant_colors_of_a_two_tone_png() {
        let mut img = image::RgbImage::from_pixel(40, 10, image::Rgb([255, 255, 255]));
        for x in 0..10 {
            for y in 0..10 {
                img.put_pixel(x, y, image::Rgb([0, 0, 255]));
            }
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let p = dominant_colors(&png, 3).unwrap();
        assert_eq!((p.width, p.height), (40, 10));
        let c = p.colors;
        assert_eq!(c[0].0, "#f8f8f8");
        assert_eq!(c[1].0, "#0808f8");
        assert!(c[0].1 >= 70 && c[1].1 >= 20, "{c:?}");
        assert!(dominant_colors(b"not an image", 3).is_err());
    }
}
