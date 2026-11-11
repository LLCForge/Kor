//! Redirect map for migrations and restructures (§29.11).
//!
//! 1. Old and new URL lists from sitemaps (index files followed), a crawl of the site, a text/CSV
//!    file (Search Console export, one URL per line) or an access log (paths that answered 200).
//! 2. Matching: same path, same last slug (Turkish letters folded), path-word similarity, and
//!    optionally page titles; every row says how it was matched and how sure it is.
//! 3. The table goes to CSV for the user's corrections; corrected CSVs are read back.
//! 4. Output: Apache `.htaccess`, nginx `map`, Cloudflare Bulk Redirects CSV, WordPress Redirection
//!    plugin CSV; chains and loops in the map are found (and can be collapsed) before going live.
//! 5. After launch, every old URL is requested: final status, hops, 404s, chains and loops.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::Result;
use super::http::Http;

pub const MAX_URLS: usize = 50_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub old: String,
    /// Empty when nothing matched (the user fills it in, or the old URL stays 404/410).
    pub new: String,
    pub score: f64,
    /// `exact`, `slug`, `words`, `title`, `manual`, `none`.
    pub method: String,
}

/// Path plus query of a URL (or a bare path), without fragment, trailing slash folded.
pub fn path_of(u: &str) -> Option<String> {
    let u = u.trim();
    if u.is_empty() || u.starts_with('#') {
        return None;
    }
    let (path, query) = if u.starts_with('/') {
        let (p, q) = u.split_once('?').map_or((u, None), |(p, q)| (p, Some(q)));
        (p.split('#').next().unwrap_or(p).to_owned(), q.map(|q| q.split('#').next().unwrap_or(q).to_owned()))
    } else {
        let parsed = url::Url::parse(u).ok()?;
        (parsed.path().to_owned(), parsed.query().map(str::to_owned))
    };
    let mut p = if path.len() > 1 { path.trim_end_matches('/').to_owned() } else { path };
    if p.is_empty() {
        p = "/".into();
    }
    Some(match query.filter(|q| !q.is_empty()) {
        Some(q) => format!("{p}?{q}"),
        None => p,
    })
}

/// Lower-case words of a path, Turkish letters folded, file extensions and numbers-only dropped.
pub fn words(path: &str) -> BTreeSet<String> {
    let p = path.split('?').next().unwrap_or(path);
    let p = percent_decode(p);
    super::tr::slug(&p.replace(['/', '.', '_'], "-"))
        .split('-')
        .filter(|w| w.len() > 1 && !matches!(*w, "html" | "htm" | "php" | "aspx" | "index") && !w.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned)
        .collect()
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map_or_else(|| s.to_owned(), |(_, v)| v.into_owned())
}

fn last_slug(path: &str) -> String {
    let p = path.split('?').next().unwrap_or(path);
    let seg = p.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    let seg = seg.rsplit_once('.').map_or(seg, |(a, _)| a);
    super::tr::slug(&percent_decode(seg))
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let j = a.intersection(b).count() as f64 / a.union(b).count() as f64;
    j
}

