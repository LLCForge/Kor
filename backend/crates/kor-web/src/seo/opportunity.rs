//! Ranking opportunity score from Search Console rows (spec §29.16), deterministic and shown
//! with its formula:
//!
//! ```text
//! target  = 3 when position > 3, else 1
//! gap     = max(0, expected_ctr(target) − ctr)
//! extra   = impressions × gap                    (clicks the row would gain at the target)
//! score   = extra × band(position)
//! band    = 1.0 for 3 < pos ≤ 10, 0.7 for 10 < pos ≤ 20, 0.3 for 20 < pos ≤ 50,
//!           0.2 for pos ≤ 3, 0.1 beyond 50
//! index   = 100 × score / max(score)             (0–100 within this data set)
//! ```
//!
//! `expected_ctr(p)` is the median CTR of this site's own rows at rounded position `p` when at
//! least [`MIN_SAMPLES`] rows exist there; otherwise Kor's default curve ([`DEFAULT_CURVE`]),
//! labelled as an assumption in the output. Nothing is a prediction of rank; it ranks where
//! effort is likely to pay off first.

use serde::Serialize;

use super::google::Row;

/// Rows needed at a position before the site's own median CTR is used.
pub const MIN_SAMPLES: usize = 5;
/// Fallback CTR by position 1–10 (assumption; the site's own data replaces it when available).
pub const DEFAULT_CURVE: [f64; 10] = [0.28, 0.15, 0.10, 0.07, 0.05, 0.04, 0.03, 0.025, 0.02, 0.018];
pub const FORMULA: &str = "score = impressions × max(0, expected_ctr(target) − ctr) × band(position); target = 3 if position > 3 else 1; band: ≤3 0.2, ≤10 1.0, ≤20 0.7, ≤50 0.3, >50 0.1; index = 100 × score / max";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Opportunity {
    pub keys: Vec<String>,
    pub impressions: f64,
    pub clicks: f64,
    pub ctr: f64,
    pub position: f64,
    pub target: u32,
    pub expected_ctr: f64,
    /// `site` (median of own rows) or `default` (assumed curve).
    pub ctr_source: &'static str,
    pub band: f64,
    pub extra_clicks: f64,
    pub score: f64,
    /// 0–100 within the data set.
    pub index: f64,
}

pub fn band(position: f64) -> f64 {
    match position {
        p if p <= 3.0 => 0.2,
        p if p <= 10.0 => 1.0,
        p if p <= 20.0 => 0.7,
        p if p <= 50.0 => 0.3,
        _ => 0.1,
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        f64::midpoint(v[n / 2 - 1], v[n / 2])
    }
}

/// Expected CTR at positions 1–10 and where each value came from.
pub fn curve(rows: &[Row]) -> [(f64, &'static str); 10] {
    let mut out = [(0.0, "default"); 10];
    for (i, slot) in out.iter_mut().enumerate() {
        #[allow(clippy::cast_precision_loss)] // 1..=10
        let p = (i + 1) as f64;
        let mut own: Vec<f64> = rows
            .iter()
            .filter(|r| r.impressions > 0.0 && (r.position.round() - p).abs() < f64::EPSILON)
            .map(|r| r.ctr)
            .collect();
        *slot = if own.len() >= MIN_SAMPLES {
            (median(&mut own), "site")
        } else {
            (DEFAULT_CURVE[i], "default")
        };
    }
    out
}

/// Score every row, best opportunity first.
pub fn score(rows: &[Row]) -> Vec<Opportunity> {
    let c = curve(rows);
    let mut out: Vec<Opportunity> = rows
        .iter()
        .filter(|r| r.impressions > 0.0)
        .map(|r| {
            let target: u32 = if r.position > 3.0 { 3 } else { 1 };
            let (expected, src) = c[(target - 1) as usize];
            let gap = (expected - r.ctr).max(0.0);
            let extra = r.impressions * gap;
            let b = band(r.position);
            Opportunity {
                keys: r.keys.clone(),
                impressions: r.impressions,
                clicks: r.clicks,
                ctr: r.ctr,
                position: r.position,
                target,
                expected_ctr: expected,
                ctr_source: src,
                band: b,
                extra_clicks: extra,
                score: extra * b,
                index: 0.0,
            }
        })
        .collect();
    let max = out.iter().map(|o| o.score).fold(0.0, f64::max);
    for o in &mut out {
        o.index = if max > 0.0 {
            (100.0 * o.score / max * 10.0).round() / 10.0
        } else {
            0.0
        };
    }
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.keys.cmp(&b.keys))
    });
    out
}

