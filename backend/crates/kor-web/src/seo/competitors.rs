//! Competitor watch (spec §29.10): snapshots of competitor pages (status, title, description,
//! headings, word count, schema types, product price) and of their sitemap URL set, diffed
//! against the previous snapshot. Ethical by construction: robots.txt obeyed, rate limited,
//! `kor/<version>` user agent, page and sitemap caps. Scheduling uses Kor's routines
//! (`kor seo competitors schedule daily|weekly`), which run `kor seo competitors run`.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::fetch::{Chain, Fetcher};

/// Pages per competitor and sitemap URLs kept per snapshot.
pub const MAX_PAGES: usize = 50;
pub const MAX_SITEMAP: usize = 5_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Competitor {
    pub name: String,
    pub urls: Vec<String>,
    /// Also track the site's sitemap URL set (new / removed pages).
    #[serde(default = "yes")]
    pub sitemap: bool,
    /// Our page for the same topic: gaps are reported against it.
    #[serde(default)]
    pub ours: String,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, rename = "competitor")]
    pub competitors: Vec<Competitor>,
}

pub fn config_path(root: &Path) -> PathBuf {
    root.join(".kor").join("seo").join("competitors.toml")
}

pub fn load(root: &Path) -> Result<Config, String> {
    match std::fs::read_to_string(config_path(root)) {
        Ok(t) => toml::from_str(&t).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn save(root: &Path, c: &Config) -> Result<(), String> {
    let p = config_path(root);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, toml::to_string_pretty(c).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Add URLs to a competitor (created when new).
pub fn add(root: &Path, name: &str, urls: &[String], ours: Option<&str>) -> Result<Config, String> {
    if !valid_name(name) {
        return Err("name: letters, digits, - and _ (max 64)".into());
    }
    for u in urls {
        let p = url::Url::parse(u).map_err(|e| format!("{u}: {e}"))?;
        if !matches!(p.scheme(), "http" | "https") {
            return Err(format!("{u}: not http(s)"));
        }
    }
    let mut c = load(root)?;
    if !c.competitors.iter().any(|x| x.name == name) {
        c.competitors.push(Competitor {
            name: name.into(),
            sitemap: true,
            ..Competitor::default()
        });
    }
    let comp = c
        .competitors
        .iter_mut()
        .find(|x| x.name == name)
        .expect("inserted");
    for u in urls {
        if !comp.urls.contains(u) && comp.urls.len() < MAX_PAGES {
            comp.urls.push(u.clone());
        }
    }
    if let Some(o) = ours {
        comp.ours = o.to_owned();
    }
    save(root, &c)?;
    Ok(c)
}

pub fn remove(root: &Path, name: &str) -> Result<bool, String> {
    let mut c = load(root)?;
    let before = c.competitors.len();
    c.competitors.retain(|x| x.name != name);
    let removed = c.competitors.len() != before;
    if removed {
        save(root, &c)?;
    }
    Ok(removed)
}

/// What Kor keeps of one page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageSnap {
    pub url: String,
    pub status: u16,
    pub title: String,
    pub description: String,
    pub h1: String,
    pub headings: Vec<String>,
    pub word_count: usize,
    pub schema: Vec<String>,
    /// `"<price> <currency>"` from Product JSON-LD offers.
    pub price: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub taken: String,
    pub pages: Vec<PageSnap>,
    pub sitemap: Vec<String>,
    #[serde(default)]
    pub ours: Option<PageSnap>,
}

fn walk(v: &Value, types: &mut BTreeSet<String>, price: &mut String) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| walk(x, types, price)),
        Value::Object(o) => {
            match o.get("@type") {
                Some(Value::String(t)) => {
                    types.insert(t.clone());
                }
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).for_each(|t| {
                    types.insert(t.to_owned());
                }),
                _ => {}
            }
            if price.is_empty()
                && let Some(off) = o.get("offers")
            {
                let off = if let Value::Array(a) = off {
                    a.first().unwrap_or(&Value::Null)
                } else {
                    off
                };
                let p = off
                    .get("price")
                    .or_else(|| off.get("lowPrice"))
                    .map(|p| p.as_str().map_or_else(|| p.to_string(), str::to_owned));
                if let Some(p) = p {
                    *price = format!(
                        "{p} {}",
                        off.get("priceCurrency")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    )
                    .trim()
                    .to_owned();
                }
            }
            o.values().for_each(|x| walk(x, types, price));
        }
        _ => {}
    }
}

