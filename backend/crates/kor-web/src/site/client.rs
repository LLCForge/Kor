//! Client workspaces and the monthly report (§29.13).
//!
//! `clients/<id>/` holds `client.toml` (sites, servers, contact), `log.jsonl` (work done: every
//! `kor host`/`kor wp`/redirect/report run of the client's servers and sites is logged with its
//! duration, plus manual entries), `tickets.json`, `uptime.jsonl` (samples from `kor client check`,
//! which a routine or `kor daemon` can run on a schedule) and `seo/` (Markdown highlights the SEO
//! module or the user writes there). `kor client report <id> --month 2026-11` turns that into a
//! Markdown/HTML/PDF report, a Telegram message draft, an email draft (.eml) and a time sheet CSV
//! ready for invoicing. Drafts are files; Kor sends nothing.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};

use super::http::Http;
use super::{Result, check_name};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Client {
    pub name: String,
    pub email: String,
    /// Telegram chat or user name, for the message draft.
    pub telegram: String,
    /// WordPress site profiles (`kor wp`).
    pub sites: Vec<String>,
    /// Host profiles (`kor host`).
    pub hosts: Vec<String>,
    /// Other sites to watch (`https://…`).
    pub urls: Vec<String>,
    /// Hourly rate for this client (lira, without KDV); `business.toml` when 0.
    pub rate: f64,
}

pub fn dir(base: &Path, id: &str) -> PathBuf {
    base.join("clients").join(id)
}

pub fn save(base: &Path, id: &str, c: &Client) -> Result<PathBuf> {
    check_name(id)?;
    for u in &c.urls {
        let p = url::Url::parse(u).map_err(|e| format!("{u}: {e}"))?;
        if !matches!(p.scheme(), "http" | "https") {
            return Err(format!("{u}: only http(s)"));
        }
    }
    for n in c.sites.iter().chain(&c.hosts) {
        check_name(n)?;
    }
    let p = dir(base, id).join("client.toml");
    super::save_toml(&p, c)?;
    Ok(p)
}

pub fn load(base: &Path, id: &str) -> Result<Client> {
    check_name(id)?;
    let p = dir(base, id).join("client.toml");
    if !p.is_file() {
        return Err(format!("no client `{id}` (kor client add {id} …)"));
    }
    super::load_toml(&p)
}

pub fn list(base: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(base.join("clients"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("client.toml").is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| super::valid_name(n))
        .collect();
    v.sort();
    v
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub ts: i64,
    /// `host-<task>`, `wp-update`, `wp-integrity`, `wp-cleanup`, `doctor`, `redirects`, `tr-check`, `manual`…
    pub kind: String,
    pub target: String,
    pub summary: String,
    pub secs: u64,
    /// `auto` (a Kor run) or `manual`.
    pub source: String,
}

fn append(path: &Path, line: &str) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(f, "{line}").map_err(|e| e.to_string())
}

/// Append to the client's work log.
pub fn log(base: &Path, id: &str, e: &Entry) -> Result<()> {
    check_name(id)?;
    append(&dir(base, id).join("log.jsonl"), &serde_json::to_string(e).map_err(|x| x.to_string())?)
}

/// Log a Kor run when the profile belongs to a client (no-op otherwise).
pub fn log_auto(base: &Path, client: Option<&str>, kind: &str, target: &str, summary: &str, secs: u64) {
    if let Some(c) = client {
        let _ = log(
            base,
            c,
            &Entry { ts: super::now(), kind: kind.into(), target: target.into(), summary: summary.into(), secs, source: "auto".into() },
        );
    }
}

