//! Markdown → HTML, PDF and DOCX for proposals, contracts and client reports (§29.12, §29.13).
//!
//! The Markdown subset Kor itself writes: `#`–`###` headings, paragraphs, `-`/`1.` lists, `>` quotes,
//! `|` tables, `---` rules and `**bold**`. No external renderer: the PDF uses the standard
//! Helvetica fonts with a Turkish-capable encoding (WinAnsi plus ğĞışŞİ), the DOCX is plain
//! WordprocessingML in a zip. Both open in any viewer and stay editable as Markdown.

use std::fmt::Write as _;
use std::io::Write as _;

use super::{Result, esc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    H(u8, String),
    P(String),
    Li(Option<u32>, String),
    Quote(String),
    Table(Vec<Vec<String>>),
    Rule,
}

pub fn blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut table: Vec<Vec<String>> = Vec::new();
    let flush = |para: &mut Vec<String>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::P(para.join(" ")));
            para.clear();
        }
    };
    for raw in md.lines() {
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.starts_with('|') {
            flush(&mut para, &mut out);
            let cells: Vec<String> = t.trim_matches('|').split('|').map(|c| c.trim().replace("\\|", "|")).collect();
            if !cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':'))) {
                table.push(cells);
            }
            continue;
        }
        if !table.is_empty() {
            out.push(Block::Table(std::mem::take(&mut table)));
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(h) = t.strip_prefix("### ") {
            flush(&mut para, &mut out);
            out.push(Block::H(3, h.to_owned()));
        } else if let Some(h) = t.strip_prefix("## ") {
            flush(&mut para, &mut out);
            out.push(Block::H(2, h.to_owned()));
        } else if let Some(h) = t.strip_prefix("# ") {
            flush(&mut para, &mut out);
            out.push(Block::H(1, h.to_owned()));
        } else if t == "---" || t == "***" {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
        } else if let Some(x) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush(&mut para, &mut out);
            out.push(Block::Li(None, x.to_owned()));
        } else if let Some((n, x)) = t.split_once(". ").filter(|(n, _)| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit())) {
            flush(&mut para, &mut out);
            out.push(Block::Li(n.parse().ok(), x.to_owned()));
        } else if let Some(q) = t.strip_prefix('>') {
            flush(&mut para, &mut out);
            out.push(Block::Quote(q.trim().to_owned()));
        } else {
            para.push(t.to_owned());
        }
    }
    if !table.is_empty() {
        out.push(Block::Table(table));
    }
    flush(&mut para, &mut out);
    out
}

/// Escape, then `**bold**` and `` `code` ``.
fn inline_html(s: &str) -> String {
    let e = esc(s);
    let mut o = String::new();
    let mut bold = false;
    let mut rest = e.as_str();
    while let Some(i) = rest.find("**") {
        o.push_str(&rest[..i]);
        o.push_str(if bold { "</strong>" } else { "<strong>" });
        bold = !bold;
        rest = &rest[i + 2..];
    }
    o.push_str(rest);
    if bold {
        o.push_str("</strong>");
    }
    let mut code = false;
    o.split('`').enumerate().fold(String::new(), |mut acc, (i, part)| {
        if i > 0 {
            acc.push_str(if code { "</code>" } else { "<code>" });
            code = !code;
        }
        acc.push_str(part);
        acc
    }) + if code { "</code>" } else { "" }
}

