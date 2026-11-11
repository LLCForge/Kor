//! `web.*`: Web siteleri paneli (§29, ADR 0054) için ince köprü. Mantık `kor_web::site` içinde; burası
//! bağımsız değişkenleri çözer ve JSON döndürür. Her yöntem tek bir nesne alır (`args[0]`).
//! Sözleşme: `frontend/src/desktop/ipc-web.ts`.
//!
//! - Sunucular (§29.1): `hosts`, `hostConnect`, `hostRemove`, `inventory`, `hostPlan`, `hostTask`
//!   (`approve: true` olmadan değişiklik yapılmaz; önce `hostPlan` gösterilir).
//! - WordPress (§29.3–29.4): `sites`, `wpConnect`, `wpRemove`, `wpInventory`, `wpHealth`, `wpDoctor`,
//!   `wpUpdate` (`live: true` olmadan canlıya dokunmaz), `wpStaging`, `wpIntegrity`, `wpCleanup`
//!   (`confirm: true`), `wpScaffold`, `detect`.
//! - Yönlendirmeler (§29.11): `redirectsBuild`, `redirectsExport`, `redirectsCheck`, `redirectsSlugs`.
//! - Teklif (§29.12): `businessInit`, `proposal`, `proposalRevise`, `templates`.
//! - Müşteriler (§29.13): `clients`, `clientAdd`, `clientLog`, `ticket`, `clientCheck`, `clientReport`.
//! - Türkiye (§29.14): `trCheck`, `trSlug`, `trKdv`, `trKvkk`, `trEfatura`, `trIrsaliye`, `trPayment`,
//!   `trMarket`, `trKeywords`.
//!
//! Dosya çıktıları açık proje klasörüne yazılır (müşteri raporları Kor kullanıcı klasörüne).

#![allow(clippy::doc_markdown, clippy::many_single_char_names)] // ürün adları; kısa `o`, `l`, `n` adları diğer ns dosyalarındaki gibi

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kor_i18n::Localizer;
use kor_secrets::ExposeSecret as _;
use kor_web::site::{self as web, client, cms, docs, doctor, host, panels, proposal, redirects, tr, wp};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root};
use crate::state::AppState;

pub const NS: &str = "web";

fn obj(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::String(s)) => json!({ "name": s }),
        Some(v) => v.clone(),
        None => json!({}),
    }
}

fn st<'a>(o: &'a Value, k: &str) -> Option<&'a str> {
    o[k].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn need<'a>(o: &'a Value, k: &str) -> Result<&'a str, String> {
    st(o, k).ok_or_else(|| format!("Eksik alan: {k}"))
}

fn strings(o: &Value, k: &str) -> Vec<String> {
    match &o[k] {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_owned())).filter(|s| !s.is_empty()).collect(),
        Value::String(s) => s.split([',', '\n']).map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect(),
        _ => Vec::new(),
    }
}

fn values(o: &Value) -> BTreeMap<String, String> {
    o["values"].as_object().into_iter().flatten().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned()))).collect()
}

fn localizer(state: &AppState) -> Localizer {
    let lang = root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .map_or(kor_i18n::Lang::Tr, |c| c.config.ui.language.resolve());
    Localizer::new(lang)
}

fn files(v: &[PathBuf]) -> Value {
    json!(v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>())
}