fn schema_and_price(jsonld: &[super::html::At<String>]) -> (Vec<String>, String) {
    let mut types = BTreeSet::new();
    let mut price = String::new();
    for j in jsonld {
        if let Ok(v) = serde_json::from_str::<Value>(&j.value) {
            walk(&v, &mut types, &mut price);
        }
    }
    (types.into_iter().collect(), price)
}

async fn snap_page(f: &Fetcher, u: &str) -> PageSnap {
    let mut s = PageSnap {
        url: u.to_owned(),
        ..PageSnap::default()
    };
    let Ok(url) = url::Url::parse(u) else {
        s.error = "bad URL".into();
        return s;
    };
    if !f.allowed(&url).await {
        s.error = "disallowed by robots.txt".into();
        return s;
    }
    match f.follow(&url).await {
        Ok((_, Chain::Done(r))) => {
            s.status = r.status;
            if r.status == 200 && r.is_html() {
                let p = super::html::parse(&r.body);
                s.title = p
                    .titles
                    .first()
                    .map(|t| t.value.clone())
                    .unwrap_or_default();
                s.description = p
                    .descriptions
                    .first()
                    .map(|t| t.value.clone())
                    .unwrap_or_default();
                s.h1 = p
                    .headings
                    .iter()
                    .find(|h| h.value.0 == 1)
                    .map(|h| h.value.1.clone())
                    .unwrap_or_default();
                s.headings = p
                    .headings
                    .iter()
                    .filter(|h| h.value.0 > 1 && h.value.0 <= 3)
                    .map(|h| h.value.1.clone())
                    .take(50)
                    .collect();
                s.word_count = p.word_count;
                (s.schema, s.price) = schema_and_price(&p.jsonld);
            }
        }
        Ok(_) => s.error = "redirect loop".into(),
        Err(e) => s.error = e,
    }
    s
}

/// Take a snapshot of one competitor.
pub async fn snapshot(c: &Competitor, user_agent: &str) -> Result<Snapshot, String> {
    let f = Fetcher::new(user_agent, Duration::from_secs(2), true)?;
    let mut s = Snapshot {
        name: c.name.clone(),
        taken: chrono::Utc::now().to_rfc3339(),
        ..Snapshot::default()
    };
    for u in c.urls.iter().take(MAX_PAGES) {
        s.pages.push(snap_page(&f, u).await);
    }
    if c.sitemap
        && let Some(first) = c.urls.first().and_then(|u| url::Url::parse(u).ok())
    {
        let robots = f.robots(&first).await;
        let mut seeds = robots.sitemaps.clone();
        if seeds.is_empty() {
            seeds.push(format!(
                "{}/sitemap.xml",
                first.origin().ascii_serialization()
            ));
        }
        let mut queue = seeds;
        let mut seen = HashSet::new();
        while let Some(sm) = queue.pop() {
            if !seen.insert(sm.clone()) || seen.len() > 20 || s.sitemap.len() >= MAX_SITEMAP {
                continue;
            }
            let Ok(u) = url::Url::parse(&sm) else {
                continue;
            };
            if let Ok(r) = f.get(&u).await
                && r.status == 200
            {
                let (index, locs) = super::audit::parse_sitemap(&r.bytes);
                if index {
                    queue.extend(locs);
                } else {
                    s.sitemap.extend(locs);
                }
            }
        }
        s.sitemap.sort();
        s.sitemap.dedup();
        s.sitemap.truncate(MAX_SITEMAP);
    }
    if !c.ours.is_empty() {
        s.ours = Some(snap_page(&f, &c.ours).await);
    }
    Ok(s)
}

fn dir(root: &Path, name: &str) -> PathBuf {
    root.join(".kor").join("seo").join("competitors").join(name)
}

/// Save a snapshot; returns the previous one.
pub fn store(root: &Path, s: &Snapshot) -> Result<Option<Snapshot>, String> {
    let d = dir(root, &s.name);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(&d)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let prev = files
        .last()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok());
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    std::fs::write(
        d.join(format!("{ts}.json")),
        serde_json::to_string_pretty(s).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(prev)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub url: String,
    /// `status`, `title`, `description`, `h1`, `headings`, `word_count`, `schema`, `price`.
    pub field: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Diff {
    pub name: String,
    pub new_urls: Vec<String>,
    pub removed_urls: Vec<String>,
    pub changes: Vec<Change>,
    /// "They have it, we do not" (against `ours`): schema types and heading topics.
    pub gaps: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.new_urls.is_empty() && self.removed_urls.is_empty() && self.changes.is_empty()
    }
}