/// A standalone, print-friendly HTML page. `accent` is a CSS colour (from `business.toml`).
pub fn html(md: &str, title: &str, accent: &str) -> String {
    let accent = if accent.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'#') && !accent.is_empty() { accent } else { "#c96442" };
    let mut body = String::new();
    let mut list: Option<bool> = None;
    let close = |body: &mut String, list: &mut Option<bool>| {
        if let Some(ordered) = list.take() {
            body.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
        }
    };
    for b in blocks(md) {
        if !matches!(b, Block::Li(..)) {
            close(&mut body, &mut list);
        }
        match b {
            Block::H(n, t) => {
                let _ = writeln!(body, "<h{n}>{}</h{n}>", inline_html(&t));
            }
            Block::P(t) => {
                let _ = writeln!(body, "<p>{}</p>", inline_html(&t));
            }
            Block::Quote(t) => {
                let _ = writeln!(body, "<blockquote>{}</blockquote>", inline_html(&t));
            }
            Block::Rule => body.push_str("<hr>\n"),
            Block::Li(n, t) => {
                let ordered = n.is_some();
                if list != Some(ordered) {
                    close(&mut body, &mut list);
                    body.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
                    list = Some(ordered);
                }
                let _ = writeln!(body, "<li>{}</li>", inline_html(&t));
            }
            Block::Table(rows) => {
                body.push_str("<table>\n");
                for (i, r) in rows.iter().enumerate() {
                    let tag = if i == 0 { "th" } else { "td" };
                    body.push_str("<tr>");
                    for c in r {
                        let _ = write!(body, "<{tag}>{}</{tag}>", inline_html(c));
                    }
                    body.push_str("</tr>\n");
                }
                body.push_str("</table>\n");
            }
        }
    }
    close(&mut body, &mut list);
    format!(
        "<!doctype html>\n<html lang=\"tr\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<title>{t}</title>\n<style>\n:root{{--accent:{accent};--fg:#2b2a27;--muted:#6b675e;--line:#e5e0d6;--bg:#ffffff}}\n\
@media (prefers-color-scheme:dark){{:root{{--fg:#e8e4d8;--muted:#a39d90;--line:#3d3a35;--bg:#1f1e1b}}}}\n\
body{{font:15px/1.6 system-ui,-apple-system,'Segoe UI',sans-serif;color:var(--fg);background:var(--bg);max-width:820px;margin:0 auto;padding:32px 20px}}\n\
h1{{font-size:26px;border-bottom:3px solid var(--accent);padding-bottom:8px}}h2{{font-size:20px;margin-top:28px}}h3{{font-size:16px}}\n\
table{{border-collapse:collapse;width:100%;margin:12px 0;font-size:14px}}th,td{{border:1px solid var(--line);padding:6px 8px;text-align:left;vertical-align:top}}\n\
th{{background:color-mix(in srgb,var(--accent) 12%,transparent)}}blockquote{{border-left:4px solid var(--accent);margin:12px 0;padding:4px 12px;color:var(--muted)}}\n\
code{{font-family:ui-monospace,monospace;font-size:13px}}hr{{border:0;border-top:1px solid var(--line)}}\n\
@media print{{body{{max-width:none;padding:0}}a{{color:inherit}}}}\n</style></head>\n<body>\n<main>\n{body}</main>\n</body></html>\n",
        t = esc(title)
    )
}

/// Character → byte in the PDF fonts' encoding; `None` = not printable there.
fn pdf_byte(c: char) -> Option<u8> {
    Some(match c {
        ' '..='~' => u8::try_from(u32::from(c)).ok()?,
        'Ğ' => 0xD0,
        'İ' => 0xDD,
        'Ş' => 0xDE,
        'ğ' => 0xF0,
        'ı' => 0xFD,
        'ş' => 0xFE,
        '\u{a0}'..='\u{ff}' if !matches!(u32::from(c), 0xD0 | 0xDD | 0xDE | 0xF0 | 0xFD | 0xFE) => u8::try_from(u32::from(c)).ok()?,
        '€' => 0x80,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        _ => return None,
    })
}

/// Text for the PDF fonts: Turkish lira sign as `TL`, arrows and `≈` spelled out.
fn pdf_text(s: &str) -> Vec<u8> {
    let s = s.replace("**", "").replace('`', "").replace('₺', "TL").replace('→', "->").replace('≈', "~").replace('✓', "+");
    s.chars().map(|c| pdf_byte(c).unwrap_or(b'?')).collect()
}

/// Helvetica advance widths (1/1000 em) for ASCII 32–126.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556,
    556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667,
    778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222,
    222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

fn width(bytes: &[u8], size: f32, bold: bool) -> f32 {
    let em: u32 = bytes
        .iter()
        .map(|b| match b {
            32..=126 => u32::from(HELV[usize::from(b - 32)]),
            _ => 600,
        })
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let w = em as f32 * size / 1000.0;
    if bold { w * 1.06 } else { w }
}

