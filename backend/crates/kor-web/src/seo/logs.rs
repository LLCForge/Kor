//! Log-file SEO (spec §29.9): Apache/nginx combined (or common) logs and JSON lines (nginx JSON,
//! Cloudflare Logpush field names). Bots are recognised by user agent; search-engine bots are
//! verified by reverse DNS + forward confirmation (Google's documented method) only when the
//! caller allows network use, otherwise they are reported as unverified. Findings carry the log
//! file and the first line where each problem shows.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use kor_runtime::analysis::{Finding, Report, Severity};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

/// `(name, user agent token, reverse DNS suffixes; empty = cannot be verified by DNS)`.
pub const BOTS: &[(&str, &str, &[&str])] = &[
    (
        "Googlebot",
        "googlebot",
        &["googlebot.com", "google.com", "googleusercontent.com"],
    ),
    (
        "Google-Other",
        "google-inspectiontool",
        &["googlebot.com", "google.com"],
    ),
    ("Bingbot", "bingbot", &["search.msn.com"]),
    (
        "YandexBot",
        "yandexbot",
        &["yandex.ru", "yandex.net", "yandex.com"],
    ),
    ("Baiduspider", "baiduspider", &["baidu.com", "baidu.jp"]),
    ("Applebot", "applebot", &["applebot.apple.com"]),
    ("DuckDuckBot", "duckduckbot", &[]),
    ("AhrefsBot", "ahrefsbot", &[]),
    ("SemrushBot", "semrushbot", &[]),
    ("MJ12bot", "mj12bot", &[]),
    ("DotBot", "dotbot", &[]),
    ("GPTBot", "gptbot", &[]),
    ("ClaudeBot", "claudebot", &[]),
    ("PerplexityBot", "perplexitybot", &[]),
    ("CCBot", "ccbot", &[]),
];

/// Distinct IPs verified per run (DNS is slow; the rest stay unverified).
pub const MAX_VERIFY: usize = 500;

/// Reverse + forward DNS (injectable for tests).
pub trait Dns {
    fn reverse(&self, ip: IpAddr) -> Option<String>;
    fn forward(&self, host: &str) -> Vec<IpAddr>;
}

/// The system resolver (`getnameinfo` / `getaddrinfo`).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemDns;

impl Dns for SystemDns {
    fn reverse(&self, ip: IpAddr) -> Option<String> {
        dns_lookup::lookup_addr(&ip).ok()
    }
    fn forward(&self, host: &str) -> Vec<IpAddr> {
        dns_lookup::lookup_host(host).unwrap_or_default()
    }
}

/// One request.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub ip: String,
    pub time: Option<DateTime<FixedOffset>>,
    pub method: String,
    /// Path + query.
    pub path: String,
    pub status: u16,
    pub bytes: u64,
    pub ua: String,
    /// Seconds.
    pub duration: Option<f64>,
}

fn combined_re() -> Regex {
    Regex::new(r#"^(\S+) \S+ \S+ \[([^\]]+)\] "(\S+) (\S+)[^"]*" (\d{3}) (\S+)(?: "[^"]*" "([^"]*)")?(?: (\d+(?:\.\d+)?))?"#).expect("static regex")
}

fn json_str<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str))
}

fn json_num(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        v.get(*k).and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
    })
}

fn json_time(v: &Value) -> Option<DateTime<FixedOffset>> {
    for k in [
        "time",
        "timestamp",
        "time_iso8601",
        "@timestamp",
        "EdgeStartTimestamp",
        "time_local",
    ] {
        match v.get(k) {
            Some(Value::String(s)) => {
                if let Ok(t) = DateTime::parse_from_rfc3339(s)
                    .or_else(|_| DateTime::parse_from_str(s, "%d/%b/%Y:%H:%M:%S %z"))
                {
                    return Some(t);
                }
            }
            Some(Value::Number(n)) => {
                let n = n.as_i64()?;
                // Seconds, milliseconds or nanoseconds since the epoch.
                let secs = if n > 1_000_000_000_000_000 {
                    n / 1_000_000_000
                } else if n > 1_000_000_000_000 {
                    n / 1000
                } else {
                    n
                };
                return DateTime::from_timestamp(secs, 0).map(|t| t.fixed_offset());
            }
            _ => {}
        }
    }
    None
}

