//! Turkey-specific layer (§29.14). Other countries can add a sibling module with the same shape.
//!
//! - Turkish SEO: slugs (ı/i, ş/s, ğ/g…), Turkish-character URL redirects, stopword- and
//!   suffix-aware keyword counts, locale checks (`lang`, `og:locale`, `hreflang`, encoding damage);
//! - TRY prices and KDV (VAT) with integer kuruş arithmetic;
//! - site checks: KVKK disclosure text (the items of KVKK art. 10), cookie notice and consent tool,
//!   trackers without a consent tool, VERBİS reminder, payment (iyzico, PayTR, Param, Sipay) and
//!   cargo integrations found on the site or in its plugins;
//! - generators: KVKK disclosure text and cookie policy, UBL-TR 1.2 e-Fatura/e-Arşiv invoices and
//!   e-İrsaliye despatch advices ([`efatura`]), payment integration templates ([`templates`]);
//! - marketplace adapters ([`market`]): Trendyol and Hepsiburada read-only counts.

pub mod efatura;
pub mod market;
pub mod templates;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};

use super::http::Http;

/// URL slug: Turkish letters folded the Turkish way (`İ`→`i`, `I`→`i`, `ı`→`i`), other Latin accents
/// stripped, everything else becomes `-`.
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let m = match c {
            'ı' | 'İ' | 'I' | 'î' | 'Î' | 'í' | 'ì' => "i",
            'ş' | 'Ş' => "s",
            'ğ' | 'Ğ' => "g",
            'ü' | 'Ü' | 'û' | 'ú' | 'ù' => "u",
            'ö' | 'Ö' | 'ô' | 'ó' | 'ò' => "o",
            'ç' | 'Ç' => "c",
            'â' | 'Â' | 'á' | 'à' | 'ä' => "a",
            'é' | 'è' | 'ê' | 'ë' => "e",
            c if c.is_ascii_alphanumeric() => {
                out.push(c.to_ascii_lowercase());
                continue;
            }
            _ => "-",
        };
        out.push_str(m);
    }
    let mut collapsed = String::with_capacity(out.len());
    for c in out.chars() {
        if c == '-' && (collapsed.is_empty() || collapsed.ends_with('-')) {
            continue;
        }
        collapsed.push(c);
    }
    collapsed.trim_end_matches('-').to_owned()
}

/// Turkish lower case (`I`→`ı`, `İ`→`i`), for keyword counting.
pub fn lower_tr(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'I' => 'ı',
            'İ' => 'i',
            c => c.to_lowercase().next().unwrap_or(c),
        })
        .collect()
}

/// Paths with Turkish letters (raw or percent-encoded) → their ASCII slug path, for redirect maps.
pub fn slug_redirects(paths: &[String]) -> Vec<(String, String)> {
    paths
        .iter()
        .filter_map(|p| {
            let decoded =
                url::form_urlencoded::parse(format!("x={}", p.replace('+', "%2B")).as_bytes())
                    .next()
                    .map(|(_, v)| v.into_owned())?;
            if decoded.is_ascii() {
                return None;
            }
            let ascii: Vec<String> = decoded
                .split('/')
                .map(|seg| {
                    if seg.is_empty() {
                        String::new()
                    } else {
                        slug(seg)
                    }
                })
                .collect();
            let target = ascii.join("/");
            (target != *p).then(|| (p.clone(), target))
        })
        .collect()
}

const STOPWORDS: &[&str] = &[
    "acaba", "ama", "ancak", "artık", "bana", "bazı", "belki", "ben", "beni", "benim", "bile",
    "bir", "biraz", "birçok", "biz", "bize", "bizim", "bu", "buna", "bunu", "bunun", "da", "daha",
    "de", "değil", "diye", "en", "gibi", "göre", "hem", "hep", "her", "hiç", "için", "ile", "ise",
    "işte", "kadar", "ki", "kim", "mi", "mı", "mu", "mü", "nasıl", "ne", "neden", "o", "olan",
    "olarak", "olduğu", "on", "onlar", "onu", "onun", "sen", "siz", "şey", "şu", "tüm", "ve",
    "veya", "ya", "yani", "çok", "çünkü", "sonra", "önce", "üzere", "var", "yok", "gerek", "aynı",
    "the", "and", "for", "with",
];

