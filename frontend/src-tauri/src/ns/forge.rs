//! `forge.*`: Evrensel Git (§8.8, ADR 0040) için ince köprü (`kor pr …`, `kor review`). İş mantığı
//! `kor_runtime::forge` ve `kor_runtime::review` içinde; burası JSON'a çevirir. Gönderme, PR açma,
//! yorum yazma yalnızca arayüzdeki onay düğmesinden sonra çağrılan yöntemlerle olur.
//!
//! - `forge.detect()` → `{ remote | null, error | null, branch, base, hasUpstream }`
//! - `forge.personas()` → `{ personas: [{ name, source, title }] }`
//! - `forge.prDescribe(base?, model?)` → `{ title, body, head, base, firstPush, note }` (yalnızca okur)
//! - `forge.prCreate({ title, body, base, draft })` → `{ url, number, forge }` (dalı gönderir)
//! - `forge.prList()` → `{ prs }`; `forge.prStatus()` → `{ branch, pr | null }`
//! - `forge.prReview(target, persona?, model?)` → `{ number, report, markdown }` (göndermez)
//! - `forge.prReviewPost(target, report)` → `{ inline, folded }`
//! - `forge.prRespond(target, model?)` → `{ pr, comments, replies, edits: [{ path, before, after }] }`
//! - `forge.prRespondApply(target, files: [{ path, content }], replies: [{ id, body }])` →
//!   `{ commit | null, sent }` (kabul edilen dosyaları yazar, commit eder, gönderir, yanıtlar)
//! - `forge.review({ persona?, staged?, range?, pr?, model? })` → `{ report, markdown }`

use std::path::{Path, PathBuf};

use kor_agent::gitflow;
use kor_runtime::forge::{self, Forge, ForgeError, PrRequest};
use kor_runtime::review::{self, DiffSource, Finding, Report, Severity};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, opt_s, root};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `forge::call`'a yönlendirir).
pub const NS: &str = "forge";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match name {
        "detect" => detect(state),
        "personas" => personas(state),
        "prDescribe" => describe(state, opt_s(args, 0), opt_s(args, 1)).await,
        "prCreate" => create(state, args.first()).await,
        "prList" => list(state).await,
        "prStatus" => status(state).await,
        "prReview" => pr_review(state, args).await,
        "prReviewPost" => review_post(state, args).await,
        "prRespond" => respond(state, args).await,
        "prRespondApply" => respond_apply(state, args).await,
        "review" => local_review(state, args.first()).await,
        _ => return None,
    })
}

struct Ctx {
    root: PathBuf,
    loaded: kor_config::Loaded,
    l: kor_i18n::Localizer,
}

/// Depo kökü, yapılandırma (ağ politikasını da uygular) ve kullanıcının dili.
fn ctx(state: &AppState) -> Result<Ctx, String> {
    let dir = root(state)?;
    let loaded = crate::bridge::load_config(&dir)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let root = gitflow::repo_root(&dir).map_err(|e| forge::error_text(&l, &e.into()))?;
    Ok(Ctx { root, loaded, l })
}

fn err(c: &Ctx) -> impl Fn(ForgeError) -> String + '_ {
    |e| forge::error_text(&c.l, &e)
}

fn open(c: &Ctx) -> Result<Forge, String> {
    Forge::open(&c.root).map_err(err(c))
}

fn target(args: &[Value]) -> Result<u64, String> {
    let t = match args.first() {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    forge::pr_number(&t).ok_or_else(|| format!("Pull request numarası ya da adresi değil: {t}"))
}

fn model(
    c: &Ctx,
    m: Option<&str>,
) -> Option<(std::sync::Arc<dyn kor_providers::Provider>, String)> {
    kor_runtime::oneshot_provider(&c.l, &c.loaded, m.filter(|m| !m.trim().is_empty())).ok()
}

fn detect(state: &AppState) -> Res {
    let c = ctx(state)?;
    let (remote, error) = match forge::detect(&c.root) {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(forge::error_text(&c.l, &e))),
    };
    let branch = forge::current_branch(&c.root).ok();
    let up = branch
        .as_deref()
        .is_some_and(|b| forge::has_upstream(&c.root, b));
    Ok(json!({
        "remote": remote,
        "error": error,
        "branch": branch,
        "base": forge::default_base(&c.root),
        "hasUpstream": up,
    }))
}

fn personas(state: &AppState) -> Res {
    let dir = root(state)?;
    let list: Vec<Value> = review::personas(&dir)
        .into_iter()
        .map(|p| {
            let title = p
                .prompt
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches('#')
                .trim()
                .to_owned();
            json!({ "name": p.name, "source": p.source, "title": title })
        })
        .collect();
    Ok(json!({ "personas": list }))
}