/// Parse one line (combined/common or JSON). `None` for lines that are neither.
pub fn parse_line(line: &str, re: &Regex) -> Option<Hit> {
    let t = line.trim();
    if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).ok()?;
        let (mut method, mut path) = (
            json_str(&v, &["method", "request_method", "ClientRequestMethod"])
                .unwrap_or("")
                .to_owned(),
            json_str(
                &v,
                &["uri", "request_uri", "path", "url", "ClientRequestURI"],
            )
            .unwrap_or("")
            .to_owned(),
        );
        if path.is_empty()
            && let Some(req) = json_str(&v, &["request"])
        {
            let mut p = req.split_whitespace();
            method = p.next().unwrap_or("").to_owned();
            path = p.next().unwrap_or("").to_owned();
        }
        let status = json_num(&v, &["status", "EdgeResponseStatus", "response_status"])?;
        let dur = json_num(&v, &["request_time", "duration"])
            .or_else(|| json_num(&v, &["OriginResponseDurationMs"]).map(|ms| ms / 1000.0));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Some(Hit {
            ip: json_str(&v, &["ip", "remote_addr", "client_ip", "ClientIP"])
                .unwrap_or("")
                .to_owned(),
            time: json_time(&v),
            method,
            path,
            status: status as u16,
            bytes: json_num(
                &v,
                &[
                    "bytes",
                    "body_bytes_sent",
                    "bytes_sent",
                    "EdgeResponseBytes",
                ],
            )
            .unwrap_or(0.0) as u64,
            ua: json_str(
                &v,
                &[
                    "ua",
                    "user_agent",
                    "http_user_agent",
                    "ClientRequestUserAgent",
                ],
            )
            .unwrap_or("")
            .to_owned(),
            duration: dur,
        });
    }
    let c = re.captures(t)?;
    Some(Hit {
        ip: c[1].to_owned(),
        time: DateTime::parse_from_str(&c[2], "%d/%b/%Y:%H:%M:%S %z").ok(),
        method: c[3].to_owned(),
        path: c[4].to_owned(),
        status: c[5].parse().ok()?,
        bytes: c[6].parse().unwrap_or(0),
        ua: c.get(7).map_or("", |m| m.as_str()).to_owned(),
        duration: c.get(8).and_then(|m| m.as_str().parse().ok()),
    })
}

pub fn bot_of(ua: &str) -> Option<usize> {
    let l = ua.to_ascii_lowercase();
    BOTS.iter().position(|(_, tok, _)| l.contains(tok))
}

/// Verification result of a claimed bot IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Verified,
    /// DNS did not confirm the claim: a fake bot.
    Fake,
    /// Not checked (network not allowed, limit reached, or the bot has no DNS method).
    Unverified,
}

