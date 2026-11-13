//! Bulk titles, descriptions and alt text (spec §29.8).
//!
//! Flow: collect pages and images from a crawl → generate a sample (default 20) → the user
//! edits the CSV review table → generate the rest with the edited rows as style examples →
//! deterministic checks (length, approximate pixel width, duplicates, keyword) → CSV export,
//! or apply to WordPress after approval with an undo file written before every change, or
//! OpenCart write-back as a SQL script that saves the old values in a table before updating
//! and a matching undo script ([`opencart_sql`]).
//! Images: a vision model sees the image; otherwise OCR text, the file name and the page
//! context stand in (spec §2 rule 4).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::audit::{Crawl, DESC_MAX, DESC_MIN, TITLE_MAX};
use super::model::{Ask, unfence};
use super::wp::{MetaKeys, Wp};

/// Google cuts titles near 580 px and descriptions near 920 px (desktop).
pub const TITLE_PX: u32 = 580;
pub const DESC_PX: u32 = 920;
/// Largest image sent to a vision model.
const MAX_IMAGE: usize = 5 * 1024 * 1024;
const PAGES_PER_CALL: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// `page` or `image`.
    pub kind: String,
    /// Page URL, or the image URL.
    pub url: String,
    /// For images: the page it was found on.
    pub page: String,
    pub current_title: String,
    pub current_description: String,
    pub current_alt: String,
    pub title: String,
    pub description: String,
    pub alt: String,
    /// Main heading / context used for generation.
    pub context: String,
    pub issues: String,
}

impl Row {
    fn done(&self) -> bool {
        if self.kind == "image" {
            !self.alt.trim().is_empty()
        } else {
            !self.title.trim().is_empty()
        }
    }
}

/// Approximate rendered width (Arial-like proportions at Google's title size, 20 px).
pub fn pixel_width(s: &str) -> u32 {
    s.chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | 'ı' | '\'' | '.' | ',' | ':' | ';' | '|' | '!' | 'I' => 5,
            'f' | 't' | 'r' | ' ' | '-' | '(' | ')' | '/' => 7,
            'm' | 'w' => 17,
            'M' | 'W' => 19,
            c if c.is_uppercase() => 13,
            _ => 11,
        })
        .sum()
}

/// Pages and images of a crawl (images without alt, or all with `all_images`).
pub fn collect(c: &Crawl, all_images: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut seen_img = std::collections::HashSet::new();
    for p in c.pages.iter().filter(|p| p.status == 200) {
        let Some(pg) = &p.page else { continue };
        let h1 = pg
            .headings
            .iter()
            .find(|h| h.value.0 == 1)
            .map(|h| h.value.1.clone())
            .unwrap_or_default();
        let excerpt: String = pg.text.chars().take(600).collect();
        rows.push(Row {
            kind: "page".into(),
            url: p.final_url.clone(),
            current_title: pg
                .titles
                .first()
                .map(|t| t.value.clone())
                .unwrap_or_default(),
            current_description: pg
                .descriptions
                .first()
                .map(|d| d.value.clone())
                .unwrap_or_default(),
            context: format!("{h1} — {excerpt}"),
            ..Row::default()
        });
        let base = url::Url::parse(&p.final_url).ok();
        for img in &pg.images {
            if !all_images && img.alt.is_some() {
                continue;
            }
            let Some(src) = base
                .as_ref()
                .and_then(|b| b.join(&img.src).ok())
                .map(String::from)
            else {
                continue;
            };
            if seen_img.insert(src.clone()) {
                rows.push(Row {
                    kind: "image".into(),
                    url: src,
                    page: p.final_url.clone(),
                    current_alt: img.alt.clone().unwrap_or_default(),
                    context: h1.clone(),
                    ..Row::default()
                });
            }
        }
    }
    rows
}

// ---------------------------------------------------------------------------------------------
// CSV (RFC 4180)
// ---------------------------------------------------------------------------------------------