async fn describe(state: &AppState, base: Option<&str>, m: Option<&str>) -> Res {
    let c = ctx(state)?;
    let head = forge::current_branch(&c.root).map_err(err(&c))?;
    let base = base
        .filter(|b| !b.trim().is_empty())
        .map_or_else(|| forge::default_base(&c.root), |b| b.trim().to_owned());
    if head == base {
        return Err(format!(
            "Hedef dal olan {base} üzerindesiniz. Önce bir özellik dalına geçin."
        ));
    }
    let provider = model(&c, m);
    let (d, note) = forge::describe(
        provider.as_ref().map(|(p, m)| (p, m.as_str())),
        &c.root,
        &base,
        &head,
    )
    .await
    .map_err(err(&c))?;
    let note = match (provider.is_some(), note) {
        (_, Some(e)) => Some(format!(
            "Model açıklamayı yazamadı ({e}); commit listesi kullanıldı."
        )),
        (false, None) => {
            Some("Model ayarlı değil; açıklama commit listesinden yazıldı.".to_owned())
        }
        (true, None) => None,
    };
    Ok(json!({
        "title": d.title,
        "body": d.body,
        "head": head,
        "base": base,
        "firstPush": !forge::has_upstream(&c.root, &head),
        "note": note,
    }))
}

fn field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

async fn create(state: &AppState, input: Option<&Value>) -> Res {
    let c = ctx(state)?;
    let v = input.ok_or("forge.prCreate: girdi nesnesi gerekli")?;
    let title = field(v, "title").trim();
    if title.is_empty() || title.len() > 256 {
        return Err("Başlık 1-256 karakter olmalı".into());
    }
    let body = field(v, "body");
    if body.len() > 60_000 {
        return Err("Açıklama en fazla 60000 karakter olabilir".into());
    }
    let head = forge::current_branch(&c.root).map_err(err(&c))?;
    let base = match field(v, "base").trim() {
        "" => forge::default_base(&c.root),
        b => b.to_owned(),
    };
    // Önce forge ve belirteç: açılamayacak bir PR için hiçbir şey gönderilmez.
    let f = open(&c)?;
    forge::push(&c.root, &head).map_err(err(&c))?;
    let req = PrRequest {
        head,
        base,
        title: title.to_owned(),
        body: body.to_owned(),
        draft: v.get("draft").and_then(Value::as_bool).unwrap_or(false),
    };
    let r = f.create(&req).await.map_err(err(&c))?;
    Ok(json!(r))
}

async fn list(state: &AppState) -> Res {
    let c = ctx(state)?;
    let prs = open(&c)?.list().await.map_err(err(&c))?;
    Ok(json!({ "prs": prs }))
}

async fn status(state: &AppState) -> Res {
    let c = ctx(state)?;
    let branch = forge::current_branch(&c.root).map_err(err(&c))?;
    let pr = open(&c)?.for_branch(&branch).await.map_err(err(&c))?;
    Ok(json!({ "branch": branch, "pr": pr }))
}

fn persona(root: &Path, name: Option<&str>) -> Result<review::Persona, String> {
    let name = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(review::DEFAULT_PERSONA);
    review::persona(root, name).ok_or_else(|| format!("Bilinmeyen persona: {name}"))
}

async fn run_review(c: &Ctx, p: &review::Persona, diff: &str, m: Option<&str>) -> Value {
    let provider = model(c, m);
    let report = review::review(
        &c.l,
        provider.as_ref().map(|(p, m)| (p, m.as_str())),
        p,
        diff,
    )
    .await;
    json!({ "markdown": review::markdown(&c.l, &report), "report": report })
}

async fn pr_review(state: &AppState, args: &[Value]) -> Res {
    let c = ctx(state)?;
    let n = target(args)?;
    let p = persona(&c.root, opt_s(args, 1))?;
    let diff = open(&c)?.diff(n).await.map_err(err(&c))?;
    let mut out = run_review(&c, &p, &diff, opt_s(args, 2)).await;
    out["number"] = json!(n);
    Ok(out)
}

/// Arayüzün gösterdiği raporu geri okur (modeli yeniden çağırmadan gönderilir).
fn report_from(v: &Value) -> Result<Report, String> {
    let findings = v
        .get("findings")
        .and_then(Value::as_array)
        .ok_or("forge.prReviewPost: rapor gerekli")?
        .iter()
        .take(500)
        .map(|f| Finding {
            file: field(f, "file").to_owned(),
            line: f.get("line").and_then(Value::as_u64),
            severity: serde_json::from_value(f.get("severity").cloned().unwrap_or(Value::Null))
                .unwrap_or(Severity::Info),
            rule: field(f, "rule").to_owned(),
            message: field(f, "message").chars().take(4000).collect(),
            source: if field(f, "source") == "check" {
                "check"
            } else {
                "model"
            },
        })
        .collect();
    Ok(Report {
        persona: field(v, "persona").to_owned(),
        findings,
        model_error: None,
    })
}

async fn review_post(state: &AppState, args: &[Value]) -> Res {
    let c = ctx(state)?;
    let n = target(args)?;
    let report = report_from(args.get(1).unwrap_or(&Value::Null))?;
    let (body, notes) = review::to_notes(&c.l, &report);
    let r = open(&c)?
        .post_review(n, &body, &notes)
        .await
        .map_err(err(&c))?;
    Ok(json!(r))
}