/// Greedy word wrap to `max` points.
fn wrap(text: &[u8], size: f32, bold: bool, max: f32) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    for word in text.split(|b| *b == b' ').filter(|w| !w.is_empty()) {
        let mut cand = cur.clone();
        if !cand.is_empty() {
            cand.push(b' ');
        }
        cand.extend_from_slice(word);
        if width(&cand, size, bold) <= max || cur.is_empty() {
            cur = cand;
            // A single word wider than the line is cut.
            while width(&cur, size, bold) > max && cur.len() > 1 {
                let mut cut = cur.len() - 1;
                while cut > 1 && width(&cur[..cut], size, bold) > max {
                    cut -= 1;
                }
                lines.push(cur[..cut].to_vec());
                cur = cur[cut..].to_vec();
            }
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_vec();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn pdf_str(b: &[u8]) -> String {
    let mut o = String::from("(");
    for &c in b {
        match c {
            b'(' | b')' | b'\\' => {
                o.push('\\');
                o.push(c as char);
            }
            32..=126 => o.push(c as char),
            _ => {
                let _ = write!(o, "\\{c:03o}");
            }
        }
    }
    o.push(')');
    o
}

struct PdfPages {
    pages: Vec<String>,
    cur: String,
    y: f32,
}

const PW: f32 = 595.0;
const PH: f32 = 842.0;
const M: f32 = 50.0;

impl PdfPages {
    fn need(&mut self, h: f32) {
        if self.y - h < M + 20.0 {
            self.pages.push(std::mem::take(&mut self.cur));
            self.y = PH - M;
        }
    }

    fn text(&mut self, x: f32, b: &[u8], size: f32, bold: bool) {
        let _ = writeln!(self.cur, "BT /{} {size} Tf {x:.1} {:.1} Td {} Tj ET", if bold { "F2" } else { "F1" }, self.y, pdf_str(b));
    }

    fn para(&mut self, x: f32, text: &[u8], size: f32, bold: bool, after: f32) {
        let lead = size * 1.4;
        for line in wrap(text, size, bold, PW - M - x) {
            self.need(lead);
            self.y -= lead;
            self.text(x, &line, size, bold);
        }
        self.y -= after;
    }
}

/// A4 PDF of the Markdown. `title` goes into the document info.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
pub fn pdf(md: &str, title: &str) -> Vec<u8> {
    let mut p = PdfPages { pages: Vec::new(), cur: String::new(), y: PH - M };
    for b in blocks(md) {
        match b {
            Block::H(n, t) => {
                let size = match n {
                    1 => 18.0,
                    2 => 14.0,
                    _ => 12.0,
                };
                p.need(size * 3.0);
                p.y -= size * 0.6;
                p.para(M, &pdf_text(&t), size, true, 4.0);
            }
            Block::P(t) => p.para(M, &pdf_text(&t), 10.5, false, 6.0),
            Block::Quote(t) => p.para(M + 14.0, &pdf_text(&t), 10.0, false, 6.0),
            Block::Li(n, t) => {
                let mark = n.map_or_else(|| vec![0x95], |n| format!("{n}.").into_bytes());
                p.need(15.0);
                let y0 = p.y;
                p.para(M + 16.0, &pdf_text(&t), 10.5, false, 2.0);
                let _ = writeln!(p.cur, "BT /F1 10.5 Tf {:.1} {:.1} Td {} Tj ET", M + 4.0, y0 - 14.7, pdf_str(&mark));
            }
            Block::Rule => {
                p.need(12.0);
                p.y -= 6.0;
                let _ = writeln!(p.cur, "0.8 G {M} {:.1} m {:.1} {:.1} l S 0 G", p.y, PW - M, p.y);
                p.y -= 6.0;
            }
            Block::Table(rows) => {
                let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let col_w = (PW - 2.0 * M) / cols as f32;
                let size = 9.0;
                let lead = size * 1.35;
                for (ri, r) in rows.iter().enumerate() {
                    let cells: Vec<Vec<Vec<u8>>> = (0..cols)
                        .map(|c| wrap(&pdf_text(r.get(c).map_or("", String::as_str)), size, ri == 0, col_w - 8.0))
                        .collect();
                    let h = cells.iter().map(Vec::len).max().unwrap_or(1) as f32 * lead + 6.0;
                    p.need(h);
                    let top = p.y;
                    if ri == 0 {
                        let _ = writeln!(p.cur, "0.93 g {M} {:.1} {:.1} {h:.1} re f 0 g", top - h, PW - 2.0 * M);
                    }
                    for (c, lines) in cells.iter().enumerate() {
                        let x = M + col_w * c as f32 + 4.0;
                        let mut y = top - 3.0;
                        for l in lines {
                            y -= lead;
                            let _ = writeln!(p.cur, "BT /{} {size} Tf {x:.1} {:.1} Td {} Tj ET", if ri == 0 { "F2" } else { "F1" }, y + 3.0, pdf_str(l));
                        }
                    }
                    let _ = writeln!(p.cur, "0.75 G 0.5 w {M} {:.1} {:.1} {h:.1} re S 0 G", top - h, PW - 2.0 * M);
                    for c in 1..cols {
                        let x = M + col_w * c as f32;
                        let _ = writeln!(p.cur, "0.75 G {x:.1} {:.1} m {x:.1} {top:.1} l S 0 G", top - h);
                    }
                    p.y -= h;
                }
                p.y -= 8.0;
            }
        }
    }
    p.pages.push(p.cur);
    let n = p.pages.len();
    // Objects: 1 catalog, 2 pages, 3 F1, 4 F2, 5 info, then (page, content) pairs.
    let enc = "/Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [208 /Gbreve 221 /Idotaccent 222 /Scedilla 240 /gbreve 253 /dotlessi 254 /scedilla] >>";
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 6 + i * 2)).collect();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).into_bytes(),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica {enc} >>").into_bytes(),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold {enc} >>").into_bytes(),
        format!("<< /Title {} /Producer (Kor) >>", pdf_str(&pdf_text(title))).into_bytes(),
    ];
    for (i, content) in p.pages.iter().enumerate() {
        let footer = format!("BT /F1 8 Tf {:.1} 30 Td {} Tj ET\n", PW - M - 30.0, pdf_str(format!("{}/{n}", i + 1).as_bytes()));
        let stream = format!("{content}{footer}");
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PW} {PH}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                7 + i * 2
            )
            .into_bytes(),
        );
        let mut s = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        s.extend_from_slice(stream.as_bytes());
        s.extend_from_slice(b"\nendstream");
        objs.push(s);
    }
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        let _ = writeln!(out, "{} 0 obj", i + 1);
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for off in offsets {
        let _ = writeln!(out, "{off:010} 00000 n ");
    }
    let _ = write!(out, "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1);
    out
}