#[allow(clippy::too_many_lines)] // one block per compared field
pub fn diff(prev: Option<&Snapshot>, cur: &Snapshot) -> Diff {
    let mut d = Diff {
        name: cur.name.clone(),
        ..Diff::default()
    };
    if let Some(p) = prev {
        let old: HashSet<&String> = p.sitemap.iter().collect();
        let now: HashSet<&String> = cur.sitemap.iter().collect();
        if !p.sitemap.is_empty() {
            d.new_urls = cur
                .sitemap
                .iter()
                .filter(|u| !old.contains(u))
                .cloned()
                .collect();
            d.removed_urls = p
                .sitemap
                .iter()
                .filter(|u| !now.contains(u))
                .cloned()
                .collect();
        }
        for c in &cur.pages {
            let Some(o) = p.pages.iter().find(|x| x.url == c.url) else {
                continue;
            };
            let mut ch = |field: &str, a: String, b: String| {
                if a != b {
                    d.changes.push(Change {
                        url: c.url.clone(),
                        field: field.into(),
                        before: a,
                        after: b,
                    });
                }
            };
            ch("status", o.status.to_string(), c.status.to_string());
            ch("title", o.title.clone(), c.title.clone());
            ch("description", o.description.clone(), c.description.clone());
            ch("h1", o.h1.clone(), c.h1.clone());
            ch("schema", o.schema.join(", "), c.schema.join(", "));
            ch("price", o.price.clone(), c.price.clone());
            if o.headings != c.headings {
                let added: Vec<&String> = c
                    .headings
                    .iter()
                    .filter(|h| !o.headings.contains(h))
                    .collect();
                let removed: Vec<&String> = o
                    .headings
                    .iter()
                    .filter(|h| !c.headings.contains(h))
                    .collect();
                d.changes.push(Change {
                    url: c.url.clone(),
                    field: "headings".into(),
                    before: format!("-{}", removed.len()),
                    after: format!(
                        "+{}: {}",
                        added.len(),
                        added
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ),
                });
            }
            let (a, b) = (o.word_count, c.word_count);
            if a.abs_diff(b) * 10 >= a.max(1) {
                d.changes.push(Change {
                    url: c.url.clone(),
                    field: "word_count".into(),
                    before: a.to_string(),
                    after: b.to_string(),
                });
            }
        }
    }
    if let Some(ours) = &cur.ours {
        let words = |s: &str| {
            s.to_lowercase()
                .split_whitespace()
                .filter(|w| w.chars().count() > 3)
                .map(str::to_owned)
                .collect::<HashSet<_>>()
        };
        let our_words: HashSet<String> = ours
            .headings
            .iter()
            .chain([&ours.h1, &ours.title])
            .flat_map(|h| words(h))
            .collect();
        for c in &cur.pages {
            for t in c.schema.iter().filter(|t| !ours.schema.contains(t)) {
                let g = format!("schema {t} ({})", c.url);
                if !d.gaps.contains(&g) {
                    d.gaps.push(g);
                }
            }
            for h in &c.headings {
                if !words(h).is_empty() && words(h).is_disjoint(&our_words) {
                    d.gaps.push(format!("topic \"{h}\" ({})", c.url));
                }
            }
            if c.word_count > ours.word_count * 3 / 2 && ours.word_count > 0 {
                d.gaps.push(format!(
                    "length {} vs our {} words ({})",
                    c.word_count, ours.word_count, c.url
                ));
            }
        }
        d.gaps.truncate(30);
    }
    d
}