/// Match old paths to new paths. `titles`: path → page title for both sites (optional).
pub fn build<S: std::hash::BuildHasher>(old: &[String], new: &[String], titles: &HashMap<String, String, S>, min: f64) -> Vec<Row> {
    let new_set: HashSet<&str> = new.iter().map(String::as_str).collect();
    let by_slug: HashMap<String, Vec<&str>> = new.iter().fold(HashMap::new(), |mut m, n| {
        m.entry(last_slug(n)).or_default().push(n.as_str());
        m
    });
    let new_words: Vec<(&str, BTreeSet<String>)> = new.iter().map(|n| (n.as_str(), words(n))).collect();
    let mut rows = Vec::new();
    for o in old {
        if new_set.contains(o.as_str()) {
            continue; // same URL on the new site: no redirect needed
        }
        let slug = last_slug(o);
        if let Some(c) = by_slug.get(&slug).filter(|c| !slug.is_empty() && c.len() == 1) {
            rows.push(Row { old: o.clone(), new: c[0].to_owned(), score: 0.9, method: "slug".into() });
            continue;
        }
        let ow = words(o);
        let best = new_words
            .iter()
            .map(|(n, w)| (*n, jaccard(&ow, w)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let by_title = titles.get(o).and_then(|t| {
            let tw = words(t);
            new.iter()
                .filter_map(|n| titles.get(n).map(|nt| (n.as_str(), jaccard(&tw, &words(nt)))))
                .max_by(|a, b| a.1.total_cmp(&b.1))
        });
        let row = match (best, by_title) {
            (_, Some((n, s))) if s >= min.max(0.6) => Row { old: o.clone(), new: n.to_owned(), score: (s * 100.0).round() / 100.0, method: "title".into() },
            (Some((n, s)), _) if s >= min => Row { old: o.clone(), new: n.to_owned(), score: (s * 100.0).round() / 100.0, method: "words".into() },
            _ => Row { old: o.clone(), new: String::new(), score: 0.0, method: "none".into() },
        };
        rows.push(row);
    }
    rows
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// One CSV line → fields (quotes, doubled quotes).
pub fn csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub fn to_csv(rows: &[Row]) -> String {
    let mut o = "old,new,score,method\n".to_owned();
    for r in rows {
        let _ = writeln!(o, "{},{},{:.2},{}", csv_field(&r.old), csv_field(&r.new), r.score, r.method);
    }
    o
}

/// Read a (possibly user-corrected) map: a changed `new` becomes `manual`.
pub fn from_csv(text: &str) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 && line.starts_with("old,") || line.trim().is_empty() {
            continue;
        }
        let f = csv_line(line);
        let old = path_of(f.first().map_or("", String::as_str)).ok_or_else(|| format!("line {}: no old URL", i + 1))?;
        let new = f.get(1).map(|s| s.trim().to_owned()).unwrap_or_default();
        let new = if new.starts_with("http") { new } else { path_of(&new).unwrap_or_default() };
        let score = f.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(1.0);
        let method = f.get(3).map_or("manual", |s| s.trim()).to_owned();
        rows.push(Row { old, new, score, method: if method.is_empty() { "manual".into() } else { method } });
    }
    Ok(rows)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MapIssues {
    /// old → … → final, length ≥ 3.
    pub chains: Vec<Vec<String>>,
    pub loops: Vec<Vec<String>>,
    pub unmatched: Vec<String>,
}

/// Chains (a → b where b is itself redirected) and loops inside the map.
pub fn issues(rows: &[Row]) -> MapIssues {
    let m: BTreeMap<&str, &str> = rows.iter().filter(|r| !r.new.is_empty()).map(|r| (r.old.as_str(), r.new.as_str())).collect();
    let mut out = MapIssues { unmatched: rows.iter().filter(|r| r.new.is_empty()).map(|r| r.old.clone()).collect(), ..MapIssues::default() };
    for start in m.keys() {
        let mut path = vec![(*start).to_owned()];
        let mut cur = *start;
        while let Some(next) = m.get(cur) {
            if path.iter().any(|p| p == next) {
                path.push((*next).to_owned());
                out.loops.push(path.clone());
                break;
            }
            path.push((*next).to_owned());
            cur = next;
        }
        if path.len() >= 3 && !out.loops.iter().any(|l| l[0] == path[0]) {
            out.chains.push(path);
        }
    }
    out
}

/// Point every old URL straight at the end of its chain (loops are left out).
pub fn collapse(rows: &[Row]) -> Vec<Row> {
    let iss = issues(rows);
    let looped: HashSet<&str> = iss.loops.iter().flatten().map(String::as_str).collect();
    let end: HashMap<&str, &str> = iss.chains.iter().filter_map(|c| Some((c.first()?.as_str(), c.last()?.as_str()))).collect();
    rows.iter()
        .filter(|r| !looped.contains(r.old.as_str()))
        .map(|r| match end.get(r.old.as_str()) {
            Some(e) => Row { new: (*e).to_owned(), ..r.clone() },
            None => r.clone(),
        })
        .collect()
}

/// Server snippets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Apache,
    Nginx,
    Cloudflare,
    Redirection,
    Csv,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "apache" | "htaccess" => Self::Apache,
            "nginx" => Self::Nginx,
            "cloudflare" => Self::Cloudflare,
            "redirection" | "wordpress" | "wp" => Self::Redirection,
            "csv" => Self::Csv,
            _ => return None,
        })
    }
}

