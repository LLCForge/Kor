//! `kor seo audit <url|sitemap>` (spec §29.5, §29.16): a bounded, polite crawl and deterministic
//! checks. Every finding names the URL (in `Finding::file`), the HTML line, a selector in the
//! message, the rule (`seo.*`) and a concrete fix. No model is involved.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_runtime::analysis::{Finding, Report, Severity};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use super::fetch::{Chain, Fetcher, Hop, Robots};
use super::html::{self, Page};
use super::rich::{self, Eligibility, Issue};

/// How much of the site is crawled (spec §29.16 `--depth quick|full|forensic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Depth {
    #[default]
    Quick,
    Full,
    Forensic,
}

impl Depth {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "quick" => Some(Self::Quick),
            "full" => Some(Self::Full),
            "forensic" => Some(Self::Forensic),
            _ => None,
        }
    }

    pub fn max_pages(self) -> usize {
        match self {
            Self::Quick => 25,
            Self::Full => 500,
            Self::Forensic => 10_000,
        }
    }

    /// Pages measured in the headless browser (mobile profile).
    pub fn vitals_pages(self) -> usize {
        match self {
            Self::Quick => 1,
            Self::Full => 5,
            Self::Forensic => 20,
        }
    }

    /// External links checked (they hit other hosts, so only on request of a deeper audit).
    fn external_links(self) -> usize {
        match self {
            Self::Quick => 0,
            Self::Full => 100,
            Self::Forensic => 1_000,
        }
    }
}

/// Audit settings.
#[derive(Debug, Clone)]
pub struct Options {
    /// A page URL or a sitemap URL (`…/sitemap.xml`, `…/sitemap_index.xml`, `.xml.gz`).
    pub start: String,
    pub depth: Depth,
    /// Overrides the depth's page limit.
    pub max_pages: Option<usize>,
    /// Minimum gap between two requests to one host (robots' `Crawl-delay` can raise it).
    pub delay: Duration,
    pub user_agent: String,
    /// Measure lab Core Web Vitals in the headless browser.
    pub vitals: bool,
    /// Fetch exactly these URLs (no link following): re-scan to verify fixes.
    pub seeds: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            start: String::new(),
            depth: Depth::Quick,
            max_pages: None,
            delay: Duration::from_millis(500),
            user_agent: super::fetch::default_user_agent(),
            vitals: true,
            seeds: Vec::new(),
        }
    }
}

/// One crawled URL.
#[allow(clippy::struct_excessive_bools)] // independent facts about one response
#[derive(Debug, Clone, Default, Serialize)]
pub struct PageResult {
    pub url: String,
    pub final_url: String,
    pub status: u16,
    pub hops: Vec<Hop>,
    pub redirect_loop: bool,
    pub redirect_too_long: bool,
    /// Clicks from the start page (`None` = only known from the sitemap).
    pub click_depth: Option<u32>,
    pub in_sitemap: bool,
    pub blocked_by_robots: bool,
    pub x_robots: Option<String>,
    pub content_type: String,
    pub elapsed_ms: u64,
    pub bytes: usize,
    pub truncated: bool,
    #[serde(skip)]
    pub page: Option<Page>,
    pub word_count: usize,
    pub vitals: Option<Value>,
    pub rich: Eligibility,
    pub error: Option<String>,
}

impl PageResult {
    fn ok_html(&self) -> bool {
        self.status == 200 && self.page.is_some()
    }

    fn noindex(&self) -> bool {
        self.x_robots
            .as_deref()
            .is_some_and(|x| x.contains("noindex"))
            || self
                .page
                .as_ref()
                .is_some_and(|p| p.robots.iter().any(|r| r.value.contains("noindex")))
    }

    /// Canonical URL resolved against the page (first one).
    pub fn canonical(&self) -> Option<String> {
        let p = self.page.as_ref()?;
        let c = p.canonicals.first()?;
        Url::parse(&self.final_url)
            .ok()?
            .join(&c.value)
            .ok()
            .map(|u| strip_fragment(&u))
    }
}

/// The crawl: pages, sitemap URLs, robots, link graph.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Crawl {
    pub start: String,
    pub origin: String,
    pub pages: Vec<PageResult>,
    pub sitemap_urls: Vec<String>,
    pub robots_sitemaps: Vec<String>,
    pub robots_found: bool,
    /// Internal links: target → [(source, line)].
    #[serde(skip)]
    pub inlinks: HashMap<String, Vec<(String, u32)>>,
    /// External links checked: url → status (0 = network error).
    pub external: BTreeMap<String, u16>,
    pub notes: Vec<String>,
}

impl Crawl {
    pub fn get(&self, url: &str) -> Option<&PageResult> {
        self.pages.iter().find(|p| p.url == url)
    }
}

fn strip_fragment(u: &Url) -> String {
    let mut u = u.clone();
    u.set_fragment(None);
    u.to_string()
}

fn is_sitemap_url(u: &Url) -> bool {
    let p = u.path().to_ascii_lowercase();
    p.contains(".xml") || p.contains("sitemap")
}

fn loc_re() -> Regex {
    Regex::new(r"(?is)<loc>\s*(?:<!\[CDATA\[)?\s*(.*?)\s*(?:\]\]>)?\s*</loc>")
        .expect("static regex")
}

/// Sitemap body (gzip or plain) → (`is_index`, locs).
pub fn parse_sitemap(bytes: &[u8]) -> (bool, Vec<String>) {
    let text = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut s = String::new();
        // Bounded: a gzip bomb stops at 50 MB of text.
        let _ = flate2::read::GzDecoder::new(bytes)
            .take(50 * 1024 * 1024)
            .read_to_string(&mut s);
        s
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let index = text.contains("<sitemapindex");
    let locs = loc_re()
        .captures_iter(&text)
        .map(|c| html::decode_entities(&c[1]))
        .collect();
    (index, locs)
}

/// Read sitemaps (index files followed, at most 50 files and 50 000 URLs).
async fn read_sitemaps(f: &Fetcher, seeds: Vec<String>, notes: &mut Vec<String>) -> Vec<String> {
    let mut queue: VecDeque<String> = seeds.into();
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    while let Some(s) = queue.pop_front() {
        if !seen.insert(s.clone()) || seen.len() > 50 {
            continue;
        }
        let Ok(u) = Url::parse(&s) else { continue };
        match f.get(&u).await {
            Ok(r) if r.status == 200 => {
                let (index, locs) = parse_sitemap(&r.bytes);
                if index {
                    queue.extend(locs);
                } else {
                    for l in locs {
                        if urls.len() >= 50_000 {
                            break;
                        }
                        urls.push(l);
                    }
                }
            }
            Ok(r) => notes.push(format!("sitemap {s}: HTTP {}", r.status)),
            Err(e) => notes.push(format!("sitemap {s}: {e}")),
        }
    }
    urls.sort();
    urls.dedup();
    urls
}