/// Snapshot every competitor, store, and diff with the previous run.
pub async fn run(
    root: &Path,
    user_agent: &str,
    only: Option<&str>,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Vec<Diff>, String> {
    let cfg = load(root)?;
    let mut out = Vec::new();
    for c in cfg
        .competitors
        .iter()
        .filter(|c| only.is_none_or(|n| n == c.name))
    {
        progress(&c.name);
        let s = snapshot(c, user_agent).await?;
        let prev = store(root, &s)?;
        out.push(diff(prev.as_ref(), &s));
    }
    Ok(out)
}

/// The routine prompt that runs the watch on a schedule (`kor routines` / desktop scheduler).
pub fn routine(root: &Path, every: &str) -> Result<serde_json::Map<String, Value>, String> {
    let minutes = match every {
        "daily" => 24 * 60,
        "weekly" => 7 * 24 * 60,
        _ => return Err("schedule: daily or weekly".into()),
    };
    let mut m = serde_json::Map::new();
    m.insert(
        "name".into(),
        Value::from(format!("SEO competitor watch ({every})")),
    );
    m.insert("prompt".into(), Value::from("Run the shell command `kor seo competitors run` in this folder and summarize what the competitors changed: new or removed pages, title/meta/price/schema changes and the gaps it lists. Do not change any files."));
    m.insert("intervalMinutes".into(), Value::from(minutes));
    m.insert(
        "root".into(),
        Value::from(root.to_string_lossy().into_owned()),
    );
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn page(title: &str, price: &str, extra: &str) -> String {
        format!(
            "<html><head><title>{title}</title><script type=\"application/ld+json\">{{\"@type\":\"Product\",\"offers\":{{\"price\":\"{price}\",\"priceCurrency\":\"TRY\"}}}}</script></head><body><h1>Ürün</h1><h2>Kargo</h2>{extra}<p>bir iki üç</p></body></html>"
        )
    }

    #[tokio::test]
    async fn snapshots_diff_over_time() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /gizli\n"),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/sitemap.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "<urlset><url><loc>{0}/urun</loc></url><url><loc>{0}/eski</loc></url></urlset>",
                s.uri()
            )))
            .up_to_n_times(1)
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/urun"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(page("Ürün A", "100", ""), "text/html"),
            )
            .up_to_n_times(1)
            .mount(&s)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        add(
            root,
            "rakip",
            &[format!("{}/urun", s.uri()), format!("{}/gizli", s.uri())],
            None,
        )
        .unwrap();
        let d1 = run(root, "kor/test", None, &|_| {}).await.unwrap();
        assert!(d1[0].is_empty());
        Mock::given(method("GET"))
            .and(path("/sitemap.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "<urlset><url><loc>{0}/urun</loc></url><url><loc>{0}/yeni</loc></url></urlset>",
                s.uri()
            )))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/urun"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                page("Ürün A - indirim", "90", "<h2>İade koşulları</h2>"),
                "text/html",
            ))
            .mount(&s)
            .await;
        let d2 = run(root, "kor/test", Some("rakip"), &|_| {}).await.unwrap();
        let d = &d2[0];
        assert_eq!(d.new_urls, [format!("{}/yeni", s.uri())]);
        assert_eq!(d.removed_urls, [format!("{}/eski", s.uri())]);
        let fields: Vec<&str> = d.changes.iter().map(|c| c.field.as_str()).collect();
        assert!(
            fields.contains(&"title") && fields.contains(&"price") && fields.contains(&"headings")
        );
        let price = d.changes.iter().find(|c| c.field == "price").unwrap();
        assert_eq!(
            (price.before.as_str(), price.after.as_str()),
            ("100 TRY", "90 TRY")
        );
        let snaps = std::fs::read_dir(super::dir(root, "rakip"))
            .unwrap()
            .count();
        assert_eq!(snaps, 2);
        // robots.txt kept /gizli out
        assert!(remove(root, "rakip").unwrap());
        assert!(routine(root, "weekly").unwrap()["intervalMinutes"] == 10080);
    }

    #[test]
    fn gaps_against_ours() {
        let ours = PageSnap {
            title: "Kargo".into(),
            headings: vec!["Kargo ücretleri".into()],
            word_count: 100,
            ..PageSnap::default()
        };
        let theirs = PageSnap {
            url: "u".into(),
            schema: vec!["FAQPage".into()],
            headings: vec!["Gümrük işlemleri".into(), "Kargo ücretleri".into()],
            word_count: 400,
            ..PageSnap::default()
        };
        let s = Snapshot {
            name: "r".into(),
            pages: vec![theirs],
            ours: Some(ours),
            ..Snapshot::default()
        };
        let d = diff(None, &s);
        assert!(d.gaps.iter().any(|g| g.starts_with("schema FAQPage")));
        assert!(d.gaps.iter().any(|g| g.contains("Gümrük")));
        assert!(!d.gaps.iter().any(|g| g.contains("\"Kargo ücretleri\"")));
        assert!(d.gaps.iter().any(|g| g.starts_with("length 400")));
    }
}
