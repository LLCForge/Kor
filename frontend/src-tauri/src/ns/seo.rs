//! `seo.*`: `kor seo` köprüsü (§29.5–29.10, §29.15, §29.16, ADR 0055). Tarama, kontroller, Google
//! bağlantısı, içerik hattı, toplu meta, log analizi, rakip izleme, ajan takımı ve fırsat skoru
//! `kor_web::seo` içinde; burası yalnız bağımsız değişkenleri çözer.
//!
//! - `seo.audit(target, depth, maxPages?, vitals)` → `{ report, rich, diff }`; ilerleme `kor:seo/progress`
//! - `seo.exportReport(report, format "html"|"pdf"|"markdown"|"json"|"sarif", path, title, brand?)` → yol
//! - `seo.gscStatus()`, `seo.gscSetup(jsonText)`, `seo.gscLogin()`, `seo.gscLogout()`, `seo.gscSites()`,
//!   `seo.gscQuery(site, days, dims, limit)`, `seo.gscDrops(site, days, top)`, `seo.gaReport(property, days, metrics, dims)`
//! - `seo.contentList()`, `seo.contentNew(keyword, lang)`, `seo.contentRun(slug, stage, serp[], site?, model?)`,
//!   `seo.contentRead(slug)`, `seo.contentSave(slug, file, text)`, `seo.contentApprove(slug)`,
//!   `seo.contentPublish(slug, wpSite?, publish, outPath?)`
//! - `seo.metaCollect(url, allImages)` → satırlar (tarama), `seo.metaGenerate(url?, rows?, sample, all, allImages, lang, model?)` → satırlar, `seo.metaValidate(rows)`,
//!   `seo.metaEstimate(rows, model?)`, `seo.metaExport(rows, path)`, `seo.metaImport(path)`,
//!   `seo.metaApply(rows, wpSite, keys)`, `seo.metaUndo(file, wpSite)`
//! - `seo.logs(paths[], sitemap?, verifyDns)` → `{ files: [{ file, stats }], report }`
//! - `seo.competitorsList()`, `seo.competitorsAdd(name, urls[], ours?)`, `seo.competitorsRemove(name)`,
//!   `seo.competitorsRun(name?)`, `seo.competitorsSchedule(every)`
//! - `seo.agents(site, depth, agents[], log?, verifyDns, useModel, fix)` → `TeamReport`; olaylar `kor:seo/agent`
//! - `seo.opportunities(site, days, dims, top, cached)` → `{ formula, rows }`
//! - `seo.verify(target)` → `Verified` (son denetimin sayfalarını yeniden tarar)
//! - `seo.bingStatus()`, `seo.bingSetup(key)`, `seo.bingStats("query"|"pages", site)`
//! - `seo.gscDrops(site, days, top, alertPct?, explain)` uyarı eşiği aşılırsa [notify] kanallarına bildirir;
//!   `seo.gscDropsSchedule(site, alertPct, every)`
//! - `seo.auditSchedule(target, depth, every)`, `seo.agentsSchedule(site, every)` → rutin (nöbet modu)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kor_web::seo::{
    agents, audit, bing, competitors, content, google, logs, meta, model, notify, opportunity, wp,
};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

pub const NS: &str = "seo";

const METHODS: &[&str] = &[
    "audit",
    "exportReport",
    "gscStatus",
    "gscSetup",
    "gscLogin",
    "gscLogout",
    "gscSites",
    "gscQuery",
    "gscDrops",
    "gaReport",
    "contentList",
    "contentNew",
    "contentRun",
    "contentRead",
    "contentSave",
    "contentApprove",
    "contentPublish",
    "metaCollect",
    "metaGenerate",
    "metaValidate",
    "metaEstimate",
    "metaExport",
    "metaImport",
    "metaApply",
    "metaUndo",
    "logs",
    "competitorsList",
    "competitorsAdd",
    "competitorsRemove",
    "competitorsRun",
    "competitorsSchedule",
    "agents",
    "opportunities",
    "auditSchedule",
    "agentsSchedule",
    "verify",
    "bingStatus",
    "bingSetup",
    "bingStats",
    "gscDropsSchedule",
];

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn user_agent() -> String {
    format!("kor/{}", env!("CARGO_PKG_VERSION"))
}