fn read_jsonl<T: serde::de::DeserializeOwned>(p: &Path) -> Vec<T> {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn entries(base: &Path, id: &str) -> Vec<Entry> {
    read_jsonl(&dir(base, id).join("log.jsonl"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ticket {
    pub id: u32,
    pub ts: i64,
    pub title: String,
    pub open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<i64>,
}

pub fn tickets(base: &Path, id: &str) -> Vec<Ticket> {
    std::fs::read_to_string(dir(base, id).join("tickets.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_tickets(base: &Path, id: &str, t: &[Ticket]) -> Result<()> {
    super::write_file(&dir(base, id).join("tickets.json"), &serde_json::to_string_pretty(t).map_err(|e| e.to_string())?)
}

pub fn ticket_add(base: &Path, id: &str, title: &str) -> Result<Ticket> {
    load(base, id)?;
    if title.trim().is_empty() || title.len() > 500 {
        return Err("ticket title: 1–500 characters".into());
    }
    let mut all = tickets(base, id);
    let t = Ticket { id: all.iter().map(|t| t.id).max().unwrap_or(0) + 1, ts: super::now(), title: title.trim().into(), open: true, closed: None };
    all.push(t.clone());
    save_tickets(base, id, &all)?;
    Ok(t)
}

pub fn ticket_close(base: &Path, id: &str, n: u32) -> Result<bool> {
    let mut all = tickets(base, id);
    let Some(t) = all.iter_mut().find(|t| t.id == n && t.open) else { return Ok(false) };
    t.open = false;
    t.closed = Some(super::now());
    save_tickets(base, id, &all)?;
    Ok(true)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub ts: i64,
    pub url: String,
    pub status: u16,
    pub ms: u64,
    pub up: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssl_days: Option<i64>,
}

/// Every URL of the client: WordPress sites, plus `urls`.
pub fn urls(base: &Path, c: &Client) -> Vec<String> {
    let mut v: Vec<String> = c.sites.iter().filter_map(|s| super::wp::load(base, s).ok().map(|w| w.url)).collect();
    v.extend(c.urls.iter().cloned());
    v.sort();
    v.dedup();
    v
}

/// One uptime/SSL sample per URL, appended to `uptime.jsonl`.
pub async fn check(base: &Path, id: &str, http: &Http) -> Result<Vec<Sample>> {
    let c = load(base, id)?;
    let mut out = Vec::new();
    for u in urls(base, &c) {
        let host = super::http::host_of(&u);
        let (status, ms, up) = match http.follow(&u).await {
            Ok((_, r)) => (r.status, r.ms, r.status < 500 && !super::host::fatal_text(&r.body)),
            Err(_) => (0, 0, false),
        };
        let ssl_days = if u.starts_with("https://") {
            super::tls::probe(&host, 443).await.ok().map(|c| c.days_left(super::now()))
        } else {
            None
        };
        let s = Sample { ts: super::now(), url: u, status, ms, up, ssl_days };
        append(&dir(base, id).join("uptime.jsonl"), &serde_json::to_string(&s).map_err(|e| e.to_string())?)?;
        out.push(s);
    }
    Ok(out)
}

/// `2026-11` → [start, end) Unix seconds.
pub fn month_range(month: &str) -> Result<(i64, i64)> {
    let start = chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").map_err(|_| format!("month `{month}`: use YYYY-MM"))?;
    let end = start.checked_add_months(chrono::Months::new(1)).ok_or("month out of range")?;
    let ts = |d: chrono::NaiveDate| d.and_hms_opt(0, 0, 0).map_or(0, |t| t.and_utc().timestamp());
    Ok((ts(start), ts(end)))
}

fn day(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

fn hours(secs: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let h = secs as f64 / 3600.0;
    (h * 100.0).round() / 100.0
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub markdown: String,
    pub telegram: String,
    pub email: String,
    pub timesheet: String,
}

/// Build the report from stored data plus `live` samples taken now.
#[allow(clippy::too_many_lines)]
pub fn report(l: &Localizer, base: &Path, id: &str, month: &str, live: &[Sample], rate_default: f64) -> Result<Report> {
    let c = load(base, id)?;
    let (from, to) = month_range(month)?;
    let in_month = |ts: i64| ts >= from && ts < to;
    let work: Vec<Entry> = entries(base, id).into_iter().filter(|e| in_month(e.ts)).collect();
    let samples: Vec<Sample> = read_jsonl::<Sample>(&dir(base, id).join("uptime.jsonl")).into_iter().filter(|s| in_month(s.ts)).collect();
    let tickets = tickets(base, id);
    let rate = if c.rate > 0.0 { c.rate } else { rate_default };
    let name = if c.name.is_empty() { id.to_owned() } else { c.name.clone() };
    let mut md = format!("# {}\n\n", l.t_args("client-report-title", &args!("client" => name.clone(), "month" => month.to_owned())));

    // Work done.
    let _ = writeln!(md, "## {}\n", l.t("client-report-work"));
    if work.is_empty() {
        let _ = writeln!(md, "{}\n", l.t("client-report-none"));
    } else {
        let _ = writeln!(md, "| {} | {} | {} | {} |\n|---|---|---|---|", l.t("client-col-date"), l.t("client-col-work"), l.t("client-col-target"), l.t("client-col-hours"));
        for e in &work {
            let _ = writeln!(md, "| {} | {} | {} | {} |", day(e.ts), super::cell(&e.summary), super::cell(&e.target), hours(e.secs));
        }
        md.push('\n');
    }

    // Uptime and SSL.
    let _ = writeln!(md, "## {}\n", l.t("client-report-uptime"));
    let mut url_list: Vec<String> = samples.iter().map(|s| s.url.clone()).chain(live.iter().map(|s| s.url.clone())).collect();
    url_list.sort();
    url_list.dedup();
    if url_list.is_empty() {
        let _ = writeln!(md, "{}\n", l.t("client-report-no-sites"));
    } else {
        let _ = writeln!(md, "| {} | {} | {} | {} |\n|---|---|---|---|", l.t("client-col-site"), l.t("client-col-uptime"), l.t("client-col-now"), l.t("client-col-ssl"));
        for u in &url_list {
            let s: Vec<&Sample> = samples.iter().filter(|s| &s.url == u).collect();
            #[allow(clippy::cast_precision_loss)]
            let pct = if s.is_empty() { "—".to_owned() } else { format!("%{:.2} ({} {})", s.iter().filter(|x| x.up).count() as f64 * 100.0 / s.len() as f64, s.len(), l.t("client-samples")) };
            let now = live.iter().find(|x| &x.url == u);
            let now_txt = now.map_or_else(|| "—".to_owned(), |x| if x.up { format!("HTTP {} · {} ms", x.status, x.ms) } else { format!("{} (HTTP {})", l.t("client-down"), x.status) });
            let ssl = now.and_then(|x| x.ssl_days).or_else(|| s.last().and_then(|x| x.ssl_days));
            let _ = writeln!(md, "| {} | {pct} | {now_txt} | {} |", super::cell(u), ssl.map_or_else(|| "—".into(), |d| l.t_args("web-days-left", &args!("days" => d))));
        }
        md.push('\n');
    }

    // Updates and security.
    let updates: Vec<&Entry> = work.iter().filter(|e| e.kind.starts_with("wp-update") || e.kind == "host-php").collect();
    let security: Vec<&Entry> = work.iter().filter(|e| matches!(e.kind.as_str(), "wp-integrity" | "wp-cleanup" | "doctor" | "tr-check")).collect();
    for (title, list) in [("client-report-updates", &updates), ("client-report-security", &security)] {
        let _ = writeln!(md, "## {}\n", l.t(title));
        if list.is_empty() {
            let _ = writeln!(md, "{}\n", l.t("client-report-none"));
        } else {
            for e in list {
                let _ = writeln!(md, "- {} — {}: {}", day(e.ts), e.target, e.summary);
            }
            md.push('\n');
        }
    }

    // SEO highlights written into seo/ during the month.
    let _ = writeln!(md, "## {}\n", l.t("client-report-seo"));
    let mut seo = Vec::new();
    for e in std::fs::read_dir(dir(base, id).join("seo")).into_iter().flatten().flatten() {
        let p = e.path();
        let modified = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        if p.extension().is_some_and(|x| x == "md") && in_month(modified)
            && let Ok(t) = std::fs::read_to_string(&p)
        {
            // Their headings go two levels down, under this section.
            seo.push(t.lines().map(|l| if l.starts_with('#') { format!("##{l}") } else { l.to_owned() }).collect::<Vec<_>>().join("\n"));
        }
    }
    if seo.is_empty() {
        let _ = writeln!(md, "{}\n", l.t("client-report-no-seo"));
    } else {
        for s in seo {
            let _ = writeln!(md, "{}\n", s.trim());
        }
    }

    // Tickets.
    let _ = writeln!(md, "## {}\n", l.t("client-report-tickets"));
    let opened = tickets.iter().filter(|t| in_month(t.ts)).count();
    let closed = tickets.iter().filter(|t| t.closed.is_some_and(in_month)).count();
    let open: Vec<&Ticket> = tickets.iter().filter(|t| t.open).collect();
    let _ = writeln!(md, "{}\n", l.t_args("client-report-ticket-counts", &args!("opened" => opened, "closed" => closed, "open" => open.len())));
    for t in &open {
        let _ = writeln!(md, "- #{} {}", t.id, t.title);
    }
    if !open.is_empty() {
        md.push('\n');
    }

    // Recommendations from the data above.
    let mut recs = Vec::new();
    for s in live {
        if !s.up {
            recs.push(l.t_args("client-rec-down", &args!("url" => s.url.clone())));
        }
        if let Some(d) = s.ssl_days.filter(|d| *d < 30) {
            recs.push(l.t_args("client-rec-ssl", &args!("url" => s.url.clone(), "days" => d)));
        }
    }
    if !open.is_empty() {
        recs.push(l.t_args("client-rec-tickets", &args!("count" => open.len())));
    }
    if updates.is_empty() && !c.sites.is_empty() {
        recs.push(l.t("client-rec-updates"));
    }
    let _ = writeln!(md, "## {}\n", l.t("client-report-recs"));
    if recs.is_empty() {
        let _ = writeln!(md, "{}\n", l.t("client-report-none"));
    }
    for r in &recs {
        let _ = writeln!(md, "- {r}");
    }

    // Time sheet.
    let total: u64 = work.iter().map(|e| e.secs).sum();
    #[allow(clippy::cast_possible_truncation)]
    let amount = (hours(total) * rate * 100.0).round() as i64;
    let _ = writeln!(
        md,
        "\n## {}\n\n{}\n",
        l.t("client-report-time"),
        l.t_args("client-report-time-total", &args!("hours" => hours(total), "amount" => super::tr::try_format(amount)))
    );
    let mut csv = "date,work,target,hours,rate,amount\n".to_owned();
    for e in &work {
        let h = hours(e.secs);
        let _ = writeln!(csv, "{},{},{},{h},{rate},{:.2}", day(e.ts), e.summary.replace([',', '\n'], ";"), e.target.replace(',', ";"), h * rate);
    }

    let up_sites = live.iter().filter(|s| s.up).count();
    let telegram = l.t_args(
        "client-telegram",
        &args!("client" => name.clone(), "month" => month.to_owned(), "tasks" => work.len(), "hours" => hours(total),
               "up" => up_sites, "sites" => live.len(), "tickets" => open.len()),
    );
    let subject = l.t_args("client-report-title", &args!("client" => name.clone(), "month" => month.to_owned()));
    let email = format!(
        "To: {}\nSubject: {}\nMIME-Version: 1.0\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: 8bit\nX-Unsent: 1\n\n{}\n",
        c.email.replace(['\r', '\n'], ""),
        subject.replace(['\r', '\n'], " "),
        md
    );
    Ok(Report { markdown: md, telegram, email, timesheet: csv })
}

/// Write report files into `clients/<id>/reports/<month>/`.
pub fn write_report(base: &Path, id: &str, month: &str, r: &Report, accent: &str, formats: &[String]) -> Result<Vec<PathBuf>> {
    let out_dir = dir(base, id).join("reports").join(month);
    let title = r.markdown.lines().next().unwrap_or("").trim_start_matches("# ").to_owned();
    let mut files = super::docs::write_all(&out_dir, "report", &r.markdown, &title, accent, formats)?;
    for (name, text) in [("telegram.txt", &r.telegram), ("email.eml", &r.email), ("timesheet.csv", &r.timesheet)] {
        let p = out_dir.join(name);
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        files.push(p);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_from_logged_work() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        save(base, "acme", &Client { name: "Acme".into(), email: "info@acme.com".into(), urls: vec!["https://acme.com".into()], ..Client::default() }).unwrap();
        // The SEO note's file time is "now", so the report month is the current one.
        let month = chrono::Utc::now().format("%Y-%m").to_string();
        let (from, _) = month_range(&month).unwrap();
        log(base, "acme", &Entry { ts: from + 3600, kind: "wp-update".into(), target: "acme.com".into(), summary: "3 eklenti güncellendi".into(), secs: 5400, source: "auto".into() }).unwrap();
        log(base, "acme", &Entry { ts: from - 10, kind: "manual".into(), target: "x".into(), summary: "eski ay".into(), secs: 3600, source: "manual".into() }).unwrap();
        append(&super::dir(base, "acme").join("uptime.jsonl"), &serde_json::to_string(&Sample { ts: from + 60, url: "https://acme.com".into(), status: 200, ms: 120, up: true, ssl_days: Some(80) }).unwrap()).unwrap();
        append(&super::dir(base, "acme").join("uptime.jsonl"), &serde_json::to_string(&Sample { ts: from + 120, url: "https://acme.com".into(), status: 503, ms: 0, up: false, ssl_days: None }).unwrap()).unwrap();
        let t = ticket_add(base, "acme", "Logo değişsin").unwrap();
        assert_eq!(t.id, 1);
        std::fs::create_dir_all(super::dir(base, "acme").join("seo")).unwrap();
        std::fs::write(super::dir(base, "acme").join("seo/kasim.md"), "# Öne çıkanlar\n\n- 12 sayfa ilk 10'da").unwrap();
        let live = [Sample { ts: 0, url: "https://acme.com".into(), status: 200, ms: 90, up: true, ssl_days: Some(12) }];
        let l = Localizer::new(kor_i18n::Lang::Tr);
        let r = report(&l, base, "acme", &month, &live, 1000.0).unwrap();
        assert!(r.markdown.contains("3 eklenti güncellendi") && !r.markdown.contains("eski ay"));
        assert!(r.markdown.contains("%50.00"), "{}", r.markdown);
        assert!(r.markdown.contains("12 sayfa ilk 10'da"));
        assert!(r.markdown.contains("Logo değişsin"));
        assert!(r.markdown.contains("1.500,00 ₺"), "{}", r.markdown);
        assert!(r.email.starts_with("To: info@acme.com\nSubject: "));
        assert!(r.timesheet.lines().nth(1).unwrap().contains(",1.5,1000,1500.00"));
        let files = write_report(base, "acme", &month, &r, "#c96442", &["html".into(), "pdf".into()]).unwrap();
        assert_eq!(files.len(), 6);
        assert!(ticket_close(base, "acme", 1).unwrap() && !ticket_close(base, "acme", 1).unwrap());
    }

    #[test]
    fn months() {
        let (a, b) = month_range("2026-12").unwrap();
        assert_eq!(b - a, 31 * 86_400);
        assert!(month_range("2026-13").is_err());
    }
}