/// Crawl breadth-first from the start page (or the sitemap's URLs), same host only.
#[allow(clippy::too_many_lines)] // one loop: fetch, follow, parse, enqueue
pub async fn crawl(opts: &Options, progress: &(dyn Fn(&str) + Sync)) -> Result<Crawl, String> {
    let start = Url::parse(opts.start.trim()).map_err(|e| format!("{}: {e}", opts.start))?;
    if !matches!(start.scheme(), "http" | "https") {
        return Err(format!("not an http(s) URL: {start}"));
    }
    let f = Fetcher::new(&opts.user_agent, opts.delay, true)?;
    let mut c = Crawl {
        start: start.to_string(),
        origin: start.origin().ascii_serialization(),
        ..Crawl::default()
    };
    let robots: Robots = f.robots(&start).await;
    c.robots_sitemaps.clone_from(&robots.sitemaps);
    c.robots_found = !robots.sitemaps.is_empty() || robots != Robots::default();
    let sitemap_mode = is_sitemap_url(&start);
    let mut seeds: Vec<String> = if sitemap_mode {
        vec![start.to_string()]
    } else {
        robots.sitemaps.clone()
    };
    if seeds.is_empty() {
        seeds.push(format!("{}/sitemap.xml", c.origin));
    }
    progress("sitemap");
    c.sitemap_urls = read_sitemaps(&f, seeds, &mut c.notes).await;
    let max = opts
        .max_pages
        .unwrap_or_else(|| opts.depth.max_pages())
        .max(1);
    let host = start.host_str().unwrap_or("").to_owned();
    let mut queue: VecDeque<(String, Option<u32>)> = VecDeque::new();
    let mut queued = HashSet::new();
    let verify_mode = !opts.seeds.is_empty();
    if verify_mode {
        for u in &opts.seeds {
            if queued.insert(u.clone()) {
                queue.push_back((u.clone(), None));
            }
        }
    } else if !sitemap_mode {
        queue.push_back((strip_fragment(&start), Some(0)));
        queued.insert(strip_fragment(&start));
    }
    if !verify_mode && (sitemap_mode || opts.depth != Depth::Quick) {
        for u in &c.sitemap_urls {
            if queued.insert(u.clone()) {
                queue.push_back((u.clone(), None));
            }
        }
    }
    let sitemap: HashSet<&String> = c.sitemap_urls.iter().collect();
    let mut external: Vec<String> = Vec::new();
    let mut blocked = 0usize;
    // Depth found later by links beats "sitemap only".
    let mut depth_of: HashMap<String, u32> = HashMap::new();
    while let Some((url, depth)) = queue.pop_front() {
        if c.pages.len() >= max {
            c.notes.push(format!(
                "page limit {max} reached; {} URLs not crawled",
                queue.len() + 1
            ));
            break;
        }
        let Ok(u) = Url::parse(&url) else { continue };
        let mut pr = PageResult {
            url: url.clone(),
            final_url: url.clone(),
            in_sitemap: sitemap.contains(&url),
            click_depth: depth.or_else(|| depth_of.get(&url).copied()),
            ..PageResult::default()
        };
        if !f.allowed(&u).await {
            pr.blocked_by_robots = true;
            blocked += 1;
            c.pages.push(pr);
            continue;
        }
        progress(&url);
        match f.follow(&u).await {
            Err(e) => pr.error = Some(e),
            Ok((hops, chain)) => {
                pr.hops = hops;
                match chain {
                    Chain::Loop => pr.redirect_loop = true,
                    Chain::TooLong => pr.redirect_too_long = true,
                    Chain::Done(r) => {
                        pr.final_url.clone_from(&r.url);
                        pr.status = r.status;
                        pr.elapsed_ms = r.elapsed_ms;
                        pr.bytes = r.bytes.len();
                        pr.truncated = r.truncated;
                        pr.x_robots = r.header("x-robots-tag").map(str::to_ascii_lowercase);
                        pr.content_type = r.header("content-type").unwrap_or("").to_owned();
                        let same_host = Url::parse(&r.url)
                            .ok()
                            .is_some_and(|fu| fu.host_str() == Some(host.as_str()));
                        if r.status == 200 && r.is_html() && same_host {
                            let page = html::parse(&r.body);
                            pr.word_count = page.word_count;
                            let base = Url::parse(&r.url).unwrap_or_else(|_| u.clone());
                            let next_depth = pr.click_depth.map(|d| d + 1);
                            for l in &page.links {
                                let Ok(t) = base.join(&l.href) else { continue };
                                if !matches!(t.scheme(), "http" | "https") {
                                    continue;
                                }
                                let target = strip_fragment(&t);
                                if t.host_str() == Some(host.as_str()) {
                                    c.inlinks
                                        .entry(target.clone())
                                        .or_default()
                                        .push((url.clone(), l.line));
                                    if let Some(d) = next_depth {
                                        depth_of.entry(target.clone()).or_insert(d);
                                    }
                                    if !verify_mode && queued.insert(target.clone()) {
                                        queue.push_back((target, next_depth));
                                    }
                                } else if !external.contains(&target) {
                                    external.push(target);
                                }
                            }
                            for j in &page.jsonld {
                                let _ = rich::check(&j.value, &mut pr.rich);
                            }
                            pr.page = Some(page);
                        }
                    }
                }
            }
        }
        c.pages.push(pr);
    }
    // Click depth learned after a sitemap-only page was fetched.
    for p in &mut c.pages {
        if p.click_depth.is_none() {
            p.click_depth = depth_of.get(&p.url).copied();
        }
    }
    if blocked > 0 {
        c.notes
            .push(format!("{blocked} URLs skipped: disallowed by robots.txt"));
    }
    for e in external.into_iter().take(opts.depth.external_links()) {
        let Ok(u) = Url::parse(&e) else { continue };
        if !f.allowed(&u).await {
            continue;
        }
        let status = match f.follow(&u).await {
            Ok((_, Chain::Done(r))) => r.status,
            Ok(_) | Err(_) => 0,
        };
        c.external.insert(e, status);
    }
    if opts.vitals {
        measure_vitals(&mut c, opts.depth.vitals_pages(), progress).await;
    } else {
        c.notes
            .push("Core Web Vitals not measured (--no-vitals)".into());
    }
    Ok(c)
}

async fn measure_vitals(c: &mut Crawl, n: usize, progress: &(dyn Fn(&str) + Sync)) {
    let targets: Vec<usize> = c
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.ok_html())
        .map(|(i, _)| i)
        .take(n)
        .collect();
    for i in targets {
        let url = c.pages[i].final_url.clone();
        progress(&format!("vitals {url}"));
        match kor_tools::lab_vitals(&url, true).await {
            Ok(v) => c.pages[i].vitals = Some(v),
            Err(e) => {
                c.notes.push(format!("Core Web Vitals not measured: {e}"));
                return;
            }
        }
    }
    c.notes.push(
        "Core Web Vitals are lab values (headless Chrome, mobile profile: 4x CPU slowdown, 150 ms RTT); TBT stands in for INP"
            .into(),
    );
}

// ---------------------------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------------------------

/// Title limit in characters (Google cuts around 580 px ≈ 60 characters).
pub const TITLE_MAX: usize = 60;
pub const TITLE_MIN: usize = 30;
/// Description limits in characters (≈ 920 px on desktop).
pub const DESC_MAX: usize = 160;
pub const DESC_MIN: usize = 70;
const THIN_WORDS: usize = 200;
const SLOW_MS: u64 = 1_500;

