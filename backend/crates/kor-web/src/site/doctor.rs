//! Plugin dependency doctor (§29.4). Deterministic: every finding names its source.
//!
//! - wordpress.org plugin API: latest version, last update, "tested up to", required PHP, closed
//!   plugins (with the reason WordPress.org gives);
//! - WPScan API v3 for known vulnerabilities when the user has a token (`WPSCAN_API_TOKEN` in the
//!   vault; free tier) — without it the report says vulnerabilities were not checked;
//! - a built-in table of plugins doing the same job (two caches, two SEO plugins…) and jQuery
//!   helpers that conflict;
//! - performance: on a staging copy, home page response time with each active plugin switched off;
//! - license: plugins not on wordpress.org are commercial or custom, their license cannot be checked.
//!
//! Each plugin gets a score, an A–F grade and one action: remove, replace (with alternatives from
//! the same category), update or keep.

use std::fmt::Write as _;

use kor_i18n::{Localizer, args};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Result;
use super::http::Http;
use super::wp::{Plugin, Wp};

/// Plugins that do the same job; two active in one category conflict. Alternatives come from here.
pub const CATEGORIES: &[(&str, &[&str])] = &[
    (
        "cache",
        &[
            "wp-super-cache",
            "w3-total-cache",
            "wp-fastest-cache",
            "litespeed-cache",
            "wp-rocket",
            "sg-cachepress",
            "breeze",
            "comet-cache",
            "cache-enabler",
            "hummingbird-performance",
            "wp-optimize",
        ],
    ),
    (
        "seo",
        &[
            "wordpress-seo",
            "all-in-one-seo-pack",
            "seo-by-rank-math",
            "autodescription",
            "wp-seopress",
        ],
    ),
    (
        "security",
        &[
            "wordfence",
            "better-wp-security",
            "sucuri-scanner",
            "all-in-one-wp-security-and-firewall",
            "defender-security",
        ],
    ),
    (
        "backup",
        &[
            "updraftplus",
            "backwpup",
            "duplicator",
            "all-in-one-wp-migration",
            "backup-backup",
        ],
    ),
    (
        "smtp",
        &[
            "wp-mail-smtp",
            "post-smtp",
            "easy-wp-smtp",
            "fluent-smtp",
            "smtp-mailer",
        ],
    ),
    (
        "images",
        &[
            "wp-smushit",
            "ewww-image-optimizer",
            "shortpixel-image-optimiser",
            "imagify",
            "optimole-wp",
        ],
    ),
    (
        "minify",
        &[
            "autoptimize",
            "fast-velocity-minify",
            "merge-minify-refresh",
        ],
    ),
    (
        "redirects",
        &[
            "redirection",
            "safe-redirect-manager",
            "simple-301-redirects",
            "eps-301-redirects",
        ],
    ),
    (
        "cookies",
        &[
            "cookie-law-info",
            "complianz-gdpr",
            "cookie-notice",
            "gdpr-cookie-compliance",
            "cookiebot",
        ],
    ),
    (
        "jquery",
        &[
            "jquery-updater",
            "enable-jquery-migrate-helper",
            "jquery-manager",
        ],
    ),
];

/// Release dates of PHP versions: a plugin not updated since then was never tested on it.
const PHP_RELEASES: &[(&str, &str)] = &[
    ("7.4", "2019-11-28"),
    ("8.0", "2020-11-26"),
    ("8.1", "2021-11-25"),
    ("8.2", "2022-12-08"),
    ("8.3", "2023-11-23"),
    ("8.4", "2024-11-21"),
];