/// A path for an Apache regex.
fn re_escape(s: &str) -> String {
    regex::escape(s)
}

/// `host` is the old domain (Cloudflare needs it).
pub fn render(rows: &[Row], fmt: Format, host: &str) -> String {
    let rows: Vec<&Row> = rows.iter().filter(|r| !r.new.is_empty() && r.new != r.old).collect();
    let mut o = String::new();
    match fmt {
        Format::Csv => return to_csv(&rows.into_iter().cloned().collect::<Vec<_>>()),
        Format::Apache => {
            o.push_str("# Kor redirect map (301)\n<IfModule mod_rewrite.c>\nRewriteEngine On\n");
            for r in rows {
                let (p, q) = r.old.split_once('?').map_or((r.old.as_str(), None), |(p, q)| (p, Some(q)));
                if let Some(q) = q {
                    let _ = writeln!(o, "RewriteCond %{{QUERY_STRING}} ^{}$", re_escape(q));
                }
                let target = if q.is_some() && !r.new.contains('?') { format!("{}?", r.new) } else { r.new.clone() };
                let _ = writeln!(o, "RewriteRule ^{}/?$ {} [R=301,L]", re_escape(p.trim_start_matches('/')), target.replace(' ', "%20"));
            }
            o.push_str("</IfModule>\n");
        }
        Format::Nginx => {
            o.push_str("# Kor redirect map (301). Put the map in the http block, the if in the server block.\nmap $request_uri $kor_redirect {\n    default \"\";\n");
            for r in rows {
                let _ = writeln!(o, "    \"{}\" \"{}\";", r.old.replace('"', "\\\""), r.new.replace('"', "\\\""));
                if !r.old.contains('?') && r.old != "/" {
                    let _ = writeln!(o, "    \"{}/\" \"{}\";", r.old.replace('"', "\\\""), r.new.replace('"', "\\\""));
                }
            }
            o.push_str("}\n\n# server { …\nif ($kor_redirect) {\n    return 301 $kor_redirect;\n}\n");
        }
        Format::Cloudflare => {
            o.push_str("source_url,target_url,status_code,preserve_query_string,include_subdomains,subpath_matching,preserve_path_suffix\n");
            for r in rows {
                let target = if r.new.starts_with("http") { r.new.clone() } else { format!("https://{host}{}", r.new) };
                let _ = writeln!(o, "{},{},301,false,false,false,false", csv_field(&format!("{host}{}", r.old)), csv_field(&target));
            }
        }
        Format::Redirection => {
            o.push_str("source,target,regex,code\n");
            for r in rows {
                let _ = writeln!(o, "{},{},0,301", csv_field(&r.old), csv_field(&r.new));
            }
        }
    }
    o
}

static LOC: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)<loc>\s*(.*?)\s*</loc>").unwrap_or_else(|_| unreachable!()));
static HREF: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r#"(?i)href\s*=\s*["']([^"'#]+)"#).unwrap_or_else(|_| unreachable!()));
static TITLE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap_or_else(|_| unreachable!()));

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

