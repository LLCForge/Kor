//! Client proposal and contract generator (§29.12).
//!
//! Input: a job description (text, meeting notes, a client email) and/or explicit items
//! (`Tasarım=12` hours) and packages from `business.toml`. Output: scope items with hours, phased
//! price with KDV and a payment plan, deliverables, revision rights; plus a service contract, a KVKK
//! data processing annex and an NDA — in Turkish or English, as Markdown/HTML/PDF/DOCX.
//!
//! Hours given by the user are used as-is. Hours Kor fills in come from a keyword table and are
//! marked `≈` (spec §5.2: no number without a source). Templates are editable files in
//! `templates/web/` (created from the built-ins on first use); `{{name}}` placeholders are filled.
//! Every contract carries the "not legal advice, have it reviewed" notice.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Result;
use super::tr::{kdv, try_format};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Package {
    pub name: String,
    /// Price without KDV, in lira.
    pub price: f64,
    pub hours: f64,
    pub includes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Efatura {
    /// Integrator endpoint for UBL-TR uploads.
    pub url: String,
    /// Vault entry with the integrator token.
    pub secret: String,
    /// Three-character invoice series (`KOR`).
    pub series: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Market {
    /// Trendyol seller id / Hepsiburada merchant id / Domain Name API reseller id; `test` for
    /// Yurtiçi's test service. Unused for N11.
    pub id: String,
    /// Vault names: API key/user and secret/password.
    pub key: String,
    pub secret: String,
}

/// `business.toml`: the agency's details and rates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Business {
    pub name: String,
    pub owner: String,
    pub email: String,
    pub phone: String,
    pub address: String,
    pub city: String,
    pub website: String,
    pub tax_office: String,
    pub tax_id: String,
    pub iban: String,
    /// Hourly rate without KDV, lira.
    pub hourly_rate: f64,
    /// KDV percent.
    pub vat: u32,
    pub hours_per_day: f64,
    pub revisions: u32,
    /// Payment plan percentages, summing to 100.
    pub payment_plan: Vec<u32>,
    pub accent: String,
    pub packages: Vec<Package>,
    pub efatura: Efatura,
    pub trendyol: Market,
    pub hepsiburada: Market,
    pub n11: Market,
    /// Yurtiçi Kargo web service user (`key`) and password (`secret`).
    pub yurtici: Market,
    /// Domain Name API: reseller id (`id`) and API key (`key`).
    pub dna: Market,
}

impl Default for Business {
    fn default() -> Self {
        Self {
            name: String::new(),
            owner: String::new(),
            email: String::new(),
            phone: String::new(),
            address: String::new(),
            city: String::new(),
            website: String::new(),
            tax_office: String::new(),
            tax_id: String::new(),
            iban: String::new(),
            hourly_rate: 0.0,
            vat: 20,
            hours_per_day: 6.0,
            revisions: 2,
            payment_plan: vec![40, 30, 30],
            accent: "#c96442".into(),
            packages: Vec::new(),
            efatura: Efatura::default(),
            trendyol: Market::default(),
            hepsiburada: Market::default(),
            n11: Market::default(),
            yurtici: Market::default(),
            dna: Market::default(),
        }
    }
}

pub fn business_path(base: &Path) -> PathBuf {
    base.join("business.toml")
}

/// The user's `business.toml`, or defaults when there is none.
pub fn business(base: &Path) -> Result<Business> {
    let p = business_path(base);
    if p.is_file() {
        super::load_toml(&p)
    } else {
        Ok(Business::default())
    }
}

/// Write a commented starter `business.toml` (never over an existing one).
pub fn init_business(base: &Path) -> Result<PathBuf> {
    let p = business_path(base);
    if p.exists() {
        return Err(format!("{} already exists", p.display()));
    }
    super::write_file(&p, STARTER)?;
    Ok(p)
}

const STARTER: &str = "# Kor teklif/sözleşme ayarları (kor proposal). Tutarlar KDV hariç, TL.\n\
name = \"\"\nowner = \"\"\nemail = \"\"\nphone = \"\"\naddress = \"\"\ncity = \"\"\nwebsite = \"\"\n\
tax_office = \"\"\ntax_id = \"\"\niban = \"\"\nhourly_rate = 0\nvat = 20\nhours_per_day = 6\nrevisions = 2\n\
payment_plan = [40, 30, 30]\naccent = \"#c96442\"\n\n\
# [[packages]]\n# name = \"Kurumsal site\"\n# price = 25000\n# hours = 30\n# includes = [\"5 sayfa\", \"İletişim formu\", \"Temel SEO\"]\n\n\
# [efatura]   # özel entegratör (UBL-TR gönderimi)\n# url = \"\"\n# secret = \"EFATURA_TOKEN\"   # kor vault add EFATURA_TOKEN\n# series = \"KOR\"\n\n\
# [n11]   # kor tr market\n# key = \"N11_APP_KEY\"\n# secret = \"N11_APP_SECRET\"\n\n\
# [yurtici]   # kor tr cargo; id = \"test\" test servisini kullanır\n# key = \"YURTICI_USER\"\n# secret = \"YURTICI_PASSWORD\"\n\n\
# [dna]   # kor tr dns (Domain Name API)\n# id = \"<bayi kimliği>\"\n# key = \"DNA_API_KEY\"\n";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub title: String,
    pub hours: f64,
    /// Hours from the keyword table, not from the user.
    pub estimated: bool,
    /// Fixed price (packages), lira; otherwise hours × rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
}