fn num(args: &[Value], i: usize, default: u64) -> u64 {
    args.get(i).and_then(Value::as_u64).unwrap_or(default)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn depth(v: Option<&str>) -> audit::Depth {
    v.and_then(audit::Depth::parse).unwrap_or_default()
}

fn site_url(x: &str) -> String {
    if x.contains("://") {
        x.to_owned()
    } else {
        format!("https://{x}/")
    }
}

fn store(root: &Path) -> Result<Box<dyn kor_secrets::SecretStore>, String> {
    let cfg = crate::bridge::load_config(root)
        .map(|l| l.config.vault)
        .unwrap_or_default();
    kor_runtime::secure::vault_store(&cfg, None).map_err(|e| e.to_string())
}

fn ask(root: &Path, job: model::Job, m: Option<&str>) -> Result<model::ProviderAsk, String> {
    let loaded = crate::bridge::load_config(root)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    model::ask_for(&l, &loaded, job, m)
}

async fn send_alert(root: &Path, title: &str, text: &str) -> Vec<String> {
    match crate::bridge::load_config(root) {
        Ok(loaded) => {
            let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
            notify::alert(&l, &loaded, title, text).await
        }
        Err(e) => vec![e],
    }
}

fn lang(root: &Path) -> String {
    crate::bridge::load_config(root)
        .map(|l| l.config.ui.language.resolve().code().to_owned())
        .unwrap_or_else(|_| "tr".into())
}

fn google_db() -> Result<PathBuf, String> {
    kor_runtime::kor_home()
        .map(|h| h.join("seo").join("google.sqlite"))
        .ok_or_else(|| "Veri klasörü yok".into())
}

fn dates(days: u64, offset: u64) -> (String, String) {
    // Search Console verisi yaklaşık iki gün gecikir.
    let end = chrono::Local::now().date_naive() - chrono::Days::new(2 + offset);
    let start = end - chrono::Days::new(days.max(1) - 1);
    (
        start.format("%Y-%m-%d").to_string(),
        end.format("%Y-%m-%d").to_string(),
    )
}

/// Göreli yollar proje köküne göre.
fn abs(root: &Path, p: &str) -> PathBuf {
    let p = PathBuf::from(p);
    if p.is_absolute() { p } else { root.join(p) }
}

fn split(v: &str) -> Vec<&str> {
    v.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect()
}

fn add_routine(m: Result<serde_json::Map<String, Value>, String>) -> Res {
    let m = m?;
    let path = kor_runtime::routines::file().ok_or("Veri klasörü yok")?;
    kor_runtime::routines::add(&path, &m, kor_runtime::cost::now_ms())
}

fn rows_arg(v: Option<&Value>) -> Result<Vec<meta::Row>, String> {
    serde_json::from_value(v.cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("Satırlar okunamadı: {e}"))
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if !METHODS.contains(&name) {
        return None;
    }
    let root = match root(state) {
        Ok(r) => r,
        Err(e) => return Some(Err(e)),
    };
    Some(dispatch(name, args, app, &root).await)
}

#[allow(clippy::too_many_lines)] // tek tablo: yöntem → çekirdek çağrısı
async fn dispatch(name: &str, args: &[Value], app: &AppHandle, root: &Path) -> Res {
    match name {
        "audit" => {
            let target = site_url(s(args, 0, "seo.audit")?);
            let opts = audit::Options {
                start: target.clone(),
                depth: depth(opt_s(args, 1)),
                max_pages: args
                    .get(2)
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok()),
                vitals: args.get(3).and_then(Value::as_bool).unwrap_or(true),
                user_agent: user_agent(),
                ..audit::Options::default()
            };
            let a = app.clone();
            let progress = move |d: &str| {
                let _ = a.emit("kor:seo/progress", json!({ "kind": "audit", "detail": d }));
            };
            let (crawl, report) = audit::audit(&opts, &progress).await?;
            let _ = audit::save_pages(root, &crawl);
            let prev = audit::save(root, &target, &report).ok().flatten();
            let rich: Vec<Value> = crawl.pages.iter().filter(|p| !p.rich.eligible.is_empty() || !p.rich.blocked.is_empty()).map(|p| json!({ "url": p.url, "eligible": p.rich.eligible, "blocked": p.rich.blocked })).collect();
            Ok(
                json!({ "report": report, "rich": rich, "diff": prev.map(|p| audit::diff(&p, &report)), "pages": crawl.pages.len() }),
            )
        }
        "exportReport" => {
            let report: kor_runtime::analysis::Report =
                serde_json::from_value(args.first().cloned().unwrap_or(Value::Null))
                    .map_err(|e| e.to_string())?;
            let format = s(args, 1, "seo.exportReport")?;
            let path = abs(root, s(args, 2, "seo.exportReport")?);
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let title = opt_s(args, 3).unwrap_or("SEO");
            let brand = opt_s(args, 4).filter(|x| !x.is_empty());
            let l = kor_i18n::Localizer::new(
                kor_i18n::Lang::from_tag(&lang(root)).unwrap_or(kor_i18n::Lang::Tr),
            );
            let labels = [
                title.to_owned(),
                "Önem".into(),
                "URL".into(),
                "Satır".into(),
                "Kural".into(),
                "Bulgu".into(),
                "Düzeltme".into(),
                "Notlar".into(),
            ];
            let lab: [&str; 8] = std::array::from_fn(|i| labels[i].as_str());
            let bytes = match format {
                "html" => audit::to_html(&report, title, brand, lab).into_bytes(),
                "pdf" => {
                    kor_tools::html_to_pdf(&audit::to_html(&report, title, brand, lab)).await?
                }
                other => {
                    let f = kor_runtime::analysis::Format::parse(other)
                        .ok_or("Biçim: html, pdf, markdown, json veya sarif")?;
                    kor_runtime::analysis::render(&report, f, &l).into_bytes()
                }
            };
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            Ok(json!(path))
        }
        "gscStatus" => to_json(&google::status(store(root)?.as_ref())),
        "gscSetup" => {
            let c = google::parse_client_json(s(args, 0, "seo.gscSetup")?)?;
            google::save_client(store(root)?.as_ref(), &c)?;
            Ok(json!(true))
        }
        "gscLogin" => {
            let st = store(root)?;
            let a = app.clone();
            let open = move |u: &str| {
                let _ = a.emit("kor:seo/oauth", json!({ "url": u }));
                let _ = a.opener().open_url(u, None::<&str>);
            };
            google::login(
                &google::Endpoints::default(),
                st.as_ref(),
                &open,
                google::LOGIN_TIMEOUT,
            )
            .await?;
            Ok(json!(true))
        }
        "gscLogout" => google::logout(store(root)?.as_ref()).map(|x| json!(x)),
        "gscSites" => {
            let g = google::Google::connect(google::Endpoints::default(), store(root)?.as_ref())
                .await?;
            to_json(&g.sites().await?)
        }
        "gscQuery" => {
            let site = s(args, 0, "seo.gscQuery")?;
            let (start, end) = dates(num(args, 1, 28), 0);
            let dims = split(opt_s(args, 2).unwrap_or("query"));
            let limit = u32::try_from(num(args, 3, 100)).unwrap_or(100);
            let g = google::Google::connect(google::Endpoints::default(), store(root)?.as_ref())
                .await?;
            let rows = g.search_analytics(site, &start, &end, &dims, limit).await?;
            let _ = google::store_rows(&google_db()?, site, &start, &end, &dims, &rows);
            Ok(json!({ "start": start, "end": end, "rows": rows }))
        }
        "gscDrops" => {
            let site = s(args, 0, "seo.gscDrops")?;
            let days = num(args, 1, 7);
            let top = usize::try_from(num(args, 2, 20)).unwrap_or(20);
            let (s1, e1) = dates(days, days);
            let (s2, e2) = dates(days, 0);
            let g = google::Google::connect(google::Endpoints::default(), store(root)?.as_ref())
                .await?;
            let before = g
                .search_analytics(site, &s1, &e1, &["page"], google::MAX_ROWS)
                .await?;
            let now = g
                .search_analytics(site, &s2, &e2, &["page"], google::MAX_ROWS)
                .await?;
            let db = google_db()?;
            let _ = google::store_rows(&db, site, &s1, &e1, &["page"], &before);
            let _ = google::store_rows(&db, site, &s2, &e2, &["page"], &now);
            let rep = audit::latest_report(root, site.trim_start_matches("sc-domain:"));
            let drops = google::drops(&before, &now, rep.as_ref(), top);
            let mut alert = Value::Null;
            if let Some(t) = args.get(3).and_then(Value::as_f64)
                && let Some((dc, di)) = notify::anomaly(&before, &now, t)
            {
                let text =
                    format!("{site}: tıklama %{dc:.0}, gösterim %{di:.0} (önceki döneme göre)");
                alert = json!({ "text": text, "notes": send_alert(root, "Arama trafiği düştü", &text).await });
            }
            let mut hypotheses = Value::Null;
            if b(args, 4) && !drops.is_empty() {
                let a = ask(root, model::Job::Writing, None)?;
                hypotheses = json!(
                    model::Ask::ask(&a, google::drops_prompt(&drops, &lang(root)), 1_500).await?
                );
            }
            Ok(
                json!({ "before": [s1, e1], "now": [s2, e2], "drops": drops, "alert": alert, "hypotheses": hypotheses }),
            )
        }
        "gaReport" => {
            let property = s(args, 0, "seo.gaReport")?;
            let (start, end) = dates(num(args, 1, 28), 0);
            let metrics = opt_s(args, 2).unwrap_or("sessions,keyEvents").to_owned();
            let dims = opt_s(args, 3).unwrap_or("landingPage").to_owned();
            let g = google::Google::connect(google::Endpoints::default(), store(root)?.as_ref())
                .await?;
            let rows = g
                .ga_report(property, &start, &end, &split(&dims), &split(&metrics))
                .await?;
            let _ = google::store_ga(
                &google_db()?,
                property,
                &start,
                &end,
                &format!("{dims}|{metrics}"),
                &rows,
            );
            Ok(json!({ "start": start, "end": end, "rows": rows }))
        }
        "contentList" => to_json(&content::list(root)),
        "contentNew" => {
            let l = opt_s(args, 1)
                .filter(|x| !x.is_empty())
                .map_or_else(|| lang(root), str::to_owned);
            let kw = s(args, 0, "seo.contentNew")?;
            match opt_s(args, 2).filter(|x| !x.is_empty()) {
                Some(u) => {
                    to_json(&content::create_from_url(root, u, kw, &l, &user_agent()).await?)
                }
                None => to_json(&content::create(root, kw, &l)?),
            }
        }
        "contentRun" => {
            let slug = s(args, 0, "seo.contentRun")?;
            let stage = opt_s(args, 1).unwrap_or("all");
            let serp = strings(args.get(2));
            let site = opt_s(args, 3).filter(|x| !x.is_empty());
            let m = opt_s(args, 4).filter(|x| !x.is_empty());
            let stages: Vec<content::Stage> = if stage == "all" {
                vec![
                    content::Stage::Brief,
                    content::Stage::Outline,
                    content::Stage::Draft,
                    content::Stage::Checked,
                ]
            } else {
                vec![
                    content::Stage::parse(stage)
                        .ok_or("Aşama: brief, outline, draft, check veya all")?,
                ]
            };
            let a = if stages.iter().any(|x| *x != content::Stage::Checked) {
                Some(ask(root, model::Job::Writing, m)?)
            } else {
                None
            };
            let pages = if stages.contains(&content::Stage::Brief) {
                content::serp_pages(&serp, &user_agent()).await
            } else {
                Vec::new()
            };
            let links = content::site_pages(root, site);
            let mut check = None;
            for st in stages {
                let _ = app.emit(
                    "kor:seo/progress",
                    json!({ "kind": "content", "detail": format!("{st:?}").to_lowercase() }),
                );
                if st == content::Stage::Checked {
                    check = Some(content::check(root, slug)?.1);
                } else {
                    content::run_stage(
                        root,
                        slug,
                        st,
                        a.as_ref().ok_or("model yok")?,
                        &pages,
                        &links,
                    )
                    .await?;
                }
            }
            Ok(json!({ "state": content::load(root, slug)?, "check": check }))
        }
        "contentRead" => {
            let slug = s(args, 0, "seo.contentRead")?;
            let st = content::load(root, slug)?;
            let read = |f: &str| {
                std::fs::read_to_string(content::dir(root, slug).join(f)).unwrap_or_default()
            };
            Ok(
                json!({ "state": st, "brief": read("brief.md"), "outline": read("outline.md"), "draft": read("draft.md"), "check": read("check.md"), "dir": content::dir(root, slug) }),
            )
        }
        "contentSave" => {
            let slug = s(args, 0, "seo.contentSave")?;
            let file = s(args, 1, "seo.contentSave")?;
            if !matches!(file, "brief.md" | "outline.md" | "draft.md") {
                return Err("Yalnız brief.md, outline.md ve draft.md düzenlenebilir".into());
            }
            content::load(root, slug)?;
            std::fs::write(
                content::dir(root, slug).join(file),
                s(args, 2, "seo.contentSave")?,
            )
            .map_err(|e| e.to_string())?;
            Ok(json!(true))
        }
        "contentApprove" => to_json(&content::approve(root, s(args, 0, "seo.contentApprove")?)?),
        "contentPublish" => {
            let slug = s(args, 0, "seo.contentPublish")?;
            let site = opt_s(args, 1).filter(|x| !x.is_empty());
            let publish = b(args, 2);
            let out = opt_s(args, 3)
                .filter(|x| !x.is_empty())
                .map(|p| abs(root, p));
            let st = match (site, out) {
                (Some(w), _) => {
                    let client = wp::from_profile(w, store(root)?.as_ref())?;
                    content::publish(
                        root,
                        slug,
                        content::Target::WordPress {
                            wp: &client,
                            publish,
                        },
                    )
                    .await?
                }
                (None, Some(p)) => {
                    content::publish(root, slug, content::Target::Export(&p)).await?
                }
                (None, None) => return Err("WordPress sitesi veya dosya yolu seçin".into()),
            };
            to_json(&st)
        }
        "metaCollect" => {
            let opts = audit::Options {
                start: site_url(s(args, 0, "seo.metaCollect")?),
                depth: audit::Depth::Full,
                vitals: false,
                user_agent: user_agent(),
                ..audit::Options::default()
            };
            let a = app.clone();
            let c = audit::crawl(&opts, &move |d: &str| {
                let _ = a.emit("kor:seo/progress", json!({ "kind": "meta", "detail": d }));
            })
            .await?;
            let mut rows = meta::collect(&c, b(args, 1));
            meta::validate(&mut rows, &HashMap::new());
            to_json(&rows)
        }
        "metaGenerate" => {
            let url = opt_s(args, 0).filter(|x| !x.is_empty());
            let mut rows = if let Some(u) = url {
                let opts = audit::Options {
                    start: site_url(u),
                    depth: audit::Depth::Full,
                    vitals: false,
                    user_agent: user_agent(),
                    ..audit::Options::default()
                };
                let a = app.clone();
                let c = audit::crawl(&opts, &move |d: &str| {
                    let _ = a.emit("kor:seo/progress", json!({ "kind": "meta", "detail": d }));
                })
                .await?;
                meta::collect(&c, b(args, 4))
            } else {
                rows_arg(args.get(1))?
            };
            let sample = usize::try_from(num(args, 2, 20)).unwrap_or(20);
            let limit = if b(args, 3) { usize::MAX } else { sample };
            let l = opt_s(args, 5).map_or_else(|| lang(root), str::to_owned);
            let a = ask(
                root,
                model::Job::Bulk,
                opt_s(args, 6).filter(|x| !x.is_empty()),
            )?;
            let app2 = app.clone();
            meta::generate(&mut rows, &a, &l, limit, &user_agent(), &move |d, t| {
                let _ = app2.emit(
                    "kor:seo/progress",
                    json!({ "kind": "meta", "done": d, "total": t }),
                );
            })
            .await?;
            meta::validate(&mut rows, &HashMap::new());
            to_json(&rows)
        }
        "metaValidate" => {
            let mut rows = rows_arg(args.first())?;
            meta::validate(&mut rows, &HashMap::new());
            to_json(&rows)
        }
        "metaEstimate" => {
            let rows = rows_arg(args.first())?;
            let a = ask(
                root,
                model::Job::Bulk,
                opt_s(args, 1).filter(|x| !x.is_empty()),
            )?;
            let (i, o) = meta::estimate_tokens(&rows, model::Ask::vision(&a));
            let loaded = crate::bridge::load_config(root)?;
            let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
            let prices = kor_runtime::bench::Prices::load(
                &l,
                kor_platform::cache_dir()
                    .map(|d| d.join("model-catalog.json"))
                    .as_deref(),
            );
            let usd = prices
                .cost(&model::Ask::id(&a), i, o)
                .map(kor_core::Cost::usd);
            Ok(
                json!({ "model": model::Ask::id(&a), "inputTokens": i, "outputTokens": o, "usd": usd, "estimated": true }),
            )
        }
        "metaExport" => {
            let rows = rows_arg(args.first())?;
            let p = abs(root, s(args, 1, "seo.metaExport")?);
            std::fs::write(&p, meta::to_csv(&rows)).map_err(|e| e.to_string())?;
            Ok(json!(p))
        }
        "metaImport" => {
            let text = std::fs::read_to_string(abs(root, s(args, 0, "seo.metaImport")?))
                .map_err(|e| e.to_string())?;
            to_json(&meta::from_csv(&text)?)
        }
        "metaApply" => {
            let rows = rows_arg(args.first())?;
            let site = s(args, 1, "seo.metaApply")?;
            let keys = wp::MetaKeys::parse(opt_s(args, 2).unwrap_or("yoast"))
                .ok_or("Alanlar: yoast, rankmath veya none")?;
            let client = wp::from_profile(site, store(root)?.as_ref())?;
            let dir = root.join(".kor").join("seo").join("meta");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let undo = dir.join(format!(
                "undo-{}.jsonl",
                chrono::Utc::now().format("%Y%m%dT%H%M%S")
            ));
            let a = app.clone();
            let applied = meta::apply(&rows, &client, keys, &undo, &move |u: &str| {
                let _ = a.emit(
                    "kor:seo/progress",
                    json!({ "kind": "metaApply", "detail": u }),
                );
            })
            .await;
            Ok(json!({ "applied": applied, "undo": undo }))
        }
        "metaUndo" => {
            let client = wp::from_profile(s(args, 1, "seo.metaUndo")?, store(root)?.as_ref())?;
            meta::undo(&abs(root, s(args, 0, "seo.metaUndo")?), &client)
                .await
                .map(|n| json!(n))
        }
        "logs" => {
            let paths = strings(args.first());
            if paths.is_empty() || paths.len() > 20 {
                return Err("1-20 log dosyası seçin".into());
            }
            let mut sitemap = Vec::new();
            if let Some(sm) = opt_s(args, 1).filter(|x| !x.is_empty()) {
                let opts = audit::Options {
                    start: sm.to_owned(),
                    max_pages: Some(1),
                    vitals: false,
                    user_agent: user_agent(),
                    ..audit::Options::default()
                };
                sitemap = audit::crawl(&opts, &|_| {}).await?.sitemap_urls;
            }
            let verify = b(args, 2) && kor_core::net::policy() == kor_core::net::Policy::Open;
            tauri::async_runtime::spawn_blocking(move || {
                let mut files = Vec::new();
                let mut all = kor_runtime::analysis::Report::new("seo-logs");
                for p in paths {
                    let text = std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?;
                    let dns = logs::SystemDns;
                    let (stats, r) = logs::analyze(
                        &p,
                        &text,
                        &logs::Options {
                            sitemap: sitemap.clone(),
                            verify_dns: verify,
                        },
                        verify.then_some(&dns as &dyn logs::Dns),
                    );
                    files.push(json!({ "file": p, "stats": stats }));
                    all.findings.extend(r.findings);
                    all.notes.extend(r.notes);
                }
                all.normalize();
                Ok(json!({ "files": files, "report": all }))
            })
            .await
            .map_err(|e| e.to_string())?
        }
        "competitorsList" => to_json(&competitors::load(root)?),
        "competitorsAdd" => to_json(&competitors::add(
            root,
            s(args, 0, "seo.competitorsAdd")?,
            &strings(args.get(1)),
            opt_s(args, 2).filter(|x| !x.is_empty()),
        )?),
        "competitorsRemove" => {
            competitors::remove(root, s(args, 0, "seo.competitorsRemove")?).map(|x| json!(x))
        }
        "competitorsRun" => {
            let a = app.clone();
            let d = competitors::run(
                root,
                &user_agent(),
                opt_s(args, 0).filter(|x| !x.is_empty()),
                &move |n: &str| {
                    let _ = a.emit(
                        "kor:seo/progress",
                        json!({ "kind": "competitors", "detail": n }),
                    );
                },
            )
            .await?;
            to_json(&d)
        }
        "auditSchedule" => add_routine(audit::routine(
            root,
            &site_url(s(args, 0, "seo.auditSchedule")?),
            depth(opt_s(args, 1)),
            s(args, 2, "seo.auditSchedule")?,
        )),
        "agentsSchedule" => add_routine(agents::routine(
            root,
            s(args, 0, "seo.agentsSchedule")?,
            s(args, 1, "seo.agentsSchedule")?,
        )),
        "verify" => {
            let start = site_url(s(args, 0, "seo.verify")?);
            let prev = audit::latest_report(root, &start)
                .ok_or("Bu projede sitenin önceki denetimi yok; önce denetleyin.")?;
            let a = app.clone();
            let opts = audit::Options {
                start,
                user_agent: user_agent(),
                ..audit::Options::default()
            };
            to_json(
                &audit::verify(&prev, &opts, &move |d: &str| {
                    let _ = a.emit("kor:seo/progress", json!({ "kind": "verify", "detail": d }));
                })
                .await?,
            )
        }
        "bingStatus" => Ok(json!(bing::has_key(store(root)?.as_ref()))),
        "bingSetup" => bing::save_key(store(root)?.as_ref(), s(args, 0, "seo.bingSetup")?)
            .map(|()| json!(true)),
        "bingStats" => {
            let kind = match s(args, 0, "seo.bingStats")? {
                "query" => "GetQueryStats",
                "pages" => "GetPageStats",
                _ => return Err("Tür: query veya pages".into()),
            };
            to_json(
                &bing::stats(
                    bing::BASE,
                    store(root)?.as_ref(),
                    kind,
                    s(args, 1, "seo.bingStats")?,
                )
                .await?,
            )
        }
        "gscDropsSchedule" => add_routine(google::drops_routine(
            root,
            s(args, 0, "seo.gscDropsSchedule")?,
            args.get(1).and_then(Value::as_f64).unwrap_or(20.0),
            s(args, 2, "seo.gscDropsSchedule")?,
        )),
        "competitorsSchedule" => add_routine(competitors::routine(
            root,
            s(args, 0, "seo.competitorsSchedule")?,
        )),
        "agents" => {
            let site = site_url(s(args, 0, "seo.agents")?);
            let opts = agents::TeamOptions {
                audit: audit::Options {
                    start: site,
                    depth: depth(opt_s(args, 1)),
                    user_agent: user_agent(),
                    ..audit::Options::default()
                },
                only: strings(args.get(2)),
                log: opt_s(args, 3).filter(|x| !x.is_empty()).map(PathBuf::from),
                verify_dns: b(args, 4),
                root: root.to_path_buf(),
            };
            let a = if b(args, 5) {
                ask(root, model::Job::Writing, None).ok()
            } else {
                None
            };
            let app2 = app.clone();
            let on = move |e: agents::Event| {
                let _ = app2.emit("kor:seo/agent", &e);
            };
            let t = agents::run(
                &opts,
                a.as_ref().map(|x| x as &dyn model::Ask),
                &lang(root),
                &on,
            )
            .await?;
            if b(args, 7)
                && let Some(text) = agents::watch_note(&t)
            {
                let _ = send_alert(root, "SEO nöbeti: yeni bulgular", &text).await;
            }
            if b(args, 6) {
                let dir = root.join(".kor").join("seo").join("fixes");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                std::fs::write(dir.join("fixes.md"), &t.fixes_markdown)
                    .map_err(|e| e.to_string())?;
                std::fs::write(dir.join("redirects.htaccess"), &t.htaccess)
                    .map_err(|e| e.to_string())?;
                std::fs::write(dir.join("gsc-urls.txt"), t.gsc_urls.join("\n"))
                    .map_err(|e| e.to_string())?;
            }
            to_json(&t)
        }
        "opportunities" => {
            let site = s(args, 0, "seo.opportunities")?;
            let (start, end) = dates(num(args, 1, 28), 0);
            let dims = split(opt_s(args, 2).unwrap_or("query"));
            let top = usize::try_from(num(args, 3, 30)).unwrap_or(30);
            let rows = if b(args, 4) {
                google::load_rows(&google_db()?, site, &start, &end, &dims)?
            } else {
                let g =
                    google::Google::connect(google::Endpoints::default(), store(root)?.as_ref())
                        .await?;
                let r = g
                    .search_analytics(site, &start, &end, &dims, google::MAX_ROWS)
                    .await?;
                let _ = google::store_rows(&google_db()?, site, &start, &end, &dims, &r);
                r
            };
            let scored = opportunity::score(&rows);
            let db = kor_runtime::kor_home()
                .map(|h| h.join("seo").join("predictions.sqlite"))
                .ok_or("Veri klasörü yok")?;
            let today = chrono::Local::now().date_naive();
            let calibration = opportunity::calibration(&db, site, today, &rows)?;
            opportunity::record(&db, site, &today.format("%Y-%m-%d").to_string(), &scored)?;
            Ok(
                json!({ "formula": opportunity::FORMULA, "start": start, "end": end, "calibration": calibration, "rows": scored.into_iter().take(top).collect::<Vec<_>>() }),
            )
        }
        _ => Err(format!("`seo.{name}` henüz Kor arka ucunda yok")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_and_helpers() {
        assert_eq!(NS, "seo");
        assert_eq!(site_url("ornek.com"), "https://ornek.com/");
        assert_eq!(site_url("http://127.0.0.1:8080/"), "http://127.0.0.1:8080/");
        assert_eq!(depth(Some("forensic")), audit::Depth::Forensic);
        assert_eq!(depth(None), audit::Depth::Quick);
        assert_eq!(split("a, b,,c"), ["a", "b", "c"]);
        let (a, b) = dates(7, 0);
        assert!(a < b);
        assert_eq!(strings(Some(&json!(["x", 1, "y"]))), ["x", "y"]);
        assert!(rows_arg(Some(&json!([{"kind": "page", "url": "u", "page": "", "current_title": "", "current_description": "", "current_alt": "", "title": "", "description": "", "alt": "", "context": "", "issues": ""}]))).is_ok());
        let mut sorted = METHODS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), METHODS.len());
    }
}