fn xml(s: &str) -> String {
    esc(s)
}

/// `**bold**` → runs.
fn runs(s: &str, bold_all: bool, size: Option<u32>) -> String {
    let mut o = String::new();
    for (i, part) in s.split("**").enumerate() {
        if part.is_empty() {
            continue;
        }
        let b = bold_all || i % 2 == 1;
        let mut rpr = String::new();
        if b {
            rpr.push_str("<w:b/>");
        }
        if let Some(sz) = size {
            let _ = write!(rpr, "<w:sz w:val=\"{sz}\"/>");
        }
        let _ = write!(o, "<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>", if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") }, xml(&part.replace('`', "")));
    }
    o
}

/// DOCX (WordprocessingML) of the Markdown.
pub fn docx(md: &str) -> Result<Vec<u8>> {
    let mut body = String::new();
    for b in blocks(md) {
        match b {
            Block::H(n, t) => {
                let sz = match n {
                    1 => 36,
                    2 => 28,
                    _ => 24,
                };
                let _ = write!(body, "<w:p><w:pPr><w:spacing w:before=\"240\" w:after=\"120\"/></w:pPr>{}</w:p>", runs(&t, true, Some(sz)));
            }
            Block::P(t) => {
                let _ = write!(body, "<w:p>{}</w:p>", runs(&t, false, None));
            }
            Block::Quote(t) => {
                let _ = write!(body, "<w:p><w:pPr><w:ind w:left=\"360\"/></w:pPr>{}</w:p>", runs(&t, false, None));
            }
            Block::Li(n, t) => {
                let mark = n.map_or_else(|| "• ".to_owned(), |n| format!("{n}. "));
                let _ = write!(body, "<w:p><w:pPr><w:ind w:left=\"360\" w:hanging=\"240\"/></w:pPr>{}</w:p>", runs(&format!("{mark}{t}"), false, None));
            }
            Block::Rule => body.push_str("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"BBBBBB\"/></w:pBdr></w:pPr></w:p>"),
            Block::Table(rows) => {
                body.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblBorders>");
                for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
                    let _ = write!(body, "<w:{side} w:val=\"single\" w:sz=\"4\" w:color=\"BBBBBB\"/>");
                }
                body.push_str("</w:tblBorders></w:tblPr>");
                for (i, r) in rows.iter().enumerate() {
                    body.push_str("<w:tr>");
                    for c in r {
                        let _ = write!(body, "<w:tc><w:p>{}</w:p></w:tc>", runs(c, i == 0, None));
                    }
                    body.push_str("</w:tr>");
                }
                body.push_str("</w:tbl><w:p/>");
            }
        }
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}\
<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    );
    let files: [(&str, String); 3] = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>".into(),
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>".into(),
        ),
        ("word/document.xml", document),
    ];
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, text) in files {
            z.start_file(name, opt).map_err(|e| e.to_string())?;
            z.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

/// Write `md` as `<stem>.md` plus the requested formats (`html`, `pdf`, `docx`) in `dir`.
pub fn write_all(dir: &std::path::Path, stem: &str, md: &str, title: &str, accent: &str, formats: &[String]) -> Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    let md_path = dir.join(format!("{stem}.md"));
    std::fs::write(&md_path, md).map_err(|e| e.to_string())?;
    out.push(md_path);
    for f in formats {
        let (ext, bytes) = match f.as_str() {
            "md" => continue,
            "html" => ("html", html(md, title, accent).into_bytes()),
            "pdf" => ("pdf", pdf(md, title)),
            "docx" => ("docx", docx(md)?),
            other => return Err(format!("unknown format `{other}` (md, html, pdf, docx)")),
        };
        let p = dir.join(format!("{stem}.{ext}"));
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# Teklif: Şık Çanta Mağazası\n\nMerhaba **Ayşe Hanım**, aşağıda kapsam var.\n\n## Kapsam\n\n- Tasarım\n- Kurulum\n\n1. Ödeme\n\n| İş | Saat | Tutar |\n|---|---|---|\n| Tasarım | 12 | 12.000,00 ₺ |\n\n> Hukuki metin değildir.\n\n---\n";

    #[test]
    fn markdown_blocks() {
        let b = blocks(MD);
        assert_eq!(b[0], Block::H(1, "Teklif: Şık Çanta Mağazası".into()));
        assert!(matches!(&b[4], Block::Li(None, t) if t == "Kurulum"));
        assert!(matches!(&b[5], Block::Li(Some(1), _)));
        assert!(matches!(&b[6], Block::Table(r) if r.len() == 2 && r[1][2] == "12.000,00 ₺"));
        assert_eq!(*b.last().unwrap(), Block::Rule);
    }

    #[test]
    fn html_escapes_and_styles() {
        let h = html("# A <b>\n\nx **y** `z`", "T<", "#123456");
        assert!(h.contains("<h1>A &lt;b&gt;</h1>") && h.contains("<strong>y</strong>") && h.contains("<code>z</code>"));
        assert!(h.contains("--accent:#123456") && h.contains("<title>T&lt;</title>"));
        assert!(html("x", "t", "red;}body{").contains("--accent:#c96442"));
    }

    #[test]
    fn pdf_is_well_formed_with_turkish_letters() {
        let long = "satır ".repeat(3000);
        let bytes = pdf(&format!("{MD}\n\n{long}\n"), "Teklif");
        assert!(bytes.starts_with(b"%PDF-1.4") && bytes.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Differences [208 /Gbreve"));
        // ş → 0xFE (octal 376), ı → 0xFD (375), ₺ → TL
        assert!(text.contains("\\376"));
        assert!(text.contains("TL"));
        // xref offsets point at the objects.
        let xref_at: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(bytes[xref_at..].starts_with(b"xref"));
        let pages = text.matches("/Type /Page ").count();
        assert!(pages >= 2, "{pages}");
    }

    #[test]
    fn docx_is_a_zip_with_the_document() {
        let bytes = docx(MD).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut doc = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut doc).unwrap();
        assert!(doc.contains("Şık Çanta") && doc.contains("<w:tbl>") && doc.contains("<w:b/>"));
        assert!(z.by_name("[Content_Types].xml").is_ok());
    }
}