/// Hours Kor assumes for a scope line when the user gave none (keyword, hours).
const ESTIMATES: &[(&[&str], f64)] = &[
    (
        &[
            "e-ticaret",
            "eticaret",
            "woocommerce",
            "ödeme",
            "odeme",
            "payment",
            "checkout",
            "sepet",
        ],
        16.0,
    ),
    (&["tasarım", "tasarim", "design", "arayüz", "ui"], 16.0),
    (
        &["taşıma", "tasima", "migration", "yönlendirme", "redirect"],
        8.0,
    ),
    (
        &[
            "geliştirme",
            "gelistirme",
            "development",
            "kodlama",
            "eklenti",
            "plugin",
            "tema",
            "theme",
        ],
        24.0,
    ),
    (
        &[
            "entegrasyon",
            "integration",
            "api",
            "kargo",
            "pazaryeri",
            "e-fatura",
        ],
        12.0,
    ),
    (&["seo", "arama motoru"], 8.0),
    (&["içerik", "icerik", "content", "metin", "blog"], 8.0),
    (&["logo", "kurumsal kimlik"], 6.0),
    (&["test", "kontrol"], 6.0),
    (
        &[
            "kurulum",
            "install",
            "setup",
            "hosting",
            "ssl",
            "alan adı",
            "domain",
        ],
        4.0,
    ),
    (
        &["bakım", "bakim", "maintenance", "güncelleme", "update"],
        4.0,
    ),
    (&["eğitim", "egitim", "training"], 3.0),
];
const DEFAULT_HOURS: f64 = 6.0;

fn hours_in(line: &str) -> Option<f64> {
    let re = regex::Regex::new(r"(?i)(\d+(?:[.,]\d+)?)\s*(saat|sa\b|hours?|hrs?|h\b)").ok()?;
    re.captures(line)
        .and_then(|c| c[1].replace(',', ".").parse().ok())
}

pub fn estimate(line: &str) -> f64 {
    let l = super::tr::lower_tr(line);
    ESTIMATES
        .iter()
        .find(|(keys, _)| keys.iter().any(|k| l.contains(k)))
        .map_or(DEFAULT_HOURS, |(_, h)| *h)
}