/// URLs in a sitemap; sitemap indexes are followed (at most 200 child sitemaps).
pub async fn sitemap(http: &Http, url: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut queue = VecDeque::from([url.to_owned()]);
    let mut seen = 0;
    while let Some(u) = queue.pop_front() {
        seen += 1;
        if seen > 200 {
            break;
        }
        let (_, r) = http.follow(&u).await?;
        if !r.ok() {
            return Err(format!("{u}: HTTP {}", r.status));
        }
        let index = r.body.contains("<sitemapindex");
        for c in LOC.captures_iter(&r.body) {
            let loc = unescape(&c[1]);
            if index {
                queue.push_back(loc);
            } else if out.len() < MAX_URLS {
                out.push(loc);
            }
        }
    }
    Ok(out)
}

/// Same-host pages reachable from `start` (HTML only), at most `limit`; also returns their titles.
pub async fn crawl(http: &Http, start: &str, limit: usize) -> Result<(Vec<String>, HashMap<String, String>)> {
    let base = url::Url::parse(start).map_err(|e| e.to_string())?;
    let host = base.host_str().unwrap_or_default().to_owned();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue = VecDeque::from([base.to_string()]);
    let mut pages = Vec::new();
    let mut titles = HashMap::new();
    while let Some(u) = queue.pop_front() {
        if pages.len() >= limit {
            break;
        }
        if !seen.insert(u.clone()) {
            continue;
        }
        let Ok((_, r)) = http.follow(&u).await else { continue };
        let html = r.header("content-type").is_none_or(|c| c.contains("html"));
        if !r.ok() || !html {
            continue;
        }
        let Ok(final_url) = url::Url::parse(&r.url) else { continue };
        if final_url.host_str() != Some(host.as_str()) {
            continue;
        }
        if let Some(p) = path_of(&u) {
            if let Some(t) = TITLE.captures(&r.body) {
                titles.insert(p.clone(), unescape(t[1].trim()));
            }
            pages.push(p);
        }
        for c in HREF.captures_iter(&r.body) {
            let Ok(next) = final_url.join(&unescape(&c[1])) else { continue };
            let skip = next.host_str() != Some(host.as_str())
                || !matches!(next.scheme(), "http" | "https")
                || [".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg", ".pdf", ".zip", ".css", ".js", ".ico", ".xml", ".mp4"]
                    .iter()
                    .any(|e| next.path().to_ascii_lowercase().ends_with(e));
            if !skip {
                let mut n = next.clone();
                n.set_fragment(None);
                if !seen.contains(n.as_str()) {
                    queue.push_back(n.to_string());
                }
            }
        }
    }
    Ok((pages, titles))
}

/// URLs from a text or CSV file (first column), or GET paths that answered 200 in an access log
/// (common/combined format).
pub fn from_file(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let log = text.lines().take(5).any(|l| l.contains("\"GET "));
    let mut out = Vec::new();
    for line in text.lines() {
        let u = if log {
            let Some(req) = line.split('"').nth(1) else { continue };
            let mut w = req.split_whitespace();
            let (Some("GET"), Some(p)) = (w.next(), w.next()) else { continue };
            let status = line.split('"').nth(2).and_then(|s| s.split_whitespace().next());
            if status != Some("200") {
                continue;
            }
            p.to_owned()
        } else {
            csv_line(line).into_iter().next().unwrap_or_default()
        };
        if u.starts_with('/') || u.starts_with("http") {
            out.push(u);
        }
    }
    Ok(out)
}