struct Out<'a> {
    r: &'a mut Report,
}

impl Out<'_> {
    fn push(
        &mut self,
        rule: &str,
        sev: Severity,
        url: &str,
        line: u32,
        msg: String,
        fix: Option<String>,
    ) {
        self.r.findings.push(Finding {
            rule: format!("seo.{rule}"),
            source: "kor".into(),
            severity: sev,
            file: PathBuf::from(url),
            line: line.max(1),
            column: None,
            message: msg,
            fix,
            cwe: None,
        });
    }
}

fn hreflang_ok(code: &str) -> bool {
    code.eq_ignore_ascii_case("x-default")
        || Regex::new(r"^[a-zA-Z]{2,3}(-([a-zA-Z]{2}|[0-9]{3}|[a-zA-Z]{4}))?$")
            .expect("static regex")
            .is_match(code)
}

/// ASCII slug the way Turkish sites expect it (ı→i, ş→s, ğ→g, ü→u, ö→o, ç→c).
pub fn tr_slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        let mapped = match ch {
            'ı' | 'İ' | 'I' => 'i',
            'ş' | 'Ş' => 's',
            'ğ' | 'Ğ' => 'g',
            'ü' | 'Ü' => 'u',
            'ö' | 'Ö' => 'o',
            'ç' | 'Ç' => 'c',
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            '/' | '.' => ch,
            _ => '-',
        };
        if mapped == '-' && out.ends_with('-') {
            continue;
        }
        out.push(mapped);
    }
    out
}

const SPAM_WORDS: &[&str] = &[
    "casino",
    "bahis",
    "betting",
    "viagra",
    "cialis",
    "porn",
    "escort",
    "kumar",
    "slot",
    "payday loan",
];