/// Scope items from free text: bullet and numbered lines; without any, sentences of 3+ words.
pub fn scope_from_text(text: &str) -> Vec<Item> {
    let mut lines: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            let b = l
                .strip_prefix("- ")
                .or_else(|| l.strip_prefix("* "))
                .or_else(|| l.strip_prefix("• "));
            let n = l
                .split_once(". ")
                .filter(|(n, _)| {
                    !n.is_empty() && n.len() <= 2 && n.bytes().all(|c| c.is_ascii_digit())
                })
                .map(|(_, r)| r);
            let n = n.or_else(|| {
                l.split_once(") ")
                    .filter(|(n, _)| {
                        !n.is_empty() && n.len() <= 2 && n.bytes().all(|c| c.is_ascii_digit())
                    })
                    .map(|(_, r)| r)
            });
            b.or(n).map(str::to_owned)
        })
        .collect();
    if lines.is_empty() {
        lines = text
            .split(['.', '\n', '!', '?'])
            .map(str::trim)
            .filter(|s| s.split_whitespace().count() >= 3)
            .filter(|s| {
                let l = super::tr::lower_tr(s);
                ![
                    "merhaba",
                    "selam",
                    "saygılar",
                    "teşekkür",
                    "iyi çalışmalar",
                    "hello",
                    "regards",
                    "thanks",
                ]
                .iter()
                .any(|g| l.starts_with(g))
            })
            .map(str::to_owned)
            .take(15)
            .collect();
    }
    lines
        .into_iter()
        .map(|l| {
            let given = hours_in(&l);
            Item {
                hours: given.unwrap_or_else(|| estimate(&l)),
                estimated: given.is_none(),
                title: l,
                price: None,
            }
        })
        .collect()
}

/// `Tasarım=12` (hours) items.
pub fn parse_item(s: &str) -> Result<Item> {
    let (t, h) = s
        .rsplit_once('=')
        .ok_or_else(|| format!("`{s}`: use title=hours"))?;
    let hours: f64 = h
        .trim()
        .replace(',', ".")
        .parse()
        .map_err(|_| format!("`{s}`: hours must be a number"))?;
    if t.trim().is_empty() || !(0.0..=10_000.0).contains(&hours) {
        return Err(format!("`{s}`: invalid item"));
    }
    Ok(Item {
        title: t.trim().to_owned(),
        hours,
        estimated: false,
        price: None,
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Tr,
    En,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tr" => Some(Self::Tr),
            "en" => Some(Self::En),
            _ => None,
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Tr => "tr",
            Self::En => "en",
        }
    }
}

/// A proposal's state (`proposal.json`), the input of revisions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    pub client: String,
    pub project: String,
    pub lang: Lang,
    pub items: Vec<Item>,
    pub version: u32,
    pub date: String,
    pub valid_days: u32,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub hours: f64,
    pub net: i64,
    pub vat: i64,
    pub gross: i64,
    /// (label, percent, kuruş).
    pub plan: Vec<(String, u32, i64)>,
    pub days: f64,
    pub estimated: bool,
}

#[allow(clippy::cast_possible_truncation)]
fn kurus(lira: f64) -> i64 {
    (lira * 100.0).round() as i64
}

pub fn totals(p: &Proposal, b: &Business) -> Totals {
    let hours: f64 = p.items.iter().map(|i| i.hours).sum();
    let net: i64 = p
        .items
        .iter()
        .map(|i| kurus(i.price.unwrap_or(i.hours * b.hourly_rate)))
        .sum();
    let (vat, gross) = kdv(net, b.vat);
    let labels: &[&str] = match p.lang {
        Lang::Tr => &[
            "Başlangıç (sözleşme imzasında)",
            "Ara teslim (tasarım/geliştirme onayında)",
            "Teslim (yayına alındığında)",
            "Ek aşama",
        ],
        Lang::En => &[
            "Start (on signing)",
            "Milestone (design/development approval)",
            "Delivery (on launch)",
            "Extra stage",
        ],
    };
    let plan_pct: Vec<u32> = if b.payment_plan.iter().sum::<u32>() == 100 {
        b.payment_plan.clone()
    } else {
        vec![100]
    };
    let mut plan = Vec::new();
    let mut given = 0;
    for (i, pct) in plan_pct.iter().enumerate() {
        let amount = if i + 1 == plan_pct.len() {
            gross - given
        } else {
            gross * i64::from(*pct) / 100
        };
        given += amount;
        plan.push((
            (*labels.get(i).unwrap_or(&labels[3])).to_owned(),
            *pct,
            amount,
        ));
    }
    Totals {
        hours,
        net,
        vat,
        gross,
        plan,
        days: if b.hours_per_day > 0.0 {
            (hours / b.hours_per_day).ceil()
        } else {
            0.0
        },
        estimated: p.items.iter().any(|i| i.estimated),
    }
}