pub const COLUMNS: [&str; 11] = [
    "kind",
    "url",
    "page",
    "current_title",
    "current_description",
    "current_alt",
    "title",
    "description",
    "alt",
    "context",
    "issues",
];

fn cell(s: &str) -> String {
    // Leading = + - @ would run as formulas in spreadsheets (CWE-1236): prefix a quote.
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn to_csv(rows: &[Row]) -> String {
    let mut out = COLUMNS.join(",");
    out.push('\n');
    for r in rows {
        let f = [
            &r.kind,
            &r.url,
            &r.page,
            &r.current_title,
            &r.current_description,
            &r.current_alt,
            &r.title,
            &r.description,
            &r.alt,
            &r.context,
            &r.issues,
        ];
        out.push_str(&f.iter().map(|s| cell(s)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

fn parse_records(text: &str) -> Vec<Vec<String>> {
    let mut recs = Vec::new();
    let mut rec = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => rec.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                rec.push(std::mem::take(&mut field));
                recs.push(std::mem::take(&mut rec));
            }
            (_, c) => field.push(c),
        }
    }
    if !field.is_empty() || !rec.is_empty() {
        rec.push(field);
        recs.push(rec);
    }
    recs
}

fn unformula(s: &str) -> String {
    match s.strip_prefix('\'') {
        Some(r) if r.starts_with(['=', '+', '-', '@']) => r.to_owned(),
        _ => s.to_owned(),
    }
}

pub fn from_csv(text: &str) -> Result<Vec<Row>, String> {
    let mut recs = parse_records(text.trim_start_matches('\u{feff}')).into_iter();
    let head = recs.next().ok_or("empty CSV")?;
    let idx: HashMap<&str, usize> = head
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim(), i))
        .collect();
    for c in ["kind", "url"] {
        if !idx.contains_key(c) {
            return Err(format!("CSV has no `{c}` column"));
        }
    }
    let rows = recs
        .filter(|r| r.iter().any(|f| !f.is_empty()))
        .map(|r| {
            let g = |k: &str| {
                idx.get(k)
                    .and_then(|i| r.get(*i))
                    .map(|s| unformula(s))
                    .unwrap_or_default()
            };
            Row {
                kind: g("kind"),
                url: g("url"),
                page: g("page"),
                current_title: g("current_title"),
                current_description: g("current_description"),
                current_alt: g("current_alt"),
                title: g("title"),
                description: g("description"),
                alt: g("alt"),
                context: g("context"),
                issues: g("issues"),
            }
        })
        .collect::<Vec<_>>();
    if rows
        .iter()
        .any(|r| !matches!(r.kind.as_str(), "page" | "image"))
    {
        return Err("kind must be `page` or `image`".into());
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------------------------
// Checks, estimate, generation
// ---------------------------------------------------------------------------------------------

/// Fill `issues` for every row: lengths, pixel width, duplicates, keyword (from `keywords`, by URL).
#[allow(clippy::implicit_hasher)]
pub fn validate(rows: &mut [Row], keywords: &HashMap<String, String>) {
    let mut titles: HashMap<String, usize> = HashMap::new();
    let mut descs: HashMap<String, usize> = HashMap::new();
    for r in rows.iter().filter(|r| r.kind == "page") {
        *titles.entry(r.title.trim().to_lowercase()).or_default() += 1;
        *descs
            .entry(r.description.trim().to_lowercase())
            .or_default() += 1;
    }
    for r in rows.iter_mut() {
        let mut v = Vec::new();
        if r.kind == "page" {
            let (tn, tp) = (r.title.chars().count(), pixel_width(&r.title));
            if r.title.trim().is_empty() {
                v.push("title-empty".to_owned());
            } else {
                if tn > TITLE_MAX || tp > TITLE_PX {
                    v.push(format!("title-long({tn}ch,{tp}px)"));
                }
                if titles
                    .get(&r.title.trim().to_lowercase())
                    .copied()
                    .unwrap_or(0)
                    > 1
                {
                    v.push("title-duplicate".into());
                }
            }
            let (dn, dp) = (
                r.description.chars().count(),
                pixel_width(&r.description) * 7 / 10,
            );
            if r.description.trim().is_empty() {
                v.push("description-empty".into());
            } else {
                if dn > DESC_MAX || dp > DESC_PX {
                    v.push(format!("description-long({dn}ch,{dp}px)"));
                } else if dn < DESC_MIN {
                    v.push(format!("description-short({dn}ch)"));
                }
                if descs
                    .get(&r.description.trim().to_lowercase())
                    .copied()
                    .unwrap_or(0)
                    > 1
                {
                    v.push("description-duplicate".into());
                }
            }
            if let Some(k) = keywords.get(&r.url)
                && !r.title.to_lowercase().contains(&k.to_lowercase())
            {
                v.push("title-no-keyword".into());
            }
        } else {
            let n = r.alt.chars().count();
            if r.alt.trim().is_empty() {
                v.push("alt-empty".into());
            } else if n > 125 {
                v.push(format!("alt-long({n}ch)"));
            }
            if r.alt.to_lowercase().starts_with("image of")
                || r.alt.to_lowercase().starts_with("resim")
            {
                v.push("alt-redundant-prefix".into());
            }
        }
        r.issues = v.join(" ");
    }
}

/// Token estimate for generating the open rows: (input, output). Pages are batched.
pub fn estimate_tokens(rows: &[Row], vision: bool) -> (u64, u64) {
    let pages = rows
        .iter()
        .filter(|r| r.kind == "page" && !r.done())
        .count() as u64;
    let images = rows
        .iter()
        .filter(|r| r.kind == "image" && !r.done())
        .count() as u64;
    let calls = pages.div_ceil(PAGES_PER_CALL as u64);
    let input = calls * 600 + pages * 250 + images * if vision { 1_200 } else { 350 };
    (input, pages * 70 + images * 30)
}

fn examples(rows: &[Row], kind: &str) -> String {
    let mut s = String::new();
    for r in rows.iter().filter(|r| r.kind == kind && r.done()).take(10) {
        if kind == "page" {
            let _ = writeln!(
                s,
                "- {} → title: \"{}\" | description: \"{}\"",
                r.url, r.title, r.description
            );
        } else {
            let _ = writeln!(s, "- {} → alt: \"{}\"", r.url, r.alt);
        }
    }
    s
}

fn page_prompt(batch: &[&Row], ex: &str, lang: &str) -> String {
    let mut s = format!(
        "Write an SEO title (at most {TITLE_MAX} characters) and meta description ({DESC_MIN}-{DESC_MAX} characters) for each page, \
         in language `{lang}`, in the site's tone. Every title and description must be unique; no clickbait, no invented facts.\n"
    );
    if !ex.is_empty() {
        let _ = write!(s, "Match the style of these reviewed examples:\n{ex}\n");
    }
    s.push_str("Pages (url, current title, context):\n");
    for r in batch {
        let _ = writeln!(
            s,
            "- {} | {} | {}",
            r.url,
            r.current_title,
            r.context.chars().take(500).collect::<String>()
        );
    }
    s.push_str(
        "\nAnswer with only a JSON array: [{\"url\":\"…\",\"title\":\"…\",\"description\":\"…\"}]",
    );
    s
}

fn alt_prompt(r: &Row, ex: &str, lang: &str, ocr: Option<&str>) -> String {
    let file = r.url.rsplit('/').next().unwrap_or(&r.url);
    let mut s = format!(
        "Write alt text (at most 125 characters, language `{lang}`) for an image on a page about \"{}\". File name: {file}. ",
        r.context
    );
    if let Some(t) = ocr {
        let _ = write!(
            s,
            "You cannot see the image; text read from it by OCR: \"{}\". ",
            t.chars().take(400).collect::<String>()
        );
    }
    if !ex.is_empty() {
        let _ = write!(s, "Match these reviewed examples:\n{ex}");
    }
    s.push_str("Describe what it shows and why it matters on the page; do not start with \"image of\". Answer with the alt text only.");
    s
}

async fn image_bytes(f: &super::fetch::Fetcher, src: &str) -> Result<(String, String), String> {
    let u = url::Url::parse(src).map_err(|e| e.to_string())?;
    let r = f.get(&u).await?;
    let mime = r
        .header("content-type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    if r.status != 200 || !mime.starts_with("image/") || r.truncated || r.bytes.len() > MAX_IMAGE {
        return Err(format!("image not usable (HTTP {}, {mime})", r.status));
    }
    Ok((
        mime,
        base64::engine::general_purpose::STANDARD.encode(&r.bytes),
    ))
}

/// Generate the open rows (at most `limit`), using done rows as style examples.
pub async fn generate(
    rows: &mut [Row],
    ask: &dyn Ask,
    lang: &str,
    limit: usize,
    user_agent: &str,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<usize, String> {
    let open: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.done())
        .map(|(i, _)| i)
        .take(limit)
        .collect();
    let total = open.len();
    let page_ex = examples(rows, "page");
    let img_ex = examples(rows, "image");
    let mut done = 0;
    let pages: Vec<usize> = open
        .iter()
        .copied()
        .filter(|i| rows[*i].kind == "page")
        .collect();
    for chunk in pages.chunks(PAGES_PER_CALL) {
        let batch: Vec<&Row> = chunk.iter().map(|i| &rows[*i]).collect();
        let reply = ask.ask(page_prompt(&batch, &page_ex, lang), 3_000).await?;
        let v: Value = serde_json::from_str(unfence(&reply))
            .map_err(|e| format!("model did not return JSON: {e}"))?;
        for item in v.as_array().into_iter().flatten() {
            let url = item["url"].as_str().unwrap_or("");
            if let Some(i) = chunk.iter().find(|i| rows[**i].url == url) {
                rows[*i].title = item["title"].as_str().unwrap_or("").trim().to_owned();
                rows[*i].description = item["description"].as_str().unwrap_or("").trim().to_owned();
                done += 1;
            }
        }
        progress(done, total);
    }
    let fetcher =
        super::fetch::Fetcher::new(user_agent, std::time::Duration::from_millis(300), true)?;
    let images: Vec<usize> = open
        .into_iter()
        .filter(|i| rows[*i].kind == "image")
        .collect();
    for i in images {
        let r = rows[i].clone();
        let img = image_bytes(&fetcher, &r.url).await;
        let text = match (&img, ask.vision()) {
            (Ok((mime, data)), true) => {
                ask.ask_image(alt_prompt(&r, &img_ex, lang, None), mime, data, 200)
                    .await?
            }
            (Ok((mime, data)), false) => {
                let src = kor_core::ImageSource::Base64 {
                    media_type: mime.clone(),
                    data: data.clone(),
                };
                let ocr = kor_tools::image_text(&src, &kor_providers::CancellationToken::new())
                    .await
                    .unwrap_or_default();
                ask.ask(alt_prompt(&r, &img_ex, lang, Some(&ocr)), 200)
                    .await?
            }
            (Err(_), _) => {
                ask.ask(alt_prompt(&r, &img_ex, lang, Some("")), 200)
                    .await?
            }
        };
        rows[i].alt = unfence(&text).trim().trim_matches('"').to_owned();
        done += 1;
        progress(done, total);
    }
    Ok(done)
}

// ---------------------------------------------------------------------------------------------
// Apply to WordPress (undoable)
// ---------------------------------------------------------------------------------------------

/// One write: what was there before, so `undo` can put it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: String,
    pub id: u64,
    pub url: String,
    pub before: Value,
    pub after: Value,
}

/// Result of an apply run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub changed: Vec<String>,
    /// URL → reason (not found in WordPress, plugin meta not exposed, error…).
    pub skipped: Vec<(String, String)>,
}