/// Run every deterministic check on a crawl.
#[allow(clippy::too_many_lines)] // a flat list of checks reads best in one place
pub fn check(c: &Crawl) -> Report {
    let mut report = Report::new("seo-audit");
    let mut o = Out { r: &mut report };
    let crawled: HashMap<&str, &PageResult> = c.pages.iter().map(|p| (p.url.as_str(), p)).collect();
    let https_site = c.origin.starts_with("https:");

    if c.sitemap_urls.is_empty() {
        o.push(
            "sitemap-missing",
            Severity::Low,
            &c.origin,
            1,
            "No XML sitemap found (robots.txt Sitemap: lines and /sitemap.xml)".into(),
            Some(format!(
                "Publish a sitemap and add `Sitemap: {}/sitemap.xml` to robots.txt",
                c.origin
            )),
        );
    } else if c.robots_sitemaps.is_empty() {
        o.push(
            "robots-sitemap-missing",
            Severity::Info,
            &format!("{}/robots.txt", c.origin),
            1,
            "robots.txt does not list the sitemap".into(),
            Some(format!(
                "Add `Sitemap: {}/sitemap.xml` to robots.txt",
                c.origin
            )),
        );
    }

    // Duplicate titles / descriptions across indexable pages.
    let mut titles: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut descs: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for p in c.pages.iter().filter(|p| p.ok_html() && !p.noindex()) {
        let pg = p.page.as_ref().expect("ok_html");
        if let Some(t) = pg.titles.first().filter(|t| !t.value.is_empty()) {
            titles.entry(t.value.clone()).or_default().push(&p.url);
        }
        if let Some(d) = pg.descriptions.first().filter(|d| !d.value.is_empty()) {
            descs.entry(d.value.clone()).or_default().push(&p.url);
        }
    }

    for p in &c.pages {
        let url = p.url.as_str();
        let first_link = c.inlinks.get(url).and_then(|v| v.first());
        if p.blocked_by_robots {
            if p.in_sitemap {
                o.push(
                    "sitemap-disallowed",
                    Severity::Medium,
                    url,
                    1,
                    "URL is in the sitemap but robots.txt disallows it".into(),
                    Some("Remove it from the sitemap or allow it in robots.txt".into()),
                );
            }
            continue;
        }
        if let Some(e) = &p.error {
            o.push(
                "fetch-error",
                Severity::High,
                url,
                1,
                format!("Request failed: {e}"),
                None,
            );
            continue;
        }
        if p.redirect_loop {
            o.push(
                "redirect-loop",
                Severity::High,
                url,
                1,
                format!("Redirect loop: {}", chain_text(&p.hops)),
                Some("Point every hop at the final URL; remove the rule that sends it back".into()),
            );
            continue;
        }
        if p.redirect_too_long {
            o.push(
                "redirect-too-long",
                Severity::High,
                url,
                1,
                format!(
                    "More than {} redirects: {}",
                    super::fetch::MAX_HOPS,
                    chain_text(&p.hops)
                ),
                None,
            );
            continue;
        }
        if p.hops.len() > 1 {
            o.push(
                "redirect-chain",
                Severity::Medium,
                url,
                1,
                format!(
                    "Redirect chain of {} hops: {}",
                    p.hops.len(),
                    chain_text(&p.hops)
                ),
                Some(format!(
                    "Redirect {url} straight to {} (one 301)",
                    p.final_url
                )),
            );
        }
        if p.hops.iter().any(|h| matches!(h.status, 302 | 307)) {
            o.push(
                "redirect-temporary",
                Severity::Low,
                url,
                1,
                format!(
                    "Temporary redirect ({}) on a permanent move",
                    chain_text(&p.hops)
                ),
                Some("Use 301/308 for permanent moves".into()),
            );
        }
        if !p.hops.is_empty()
            && let Some((src, line)) = first_link
        {
            o.push(
                "internal-link-redirect",
                Severity::Low,
                src,
                *line,
                format!("a[href] links to {url}, which redirects to {}", p.final_url),
                Some(format!("Link to {} directly", p.final_url)),
            );
        }
        if p.status >= 400 {
            let sev = if p.status >= 500 {
                Severity::High
            } else {
                Severity::Medium
            };
            o.push(
                "status",
                sev,
                url,
                1,
                format!("HTTP {}", p.status),
                Some("Fix the page or 301 it to the closest live page; update links".into()),
            );
            for (src, line) in c.inlinks.get(url).into_iter().flatten().take(20) {
                o.push(
                    "broken-link",
                    Severity::Medium,
                    src,
                    *line,
                    format!("a[href=\"{url}\"] → HTTP {}", p.status),
                    Some(format!("Change or remove the link to {url}")),
                );
            }
        }
        if p.in_sitemap && (p.status != 200 || !p.hops.is_empty()) {
            o.push(
                "sitemap-url-status",
                Severity::Medium,
                url,
                1,
                format!(
                    "Sitemap URL answers {} {}",
                    p.status,
                    if p.hops.is_empty() { "" } else { "(redirect)" }
                ),
                Some("List only final, 200 URLs in the sitemap".into()),
            );
        }
        if p.elapsed_ms > SLOW_MS {
            o.push(
                "slow-response",
                Severity::Low,
                url,
                1,
                format!("Server answered in {} ms (over {SLOW_MS} ms)", p.elapsed_ms),
                Some("Enable page caching / check server time to first byte".into()),
            );
        }
        let Some(pg) = p.page.as_ref().filter(|_| p.status == 200) else {
            continue;
        };
        let noindex = p.noindex();
        if noindex {
            let (rule, line, sel) = match pg.robots.iter().find(|r| r.value.contains("noindex")) {
                Some(r) => ("noindex", r.line, "meta[name=robots]"),
                None => ("x-robots-noindex", 1, "X-Robots-Tag header"),
            };
            let sev = if p.in_sitemap {
                Severity::Medium
            } else {
                Severity::Info
            };
            o.push(
                rule,
                sev,
                url,
                line,
                format!(
                    "{sel} says noindex{}",
                    if p.in_sitemap {
                        " but the URL is in the sitemap"
                    } else {
                        ""
                    }
                ),
                Some(
                    "Remove noindex if the page should rank, else drop it from the sitemap".into(),
                ),
            );
        }
        if p.truncated || p.bytes > 1_500_000 {
            o.push(
                "large-html",
                Severity::Low,
                url,
                1,
                format!("HTML is {} bytes", p.bytes),
                None,
            );
        }
        // Title
        match pg.titles.as_slice() {
            [] => o.push(
                "title-missing",
                Severity::High,
                url,
                1,
                "head > title is missing".into(),
                Some("Add a unique <title> of 30–60 characters with the main keyword".into()),
            ),
            [t, rest @ ..] => {
                if !rest.is_empty() {
                    o.push(
                        "title-multiple",
                        Severity::Medium,
                        url,
                        rest[0].line,
                        format!("{} <title> elements", rest.len() + 1),
                        Some("Keep one <title>".into()),
                    );
                }
                let n = t.value.chars().count();
                if n == 0 {
                    o.push(
                        "title-missing",
                        Severity::High,
                        url,
                        t.line,
                        "head > title is empty".into(),
                        None,
                    );
                } else if n > TITLE_MAX {
                    o.push("title-length", Severity::Low, url, t.line, format!("head > title is {n} characters (over {TITLE_MAX}, cut in results): \"{}\"", t.value), Some(format!("Shorten to ≤{TITLE_MAX} characters")));
                } else if n < TITLE_MIN {
                    o.push(
                        "title-length",
                        Severity::Low,
                        url,
                        t.line,
                        format!(
                            "head > title is {n} characters (under {TITLE_MIN}): \"{}\"",
                            t.value
                        ),
                        Some("Make it more descriptive".into()),
                    );
                }
                if !noindex && let Some(others) = titles.get(&t.value).filter(|v| v.len() > 1) {
                    let list: Vec<&&str> = others.iter().filter(|u| **u != url).take(5).collect();
                    o.push(
                        "title-duplicate",
                        Severity::Medium,
                        url,
                        t.line,
                        format!(
                            "Same title on {} pages: {}",
                            others.len(),
                            list.iter().map(|s| **s).collect::<Vec<_>>().join(", ")
                        ),
                        Some("Write a unique title per page".into()),
                    );
                }
            }
        }
        // Description
        match pg.descriptions.first() {
            None => o.push(
                "description-missing",
                Severity::Medium,
                url,
                1,
                "meta[name=description] is missing".into(),
                Some(format!("Add a {DESC_MIN}–{DESC_MAX} character description")),
            ),
            Some(d) => {
                let n = d.value.chars().count();
                if n == 0 {
                    o.push(
                        "description-missing",
                        Severity::Medium,
                        url,
                        d.line,
                        "meta[name=description] is empty".into(),
                        None,
                    );
                } else if !(DESC_MIN..=DESC_MAX).contains(&n) {
                    o.push("description-length", Severity::Low, url, d.line, format!("meta[name=description] is {n} characters (aim for {DESC_MIN}–{DESC_MAX})"), None);
                }
                if !noindex && let Some(others) = descs.get(&d.value).filter(|v| v.len() > 1) {
                    o.push(
                        "description-duplicate",
                        Severity::Low,
                        url,
                        d.line,
                        format!("Same description on {} pages", others.len()),
                        Some("Write a unique description per page".into()),
                    );
                }
            }
        }
        // Headings
        let h1: Vec<_> = pg.headings.iter().filter(|h| h.value.0 == 1).collect();
        match h1.len() {
            0 => o.push(
                "h1-missing",
                Severity::Medium,
                url,
                1,
                "No h1 on the page".into(),
                Some("Add one h1 that states the page topic".into()),
            ),
            1 => {}
            n => o.push(
                "h1-multiple",
                Severity::Low,
                url,
                h1[1].line,
                format!("{n} h1 elements"),
                Some("Keep one h1; demote the others to h2".into()),
            ),
        }
        let mut prev = 0u8;
        for h in &pg.headings {
            let lvl = h.value.0;
            if prev > 0 && lvl > prev + 1 {
                o.push(
                    "heading-skip",
                    Severity::Low,
                    url,
                    h.line,
                    format!("h{lvl} \"{}\" follows h{prev} (level skipped)", h.value.1),
                    Some(format!("Use h{}", prev + 1)),
                );
                break;
            }
            prev = lvl;
        }
        // Images
        for img in &pg.images {
            if img.alt.is_none() {
                o.push(
                    "img-alt-missing",
                    Severity::Low,
                    url,
                    img.line,
                    format!("img[src=\"{}\"] has no alt attribute", img.src),
                    Some("Describe the image in alt (alt=\"\" if decorative)".into()),
                );
            }
            if img.width.is_none() || img.height.is_none() {
                o.push(
                    "img-size-missing",
                    Severity::Low,
                    url,
                    img.line,
                    format!(
                        "img[src=\"{}\"] has no width/height (layout shift)",
                        img.src
                    ),
                    Some("Set width and height attributes".into()),
                );
            }
        }
        // Canonical
        match pg.canonicals.as_slice() {
            [] => o.push(
                "canonical-missing",
                Severity::Low,
                url,
                1,
                "link[rel=canonical] is missing".into(),
                Some(format!(
                    "Add <link rel=\"canonical\" href=\"{}\">",
                    p.final_url
                )),
            ),
            [first, rest @ ..] => {
                if !rest.is_empty() {
                    o.push(
                        "canonical-multiple",
                        Severity::Medium,
                        url,
                        rest[0].line,
                        format!("{} canonical links", rest.len() + 1),
                        Some("Keep one canonical".into()),
                    );
                }
                if let Some(canon) = p.canonical() {
                    if canon != p.final_url {
                        if let Some(t) = crawled.get(canon.as_str())
                            && (t.status != 200 || !t.hops.is_empty() || t.noindex())
                        {
                            o.push(
                                "canonical-target",
                                Severity::High,
                                url,
                                first.line,
                                format!(
                                    "link[rel=canonical] points to {canon}, which is {}",
                                    if t.noindex() {
                                        "noindex".to_owned()
                                    } else if t.hops.is_empty() {
                                        format!("HTTP {}", t.status)
                                    } else {
                                        "a redirect".to_owned()
                                    }
                                ),
                                Some("Point the canonical at a live, indexable URL".into()),
                            );
                        }
                        if p.in_sitemap {
                            o.push(
                                "sitemap-non-canonical",
                                Severity::Medium,
                                url,
                                first.line,
                                format!("Sitemap lists a page whose canonical is {canon}"),
                                Some(format!("List {canon} in the sitemap instead")),
                            );
                        }
                    }
                } else {
                    o.push(
                        "canonical-invalid",
                        Severity::Medium,
                        url,
                        first.line,
                        format!("link[rel=canonical] href \"{}\" is not a URL", first.value),
                        None,
                    );
                }
            }
        }
        // hreflang
        if !pg.hreflang.is_empty() {
            let base = Url::parse(&p.final_url).ok();
            let mut self_ref = false;
            for h in &pg.hreflang {
                if !hreflang_ok(&h.lang) {
                    o.push(
                        "hreflang-invalid",
                        Severity::Medium,
                        url,
                        h.line,
                        format!(
                            "link[hreflang=\"{}\"] is not an ISO 639-1 / 3166-1 code",
                            h.lang
                        ),
                        None,
                    );
                }
                let target = base
                    .as_ref()
                    .and_then(|b| b.join(&h.href).ok())
                    .map(|u| strip_fragment(&u));
                if target.as_deref() == Some(p.final_url.as_str()) {
                    self_ref = true;
                } else if let Some(t) = target.as_deref().and_then(|t| crawled.get(t))
                    && let Some(tp) = t.page.as_ref()
                {
                    let back = tp.hreflang.iter().any(|x| {
                        Url::parse(&t.final_url)
                            .ok()
                            .and_then(|b| b.join(&x.href).ok())
                            .is_some_and(|u| strip_fragment(&u) == p.final_url)
                    });
                    if !back {
                        o.push(
                            "hreflang-no-return",
                            Severity::Medium,
                            url,
                            h.line,
                            format!("{} does not link back with hreflang", t.url),
                            Some(
                                "Every alternate must list all the others, including this page"
                                    .into(),
                            ),
                        );
                    }
                    if t.status != 200 {
                        o.push(
                            "hreflang-target",
                            Severity::Medium,
                            url,
                            h.line,
                            format!("hreflang target {} answers HTTP {}", t.url, t.status),
                            None,
                        );
                    }
                }
            }
            if !self_ref {
                o.push(
                    "hreflang-no-self",
                    Severity::Low,
                    url,
                    pg.hreflang[0].line,
                    "hreflang set without a self-reference".into(),
                    Some(format!(
                        "Add <link rel=\"alternate\" hreflang=\"…\" href=\"{}\">",
                        p.final_url
                    )),
                );
            }
        }
        // Structured data
        for j in &pg.jsonld {
            let mut scratch = Eligibility::default();
            for issue in rich::check(&j.value, &mut scratch) {
                let (rule, sev, msg) = match issue {
                    Issue::Invalid(e) => (
                        "jsonld-invalid",
                        Severity::Medium,
                        format!("script[type=\"application/ld+json\"] is not valid JSON: {e}"),
                    ),
                    Issue::NoContext => (
                        "jsonld-no-context",
                        Severity::Low,
                        "JSON-LD without @context".into(),
                    ),
                    Issue::NoType => (
                        "jsonld-no-type",
                        Severity::Low,
                        "JSON-LD without @type".into(),
                    ),
                    Issue::MissingRequired { ty, field } => (
                        "richresult-required",
                        Severity::Medium,
                        format!("{ty}: required property `{field}` is missing (no rich result)"),
                    ),
                    Issue::MissingRecommended { ty, field } => (
                        "richresult-recommended",
                        Severity::Info,
                        format!("{ty}: recommended property `{field}` is missing"),
                    ),
                    Issue::Retired { ty, note } => (
                        "richresult-retired",
                        Severity::Info,
                        format!("{ty}: {note}"),
                    ),
                };
                o.push(rule, sev, url, j.line, msg, None);
            }
        }
        // Mobile and language
        if !pg.viewport {
            o.push(
                "viewport-missing",
                Severity::Medium,
                url,
                1,
                "meta[name=viewport] is missing (not mobile friendly)".into(),
                Some(
                    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
                        .into(),
                ),
            );
        }
        if pg.lang.as_deref().is_none_or(str::is_empty) {
            o.push(
                "lang-missing",
                Severity::Low,
                url,
                1,
                "html[lang] is missing".into(),
                Some("Set <html lang=\"tr\"> (or the page language)".into()),
            );
        }
        if pg.word_count < THIN_WORDS && !noindex {
            o.push(
                "thin-content",
                Severity::Low,
                url,
                1,
                format!("{} words of text (under {THIN_WORDS})", pg.word_count),
                Some("Expand the content or noindex the page".into()),
            );
        }
        // Mixed content
        if p.final_url.starts_with("https:") {
            for r in &pg.resources {
                if r.url.starts_with("http://") {
                    let sev = if matches!(r.tag.as_str(), "script" | "iframe") {
                        Severity::High
                    } else {
                        Severity::Medium
                    };
                    o.push(
                        "mixed-content",
                        sev,
                        url,
                        r.line,
                        format!(
                            "{}[src=\"{}\"] loads over http on an https page",
                            r.tag, r.url
                        ),
                        Some(format!("Use {}", r.url.replacen("http://", "https://", 1))),
                    );
                }
            }
        }
        // Links
        let base = Url::parse(&p.final_url).ok();
        for l in &pg.links {
            let Some(t) = base.as_ref().and_then(|b| b.join(&l.href).ok()) else {
                continue;
            };
            let internal = Some(t.origin().ascii_serialization()) == Some(c.origin.clone());
            if internal && l.rel.contains("nofollow") {
                o.push(
                    "nofollow-internal",
                    Severity::Low,
                    url,
                    l.line,
                    format!("a[href=\"{}\"][rel=nofollow] on an internal link", l.href),
                    Some("Drop nofollow from internal links".into()),
                );
            }
            if internal && t.query().is_some() {
                o.push(
                    "param-url",
                    Severity::Info,
                    url,
                    l.line,
                    format!(
                        "a[href=\"{}\"] links a parameter URL (crawl budget)",
                        l.href
                    ),
                    Some("Link the clean URL; canonicalize parameter variants".into()),
                );
            }
            if https_site
                && t.scheme() == "http"
                && t.host_str() == base.as_ref().and_then(Url::host_str)
            {
                o.push(
                    "insecure-link",
                    Severity::Low,
                    url,
                    l.line,
                    format!("a[href=\"{}\"] uses http", l.href),
                    Some("Link the https URL".into()),
                );
            }
            if !internal {
                let ext = strip_fragment(&t);
                if let Some(&st) = c.external.get(&ext)
                    && (st == 0 || st >= 400)
                {
                    o.push(
                        "broken-external-link",
                        Severity::Low,
                        url,
                        l.line,
                        format!(
                            "a[href=\"{ext}\"] → {}",
                            if st == 0 {
                                "unreachable".to_owned()
                            } else {
                                format!("HTTP {st}")
                            }
                        ),
                        Some("Update or remove the link".into()),
                    );
                }
                let lower = l.text.to_lowercase();
                if l.hidden {
                    let spam = SPAM_WORDS
                        .iter()
                        .any(|w| lower.contains(w) || ext.to_lowercase().contains(w));
                    o.push(
                        if spam { "spam-link" } else { "hidden-link" },
                        if spam {
                            Severity::High
                        } else {
                            Severity::Medium
                        },
                        url,
                        l.line,
                        format!(
                            "Hidden link to {ext} (\"{}\") — possible injected spam",
                            l.text
                        ),
                        Some("Check theme/plugin files and the database for injected links".into()),
                    );
                }
            }
        }
        // Turkish slugs
        if let Ok(u) = Url::parse(url) {
            let decoded = percent_encoding::percent_decode_str(u.path())
                .decode_utf8_lossy()
                .into_owned();
            if decoded.chars().any(|ch| "ıİşŞğĞüÜöÖçÇ".contains(ch))
                || decoded.chars().any(char::is_uppercase)
            {
                o.push(
                    "tr-slug",
                    Severity::Low,
                    url,
                    1,
                    format!("URL path \"{decoded}\" has Turkish or uppercase characters"),
                    Some(format!(
                        "Use /{} and 301 the old URL",
                        tr_slug(decoded.trim_start_matches('/'))
                    )),
                );
            }
        }
        // Depth / orphan
        if let Some(d) = p.click_depth.filter(|d| *d > 3) {
            o.push(
                "deep-page",
                Severity::Low,
                url,
                1,
                format!("{d} clicks from the start page"),
                Some("Link it from a hub or category page".into()),
            );
        }
        if p.in_sitemap && !c.inlinks.contains_key(url) && p.click_depth != Some(0) {
            o.push(
                "orphan",
                Severity::Medium,
                url,
                1,
                "In the sitemap but no crawled page links to it".into(),
                Some("Add internal links from related pages".into()),
            );
        }
        if !c.sitemap_urls.is_empty()
            && !p.in_sitemap
            && !noindex
            && p.hops.is_empty()
            && p.canonical().is_none_or(|cn| cn == p.final_url)
        {
            o.push(
                "not-in-sitemap",
                Severity::Low,
                url,
                1,
                "Indexable page missing from the sitemap".into(),
                Some("Add it to the sitemap".into()),
            );
        }
        // Lab vitals
        if let Some(v) = &p.vitals {
            vitals_findings(&mut o, url, v);
        }
    }
    report.notes.extend(c.notes.iter().cloned());
    report.notes.push(format!(
        "{} pages crawled from {}; {} sitemap URLs",
        c.pages.len(),
        c.start,
        c.sitemap_urls.len()
    ));
    report.normalize();
    report
}