fn fmt_hours(h: f64) -> String {
    let s = format!("{h:.1}");
    s.trim_end_matches('0')
        .trim_end_matches('.')
        .replace('.', ",")
}

/// `{{key}}` → value; unknown keys stay visible so the user sees what to fill.
pub fn fill(template: &str, vars: &BTreeMap<&str, String>) -> String {
    let mut out = template.to_owned();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

pub fn templates_dir(base: &Path) -> PathBuf {
    base.join("templates").join("web")
}

/// Built-in templates: (kind, language) → text.
fn builtin(kind: &str, lang: Lang) -> &'static str {
    match (kind, lang) {
        ("proposal", Lang::Tr) => include_str!("templates/proposal.tr.md"),
        ("proposal", Lang::En) => include_str!("templates/proposal.en.md"),
        ("contract", Lang::Tr) => include_str!("templates/contract.tr.md"),
        ("contract", Lang::En) => include_str!("templates/contract.en.md"),
        ("dpa", Lang::Tr) => include_str!("templates/dpa.tr.md"),
        ("dpa", Lang::En) => include_str!("templates/dpa.en.md"),
        ("nda", Lang::Tr) => include_str!("templates/nda.tr.md"),
        _ => include_str!("templates/nda.en.md"),
    }
}

pub const KINDS: [&str; 4] = ["proposal", "contract", "dpa", "nda"];

/// The user's editable copy, created from the built-in on first use.
pub fn template(base: &Path, kind: &str, lang: Lang) -> Result<String> {
    if !KINDS.contains(&kind) {
        return Err(format!("unknown template `{kind}`"));
    }
    let p = templates_dir(base).join(format!("{kind}.{}.md", lang.code()));
    if !p.is_file() {
        super::write_file(&p, builtin(kind, lang))?;
    }
    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Variables shared by all templates.
pub fn vars(p: &Proposal, b: &Business) -> BTreeMap<&'static str, String> {
    let t = totals(p, b);
    let tr = p.lang == Lang::Tr;
    let mut items = if tr {
        "| # | İş | Saat | Tutar (KDV hariç) |\n|---|---|---|---|\n"
    } else {
        "| # | Work | Hours | Amount (excl. VAT) |\n|---|---|---|---|\n"
    }
    .to_owned();
    for (i, it) in p.items.iter().enumerate() {
        let amount = kurus(it.price.unwrap_or(it.hours * b.hourly_rate));
        let _ = writeln!(
            items,
            "| {} | {} | {}{} | {} |",
            i + 1,
            super::cell(&it.title),
            if it.estimated { "≈ " } else { "" },
            fmt_hours(it.hours),
            try_format(amount)
        );
    }
    let mut plan = if tr {
        "| Aşama | Oran | Tutar (KDV dahil) |\n|---|---|---|\n"
    } else {
        "| Stage | Share | Amount (incl. VAT) |\n|---|---|---|\n"
    }
    .to_owned();
    for (label, pct, amount) in &t.plan {
        let _ = writeln!(plan, "| {label} | %{pct} | {} |", try_format(*amount));
    }
    let deliverables: String = p.items.iter().fold(String::new(), |mut o, i| {
        let _ = writeln!(o, "- {}", i.title);
        o
    });
    let mut v = BTreeMap::new();
    v.insert("client", p.client.clone());
    v.insert("project", p.project.clone());
    v.insert("date", p.date.clone());
    v.insert("version", p.version.to_string());
    v.insert("valid_days", p.valid_days.to_string());
    v.insert("business", b.name.clone());
    v.insert("owner", b.owner.clone());
    v.insert("email", b.email.clone());
    v.insert("phone", b.phone.clone());
    v.insert("address", b.address.clone());
    v.insert("city", b.city.clone());
    v.insert("website", b.website.clone());
    v.insert("tax_office", b.tax_office.clone());
    v.insert("tax_id", b.tax_id.clone());
    v.insert("iban", b.iban.clone());
    v.insert("hourly_rate", try_format(kurus(b.hourly_rate)));
    v.insert("vat_rate", b.vat.to_string());
    v.insert("revisions", b.revisions.to_string());
    v.insert("items_table", items);
    v.insert("payment_table", plan);
    v.insert("deliverables", deliverables);
    v.insert("hours", fmt_hours(t.hours));
    v.insert("days", fmt_hours(t.days));
    v.insert("net", try_format(t.net));
    v.insert("vat", try_format(t.vat));
    v.insert("gross", try_format(t.gross));
    v.insert(
        "estimate_note",
        if t.estimated {
            if tr { "≈ işaretli süreler Kor'un anahtar kelime tablosundan gelen tahminlerdir; kesinleştirmek için gözden geçirin.".into() } else { "Hours marked ≈ are Kor's keyword-table estimates; review them before sending.".into() }
        } else {
            String::new()
        },
    );
    v.insert(
        "notes",
        p.notes.iter().fold(String::new(), |mut o, n| {
            let _ = writeln!(o, "- {n}");
            o
        }),
    );
    v
}