/// Inflectional suffixes stripped (longest first) so `ayakkabılar`, `ayakkabıları`, `ayakkabının`
/// count as `ayakkabı`. A light stemmer: the stem keeps at least four letters.
const SUFFIXES: &[&str] = &[
    "larından",
    "lerinden",
    "larında",
    "lerinde",
    "larının",
    "lerinin",
    "ları",
    "leri",
    "ların",
    "lerin",
    "lara",
    "lere",
    "lar",
    "ler",
    "ından",
    "inden",
    "undan",
    "ünden",
    "ında",
    "inde",
    "unda",
    "ünde",
    "dan",
    "den",
    "tan",
    "ten",
    "nın",
    "nin",
    "nun",
    "nün",
    "da",
    "de",
    "ta",
    "te",
    "ın",
    "in",
    "un",
    "ün",
    "yı",
    "yi",
    "yu",
    "yü",
    "ya",
    "ye",
];

pub fn stem(word: &str) -> String {
    // Case after plural (`ayakkabı-lar-da`): strip at most two suffixes.
    let mut w = word.to_owned();
    for _ in 0..2 {
        match SUFFIXES
            .iter()
            .find_map(|s| w.strip_suffix(s).filter(|b| b.chars().count() >= 4))
        {
            Some(base) => w = base.to_owned(),
            None => break,
        }
    }
    w
}

/// Most frequent stems of a text, stopwords removed: (stem, count).
pub fn keywords(text: &str, top: usize) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for w in lower_tr(text).split(|c: char| !c.is_alphanumeric()) {
        if w.chars().count() < 3 || STOPWORDS.contains(&w) || w.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        *counts.entry(stem(w)).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(top);
    v
}

/// `1234567` kuruş → `12.345,67 ₺`.
pub fn try_format(kurus: i64) -> String {
    let neg = kurus < 0;
    let k = kurus.unsigned_abs();
    let (lira, kr) = (k / 100, k % 100);
    let digits = lira.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    format!("{}{grouped},{kr:02} ₺", if neg { "-" } else { "" })
}

/// Parse `1.234,56`, `1234.56`, `1234` (TL) → kuruş.
pub fn parse_try(s: &str) -> Option<i64> {
    let t = s
        .trim()
        .trim_end_matches(['₺', ' '])
        .trim_end_matches("TL")
        .trim()
        .replace(' ', "");
    let (int, frac) = if let Some((a, b)) = t.rsplit_once(',') {
        (a.replace('.', ""), b.to_owned())
    } else if let Some((a, b)) = t.rsplit_once('.').filter(|(_, b)| b.len() <= 2) {
        (a.replace(',', ""), b.to_owned())
    } else {
        (t.replace(['.', ','], ""), String::new())
    };
    let lira: i64 = int.parse().ok()?;
    let kr: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        2 => frac.parse().ok()?,
        _ => return None,
    };
    lira.checked_mul(100)?.checked_add(kr)
}

/// KDV rates in force since 10 July 2023 (Presidential Decree 7346): 1, 10, 20 percent.
pub const KDV_RATES: [u32; 3] = [1, 10, 20];

/// Net kuruş at `rate`% → (KDV, gross), rounded half up to the kuruş.
pub fn kdv(net: i64, rate: u32) -> (i64, i64) {
    let tax = (net * i64::from(rate) + 50).div_euclid(100);
    (tax, net + tax)
}