fn to_json<T: serde::Serialize>(v: &T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

pub async fn call(_ns: &str, name: &str, args: &[Value], _app: &AppHandle, state: &AppState) -> Option<Res> {
    let o = obj(args);
    let l = localizer(state);
    let project = root(state).ok();
    Some(match name {
        "hosts" | "hostConnect" | "hostRemove" | "inventory" | "hostPlan" | "hostTask" => hosts(name, &o, &l).await,
        "sites" | "wpConnect" | "wpRemove" | "wpInventory" | "wpHealth" | "wpDoctor" | "wpUpdate" | "wpStaging" | "wpIntegrity"
        | "wpCleanup" | "wpScaffold" | "detect" => wordpress(name, &o, &l, project.as_deref()).await,
        "redirectsBuild" | "redirectsExport" | "redirectsCheck" | "redirectsSlugs" => redirects_ns(name, &o, project.as_deref()).await,
        "businessInit" | "proposal" | "proposalRevise" | "templates" => proposals(name, &o, project.as_deref()),
        "clients" | "clientAdd" | "clientLog" | "ticket" | "clientCheck" | "clientReport" => clients(name, &o, &l).await,
        "trCheck" | "trSlug" | "trKdv" | "trKvkk" | "trEfatura" | "trIrsaliye" | "trPayment" | "trMarket" | "trKeywords" => {
            turkey(name, &o, &l, project.as_deref()).await
        }
        _ => return None,
    })
}

async fn hosts(name: &str, o: &Value, l: &Localizer) -> Res {
    let base = web::base()?;
    let store = web::vault().ok();
    match name {
        "hosts" => Ok(json!(host::list(&base)
            .into_iter()
            .map(|(n, p)| match p {
                Ok(p) => json!({ "name": n, "kind": p.kind().as_str(), "url": p.url, "ssh": p.ssh, "client": p.client, "account": p.account }),
                Err(e) => json!({ "name": n, "error": e }),
            })
            .collect::<Vec<_>>())),
        "hostConnect" => {
            let n = need(o, "name")?;
            let kind = host::Kind::parse(need(o, "kind")?).ok_or("Bilinmeyen panel türü")?;
            let s = |k: &str| st(o, k).map(str::to_owned);
            let mut p = host::HostProfile { kind: Some(kind), url: s("url"), user: s("user"), secret: s("secret"), ssh: s("ssh"), account: s("account"), client: s("client"), email: s("email") };
            p.validate()?;
            let secret = match &p.secret {
                Some(x) => Some(web::secret(store.as_deref(), x)?.expose_secret().to_owned()),
                None => None,
            };
            let panel = host::open(n, &p, secret.as_deref(), host::ssh_shell(&p)?, None)?;
            let inv = host::inventory(panel.as_ref()).await?;
            host::save(&base, n, &p)?;
            Ok(json!({ "name": n, "markdown": host::inventory_markdown(l, &inv, web::now()), "inventory": inv }))
        }
        "hostRemove" => Ok(json!(host::remove(&base, need(o, "name")?)?)),
        "inventory" => {
            let names: Vec<String> = if o["all"] == Value::Bool(true) {
                host::list(&base).into_iter().map(|(n, _)| n).collect()
            } else {
                vec![need(o, "name")?.to_owned()]
            };
            let mut out = Vec::new();
            for n in names {
                let r = match host::connect(&base, &n, store.as_deref(), st(o, "account").map(str::to_owned)) {
                    Ok((_, p)) => host::inventory(p.as_ref()).await,
                    Err(e) => Err(e),
                };
                out.push(match r {
                    Ok(inv) => json!({ "name": n, "markdown": host::inventory_markdown(l, &inv, web::now()), "inventory": inv }),
                    Err(e) => json!({ "name": n, "error": e }),
                });
            }
            Ok(json!(out))
        }
        _ => {
            let n = need(o, "name")?;
            let task = panels::task_from(need(o, "task")?, &values(o))?;
            task.validate()?;
            let (prof, panel) = host::connect(&base, n, store.as_deref(), st(o, "account").map(str::to_owned))?;
            let warnings = web::tools::php_warnings(l, &base, n, &task).await;
            if name == "hostPlan" {
                let inv = host::inventory(panel.as_ref()).await?;
                return Ok(json!({ "plan": host::plan(l, &task, &inv, &warnings), "reversible": task.reversible() }));
            }
            if o["approve"] != Value::Bool(true) {
                return Err("Değişiklik onaylanmadı".into());
            }
            let kind = format!("host-{}", task.id());
            let rep = host::run(l, n, &prof, panel.as_ref(), task, &warnings, store.as_deref(), &|_| true).await;
            if rep.applied.is_some() {
                client::log_auto(&base, prof.client.as_deref(), &kind, n, rep.applied.as_deref().unwrap_or(""), rep.secs);
            }
            to_json(&rep)
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn wordpress(name: &str, o: &Value, l: &Localizer, project: Option<&Path>) -> Res {
    let base = web::base()?;
    let open = |n: &str| web::tools::open_wp(&base, n);
    match name {
        "sites" => Ok(json!(wp::list(&base)
            .into_iter()
            .map(|(n, s)| match s {
                Ok(s) => json!({ "name": n, "url": s.url, "rest": s.secret.is_some(), "cli": s.ssh.is_some(), "host": s.host, "client": s.client }),
                Err(e) => json!({ "name": n, "error": e }),
            })
            .collect::<Vec<_>>())),
        "wpConnect" => {
            let n = need(o, "name")?;
            let s = |k: &str| st(o, k).map(str::to_owned);
            let mut site = wp::WpSite { url: need(o, "url")?.to_owned(), user: s("user"), secret: s("secret"), ssh: s("ssh"), path: s("path"), host: s("host"), client: s("client") };
            site.validate()?;
            wp::save(&base, n, &site)?;
            match open(n)?.inventory().await {
                Ok(inv) => to_json(&inv),
                Err(e) => {
                    let _ = wp::remove(&base, n);
                    Err(e)
                }
            }
        }
        "wpRemove" => Ok(json!(wp::remove(&base, need(o, "name")?)?)),
        "wpInventory" => to_json(&open(need(o, "name")?)?.inventory().await?),
        "wpHealth" => to_json(&wp::health(&open(need(o, "name")?)?).await),
        "wpDoctor" => {
            let n = need(o, "name")?;
            let w = open(n)?;
            let inv = w.inventory().await?;
            let token = web::secret(web::vault().ok().as_deref(), "WPSCAN_API_TOKEN").ok().map(|t| t.expose_secret().to_owned());
            let mut rep = doctor::diagnose(w.http(), n, &inv.plugins, inv.version.as_deref(), token.as_deref().map(|t| ("https://wpscan.com", t)), None).await;
            if o["measure"] == Value::Bool(true) {
                let root = w.site.path.clone().unwrap_or_default();
                doctor::measure(&w, &format!("{}/kor-staging", root.trim_end_matches('/')), &format!("{}/kor-staging", w.site.url), &mut rep, 5).await?;
            }
            let php = st(o, "php").map(|v| {
                let infos: Vec<_> = rep.rows.iter().map(|r| (r.plugin.clone(), r.org.clone())).collect();
                doctor::php_warnings(l, &infos, v)
            });
            client::log_auto(&base, w.site.client.as_deref(), "doctor", n, &format!("{} plugins", rep.rows.len()), 0);
            Ok(json!({ "markdown": doctor::render(l, &rep), "report": rep, "phpWarnings": php.unwrap_or_default() }))
        }
        "wpUpdate" => {
            let n = need(o, "name")?;
            let w = open(n)?;
            let inv = w.inventory().await?;
            let only = strings(o, "only");
            let root = w.site.path.clone().ok_or("Bu sitenin WP-CLI yolu yok (SSH + yol gerekir)")?;
            let mut out = json!({ "pending": wp::pending(&inv) });
            if o["staging"] == Value::Bool(true) {
                let url = wp::create_staging(&w).await?;
                out["staging"] = to_json(&wp::update(&w, &format!("{}/kor-staging", root.trim_end_matches('/')), &url, &inv, &only).await?)?;
            }
            if o["live"] == Value::Bool(true) {
                let start = std::time::Instant::now();
                let rep = wp::update(&w, &root, &w.site.url, &inv, &only).await?;
                let done = rep.items.iter().filter(|i| i.result == "updated").count();
                client::log_auto(&base, w.site.client.as_deref(), "wp-update", n, &format!("{done} updated"), start.elapsed().as_secs());
                out["live"] = to_json(&rep)?;
            }
            Ok(out)
        }
        "wpStaging" => {
            let w = open(need(o, "name")?)?;
            match need(o, "action")? {
                "create" => Ok(json!({ "url": wp::create_staging(&w).await? })),
                "drop" => wp::drop_staging(&w).await.map(|()| json!(true)),
                _ => Err("create | drop".into()),
            }
        }
        "wpIntegrity" => {
            let n = need(o, "name")?;
            let w = open(n)?;
            let i = wp::integrity(&w).await?;
            client::log_auto(&base, w.site.client.as_deref(), "wp-integrity", n, &format!("{} modified, {} unknown", i.modified.len(), i.unknown.len()), 0);
            to_json(&i)
        }
        "wpCleanup" => {
            if o["confirm"] != Value::Bool(true) {
                return Err("Temizlik onaylanmadı".into());
            }
            let n = need(o, "name")?;
            let w = open(n)?;
            let found = wp::integrity(&w).await?;
            let done = wp::cleanup(&w, &found).await?;
            client::log_auto(&base, w.site.client.as_deref(), "wp-cleanup", n, &done.join("; "), 0);
            Ok(json!(done))
        }
        "wpScaffold" => {
            let slug = need(o, "slug")?;
            if !web::valid_name(slug) {
                return Err(format!("Geçersiz klasör adı: {slug}"));
            }
            let title = need(o, "title")?;
            let list = match need(o, "kind")? {
                "child-theme" => wp::child_theme(st(o, "parent").unwrap_or("twentytwentyfive"), title, slug),
                "plugin" => wp::plugin_skeleton(title, slug, o["woocommerce"] == Value::Bool(true)),
                "opencart" => cms::opencart_module(slug, title),
                "prestashop" => cms::prestashop_module(slug, title),
                _ => return Err("child-theme | plugin | opencart | prestashop".into()),
            };
            let dir = project.ok_or("Önce bir proje klasörü açın")?;
            Ok(files(&wp::write_scaffold(dir, &list)?))
        }
        _ => {
            let target = need(o, "target")?;
            if target.starts_with("http") {
                let (_, r) = web::http::Http::new()?.follow(target).await?;
                to_json(&cms::detect_html(&r.body))
            } else {
                to_json(&cms::detect_dir(Path::new(target)))
            }
        }
    }
}

fn rows_of(o: &Value) -> Result<Vec<redirects::Row>, String> {
    if let Some(p) = st(o, "map") {
        return redirects::from_csv(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?);
    }
    serde_json::from_value(o["rows"].clone()).map_err(|e| e.to_string())
}

async fn redirects_ns(name: &str, o: &Value, project: Option<&Path>) -> Res {
    let http = web::http::Http::new()?;
    let limit = o["limit"].as_u64().and_then(|x| usize::try_from(x).ok()).unwrap_or(2000);
    match name {
        "redirectsBuild" => {
            let (old, mut titles) = redirects::collect(&http, need(o, "old")?, limit).await?;
            let (new, nt) = redirects::collect(&http, need(o, "new")?, limit).await?;
            titles.extend(nt);
            let rows = redirects::build(&old, &new, &titles, o["min"].as_f64().unwrap_or(0.34));
            let saved = match project {
                Some(d) => {
                    let p = d.join("redirects.csv");
                    std::fs::write(&p, redirects::to_csv(&rows)).map_err(|e| e.to_string())?;
                    Some(p.display().to_string())
                }
                None => None,
            };
            Ok(json!({ "old": old.len(), "new": new.len(), "rows": rows, "issues": redirects::issues(&rows), "saved": saved }))
        }
        "redirectsExport" => {
            let fmt = redirects::Format::parse(need(o, "format")?).ok_or("apache | nginx | cloudflare | redirection | csv")?;
            let rows = rows_of(o)?;
            let rows = if o["collapse"] == Value::Bool(true) { redirects::collapse(&rows) } else { rows };
            Ok(json!({ "text": redirects::render(&rows, fmt, st(o, "host").unwrap_or("")), "issues": redirects::issues(&rows) }))
        }
        "redirectsCheck" => to_json(&redirects::verify(&http, need(o, "base")?, &rows_of(o)?).await),
        _ => {
            let (paths, _) = redirects::collect(&http, need(o, "source")?, 5000).await?;
            let rows: Vec<redirects::Row> =
                tr::slug_redirects(&paths).into_iter().map(|(old, new)| redirects::Row { old, new, score: 1.0, method: "slug".into() }).collect();
            to_json(&rows)
        }
    }
}

fn proposals(name: &str, o: &Value, project: Option<&Path>) -> Res {
    let base = web::base()?;
    let biz = proposal::business(&base)?;
    let formats = {
        let f = strings(o, "format");
        if f.is_empty() { vec!["html".into(), "pdf".into()] } else { f }
    };
    match name {
        "businessInit" => {
            let p = proposal::business_path(&base);
            if !p.exists() {
                proposal::init_business(&base)?;
            }
            Ok(json!({ "path": p.display().to_string(), "business": biz }))
        }
        "templates" => {
            for k in proposal::KINDS {
                for lang in [proposal::Lang::Tr, proposal::Lang::En] {
                    proposal::template(&base, k, lang)?;
                }
            }
            Ok(json!({ "dir": proposal::templates_dir(&base).display().to_string() }))
        }
        "proposal" => {
            let client_name = need(o, "client")?;
            let lang = proposal::Lang::parse(st(o, "lang").unwrap_or("tr")).ok_or("tr | en")?;
            let mut items = st(o, "text").map(proposal::scope_from_text).unwrap_or_default();
            for i in strings(o, "items") {
                items.push(proposal::parse_item(&i)?);
            }
            for p in strings(o, "packages") {
                let x = biz.packages.iter().find(|x| x.name == p).ok_or_else(|| format!("Paket yok: {p}"))?;
                items.push(proposal::Item { title: x.name.clone(), hours: x.hours, estimated: false, price: Some(x.price) });
            }
            if items.is_empty() {
                return Err("Kapsam boş: iş tanımı ya da kalem girin".into());
            }
            let p = proposal::Proposal {
                client: client_name.to_owned(),
                project: need(o, "project")?.to_owned(),
                lang,
                items,
                version: 1,
                date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                valid_days: o["validDays"].as_u64().and_then(|x| u32::try_from(x).ok()).unwrap_or(15),
                notes: Vec::new(),
            };
            let dir = project.ok_or("Önce bir proje klasörü açın")?.join(format!("teklif-{}", tr::slug(client_name)));
            let out = proposal::write(&base, &dir, &p, &biz, o["contracts"] == Value::Bool(true), &formats)?;
            let t = proposal::totals(&p, &biz);
            Ok(json!({
                "dir": dir.display().to_string(), "files": files(&out), "items": p.items,
                "hours": t.hours, "net": tr::try_format(t.net), "vat": tr::try_format(t.vat), "gross": tr::try_format(t.gross),
                "estimated": t.estimated, "rateMissing": biz.hourly_rate <= 0.0,
            }))
        }
        _ => {
            let dir = PathBuf::from(need(o, "dir")?);
            let mut p = proposal::load_state(&dir)?;
            let changes = proposal::revise(&mut p, need(o, "reply")?);
            if changes.is_empty() {
                return Ok(json!({ "changes": [], "files": [] }));
            }
            let out = proposal::write(&base, &dir, &p, &biz, o["contracts"] == Value::Bool(true), &formats)?;
            Ok(json!({ "changes": changes, "files": files(&out), "version": p.version }))
        }
    }
}

async fn clients(name: &str, o: &Value, l: &Localizer) -> Res {
    let base = web::base()?;
    match name {
        "clients" => Ok(json!(client::list(&base)
            .into_iter()
            .map(|id| {
                let c = client::load(&base, &id).unwrap_or_default();
                let open = client::tickets(&base, &id).iter().filter(|t| t.open).count();
                json!({ "id": id, "client": c, "openTickets": open })
            })
            .collect::<Vec<_>>())),
        "clientAdd" => {
            let id = need(o, "id")?;
            let mut c = client::load(&base, id).unwrap_or_default();
            for (k, f) in [("name", &mut c.name), ("email", &mut c.email), ("telegram", &mut c.telegram)] {
                if let Some(v) = st(o, k) {
                    v.clone_into(f);
                }
            }
            if let Some(r) = o["rate"].as_f64() {
                c.rate = r;
            }
            c.sites = strings(o, "sites");
            c.hosts = strings(o, "hosts");
            c.urls = strings(o, "urls");
            Ok(json!(client::save(&base, id, &c)?.display().to_string()))
        }
        "clientLog" => {
            let id = need(o, "id")?;
            client::load(&base, id)?;
            let h = o["hours"].as_f64().filter(|h| (0.0..=24.0).contains(h)).ok_or("Saat 0–24 olmalı")?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let secs = (h * 3600.0).round() as u64;
            client::log(&base, id, &client::Entry { ts: web::now(), kind: "manual".into(), target: String::new(), summary: need(o, "note")?.to_owned(), secs, source: "manual".into() })?;
            Ok(json!(true))
        }
        "ticket" => {
            let id = need(o, "id")?;
            match need(o, "action")? {
                "add" => to_json(&client::ticket_add(&base, id, need(o, "title")?)?),
                "close" => Ok(json!(client::ticket_close(&base, id, o["number"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("Bilet numarası")?)?)),
                _ => to_json(&client::tickets(&base, id)),
            }
        }
        "clientCheck" => to_json(&client::check(&base, need(o, "id")?, &web::http::Http::new()?).await?),
        _ => {
            let id = need(o, "id")?;
            let month = st(o, "month").map_or_else(|| chrono::Local::now().format("%Y-%m").to_string(), str::to_owned);
            let live = client::check(&base, id, &web::http::Http::new()?).await?;
            let biz = proposal::business(&base).unwrap_or_default();
            let r = client::report(l, &base, id, &month, &live, biz.hourly_rate)?;
            let mut formats = strings(o, "format");
            if formats.is_empty() {
                formats = vec!["html".into(), "pdf".into()];
            }
            let out = client::write_report(&base, id, &month, &r, &biz.accent, &formats)?;
            Ok(json!({ "markdown": r.markdown, "telegram": r.telegram, "files": files(&out) }))
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn turkey(name: &str, o: &Value, l: &Localizer, project: Option<&Path>) -> Res {
    match name {
        "trCheck" => {
            let base = web::base()?;
            let plugins: Vec<String> = match st(o, "site") {
                Some(s) => match web::tools::open_wp(&base, s) {
                    Ok(w) => w.inventory().await.map(|i| i.plugins.into_iter().map(|p| p.slug).collect()).unwrap_or_default(),
                    Err(_) => Vec::new(),
                },
                None => Vec::new(),
            };
            let rep = tr::check(&web::http::Http::new()?, need(o, "url")?, &plugins).await?;
            Ok(json!({ "markdown": tr::render(l, &rep), "report": rep }))
        }
        "trSlug" => Ok(json!(tr::slug(need(o, "text")?))),
        "trKdv" => {
            let k = tr::parse_try(need(o, "amount")?).ok_or("Tutar okunamadı")?;
            let rate = o["rate"].as_u64().and_then(|r| u32::try_from(r).ok()).unwrap_or(20);
            let (net, tax, gross) = if o["gross"] == Value::Bool(true) {
                let (n, t) = tr::kdv_from_gross(k, rate);
                (n, t, k)
            } else {
                let (t, g) = tr::kdv(k, rate);
                (k, t, g)
            };
            Ok(json!({ "net": tr::try_format(net), "kdv": tr::try_format(tax), "gross": tr::try_format(gross), "rate": rate }))
        }
        "trKvkk" => {
            let base = web::base()?;
            let biz = proposal::business(&base)?;
            let cookies = strings(o, "cookies")
                .iter()
                .map(|c| {
                    let mut p = c.splitn(3, '|').map(|x| x.trim().to_owned());
                    (p.next().unwrap_or_default(), p.next().unwrap_or_default(), p.next().unwrap_or_default())
                })
                .collect();
            let input = tr::KvkkInput {
                company: biz.name.clone(),
                address: format!("{} {}", biz.address, biz.city).trim().to_owned(),
                email: biz.email.clone(),
                website: biz.website.clone(),
                data: strings(o, "data"),
                purposes: strings(o, "purposes"),
                recipients: strings(o, "recipients"),
                abroad: strings(o, "abroad"),
                cookies,
            };
            let (d, c) = tr::kvkk_text(&input);
            let dir = project.ok_or("Önce bir proje klasörü açın")?.join("kvkk");
            let mut out = docs::write_all(&dir, "aydinlatma-metni", &d, "Aydınlatma Metni", &biz.accent, &["html".into()])?;
            out.extend(docs::write_all(&dir, "cerez-politikasi", &c, "Çerez Politikası", &biz.accent, &["html".into()])?);
            Ok(json!({ "files": files(&out), "disclosure": d, "cookies": c }))
        }
        "trEfatura" | "trIrsaliye" => {
            let uuid = tr::efatura::uuid4()?;
            let xml = if name == "trEfatura" {
                let inv: tr::efatura::Invoice = serde_json::from_value(o["document"].clone()).map_err(|e| e.to_string())?;
                tr::efatura::invoice_xml(&inv, &uuid)?
            } else {
                let d: tr::efatura::Despatch = serde_json::from_value(o["document"].clone()).map_err(|e| e.to_string())?;
                tr::efatura::despatch_xml(&d, &uuid)?
            };
            let mut out = json!({ "uuid": uuid, "xml": xml });
            if let Some(dir) = project {
                let p = dir.join(format!("{}-{uuid}.xml", if name == "trEfatura" { "efatura" } else { "eirsaliye" }));
                std::fs::write(&p, &xml).map_err(|e| e.to_string())?;
                out["saved"] = json!(p.display().to_string());
            }
            if o["send"] == Value::Bool(true) {
                let base = web::base()?;
                let biz = proposal::business(&base)?;
                if biz.efatura.url.is_empty() || biz.efatura.secret.is_empty() {
                    return Err("business.toml içinde [efatura] url ve secret tanımlı değil".into());
                }
                let token = web::secret(web::vault().ok().as_deref(), &biz.efatura.secret)?;
                out["sent"] = json!(tr::efatura::send(&web::http::Http::new()?, &biz.efatura.url, token.expose_secret(), &xml).await?);
            }
            Ok(out)
        }
        "trPayment" => {
            let list = tr::templates::payment(need(o, "provider")?).ok_or("iyzico | paytr")?;
            Ok(files(&wp::write_scaffold(project.ok_or("Önce bir proje klasörü açın")?, &list)?))
        }
        "trMarket" => {
            let base = web::base()?;
            let biz = proposal::business(&base)?;
            let store = web::vault().ok();
            let get = |n: &str| web::secret(store.as_deref(), n).map(|s| s.expose_secret().to_owned());
            let http = web::http::Http::new()?;
            let mut out = Vec::new();
            if !biz.trendyol.id.is_empty() {
                let r = tr::market::trendyol(&http, tr::market::TRENDYOL, &biz.trendyol.id, &get(&biz.trendyol.key)?, &get(&biz.trendyol.secret)?).await;
                out.push(r.map_or_else(|e| json!({ "market": "Trendyol", "error": e }), |c| json!(c)));
            }
            if !biz.hepsiburada.id.is_empty() {
                let r = tr::market::hepsiburada(&http, tr::market::HEPSIBURADA, &biz.hepsiburada.id, &get(&biz.hepsiburada.key)?, &get(&biz.hepsiburada.secret)?).await;
                out.push(r.map_or_else(|e| json!({ "market": "Hepsiburada", "error": e }), |c| json!(c)));
            }
            Ok(json!(out))
        }
        _ => {
            let src = need(o, "source")?;
            let text = if src.starts_with("http") {
                tr::strip_tags(&web::http::Http::new()?.follow(src).await?.1.body)
            } else {
                src.to_owned()
            };
            let top = o["top"].as_u64().and_then(|t| usize::try_from(t).ok()).unwrap_or(20);
            Ok(json!(tr::keywords(&text, top)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_helpers() {
        let o = obj(&[json!({"name": " prod ", "only": "a, b", "values": {"domain": "a.com", "n": 1}})]);
        assert_eq!(need(&o, "name").unwrap(), "prod");
        assert!(need(&o, "missing").is_err());
        assert_eq!(strings(&o, "only"), ["a", "b"]);
        assert_eq!(values(&o).len(), 1);
        assert_eq!(obj(&[json!("shop")])["name"], "shop");
    }
}