/// Render one document.
pub fn render(base: &Path, kind: &str, p: &Proposal, b: &Business) -> Result<String> {
    Ok(fill(&template(base, kind, p.lang)?, &vars(p, b)))
}

/// Revision directives from the client's reply, one per line:
/// `çıkar: <item>` / `remove: <item>`, `ekle: <item>[=hours]` / `add: …`, `<item> = <hours>`,
/// `not: <text>` / `note: <text>`. Other lines are ignored (they are conversation). Returns what changed.
pub fn revise(p: &mut Proposal, reply: &str) -> Vec<String> {
    let mut changes = Vec::new();
    let find = |items: &[Item], name: &str| {
        let n = super::tr::lower_tr(name.trim());
        items
            .iter()
            .position(|i| super::tr::lower_tr(&i.title).contains(&n))
    };
    for raw in reply.lines() {
        let line = raw.trim().trim_start_matches(['-', '*', '•']).trim();
        let lower = super::tr::lower_tr(line);
        // lower_tr maps one char to one char, so char counts line up (byte lengths may not).
        let rest = |prefixes: &[&str]| {
            prefixes.iter().find_map(|x| {
                lower.strip_prefix(x).map(|_| {
                    line.chars()
                        .skip(x.chars().count())
                        .collect::<String>()
                        .trim()
                        .to_owned()
                })
            })
        };
        if let Some(name) = rest(&["çıkar:", "cikar:", "remove:", "sil:"]) {
            if let Some(i) = find(&p.items, &name) {
                let it = p.items.remove(i);
                changes.push(format!("- {}", it.title));
            }
        } else if let Some(spec) = rest(&["ekle:", "add:"]) {
            let item = parse_item(&spec).unwrap_or_else(|_| Item {
                hours: estimate(&spec),
                estimated: true,
                title: spec.clone(),
                price: None,
            });
            changes.push(format!("+ {}", item.title));
            p.items.push(item);
        } else if let Some(note) = rest(&["not:", "note:"]) {
            changes.push(format!("note: {note}"));
            p.notes.push(note);
        } else if let Ok(it) = parse_item(line)
            && let Some(i) = find(&p.items, &it.title)
        {
            changes.push(format!(
                "{}: {} → {}",
                p.items[i].title,
                fmt_hours(p.items[i].hours),
                fmt_hours(it.hours)
            ));
            p.items[i].hours = it.hours;
            p.items[i].estimated = false;
        }
    }
    if !changes.is_empty() {
        p.version += 1;
    }
    changes
}