/// Gross (KDV included) → (net, KDV).
pub fn kdv_from_gross(gross: i64, rate: u32) -> (i64, i64) {
    let d = 100 + i64::from(rate);
    let net = (gross * 100 + d / 2).div_euclid(d);
    (net, gross - net)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    /// Message id (`tr-*`), rendered with [`render`].
    pub id: String,
    /// `ok`, `warn`, `fail`, `info`.
    pub status: String,
    pub detail: String,
    /// What the item was found in (URL, plugin list).
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Report {
    pub url: String,
    pub items: Vec<Item>,
    pub payments: Vec<String>,
    pub cargo: Vec<String>,
}

fn has_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// KVKK art. 10 disclosure items and the words that show each is covered.
const KVKK_ITEMS: &[(&str, &[&str])] = &[
    ("tr-kvkk-controller", &["veri sorumlusu"]),
    (
        "tr-kvkk-purpose",
        &["amaç", "amacı", "amaçları", "amaçlarla"],
    ),
    ("tr-kvkk-transfer", &["aktarıl", "aktarım"]),
    (
        "tr-kvkk-method",
        &[
            "toplama yöntemi",
            "toplanma yöntemi",
            "yöntemle",
            "hukuki sebeb",
            "hukuki sebep",
        ],
    ),
    (
        "tr-kvkk-rights",
        &[
            "11. madde",
            "11 inci madde",
            "11. maddesi",
            "haklarınız",
            "bilgi talep",
            "başvuru",
        ],
    ),
];

const CMPS: &[(&str, &str)] = &[
    ("cookieyes", "CookieYes"),
    ("cookie-law-info", "CookieYes (GDPR Cookie Consent)"),
    ("cookiebot", "Cookiebot"),
    ("complianz", "Complianz"),
    ("onetrust", "OneTrust"),
    ("iubenda", "iubenda"),
    ("borlabs", "Borlabs Cookie"),
    ("cookie-notice", "Cookie Notice"),
    ("klaro", "Klaro"),
    ("osano", "Osano"),
    ("usercentrics", "Usercentrics"),
];

const TRACKERS: &[(&str, &str)] = &[
    ("googletagmanager.com/gtag", "Google Analytics"),
    ("google-analytics.com", "Google Analytics"),
    ("googletagmanager.com/gtm", "Google Tag Manager"),
    ("connect.facebook.net", "Meta Pixel"),
    ("mc.yandex.ru", "Yandex Metrica"),
    ("static.hotjar.com", "Hotjar"),
    ("analytics.tiktok.com", "TikTok Pixel"),
];

pub const PAYMENTS: &[(&str, &[&str])] = &[
    ("iyzico", &["iyzipay", "iyzico"]),
    ("PayTR", &["paytr"]),
    (
        "Param",
        &["param.com.tr", "parampos", "param-pos", "turkpos"],
    ),
    ("Sipay", &["sipay"]),
];

pub const CARGO: &[(&str, &[&str])] = &[
    ("Yurtiçi Kargo", &["yurtici", "yurtiçi kargo"]),
    ("Aras Kargo", &["araskargo", "aras-kargo", "aras kargo"]),
    ("MNG Kargo", &["mngkargo", "mng-kargo", "mng kargo"]),
    ("Sürat Kargo", &["suratkargo", "surat-kargo", "sürat kargo"]),
    ("PTT Kargo", &["pttkargo", "ptt-kargo", "ptt kargo"]),
];

static HREF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"(?is)<a\s[^>]*href\s*=\s*["']([^"'#]+)["'][^>]*>(.*?)</a>"#)
        .unwrap_or_else(|_| unreachable!())
});

/// The disclosure-text check on one page's text (lower-cased Turkish).
pub fn kvkk_items(text: &str, source: &str) -> Vec<Item> {
    let t = lower_tr(text);
    KVKK_ITEMS
        .iter()
        .map(|(id, words)| Item {
            id: (*id).into(),
            status: if has_any(&t, words) { "ok" } else { "fail" }.into(),
            detail: String::new(),
            source: source.into(),
        })
        .collect()
}