fn vitals_findings(o: &mut Out, url: &str, v: &Value) {
    let num = |k: &str| v.get(k).and_then(Value::as_f64);
    if let Some(lcp) = num("lcp_ms")
        && lcp > 2_500.0
    {
        o.push(
            "cwv-lcp",
            if lcp > 4_000.0 {
                Severity::High
            } else {
                Severity::Medium
            },
            url,
            1,
            format!("LCP {lcp:.0} ms (lab, mobile; good ≤ 2500)"),
            Some("Preload the LCP image, cut render-blocking CSS/JS, improve TTFB".into()),
        );
    }
    if let Some(cls) = num("cls")
        && cls > 0.1
    {
        o.push(
            "cwv-cls",
            if cls > 0.25 {
                Severity::High
            } else {
                Severity::Medium
            },
            url,
            1,
            format!("CLS {cls:.3} (lab; good ≤ 0.1)"),
            Some("Reserve space for images, ads and embeds".into()),
        );
    }
    if let Some(tbt) = num("tbt_ms")
        && tbt > 200.0
    {
        o.push(
            "cwv-tbt",
            if tbt > 600.0 {
                Severity::High
            } else {
                Severity::Medium
            },
            url,
            1,
            format!("TBT {tbt:.0} ms (lab stand-in for INP; good ≤ 200)"),
            Some("Split long tasks, defer third-party scripts".into()),
        );
    }
    if let Some(list) = v
        .get("render_blocking")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
    {
        let names: Vec<&str> = list.iter().filter_map(Value::as_str).take(5).collect();
        o.push(
            "render-blocking",
            Severity::Low,
            url,
            1,
            format!(
                "{} render-blocking resources: {}",
                list.len(),
                names.join(", ")
            ),
            Some("Inline critical CSS; add defer/async to scripts".into()),
        );
    }
}