fn append_undo(path: &Path, c: &Change) -> Result<(), String> {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(
        f,
        "{}",
        serde_json::to_string(c).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())
}

/// Write the reviewed rows to WordPress. Each previous value goes to `undo` (JSON lines) before
/// its write; every write is read back.
pub async fn apply(
    rows: &[Row],
    wp: &Wp,
    keys: MetaKeys,
    undo: &Path,
    progress: &(dyn Fn(&str) + Sync),
) -> Applied {
    let mut out = Applied::default();
    for r in rows.iter().filter(|r| r.done()) {
        progress(&r.url);
        let res: Result<(), String> = async {
            if r.kind == "image" {
                let item = wp
                    .find_media(&r.url)
                    .await?
                    .ok_or("not in the media library")?;
                let after = serde_json::json!({ "alt_text": r.alt });
                append_undo(
                    undo,
                    &Change {
                        kind: "media".into(),
                        id: item.id,
                        url: r.url.clone(),
                        before: serde_json::json!({ "alt_text": item.object["alt_text"] }),
                        after: after.clone(),
                    },
                )?;
                let resp = wp.update("media", item.id, &after).await?;
                if resp["alt_text"] != r.alt {
                    return Err("alt text did not stick".into());
                }
            } else {
                let item = wp
                    .find_by_url(&r.url)
                    .await?
                    .ok_or("no post or page with this URL")?;
                let (t, d) = keys.current(&item.object);
                let after = keys.fields(&r.title, &r.description);
                append_undo(
                    undo,
                    &Change {
                        kind: item.kind.clone(),
                        id: item.id,
                        url: r.url.clone(),
                        before: keys.fields(&t, &d),
                        after: after.clone(),
                    },
                )?;
                let resp = wp.update(&item.kind, item.id, &after).await?;
                if !keys.applied(&resp, &r.title, &r.description) {
                    return Err(
                        "SEO plugin meta is not exposed to the REST API (see ADR 0055)".into(),
                    );
                }
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => out.changed.push(r.url.clone()),
            Err(e) => out.skipped.push((r.url.clone(), e)),
        }
    }
    out
}

/// OpenCart tables whose `meta_title` / `meta_description` a page row may update, with their id
/// column and the SEO URL key that points at them.
const OC_TABLES: [(&str, &str); 3] = [
    ("product", "product_id"),
    ("category", "category_id"),
    ("information", "information_id"),
];

/// SQL apply and undo scripts for OpenCart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpencartSql {
    pub apply: String,
    pub undo: String,
    pub rows: usize,
    /// URL → reason it is not in the script.
    pub skipped: Vec<(String, String)>,
}