/// Locale checks on the home page HTML.
pub fn locale_items(html: &str, url: &str) -> Vec<Item> {
    let h = html.to_ascii_lowercase();
    let item = |id: &str, ok: bool, detail: String| Item {
        id: id.into(),
        status: if ok { "ok" } else { "warn" }.into(),
        detail,
        source: url.into(),
    };
    let lang = regex::Regex::new(r#"<html[^>]*\slang\s*=\s*["']([^"']+)"#)
        .ok()
        .and_then(|r| r.captures(&h).map(|c| c[1].to_owned()));
    let mut v = vec![
        item(
            "tr-lang",
            lang.as_deref().is_some_and(|l| l.starts_with("tr")),
            lang.unwrap_or_default(),
        ),
        item(
            "tr-og-locale",
            h.contains("og:locale") && h.contains("tr_tr"),
            String::new(),
        ),
        item(
            "tr-charset",
            h.contains("charset=utf-8") || h.contains("charset=\"utf-8\""),
            String::new(),
        ),
        item(
            "tr-hreflang",
            !h.contains("hreflang") || h.contains("hreflang=\"tr") || h.contains("hreflang='tr"),
            String::new(),
        ),
        item(
            "tr-yandex",
            h.contains("yandex-verification"),
            String::new(),
        ),
    ];
    // UTF-8 read as Latin-1: "Ä±" (ı), "ÅŸ" (ş), "Ã§" (ç), "Ã¼" (ü), "Ã¶" (ö), "ÄŸ" (ğ).
    let damaged = ["Ä±", "ÅŸ", "Ã§", "Ã¼", "Ã¶", "ÄŸ", "Ä°"]
        .iter()
        .filter(|m| html.contains(*m))
        .count();
    v.push(item(
        "tr-mojibake",
        damaged == 0,
        if damaged > 0 {
            format!("{damaged}")
        } else {
            String::new()
        },
    ));
    let non_ascii_links = HREF
        .captures_iter(html)
        .filter(|c| {
            let u = &c[1];
            u.starts_with('/')
                && (!u.is_ascii()
                    || has_any(
                        &u.to_ascii_uppercase(),
                        &[
                            "%C4%B1", "%C5%9F", "%C4%9F", "%C3%BC", "%C3%B6", "%C3%A7", "%C4%B0",
                        ],
                    ))
        })
        .count();
    v.push(item(
        "tr-url-chars",
        non_ascii_links == 0,
        if non_ascii_links > 0 {
            format!("{non_ascii_links}")
        } else {
            String::new()
        },
    ));
    v
}

fn detect(hay: &str, table: &[(&str, &[&str])]) -> Vec<String> {
    table
        .iter()
        .filter(|(_, n)| has_any(hay, n))
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

/// Every check on a live site. `plugins`: WordPress plugin slugs when known.
#[allow(clippy::too_many_lines)]
pub async fn check(http: &Http, url: &str, plugins: &[String]) -> super::Result<Report> {
    let base = url.trim_end_matches('/');
    let (_, home) = http.follow(&format!("{base}/")).await?;
    if !home.ok() {
        return Err(format!("{base}: HTTP {}", home.status));
    }
    let lower = lower_tr(&home.body);
    let mut rep = Report {
        url: base.to_owned(),
        ..Report::default()
    };
    // KVKK disclosure page: link text or address naming it.
    let kvkk_link = HREF.captures_iter(&home.body).find_map(|c| {
        let (href, text) = (c[1].to_owned(), lower_tr(&c[2]));
        let h = href.to_ascii_lowercase();
        let hit = has_any(&text, &["kvkk", "aydınlatma", "kişisel veri"])
            || has_any(&h, &["kvkk", "aydinlatma", "kisisel-veri"]);
        hit.then_some(href)
    });
    let privacy_link = HREF.captures_iter(&home.body).find_map(|c| {
        let (href, text) = (c[1].to_owned(), lower_tr(&c[2]));
        (has_any(&text, &["gizlilik", "çerez politika"])
            || has_any(
                &href.to_ascii_lowercase(),
                &["gizlilik", "privacy", "cerez"],
            ))
        .then_some(href)
    });
    match kvkk_link.or(privacy_link.clone()) {
        Some(href) => {
            let target = url::Url::parse(&format!("{base}/"))
                .and_then(|b| b.join(&href))
                .map(|u| u.to_string())
                .unwrap_or(href);
            rep.items.push(Item {
                id: "tr-kvkk-page".into(),
                status: "ok".into(),
                detail: target.clone(),
                source: format!("{base}/"),
            });
            match http.follow(&target).await {
                Ok((_, page)) if page.ok() => rep
                    .items
                    .extend(kvkk_items(&strip_tags(&page.body), &target)),
                Ok((_, page)) => rep.items.push(Item {
                    id: "tr-kvkk-page".into(),
                    status: "fail".into(),
                    detail: format!("HTTP {}", page.status),
                    source: target,
                }),
                Err(e) => rep.items.push(Item {
                    id: "tr-kvkk-page".into(),
                    status: "fail".into(),
                    detail: e,
                    source: target,
                }),
            }
        }
        None => rep.items.push(Item {
            id: "tr-kvkk-page".into(),
            status: "fail".into(),
            detail: String::new(),
            source: format!("{base}/"),
        }),
    }
    // Cookie notice: a consent tool, or banner text.
    let hay = format!("{}\n{}", home.body.to_ascii_lowercase(), plugins.join("\n"));
    let cmp: Vec<&str> = CMPS
        .iter()
        .filter(|(n, _)| hay.contains(n))
        .map(|(_, name)| *name)
        .collect();
    let banner = lower.contains("çerez") && has_any(&lower, &["kabul", "onay", "reddet", "tercih"]);
    rep.items.push(Item {
        id: "tr-cookie-notice".into(),
        status: if !cmp.is_empty() || banner {
            "ok"
        } else {
            "fail"
        }
        .into(),
        detail: cmp.join(", "),
        source: format!("{base}/"),
    });
    rep.items.push(Item {
        id: "tr-cookie-policy".into(),
        status: if lower.contains("çerez politika")
            || hay.contains("cerez-politika")
            || hay.contains("cookie-policy")
        {
            "ok"
        } else {
            "warn"
        }
        .into(),
        detail: String::new(),
        source: format!("{base}/"),
    });
    let trackers: Vec<&str> = TRACKERS
        .iter()
        .filter(|(n, _)| hay.contains(n))
        .map(|(_, name)| *name)
        .collect();
    if !trackers.is_empty() && cmp.is_empty() {
        rep.items.push(Item {
            id: "tr-trackers-no-consent".into(),
            status: "warn".into(),
            detail: trackers.join(", "),
            source: format!("{base}/"),
        });
    }
    rep.items.push(Item {
        id: "tr-verbis".into(),
        status: "info".into(),
        detail: "https://verbis.kvkk.gov.tr".into(),
        source: "KVKK".into(),
    });
    rep.items
        .extend(locale_items(&home.body, &format!("{base}/")));
    rep.payments = detect(&hay, PAYMENTS);
    rep.cargo = detect(&hay, CARGO);
    let ecommerce = plugins.iter().any(|p| p == "woocommerce")
        || has_any(&lower, &["sepete ekle", "sepetim", "add-to-cart"]);
    if ecommerce {
        rep.items.push(Item {
            id: "tr-payments".into(),
            status: if rep.payments.is_empty() {
                "warn"
            } else {
                "ok"
            }
            .into(),
            detail: rep.payments.join(", "),
            source: format!("{base}/"),
        });
        // Distance sales (Law 6502 / Distance Contracts Regulation): pre-information and contract texts.
        let ds = has_any(
            &lower,
            &["mesafeli satış", "mesafeli satis", "ön bilgilendirme"],
        ) || has_any(&hay, &["mesafeli-satis", "on-bilgilendirme"]);
        rep.items.push(Item {
            id: "tr-distance-sales".into(),
            status: if ds { "ok" } else { "warn" }.into(),
            detail: String::new(),
            source: format!("{base}/"),
        });
    }
    Ok(rep)
}

pub fn strip_tags(html: &str) -> String {
    let no_scripts = regex::Regex::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>")
        .map_or_else(
            |_| html.to_owned(),
            |r| r.replace_all(html, " ").into_owned(),
        );
    let text = regex::Regex::new(r"(?s)<[^>]+>").map_or(no_scripts.clone(), |r| {
        r.replace_all(&no_scripts, " ").into_owned()
    });
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&#8217;", "'")
}

pub fn render(l: &Localizer, rep: &Report) -> String {
    let mut o = format!(
        "# {}\n\n",
        l.t_args("tr-title", &args!("url" => rep.url.clone()))
    );
    let _ = writeln!(
        o,
        "| {} | {} | {} | {} |\n|---|---|---|---|",
        l.t("tr-col-check"),
        l.t("tr-col-status"),
        l.t("tr-col-detail"),
        l.t("tr-col-source")
    );
    for i in &rep.items {
        let _ = writeln!(
            o,
            "| {} | {} | {} | {} |",
            l.t(&i.id),
            l.t(&format!("tr-status-{}", i.status)),
            super::cell(&i.detail),
            super::cell(&i.source)
        );
    }
    let none = || "—".to_owned();
    let _ = writeln!(
        o,
        "\n- {}: {}\n- {}: {}\n\n> {}",
        l.t("tr-payments-found"),
        if rep.payments.is_empty() {
            none()
        } else {
            rep.payments.join(", ")
        },
        l.t("tr-cargo-found"),
        if rep.cargo.is_empty() {
            none()
        } else {
            rep.cargo.join(", ")
        },
        l.t("tr-verbis-note")
    );
    o
}

/// Input for the KVKK disclosure text and cookie policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KvkkInput {
    pub company: String,
    pub address: String,
    pub email: String,
    pub website: String,
    /// Personal data categories (`kimlik`, `iletişim`, `müşteri işlem`…).
    pub data: Vec<String>,
    pub purposes: Vec<String>,
    /// Recipients (`kargo şirketleri`, `ödeme kuruluşları`…).
    pub recipients: Vec<String>,
    /// Transfers abroad (e.g. cloud hosting, analytics).
    pub abroad: Vec<String>,
    /// Cookies in use: (name or tool, purpose, duration).
    pub cookies: Vec<(String, String, String)>,
}

/// Disclosure text per KVKK art. 10 with the art. 11 rights, and a cookie policy; Markdown in Turkish.
/// Not legal advice: the text says so and should be reviewed by a lawyer.
pub fn kvkk_text(i: &KvkkInput) -> (String, String) {
    let list = |v: &[String], fallback: &str| {
        if v.is_empty() {
            format!("- {fallback}\n")
        } else {
            v.iter().fold(String::new(), |mut o, x| {
                let _ = writeln!(o, "- {x}");
                o
            })
        }
    };
    let disclosure = format!(
        "# Kişisel Verilerin İşlenmesine İlişkin Aydınlatma Metni\n\n\
> Bu metin Kor tarafından şablondan üretilmiştir; hukuki görüş değildir. Yayımlamadan önce bir hukukçuya kontrol ettiriniz.\n\n\
## 1. Veri sorumlusu\n\n6698 sayılı Kişisel Verilerin Korunması Kanunu (“KVKK”) uyarınca veri sorumlusu **{c}** ({a}) olup kişisel verileriniz aşağıda açıklanan kapsamda işlenmektedir. İletişim: {e}\n\n\
## 2. İşlenen kişisel veriler\n\n{data}\n## 3. İşlenme amaçları\n\nKişisel verileriniz şu amaçlarla işlenmektedir:\n\n{purp}\n\
## 4. Aktarım\n\nKişisel verileriniz yukarıdaki amaçlarla sınırlı olarak KVKK’nın 8. ve 9. maddelerine uygun şekilde şu alıcı gruplarına aktarılabilir:\n\n{rec}\n{abroad}\
## 5. Toplama yöntemi ve hukuki sebep\n\nKişisel verileriniz {w} internet sitesi, e-posta ve sözleşme süreçleri aracılığıyla elektronik ortamda; KVKK’nın 5. maddesinde sayılan sözleşmenin kurulması veya ifası, hukuki yükümlülüğün yerine getirilmesi, meşru menfaat ve gerektiğinde açık rıza hukuki sebeplerine dayanılarak toplanmaktadır.\n\n\
## 6. Haklarınız (KVKK m. 11)\n\nKVKK’nın 11. maddesi uyarınca; kişisel verilerinizin işlenip işlenmediğini öğrenme, işlenmişse bilgi talep etme, işlenme amacını ve amacına uygun kullanılıp kullanılmadığını öğrenme, yurt içinde veya yurt dışında aktarıldığı üçüncü kişileri bilme, eksik veya yanlış işlenmişse düzeltilmesini, KVKK’nın 7. maddesi çerçevesinde silinmesini veya yok edilmesini isteme, bu işlemlerin aktarıldığı üçüncü kişilere bildirilmesini isteme, münhasıran otomatik sistemlerle analiz edilmesi suretiyle aleyhinize bir sonucun ortaya çıkmasına itiraz etme ve kanuna aykırı işlenmesi sebebiyle zarara uğramanız hâlinde zararın giderilmesini talep etme haklarına sahipsiniz.\n\n\
Başvurularınızı Veri Sorumlusuna Başvuru Usul ve Esasları Hakkında Tebliğ’e uygun olarak {e} adresine veya {a} adresine yazılı olarak iletebilirsiniz.\n",
        c = i.company,
        a = i.address,
        e = i.email,
        w = i.website,
        data = list(&i.data, "Kimlik, iletişim ve müşteri işlem bilgileri"),
        purp = list(
            &i.purposes,
            "Ürün ve hizmetlerin sunulması, sözleşme süreçlerinin yürütülmesi"
        ),
        rec = list(
            &i.recipients,
            "Yetkili kamu kurum ve kuruluşları, iş ortakları ve tedarikçiler"
        ),
        abroad = if i.abroad.is_empty() {
            String::new()
        } else {
            format!(
                "Yurt dışına aktarım (KVKK m. 9): {}\n\n",
                i.abroad.join(", ")
            )
        },
    );
    let mut cookies = format!(
        "# Çerez Politikası\n\n> Bu metin Kor tarafından şablondan üretilmiştir; hukuki görüş değildir.\n\n\
{w} internet sitesinde, sitenin çalışması için zorunlu çerezler ile yalnızca açık rızanız olduğunda analiz ve pazarlama çerezleri kullanılır. Zorunlu olmayan çerezler, çerez bildirimindeki tercihiniz olmadan yerleştirilmez; tercihinizi istediğiniz zaman değiştirebilirsiniz.\n\n\
| Çerez / araç | Amaç | Süre |\n|---|---|---|\n",
        w = i.website
    );
    for (n, p, d) in &i.cookies {
        let _ = writeln!(
            cookies,
            "| {} | {} | {} |",
            super::cell(n),
            super::cell(p),
            super::cell(d)
        );
    }
    let _ = write!(cookies, "\nVeri sorumlusu: {} — {}\n", i.company, i.email);
    (disclosure, cookies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn turkish_slugs_and_keywords() {
        assert_eq!(
            slug("İstanbul'da Şık Çantalar & Ayakkabılar"),
            "istanbul-da-sik-cantalar-ayakkabilar"
        );
        assert_eq!(slug("IĞDIR ÜRÜNLERİ"), "igdir-urunleri");
        assert_eq!(lower_tr("IŞIK İNCE"), "ışık ince");
        let k = keywords(
            "Ayakkabılar ve ayakkabıların bakımı: ayakkabı fiyatları. Ayakkabılarda indirim!",
            3,
        );
        assert_eq!(k[0], ("ayakkabı".into(), 4));
        let r = slug_redirects(&[
            "/kategori/%C3%A7anta".into(),
            "/ascii/yol".into(),
            "/ürün/kırmızı".into(),
        ]);
        assert_eq!(
            r,
            vec![
                ("/kategori/%C3%A7anta".into(), "/kategori/canta".into()),
                ("/ürün/kırmızı".into(), "/urun/kirmizi".into())
            ]
        );
    }

    #[test]
    fn money_and_vat() {
        assert_eq!(try_format(1_234_567), "12.345,67 ₺");
        assert_eq!(try_format(-5), "-0,05 ₺");
        assert_eq!(parse_try("12.345,67 ₺"), Some(1_234_567));
        assert_eq!(parse_try("1234.5"), Some(123_450));
        assert_eq!(parse_try("1.000"), Some(100_000));
        assert_eq!(kdv(10_000, 20), (2_000, 12_000));
        assert_eq!(kdv(333, 20), (67, 400));
        assert_eq!(kdv_from_gross(12_000, 20), (10_000, 2_000));
    }

    #[test]
    fn kvkk_text_covers_every_item() {
        let (d, c) = kvkk_text(&KvkkInput {
            company: "Acme Ltd. Şti.".into(),
            address: "İstanbul".into(),
            email: "kvkk@acme.com.tr".into(),
            website: "acme.com.tr".into(),
            cookies: vec![("_ga".into(), "Analiz".into(), "2 yıl".into())],
            ..KvkkInput::default()
        });
        assert!(
            kvkk_items(&d, "x").iter().all(|i| i.status == "ok"),
            "{:?}",
            kvkk_items(&d, "x")
        );
        assert!(c.contains("| _ga | Analiz | 2 yıl |"));
        assert!(d.contains("hukuki görüş değildir"));
    }

    #[tokio::test]
    async fn site_checks_on_a_fixture_shop() {
        let s = MockServer::start().await;
        let home = r#"<html lang="tr"><head><meta charset="utf-8"><meta property="og:locale" content="tr_TR">
<script src="https://www.googletagmanager.com/gtag/js?id=G-X"></script><script src="https://static.iyzipay.com/checkout.js"></script></head>
<body><a href="/kvkk-aydinlatma-metni">KVKK Aydınlatma Metni</a><a href="/ürün/çanta">Çanta</a> Sepete ekle. Bu sitede çerez kullanılır, kabul et.</body></html>"#;
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(home))
            .mount(&s)
            .await;
        Mock::given(path("/kvkk-aydinlatma-metni"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<h1>Aydınlatma</h1><p>Veri sorumlusu Acme. İşlenme amaçları. Hukuki sebep.</p>",
            ))
            .mount(&s)
            .await;
        let rep = check(&Http::new().unwrap(), &s.uri(), &[]).await.unwrap();
        let st = |id: &str| {
            rep.items
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.status.clone())
                .unwrap_or_default()
        };
        assert_eq!(st("tr-kvkk-page"), "ok");
        assert_eq!(st("tr-kvkk-controller"), "ok");
        assert_eq!(st("tr-kvkk-transfer"), "fail");
        assert_eq!(st("tr-cookie-notice"), "ok");
        assert_eq!(st("tr-trackers-no-consent"), "warn");
        assert_eq!(st("tr-lang"), "ok");
        assert_eq!(st("tr-url-chars"), "warn");
        assert_eq!(st("tr-distance-sales"), "warn");
        assert_eq!(rep.payments, ["iyzico"]);
        let md = render(&Localizer::new(kor_i18n::Lang::Tr), &rep);
        assert!(md.contains("iyzico"));
    }
}