/// Collect URLs from `source`: `sitemap:<url>`, `crawl:<url>`, a URL ending in `.xml` (sitemap),
/// another URL (crawl), or a file. Returns paths and titles (crawl only).
pub async fn collect(http: &Http, source: &str, limit: usize) -> Result<(Vec<String>, HashMap<String, String>)> {
    let (urls, titles) = if let Some(u) = source.strip_prefix("sitemap:") {
        (sitemap(http, u).await?, HashMap::new())
    } else if let Some(u) = source.strip_prefix("crawl:") {
        crawl(http, u, limit).await?
    } else if source.starts_with("http") {
        if std::path::Path::new(source.split('?').next().unwrap_or(source))
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
        {
            (sitemap(http, source).await?, HashMap::new())
        } else {
            crawl(http, source, limit).await?
        }
    } else {
        (from_file(Path::new(source))?, HashMap::new())
    };
    let mut paths: Vec<String> = urls.iter().filter_map(|u| path_of(u)).collect();
    paths.sort();
    paths.dedup();
    Ok((paths, titles))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checked {
    pub old: String,
    pub expected: String,
    pub final_url: String,
    pub status: u16,
    pub hops: usize,
    /// `ok`, `404`, `chain`, `loop`, `wrong-target`, `not-redirected`, `error`.
    pub verdict: String,
}

/// Request every old URL on the live site (`base` = `https://example.com`) and judge the answer.
pub async fn verify(http: &Http, base: &str, rows: &[Row]) -> Vec<Checked> {
    let base = base.trim_end_matches('/');
    let mut out = Vec::new();
    for r in rows {
        let url = format!("{base}{}", r.old);
        let c = match http.follow(&url).await {
            Ok((hops, resp)) => {
                let final_path = path_of(&resp.url).unwrap_or_default();
                let expected = if r.new.starts_with("http") { path_of(&r.new).unwrap_or_default() } else { r.new.clone() };
                let looped = hops.iter().any(|h| h.url == resp.url);
                let verdict = if looped {
                    "loop"
                } else if resp.status == 404 || resp.status == 410 {
                    "404"
                } else if hops.is_empty() && !r.new.is_empty() {
                    "not-redirected"
                } else if !expected.is_empty() && final_path != expected {
                    "wrong-target"
                } else if hops.len() > 1 {
                    "chain"
                } else if resp.status >= 400 {
                    "error"
                } else {
                    "ok"
                };
                Checked { old: r.old.clone(), expected: r.new.clone(), final_url: resp.url, status: resp.status, hops: hops.len(), verdict: verdict.into() }
            }
            Err(e) => Checked { old: r.old.clone(), expected: r.new.clone(), final_url: e, verdict: "error".into(), ..Checked::default() },
        };
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn matching() {
        let old = s(&["/urunler/kirmizi-ayakkabi.html", "/hakkimizda", "/blog/2019/05/yaz-indirimi", "/iletisim", "/eski-sayfa-x"]);
        let new = s(&["/magaza/kirmizi-ayakkabi", "/kurumsal/hakkimizda", "/blog/yaz-indirimi-kampanyasi", "/iletisim"]);
        let rows = build(&old, &new, &HashMap::new(), 0.3);
        let get = |o: &str| rows.iter().find(|r| r.old == o).unwrap();
        assert_eq!(get("/urunler/kirmizi-ayakkabi.html").new, "/magaza/kirmizi-ayakkabi");
        assert_eq!(get("/hakkimizda").method, "slug");
        assert_eq!(get("/blog/2019/05/yaz-indirimi").method, "words");
        assert!(rows.iter().all(|r| r.old != "/iletisim"));
        assert_eq!(get("/eski-sayfa-x").method, "none");
        assert!(words("/ürünler/Çanta").contains("urunler"));
    }

    #[test]
    fn chains_loops_and_collapse() {
        let r = |o: &str, n: &str| Row { old: o.into(), new: n.into(), score: 1.0, method: "manual".into() };
        let rows = vec![r("/a", "/b"), r("/b", "/c"), r("/x", "/y"), r("/y", "/x"), r("/z", "")];
        let i = issues(&rows);
        assert_eq!(i.chains, vec![s(&["/a", "/b", "/c"])]);
        assert_eq!(i.loops.len(), 2);
        assert_eq!(i.unmatched, ["/z"]);
        let c = collapse(&rows);
        assert_eq!(c.iter().find(|x| x.old == "/a").unwrap().new, "/c");
        assert!(!c.iter().any(|x| x.old == "/x"));
    }

    #[test]
    fn snippets_and_csv_round_trip() {
        let rows = vec![
            Row { old: "/eski?id=5".into(), new: "/yeni".into(), score: 1.0, method: "manual".into() },
            Row { old: "/hakkimizda".into(), new: "/kurumsal".into(), score: 0.9, method: "slug".into() },
        ];
        let ap = render(&rows, Format::Apache, "a.com");
        assert!(ap.contains("RewriteCond %{QUERY_STRING} ^id=5$"));
        assert!(ap.contains("RewriteRule ^hakkimizda/?$ /kurumsal [R=301,L]"));
        assert!(render(&rows, Format::Nginx, "a.com").contains("\"/hakkimizda/\" \"/kurumsal\";"));
        assert!(render(&rows, Format::Cloudflare, "a.com").contains("a.com/hakkimizda,https://a.com/kurumsal,301"));
        assert!(render(&rows, Format::Redirection, "a.com").contains("/hakkimizda,/kurumsal,0,301"));
        let back = from_csv(&to_csv(&rows)).unwrap();
        assert_eq!(back, rows);
        assert_eq!(csv_line(r#""a,b","c""d",e"#), ["a,b", "c\"d", "e"]);
    }

    #[test]
    fn access_logs() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("access.log");
        std::fs::write(&f, "1.2.3.4 - - [10/Nov/2026:10:00:00 +0300] \"GET /urun/1 HTTP/1.1\" 200 512 \"-\" \"UA\"\n1.2.3.4 - - [x] \"GET /yok HTTP/1.1\" 404 0 \"-\" \"UA\"\n").unwrap();
        assert_eq!(from_file(&f).unwrap(), ["/urun/1"]);
    }

    #[tokio::test]
    async fn fixture_sites_crawl_sitemap_and_verify() {
        let old = MockServer::start().await;
        Mock::given(path("/sitemap.xml")).respond_with(ResponseTemplate::new(200).set_body_string(format!(
            "<sitemapindex><sitemap><loc>{0}/s1.xml</loc></sitemap></sitemapindex>", old.uri()))).mount(&old).await;
        Mock::given(path("/s1.xml")).respond_with(ResponseTemplate::new(200).set_body_string(format!(
            "<urlset><url><loc>{0}/urunler/canta</loc></url><url><loc>{0}/iletisim</loc></url></urlset>", old.uri()))).mount(&old).await;
        let new = MockServer::start().await;
        let html = |body: &str| ResponseTemplate::new(200).set_body_raw(body.to_owned(), "text/html");
        Mock::given(path("/")).respond_with(html("<title>Ana</title><a href=\"/magaza/canta\">x</a><a href=\"/iletisim\">y</a><a href=\"https://other.example/\">z</a>")).mount(&new).await;
        Mock::given(path("/magaza/canta")).respond_with(html("<title>Çanta</title>")).mount(&new).await;
        Mock::given(path("/iletisim")).respond_with(html("<title>İletişim</title>")).mount(&new).await;
        Mock::given(path("/urunler/canta")).respond_with(ResponseTemplate::new(301).insert_header("location", "/magaza/canta")).mount(&new).await;
        let http = Http::new().unwrap();
        let (o, _) = collect(&http, &format!("{}/sitemap.xml", old.uri()), 100).await.unwrap();
        assert_eq!(o, ["/iletisim", "/urunler/canta"]);
        let (n, titles) = collect(&http, &format!("crawl:{}/", new.uri()), 100).await.unwrap();
        assert_eq!(n, ["/", "/iletisim", "/magaza/canta"]);
        assert_eq!(titles.get("/magaza/canta").map(String::as_str), Some("Çanta"));
        let rows = build(&o, &n, &titles, 0.3);
        assert_eq!(rows.len(), 1);
        let checked = verify(&http, &new.uri(), &rows).await;
        assert_eq!(checked[0].verdict, "ok", "{checked:?}");
    }
}