/// Generate the proposal (and with `contracts`, the contract, KVKK annex and NDA) into `dir`.
pub fn write(
    base: &Path,
    dir: &Path,
    p: &Proposal,
    b: &Business,
    contracts: bool,
    formats: &[String],
) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let suffix = if p.version > 1 {
        format!("-v{}", p.version)
    } else {
        String::new()
    };
    let kinds: &[&str] = if contracts { &KINDS } else { &["proposal"] };
    for kind in kinds {
        let md = render(base, kind, p, b)?;
        let title = format!("{} — {}", p.project, p.client);
        out.extend(super::docs::write_all(
            dir,
            &format!("{kind}{suffix}"),
            &md,
            &title,
            &b.accent,
            formats,
        )?);
    }
    let state = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    let sp = dir.join("proposal.json");
    std::fs::write(&sp, state).map_err(|e| e.to_string())?;
    out.push(sp);
    Ok(out)
}

pub fn load_state(dir: &Path) -> Result<Proposal> {
    let p = dir.join("proposal.json");
    let t = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn biz() -> Business {
        Business {
            name: "Acme Dijital".into(),
            hourly_rate: 1000.0,
            ..Business::default()
        }
    }

    #[test]
    fn scope_from_an_email() {
        let mail = "Merhaba,\n\n- Kurumsal site tasarımı (12 saat)\n- WooCommerce ödeme entegrasyonu\n- SEO\n\nSaygılar";
        let items = scope_from_text(mail);
        assert_eq!(items.len(), 3);
        assert_eq!((items[0].hours, items[0].estimated), (12.0, false));
        assert_eq!((items[1].hours, items[1].estimated), (16.0, true));
        let prose = scope_from_text(
            "Merhaba. Mevcut sitemizi yeni bir temaya taşımak istiyoruz. Teşekkürler.",
        );
        assert_eq!(prose.len(), 1);
        assert_eq!(prose[0].hours, 8.0);
    }

    #[test]
    fn totals_and_payment_plan_add_up() {
        let p = Proposal {
            client: "Ayşe".into(),
            project: "Site".into(),
            items: vec![
                parse_item("Tasarım=12").unwrap(),
                Item {
                    title: "Paket".into(),
                    hours: 0.0,
                    estimated: false,
                    price: Some(5000.0),
                },
            ],
            version: 1,
            ..Proposal::default()
        };
        let t = totals(&p, &biz());
        assert_eq!((t.net, t.vat, t.gross), (1_700_000, 340_000, 2_040_000));
        assert_eq!(t.plan.iter().map(|x| x.2).sum::<i64>(), t.gross);
        assert_eq!(t.days, 2.0);
    }

    #[test]
    fn templates_render_and_revise() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Proposal {
            client: "Ayşe Yılmaz".into(),
            project: "Mağaza".into(),
            items: scope_from_text("- Tasarım\n- SEO\n- Logo"),
            version: 1,
            date: "2026-11-10".into(),
            valid_days: 15,
            ..Proposal::default()
        };
        for lang in [Lang::Tr, Lang::En] {
            p.lang = lang;
            for k in KINDS {
                let md = render(dir.path(), k, &p, &biz()).unwrap();
                assert!(!md.contains("{{"), "{k} {lang:?}: unfilled placeholder");
                if k != "proposal" {
                    assert!(
                        md.contains("hukuki") || md.contains("legal advice"),
                        "{k} {lang:?}: notice"
                    );
                }
            }
        }
        assert!(templates_dir(dir.path()).join("contract.tr.md").is_file());
        p.lang = Lang::Tr;
        let ch = revise(
            &mut p,
            "Teşekkürler, birkaç değişiklik:\n- çıkar: logo\n- Tasarım = 20\n- ekle: Eğitim=2\nnot: Teslim Aralık sonu",
        );
        assert_eq!(ch.len(), 4, "{ch:?}");
        assert_eq!(p.version, 2);
        assert!(p.items.iter().all(|i| i.title != "Logo"));
        let out = write(
            dir.path(),
            &dir.path().join("out"),
            &p,
            &biz(),
            true,
            &["html".into(), "pdf".into(), "docx".into()],
        )
        .unwrap();
        assert!(out.iter().any(|f| f.ends_with("contract-v2.pdf")));
        assert_eq!(load_state(&dir.path().join("out")).unwrap(), p);
    }
}