pub const ABANDONED_DAYS: i64 = 730;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OrgInfo {
    pub found: bool,
    pub closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tested: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_php: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_installs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vuln {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_in: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    /// Message id (`doctor-*` in the .ftl files) and its arguments, rendered by [`render`].
    pub what: String,
    pub detail: String,
    pub source: String,
    pub points: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub plugin: Plugin,
    pub org: OrgInfo,
    pub vulns: Vec<Vuln>,
    pub findings: Vec<Finding>,
    pub score: i32,
    pub grade: char,
    /// `remove`, `replace`, `update`, `keep`.
    pub action: String,
    pub alternatives: Vec<String>,
    /// Home page response time saved by switching it off (staging measurement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub site: String,
    pub wp_version: Option<String>,
    pub rows: Vec<Row>,
    pub vulns_checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_ms: Option<u64>,
    pub sources: Vec<String>,
}

/// Compare dotted versions (`6.10` > `6.9`).
#[allow(clippy::many_single_char_names)]
pub fn version_lt(a: &str, b: &str) -> bool {
    let p = |s: &str| -> Vec<u64> {
        s.split(['.', '-'])
            .map(|x| x.parse().unwrap_or(0))
            .collect()
    };
    let (x, y) = (p(a), p(b));
    for i in 0..x.len().max(y.len()) {
        let (m, n) = (
            x.get(i).copied().unwrap_or(0),
            y.get(i).copied().unwrap_or(0),
        );
        if m != n {
            return m < n;
        }
    }
    false
}

/// `2024-05-01 3:12pm GMT` → `2024-05-01`.
fn day(s: &str) -> Option<String> {
    let d = s.get(..10)?;
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
        .ok()
        .map(|_| d.to_owned())
}

pub fn parse_org(v: &Value) -> OrgInfo {
    if v["error"].is_string() && v["closed"] != Value::Bool(true) && v["error"] != "closed" {
        return OrgInfo::default();
    }
    let s = |k: &str| v[k].as_str().filter(|x| !x.is_empty()).map(str::to_owned);
    let closed = v["closed"] == Value::Bool(true) || v["error"] == "closed";
    OrgInfo {
        found: true,
        closed,
        closed_reason: if closed {
            s("reason_text").or(s("reason")).or(s("description"))
        } else {
            None
        },
        version: s("version"),
        last_updated: v["last_updated"].as_str().and_then(day),
        tested: s("tested"),
        requires_php: v["requires_php"]
            .as_str()
            .filter(|x| !x.is_empty() && *x != "false")
            .map(str::to_owned),
        active_installs: v["active_installs"].as_u64(),
    }
}

pub async fn org_info(http: &Http, slug: &str) -> Result<OrgInfo> {
    let q = super::http::form(&[
        ("action", "plugin_information"),
        ("request[slug]", slug),
        ("request[fields][sections]", "0"),
        ("request[fields][active_installs]", "1"),
    ]);
    let r = http
        .get(&format!("https://api.wordpress.org/plugins/info/1.2/?{q}"))
        .await?;
    if r.status == 404 {
        // Not found and closed both answer 404 with a JSON error body.
        return Ok(r.json().map(|v| parse_org(&v)).unwrap_or_default());
    }
    Ok(parse_org(&r.json()?))
}

/// WPScan API v3 (`Authorization: Token token=…`).
pub async fn wpscan(http: &Http, base: &str, token: &str, slug: &str) -> Result<Vec<Vuln>> {
    let r = http
        .get_with(
            &format!("{base}/api/v3/plugins/{slug}"),
            &[("authorization", format!("Token token={token}"))],
        )
        .await?;
    if r.status == 404 {
        return Ok(Vec::new());
    }
    if !r.ok() {
        return Err(format!("WPScan: HTTP {}", r.status));
    }
    let v = r.json()?;
    Ok(v[slug]["vulnerabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| Vuln {
            title: x["title"].as_str().unwrap_or("?").to_owned(),
            fixed_in: x["fixed_in"].as_str().map(str::to_owned),
            source: format!("WPScan {}", x["id"].as_str().unwrap_or("")),
        })
        .collect())
}

pub fn category(slug: &str) -> Option<&'static str> {
    CATEGORIES
        .iter()
        .find(|(_, l)| l.contains(&slug))
        .map(|(c, _)| *c)
}

