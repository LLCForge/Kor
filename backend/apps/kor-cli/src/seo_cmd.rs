//! `kor seo …` (spec §29.5–29.10, §29.15, §29.16; ADR 0055). Thin CLI over `kor_web::seo`:
//! `audit`, `gsc`, `content`, `meta`, `logs`, `competitors`, `agents`, `opportunities`.

#![allow(clippy::many_single_char_names)] // l / a / c / t / s: the file's short names

use std::collections::HashMap;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand, ValueEnum};
use kor_i18n::{Localizer, args};
use kor_runtime::analysis::{Format, Report, render};
use kor_secrets::SecretStore;
use kor_web::seo::{
    agents, audit, backlinks, bing, competitors, content, google, logs, meta, model, notify,
    opportunity, wp,
};

use crate::Ctx;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DepthArg {
    Quick,
    Full,
    Forensic,
}

impl DepthArg {
    fn get(self) -> audit::Depth {
        match self {
            Self::Quick => audit::Depth::Quick,
            Self::Full => audit::Depth::Full,
            Self::Forensic => audit::Depth::Forensic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutArg {
    Markdown,
    Json,
    Sarif,
    Html,
    Pdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Every {
    Daily,
    Weekly,
}

impl Every {
    fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct SeoArgs {
    #[command(subcommand)]
    pub command: SeoSub,
}

#[derive(Debug, Clone, Args)]
pub struct AuditArgs {
    /// Site or page URL, or a sitemap URL.
    pub target: String,
    #[arg(long, value_enum, default_value = "quick")]
    pub depth: DepthArg,
    #[arg(long)]
    pub max_pages: Option<usize>,
    #[arg(long, default_value_t = 500)]
    pub delay_ms: u64,
    #[arg(long)]
    pub no_vitals: bool,
    #[arg(long, value_enum, default_value = "markdown")]
    pub format: OutArg,
    #[arg(short, long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub brand: Option<String>,
    #[arg(long, value_enum)]
    pub schedule: Option<Every>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum GscSub {
    /// Store your own Google OAuth client (Desktop app JSON), or show the setup steps.
    Setup {
        #[arg(long)]
        client_json: Option<PathBuf>,
    },
    Login,
    Logout,
    Status,
    Sites,
    Query {
        site: String,
        #[arg(long, default_value_t = 28)]
        days: u32,
        #[arg(long, default_value = "query")]
        dims: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long)]
        json: bool,
    },
    Drops {
        site: String,
        #[arg(long, default_value_t = 7)]
        days: u32,
        #[arg(long, default_value_t = 20)]
        top: usize,
        #[arg(long)]
        json: bool,
        /// Notify when total clicks fell by at least this percent.
        #[arg(long)]
        alert: Option<f64>,
        #[arg(long)]
        explain: bool,
        #[arg(long, value_enum)]
        schedule: Option<Every>,
    },
    Ga {
        property: String,
        #[arg(long, default_value_t = 28)]
        days: u32,
        #[arg(long, default_value = "sessions,keyEvents")]
        metrics: String,
        #[arg(long, default_value = "landingPage")]
        dims: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ContentSub {
    New {
        keyword: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        from_url: Option<String>,
    },
    Run {
        slug: String,
        #[arg(long, default_value = "all")]
        stage: String,
        #[arg(long = "serp")]
        serp: Vec<String>,
        #[arg(long)]
        site: Option<String>,
        #[arg(short, long)]
        model: Option<String>,
    },
    List,
    Show {
        slug: String,
    },
    Approve {
        slug: String,
    },
    Publish {
        slug: String,
        #[arg(long)]
        wp: Option<String>,
        #[arg(long)]
        publish: bool,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum MetaSub {
    Generate {
        /// Site URL to crawl (or use --from).
        url: Option<String>,
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long, default_value_t = 20)]
        sample: usize,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        all_images: bool,
        #[arg(long, value_enum, default_value = "full")]
        depth: DepthArg,
        #[arg(short, long, default_value = "kor-meta.csv")]
        out: PathBuf,
        #[arg(long)]
        lang: Option<String>,
        #[arg(short, long)]
        model: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    Check {
        csv: PathBuf,
    },
    Apply {
        csv: PathBuf,
        #[arg(long)]
        wp: String,
        #[arg(long, default_value = "yoast")]
        keys: String,
        #[arg(long)]
        yes: bool,
    },
    /// `OpenCart` write-back: SQL apply and undo scripts from the review CSV.
    Opencart {
        csv: PathBuf,
        #[arg(long, default_value = "oc_")]
        prefix: String,
        #[arg(long, default_value_t = 1)]
        language: u32,
        #[arg(long, default_value_t = 4)]
        oc_version: u8,
        #[arg(short, long, default_value = "kor-opencart")]
        out: PathBuf,
    },
    Undo {
        file: PathBuf,
        #[arg(long)]
        wp: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum CompSub {
    Add {
        name: String,
        #[arg(required = true)]
        urls: Vec<String>,
        #[arg(long)]
        ours: Option<String>,
    },
    Remove {
        name: String,
    },
    List,
    Run {
        name: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        notify: bool,
    },
    Schedule {
        #[arg(value_enum)]
        every: Every,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum BingSub {
    Setup {
        #[arg(long)]
        key: Option<String>,
    },
    Query {
        site: String,
        #[arg(long)]
        json: bool,
    },
    Pages {
        site: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SeoSub {
    Audit(AuditArgs),
    Verify {
        target: String,
        #[arg(long)]
        no_vitals: bool,
    },
    Bing {
        #[command(subcommand)]
        command: BingSub,
    },
    Gsc {
        #[command(subcommand)]
        command: GscSub,
    },
    Content {
        #[command(subcommand)]
        command: ContentSub,
    },
    Meta {
        #[command(subcommand)]
        command: MetaSub,
    },
    Logs {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        sitemap: Option<String>,
        #[arg(long)]
        verify_dns: bool,
        #[arg(long, value_enum, default_value = "markdown")]
        format: OutArg,
    },
    Competitors {
        #[command(subcommand)]
        command: CompSub,
    },
    Agents {
        site: String,
        #[arg(long, value_enum, default_value = "quick")]
        depth: DepthArg,
        #[arg(long, value_delimiter = ',')]
        agents: Vec<String>,
        #[arg(long)]
        log: Option<PathBuf>,
        #[arg(long)]
        verify_dns: bool,
        #[arg(long)]
        fix: bool,
        #[arg(long)]
        no_model: bool,
        #[arg(long)]
        notify: bool,
        #[arg(long, value_enum)]
        schedule: Option<Every>,
        #[arg(long)]
        json: bool,
        #[arg(short, long)]
        model: Option<String>,
    },
    Opportunities {
        #[arg(required_unless_present = "keywords")]
        site: Option<String>,
        #[arg(long, value_name = "CSV")]
        keywords: Option<std::path::PathBuf>,
        #[arg(long, default_value_t = 28)]
        days: u32,
        #[arg(long, default_value = "query")]
        dims: String,
        #[arg(long, default_value_t = 30)]
        top: usize,
        #[arg(long)]
        cached: bool,
        #[arg(long)]
        explain: bool,
        #[arg(long)]
        json: bool,
    },
    Backlinks {
        #[arg(required = true)]
        domains: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    Competition {
        keyword: String,
        #[arg(long = "serp", required = true)]
        serp: Vec<String>,
        #[arg(long)]
        json: bool,
    },
}

/// Backlink numbers of `domain` from every connected source (keys in the vault or environment).
async fn backlinks_for(
    st: Option<&dyn SecretStore>,
    http: &kor_web::site::http::Http,
    domain: &str,
) -> (Vec<backlinks::Backlinks>, Vec<String>, bool) {
    let get = |n: &str| {
        kor_web::site::secret(st, n)
            .ok()
            .map(|s| kor_secrets::ExposeSecret::expose_secret(&s).to_owned())
    };
    let (mut out, mut errors, mut any) = (Vec::new(), Vec::new(), false);
    let mut take = |r: Result<backlinks::Backlinks, String>| match r {
        Ok(b) => out.push(b),
        Err(e) => errors.push(e),
    };
    if let Some(k) = get("AHREFS_API_KEY") {
        any = true;
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        take(backlinks::ahrefs(http, backlinks::AHREFS, &k, domain, &date).await);
    }
    if let Some(k) = get("SEMRUSH_API_KEY") {
        any = true;
        take(backlinks::semrush(http, backlinks::SEMRUSH, &k, domain).await);
    }
    if let (Some(id), Some(s)) = (get("MOZ_ACCESS_ID"), get("MOZ_SECRET_KEY")) {
        any = true;
        take(backlinks::moz(http, backlinks::MOZ, &id, &s, domain).await);
    }
    (out, errors, any)
}

async fn backlinks_cmd(ctx: &Ctx, domains: &[String], json: bool) -> bool {
    let l = &ctx.l;
    let st = store(ctx).ok();
    let http = match kor_web::site::http::Http::new() {
        Ok(h) => h,
        Err(e) => return fail(l, e),
    };
    let mut all = Vec::new();
    let mut ok = true;
    for d in domains {
        let (rows, errors, any) = backlinks_for(st.as_deref(), &http, d).await;
        if !any {
            return fail(l, l.t("seo-backlinks-none"));
        }
        for e in errors {
            ok = false;
            eprintln!("{}", l.t_args("seo-error", &args!("error" => e)));
        }
        all.extend(rows);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
    } else {
        let n = |v: Option<u64>| v.map_or_else(|| "-".to_owned(), |x| x.to_string());
        for b in &all {
            println!(
                "{:<28} {:<8} {:>6} {:>12} {:>10}",
                b.domain,
                b.source,
                b.authority
                    .map_or_else(|| "-".to_owned(), |a| format!("{a:.0}")),
                n(b.backlinks),
                n(b.ref_domains)
            );
        }
    }
    ok
}

async fn competition_cmd(ctx: &Ctx, keyword: &str, serp: &[String], json: bool) -> bool {
    let l = &ctx.l;
    let pages = content::serp_pages(serp, &user_agent()).await;
    if pages.is_empty() {
        return fail(l, l.t("seo-competition-no-pages"));
    }
    // Authority per ranking domain from the first backlink source that answers.
    let mut auth = HashMap::new();
    let st = store(ctx).ok();
    if let Ok(http) = kor_web::site::http::Http::new() {
        for p in &pages {
            let Ok(d) = backlinks::domain_of(&p.url) else {
                continue;
            };
            if auth.contains_key(&d) {
                continue;
            }
            let (rows, _, any) = backlinks_for(st.as_deref(), &http, &d).await;
            if !any {
                break;
            }
            if let Some(a) = rows.iter().find_map(|b| b.authority) {
                auth.insert(d, a);
            }
        }
    }
    let c = opportunity::competition(keyword, &pages, &auth);
    if json {
        println!("{}", serde_json::to_string_pretty(&c).unwrap_or_default());
        return true;
    }
    println!(
        "{}",
        l.t_args(
            "seo-competition",
            &args!(
                "keyword" => keyword.to_owned(),
                "difficulty" => format!("{:.1}", c.difficulty),
                "n" => c.results,
                "authority" => if c.authority_known { format!("{:.2}", c.authority) } else { "?".to_owned() },
                "title" => format!("{:.2}", c.title_match),
                "depth" => format!("{:.2}", c.depth),
                "ugc" => format!("{:.2}", c.ugc)
            )
        )
    );
    println!("{}", opportunity::COMPETITION_FORMULA);
    true
}

/// Localize the whole tree: `cmd-seo[-sub…]` for every command, `arg-seo-<id>` for every arg.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    loc(c, l, "seo")
}

fn loc(mut c: clap::Command, l: &Localizer, path: &str) -> clap::Command {
    c = c.about(l.t(&format!("cmd-{path}")));
    let ids: Vec<String> = c
        .get_arguments()
        .map(|a| a.get_id().to_string())
        .filter(|id| !matches!(id.as_str(), "help" | "version"))
        .collect();
    for id in ids {
        let key = format!("arg-seo-{}", id.replace('_', "-"));
        c = c.mut_arg(id, |a| a.help(l.t(&key)));
    }
    let subs: Vec<String> = c
        .get_subcommands()
        .map(|s| s.get_name().to_owned())
        .collect();
    for s in subs {
        let p = format!("{path}-{s}");
        c = c.mut_subcommand(s, |sc| loc(sc, l, &p));
    }
    c
}

fn user_agent() -> String {
    format!("kor/{}", env!("CARGO_PKG_VERSION"))
}

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("seo-error", &args!("error" => e.to_string()))
    );
    false
}

fn store(ctx: &Ctx) -> Result<Box<dyn SecretStore>, String> {
    let cfg = ctx
        .config
        .as_ref()
        .map(|c| c.config.vault.clone())
        .unwrap_or_default();
    let env_set = std::env::var(kor_runtime::secure::PASSPHRASE_ENV).is_ok_and(|v| !v.is_empty());
    let pass = if cfg.backend == kor_config::VaultBackend::File && !env_set {
        crate::auth::read_secret(&ctx.l.t("vault-passphrase")).map_err(|e| e.to_string())?
    } else {
        None
    };
    kor_runtime::secure::vault_store(&cfg, pass).map_err(|e| e.to_string())
}

fn ask_model(ctx: &Ctx, job: model::Job, m: Option<&str>) -> Result<model::ProviderAsk, String> {
    let loaded = ctx.config.as_ref().map_err(ToString::to_string)?;
    let a = model::ask_for(&ctx.l, loaded, job, m)?;
    eprintln!(
        "{}",
        ctx.l
            .t_args("seo-model", &args!("model" => model::Ask::id(&a)))
    );
    Ok(a)
}

fn lang_code(l: &Localizer) -> String {
    l.lang().code().to_owned()
}

/// One progress line that animates the ember frame on every update (§16.1).
fn spinner(l: &Localizer) -> impl Fn(&str) + Sync + '_ {
    let tty = std::io::stderr().is_terminal();
    let tick = std::sync::atomic::AtomicUsize::new(0);
    move |s: &str| {
        if tty {
            let short: String = s.chars().take(100).collect();
            let i = tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let frame = crate::brand::ember_frame(i, crate::brand::Depth::detect());
            eprint!("\r\x1b[2K{frame} {} {short}", l.t("seo-working"));
        }
    }
}

fn end_spinner() {
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[2K");
    }
}

fn schedule(
    l: &Localizer,
    input: Result<serde_json::Map<String, serde_json::Value>, String>,
) -> bool {
    let res = input.and_then(|m| {
        let path = kor_runtime::routines::file().ok_or("no data directory")?;
        kor_runtime::routines::add(&path, &m, kor_runtime::cost::now_ms())
    });
    match res {
        Ok(v) => {
            println!(
                "{}",
                l.t_args(
                    "seo-scheduled",
                    &args!("name" => v["name"].as_str().unwrap_or("").to_owned())
                )
            );
            true
        }
        Err(e) => fail(l, e),
    }
}

pub fn run(ctx: &Ctx, a: SeoArgs) -> anyhow::Result<bool> {
    let rt = crate::providers::runtime()?;
    let root = kor_runtime::project_root()?;
    Ok(match a.command {
        SeoSub::Audit(x) => rt.block_on(audit_cmd(ctx, &root, &x)),
        SeoSub::Verify { target, no_vitals } => {
            rt.block_on(verify_cmd(ctx, &root, &target, !no_vitals))
        }
        SeoSub::Bing { command } => rt.block_on(bing_cmd(ctx, command)),
        SeoSub::Gsc { command } => rt.block_on(gsc_cmd(ctx, &root, command)),
        SeoSub::Content { command } => rt.block_on(content_cmd(ctx, &root, command)),
        SeoSub::Meta { command } => rt.block_on(meta_cmd(ctx, &root, command)),
        SeoSub::Logs {
            files,
            sitemap,
            verify_dns,
            format,
        } => rt.block_on(logs_cmd(
            ctx,
            &files,
            sitemap.as_deref(),
            verify_dns,
            format,
        )),
        SeoSub::Competitors { command } => rt.block_on(comp_cmd(ctx, &root, command)),
        SeoSub::Agents {
            site,
            depth,
            agents,
            log,
            verify_dns,
            fix,
            no_model,
            notify,
            schedule: every,
            json,
            model,
        } => {
            if let Some(e) = every {
                return Ok(schedule(&ctx.l, agents::routine(&root, &site, e.as_str())));
            }
            rt.block_on(agents_cmd(
                ctx,
                &root,
                AgentsRun {
                    site,
                    depth,
                    agents,
                    log,
                    verify_dns,
                    fix,
                    no_model,
                    notify,
                    json,
                    model,
                },
            ))
        }
        SeoSub::Opportunities {
            site,
            keywords,
            days,
            dims,
            top,
            cached,
            explain,
            json,
        } => match keywords {
            // §29.16: a keyword list (Search Console "Queries" export or one's own CSV) instead of the API.
            Some(file) => rt.block_on(opp_cmd(
                ctx,
                Source::Csv(&file),
                site.as_deref().unwrap_or("csv"),
                days,
                "query",
                top,
                explain,
                json,
            )),
            None => rt.block_on(opp_cmd(
                ctx,
                if cached { Source::Cached } else { Source::Api },
                site.as_deref().unwrap_or_default(),
                days,
                &dims,
                top,
                explain,
                json,
            )),
        },
        SeoSub::Backlinks { domains, json } => rt.block_on(backlinks_cmd(ctx, &domains, json)),
        SeoSub::Competition {
            keyword,
            serp,
            json,
        } => rt.block_on(competition_cmd(ctx, &keyword, &serp, json)),
    })
}

fn site_url(s: &str) -> String {
    if s.contains("://") {
        s.to_owned()
    } else {
        format!("https://{s}/")
    }
}

/// Print or write a report in the asked format (HTML/PDF via `audit::to_html`).
async fn emit(
    l: &Localizer,
    r: &Report,
    title: &str,
    format: OutArg,
    out: Option<&Path>,
    brand: Option<&str>,
) -> bool {
    let labels = [
        title.to_owned(),
        l.t("seo-col-severity"),
        l.t("seo-col-url"),
        l.t("seo-col-line"),
        l.t("seo-col-rule"),
        l.t("seo-col-finding"),
        l.t("seo-col-fix"),
        l.t("seo-col-notes"),
    ];
    let lab: [&str; 8] = std::array::from_fn(|i| labels[i].as_str());
    let bytes: Vec<u8> = match format {
        OutArg::Markdown => render(r, Format::Markdown, l).into_bytes(),
        OutArg::Json => render(r, Format::Json, l).into_bytes(),
        OutArg::Sarif => render(r, Format::Sarif, l).into_bytes(),
        OutArg::Html => audit::to_html(r, title, brand, lab).into_bytes(),
        OutArg::Pdf => match kor_tools::html_to_pdf(&audit::to_html(r, title, brand, lab)).await {
            Ok(b) => b,
            Err(e) => return fail(l, e),
        },
    };
    match out {
        Some(p) => match std::fs::write(p, &bytes) {
            Ok(()) => {
                eprintln!(
                    "{}",
                    l.t_args("seo-written", &args!("path" => kor_tools::display_path(p)))
                );
                true
            }
            Err(e) => fail(l, e),
        },
        None if format == OutArg::Pdf => fail(l, l.t("seo-pdf-needs-out")),
        None => {
            print!("{}", String::from_utf8_lossy(&bytes));
            true
        }
    }
}

async fn audit_cmd(ctx: &Ctx, root: &Path, a: &AuditArgs) -> bool {
    let l = &ctx.l;
    let start = site_url(&a.target);
    if let Some(e) = a.schedule {
        return schedule(l, audit::routine(root, &start, a.depth.get(), e.as_str()));
    }
    let opts = audit::Options {
        start: start.clone(),
        depth: a.depth.get(),
        max_pages: a.max_pages,
        delay: Duration::from_millis(a.delay_ms),
        user_agent: user_agent(),
        vitals: !a.no_vitals,
        seeds: Vec::new(),
    };
    let sp = spinner(l);
    let res = audit::audit(&opts, &sp).await;
    end_spinner();
    let (crawl, report) = match res {
        Ok(x) => x,
        Err(e) => return fail(l, e),
    };
    let _ = audit::save_pages(root, &crawl);
    let prev = audit::save(root, &start, &report).ok().flatten();
    let ok = emit(
        l,
        &report,
        &l.t_args("seo-audit-title", &args!("site" => start.clone())),
        a.format,
        a.out.as_deref(),
        a.brand.as_deref(),
    )
    .await;
    // Rich-result sub-report (§29.16).
    let eligible: Vec<String> = crawl
        .pages
        .iter()
        .filter(|p| !p.rich.eligible.is_empty() || !p.rich.blocked.is_empty())
        .map(|p| {
            let blocked: Vec<String> = p
                .rich
                .blocked
                .iter()
                .map(|(t, f)| format!("{t} (+{})", f.join(", ")))
                .collect();
            format!(
                "  {} — {} {}",
                p.url,
                p.rich.eligible.join(", "),
                if blocked.is_empty() {
                    String::new()
                } else {
                    l.t_args("seo-rich-blocked", &args!("list" => blocked.join("; ")))
                }
            )
        })
        .collect();
    if !eligible.is_empty() && matches!(a.format, OutArg::Markdown) && a.out.is_none() {
        println!("\n## {}\n{}", l.t("seo-rich-title"), eligible.join("\n"));
    }
    if let Some(p) = prev {
        let d = audit::diff(&p, &report);
        eprintln!(
            "{}",
            l.t_args(
                "seo-audit-diff",
                &args!("new" => d.new.len(), "fixed" => d.fixed.len())
            )
        );
    }
    ok
}

async fn send_alert(ctx: &Ctx, title: &str, text: &str) {
    let Ok(loaded) = ctx.config.as_ref() else {
        return;
    };
    for n in notify::alert(&ctx.l, loaded, title, text).await {
        eprintln!("{n}");
    }
}

async fn verify_cmd(ctx: &Ctx, root: &Path, target: &str, vitals: bool) -> bool {
    let l = &ctx.l;
    let start = site_url(target);
    let Some(prev) = audit::latest_report(root, &start) else {
        return fail(l, l.t("seo-verify-no-audit"));
    };
    let opts = audit::Options {
        start: start.clone(),
        vitals,
        user_agent: user_agent(),
        ..audit::Options::default()
    };
    let sp = spinner(l);
    let r = audit::verify(&prev, &opts, &sp).await;
    end_spinner();
    match r {
        Ok(v) => {
            println!("{}", l.t_args("seo-verify-result", &args!("urls" => v.urls, "fixed" => v.fixed.len(), "remaining" => v.remaining.len(), "new" => v.new.len())));
            for f in &v.remaining {
                println!("  · {} {} — {}", f.location(), f.rule, f.message);
            }
            for f in &v.new {
                println!("  + {} {} — {}", f.location(), f.rule, f.message);
            }
            v.remaining.is_empty() && v.new.is_empty()
        }
        Err(e) => fail(l, e),
    }
}

async fn bing_cmd(ctx: &Ctx, c: BingSub) -> bool {
    let l = &ctx.l;
    let st = match store(ctx) {
        Ok(s) => s,
        Err(e) => return fail(l, e),
    };
    let (kind, site, json) = match c {
        BingSub::Setup { key: None } => {
            println!("{}", l.t("seo-bing-steps"));
            return true;
        }
        BingSub::Setup { key: Some(k) } => {
            return match bing::save_key(st.as_ref(), &k) {
                Ok(()) => {
                    println!("{}", l.t("seo-bing-saved"));
                    true
                }
                Err(e) => fail(l, e),
            };
        }
        BingSub::Query { site, json } => ("GetQueryStats", site, json),
        BingSub::Pages { site, json } => ("GetPageStats", site, json),
    };
    match bing::stats(bing::BASE, st.as_ref(), kind, &site).await {
        Ok(rows) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows).unwrap_or_default()
                );
            } else {
                let mut t = vec![vec![
                    String::new(),
                    l.t("seo-col-clicks"),
                    l.t("seo-col-impressions"),
                    l.t("seo-col-position"),
                ]];
                t.extend(rows.iter().take(100).map(|r| {
                    vec![
                        r.key.clone(),
                        format!("{:.0}", r.clicks),
                        format!("{:.0}", r.impressions),
                        format!("{:.1}", r.avg_impression_position),
                    ]
                }));
                crate::providers::table(&t);
            }
            true
        }
        Err(e) => fail(l, e),
    }
}

fn dates(days: u32, offset: u32) -> (String, String) {
    // Search Console data lags about two days.
    let end = chrono::Local::now().date_naive() - chrono::Days::new(u64::from(2 + offset));
    let start = end - chrono::Days::new(u64::from(days.max(1) - 1));
    (
        start.format("%Y-%m-%d").to_string(),
        end.format("%Y-%m-%d").to_string(),
    )
}

fn google_db() -> Option<PathBuf> {
    kor_runtime::kor_home().map(|h| h.join("seo").join("google.sqlite"))
}

fn setup_steps(l: &Localizer) {
    println!("{}", l.t("seo-gsc-setup-steps"));
}

async fn connect(ctx: &Ctx) -> Result<(google::Google, Box<dyn SecretStore>), String> {
    let st = store(ctx)?;
    if google::client(st.as_ref()).is_none() {
        setup_steps(&ctx.l);
        return Err(ctx.l.t("seo-gsc-no-client"));
    }
    let g = google::Google::connect(google::Endpoints::default(), st.as_ref()).await?;
    Ok((g, st))
}

#[allow(clippy::too_many_lines)]
async fn gsc_cmd(ctx: &Ctx, root: &Path, c: GscSub) -> bool {
    let l = &ctx.l;
    match c {
        GscSub::Setup { client_json } => {
            let Some(p) = client_json else {
                setup_steps(l);
                return true;
            };
            let r = std::fs::read_to_string(&p)
                .map_err(|e| e.to_string())
                .and_then(|t| google::parse_client_json(&t))
                .and_then(|c| store(ctx).and_then(|s| google::save_client(s.as_ref(), &c)));
            match r {
                Ok(()) => {
                    println!("{}", l.t("seo-gsc-client-saved"));
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        GscSub::Login => {
            let st = match store(ctx) {
                Ok(s) => s,
                Err(e) => return fail(l, e),
            };
            if google::client(st.as_ref()).is_none() {
                setup_steps(l);
                return fail(l, l.t("seo-gsc-no-client"));
            }
            let open = |u: &str| {
                println!("{}\n{u}", l.t("seo-gsc-open"));
                let _ = crate::graph_cmd::open_in_browser(Path::new(u));
            };
            match google::login(
                &google::Endpoints::default(),
                st.as_ref(),
                &open,
                google::LOGIN_TIMEOUT,
            )
            .await
            {
                Ok(()) => {
                    println!("{}", l.t("seo-gsc-signed-in"));
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        GscSub::Logout => match store(ctx).and_then(|s| google::logout(s.as_ref())) {
            Ok(_) => {
                println!("{}", l.t("seo-gsc-signed-out"));
                true
            }
            Err(e) => fail(l, e),
        },
        GscSub::Status => match store(ctx) {
            Ok(s) => {
                let st = google::status(s.as_ref());
                println!("{}", l.t_args("seo-gsc-status", &args!("client" => st.client.to_string(), "signed" => st.signed_in.to_string())));
                if !st.client {
                    setup_steps(l);
                }
                true
            }
            Err(e) => fail(l, e),
        },
        GscSub::Sites => match connect(ctx).await {
            Ok((g, _)) => match g.sites().await {
                Ok(v) => {
                    for site in &v {
                        println!("{site}");
                    }
                    true
                }
                Err(e) => fail(l, e),
            },
            Err(e) => fail(l, e),
        },
        GscSub::Query {
            site,
            days,
            dims,
            limit,
            json,
        } => {
            let (g, _) = match connect(ctx).await {
                Ok(x) => x,
                Err(e) => return fail(l, e),
            };
            let d: Vec<&str> = dims
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let (s, e) = dates(days, 0);
            match g.search_analytics(&site, &s, &e, &d, limit).await {
                Ok(rows) => {
                    if let Some(db) = google_db() {
                        let _ = google::store_rows(&db, &site, &s, &e, &d, &rows);
                    }
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&rows).unwrap_or_default()
                        );
                    } else {
                        let mut t = vec![vec![
                            dims.clone(),
                            l.t("seo-col-clicks"),
                            l.t("seo-col-impressions"),
                            "CTR".into(),
                            l.t("seo-col-position"),
                        ]];
                        t.extend(rows.iter().map(|r| {
                            vec![
                                r.keys.join(" · "),
                                format!("{:.0}", r.clicks),
                                format!("{:.0}", r.impressions),
                                format!("{:.1}%", r.ctr * 100.0),
                                format!("{:.1}", r.position),
                            ]
                        }));
                        crate::providers::table(&t);
                    }
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        GscSub::Drops {
            site,
            days,
            top,
            json,
            alert,
            explain,
            schedule: every,
        } => {
            if let Some(e) = every {
                return schedule(
                    l,
                    google::drops_routine(root, &site, alert.unwrap_or(20.0), e.as_str()),
                );
            }
            let (g, _) = match connect(ctx).await {
                Ok(x) => x,
                Err(e) => return fail(l, e),
            };
            let (s1, e1) = dates(days, days);
            let (s2, e2) = dates(days, 0);
            let before = g
                .search_analytics(&site, &s1, &e1, &["page"], google::MAX_ROWS)
                .await;
            let now = g
                .search_analytics(&site, &s2, &e2, &["page"], google::MAX_ROWS)
                .await;
            let (before, now) = match (before, now) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => return fail(l, e),
            };
            if let Some(db) = google_db() {
                let _ = google::store_rows(&db, &site, &s1, &e1, &["page"], &before);
                let _ = google::store_rows(&db, &site, &s2, &e2, &["page"], &now);
            }
            let site_host = site.trim_start_matches("sc-domain:");
            let rep = audit::latest_report(root, site_host);
            let d = google::drops(&before, &now, rep.as_ref(), top);
            if let Some(t) = alert
                && let Some((dc, di)) = notify::anomaly(&before, &now, t)
            {
                let text = l.t_args("seo-anomaly", &args!("site" => site.clone(), "clicks" => format!("{dc:.0}"), "impressions" => format!("{di:.0}")));
                eprintln!("{text}");
                send_alert(ctx, &l.t("seo-anomaly-title"), &text).await;
            }
            if explain && !d.is_empty() {
                match ask_model(ctx, model::Job::Writing, None) {
                    Ok(a) => {
                        match model::Ask::ask(&a, google::drops_prompt(&d, &lang_code(l)), 1_500)
                            .await
                        {
                            Ok(t) => println!("## {}\n{}\n", l.t("seo-hypotheses"), t.trim()),
                            Err(e) => eprintln!("{e}"),
                        }
                    }
                    Err(e) => eprintln!("{e}"),
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&d).unwrap_or_default());
                return true;
            }
            println!(
                "{}",
                l.t_args(
                    "seo-drops-head",
                    &args!("a" => format!("{s1}…{e1}"), "b" => format!("{s2}…{e2}"))
                )
            );
            for x in &d {
                let hints: Vec<String> = x
                    .hints
                    .iter()
                    .map(|h| match h.as_str() {
                        "position" | "demand" | "snippet" | "gone" => l.t(&format!("seo-hint-{h}")),
                        other => other.to_owned(),
                    })
                    .collect();
                println!("{}", l.t_args("seo-drop-line", &args!("page" => x.page.clone(), "before" => format!("{:.0}", x.clicks_before), "now" => format!("{:.0}", x.clicks_now),
                    "pos1" => format!("{:.1}", x.position_before), "pos2" => format!("{:.1}", x.position_now), "hints" => hints.join("; "))));
            }
            true
        }
        GscSub::Ga {
            property,
            days,
            metrics,
            dims,
            json,
        } => {
            let (g, _) = match connect(ctx).await {
                Ok(x) => x,
                Err(e) => return fail(l, e),
            };
            let (s, e) = dates(days, 0);
            let m: Vec<&str> = metrics
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let d: Vec<&str> = dims
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            match g.ga_report(&property, &s, &e, &d, &m).await {
                Ok(rows) => {
                    if let Some(db) = google_db() {
                        let _ = google::store_ga(
                            &db,
                            &property,
                            &s,
                            &e,
                            &format!("{dims}|{metrics}"),
                            &rows,
                        );
                    }
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&rows).unwrap_or_default()
                        );
                    } else {
                        let cols: Vec<&str> = d.iter().chain(m.iter()).copied().collect();
                        let mut t = vec![cols.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>()];
                        t.extend(rows.iter().map(|r| {
                            cols.iter()
                                .map(|c| {
                                    r[*c]
                                        .as_str()
                                        .map_or_else(|| r[*c].to_string(), str::to_owned)
                                })
                                .collect()
                        }));
                        crate::providers::table(&t);
                    }
                    true
                }
                Err(e) => fail(l, e),
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn content_cmd(ctx: &Ctx, root: &Path, c: ContentSub) -> bool {
    let l = &ctx.l;
    match c {
        ContentSub::New {
            keyword,
            lang,
            from_url,
        } => match match &from_url {
            Some(u) => {
                content::create_from_url(
                    root,
                    u,
                    &keyword,
                    &lang.clone().unwrap_or_else(|| lang_code(l)),
                    &user_agent(),
                )
                .await
            }
            None => content::create(root, &keyword, &lang.unwrap_or_else(|| lang_code(l))),
        } {
            Ok(st) => {
                println!("{}", l.t_args("seo-content-created", &args!("slug" => st.slug.clone(), "dir" => kor_tools::display_path(&content::dir(root, &st.slug)))));
                true
            }
            Err(e) => fail(l, e),
        },
        ContentSub::Run {
            slug,
            stage,
            serp,
            site,
            model: m,
        } => {
            let stages: Vec<content::Stage> = if stage == "all" {
                vec![
                    content::Stage::Brief,
                    content::Stage::Outline,
                    content::Stage::Draft,
                    content::Stage::Checked,
                ]
            } else if let Some(s) = content::Stage::parse(&stage) {
                vec![s]
            } else {
                return fail(l, l.t("seo-content-bad-stage"));
            };
            let needs_model = stages.iter().any(|s| *s != content::Stage::Checked);
            let ask = if needs_model {
                match ask_model(ctx, model::Job::Writing, m.as_deref()) {
                    Ok(a) => Some(a),
                    Err(e) => return fail(l, e),
                }
            } else {
                None
            };
            let pages = if stages.contains(&content::Stage::Brief) {
                content::serp_pages(&serp, &user_agent()).await
            } else {
                Vec::new()
            };
            let links = content::site_pages(root, site.as_deref());
            for s in stages {
                eprintln!(
                    "{}",
                    l.t_args(
                        "seo-content-stage",
                        &args!("stage" => format!("{s:?}").to_lowercase())
                    )
                );
                let r = if s == content::Stage::Checked {
                    content::check(root, &slug).map(|(st, rep)| {
                        print!("{}", render(&rep, Format::Markdown, l));
                        st
                    })
                } else {
                    content::run_stage(
                        root,
                        &slug,
                        s,
                        ask.as_ref().expect("model stages have a model"),
                        &pages,
                        &links,
                    )
                    .await
                };
                if let Err(e) = r {
                    return fail(l, e);
                }
            }
            println!("{}", l.t_args("seo-content-review", &args!("dir" => kor_tools::display_path(&content::dir(root, &slug)), "slug" => slug)));
            true
        }
        ContentSub::List => {
            let mut t = vec![vec![
                l.t("seo-col-slug"),
                l.t("seo-col-keyword"),
                l.t("seo-col-stage"),
                l.t("seo-col-created"),
            ]];
            t.extend(content::list(root).into_iter().map(|s| {
                vec![
                    s.slug,
                    s.keyword,
                    format!("{:?}", s.stage).to_lowercase(),
                    s.created.chars().take(10).collect(),
                ]
            }));
            crate::providers::table(&t);
            true
        }
        ContentSub::Show { slug } => match content::load(root, &slug) {
            Ok(st) => {
                println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default());
                for f in ["brief.md", "outline.md", "draft.md", "check.md"] {
                    let p = content::dir(root, &slug).join(f);
                    if p.is_file() {
                        println!("  {}", kor_tools::display_path(&p));
                    }
                }
                true
            }
            Err(e) => fail(l, e),
        },
        ContentSub::Approve { slug } => match content::approve(root, &slug) {
            Ok(_) => {
                println!("{}", l.t("seo-content-approved"));
                true
            }
            Err(e) => fail(l, e),
        },
        ContentSub::Publish {
            slug,
            wp: site,
            publish,
            out,
            yes,
        } => {
            let target_text = match (&site, &out) {
                (Some(w), _) => l.t_args("seo-publish-wp", &args!("site" => w.clone(), "status" => if publish { "publish" } else { "draft" })),
                (None, Some(p)) => l.t_args("seo-publish-file", &args!("path" => kor_tools::display_path(p))),
                (None, None) => return fail(l, l.t("seo-publish-needs-target")),
            };
            if !crate::shell_cmd::confirm(l, &target_text, yes) {
                return false;
            }
            let res = match (&site, &out) {
                (Some(w), _) => match store(ctx).and_then(|s| wp::from_profile(w, s.as_ref())) {
                    Ok(client) => {
                        content::publish(
                            root,
                            &slug,
                            content::Target::WordPress {
                                wp: &client,
                                publish,
                            },
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
                (None, Some(p)) => content::publish(root, &slug, content::Target::Export(p)).await,
                (None, None) => unreachable!("checked above"),
            };
            match res {
                Ok(st) => {
                    println!("{}", l.t_args("seo-published", &args!("info" => st.published.map(|v| v.to_string()).unwrap_or_default())));
                    true
                }
                Err(e) => fail(l, e),
            }
        }
    }
}

fn price_note(l: &Localizer, model_id: &str, tokens: (u64, u64)) -> String {
    let prices = kor_runtime::bench::Prices::load(l, crate::catalog_cmd::cache_path().as_deref());
    match prices.cost(model_id, tokens.0, tokens.1) {
        Some(c) => format!("≈ ${:.4}", c.usd()),
        None => l.t("seo-price-unknown"),
    }
}

#[allow(clippy::too_many_lines)]
async fn meta_cmd(ctx: &Ctx, root: &Path, c: MetaSub) -> bool {
    let l = &ctx.l;
    match c {
        MetaSub::Generate {
            url,
            from,
            sample,
            all,
            all_images,
            depth,
            out,
            lang,
            model: m,
            yes,
        } => {
            let mut rows = if let Some(f) = &from {
                match std::fs::read_to_string(f)
                    .map_err(|e| e.to_string())
                    .and_then(|t| meta::from_csv(&t))
                {
                    Ok(r) => r,
                    Err(e) => return fail(l, e),
                }
            } else if let Some(u) = &url {
                let opts = audit::Options {
                    start: site_url(u),
                    depth: depth.get(),
                    vitals: false,
                    user_agent: user_agent(),
                    ..audit::Options::default()
                };
                let sp = spinner(l);
                let c = audit::crawl(&opts, &sp).await;
                end_spinner();
                match c {
                    Ok(c) => meta::collect(&c, all_images),
                    Err(e) => return fail(l, e),
                }
            } else {
                return fail(l, l.t("seo-meta-needs-source"));
            };
            let ask = match ask_model(ctx, model::Job::Bulk, m.as_deref()) {
                Ok(a) => a,
                Err(e) => return fail(l, e),
            };
            let limit = if all { usize::MAX } else { sample };
            let open: Vec<meta::Row> = rows
                .iter()
                .filter(|r| {
                    if r.kind == "image" {
                        r.alt.is_empty()
                    } else {
                        r.title.is_empty()
                    }
                })
                .take(limit)
                .cloned()
                .collect();
            let tokens = meta::estimate_tokens(&open, model::Ask::vision(&ask));
            let q = l.t_args(
                "seo-meta-confirm",
                &args!("n" => open.len(), "cost" => price_note(l, &model::Ask::id(&ask), tokens)),
            );
            if !crate::shell_cmd::confirm(l, &q, yes) {
                return false;
            }
            let progress = |d: usize, t: usize| {
                if std::io::stderr().is_terminal() {
                    eprint!("\r\x1b[2K{d}/{t}");
                }
            };
            let res = meta::generate(
                &mut rows,
                &ask,
                &lang.unwrap_or_else(|| lang_code(l)),
                limit,
                &user_agent(),
                &progress,
            )
            .await;
            end_spinner();
            if let Err(e) = res {
                return fail(l, e);
            }
            meta::validate(&mut rows, &HashMap::new());
            match std::fs::write(&out, meta::to_csv(&rows)) {
                Ok(()) => {
                    println!(
                        "{}",
                        l.t_args(
                            "seo-meta-written",
                            &args!("path" => kor_tools::display_path(&out))
                        )
                    );
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        MetaSub::Check { csv } => {
            let mut rows = match std::fs::read_to_string(&csv)
                .map_err(|e| e.to_string())
                .and_then(|t| meta::from_csv(&t))
            {
                Ok(r) => r,
                Err(e) => return fail(l, e),
            };
            meta::validate(&mut rows, &HashMap::new());
            let bad: Vec<&meta::Row> = rows.iter().filter(|r| !r.issues.is_empty()).collect();
            for r in &bad {
                println!("{}  {}", r.url, r.issues);
            }
            println!(
                "{}",
                l.t_args(
                    "seo-meta-checked",
                    &args!("n" => rows.len(), "bad" => bad.len())
                )
            );
            let _ = std::fs::write(&csv, meta::to_csv(&rows));
            true
        }
        MetaSub::Apply {
            csv,
            wp: site,
            keys,
            yes,
        } => {
            let Some(k) = wp::MetaKeys::parse(&keys) else {
                return fail(l, l.t("seo-meta-bad-keys"));
            };
            let rows = match std::fs::read_to_string(&csv)
                .map_err(|e| e.to_string())
                .and_then(|t| meta::from_csv(&t))
            {
                Ok(r) => r,
                Err(e) => return fail(l, e),
            };
            let client = match store(ctx).and_then(|s| wp::from_profile(&site, s.as_ref())) {
                Ok(c) => c,
                Err(e) => return fail(l, e),
            };
            let n = rows
                .iter()
                .filter(|r| !r.title.is_empty() || !r.alt.is_empty())
                .count();
            if !crate::shell_cmd::confirm(
                l,
                &l.t_args(
                    "seo-meta-apply-confirm",
                    &args!("n" => n, "site" => site.clone()),
                ),
                yes,
            ) {
                return false;
            }
            let dir = root.join(".kor").join("seo").join("meta");
            let _ = std::fs::create_dir_all(&dir);
            let undo = dir.join(format!(
                "undo-{}.jsonl",
                chrono::Utc::now().format("%Y%m%dT%H%M%S")
            ));
            let sp = spinner(l);
            let a = meta::apply(&rows, &client, k, &undo, &sp).await;
            end_spinner();
            for (u, why) in &a.skipped {
                println!("  {u}: {why}");
            }
            println!("{}", l.t_args("seo-meta-applied", &args!("n" => a.changed.len(), "skipped" => a.skipped.len(), "undo" => kor_tools::display_path(&undo))));
            a.skipped.is_empty()
        }
        MetaSub::Opencart {
            csv,
            prefix,
            language,
            oc_version,
            out,
        } => {
            let rows = match tokio::fs::read_to_string(&csv)
                .await
                .map_err(|e| e.to_string())
                .and_then(|t| meta::from_csv(&t))
            {
                Ok(r) => r,
                Err(e) => return fail(l, e),
            };
            let run = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
            let s = match meta::opencart_sql(&rows, &prefix, language, oc_version, &run) {
                Ok(s) => s,
                Err(e) => return fail(l, e),
            };
            let apply = out.with_extension("apply.sql");
            let undo = out.with_extension("undo.sql");
            let written = match tokio::fs::write(&apply, &s.apply).await {
                Ok(()) => tokio::fs::write(&undo, &s.undo).await,
                Err(e) => Err(e),
            };
            if let Err(e) = written {
                return fail(l, e);
            }
            for (u, why) in &s.skipped {
                println!("  {u}: {why}");
            }
            println!(
                "{}",
                l.t_args(
                    "seo-meta-opencart",
                    &args!(
                        "n" => s.rows,
                        "apply" => kor_tools::display_path(&apply),
                        "undo" => kor_tools::display_path(&undo)
                    )
                )
            );
            true
        }
        MetaSub::Undo {
            file,
            wp: site,
            yes,
        } => {
            let client = match store(ctx).and_then(|s| wp::from_profile(&site, s.as_ref())) {
                Ok(c) => c,
                Err(e) => return fail(l, e),
            };
            if !crate::shell_cmd::confirm(
                l,
                &l.t_args("seo-meta-undo-confirm", &args!("site" => site)),
                yes,
            ) {
                return false;
            }
            match meta::undo(&file, &client).await {
                Ok(n) => {
                    println!("{}", l.t_args("seo-meta-undone", &args!("n" => n)));
                    true
                }
                Err(e) => fail(l, e),
            }
        }
    }
}

async fn logs_cmd(
    ctx: &Ctx,
    files: &[PathBuf],
    sitemap: Option<&str>,
    verify_dns: bool,
    format: OutArg,
) -> bool {
    let l = &ctx.l;
    let mut sm = Vec::new();
    if let Some(s) = sitemap {
        let opts = audit::Options {
            start: s.to_owned(),
            max_pages: Some(1),
            vitals: false,
            user_agent: user_agent(),
            ..audit::Options::default()
        };
        match audit::crawl(&opts, &|_| {}).await {
            Ok(c) => sm = c.sitemap_urls,
            Err(e) => return fail(l, e),
        }
    }
    let verify = verify_dns && kor_core::net::policy() == kor_core::net::Policy::Open;
    let mut all = Report::new("seo-logs");
    for f in files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => return fail(l, format!("{}: {e}", f.display())),
        };
        let shown = kor_tools::display_path(f);
        let opts = logs::Options {
            sitemap: sm.clone(),
            verify_dns: verify,
        };
        let res = tokio::task::spawn_blocking(move || {
            let dns = logs::SystemDns;
            logs::analyze(
                &shown,
                &text,
                &opts,
                verify.then_some(&dns as &dyn logs::Dns),
            )
        })
        .await;
        let (stats, r) = match res {
            Ok(x) => x,
            Err(e) => return fail(l, e),
        };
        if format == OutArg::Markdown {
            println!("{}", l.t_args("seo-logs-head", &args!("file" => f.display().to_string(), "lines" => stats.lines, "parsed" => stats.parsed)));
            let mut t = vec![vec![
                l.t("seo-col-bot"),
                l.t("seo-col-hits"),
                l.t("seo-col-verified"),
                l.t("seo-col-fake"),
                l.t("seo-col-unverified"),
            ]];
            t.extend(stats.bots.iter().map(|(n, b)| {
                vec![
                    n.clone(),
                    b.hits.to_string(),
                    b.verified.to_string(),
                    b.fake.to_string(),
                    b.unverified.to_string(),
                ]
            }));
            crate::providers::table(&t);
            let waste: Vec<String> = stats
                .waste
                .iter()
                .map(|(k, v)| format!("{k} {v}%"))
                .collect();
            println!(
                "{}",
                l.t_args("seo-logs-waste", &args!("list" => waste.join(", ")))
            );
            if let (Some(a), Some(b)) = (stats.p50_ms, stats.p95_ms) {
                println!(
                    "{}",
                    l.t_args(
                        "seo-logs-latency",
                        &args!("p50" => format!("{a:.0}"), "p95" => format!("{b:.0}"))
                    )
                );
            }
        }
        all.findings.extend(r.findings);
        all.notes.extend(r.notes);
    }
    all.normalize();
    emit(l, &all, &l.t("seo-logs-title"), format, None, None).await
}

#[allow(clippy::too_many_lines)] // one arm per subcommand
async fn comp_cmd(ctx: &Ctx, root: &Path, c: CompSub) -> bool {
    let l = &ctx.l;
    match c {
        CompSub::Add { name, urls, ours } => {
            match competitors::add(root, &name, &urls, ours.as_deref()) {
                Ok(_) => {
                    println!("{}", l.t_args("seo-comp-added", &args!("name" => name)));
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        CompSub::Remove { name } => match competitors::remove(root, &name) {
            Ok(r) => {
                println!(
                    "{}",
                    l.t(if r {
                        "seo-comp-removed"
                    } else {
                        "seo-comp-none"
                    })
                );
                r
            }
            Err(e) => fail(l, e),
        },
        CompSub::List => match competitors::load(root) {
            Ok(cfg) => {
                for c in cfg.competitors {
                    println!(
                        "{}: {}{}",
                        c.name,
                        c.urls.join(", "),
                        if c.ours.is_empty() {
                            String::new()
                        } else {
                            format!(" ↔ {}", c.ours)
                        }
                    );
                }
                true
            }
            Err(e) => fail(l, e),
        },
        CompSub::Run {
            name,
            json,
            notify: notify_on,
        } => {
            let sp = spinner(l);
            let r = competitors::run(root, &user_agent(), name.as_deref(), &sp).await;
            end_spinner();
            match r {
                Ok(diffs) => {
                    if notify_on {
                        let text: Vec<String> = diffs
                            .iter()
                            .filter(|d| !d.is_empty() || !d.gaps.is_empty())
                            .map(|d| {
                                format!(
                                    "{}: +{} −{} ~{} · {}",
                                    d.name,
                                    d.new_urls.len(),
                                    d.removed_urls.len(),
                                    d.changes.len(),
                                    d.gaps
                                        .iter()
                                        .take(3)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                )
                            })
                            .collect();
                        if !text.is_empty() {
                            send_alert(ctx, &l.t("seo-comp-alert-title"), &text.join("\n")).await;
                        }
                    }
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&diffs).unwrap_or_default()
                        );
                        return true;
                    }
                    for d in diffs {
                        println!("## {}", d.name);
                        if d.is_empty() && d.gaps.is_empty() {
                            println!("{}", l.t("seo-comp-nochange"));
                        }
                        d.new_urls.iter().for_each(|u| println!("+ {u}"));
                        d.removed_urls.iter().for_each(|u| println!("- {u}"));
                        for c in &d.changes {
                            println!(
                                "~ {} {}: \"{}\" → \"{}\"",
                                c.url, c.field, c.before, c.after
                            );
                        }
                        if !d.gaps.is_empty() {
                            println!("{}", l.t("seo-comp-gaps"));
                            d.gaps.iter().for_each(|g| println!("  · {g}"));
                        }
                    }
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        CompSub::Schedule { every } => schedule(l, competitors::routine(root, every.as_str())),
    }
}

#[allow(clippy::struct_excessive_bools)] // CLI flags
struct AgentsRun {
    site: String,
    depth: DepthArg,
    agents: Vec<String>,
    log: Option<PathBuf>,
    verify_dns: bool,
    fix: bool,
    no_model: bool,
    notify: bool,
    json: bool,
    model: Option<String>,
}

#[allow(clippy::too_many_lines)] // run, then print each part of the team report
async fn agents_cmd(ctx: &Ctx, root: &Path, a: AgentsRun) -> bool {
    let l = &ctx.l;
    let opts = agents::TeamOptions {
        audit: audit::Options {
            start: site_url(&a.site),
            depth: a.depth.get(),
            user_agent: user_agent(),
            ..audit::Options::default()
        },
        only: a.agents,
        log: a.log,
        verify_dns: a.verify_dns,
        root: root.to_path_buf(),
    };
    let ask = if a.no_model {
        None
    } else {
        ask_model(ctx, model::Job::Writing, a.model.as_deref()).ok()
    };
    // Terminal: one braille spinner row per SEO agent, redrawn in place (§16.1, §16.2 rule 5).
    let live = if a.json {
        None
    } else {
        crate::brand::LiveMulti::start()
    };
    let rows: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let row = |agent: &str| {
        let mut r = rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        r.iter().position(|x| x == agent).unwrap_or_else(|| {
            r.push(agent.to_owned());
            r.len() - 1
        })
    };
    let on = |e: agents::Event| {
        use crate::brand::RowState;
        let Some(live) = &live else { return };
        match e {
            agents::Event::Started { agent } => {
                live.set(row(&agent), &agent, "", RowState::Running);
            }
            agents::Event::Progress { agent, detail } => {
                let short: String = detail.chars().take(80).collect();
                live.set(row(&agent), &agent, &short, RowState::Running);
            }
            agents::Event::Done { agent, findings } => {
                let text = l.t_args(
                    "seo-agent-done",
                    &args!("agent" => agent.clone(), "n" => findings),
                );
                live.set(row(&agent), &agent, &text, RowState::Done);
            }
            agents::Event::Skipped { agent, reason } => {
                live.set(row(&agent), &agent, &reason, RowState::Failed);
            }
        }
    };
    let t = match agents::run(
        &opts,
        ask.as_ref().map(|x| x as &dyn model::Ask),
        &lang_code(l),
        &on,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => return fail(l, e),
    };
    if a.notify
        && let Some(text) = agents::watch_note(&t)
    {
        send_alert(ctx, &l.t("seo-watch-title"), &text).await;
    }
    if a.fix {
        let dir = root.join(".kor").join("seo").join("fixes");
        let res = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join("fixes.md"), &t.fixes_markdown))
            .and_then(|()| std::fs::write(dir.join("redirects.htaccess"), &t.htaccess))
            .and_then(|()| std::fs::write(dir.join("gsc-urls.txt"), t.gsc_urls.join("\n")));
        match res {
            Ok(()) => eprintln!(
                "{}",
                l.t_args(
                    "seo-fix-written",
                    &args!("dir" => kor_tools::display_path(&dir))
                )
            ),
            Err(e) => return fail(l, e),
        }
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&t).unwrap_or_default());
        return true;
    }
    if let Some(s) = &t.summary {
        println!("## {}\n{s}\n", l.t("seo-summary"));
    }
    println!("## {}", l.t("seo-priorities"));
    let mut tab = vec![vec![
        l.t("seo-col-rule"),
        l.t("seo-col-agent"),
        l.t("seo-col-severity"),
        l.t("seo-col-pages"),
        l.t("seo-col-effort"),
        l.t("seo-col-score"),
    ]];
    tab.extend(t.priorities.iter().take(25).map(|p| {
        vec![
            p.rule.clone(),
            p.agent.clone(),
            p.severity.as_str().into(),
            p.pages.to_string(),
            p.effort.to_string(),
            format!("{:.1}", p.score),
        ]
    }));
    crate::providers::table(&tab);
    for (u, n) in &t.conflicts {
        println!(
            "{}",
            l.t_args(
                "seo-conflict",
                &args!("url" => u.clone(), "note" => n.clone())
            )
        );
    }
    print!("{}", render(&t.report, Format::Markdown, l));
    if let Some(d) = &t.diff {
        eprintln!(
            "{}",
            l.t_args(
                "seo-audit-diff",
                &args!("new" => d.new.len(), "fixed" => d.fixed.len())
            )
        );
    }
    true
}

/// Where `kor seo opportunities` reads its rows.
#[derive(Clone, Copy)]
enum Source<'a> {
    Api,
    Cached,
    Csv(&'a std::path::Path),
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // three sources, one table
async fn opp_cmd(
    ctx: &Ctx,
    source: Source<'_>,
    site: &str,
    days: u32,
    dims: &str,
    top: usize,
    explain: bool,
    json: bool,
) -> bool {
    let l = &ctx.l;
    let d: Vec<&str> = dims
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let (s, e) = dates(days, 0);
    let rows = if let Source::Csv(file) = source {
        // SECURITY: a bounded read; the CSV is the user's own file.
        let read = match tokio::fs::metadata(file).await {
            Ok(m) if m.len() > 20 * 1024 * 1024 => {
                Err(format!("{}: larger than 20 MB", file.display()))
            }
            Ok(_) => tokio::fs::read_to_string(file)
                .await
                .map_err(|e| format!("{}: {e}", file.display())),
            Err(e) => Err(format!("{}: {e}", file.display())),
        };
        match read.and_then(|text| opportunity::rows_from_csv(&text)) {
            Ok(r) => r,
            Err(err) => return fail(l, err),
        }
    } else if matches!(source, Source::Cached) {
        match google_db()
            .ok_or_else(|| "no data directory".to_owned())
            .and_then(|db| google::load_rows(&db, site, &s, &e, &d))
        {
            Ok(r) => r,
            Err(err) => return fail(l, err),
        }
    } else {
        let (g, _) = match connect(ctx).await {
            Ok(x) => x,
            Err(err) => return fail(l, err),
        };
        match g.search_analytics(site, &s, &e, &d, google::MAX_ROWS).await {
            Ok(r) => {
                if let Some(db) = google_db() {
                    let _ = google::store_rows(&db, site, &s, &e, &d, &r);
                }
                r
            }
            Err(err) => return fail(l, err),
        }
    };
    let scored = opportunity::score(&rows);
    // A keyword list is a one-off: it is not recorded for calibration against the site.
    let calib = kor_runtime::kor_home()
        .filter(|_| !matches!(source, Source::Csv(_)))
        .map(|h| h.join("seo").join("predictions.sqlite"))
        .and_then(|db| {
            let today = chrono::Local::now().date_naive();
            let c = opportunity::calibration(&db, site, today, &rows)
                .ok()
                .flatten();
            let _ = opportunity::record(&db, site, &today.format("%Y-%m-%d").to_string(), &scored);
            c
        });
    if let Some(c) = calib {
        println!(
            "{}",
            l.t_args(
                "seo-opp-calibration",
                &args!("rate" => format!("{:.0}", c.hit_rate * 100.0), "n" => c.checked)
            )
        );
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "formula": opportunity::FORMULA, "rows": scored.iter().take(top).collect::<Vec<_>>() })).unwrap_or_default());
        return true;
    }
    println!("{}\n{}", l.t("seo-opp-formula"), opportunity::FORMULA);
    let mut t = vec![vec![
        dims.to_owned(),
        l.t("seo-col-impressions"),
        "CTR".into(),
        l.t("seo-col-position"),
        l.t("seo-col-extra"),
        l.t("seo-col-index"),
    ]];
    t.extend(scored.iter().take(top).map(|o| {
        vec![
            o.keys.join(" · "),
            format!("{:.0}", o.impressions),
            format!("{:.1}%", o.ctr * 100.0),
            format!("{:.1}", o.position),
            format!("{:.0}", o.extra_clicks),
            format!("{:.1}", o.index),
        ]
    }));
    crate::providers::table(&t);
    if explain {
        for o in scored.iter().take(top) {
            println!("{}", l.t_args("seo-opp-explain", &args!("key" => o.keys.join(" · "), "imp" => format!("{:.0}", o.impressions), "exp" => format!("{:.3}", o.expected_ctr),
                "target" => o.target, "src" => o.ctr_source, "ctr" => format!("{:.3}", o.ctr), "band" => format!("{:.1}", o.band), "score" => format!("{:.1}", o.score))));
        }
    }
    if scored.iter().any(|o| o.ctr_source == "default") {
        println!("{}", l.t("seo-opp-default-curve"));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(flatten)]
        a: SeoArgs,
    }

    fn walk(c: &clap::Command) {
        if let Some(a) = c.get_about() {
            assert!(!a.to_string().starts_with("cmd-seo"), "{a}");
        }
        for x in c.get_arguments() {
            if let Some(h) = x.get_help() {
                assert!(!h.to_string().starts_with("arg-seo"), "{h}");
            }
        }
        c.get_subcommands().for_each(walk);
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from([
            "x",
            "audit",
            "example.com",
            "--depth",
            "full",
            "--format",
            "sarif",
        ])
        .unwrap();
        assert!(
            matches!(t.a.command, SeoSub::Audit(ref a) if a.depth == DepthArg::Full && a.format == OutArg::Sarif)
        );
        let t = T::try_parse_from([
            "x",
            "agents",
            "example.com",
            "--agents",
            "teknik,performans",
            "--fix",
        ])
        .unwrap();
        assert!(
            matches!(t.a.command, SeoSub::Agents { ref agents, fix: true, .. } if agents.len() == 2)
        );
        assert!(
            T::try_parse_from(["x", "gsc", "drops", "sc-domain:x.test", "--days", "14"]).is_ok()
        );
        assert!(T::try_parse_from(["x", "content", "publish", "slug", "--wp", "site"]).is_ok());
        assert!(
            T::try_parse_from([
                "x", "meta", "apply", "m.csv", "--wp", "s", "--keys", "rankmath"
            ])
            .is_ok()
        );
        assert!(T::try_parse_from(["x", "logs", "a.log", "--verify-dns"]).is_ok());
        assert!(T::try_parse_from(["x", "competitors", "schedule", "weekly"]).is_ok());
        assert!(T::try_parse_from(["x", "opportunities", "sc-domain:x.test", "--explain"]).is_ok());
        // §29.16: a keyword list needs no Search Console property; without either it is an error.
        assert!(T::try_parse_from(["x", "opportunities", "--keywords", "list.csv"]).is_ok());
        assert!(T::try_parse_from(["x", "opportunities"]).is_err());
        assert!(T::try_parse_from(["x", "verify", "x.test"]).is_ok());
        assert!(T::try_parse_from(["x", "bing", "query", "https://x.test/"]).is_ok());
        assert!(
            T::try_parse_from([
                "x",
                "gsc",
                "drops",
                "sc-domain:x.test",
                "--alert",
                "20",
                "--schedule",
                "daily"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "x",
                "content",
                "new",
                "kargo",
                "--from-url",
                "https://x.test/a"
            ])
            .is_ok()
        );
        for lang in [kor_i18n::Lang::En, kor_i18n::Lang::Tr, kor_i18n::Lang::Fr] {
            let l = Localizer::new(lang);
            let c = localize(T::command(), &l);
            c.clone().debug_assert();
            // Every about/help resolves (no raw ids left).
            for s in c.get_subcommands() {
                walk(s);
            }
        }
    }
}
