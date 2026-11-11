//! `kor seo agents` (spec §29.15): the multi-agent form of the audit.
//!
//! The lead auditor (Baş Denetçi) runs one shared crawl (Crawler agent), hands each specialist
//! its slice of the deterministic checks, and runs the I/O agents (Competitor, Log) in parallel.
//! Findings merge into one report: duplicates removed, conflicting advice on the same URL
//! reconciled into one decision note, every rule ranked by impact × effort (Reporter) and turned
//! into one fix list (Fixer: per-URL fixes, an `.htaccess` block for redirect chains, the URL list
//! to resubmit in Search Console). A model is used only for the optional summary and for
//! user-defined agents with a checklist (`.kor/agents/seo/<name>.toml`), never for the checks.
//!
//! Why not `kor_runtime::task` (ADR 0037): that engine plans code changes into git worktrees and
//! merges branches; an audit team edits no files and its "merge" is a findings merge, so the
//! team runs as concurrent futures here (ADR 0055).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_runtime::analysis::{Finding, Report, Severity};
use serde::{Deserialize, Serialize};

use super::audit::{self, Crawl, Options};
use super::model::{Ask, unfence};

/// Built-in agents: `(id, Turkish label, rule prefixes after "seo.")`.
pub const AGENTS: &[(&str, &str, &[&str])] = &[
    (
        "crawler",
        "Tarayıcı",
        &[
            "orphan",
            "deep-page",
            "param-url",
            "not-in-sitemap",
            "internal-link-redirect",
            "broken-link",
            "broken-external-link",
            "nofollow-internal",
        ],
    ),
    (
        "technical",
        "Teknik",
        &[
            "status",
            "fetch-error",
            "redirect-",
            "canonical-",
            "hreflang-",
            "noindex",
            "x-robots-noindex",
            "sitemap-",
            "robots-",
            "viewport-missing",
            "large-html",
        ],
    ),
    (
        "performance",
        "Performans",
        &[
            "cwv-",
            "render-blocking",
            "slow-response",
            "img-size-missing",
        ],
    ),
    (
        "content",
        "İçerik",
        &[
            "title-",
            "description-",
            "h1-",
            "heading-skip",
            "thin-content",
            "img-alt-missing",
        ],
    ),
    (
        "structured",
        "Yapılandırılmış veri",
        &["jsonld-", "richresult-"],
    ),
    ("local", "Yerel / Türkçe", &["tr-slug", "lang-missing"]),
    (
        "security",
        "Güvenlik & güven",
        &["mixed-content", "insecure-link", "hidden-link", "spam-link"],
    ),
    ("competitor", "Rakip", &["competitor."]),
    ("log", "Log", &["log."]),
    ("fixer", "Düzeltici", &[]),
    ("reporter", "Raporcu", &[]),
];

/// Turkish/English names → id (`teknik`, `performans`, …).
pub fn agent_id(name: &str) -> Option<&'static str> {
    let n = name.trim().to_lowercase();
    let alias = match n.as_str() {
        "tarayici" | "tarayıcı" => "crawler",
        "teknik" => "technical",
        "performans" => "performance",
        "icerik" | "içerik" => "content",
        "yapilandirilmis" | "yapılandırılmış" | "schema" => "structured",
        "yerel" | "turkce" | "türkçe" => "local",
        "guvenlik" | "güvenlik" => "security",
        "rakip" => "competitor",
        "duzeltici" | "düzeltici" => "fixer",
        "raporcu" => "reporter",
        other => other,
    };
    AGENTS
        .iter()
        .find(|(id, ..)| *id == alias)
        .map(|(id, ..)| *id)
}

/// A user-defined specialist (`.kor/agents/seo/<name>.toml`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custom {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    /// Rule prefixes (after `seo.`) whose findings this agent owns.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Checks the model applies to the crawled pages' facts.
    #[serde(default)]
    pub checklist: Vec<String>,
}

pub fn custom_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("agents").join("seo")
}