// ---------------------------------------------------------------------------------------------
// Calibration (spec §29.16): predictions are kept and checked against Search Console later.
// ---------------------------------------------------------------------------------------------

/// Rows at or above this index are "predicted to improve" when worked on.
pub const PREDICT_INDEX: f64 = 50.0;
/// Predictions are checked once they are at least this old.
pub const CHECK_AFTER_DAYS: i64 = 30;

fn db(path: &std::path::Path) -> Result<rusqlite::Connection, String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let c = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS predictions (site TEXT NOT NULL, keys TEXT NOT NULL, made TEXT NOT NULL,
           position REAL NOT NULL, target INTEGER NOT NULL, idx REAL NOT NULL, PRIMARY KEY (site, keys, made));",
    )
    .map_err(|e| e.to_string())?;
    Ok(c)
}

/// Store today's top opportunities (`made` = `YYYY-MM-DD`).
pub fn record(
    path: &std::path::Path,
    site: &str,
    made: &str,
    scored: &[Opportunity],
) -> Result<usize, String> {
    let mut c = db(path)?;
    let tx = c.transaction().map_err(|e| e.to_string())?;
    let mut n = 0;
    for o in scored.iter().filter(|o| o.index >= PREDICT_INDEX) {
        n += tx
            .execute(
                "INSERT OR REPLACE INTO predictions VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    site,
                    serde_json::to_string(&o.keys).unwrap_or_default(),
                    made,
                    o.position,
                    o.target,
                    o.index
                ],
            )
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(n)
}

/// Past accuracy: of predictions made at least [`CHECK_AFTER_DAYS`] before `today`, the share
/// whose position is better now (rows from Search Console for the same keys).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Calibration {
    pub checked: usize,
    pub improved: usize,
    pub hit_rate: f64,
}

