//! Content pipeline (spec §29.7): brief → outline → draft → SEO check → human review → publish.
//!
//! Every stage is a Markdown file in `<project>/.kor/seo/content/<slug>/` that the user can edit
//! before the next stage reads it. The model writes brief, outline and draft (cost router route
//! `docs`); the SEO check is deterministic. Publishing needs an approval recorded against the
//! draft's SHA-256, so a draft edited after approval must be checked and approved again; it then
//! goes to WordPress as a draft (or published with `--publish`), or is exported to a file.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kor_runtime::analysis::{Finding, Report, Severity};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use super::model::{Ask, markdown_to_html, unfence};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    New,
    Brief,
    Outline,
    Draft,
    Checked,
    Approved,
    Published,
}

impl Stage {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "brief" => Self::Brief,
            "outline" => Self::Outline,
            "draft" => Self::Draft,
            "check" | "checked" => Self::Checked,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub slug: String,
    pub keyword: String,
    /// `tr`, `en`, `fr`, … (the language the text is written in).
    pub lang: String,
    pub stage: Stage,
    pub created: String,
    /// SHA-256 of `draft.md` at approval.
    #[serde(default)]
    pub approved_sha: Option<String>,
    #[serde(default)]
    pub published: Option<serde_json::Value>,
    #[serde(default)]
    pub model: Option<String>,
    /// Update mode: the existing page this piece rewrites (spec §29.7 "move this post to 2026").
    #[serde(default)]
    pub source: Option<String>,
}

/// A page the user named as a current top result (fetched politely, facts only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerpPage {
    pub url: String,
    pub title: String,
    pub headings: Vec<String>,
    pub word_count: usize,
}

pub fn root_dir(root: &Path) -> PathBuf {
    root.join(".kor").join("seo").join("content")
}