pub fn load_custom(root: &Path) -> Vec<Custom> {
    let mut v: Vec<Custom> = std::fs::read_dir(custom_dir(root))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .filter_map(|e| {
            let mut c: Custom = toml::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            if c.name.is_empty() {
                c.name = e.path().file_stem()?.to_string_lossy().into_owned();
            }
            Some(c)
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Team settings.
#[derive(Debug, Clone, Default)]
pub struct TeamOptions {
    pub audit: Options,
    /// Empty = all built-in agents.
    pub only: Vec<String>,
    /// Access log for the Log agent.
    pub log: Option<PathBuf>,
    pub verify_dns: bool,
    /// Project root (competitor config, custom agents, history).
    pub root: PathBuf,
}

/// Live progress for spinners / cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Event {
    Started { agent: String },
    Progress { agent: String, detail: String },
    Done { agent: String, findings: usize },
    Skipped { agent: String, reason: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentResult {
    pub id: String,
    pub label: String,
    pub findings: usize,
    pub notes: Vec<String>,
}

/// A rule ranked by the Reporter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Priority {
    pub rule: String,
    pub agent: String,
    pub severity: Severity,
    pub pages: usize,
    /// 1 (template/setting) … 3 (engineering work).
    pub effort: u8,
    /// severity weight × pages ÷ effort.
    pub score: f64,
    pub fix: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TeamReport {
    pub report: Report,
    pub agents: Vec<AgentResult>,
    pub priorities: Vec<Priority>,
    /// Reconciled conflicts: URL → decision note.
    pub conflicts: Vec<(String, String)>,
    pub fixes_markdown: String,
    pub htaccess: String,
    pub gsc_urls: Vec<String>,
    pub diff: Option<audit::Diff>,
    pub summary: Option<String>,
}

fn owner(rule: &str) -> &'static str {
    let r = rule.strip_prefix("seo.").unwrap_or(rule);
    AGENTS
        .iter()
        .find(|(_, _, pre)| {
            pre.iter().any(|p| {
                if p.ends_with(['-', '.']) {
                    r.starts_with(p)
                } else {
                    r == *p
                }
            })
        })
        .map_or("technical", |(id, ..)| *id)
}

fn weight(s: Severity) -> f64 {
    match s {
        Severity::Critical => 5.0,
        Severity::High => 4.0,
        Severity::Medium => 3.0,
        Severity::Low => 2.0,
        Severity::Info => 1.0,
    }
}

fn effort(rule: &str) -> u8 {
    let r = rule.strip_prefix("seo.").unwrap_or(rule);
    if [
        "title-",
        "description-",
        "img-alt",
        "lang-",
        "viewport",
        "canonical-missing",
        "robots-",
        "nofollow",
        "insecure-link",
        "mixed-content",
    ]
    .iter()
    .any(|p| r.starts_with(p))
    {
        1
    } else if [
        "cwv-",
        "render-blocking",
        "slow-response",
        "thin-content",
        "spam-link",
        "hidden-link",
        "log.server",
        "status",
    ]
    .iter()
    .any(|p| r.starts_with(p))
    {
        3
    } else {
        2
    }
}

/// Run the team.
#[allow(clippy::too_many_lines)] // the lead's whole run reads best top to bottom
pub async fn run(
    opts: &TeamOptions,
    ask: Option<&dyn Ask>,
    lang: &str,
    on: &(dyn Fn(Event) + Sync),
) -> Result<TeamReport, String> {
    let wanted: HashSet<&str> = if opts.only.is_empty() {
        AGENTS.iter().map(|(id, ..)| *id).collect()
    } else {
        opts.only
            .iter()
            .map(|n| agent_id(n).ok_or_else(|| format!("unknown agent: {n}")))
            .collect::<Result<_, _>>()?
    };

    // The crawl is shared: every page-level agent needs it.
    on(Event::Started {
        agent: "crawler".into(),
    });
    let progress = |d: &str| {
        on(Event::Progress {
            agent: "crawler".into(),
            detail: d.to_owned(),
        });
    };
    let mut aopts = opts.audit.clone();
    aopts.vitals = aopts.vitals && wanted.contains("performance");
    let crawl_fut = audit::crawl(&aopts, &progress);
    let comp_fut = async {
        if !wanted.contains("competitor") {
            return None;
        }
        let cfg = super::competitors::load(&opts.root).ok()?;
        if cfg.competitors.is_empty() {
            on(Event::Skipped {
                agent: "competitor".into(),
                reason: "no competitors configured (kor seo competitors add)".into(),
            });
            return None;
        }
        on(Event::Started {
            agent: "competitor".into(),
        });
        Some(
            super::competitors::run(&opts.root, &aopts.user_agent, None, &|n| {
                on(Event::Progress {
                    agent: "competitor".into(),
                    detail: n.to_owned(),
                });
            })
            .await,
        )
    };
    let log_fut = async {
        if !wanted.contains("log") {
            return None;
        }
        let Some(p) = &opts.log else {
            on(Event::Skipped {
                agent: "log".into(),
                reason: "no access log given (--log)".into(),
            });
            return None;
        };
        on(Event::Started {
            agent: "log".into(),
        });
        Some(
            tokio::fs::read_to_string(p)
                .await
                .map(|t| (p.clone(), t))
                .map_err(|e| e.to_string()),
        )
    };
    let (crawl, comp, log) = futures::join!(crawl_fut, comp_fut, log_fut);
    let crawl: Crawl = crawl?;
    let base = audit::check(&crawl);

    let mut merged = Report::new("seo-agents");
    merged.notes.clone_from(&base.notes);
    let mut results: BTreeMap<&str, AgentResult> = BTreeMap::new();
    for f in base.findings {
        let id = owner(&f.rule);
        if wanted.contains(id) {
            merged.findings.push(f);
        }
    }
    // Competitor agent.
    if let Some(r) = comp {
        match r {
            Ok(diffs) => {
                for d in diffs {
                    for g in &d.gaps {
                        merged.findings.push(Finding {
                            rule: "seo.competitor.gap".into(),
                            source: "kor".into(),
                            severity: Severity::Low,
                            file: PathBuf::from(format!("competitor:{}", d.name)),
                            line: 1,
                            message: format!("They have, we lack: {g}"),
                            ..Finding::default()
                        });
                    }
                    for c in &d.changes {
                        merged.findings.push(Finding {
                            rule: "seo.competitor.change".into(),
                            source: "kor".into(),
                            severity: Severity::Info,
                            file: PathBuf::from(&c.url),
                            line: 1,
                            message: format!("{}: \"{}\" → \"{}\"", c.field, c.before, c.after),
                            ..Finding::default()
                        });
                    }
                    if !d.new_urls.is_empty() {
                        merged.findings.push(Finding {
                            rule: "seo.competitor.new-pages".into(),
                            source: "kor".into(),
                            severity: Severity::Info,
                            file: PathBuf::from(format!("competitor:{}", d.name)),
                            line: 1,
                            message: format!(
                                "{} new pages: {}",
                                d.new_urls.len(),
                                d.new_urls
                                    .iter()
                                    .take(10)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            ..Finding::default()
                        });
                    }
                }
            }
            Err(e) => merged.notes.push(format!("competitor agent: {e}")),
        }
    }
    // Log agent.
    if let Some(r) = log {
        match r {
            Ok((path, text)) => {
                let sitemap = crawl.sitemap_urls.clone();
                let verify =
                    opts.verify_dns && kor_core::net::policy() == kor_core::net::Policy::Open;
                let shown = path.to_string_lossy().into_owned();
                let (_, lr) = tokio::task::spawn_blocking(move || {
                    let dns = super::logs::SystemDns;
                    super::logs::analyze(
                        &shown,
                        &text,
                        &super::logs::Options {
                            sitemap,
                            verify_dns: verify,
                        },
                        verify.then_some(&dns as &dyn super::logs::Dns),
                    )
                })
                .await
                .map_err(|e| e.to_string())?;
                merged.findings.extend(lr.findings);
                merged.notes.extend(lr.notes);
            }
            Err(e) => merged.notes.push(format!("log agent: {e}")),
        }
    }
    // User-defined agents.
    for c in load_custom(&opts.root) {
        on(Event::Started {
            agent: c.name.clone(),
        });
        let n = custom_agent(&c, &crawl, ask, &mut merged).await;
        on(Event::Done {
            agent: c.name.clone(),
            findings: n,
        });
    }

    // Lead: dedup, reconcile conflicts per URL.
    merged.normalize();
    let mut by_url: HashMap<String, Vec<&str>> = HashMap::new();
    for f in &merged.findings {
        by_url
            .entry(f.file.to_string_lossy().into_owned())
            .or_default()
            .push(f.rule.as_str());
    }
    let mut conflicts = Vec::new();
    for (u, rules) in &by_url {
        let has = |r: &str| rules.contains(&r);
        if has("seo.noindex")
            && (has("seo.orphan") || rules.iter().any(|r| r.starts_with("seo.sitemap")))
        {
            conflicts.push((u.clone(), "Decide first whether this page should rank: if yes remove noindex (the sitemap and link fixes then apply); if no, drop it from the sitemap and ignore the other findings.".to_owned()));
        }
        if has("seo.canonical-target") && has("seo.sitemap-non-canonical") {
            conflicts.push((
                u.clone(),
                "Fix the canonical target first; the sitemap entry follows the canonical you keep."
                    .to_owned(),
            ));
        }
        if has("seo.title-length") && has("seo.title-duplicate") {
            conflicts.push((u.clone(), "Rewrite the title once: unique and ≤ 60 characters (one change fixes both findings).".to_owned()));
        }
    }
    conflicts.sort();

    for (id, l, _) in AGENTS {
        if wanted.contains(id) && !matches!(*id, "fixer" | "reporter") {
            let n = merged
                .findings
                .iter()
                .filter(|f| owner(&f.rule) == *id)
                .count();
            results.insert(
                *id,
                AgentResult {
                    id: (*id).into(),
                    label: (*l).into(),
                    findings: n,
                    notes: Vec::new(),
                },
            );
            on(Event::Done {
                agent: (*id).into(),
                findings: n,
            });
        }
    }

    // Reporter: impact × effort.
    let mut per_rule: BTreeMap<&str, (Severity, HashSet<String>, String)> = BTreeMap::new();
    for f in &merged.findings {
        let e = per_rule.entry(f.rule.as_str()).or_insert((
            f.severity,
            HashSet::new(),
            f.fix.clone().unwrap_or_default(),
        ));
        e.0 = e.0.min(f.severity);
        e.1.insert(f.file.to_string_lossy().into_owned());
    }
    #[allow(clippy::cast_precision_loss)]
    let mut priorities: Vec<Priority> = per_rule
        .into_iter()
        .map(|(rule, (sev, pages, fix))| {
            let eff = effort(rule);
            Priority {
                rule: rule.into(),
                agent: owner(rule).into(),
                severity: sev,
                pages: pages.len(),
                effort: eff,
                score: weight(sev) * pages.len() as f64 / f64::from(eff),
                fix,
            }
        })
        .collect();
    priorities.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.rule.cmp(&b.rule))
    });

    // Fixer: per-URL fix list, redirect block, GSC resubmit list.
    let mut fixes = String::new();
    let mut urls: BTreeMap<String, Vec<&Finding>> = BTreeMap::new();
    for f in merged
        .findings
        .iter()
        .filter(|f| f.severity <= Severity::Low)
    {
        urls.entry(f.file.to_string_lossy().into_owned())
            .or_default()
            .push(f);
    }
    for (u, fs) in &urls {
        let _ = writeln!(fixes, "## {u}");
        for f in fs {
            let _ = writeln!(
                fixes,
                "- [{}] `{}` line {}: {}{}",
                f.severity.as_str(),
                f.rule,
                f.line,
                f.message,
                f.fix
                    .as_deref()
                    .map(|x| format!("\n  → {x}"))
                    .unwrap_or_default()
            );
        }
        if let Some((_, note)) = conflicts.iter().find(|(cu, _)| cu == u) {
            let _ = writeln!(fixes, "- Lead auditor: {note}");
        }
        fixes.push('\n');
    }
    let mut htaccess = String::new();
    for p in crawl
        .pages
        .iter()
        .filter(|p| p.hops.len() > 1 && !p.redirect_loop)
    {
        if let Ok(u) = url::Url::parse(&p.url) {
            let _ = writeln!(htaccess, "Redirect 301 {} {}", u.path(), p.final_url);
        }
    }
    let gsc_urls: Vec<String> = merged
        .findings
        .iter()
        .filter(|f| f.severity <= Severity::Medium && f.file.to_string_lossy().starts_with("http"))
        .map(|f| f.file.to_string_lossy().into_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    // History: diff with the previous team run.
    let diff = audit::save_to(
        &audit::history_dir(&opts.root, &crawl.start).join("agents"),
        &merged,
    )
    .ok()
    .flatten()
    .map(|prev| audit::diff(&prev, &merged));

    let mut team = TeamReport {
        agents: results.into_values().collect(),
        priorities,
        conflicts,
        fixes_markdown: fixes,
        htaccess,
        gsc_urls,
        diff,
        summary: None,
        report: merged,
    };
    if let Some(a) = ask.filter(|_| wanted.contains("reporter")) {
        on(Event::Started {
            agent: "reporter".into(),
        });
        match a.ask(summary_prompt(&team, lang), 1_500).await {
            Ok(t) => team.summary = Some(t.trim().to_owned()),
            Err(e) => team.report.notes.push(format!("summary: {e}")),
        }
        on(Event::Done {
            agent: "reporter".into(),
            findings: 0,
        });
    }
    Ok(team)
}

fn summary_prompt(t: &TeamReport, lang: &str) -> String {
    let mut s = format!(
        "You are the reporting agent of an SEO audit team. Write a short client-ready summary in language `{lang}`: \
         3-5 sentences on overall health, then the top 5 actions in priority order with the affected page count. \
         Use only the data below (deterministic findings); do not invent numbers.\n\nPriorities (rule, severity, pages, effort 1-3, fix):\n"
    );
    for p in t.priorities.iter().take(15) {
        let _ = writeln!(
            s,
            "- {} [{}] {} pages, effort {}: {}",
            p.rule,
            p.severity.as_str(),
            p.pages,
            p.effort,
            p.fix
        );
    }
    let c = t.report.counts();
    let _ = writeln!(
        s,
        "\nTotals: critical {} high {} medium {} low {} info {}",
        c[0], c[1], c[2], c[3], c[4]
    );
    if let Some(d) = &t.diff {
        let _ = writeln!(
            s,
            "Since the previous audit: {} new, {} fixed.",
            d.new.len(),
            d.fixed.len()
        );
    }
    s
}

/// A user agent: owns its rules; with a checklist and a model, reviews page facts.
async fn custom_agent(
    c: &Custom,
    crawl: &Crawl,
    ask: Option<&dyn Ask>,
    merged: &mut Report,
) -> usize {
    let mut n = merged
        .findings
        .iter()
        .filter(|f| {
            c.rules.iter().any(|r| {
                f.rule
                    .strip_prefix("seo.")
                    .unwrap_or(&f.rule)
                    .starts_with(r.as_str())
            })
        })
        .count();
    let (Some(a), false) = (ask, c.checklist.is_empty()) else {
        return n;
    };
    let mut facts = String::new();
    for p in crawl.pages.iter().filter(|p| p.page.is_some()).take(40) {
        let pg = p.page.as_ref().expect("filtered");
        let _ = writeln!(
            facts,
            "- {} | title: {} | h1: {} | words: {} | schema: {} | h2: {}",
            p.final_url,
            pg.titles.first().map_or("", |t| t.value.as_str()),
            pg.headings
                .iter()
                .find(|h| h.value.0 == 1)
                .map_or("", |h| h.value.1.as_str()),
            p.word_count,
            p.rich.eligible.join(","),
            pg.headings
                .iter()
                .filter(|h| h.value.0 == 2)
                .map(|h| h.value.1.as_str())
                .take(8)
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    let prompt = format!(
        "You are the \"{}\" SEO agent: {}. Check these pages against the checklist. Report only concrete, page-level problems \
         visible in the facts (no generic advice).\nChecklist:\n{}\nPages:\n{facts}\nAnswer with only a JSON array \
         [{{\"url\":\"…\",\"severity\":\"high|medium|low\",\"message\":\"…\",\"fix\":\"…\"}}] (empty array if nothing).",
        c.name,
        c.role,
        c.checklist
            .iter()
            .map(|x| format!("- {x}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    match a
        .ask(prompt, 3_000)
        .await
        .map(|t| serde_json::from_str::<serde_json::Value>(unfence(&t)).map_err(|e| e.to_string()))
    {
        Ok(Ok(v)) => {
            for item in v.as_array().into_iter().flatten().take(100) {
                let url = item["url"].as_str().unwrap_or("");
                if !crawl
                    .pages
                    .iter()
                    .any(|p| p.final_url == url || p.url == url)
                {
                    continue; // only pages that were actually crawled
                }
                merged.findings.push(Finding {
                    rule: format!("seo.agent.{}", c.name),
                    source: format!("model:{}", a.id()),
                    severity: Severity::parse(item["severity"].as_str().unwrap_or("low"))
                        .unwrap_or(Severity::Low),
                    file: PathBuf::from(url),
                    line: 1,
                    message: item["message"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(500)
                        .collect(),
                    fix: item["fix"].as_str().map(|s| s.chars().take(500).collect()),
                    ..Finding::default()
                });
                n += 1;
            }
        }
        Ok(Err(e)) | Err(e) => merged.notes.push(format!("agent {}: {e}", c.name)),
    }
    n
}

/// Watch-mode notification text: new findings since the previous team run (critical/high
/// counted) and Core Web Vitals regressions; `None` when nothing new.
pub fn watch_note(t: &TeamReport) -> Option<String> {
    let d = t.diff.as_ref()?;
    if d.new.is_empty() {
        return None;
    }
    let severe = d
        .new
        .iter()
        .filter(|f| f.severity <= Severity::High)
        .count();
    let cwv: Vec<String> = d
        .new
        .iter()
        .filter(|f| f.rule.starts_with("seo.cwv-"))
        .map(|f| format!("{} {}", f.file.to_string_lossy(), f.message))
        .take(5)
        .collect();
    let mut s = format!(
        "{} new findings, {severe} critical/high; {} fixed.",
        d.new.len(),
        d.fixed.len()
    );
    if !cwv.is_empty() {
        let _ = write!(s, " Core Web Vitals regressed: {}", cwv.join("; "));
    }
    Some(s)
}

/// Routine for `--schedule daily|weekly` (watch mode): rerun and report the difference.
pub fn routine(
    root: &Path,
    site: &str,
    every: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let minutes = match every {
        "daily" => 24 * 60,
        "weekly" => 7 * 24 * 60,
        _ => return Err("schedule: daily or weekly".into()),
    };
    if site.contains(['`', '\n', ';', '&', '|']) {
        return Err("bad site".into());
    }
    let mut m = serde_json::Map::new();
    m.insert(
        "name".into(),
        format!("SEO agents: {site} ({every})").into(),
    );
    m.insert("prompt".into(), format!("Run the shell command `kor seo agents {site} --depth quick` in this folder. Report how many findings are new and how many were fixed since the previous run, list new critical/high findings, and alert if Core Web Vitals got worse. Do not change any files.").into());
    m.insert("intervalMinutes".into(), minutes.into());
    m.insert("root".into(), root.to_string_lossy().into_owned().into());
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::super::model::testing::Scripted;
    use super::*;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn watch_note_counts_new_and_cwv() {
        let f = |rule: &str, sev: Severity| Finding {
            rule: rule.into(),
            severity: sev,
            file: "https://x.test/".into(),
            line: 1,
            message: "LCP 5000 ms".into(),
            ..Finding::default()
        };
        let mut t = TeamReport::default();
        assert!(watch_note(&t).is_none());
        t.diff = Some(audit::Diff {
            new: vec![
                f("seo.cwv-lcp", Severity::High),
                f("seo.title-length", Severity::Low),
            ],
            fixed: vec![],
        });
        let n = watch_note(&t).unwrap();
        assert!(
            n.starts_with("2 new findings, 1 critical/high")
                && n.contains("Core Web Vitals regressed")
        );
    }

    #[test]
    fn names_and_owners() {
        assert_eq!(agent_id("Teknik"), Some("technical"));
        assert_eq!(agent_id("performans"), Some("performance"));
        assert_eq!(agent_id("bogus"), None);
        assert_eq!(owner("seo.title-length"), "content");
        assert_eq!(owner("seo.redirect-chain"), "technical");
        assert_eq!(owner("seo.log.fake-bot"), "log");
        assert_eq!(owner("seo.status"), "technical");
    }

    #[tokio::test]
    async fn team_merges_reconciles_and_reports() {
        let s = MockServer::start().await;
        let u = s.uri();
        Mock::given(method("GET"))
            .and(path("/robots.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "User-agent: *\nAllow: /\nSitemap: {u}/sitemap.xml\n"
            )))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/sitemap.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "<urlset><url><loc>{u}/</loc></url><url><loc>{u}/gizli</loc></url></urlset>"
            )))
            .mount(&s)
            .await;
        Mock::given(method("GET")).and(path("/")).respond_with(ResponseTemplate::new(200).set_body_raw("<html><head><title>Ana sayfa başlığı burada uzunca</title></head><body><h1>Ana</h1><a href=\"/eski\">x</a></body></html>", "text/html")).mount(&s).await;
        Mock::given(method("GET"))
            .and(path("/eski"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "/ara"))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/ara"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "/"))
            .mount(&s)
            .await;
        Mock::given(method("GET")).and(path("/gizli")).respond_with(ResponseTemplate::new(200).set_body_raw("<html><head><meta name=robots content=noindex><title>Gizli</title></head><body></body></html>", "text/html")).mount(&s).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(custom_dir(dir.path())).unwrap();
        std::fs::write(custom_dir(dir.path()).join("eticaret.toml"), "role = \"product pages\"\nrules = [\"title-\"]\nchecklist = [\"Product pages need a price\"]\n").unwrap();
        let log = dir.path().join("access.log");
        std::fs::write(&log, "66.249.66.1 - - [01/Oct/2026:10:00:00 +0300] \"GET /yok HTTP/1.1\" 404 1 \"-\" \"Googlebot/2.1\"\n").unwrap();
        let opts = TeamOptions {
            audit: Options {
                start: format!("{u}/"),
                depth: audit::Depth::Full,
                delay: std::time::Duration::ZERO,
                vitals: false,
                ..Options::default()
            },
            log: Some(log),
            root: dir.path().to_path_buf(),
            ..TeamOptions::default()
        };
        let ask = Scripted::new(&[
            &format!(
                "[{{\"url\":\"{u}/\",\"severity\":\"medium\",\"message\":\"no price\"}},{{\"url\":\"https://elsewhere.test/\",\"message\":\"ignored\"}}]"
            ),
            "Özet: iyi.",
        ]);
        let events = Mutex::new(Vec::new());
        let t = run(&opts, Some(&ask), "tr", &|e| events.lock().unwrap().push(e))
            .await
            .unwrap();
        let rules: Vec<&str> = t.report.findings.iter().map(|f| f.rule.as_str()).collect();
        assert!(rules.contains(&"seo.redirect-chain"), "{rules:?}");
        assert!(rules.contains(&"seo.noindex"));
        assert!(
            rules.contains(&"seo.log.crawl-waste-errors") || rules.contains(&"seo.log.error-url")
        );
        assert!(rules.contains(&"seo.agent.eticaret"));
        assert!(
            !t.report
                .findings
                .iter()
                .any(|f| f.file.to_string_lossy().contains("elsewhere"))
        );
        assert!(t.conflicts.iter().any(|(cu, _)| cu.ends_with("/gizli")));
        assert!(t.htaccess.contains("Redirect 301 /eski "));
        assert_eq!(t.summary.as_deref(), Some("Özet: iyi."));
        assert!(!t.priorities.is_empty());
        assert!(t.fixes_markdown.contains("## "));
        {
            let ev = events.lock().unwrap();
            assert!(
                ev.iter()
                    .any(|e| matches!(e, Event::Skipped { agent, .. } if agent == "competitor"))
            );
            assert!(
                ev.iter()
                    .any(|e| matches!(e, Event::Done { agent, .. } if agent == "technical"))
            );
        }
        // Filtered team: only the content agent's findings.
        let only = TeamOptions {
            only: vec!["içerik".into()],
            log: None,
            ..opts.clone()
        };
        let t2 = run(&only, None, "tr", &|_| {}).await.unwrap();
        assert!(
            t2.report
                .findings
                .iter()
                .all(|f| owner(&f.rule) == "content" || f.rule.starts_with("seo.agent."))
        );
        assert!(t2.diff.is_some());
    }
}