fn sql_str(s: &str) -> String {
    format!(
        "'{}'",
        s.replace('\\', "\\\\")
            .replace('\'', "''")
            .replace('\0', "")
    )
}

/// The page a URL names: `(Some(table, id), None)` for `index.php?route=…&product_id=42`, else
/// `(None, Some(keyword))` from the last path segment (an SEO URL keyword).
#[allow(clippy::type_complexity)]
fn oc_target(u: &str) -> Option<(Option<(&'static str, u64)>, Option<String>)> {
    let url = url::Url::parse(u).ok()?;
    for (k, v) in url.query_pairs() {
        for (table, col) in OC_TABLES {
            if k == col
                && let Ok(id) = v.parse()
            {
                return Some((Some((table, id)), None));
            }
        }
        if k == "path"
            && let Some(id) = v.rsplit('_').next().and_then(|x| x.parse().ok())
        {
            return Some((Some(("category", id)), None));
        }
    }
    let seg = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let seg = percent_encoding::percent_decode_str(seg)
        .decode_utf8()
        .ok()?
        .trim_end_matches(".html")
        .to_owned();
    (!seg.is_empty() && seg != "index.php").then_some((None, Some(seg)))
}

/// Write-back of reviewed page rows to an OpenCart database (`version` 3 or 4, table `prefix`
/// such as `oc_`, one `language_id`). The apply script first copies the current values into
/// `<prefix>kor_meta_undo` under `run`; the undo script restores them. Images are skipped:
/// OpenCart stores no per-image alt text.
///
/// # Errors
/// An invalid prefix or version.
#[allow(clippy::too_many_lines)]
pub fn opencart_sql(
    rows: &[Row],
    prefix: &str,
    language_id: u32,
    version: u8,
    run: &str,
) -> Result<OpencartSql, String> {
    if prefix.len() > 20
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(format!("invalid table prefix `{prefix}`"));
    }
    if version != 3 && version != 4 {
        return Err("OpenCart version must be 3 or 4".into());
    }
    let p = prefix;
    let run_q = sql_str(run);
    let mut out = OpencartSql::default();
    let _ = write!(
        out.apply,
        "-- Kor meta write-back ({run}), OpenCart {version}, language_id {language_id}.\n\
         -- Run on a backup first. Undo: the matching undo script.\nSTART TRANSACTION;\n\
         CREATE TABLE IF NOT EXISTS `{p}kor_meta_undo` (run VARCHAR(40) NOT NULL, tbl VARCHAR(20) NOT NULL, \
         id INT NOT NULL, language_id INT NOT NULL, meta_title VARCHAR(255) NOT NULL, meta_description TEXT NOT NULL);\n"
    );
    let _ = writeln!(
        out.undo,
        "-- Undo of Kor meta write-back ({run}).\nSTART TRANSACTION;"
    );
    for (table, col) in OC_TABLES {
        let _ = writeln!(
            out.undo,
            "UPDATE `{p}{table}_description` d JOIN `{p}kor_meta_undo` u ON u.run = {run_q} AND u.tbl = '{table}' \
             AND u.id = d.{col} AND u.language_id = d.language_id SET d.meta_title = u.meta_title, d.meta_description = u.meta_description;"
        );
    }
    let _ = writeln!(
        out.undo,
        "DELETE FROM `{p}kor_meta_undo` WHERE run = {run_q};\nCOMMIT;"
    );
    for r in rows.iter().filter(|r| r.done()) {
        if r.kind == "image" {
            out.skipped
                .push((r.url.clone(), "OpenCart has no per-image alt text".into()));
            continue;
        }
        let Some((by_id, by_keyword)) = oc_target(&r.url) else {
            out.skipped.push((
                r.url.clone(),
                "no product, category or page in the URL".into(),
            ));
            continue;
        };
        let set = format!(
            "d.meta_title = {}, d.meta_description = {}",
            sql_str(&r.title),
            sql_str(&r.description)
        );
        let _ = writeln!(out.apply, "-- {}", r.url.replace('\n', " "));
        let targets: Vec<(&str, &str, String)> = match (by_id, &by_keyword) {
            (Some((table, id)), _) => {
                let col = OC_TABLES
                    .iter()
                    .find(|(t, _)| *t == table)
                    .map_or("product_id", |(_, c)| *c);
                vec![(table, col, format!("d.{col} = {id}"))]
            }
            (None, Some(kw)) => OC_TABLES
                .iter()
                .map(|(table, col)| {
                    (
                        *table,
                        *col,
                        format!(
                            "d.{col} IN (SELECT {sel} FROM `{p}seo_url` s0 WHERE s0.keyword = {kw} AND s0.language_id = {language_id}{cond})",
                            sel = if version == 3 {
                                format!("CAST(SUBSTRING(s0.query, {}) AS UNSIGNED)", col.len() + 2)
                            } else {
                                "CAST(s0.`value` AS UNSIGNED)".to_owned()
                            },
                            kw = sql_str(kw),
                            cond = if version == 3 {
                                format!(" AND s0.query LIKE '{col}=%'")
                            } else {
                                format!(" AND s0.`key` = '{col}'")
                            },
                        ),
                    )
                })
                .collect(),
            (None, None) => Vec::new(),
        };
        for (table, col, cond) in targets {
            let _ = writeln!(
                out.apply,
                "INSERT INTO `{p}kor_meta_undo` SELECT {run_q}, '{table}', d.{col}, d.language_id, d.meta_title, d.meta_description \
                 FROM `{p}{table}_description` d WHERE d.language_id = {language_id} AND {cond};\n\
                 UPDATE `{p}{table}_description` d SET {set} WHERE d.language_id = {language_id} AND {cond};"
            );
        }
        out.rows += 1;
    }
    out.apply.push_str("COMMIT;\n");
    Ok(out)
}