pub fn calibration(
    path: &std::path::Path,
    site: &str,
    today: chrono::NaiveDate,
    now: &[Row],
) -> Result<Option<Calibration>, String> {
    let c = db(path)?;
    let cutoff = (today - chrono::Days::new(CHECK_AFTER_DAYS.unsigned_abs()))
        .format("%Y-%m-%d")
        .to_string();
    let mut st = c
        .prepare("SELECT keys, position FROM predictions WHERE site = ?1 AND made <= ?2")
        .map_err(|e| e.to_string())?;
    let past: Vec<(String, f64)> = st
        .query_map(rusqlite::params![site, cutoff], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let cur: std::collections::HashMap<String, f64> = now
        .iter()
        .map(|r| {
            (
                serde_json::to_string(&r.keys).unwrap_or_default(),
                r.position,
            )
        })
        .collect();
    let (mut checked, mut improved) = (0usize, 0usize);
    for (k, pos) in past {
        if let Some(p) = cur.get(&k) {
            checked += 1;
            if *p < pos {
                improved += 1;
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    Ok((checked > 0).then(|| Calibration {
        checked,
        improved,
        hit_rate: improved as f64 / checked as f64,
    }))
}

/// Keyword rows from a CSV (`kor seo opportunities --keywords list.csv`): a Search Console
/// "Queries" export in English or Turkish, or any list with a keyword, impressions (or search
/// volume) and position column. Delimiter `,`, `;` or tab; decimals with `.` or `,`; CTR as
/// `0.032`, `3.2%` or `3,2%` (computed from clicks when missing).
///
/// # Errors
/// No header row, or no keyword / impressions / position column.
#[allow(clippy::too_many_lines)] // header aliases and number formats, read in one pass
pub fn rows_from_csv(text: &str) -> Result<Vec<Row>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head = lines.next().ok_or("empty file")?;
    let delim = [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| head.matches(*d).count())
        .unwrap_or(',');
    let norm = |s: &str| {
        s.trim()
            .trim_matches('"')
            .to_lowercase()
            .replace(['ı', 'İ'], "i")
            .replace('ş', "s")
            .replace('ğ', "g")
            .replace('ü', "u")
            .replace('ö', "o")
            .replace('ç', "c")
    };
    let cols: Vec<String> = split_csv(head, delim).iter().map(|c| norm(c)).collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.iter().any(|n| c == n));
    let kw = find(&[
        "query",
        "queries",
        "top queries",
        "keyword",
        "keywords",
        "sorgu",
        "sorgular",
        "en cok yapilan sorgular",
        "anahtar kelime",
        "kelime",
    ])
    .ok_or("no keyword column (query / keyword / sorgu / anahtar kelime)")?;
    let imp = find(&[
        "impressions",
        "gosterimler",
        "gosterim",
        "volume",
        "search volume",
        "hacim",
        "arama hacmi",
    ])
    .ok_or("no impressions or search volume column (impressions / gösterimler / volume / hacim)")?;
    let pos = find(&[
        "position",
        "average position",
        "konum",
        "ortalama konum",
        "pozisyon",
        "sira",
    ])
    .ok_or("no position column (position / konum / pozisyon)")?;
    let clicks = find(&["clicks", "tiklamalar", "tiklama"]);
    let ctr = find(&["ctr", "to", "tiklama orani"]);
    let num = |s: &str| -> Option<f64> {
        let s = s.trim().trim_matches('"').trim_end_matches('%').trim();
        // `1.234,5` and `1,234.5` both mean 1234.5; a lone comma is a decimal comma.
        let s = match (s.rfind(','), s.rfind('.')) {
            (Some(c), Some(d)) if c > d => s.replace('.', "").replace(',', "."),
            (Some(_), Some(_)) => s.replace(',', ""),
            (Some(_), None) => s.replace(',', "."),
            _ => s.to_owned(),
        };
        s.parse().ok()
    };
    // Counts (impressions, clicks) are whole numbers: `1.200` / `1,200` is a thousands separator.
    let count = |s: &str| -> Option<f64> {
        let raw = s.trim().trim_matches('"');
        let groups: Vec<&str> = raw.split(['.', ',']).collect();
        let grouped = groups.len() > 1
            && groups[0].len() <= 3
            && groups
                .iter()
                .all(|g| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit()))
            && groups[1..].iter().all(|g| g.len() == 3);
        if grouped {
            groups.concat().parse().ok()
        } else {
            num(raw)
        }
    };
    let mut out = Vec::new();
    for line in lines {
        let f = split_csv(line, delim);
        let get = |i: usize| f.get(i).map_or("", String::as_str);
        let (Some(impressions), Some(position)) = (count(get(imp)), num(get(pos))) else {
            continue;
        };
        let keyword = get(kw).trim().to_owned();
        if keyword.is_empty() || impressions <= 0.0 {
            continue;
        }
        let clicks = clicks.and_then(|i| count(get(i))).unwrap_or(0.0);
        let ctr = ctr
            .and_then(|i| {
                let raw = get(i);
                num(raw).map(|v| {
                    if raw.contains('%') || v > 1.0 {
                        v / 100.0
                    } else {
                        v
                    }
                })
            })
            .unwrap_or(clicks / impressions);
        out.push(Row {
            keys: vec![keyword],
            clicks,
            impressions,
            ctr,
            position,
        });
    }
    Ok(out)
}

/// One CSV line; `"…"` fields may hold the delimiter and `""` for a quote.
fn split_csv(line: &str, delim: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                out.last_mut().unwrap_or(&mut String::new()).push('"');
            }
            '"' => quoted = !quoted,
            c if c == delim && !quoted => out.push(String::new()),
            c => {
                if let Some(s) = out.last_mut() {
                    s.push(c);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SERP competition factors (spec §29.16): how hard the current top results are to beat.
// ---------------------------------------------------------------------------------------------

pub const COMPETITION_FORMULA: &str = "difficulty = 100 × (0.35 × authority + 0.25 × title_match + 0.25 × depth + 0.15 × (1 − ugc)); authority = mean source authority / 100 (0.5 when no backlink source answered), title_match = share of titles containing every keyword word, depth = min(1, mean words / 2500), ugc = share of forum / Q&A results; adjusted score = score × (1 − difficulty / 200)";

/// Forum and Q&A hosts: a SERP full of them is easier to enter.
const UGC: &[&str] = &[
    "reddit.com",
    "quora.com",
    "eksisozluk.com",
    "donanimhaber.com",
    "technopat.net",
    "sikayetvar.com",
    "stackoverflow.com",
    "kizlarsoruyor.com",
    "r10.net",
    "medium.com",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Competition {
    pub keyword: String,
    pub results: usize,
    /// 0–1 each.
    pub authority: f64,
    pub authority_known: bool,
    pub title_match: f64,
    pub depth: f64,
    pub ugc: f64,
    /// 0–100.
    pub difficulty: f64,
}

fn host(u: &str) -> String {
    url::Url::parse(u)
        .ok()
        .and_then(|x| {
            x.host_str()
                .map(|h| h.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_default()
}

/// Competition of a keyword's top results; `authority` maps a domain to its 0–100 authority from
/// a backlink source (empty when none is connected).
#[allow(clippy::implicit_hasher, clippy::cast_precision_loss)]
pub fn competition(
    keyword: &str,
    pages: &[super::content::SerpPage],
    authority: &std::collections::HashMap<String, f64>,
) -> Competition {
    let lower = crate::site::tr::lower_tr;
    let words: Vec<String> = lower(keyword)
        .split_whitespace()
        .filter(|w| w.chars().count() > 2)
        .map(str::to_owned)
        .collect();
    let n = pages.len().max(1) as f64;
    let share = |f: &dyn Fn(&super::content::SerpPage) -> bool| {
        pages.iter().filter(|p| f(p)).count() as f64 / n
    };
    let title_match = share(&|p| {
        let t = lower(&p.title);
        !words.is_empty() && words.iter().all(|w| t.contains(w.as_str()))
    });
    let ugc = share(&|p| {
        let h = host(&p.url);
        UGC.iter().any(|u| h == *u || h.ends_with(&format!(".{u}")))
    });
    let depth = (pages.iter().map(|p| p.word_count as f64).sum::<f64>() / n / 2500.0).min(1.0);
    let known: Vec<f64> = pages
        .iter()
        .filter_map(|p| authority.get(&host(&p.url)).copied())
        .collect();
    let authority_known = !known.is_empty();
    let auth = if authority_known {
        (known.iter().sum::<f64>() / known.len() as f64 / 100.0).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let difficulty = 100.0 * (0.35 * auth + 0.25 * title_match + 0.25 * depth + 0.15 * (1.0 - ugc));
    Competition {
        keyword: keyword.to_owned(),
        results: pages.len(),
        authority: auth,
        authority_known,
        title_match,
        depth,
        ugc,
        difficulty: (difficulty * 10.0).round() / 10.0,
    }
}

/// An opportunity score lowered by its keyword's competition (halved at difficulty 100).
pub fn adjusted(score: f64, c: &Competition) -> f64 {
    score * (1.0 - c.difficulty / 200.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn competition_factors() {
        use super::super::content::SerpPage;
        let p = |u: &str, t: &str, w: usize| SerpPage {
            url: u.into(),
            title: t.into(),
            headings: Vec::new(),
            word_count: w,
        };
        let pages = [
            p("https://www.buyuk.com/a", "Kargo Takip Nasıl Yapılır", 3000),
            p("https://www.reddit.com/r/x", "kargo sorusu", 200),
        ];
        let mut auth = std::collections::HashMap::new();
        let c = super::competition("kargo takip", &pages, &auth);
        assert!((c.title_match - 0.5).abs() < 1e-9 && (c.ugc - 0.5).abs() < 1e-9);
        assert!(!c.authority_known && (c.authority - 0.5).abs() < 1e-9);
        auth.insert("buyuk.com".to_owned(), 80.0);
        let c2 = super::competition("kargo takip", &pages, &auth);
        assert!(c2.authority_known && c2.difficulty > c.difficulty);
        assert!(super::adjusted(10.0, &c2) < 10.0);
    }

    use super::*;

    #[test]
    fn keyword_csv_in_turkish_and_english() {
        let tr = "\u{feff}En çok yapılan sorgular;Tıklamalar;Gösterimler;TO;Konum\n\"kargo takip; online\";12;1.200;1%;8,4\nboş satır;0;0;0%;3\nkdv hesaplama;40;800;5%;2,1\n";
        let rows = rows_from_csv(tr).unwrap();
        assert_eq!(rows.len(), 2, "rows without impressions are skipped");
        assert_eq!(rows[0].keys, ["kargo takip; online"]);
        assert!((rows[0].impressions - 1200.0).abs() < 1e-9);
        assert!((rows[0].ctr - 0.01).abs() < 1e-9);
        assert!((rows[0].position - 8.4).abs() < 1e-9);
        let en = "keyword,search volume,position\nseo audit,500,14\n";
        let rows = rows_from_csv(en).unwrap();
        assert!((rows[0].ctr).abs() < 1e-9, "no clicks: CTR 0");
        assert!(!score(&rows).is_empty());
        assert!(
            rows_from_csv("keyword,volume\nx,1\n")
                .unwrap_err()
                .contains("position")
        );
    }

    fn r(k: &str, imp: f64, ctr: f64, pos: f64) -> Row {
        Row {
            keys: vec![k.into()],
            clicks: imp * ctr,
            impressions: imp,
            ctr,
            position: pos,
        }
    }

    #[test]
    fn striking_distance_wins_and_formula_holds() {
        let rows = [
            r("top", 10_000.0, 0.30, 1.2),
            r("p7", 5_000.0, 0.01, 7.0),
            r("p15", 5_000.0, 0.005, 15.0),
            r("p80", 20_000.0, 0.0, 80.0),
        ];
        let s = score(&rows);
        assert_eq!(s[0].keys[0], "p7");
        // 5000 × (0.10 − 0.01) × 1.0 = 450
        assert!((s[0].score - 450.0).abs() < 1e-9);
        assert!((s[0].index - 100.0).abs() < 1e-9);
        assert_eq!(s[0].ctr_source, "default");
        let top = s.iter().find(|o| o.keys[0] == "top").unwrap();
        assert!(top.score.abs() < 1e-12); // above the expected CTR at position 1
        // Deterministic: same input, same output.
        assert_eq!(score(&rows), s);
    }

    #[test]
    fn predictions_are_checked_after_thirty_days() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("predictions.sqlite");
        let rows = [
            r("a", 5_000.0, 0.01, 7.0),
            r("b", 5_000.0, 0.02, 8.0),
            r("c", 10.0, 0.0, 60.0),
        ];
        let scored = score(&rows);
        assert_eq!(record(&p, "s", "2026-09-01", &scored).unwrap(), 2);
        let d = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let later = [r("a", 5_000.0, 0.05, 4.0), r("b", 5_000.0, 0.02, 9.0)];
        assert_eq!(calibration(&p, "s", d("2026-09-15"), &later).unwrap(), None);
        let c = calibration(&p, "s", d("2026-10-05"), &later)
            .unwrap()
            .unwrap();
        assert_eq!((c.checked, c.improved), (2, 1));
        assert!((c.hit_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn site_curve_replaces_default_with_enough_rows() {
        let rows: Vec<Row> = (0..5)
            .map(|i| r(&format!("q{i}"), 100.0, 0.2 + f64::from(i) * 0.01, 3.0))
            .collect();
        let c = curve(&rows);
        assert!((c[2].0 - 0.22).abs() < 1e-9 && c[2].1 == "site");
        assert_eq!(c[0].1, "default");
    }
}