fn chain_text(h: &[Hop]) -> String {
    let mut s = String::new();
    for (i, x) in h.iter().enumerate() {
        if i == 0 {
            s.push_str(&x.from);
        }
        let _ = write!(s, " →{} {}", x.status, x.to);
    }
    s
}

// ---------------------------------------------------------------------------------------------
// History and reports
// ---------------------------------------------------------------------------------------------

/// `<root>/.kor/seo/audits/<host>/`.
pub fn history_dir(root: &Path, start: &str) -> PathBuf {
    let host = Url::parse(start)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "site".into());
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    root.join(".kor").join("seo").join("audits").join(safe)
}

/// Save a report as `<ts>.json`; returns the previous one, if any.
pub fn save(root: &Path, start: &str, r: &Report) -> Result<Option<Report>, String> {
    save_to(&history_dir(root, start), r)
}

/// Save into `dir` (the team keeps its own history in `<host>/agents`).
pub fn save_to(dir: &Path, r: &Report) -> Result<Option<Report>, String> {
    let dir = dir.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let prev = latest(&dir);
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let text = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{ts}.json")), text).map_err(|e| e.to_string())?;
    Ok(prev)
}

fn latest(dir: &Path) -> Option<Report> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        // `<timestamp>.json` only (not `pages-latest.json`).
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name().is_some_and(|n| {
                    n.to_string_lossy()
                        .starts_with(|c: char| c.is_ascii_digit())
                })
        })
        .collect();
    files.sort();
    let text = std::fs::read_to_string(files.last()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Save the crawled pages' URL and title (`pages-latest.json`) for internal-link suggestions.
pub fn save_pages(root: &Path, c: &Crawl) -> Result<(), String> {
    let dir = history_dir(root, &c.start);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let pages: Vec<(String, String)> = c
        .pages
        .iter()
        .filter(|p| p.ok_html() && !p.noindex())
        .map(|p| {
            let t = p
                .page
                .as_ref()
                .and_then(|pg| pg.titles.first())
                .map(|t| t.value.clone())
                .unwrap_or_default();
            (p.final_url.clone(), t)
        })
        .collect();
    let text = serde_json::to_string(&pages).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pages-latest.json"), text).map_err(|e| e.to_string())
}

/// Pages saved by the latest audit of `site` (a URL or host).
pub fn load_pages(root: &Path, site: &str) -> Vec<(String, String)> {
    let start = if site.contains("://") {
        site.to_owned()
    } else {
        format!("https://{site}")
    };
    std::fs::read_to_string(history_dir(root, &start).join("pages-latest.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The latest saved report of `site`, if any.
pub fn latest_report(root: &Path, site: &str) -> Option<Report> {
    let start = if site.contains("://") {
        site.to_owned()
    } else {
        format!("https://{site}")
    };
    latest(&history_dir(root, &start))
}

/// What changed since the previous audit, keyed by URL + rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Diff {
    pub new: Vec<Finding>,
    pub fixed: Vec<Finding>,
}

pub fn diff(prev: &Report, cur: &Report) -> Diff {
    let key = |f: &Finding| (f.file.clone(), f.rule.clone(), f.message.clone());
    let old: HashSet<_> = prev.findings.iter().map(key).collect();
    let now: HashSet<_> = cur.findings.iter().map(key).collect();
    Diff {
        new: cur
            .findings
            .iter()
            .filter(|f| !old.contains(&key(f)))
            .cloned()
            .collect(),
        fixed: prev
            .findings
            .iter()
            .filter(|f| !now.contains(&key(f)))
            .cloned()
            .collect(),
    }
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Self-contained, printable HTML report (brandable: `brand` is shown as the header line).
/// `labels` = (title, severity, url, line, rule, finding, fix, notes).
pub fn to_html(r: &Report, title: &str, brand: Option<&str>, labels: [&str; 8]) -> String {
    let mut rows = String::new();
    for f in &r.findings {
        let _ = write!(
            rows,
            "<tr class=\"{}\"><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            f.severity.as_str(),
            f.severity.as_str(),
            escape(&f.file.to_string_lossy()),
            f.line,
            escape(&f.rule),
            escape(&f.message),
            escape(f.fix.as_deref().unwrap_or(""))
        );
    }
    let notes: String = r
        .notes
        .iter()
        .map(|n| format!("<li>{}</li>", escape(n)))
        .collect();
    let c = r.counts();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{t}</title><style>\
body{{font-family:system-ui,sans-serif;margin:24px;color:#1b1b1b}}table{{border-collapse:collapse;width:100%;font-size:12px}}\
td,th{{border:1px solid #ccc;padding:4px 6px;text-align:left;vertical-align:top;word-break:break-word}}\
.critical td:first-child,.high td:first-child{{color:#b00020;font-weight:600}}.medium td:first-child{{color:#a15c00}}</style></head>\
<body>{b}<h1>{t}</h1><p>critical {c0} · high {c1} · medium {c2} · low {c3} · info {c4}</p>\
<table><tr><th>{l1}</th><th>{l2}</th><th>{l3}</th><th>{l4}</th><th>{l5}</th><th>{l6}</th></tr>{rows}</table>\
<h2>{l7}</h2><ul>{notes}</ul></body></html>",
        t = escape(title),
        b = brand
            .map(|b| format!("<p><strong>{}</strong></p>", escape(b)))
            .unwrap_or_default(),
        c0 = c[0],
        c1 = c[1],
        c2 = c[2],
        c3 = c[3],
        c4 = c[4],
        l1 = escape(labels[1]),
        l2 = escape(labels[2]),
        l3 = escape(labels[3]),
        l4 = escape(labels[4]),
        l5 = escape(labels[5]),
        l6 = escape(labels[6]),
        l7 = escape(labels[7]),
    )
}

/// Routine for `kor seo audit --schedule daily|weekly`: rerun and report the difference.
pub fn routine(
    root: &Path,
    start: &str,
    depth: Depth,
    every: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let minutes = match every {
        "daily" => 24 * 60,
        "weekly" => 7 * 24 * 60,
        _ => return Err("schedule: daily or weekly".into()),
    };
    if Url::parse(start).is_err() || start.contains(['`', '\n', ';', '&', '|', ' ']) {
        return Err(format!("bad URL: {start}"));
    }
    let d = match depth {
        Depth::Quick => "quick",
        Depth::Full => "full",
        Depth::Forensic => "forensic",
    };
    let mut m = serde_json::Map::new();
    m.insert(
        "name".into(),
        format!("SEO audit: {start} ({every})").into(),
    );
    m.insert("prompt".into(), format!("Run the shell command `kor seo audit {start} --depth {d}` in this folder and report the new and fixed findings it lists compared with the previous audit; alert on new critical/high findings and worse Core Web Vitals. Do not change any files.").into());
    m.insert("intervalMinutes".into(), minutes.into());
    m.insert("root".into(), root.to_string_lossy().into_owned().into());
    Ok(m)
}

/// Result of re-scanning the URLs of a previous report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Verified {
    pub fixed: Vec<Finding>,
    pub remaining: Vec<Finding>,
    pub new: Vec<Finding>,
    pub urls: usize,
}

/// Re-fetch only the pages that had findings in `prev` and compare (spec §29.5: fix → re-scan).
/// Site-wide rules (orphans, duplicates across pages, sitemap coverage) need a full audit.
pub async fn verify(
    prev: &Report,
    opts: &Options,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Verified, String> {
    const SITE_WIDE: &[&str] = &[
        "seo.orphan",
        "seo.title-duplicate",
        "seo.description-duplicate",
        "seo.not-in-sitemap",
        "seo.deep-page",
        "seo.broken-link",
        "seo.internal-link-redirect",
        "seo.sitemap-missing",
        "seo.robots-sitemap-missing",
    ];
    let origin = Url::parse(&opts.start)
        .map_err(|e| e.to_string())?
        .origin()
        .ascii_serialization();
    let mut seeds: Vec<String> = prev
        .findings
        .iter()
        .map(|f| f.file.to_string_lossy().into_owned())
        .filter(|u| Url::parse(u).is_ok_and(|x| x.origin().ascii_serialization() == origin))
        .collect();
    seeds.sort();
    seeds.dedup();
    let o = Options {
        seeds: seeds.clone(),
        max_pages: Some(seeds.len().max(1)),
        vitals: opts.vitals && prev.findings.iter().any(|f| f.rule.starts_with("seo.cwv-")),
        ..opts.clone()
    };
    let cur = check(&crawl(&o, progress).await?);
    let scoped = |f: &&Finding| {
        seeds.iter().any(|u| f.file.to_string_lossy() == u.as_str())
            && !SITE_WIDE.contains(&f.rule.as_str())
    };
    let key = |f: &Finding| (f.file.clone(), f.rule.clone(), f.message.clone());
    let now: HashSet<_> = cur.findings.iter().filter(scoped).map(key).collect();
    let before: HashSet<_> = prev.findings.iter().filter(scoped).map(key).collect();
    Ok(Verified {
        fixed: prev
            .findings
            .iter()
            .filter(scoped)
            .filter(|f| !now.contains(&key(f)))
            .cloned()
            .collect(),
        remaining: prev
            .findings
            .iter()
            .filter(scoped)
            .filter(|f| now.contains(&key(f)))
            .cloned()
            .collect(),
        new: cur
            .findings
            .iter()
            .filter(scoped)
            .filter(|f| !before.contains(&key(f)))
            .cloned()
            .collect(),
        urls: seeds.len(),
    })
}

/// Crawl and check.
pub async fn audit(
    opts: &Options,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<(Crawl, Report), String> {
    let c = crawl(opts, progress).await?;
    let r = check(&c);
    Ok((c, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sitemap_parse_plain_and_gzip() {
        let xml = b"<urlset><url><loc> https://x.test/a?b=1&amp;c=2 </loc></url><url><loc><![CDATA[https://x.test/b]]></loc></url></urlset>";
        let (idx, locs) = parse_sitemap(xml);
        assert!(!idx);
        assert_eq!(locs, ["https://x.test/a?b=1&c=2", "https://x.test/b"]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(
            &mut gz,
            b"<sitemapindex><sitemap><loc>https://x.test/s1.xml</loc></sitemap></sitemapindex>",
        )
        .unwrap();
        let (idx, locs) = parse_sitemap(&gz.finish().unwrap());
        assert!(idx);
        assert_eq!(locs, ["https://x.test/s1.xml"]);
    }

    async fn fixture(s: &wiremock::MockServer, home: &str) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let u = s.uri();
        let html = |b: &str| ResponseTemplate::new(200).set_body_raw(b.to_owned(), "text/html");
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "User-agent: *
Disallow: /admin
Sitemap: {u}/sitemap.xml
"
            )))
            .mount(s)
            .await;
        Mock::given(method("GET")).and(path("/sitemap.xml")).respond_with(ResponseTemplate::new(200).set_body_string(format!("<urlset><url><loc>{u}/</loc></url><url><loc>{u}/yetim</loc></url><url><loc>{u}/admin</loc></url></urlset>"))).mount(s).await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(html(home))
            .mount(s)
            .await;
        Mock::given(method("GET"))
            .and(path("/kirik"))
            .respond_with(ResponseTemplate::new(404))
            .mount(s)
            .await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "/b"))
            .mount(s)
            .await;
        Mock::given(method("GET"))
            .and(path("/b"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/c"))
            .mount(s)
            .await;
        Mock::given(method("GET")).and(path("/c")).respond_with(html("<html lang=tr><head><title>C sayfası için yeterince uzun bir başlık</title><meta name=viewport content=x><meta name=description content=\"Kısa\"></head><body><h1>C</h1></body></html>")).mount(s).await;
        Mock::given(method("GET"))
            .and(path("/yetim"))
            .respond_with(html(
                "<html><head><title>Yetim</title></head><body><h1>Y</h1></body></html>",
            ))
            .mount(s)
            .await;
        Mock::given(method("GET"))
            .and(path("/admin"))
            .respond_with(html("secret"))
            .expect(0)
            .mount(s)
            .await;
    }

    const HOME: &str = "<!doctype html>
<html lang=\"tr\">
<head>
<title>Ana</title>
<link rel=canonical href=\"/\">
        <script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"@type\":\"Product\",\"name\":\"X\"}</script>
</head>
        <body>
<h1>Bir</h1><h1>İki</h1>
<h4>atlama</h4>
<img src=\"http://cdn.test/a.png\">
<a href=\"/kirik\">k</a>
<a href=\"/a\">a</a>
<a href=\"/admin\">adm</a>
</body></html>";

    #[tokio::test]
    async fn crawl_checks_fixture_site_then_verifies_fix() {
        let s = wiremock::MockServer::start().await;
        fixture(&s, HOME).await;
        let opts = Options {
            start: format!("{}/", s.uri()),
            depth: Depth::Full,
            delay: Duration::ZERO,
            vitals: false,
            ..Options::default()
        };
        let (c, r) = audit(&opts, &|_| {}).await.unwrap();
        let at = |rule: &str| {
            r.findings
                .iter()
                .find(|f| f.rule == rule)
                .unwrap_or_else(|| panic!("{rule} missing: {:#?}", r.findings))
        };
        assert_eq!(at("seo.title-length").line, 4);
        assert_eq!(at("seo.h1-multiple").line, 9);
        assert_eq!(at("seo.heading-skip").line, 10);
        assert_eq!(at("seo.img-alt-missing").line, 11);
        assert_eq!(at("seo.broken-link").line, 12);
        assert!(at("seo.broken-link").file.to_string_lossy().ends_with('/'));
        assert_eq!(at("seo.richresult-required").line, 6);
        assert!(at("seo.redirect-chain").message.contains("→301"));
        assert!(
            r.findings
                .iter()
                .any(|f| f.rule == "seo.orphan" && f.file.to_string_lossy().ends_with("/yetim"))
        );
        assert!(
            r.findings
                .iter()
                .any(|f| f.rule == "seo.sitemap-disallowed")
        );
        assert!(r.findings.iter().any(|f| f.rule == "seo.viewport-missing"));
        assert!(c.notes.iter().any(|n| n.contains("robots.txt")));
        assert!(
            c.get(&format!("{}/admin", s.uri()))
                .unwrap()
                .blocked_by_robots
        );
        let rich = &c.get(&format!("{}/", s.uri())).unwrap().rich;
        assert_eq!(rich.blocked[0].0, "Product");
        // HTML report escapes page content.
        let h = to_html(
            &r,
            "T <x>",
            Some("Ajans"),
            ["T", "a", "b", "c", "d", "e", "f", "g"],
        );
        assert!(h.contains("T &lt;x&gt;") && h.contains("Ajans"));

        // The site is fixed: one h1, alt text; verify re-fetches only pages with findings.
        let s2 = wiremock::MockServer::start().await;
        let fixed = HOME
            .replace("<h1>İki</h1>", "")
            .replace("<img src=", "<img alt=\"x\" src=")
            .replace(&s.uri(), &s2.uri());
        fixture(&s2, &fixed).await;
        let moved = Report {
            findings: r
                .findings
                .iter()
                .map(|f| Finding {
                    file: PathBuf::from(f.file.to_string_lossy().replace(&s.uri(), &s2.uri())),
                    ..f.clone()
                })
                .collect(),
            ..r.clone()
        };
        let v = verify(
            &moved,
            &Options {
                start: format!("{}/", s2.uri()),
                ..opts.clone()
            },
            &|_| {},
        )
        .await
        .unwrap();
        assert!(v.fixed.iter().any(|f| f.rule == "seo.h1-multiple"));
        assert!(v.fixed.iter().any(|f| f.rule == "seo.img-alt-missing"));
        assert!(v.remaining.iter().any(|f| f.rule == "seo.title-length"));
        assert!(!v.remaining.iter().any(|f| f.rule == "seo.orphan"));
    }

    #[test]
    fn turkish_slug() {
        assert_eq!(tr_slug("Çok Güzel Şişe Ürünü"), "cok-guzel-sise-urunu");
        assert_eq!(tr_slug("kategori/IŞIK"), "kategori/isik");
        assert!(hreflang_ok("tr-TR") && hreflang_ok("x-default") && !hreflang_ok("turkish"));
    }

    #[test]
    fn diff_new_and_fixed() {
        let f = |r: &str| Finding {
            rule: r.into(),
            file: "u".into(),
            line: 1,
            ..Finding::default()
        };
        let a = Report {
            tool: "t".into(),
            findings: vec![f("a"), f("b")],
            notes: vec![],
        };
        let b = Report {
            tool: "t".into(),
            findings: vec![f("b"), f("c")],
            notes: vec![],
        };
        let d = diff(&a, &b);
        assert_eq!(d.new[0].rule, "c");
        assert_eq!(d.fixed[0].rule, "a");
    }
}