/// Reverse lookup must end in a bot domain, and the name must resolve back to the same IP.
pub fn verify(dns: &dyn Dns, ip: IpAddr, suffixes: &[&str]) -> Verdict {
    let Some(host) = dns.reverse(ip) else {
        return Verdict::Fake;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let ok_name = suffixes
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")));
    if ok_name && dns.forward(&host).contains(&ip) {
        Verdict::Verified
    } else {
        Verdict::Fake
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BotStats {
    pub hits: u64,
    pub verified: u64,
    pub fake: u64,
    pub unverified: u64,
    pub status: BTreeMap<u16, u64>,
}

/// Everything the analysis found.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LogStats {
    pub lines: u64,
    pub parsed: u64,
    pub first: Option<String>,
    pub last: Option<String>,
    pub status: BTreeMap<u16, u64>,
    pub bots: BTreeMap<String, BotStats>,
    /// Googlebot (verified or unverified, never fake) hits per day `YYYY-MM-DD`.
    pub googlebot_per_day: BTreeMap<String, u64>,
    /// Share of Googlebot hits on parameter URLs, redirects, 4xx, 5xx (percent).
    pub waste: BTreeMap<String, f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub never_crawled: Vec<String>,
    pub not_in_sitemap: Vec<String>,
    pub dropping: Vec<(String, u64, u64)>,
    pub fake_ips: Vec<String>,
    pub dns_checked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Sitemap URLs (full); compared by path + query.
    pub sitemap: Vec<String>,
    /// Network allowed for reverse DNS (and `kor_core::net` open).
    pub verify_dns: bool,
}

fn pct(n: u64, d: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    if d == 0 {
        0.0
    } else {
        (n as f64 * 1000.0 / d as f64).round() / 10.0
    }
}

/// Analyse one log (`file` names it in findings).
#[allow(clippy::too_many_lines)] // one pass, then the derived figures
pub fn analyze(
    file: &str,
    text: &str,
    opts: &Options,
    dns: Option<&dyn Dns>,
) -> (LogStats, Report) {
    let re = combined_re();
    let mut s = LogStats {
        dns_checked: dns.is_some() && opts.verify_dns,
        ..LogStats::default()
    };
    let mut verdicts: HashMap<String, Verdict> = HashMap::new();
    let mut google_paths: HashMap<String, (u64, u16, u64)> = HashMap::new(); // path → (hits, last status, first line)
    let mut google_times: Vec<(DateTime<FixedOffset>, String)> = Vec::new();
    let mut durations = Vec::new();
    let mut first_line: HashMap<&'static str, u64> = HashMap::new();
    let mut fake_first: BTreeMap<String, u64> = BTreeMap::new();
    let (mut g_total, mut g_param, mut g_3xx, mut g_4xx, mut g_5xx) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut min_t: Option<DateTime<FixedOffset>> = None;
    let mut max_t: Option<DateTime<FixedOffset>> = None;
    for (n, line) in text.lines().enumerate() {
        let ln = n as u64 + 1;
        s.lines += 1;
        let Some(h) = parse_line(line, &re) else {
            continue;
        };
        s.parsed += 1;
        *s.status.entry(h.status).or_default() += 1;
        if let Some(t) = h.time {
            min_t = Some(min_t.map_or(t, |m| m.min(t)));
            max_t = Some(max_t.map_or(t, |m| m.max(t)));
        }
        if let Some(d) = h.duration {
            durations.push(d * 1000.0);
        }
        let Some(bi) = bot_of(&h.ua) else { continue };
        let (name, _, suffixes) = BOTS[bi];
        let verdict = if suffixes.is_empty() || !s.dns_checked {
            Verdict::Unverified
        } else if let Some(v) = verdicts.get(&h.ip) {
            *v
        } else if verdicts.len() >= MAX_VERIFY {
            Verdict::Unverified
        } else {
            let v = h.ip.parse::<IpAddr>().map_or(Verdict::Fake, |ip| {
                verify(dns.expect("checked"), ip, suffixes)
            });
            verdicts.insert(h.ip.clone(), v);
            v
        };
        let b = s.bots.entry(name.to_owned()).or_default();
        b.hits += 1;
        *b.status.entry(h.status).or_default() += 1;
        match verdict {
            Verdict::Verified => b.verified += 1,
            Verdict::Fake => {
                b.fake += 1;
                fake_first.entry(h.ip.clone()).or_insert(ln);
                continue;
            }
            Verdict::Unverified => b.unverified += 1,
        }
        if name != "Googlebot" {
            continue;
        }
        g_total += 1;
        if h.path.contains('?') {
            g_param += 1;
            first_line.entry("param").or_insert(ln);
        }
        match h.status {
            300..=399 => {
                g_3xx += 1;
                first_line.entry("3xx").or_insert(ln);
            }
            400..=499 => {
                g_4xx += 1;
                first_line.entry("4xx").or_insert(ln);
            }
            500..=599 => {
                g_5xx += 1;
                first_line.entry("5xx").or_insert(ln);
            }
            _ => {}
        }
        let e = google_paths
            .entry(h.path.clone())
            .or_insert((0, h.status, ln));
        e.0 += 1;
        e.1 = h.status;
        if let Some(t) = h.time {
            *s.googlebot_per_day
                .entry(t.format("%Y-%m-%d").to_string())
                .or_default() += 1;
            google_times.push((t, h.path.clone()));
        }
    }
    s.first = min_t.map(|t| t.to_rfc3339());
    s.last = max_t.map(|t| t.to_rfc3339());
    s.waste = BTreeMap::from([
        ("parameter_urls".into(), pct(g_param, g_total)),
        ("redirects".into(), pct(g_3xx, g_total)),
        ("client_errors".into(), pct(g_4xx, g_total)),
        ("server_errors".into(), pct(g_5xx, g_total)),
    ]);
    if !durations.is_empty() {
        durations.sort_by(f64::total_cmp);
        let at = |q: f64| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let i = ((durations.len() - 1) as f64 * q).round() as usize;
            durations[i]
        };
        s.p50_ms = Some(at(0.5));
        s.p95_ms = Some(at(0.95));
    }
    // Crawl frequency drop: first half vs second half of the time range.
    if let (Some(a), Some(b)) = (min_t, max_t)
        && b > a
    {
        let mid = a + (b - a) / 2;
        let mut halves: HashMap<&str, (u64, u64)> = HashMap::new();
        for (t, p) in &google_times {
            let e = halves.entry(p.as_str()).or_default();
            if *t < mid { e.0 += 1 } else { e.1 += 1 }
        }
        let mut d: Vec<(String, u64, u64)> = halves
            .into_iter()
            .filter(|(_, (x, y))| *x >= 3 && *y * 3 <= *x)
            .map(|(p, (x, y))| (p.to_owned(), x, y))
            .collect();
        d.sort_by(|x, y| (y.1 - y.2).cmp(&(x.1 - x.2)).then_with(|| x.0.cmp(&y.0)));
        d.truncate(50);
        s.dropping = d;
    }
    if !opts.sitemap.is_empty() {
        let sm: HashSet<String> = opts
            .sitemap
            .iter()
            .filter_map(|u| url::Url::parse(u).ok())
            .map(|u| match u.query() {
                Some(q) => format!("{}?{q}", u.path()),
                None => u.path().to_owned(),
            })
            .collect();
        let mut never: Vec<String> = sm
            .iter()
            .filter(|p| !google_paths.contains_key(*p))
            .cloned()
            .collect();
        never.sort();
        s.never_crawled = never;
        let mut extra: Vec<String> = google_paths
            .iter()
            .filter(|(p, (_, st, _))| *st == 200 && !sm.contains(*p) && !p.contains('?'))
            .map(|(p, _)| p.clone())
            .collect();
        extra.sort();
        s.not_in_sitemap = extra;
    }
    s.fake_ips = fake_first.keys().cloned().collect();

    // Findings.
    let mut r = Report::new("seo-logs");
    let mut push = |rule: &str, sev: Severity, line: u64, msg: String, fix: Option<&str>| {
        r.findings.push(Finding {
            rule: format!("seo.log.{rule}"),
            source: "kor".into(),
            severity: sev,
            file: PathBuf::from(file),
            line: u32::try_from(line.max(1)).unwrap_or(u32::MAX),
            message: msg,
            fix: fix.map(str::to_owned),
            ..Finding::default()
        });
    };
    for (ip, ln) in &fake_first {
        push(
            "fake-bot",
            Severity::High,
            *ln,
            format!("{ip} claims to be a search bot but reverse DNS does not confirm it"),
            Some("Block or rate-limit this IP; it is not a search engine"),
        );
    }
    let waste = [
        (
            "param",
            "crawl-waste-params",
            g_param,
            Severity::Medium,
            "Googlebot spends {p}% of hits on parameter URLs",
            "Canonicalize or disallow parameter variants; stop linking them",
        ),
        (
            "3xx",
            "crawl-waste-redirects",
            g_3xx,
            Severity::Low,
            "{p}% of Googlebot hits get redirects",
            "Update internal links and sitemap to the final URLs",
        ),
        (
            "4xx",
            "crawl-waste-errors",
            g_4xx,
            Severity::Medium,
            "{p}% of Googlebot hits get 4xx",
            "Fix or 301 the missing URLs; remove links to them",
        ),
        (
            "5xx",
            "server-errors",
            g_5xx,
            Severity::High,
            "{p}% of Googlebot hits get 5xx",
            "Check server errors and capacity: 5xx slows crawling",
        ),
    ];
    for (k, rule, n, sev, msg, fix) in waste {
        let p = pct(n, g_total);
        if n > 0 && (p >= 5.0 || k == "5xx") {
            push(
                rule,
                sev,
                first_line.get(k).copied().unwrap_or(1),
                msg.replace("{p}", &format!("{p}")),
                Some(fix),
            );
        }
    }
    let mut errs: Vec<(&String, &(u64, u16, u64))> = google_paths
        .iter()
        .filter(|(_, (_, st, _))| *st >= 400)
        .collect();
    errs.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(b.0)));
    for (p, (n, st, ln)) in errs.into_iter().take(20) {
        push(
            "error-url",
            if *st >= 500 {
                Severity::High
            } else {
                Severity::Low
            },
            *ln,
            format!("Googlebot got HTTP {st} for {p} ({n} hits)"),
            None,
        );
    }
    if let Some(p95) = s.p95_ms.filter(|p| *p > 1_000.0) {
        push(
            "slow",
            Severity::Medium,
            1,
            format!("p95 response time {p95:.0} ms (over 1000 ms)"),
            Some("Cache pages and check slow endpoints; slow answers lower the crawl rate"),
        );
    }
    for (p, a, b) in s.dropping.iter().take(20) {
        push(
            "crawl-drop",
            Severity::Low,
            google_paths.get(p).map_or(1, |x| x.2),
            format!(
                "Googlebot crawled {p} {a}× in the first half of the period, {b}× in the second"
            ),
            None,
        );
    }
    for p in s.never_crawled.iter().take(50) {
        push(
            "never-crawled",
            Severity::Medium,
            1,
            format!("Sitemap URL {p} was never crawled by Googlebot in this log"),
            Some("Link it internally and check it is indexable"),
        );
    }
    for p in s.not_in_sitemap.iter().take(50) {
        push(
            "not-in-sitemap",
            Severity::Low,
            google_paths.get(p).map_or(1, |x| x.2),
            format!("Googlebot crawls {p} (200) but it is not in the sitemap"),
            Some("Add it to the sitemap, or noindex/redirect it if it should not rank"),
        );
    }
    r.notes.push(format!(
        "{} of {} lines parsed; {} Googlebot hits",
        s.parsed, s.lines, g_total
    ));
    if !s.dns_checked {
        r.notes.push("bots identified by user agent only (unverified): reverse DNS needs --verify-dns and network access".into());
    }
    r.normalize();
    (s, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeDns;
    impl Dns for FakeDns {
        fn reverse(&self, ip: IpAddr) -> Option<String> {
            match ip.to_string().as_str() {
                "66.249.66.1" => Some("crawl-66-249-66-1.googlebot.com.".into()),
                "1.2.3.4" => Some("evil.example.net".into()),
                _ => None,
            }
        }
        fn forward(&self, host: &str) -> Vec<IpAddr> {
            if host == "crawl-66-249-66-1.googlebot.com" {
                vec!["66.249.66.1".parse().unwrap()]
            } else {
                vec![]
            }
        }
    }

    const G: &str = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";

    fn line(ip: &str, day: u32, path: &str, st: u16, ua: &str) -> String {
        format!(
            "{ip} - - [{day:02}/Oct/2026:10:00:00 +0300] \"GET {path} HTTP/1.1\" {st} 512 \"-\" \"{ua}\" 0.120"
        )
    }

    #[test]
    fn parses_both_formats() {
        let re = combined_re();
        let h = parse_line(&line("1.1.1.1", 3, "/a?x=1", 404, G), &re).unwrap();
        assert_eq!(
            (h.path.as_str(), h.status, h.duration),
            ("/a?x=1", 404, Some(0.12))
        );
        assert!(
            parse_line(
                "1.1.1.1 - - [03/Oct/2026:10:00:00 +0300] \"GET / HTTP/1.0\" 200 -",
                &re
            )
            .is_some()
        );
        let j = parse_line(r#"{"ClientIP":"9.9.9.9","ClientRequestURI":"/x","EdgeResponseStatus":301,"ClientRequestUserAgent":"bingbot","EdgeStartTimestamp":1793000000000000000}"#, &re).unwrap();
        assert_eq!((j.status, j.path.as_str()), (301, "/x"));
        assert!(j.time.is_some());
        assert!(parse_line("garbage", &re).is_none());
    }

    #[test]
    fn verifies_waste_and_sitemap() {
        let mut log = Vec::new();
        for d in 1..=3 {
            log.push(line("66.249.66.1", d, "/hot", 200, G));
        }
        log.push(line("66.249.66.1", 4, "/old", 404, G));
        log.push(line("66.249.66.1", 5, "/p?sort=1", 200, G));
        log.push(line("66.249.66.1", 9, "/extra", 200, G));
        log.push(line("1.2.3.4", 9, "/hot", 200, G));
        log.push(line("8.8.4.4", 9, "/hot", 500, "Mozilla/5.0"));
        let text = log.join("\n");
        let opts = Options {
            sitemap: vec!["https://x.test/hot".into(), "https://x.test/never".into()],
            verify_dns: true,
        };
        let (s, r) = analyze("access.log", &text, &opts, Some(&FakeDns));
        let g = &s.bots["Googlebot"];
        assert_eq!((g.hits, g.verified, g.fake), (7, 6, 1));
        assert_eq!(s.fake_ips, ["1.2.3.4"]);
        assert_eq!(s.never_crawled, ["/never"]);
        assert_eq!(s.not_in_sitemap, ["/extra"]);
        assert_eq!(s.dropping[0].0, "/hot");
        let rules: Vec<&str> = r.findings.iter().map(|f| f.rule.as_str()).collect();
        assert!(rules.contains(&"seo.log.fake-bot"));
        assert!(rules.contains(&"seo.log.crawl-waste-errors"));
        let fake = r
            .findings
            .iter()
            .find(|f| f.rule == "seo.log.fake-bot")
            .unwrap();
        assert_eq!(fake.line, 7);
        // Without DNS: unverified, no fake findings.
        let (s2, r2) = analyze("access.log", &text, &Options::default(), None);
        assert_eq!(s2.bots["Googlebot"].unverified, 7);
        assert!(r2.notes.iter().any(|n| n.contains("unverified")));
    }
}