pub fn dir(root: &Path, slug: &str) -> PathBuf {
    root_dir(root).join(slug)
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn slug(keyword: &str) -> String {
    super::audit::tr_slug(keyword.trim())
        .replace(['/', '.'], "-")
        .trim_matches('-')
        .chars()
        .take(80)
        .collect()
}

fn save(root: &Path, st: &State) -> Result<(), String> {
    let d = dir(root, &st.slug);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    std::fs::write(
        d.join("state.json"),
        serde_json::to_string_pretty(st).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn load(root: &Path, slug: &str) -> Result<State, String> {
    if !valid_slug(slug) {
        return Err(format!("bad slug: {slug}"));
    }
    let text = std::fs::read_to_string(dir(root, slug).join("state.json"))
        .map_err(|e| format!("{slug}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn list(root: &Path) -> Vec<State> {
    let mut v: Vec<State> = std::fs::read_dir(root_dir(root))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| load(root, &e.file_name().to_string_lossy()).ok())
        .collect();
    v.sort_by(|a, b| b.created.cmp(&a.created));
    v
}

pub fn create(root: &Path, keyword: &str, lang: &str) -> Result<State, String> {
    let s = slug(keyword);
    if !valid_slug(&s) || keyword.chars().count() > 200 {
        return Err("keyword must be 1-200 characters".into());
    }
    if let Ok(st) = load(root, &s) {
        return Ok(st);
    }
    let st = State {
        slug: s,
        keyword: keyword.trim().to_owned(),
        lang: lang.to_owned(),
        stage: Stage::New,
        created: chrono::Utc::now().to_rfc3339(),
        approved_sha: None,
        published: None,
        model: None,
        source: None,
    };
    save(root, &st)?;
    Ok(st)
}

/// Update mode: start a piece from an existing page (its text goes to `source.md`).
pub async fn create_from_url(
    root: &Path,
    page: &str,
    keyword: &str,
    lang: &str,
    user_agent: &str,
) -> Result<State, String> {
    let u = url::Url::parse(page).map_err(|e| format!("{page}: {e}"))?;
    let f = super::fetch::Fetcher::new(user_agent, std::time::Duration::from_millis(200), true)?;
    if !f.allowed(&u).await {
        return Err("robots.txt disallows this page".into());
    }
    let r = match f.follow(&u).await? {
        (_, super::fetch::Chain::Done(r)) if r.status == 200 => r,
        (_, super::fetch::Chain::Done(r)) => return Err(format!("HTTP {}", r.status)),
        _ => return Err("redirect loop".into()),
    };
    let p = super::html::parse(&r.body);
    let mut md = format!(
        "# {}\n\n",
        p.titles.first().map_or("", |t| t.value.as_str())
    );
    for h in &p.headings {
        let _ = writeln!(
            md,
            "{} {}",
            "#".repeat(usize::from(h.value.0).clamp(1, 6)),
            h.value.1
        );
    }
    md.push('\n');
    md.push_str(&p.text);
    let mut st = create(root, keyword, lang)?;
    write(root, &st.slug, "source.md", &md)?;
    st.source = Some(r.url);
    save(root, &st)?;
    Ok(st)
}

/// JSON-LD for the draft: Article, plus FAQPage when at least two headings are questions
/// (each followed by its answer paragraph).
pub fn schema(keyword: &str, lang: &str, draft: &str) -> serde_json::Value {
    let (title, desc, body) = split_draft(draft);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut graph = vec![serde_json::json!({
        "@type": "Article", "headline": title, "description": desc, "inLanguage": lang,
        "datePublished": today, "dateModified": today, "keywords": keyword,
    })];
    let lines: Vec<&str> = body.lines().collect();
    let mut qa = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let h = l.trim_start_matches('#').trim();
        if l.starts_with("##") && h.ends_with('?') {
            let answer: String = lines[i + 1..]
                .iter()
                .take_while(|x| !x.starts_with('#'))
                .map(|x| x.trim())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !answer.is_empty() {
                qa.push(serde_json::json!({ "@type": "Question", "name": h, "acceptedAnswer": { "@type": "Answer", "text": answer } }));
            }
        }
    }
    if qa.len() >= 2 {
        graph.push(serde_json::json!({ "@type": "FAQPage", "mainEntity": qa }));
    }
    serde_json::json!({ "@context": "https://schema.org", "@graph": graph })
}

fn read(root: &Path, slug: &str, file: &str) -> String {
    std::fs::read_to_string(dir(root, slug).join(file)).unwrap_or_default()
}

fn write(root: &Path, slug: &str, file: &str, text: &str) -> Result<(), String> {
    std::fs::write(dir(root, slug).join(file), text).map_err(|e| e.to_string())
}

/// `.kor/brand-voice.md` (spec §29.7), if the project has one.
pub fn brand_voice(root: &Path) -> String {
    std::fs::read_to_string(root.join(".kor").join("brand-voice.md")).unwrap_or_default()
}

fn serp_text(serp: &[SerpPage]) -> String {
    let mut s = String::new();
    for p in serp {
        let _ = writeln!(
            s,
            "- {} — \"{}\" ({} words); headings: {}",
            p.url,
            p.title,
            p.word_count,
            p.headings.join(" | ")
        );
    }
    s
}

fn prompt(
    st: &State,
    stage: Stage,
    root: &Path,
    serp: &[SerpPage],
    links: &[(String, String)],
) -> String {
    let voice = brand_voice(root);
    let voice = if voice.trim().is_empty() {
        String::new()
    } else {
        format!("\nBrand voice (follow it):\n{voice}\n")
    };
    let lang = &st.lang;
    let source = read(root, &st.slug, "source.md");
    let update = if source.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\nUPDATE MODE: this replaces an existing article. Keep what is still correct, update outdated facts \
             (write [source needed] where you cannot confirm them), keep the URL topic, and add what the brief says is missing.\n\
             Existing article (from {}):\n{}\n",
            st.source.as_deref().unwrap_or(""),
            source.chars().take(12_000).collect::<String>()
        )
    };
    match stage {
        Stage::Brief => format!(
            "Write an SEO content brief in Markdown for the keyword \"{kw}\" (write in language `{lang}`).\n\
             Sections: ## Search intent, ## Audience, ## Title ideas (3), ## Must-cover subtopics, ## Questions to answer, \
             ## Target length (a single line `Length: <words>`), ## Schema (Article/FAQ/Product…), ## Internal links.\n\
             Pages currently ranking for it (facts gathered by Kor; summarize their coverage, do not copy):\n{serp}\
             Existing pages on our site that could be linked:\n{links}{voice}{update}\nReturn only the Markdown.",
            kw = st.keyword,
            serp = if serp.is_empty() {
                "- (none provided)\n".to_owned()
            } else {
                serp_text(serp)
            },
            links = if links.is_empty() {
                "- (none known)\n".to_owned()
            } else {
                links.iter().map(|(u, t)| format!("- {t}: {u}\n")).collect()
            },
        ),
        Stage::Outline => format!(
            "From this brief, write a detailed article outline in Markdown (H1 then H2/H3 with 1-line notes per section) for \"{kw}\", language `{lang}`.\n\nBrief:\n{brief}\n{voice}\nReturn only the Markdown outline.",
            kw = st.keyword,
            brief = read(root, &st.slug, "brief.md"),
        ),
        _ => format!(
            "Write the full article in Markdown, language `{lang}`, following the outline and brief. Keyword: \"{kw}\".\n\
             Start with exactly two lines:\nTitle: <SEO title, at most 60 characters, with the keyword>\nDescription: <meta description, 120-155 characters>\n\
             then a blank line and the article starting with one `# H1`. Use the keyword naturally (no stuffing), \
             answer the brief's questions, add internal links from the brief where they fit, no invented statistics or quotes \
             (write [source needed] where a fact needs one).\n\nBrief:\n{brief}\n\nOutline:\n{outline}\n{voice}{update}",
            kw = st.keyword,
            brief = read(root, &st.slug, "brief.md"),
            outline = read(root, &st.slug, "outline.md"),
        ),
    }
}

/// Internal pages known from the latest audit of `site` (url, title).
pub fn site_pages(root: &Path, site: Option<&str>) -> Vec<(String, String)> {
    site.map(|s| super::audit::load_pages(root, s))
        .unwrap_or_default()
}

/// Run one model stage (`brief`, `outline`, `draft`) and save its file.
pub async fn run_stage(
    root: &Path,
    slug: &str,
    stage: Stage,
    ask: &dyn Ask,
    serp: &[SerpPage],
    links: &[(String, String)],
) -> Result<State, String> {
    let mut st = load(root, slug)?;
    let (file, need, tokens) = match stage {
        Stage::Brief => ("brief.md", None, 2_000),
        Stage::Outline => ("outline.md", Some("brief.md"), 2_000),
        Stage::Draft => ("draft.md", Some("outline.md"), 8_000),
        _ => return Err("only brief, outline and draft use the model".into()),
    };
    if let Some(n) = need
        && read(root, slug, n).trim().is_empty()
    {
        return Err(format!("{n} is missing: run the previous stage first"));
    }
    let text = ask
        .ask(prompt(&st, stage, root, serp, links), tokens)
        .await?;
    write(root, slug, file, &format!("{}\n", unfence(&text)))?;
    st.stage = stage;
    st.approved_sha = None;
    st.model = Some(ask.id());
    save(root, &st)?;
    Ok(st)
}

/// `Title:` / `Description:` header lines and the body of a draft.
pub fn split_draft(draft: &str) -> (String, String, String) {
    let mut title = String::new();
    let mut desc = String::new();
    let mut body = Vec::new();
    for l in draft.lines() {
        if body.is_empty()
            && title.is_empty()
            && let Some(t) = l.strip_prefix("Title:")
        {
            title = t.trim().to_owned();
        } else if body.is_empty()
            && desc.is_empty()
            && let Some(d) = l.strip_prefix("Description:")
        {
            desc = d.trim().to_owned();
        } else if !(body.is_empty() && l.trim().is_empty()) {
            body.push(l);
        }
    }
    (title, desc, body.join("\n"))
}

/// Deterministic SEO check of `draft.md`; writes `check.md` and moves to `checked`.
pub fn check(root: &Path, slug: &str) -> Result<(State, Report), String> {
    let mut st = load(root, slug)?;
    let draft = read(root, slug, "draft.md");
    if draft.trim().is_empty() {
        return Err("draft.md is missing: run the draft stage first".into());
    }
    let r = check_text(&st.keyword, &draft, &read(root, slug, "brief.md"));
    let mut md = format!("# SEO check: {}\n\n", st.keyword);
    for f in &r.findings {
        let _ = writeln!(
            md,
            "- [{}] line {} `{}` {}{}",
            f.severity.as_str(),
            f.line,
            f.rule,
            f.message,
            f.fix
                .as_deref()
                .map(|x| format!(" — {x}"))
                .unwrap_or_default()
        );
    }
    for n in &r.notes {
        let _ = writeln!(md, "- {n}");
    }
    write(root, slug, "check.md", &md)?;
    let sc = schema(&st.keyword, &st.lang, &draft);
    write(
        root,
        slug,
        "schema.json",
        &serde_json::to_string_pretty(&sc).map_err(|e| e.to_string())?,
    )?;
    let (title, _, _) = split_draft(&draft);
    // Featured image brief; `kor media image` (spec §20) generates it with its own cost line.
    write(
        root,
        slug,
        "image.md",
        &format!(
            "Prompt: editorial photo illustrating \"{title}\", no text in the image, 16:9\nAlt: {title}\nCommand: kor media image \"editorial photo illustrating {title}, no text, 16:9\"\n"
        ),
    )?;
    st.stage = Stage::Checked;
    st.approved_sha = None;
    save(root, &st)?;
    Ok((st, r))
}

/// The checks behind [`check`] (keyword placement, lengths, density, repetition, headings).
#[allow(clippy::too_many_lines)] // a flat list of checks
pub fn check_text(keyword: &str, draft: &str, brief: &str) -> Report {
    let mut r = Report::new("seo-content");
    let (title, desc, body) = split_draft(draft);
    let kw = keyword.to_lowercase();
    let file = PathBuf::from("draft.md");
    let mut push = |rule: &str, sev: Severity, line: u32, msg: String, fix: Option<&str>| {
        r.findings.push(Finding {
            rule: format!("seo.content.{rule}"),
            source: "kor".into(),
            severity: sev,
            file: file.clone(),
            line,
            message: msg,
            fix: fix.map(str::to_owned),
            ..Finding::default()
        });
    };
    let tn = title.chars().count();
    if title.is_empty() {
        push(
            "title-missing",
            Severity::High,
            1,
            "No `Title:` line".into(),
            Some("Start the draft with `Title: …`"),
        );
    } else {
        if tn > super::audit::TITLE_MAX {
            push(
                "title-length",
                Severity::Medium,
                1,
                format!(
                    "Title is {tn} characters (over {})",
                    super::audit::TITLE_MAX
                ),
                None,
            );
        }
        if !title.to_lowercase().contains(&kw) {
            push(
                "title-keyword",
                Severity::Medium,
                1,
                format!("Title does not contain \"{keyword}\""),
                None,
            );
        }
    }
    let dn = desc.chars().count();
    if desc.is_empty() {
        push(
            "description-missing",
            Severity::Medium,
            2,
            "No `Description:` line".into(),
            None,
        );
    } else if !(super::audit::DESC_MIN..=super::audit::DESC_MAX).contains(&dn) {
        push(
            "description-length",
            Severity::Low,
            2,
            format!(
                "Description is {dn} characters ({}–{})",
                super::audit::DESC_MIN,
                super::audit::DESC_MAX
            ),
            None,
        );
    }
    let offset = u32::try_from(draft.lines().count() - body.lines().count()).unwrap_or(0);
    let h1: Vec<(usize, &str)> = body
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("# "))
        .collect();
    match h1.as_slice() {
        [] => push(
            "h1-missing",
            Severity::Medium,
            offset + 1,
            "No `# H1` heading".into(),
            None,
        ),
        [(i, h), rest @ ..] => {
            if !rest.is_empty() {
                push(
                    "h1-multiple",
                    Severity::Low,
                    offset + u32::try_from(rest[0].0 + 1).unwrap_or(0),
                    "More than one H1".into(),
                    None,
                );
            }
            if !h.to_lowercase().contains(&kw) {
                push(
                    "h1-keyword",
                    Severity::Low,
                    offset + u32::try_from(i + 1).unwrap_or(0),
                    format!("H1 does not contain \"{keyword}\""),
                    None,
                );
            }
        }
    }
    if body.lines().filter(|l| l.starts_with("## ")).count() < 2 {
        push(
            "structure",
            Severity::Low,
            offset + 1,
            "Fewer than two `##` sections".into(),
            None,
        );
    }
    let words: Vec<String> = body
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let first100 = words
        .iter()
        .take(100)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if !first100.contains(&kw) {
        push(
            "intro-keyword",
            Severity::Low,
            offset + 1,
            format!("\"{keyword}\" is not in the first 100 words"),
            None,
        );
    }
    let target: usize = brief
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("Length:")
                .map(|x| x.chars().filter(char::is_ascii_digit).collect::<String>())
        })
        .and_then(|d| d.parse().ok())
        .unwrap_or(600);
    if words.len() * 10 < target * 8 {
        push(
            "length",
            Severity::Medium,
            offset + 1,
            format!("{} words; the brief asks for about {target}", words.len()),
            Some("Cover the brief's missing subtopics"),
        );
    }
    let kw_words = kw.split_whitespace().count().max(1);
    let hits = body.to_lowercase().matches(&kw).count();
    #[allow(clippy::cast_precision_loss)]
    let density = if words.is_empty() {
        0.0
    } else {
        (hits * kw_words) as f64 * 100.0 / words.len() as f64
    };
    if density > 2.5 {
        push(
            "keyword-stuffing",
            Severity::Medium,
            offset + 1,
            format!("Keyword density {density:.1}% (over 2.5%)"),
            Some("Use synonyms and natural phrasing"),
        );
    }
    // Repetition / filler: the same sentence twice.
    let mut seen = std::collections::HashMap::new();
    for (i, l) in body.lines().enumerate() {
        for s in l
            .split(['.', '!', '?'])
            .map(str::trim)
            .filter(|s| s.split_whitespace().count() >= 6)
        {
            let k = s.to_lowercase();
            if let Some(first) = seen.insert(k, i) {
                push(
                    "repetition",
                    Severity::Low,
                    offset + u32::try_from(i + 1).unwrap_or(0),
                    format!(
                        "Sentence repeated (first on line {}): \"{s}\"",
                        offset + u32::try_from(first + 1).unwrap_or(0)
                    ),
                    None,
                );
            }
        }
    }
    let unsourced = body.matches("[source needed]").count();
    if unsourced > 0 {
        push(
            "claims",
            Severity::Medium,
            offset + 1,
            format!("{unsourced} statements marked [source needed]"),
            Some("Add a source or remove the claim before publishing"),
        );
    }
    r.notes.push(format!(
        "{} words, keyword density {density:.1}%, title {tn} chars, description {dn} chars",
        words.len()
    ));
    r.normalize();
    r
}