async fn respond(state: &AppState, args: &[Value]) -> Res {
    let c = ctx(state)?;
    let n = target(args)?;
    let f = open(&c)?;
    let pr = f.get(n).await.map_err(err(&c))?;
    let branch = forge::current_branch(&c.root).map_err(err(&c))?;
    if branch != pr.head {
        return Err(format!(
            "Önce pull request'in dalı olan {} dalına geçin.",
            pr.head
        ));
    }
    let comments = f.comments(n).await.map_err(err(&c))?;
    if comments.is_empty() {
        return Ok(json!({ "pr": pr, "comments": comments, "replies": [], "edits": [] }));
    }
    let (p, m) = model(&c, opt_s(args, 1))
        .ok_or("Model ayarlı değil; yanıt taslağı için bir model seçin.")?;
    let r = forge::respond::draft(&p, &m, &c.root, &pr, &comments)
        .await
        .map_err(err(&c))?;
    Ok(json!({ "pr": pr, "comments": comments, "replies": r.replies, "edits": r.edits }))
}

async fn respond_apply(state: &AppState, args: &[Value]) -> Res {
    let c = ctx(state)?;
    let n = target(args)?;
    let files: Vec<(String, String)> = args
        .get(1)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| (field(f, "path").to_owned(), field(f, "content").to_owned()))
        .filter(|(p, _)| forge::respond::safe_path(&c.root, p).is_some())
        .collect();
    let replies: Vec<(String, String)> = args
        .get(2)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| (field(r, "id").to_owned(), field(r, "body").to_owned()))
        .filter(|(_, b)| !b.trim().is_empty())
        .collect();
    let f = open(&c)?;
    let pr = f.get(n).await.map_err(err(&c))?;
    let branch = forge::current_branch(&c.root).map_err(err(&c))?;
    if branch != pr.head {
        return Err(format!(
            "Önce pull request'in dalı olan {} dalına geçin.",
            pr.head
        ));
    }
    let commit = if files.is_empty() {
        None
    } else {
        let paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        forge::respond::write(&c.root, &files).map_err(err(&c))?;
        let msg = format!("fix: address review comments on #{n}");
        let id = forge::respond::commit(&c.root, &paths, &msg).map_err(err(&c))?;
        forge::push(&c.root, &branch).map_err(err(&c))?;
        Some(id)
    };
    let comments = f.comments(n).await.map_err(err(&c))?;
    let mut sent = 0usize;
    let mut errors = Vec::new();
    for (id, body) in &replies {
        let Some(cm) = comments.iter().find(|x| &x.id == id) else {
            continue;
        };
        match f.reply(n, cm, body).await {
            Ok(()) => sent += 1,
            Err(e) => errors.push(forge::error_text(&c.l, &e)),
        }
    }
    Ok(json!({ "commit": commit, "sent": sent, "errors": errors }))
}

async fn local_review(state: &AppState, opts: Option<&Value>) -> Res {
    let c = ctx(state)?;
    let o = opts.cloned().unwrap_or(Value::Null);
    let p = persona(&c.root, o.get("persona").and_then(Value::as_str))?;
    let diff = if let Some(t) = o.get("pr").filter(|v| !v.is_null()) {
        let n = target(std::slice::from_ref(t))?;
        open(&c)?.diff(n).await.map_err(err(&c))?
    } else {
        let src = diff_source(&o);
        review::local_diff(&c.root, &src).map_err(|e| forge::error_text(&c.l, &e.into()))?
    };
    if diff.trim().is_empty() {
        return Err("İncelenecek bir şey yok: diff boş.".into());
    }
    Ok(run_review(&c, &p, &diff, o.get("model").and_then(Value::as_str)).await)
}

fn diff_source(o: &Value) -> DiffSource {
    match o
        .get("range")
        .and_then(Value::as_str)
        .filter(|r| !r.trim().is_empty())
    {
        Some(r) => DiffSource::Range(r.trim().to_owned()),
        None if o.get("staged").and_then(Value::as_bool) == Some(true) => DiffSource::Staged,
        None => DiffSource::Working,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_sources_are_read() {
        assert_eq!(target(&[json!(12)]).unwrap(), 12);
        assert_eq!(
            target(&[json!("https://github.com/o/r/pull/7")]).unwrap(),
            7
        );
        assert!(target(&[]).is_err());
        assert_eq!(
            diff_source(&json!({ "range": "a..b" })),
            DiffSource::Range("a..b".into())
        );
        assert_eq!(diff_source(&json!({ "staged": true })), DiffSource::Staged);
        assert_eq!(diff_source(&Value::Null), DiffSource::Working);
    }

    #[test]
    fn reports_round_trip_from_the_renderer() {
        let r = report_from(&json!({ "persona": "security", "findings": [
            { "file": "a.rs", "line": 3, "severity": "high", "rule": "x", "message": "m", "source": "model" },
            { "file": "b.rs", "severity": "bogus", "message": "n", "source": "check" }
        ]}))
        .unwrap();
        assert_eq!(r.findings.len(), 2);
        assert_eq!(r.findings[0].severity, Severity::High);
        assert_eq!(
            (r.findings[1].severity, r.findings[1].source),
            (Severity::Info, "check")
        );
        assert!(report_from(&json!({})).is_err());
    }
}