pub fn grade(score: i32) -> char {
    match score {
        90.. => 'A',
        80..=89 => 'B',
        65..=79 => 'C',
        50..=64 => 'D',
        35..=49 => 'E',
        _ => 'F',
    }
}

/// Score one plugin. `active` is the full plugin list (for conflicts), `today` `YYYY-MM-DD`.
#[allow(clippy::too_many_lines)]
pub fn assess(
    p: &Plugin,
    org: OrgInfo,
    vulns: Vec<Vuln>,
    all: &[Plugin],
    wp_version: Option<&str>,
    today: chrono::NaiveDate,
) -> Row {
    let mut f = Vec::new();
    let org_src = "api.wordpress.org/plugins/info/1.2";
    let open: Vec<&Vuln> = vulns
        .iter()
        .filter(|v| {
            v.fixed_in
                .as_deref()
                .is_none_or(|fx| version_lt(&p.version, fx))
        })
        .collect();
    for v in &open {
        f.push(Finding {
            what: "wpdoc-vuln".into(),
            detail: format!(
                "{}{}",
                v.title,
                v.fixed_in
                    .as_deref()
                    .map(|x| format!(" (fixed in {x})"))
                    .unwrap_or_default()
            ),
            source: v.source.clone(),
            points: 60,
        });
    }
    if !org.found {
        f.push(Finding {
            what: "wpdoc-not-org".into(),
            detail: String::new(),
            source: org_src.into(),
            points: 5,
        });
    }
    if org.closed {
        f.push(Finding {
            what: "wpdoc-closed".into(),
            detail: org.closed_reason.clone().unwrap_or_default(),
            source: org_src.into(),
            points: 60,
        });
    }
    if let Some(d) = org
        .last_updated
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
    {
        let days = (today - d).num_days();
        if days > ABANDONED_DAYS {
            f.push(Finding {
                what: "wpdoc-abandoned".into(),
                detail: format!("{d} ({days} d)"),
                source: org_src.into(),
                points: 35,
            });
        }
    }
    if let (Some(t), Some(wp)) = (&org.tested, wp_version) {
        let major = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
        if version_lt(&major(t), &major(wp)) {
            f.push(Finding {
                what: "wpdoc-untested".into(),
                detail: format!("{t} < {wp}"),
                source: org_src.into(),
                points: 15,
            });
        }
    }
    let update = p
        .update
        .clone()
        .or_else(|| org.version.clone().filter(|v| version_lt(&p.version, v)));
    if let Some(u) = &update {
        f.push(Finding {
            what: "wpdoc-outdated".into(),
            detail: format!("{} → {u}", p.version),
            source: if p.update.is_some() {
                "wp-cli".into()
            } else {
                org_src.into()
            },
            points: 10,
        });
    }
    if !p.active {
        f.push(Finding {
            what: "wpdoc-inactive".into(),
            detail: String::new(),
            source: "wp".into(),
            points: 10,
        });
    }
    let cat = category(&p.slug);
    if p.active
        && let Some(c) = cat
    {
        let others: Vec<&str> = all
            .iter()
            .filter(|o| o.active && o.slug != p.slug && category(&o.slug) == Some(c))
            .map(|o| o.slug.as_str())
            .collect();
        if !others.is_empty() {
            f.push(Finding {
                what: "wpdoc-conflict".into(),
                detail: format!("{c}: {}", others.join(", ")),
                source: "kor".into(),
                points: 20,
            });
        }
    }
    let score = 100 - f.iter().map(|x| x.points).sum::<i32>();
    let fixable = open.iter().any(|v| {
        v.fixed_in
            .as_deref()
            .is_some_and(|fx| org.version.as_deref().is_some_and(|l| !version_lt(l, fx)))
    });
    let action = if !p.active {
        "remove"
    } else if org.closed
        || f.iter().any(|x| x.what == "wpdoc-abandoned")
        || (!open.is_empty() && !fixable)
    {
        "replace"
    } else if update.is_some() || fixable {
        "update"
    } else if f.iter().any(|x| x.what == "wpdoc-conflict") {
        "remove"
    } else {
        "keep"
    };
    let alternatives = if action == "replace" {
        cat.and_then(|c| CATEGORIES.iter().find(|(x, _)| *x == c))
            .map(|(_, l)| {
                l.iter()
                    .filter(|s| **s != p.slug)
                    .take(3)
                    .map(|s| (*s).to_owned())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Row {
        plugin: p.clone(),
        org,
        vulns,
        findings: f,
        score: score.max(0),
        grade: grade(score),
        action: action.into(),
        alternatives,
        cost_ms: None,
    }
}

/// Plugins likely to break on PHP `target` (for `kor host php`): required PHP above the target, or
/// no update since `target` was released.
pub fn php_warnings(l: &Localizer, plugins: &[(Plugin, OrgInfo)], target: &str) -> Vec<String> {
    let release = PHP_RELEASES
        .iter()
        .find(|(v, _)| *v == target)
        .map(|(_, d)| *d);
    let mut out = Vec::new();
    for (p, o) in plugins.iter().filter(|(p, _)| p.active) {
        if let Some(req) = &o.requires_php
            && version_lt(target, req)
        {
            out.push(l.t_args(
                "wpdoc-php-requires",
                &args!("plugin" => p.slug.clone(), "php" => req.clone()),
            ));
        } else if let (Some(rel), Some(last)) = (release, &o.last_updated)
            && last.as_str() < rel
        {
            out.push(l.t_args("wpdoc-php-stale", &args!("plugin" => p.slug.clone(), "date" => last.clone(), "php" => target.to_owned())));
        }
    }
    out
}

/// Look every plugin up and score it. `wpscan`: (API base, token).
pub async fn diagnose(
    http: &Http,
    site: &str,
    plugins: &[Plugin],
    wp_version: Option<&str>,
    wpscan_with: Option<(&str, &str)>,
    org_base: Option<&str>,
) -> Report {
    let today = chrono::Utc::now().date_naive();
    let mut rep = Report {
        site: site.to_owned(),
        wp_version: wp_version.map(str::to_owned),
        vulns_checked: wpscan_with.is_some(),
        sources: vec!["api.wordpress.org/plugins/info/1.2".into()],
        ..Report::default()
    };
    if wpscan_with.is_some() {
        rep.sources.push("wpscan.com/api/v3".into());
    }
    for p in plugins {
        let org = match org_base {
            Some(b) => org_info_at(http, b, &p.slug).await.unwrap_or_default(),
            None => org_info(http, &p.slug).await.unwrap_or_default(),
        };
        let vulns = match wpscan_with {
            Some((base, token)) => wpscan(http, base, token, &p.slug).await.unwrap_or_default(),
            None => Vec::new(),
        };
        rep.rows
            .push(assess(p, org, vulns, plugins, wp_version, today));
    }
    rep.rows.sort_by_key(|r| r.score);
    rep
}

/// Same as [`org_info`] against another base URL (tests, mirrors).
pub async fn org_info_at(http: &Http, base: &str, slug: &str) -> Result<OrgInfo> {
    let q = super::http::form(&[("action", "plugin_information"), ("request[slug]", slug)]);
    let r = http.get(&format!("{base}/plugins/info/1.2/?{q}")).await?;
    Ok(r.json().map(|v| parse_org(&v)).unwrap_or_default())
}

/// Median response time of `url` over `n` requests, in ms.
async fn median_ms(http: &Http, url: &str, n: usize) -> Option<u64> {
    let mut v = Vec::new();
    for _ in 0..n {
        if let Ok(r) = http.get(url).await {
            v.push(r.ms);
        }
    }
    v.sort_unstable();
    v.get(v.len() / 2).copied()
}

/// On the staging copy: switch each active plugin off, measure the home page, switch it back on.
/// Fills `cost_ms` (positive = the plugin costs that many ms).
pub async fn measure(
    wp: &Wp,
    staging_root: &str,
    staging_url: &str,
    rep: &mut Report,
    rounds: usize,
) -> Result<()> {
    let home = format!("{staging_url}/");
    let base = median_ms(wp.http(), &home, rounds)
        .await
        .ok_or("the staging home page does not answer")?;
    rep.baseline_ms = Some(base);
    for row in rep
        .rows
        .iter_mut()
        .filter(|r| r.plugin.active && r.plugin.slug != "woocommerce")
    {
        let slug = row.plugin.slug.clone();
        if !slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            continue;
        }
        wp.cli_at(staging_root, &format!("plugin deactivate {slug} --quiet"))
            .await?;
        let without = median_ms(wp.http(), &home, rounds).await;
        wp.cli_at(staging_root, &format!("plugin activate {slug} --quiet"))
            .await?;
        row.cost_ms =
            without.map(|w| i64::try_from(base).unwrap_or(0) - i64::try_from(w).unwrap_or(0));
    }
    Ok(())
}

/// Markdown table: grade, plugin, version, findings with sources, action.
pub fn render(l: &Localizer, rep: &Report) -> String {
    let mut o = format!(
        "# {}\n\n",
        l.t_args("wpdoc-title", &args!("site" => rep.site.clone()))
    );
    if !rep.vulns_checked {
        let _ = writeln!(o, "> {}\n", l.t("wpdoc-no-vuln-source"));
    }
    if let Some(b) = rep.baseline_ms {
        let _ = writeln!(o, "{}\n", l.t_args("wpdoc-baseline", &args!("ms" => b)));
    }
    let _ = writeln!(
        o,
        "| {} | {} | {} | {} | {} | {} |\n|---|---|---|---|---|---|",
        l.t("wpdoc-col-grade"),
        l.t("wpdoc-col-plugin"),
        l.t("wpdoc-col-version"),
        l.t("wpdoc-col-findings"),
        l.t("wpdoc-col-cost"),
        l.t("wpdoc-col-action")
    );
    for r in &rep.rows {
        let findings: Vec<String> = r
            .findings
            .iter()
            .map(|f| {
                let head = l.t(&f.what);
                let d = if f.detail.is_empty() {
                    String::new()
                } else {
                    format!(": {}", f.detail)
                };
                format!("{head}{d} [{}]", f.source)
            })
            .collect();
        let mut action = l.t(&format!("wpdoc-action-{}", r.action));
        if !r.alternatives.is_empty() {
            let _ = write!(action, " ({})", r.alternatives.join(", "));
        }
        let _ = writeln!(
            o,
            "| {} ({}) | {} | {} | {} | {} | {} |",
            r.grade,
            r.score,
            super::cell(&r.plugin.slug),
            super::cell(&r.plugin.version),
            super::cell(&if findings.is_empty() {
                "—".into()
            } else {
                findings.join("; ")
            }),
            r.cost_ms.map_or_else(|| "—".into(), |m| format!("{m} ms")),
            super::cell(&action)
        );
    }
    let _ = writeln!(o, "\n{}: {}", l.t("wpdoc-sources"), rep.sources.join(", "));
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn p(slug: &str, v: &str, active: bool) -> Plugin {
        Plugin {
            slug: slug.into(),
            name: slug.into(),
            version: v.into(),
            active,
            update: None,
        }
    }

    #[test]
    fn versions_and_grades() {
        assert!(
            version_lt("6.9", "6.10") && !version_lt("6.10", "6.9") && !version_lt("1.0", "1.0.0")
        );
        assert_eq!(
            (
                grade(100),
                grade(85),
                grade(70),
                grade(55),
                grade(40),
                grade(10)
            ),
            ('A', 'B', 'C', 'D', 'E', 'F')
        );
    }

    #[test]
    fn scoring_rules() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let all = [
            p("wp-super-cache", "1.9", true),
            p("w3-total-cache", "2.7", true),
            p("old-slider", "1.0", true),
            p("hello-dolly", "1.7", false),
        ];
        let fresh = OrgInfo {
            found: true,
            version: Some("1.9".into()),
            last_updated: Some("2026-10-01".into()),
            tested: Some("6.7".into()),
            ..OrgInfo::default()
        };
        let r = assess(&all[0], fresh.clone(), vec![], &all, Some("6.7"), today);
        assert_eq!(r.action, "remove", "{r:?}");
        assert!(r.findings.iter().any(|f| f.what == "wpdoc-conflict"));
        let old = OrgInfo {
            found: true,
            version: Some("1.0".into()),
            last_updated: Some("2021-01-01".into()),
            ..OrgInfo::default()
        };
        let r = assess(&all[2], old, vec![], &all, None, today);
        assert_eq!(r.action, "replace");
        let r = assess(&all[3], fresh.clone(), vec![], &all, None, today);
        assert_eq!(r.action, "remove");
        let v = vec![Vuln {
            title: "XSS".into(),
            fixed_in: Some("1.9.1".into()),
            source: "WPScan 1".into(),
        }];
        let newer = OrgInfo {
            version: Some("1.9.1".into()),
            ..fresh
        };
        let w3 = p("w3-total-cache", "1.9.0", true);
        let r = assess(&w3, newer, v, std::slice::from_ref(&w3), None, today);
        assert_eq!(r.action, "update");
        assert_eq!((r.score, r.grade), (30, 'F'));
    }

    #[test]
    fn closed_plugins_and_php() {
        let closed = parse_org(
            &json!({"error": "closed", "closed": true, "reason_text": "Security Issue", "slug": "x"}),
        );
        assert!(closed.found && closed.closed);
        assert_eq!(closed.closed_reason.as_deref(), Some("Security Issue"));
        assert!(!parse_org(&json!({"error": "Plugin not found."})).found);
        let l = Localizer::new(kor_i18n::Lang::En);
        let list = vec![
            (
                p("a", "1", true),
                OrgInfo {
                    requires_php: Some("8.1".into()),
                    ..OrgInfo::default()
                },
            ),
            (
                p("b", "1", true),
                OrgInfo {
                    last_updated: Some("2020-01-01".into()),
                    ..OrgInfo::default()
                },
            ),
            (
                p("c", "1", true),
                OrgInfo {
                    last_updated: Some("2025-01-01".into()),
                    ..OrgInfo::default()
                },
            ),
        ];
        let w = php_warnings(&l, &list, "7.4");
        assert_eq!(w.len(), 1);
        let w = php_warnings(&l, &list, "8.3");
        assert_eq!(w.len(), 1);
        assert!(w[0].contains('b'));
    }

    #[tokio::test]
    async fn diagnose_with_mock_sources() {
        let s = MockServer::start().await;
        Mock::given(path("/plugins/info/1.2/")).and(query_param("request[slug]", "contact-form-7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"slug": "contact-form-7", "version": "6.0", "last_updated": "2026-09-01 1:00pm GMT", "tested": "6.7", "requires_php": "7.4"})))
            .mount(&s).await;
        Mock::given(path("/api/v3/plugins/contact-form-7")).and(header("authorization", "Token token=T"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"contact-form-7": {"vulnerabilities": [{"id": "abc", "title": "Unrestricted upload", "fixed_in": "5.3.2"}]}})))
            .mount(&s).await;
        let http = Http::new().unwrap();
        let rep = diagnose(
            &http,
            "shop",
            &[p("contact-form-7", "5.3.1", true)],
            Some("6.7"),
            Some((&s.uri(), "T")),
            Some(&s.uri()),
        )
        .await;
        let r = &rep.rows[0];
        assert_eq!(r.vulns.len(), 1);
        assert_eq!(r.action, "update");
        let md = render(&Localizer::new(kor_i18n::Lang::En), &rep);
        assert!(md.contains("WPScan abc") && md.contains("contact-form-7"));
    }
}