pub fn draft_sha(root: &Path, slug: &str) -> String {
    format!(
        "{:x}",
        sha2::Sha256::digest(read(root, slug, "draft.md").as_bytes())
    )
}

/// Human review: record approval of the current draft.
pub fn approve(root: &Path, slug: &str) -> Result<State, String> {
    let mut st = load(root, slug)?;
    if st.stage < Stage::Checked {
        return Err("run the SEO check before approving".into());
    }
    st.approved_sha = Some(draft_sha(root, slug));
    st.stage = Stage::Approved;
    save(root, &st)?;
    Ok(st)
}

/// Where an approved piece goes.
#[derive(Debug)]
pub enum Target<'a> {
    WordPress {
        wp: &'a super::wp::Wp,
        publish: bool,
    },
    Export(&'a Path),
}

/// Publish or export an approved draft. Refuses a draft changed since approval.
pub async fn publish(root: &Path, slug: &str, target: Target<'_>) -> Result<State, String> {
    let mut st = load(root, slug)?;
    if st.stage < Stage::Approved
        || st.approved_sha.as_deref() != Some(draft_sha(root, slug).as_str())
    {
        return Err(
            "not approved (or the draft changed after approval): check and approve it first".into(),
        );
    }
    let (title, desc, body) = split_draft(&read(root, slug, "draft.md"));
    // The H1 becomes the WordPress title; the body keeps the rest.
    let body_wo_h1: String = body
        .lines()
        .filter(|l| !l.starts_with("# "))
        .collect::<Vec<_>>()
        .join("\n");
    match target {
        Target::WordPress { wp, publish } => {
            let status = if publish { "publish" } else { "draft" };
            let schema = read(root, slug, "schema.json");
            let mut html = markdown_to_html(&body_wo_h1);
            if serde_json::from_str::<serde_json::Value>(&schema).is_ok() {
                // Kept by WordPress only for users with `unfiltered_html` (administrators).
                let _ = write!(
                    html,
                    "\n<script type=\"application/ld+json\">{}</script>\n",
                    schema.replace("</", "<\\/")
                );
            }
            let r = wp.create_post(&title, &html, &desc, status, None).await?;
            st.published = Some(
                serde_json::json!({ "wordpress": { "id": r["id"], "link": r["link"], "status": r["status"] } }),
            );
            if publish {
                st.stage = Stage::Published;
            }
        }
        Target::Export(path) => {
            let text = format!(
                "---\ntitle: \"{}\"\ndescription: \"{}\"\nkeyword: \"{}\"\n---\n\n{body}\n",
                title.replace('"', "'"),
                desc.replace('"', "'"),
                st.keyword.replace('"', "'")
            );
            std::fs::write(path, text).map_err(|e| e.to_string())?;
            st.published = Some(serde_json::json!({ "export": path.to_string_lossy() }));
            st.stage = Stage::Published;
        }
    }
    save(root, &st)?;
    Ok(st)
}