/// Restore every `before` from an undo file (last change first).
pub async fn undo(path: &Path, wp: &Wp) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let changes: Vec<Change> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut n = 0;
    for c in changes.iter().rev() {
        wp.update(&c.kind, c.id, &c.before).await?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::super::model::testing::Scripted;
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn opencart_scripts() {
        let row = |url: &str, title: &str| Row {
            kind: "page".into(),
            url: url.into(),
            title: title.into(),
            description: "Açıklama".into(),
            ..Row::default()
        };
        let rows = [
            row(
                "https://shop.example/index.php?route=product/product&product_id=42",
                "Ürün",
            ),
            row(
                "https://shop.example/kis-botu",
                "Kış botu'); DROP TABLE x; --",
            ),
            row(
                "https://shop.example/index.php?route=product/category&path=20_27",
                "Botlar",
            ),
            Row {
                kind: "image".into(),
                url: "https://shop.example/a.jpg".into(),
                alt: "x".into(),
                ..Row::default()
            },
        ];
        let s = opencart_sql(&rows, "oc_", 1, 3, "20261113").unwrap();
        assert_eq!((s.rows, s.skipped.len()), (3, 1));
        assert!(s.apply.contains("d.product_id = 42"));
        assert!(s.apply.contains("d.category_id = 27"));
        assert!(s.apply.contains("s0.keyword = 'kis-botu'"));
        // Quotes are doubled, so the value stays one SQL string.
        assert!(s.apply.contains("'Kış botu''); DROP TABLE x; --'"));
        assert!(
            s.apply.find("INSERT INTO `oc_kor_meta_undo`")
                < s.apply.find("UPDATE `oc_product_description`")
        );
        assert!(s.undo.contains("u.run = '20261113'"));
        let v4 = opencart_sql(&rows[1..2], "oc_", 1, 4, "r").unwrap();
        assert!(v4.apply.contains("s0.`key` = 'product_id'"));
        assert!(opencart_sql(&rows, "oc`;", 1, 3, "r").is_err());
        assert!(opencart_sql(&rows, "oc_", 1, 2, "r").is_err());
    }

    #[test]
    fn csv_round_trip_and_formula_guard() {
        let rows = vec![Row {
            kind: "page".into(),
            url: "https://x.test/a".into(),
            title: "=HYPERLINK(\"x\")".into(),
            description: "a, \"b\"\nc".into(),
            ..Row::default()
        }];
        let csv = to_csv(&rows);
        assert!(csv.contains("\"'=HYPERLINK(\"\"x\"\")\""));
        assert_eq!(from_csv(&csv).unwrap(), rows);
        assert!(from_csv("kind,url\nbad,x\n").is_err());
    }

    #[test]
    fn validation_flags() {
        let mut rows = vec![
            Row {
                kind: "page".into(),
                url: "a".into(),
                title: "Aynı başlık".into(),
                description: "kısa".into(),
                ..Row::default()
            },
            Row {
                kind: "page".into(),
                url: "b".into(),
                title: "Aynı başlık".into(),
                description: "x".repeat(200),
                ..Row::default()
            },
            Row {
                kind: "image".into(),
                url: "i".into(),
                alt: "Image of a cat".into(),
                ..Row::default()
            },
        ];
        let kw = HashMap::from([("a".to_owned(), "kargo".to_owned())]);
        validate(&mut rows, &kw);
        assert!(
            rows[0].issues.contains("title-duplicate")
                && rows[0].issues.contains("description-short")
                && rows[0].issues.contains("title-no-keyword")
        );
        assert!(rows[1].issues.contains("description-long"));
        assert!(rows[2].issues.contains("alt-redundant-prefix"));
        assert!(pixel_width("WWWW") > pixel_width("iiii"));
    }

    #[tokio::test]
    async fn generate_uses_examples_and_ocr_fallback_then_apply_and_undo() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/img/kedi.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png")
                    .set_body_bytes(vec![0x89, b'P', b'N', b'G']),
            )
            .mount(&s)
            .await;
        let mut rows = vec![
            Row {
                kind: "page".into(),
                url: format!("{}/done", s.uri()),
                title: "Örnek".into(),
                description: "Örnek açıklama".into(),
                ..Row::default()
            },
            Row {
                kind: "page".into(),
                url: format!("{}/hello", s.uri()),
                context: "Merhaba".into(),
                ..Row::default()
            },
            Row {
                kind: "image".into(),
                url: format!("{}/img/kedi.png", s.uri()),
                page: s.uri(),
                context: "Kediler".into(),
                ..Row::default()
            },
        ];
        let reply = format!(
            "```json\n[{{\"url\":\"{}/hello\",\"title\":\"Merhaba dünya\",\"description\":\"Açıklama\"}}]\n```",
            s.uri()
        );
        let ask = Scripted::new(&[&reply, "\"Uyuyan tekir kedi\""]);
        let n = generate(&mut rows, &ask, "tr", 100, "kor/test", &|_, _| {})
            .await
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(rows[1].title, "Merhaba dünya");
        assert_eq!(rows[2].alt, "Uyuyan tekir kedi");
        let prompts = ask.prompts.lock().unwrap().clone();
        assert!(prompts[0].contains("Örnek açıklama"));
        assert!(prompts[1].contains("You cannot see the image"));

        Mock::given(method("GET")).and(path("/wp-json/wp/v2/posts")).and(query_param("slug", "hello"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id": 3, "link": format!("{}/hello/", s.uri()), "excerpt": {"raw": "eski"}}]))).mount(&s).await;
        Mock::given(method("GET"))
            .and(path("/wp-json/wp/v2/posts"))
            .and(query_param("slug", "done"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/wp-json/wp/v2/pages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/wp-json/wp/v2/posts/3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 3, "excerpt": {"raw": "Açıklama"}})),
            )
            .expect(2)
            .mount(&s)
            .await;
        Mock::given(method("GET")).and(path("/wp-json/wp/v2/media")).and(query_param("search", "kedi"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id": 9, "source_url": format!("{}/img/kedi.png", s.uri()), "alt_text": ""}]))).mount(&s).await;
        Mock::given(method("POST"))
            .and(path("/wp-json/wp/v2/media/9"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 9, "alt_text": "Uyuyan tekir kedi"})),
            )
            .expect(2)
            .mount(&s)
            .await;
        let wp = Wp::new(&s.uri(), "u", "p").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let undo_file = dir.path().join("undo.jsonl");
        let a = apply(&rows, &wp, MetaKeys::None, &undo_file, &|_| {}).await;
        assert_eq!(a.changed.len(), 2, "{a:?}");
        assert_eq!(a.skipped.len(), 1);
        assert_eq!(undo(&undo_file, &wp).await.unwrap(), 2);
    }
}