/// Fetch the user's list of ranking pages (robots-respecting) and keep the facts.
pub async fn serp_pages(urls: &[String], user_agent: &str) -> Vec<SerpPage> {
    let Ok(f) = super::fetch::Fetcher::new(user_agent, std::time::Duration::from_secs(1), true)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for u in urls.iter().take(10) {
        let Ok(url) = url::Url::parse(u) else {
            continue;
        };
        if !f.allowed(&url).await {
            continue;
        }
        if let Ok((_, super::fetch::Chain::Done(r))) = f.follow(&url).await
            && r.status == 200
        {
            let p = super::html::parse(&r.body);
            out.push(SerpPage {
                url: u.clone(),
                title: p
                    .titles
                    .first()
                    .map(|t| t.value.clone())
                    .unwrap_or_default(),
                headings: p
                    .headings
                    .iter()
                    .filter(|h| h.value.0 <= 3)
                    .map(|h| h.value.1.clone())
                    .take(30)
                    .collect(),
                word_count: p.word_count,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::model::testing::Scripted;
    use super::*;

    const DRAFT: &str = "Title: Yurtdışı kargo rehberi: ücretler ve süreler\nDescription: Yurtdışı kargo gönderirken ücretler, süreler ve gümrük kuralları hakkında bilmeniz gereken her şey tek rehberde toplandı.\n\n# Yurtdışı kargo rehberi\n\nYurtdışı kargo göndermek kolaydır.\n\n## Ücretler\n\nFiyatlar değişir [source needed].\n\n## Süreler\n\nTeslim süresi ülkeye göre değişir ve takip edilebilir.\n";

    #[tokio::test]
    async fn pipeline_requires_approval_of_current_draft() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".kor")).unwrap();
        std::fs::write(root.join(".kor/brand-voice.md"), "Samimi, kısa cümleler.").unwrap();
        let st = create(root, "Yurtdışı Kargo", "tr").unwrap();
        assert_eq!(st.slug, "yurtdisi-kargo");
        let ask = Scripted::new(&[
            "## Search intent\nLength: 100\n",
            "# Outline",
            &format!("```markdown\n{DRAFT}```"),
        ]);
        assert!(
            run_stage(root, &st.slug, Stage::Outline, &ask, &[], &[])
                .await
                .is_err()
        );
        run_stage(
            root,
            &st.slug,
            Stage::Brief,
            &ask,
            &[SerpPage {
                url: "https://r.test".into(),
                title: "Rakip".into(),
                ..SerpPage::default()
            }],
            &[],
        )
        .await
        .unwrap();
        run_stage(root, &st.slug, Stage::Outline, &ask, &[], &[])
            .await
            .unwrap();
        run_stage(root, &st.slug, Stage::Draft, &ask, &[], &[])
            .await
            .unwrap();
        let prompts = ask.prompts.lock().unwrap().clone();
        assert!(prompts[0].contains("Rakip") && prompts[0].contains("Samimi"));
        assert!(prompts[2].contains("# Outline"));
        assert!(approve(root, &st.slug).is_err());
        let (_, r) = check(root, &st.slug).unwrap();
        assert!(r.findings.iter().any(|f| f.rule == "seo.content.claims"));
        assert!(
            !r.findings
                .iter()
                .any(|f| f.rule == "seo.content.title-keyword")
        );
        let out = root.join("out.md");
        assert!(publish(root, &st.slug, Target::Export(&out)).await.is_err());
        approve(root, &st.slug).unwrap();
        std::fs::write(
            super::dir(root, &st.slug).join("draft.md"),
            format!("{DRAFT}\nedited"),
        )
        .unwrap();
        assert!(publish(root, &st.slug, Target::Export(&out)).await.is_err());
        check(root, &st.slug).unwrap();
        approve(root, &st.slug).unwrap();
        let st = publish(root, &st.slug, Target::Export(&out)).await.unwrap();
        assert_eq!(st.stage, Stage::Published);
        assert!(
            std::fs::read_to_string(out)
                .unwrap()
                .starts_with("---\ntitle: \"Yurtdışı kargo")
        );
        assert_eq!(list(root).len(), 1);
    }

    #[test]
    fn schema_article_and_faq() {
        let d = "Title: T\nDescription: D\n\n# H\n\n## Ne kadar sürer?\n\nÜç gün.\n\n## Ücret nedir?\n\nDeğişir.\n";
        let v = schema("k", "tr", d);
        assert_eq!(v["@graph"][0]["headline"], "T");
        assert_eq!(
            v["@graph"][1]["mainEntity"][0]["acceptedAnswer"]["text"],
            "Üç gün."
        );
        let mut e = super::super::rich::Eligibility::default();
        let issues = super::super::rich::check(&v.to_string(), &mut e);
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, super::super::rich::Issue::MissingRequired { .. }))
        );
    }

    #[tokio::test]
    async fn update_mode_reads_the_existing_page() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let s = MockServer::start().await;
        Mock::given(method("GET")).and(path("/yazi")).respond_with(ResponseTemplate::new(200).set_body_raw("<html><head><title>Eski yazı</title></head><body><h2>2024 fiyatları</h2><p>metin</p></body></html>", "text/html")).mount(&s).await;
        let dir = tempfile::tempdir().unwrap();
        let st = create_from_url(
            dir.path(),
            &format!("{}/yazi", s.uri()),
            "kargo fiyatları",
            "tr",
            "kor/test",
        )
        .await
        .unwrap();
        assert!(st.source.as_deref().unwrap().ends_with("/yazi"));
        let ask = Scripted::new(&["brief"]);
        run_stage(dir.path(), &st.slug, Stage::Brief, &ask, &[], &[])
            .await
            .unwrap();
        let p = ask.prompts.lock().unwrap()[0].clone();
        assert!(p.contains("UPDATE MODE") && p.contains("2024 fiyatları"));
    }

    #[test]
    fn checks_catch_stuffing_and_repetition() {
        let body = "Title: x\nDescription: y\n\n# kargo\n\nkargo kargo kargo kargo kargo. Bu cümle tekrar eden bir dolgu cümlesidir aslında. Bu cümle tekrar eden bir dolgu cümlesidir aslında.\n";
        let r = check_text("kargo", body, "");
        let rules: Vec<&str> = r.findings.iter().map(|f| f.rule.as_str()).collect();
        assert!(rules.contains(&"seo.content.keyword-stuffing"));
        assert!(rules.contains(&"seo.content.repetition"));
        assert!(rules.contains(&"seo.content.title-keyword"));
    }
}
