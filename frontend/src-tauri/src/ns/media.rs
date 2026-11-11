//! `media.*`: `kor media` ve `kor hf` komutlarının masaüstü karşılıkları (parity.toml). İş mantığı
//! `kor_providers::{fal, huggingface}` ve `kor_agent::media` içinde; burası yalnızca JSON'a çevirir
//! ve ilerlemeyi `kor:media/progress` / `kor:media/hfProgress` olaylarıyla yayar.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use kor_providers::CancellationToken;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Çalışan medya işleri (`falRun`, `falImage`, `hfDownload`): iş kimliği → iptal belirteci.
/// Kimlik ilerleme olaylarında (`jobId`) arayüze gider; `media.cancel` onunla iptal eder,
/// `media.jobs` (`{ jobId, kind, target, startedAt }`) panel sonradan açılsa da listeler.
static JOBS: LazyLock<Mutex<HashMap<String, (CancellationToken, Value)>>> =
    LazyLock::new(Mutex::default);
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, (CancellationToken, Value)>> {
    JOBS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Kayıtlı bir iş; düşürülünce kayıttan silinir (başarı, hata veya iptal).
struct Job {
    id: String,
    cancel: CancellationToken,
}

impl Job {
    /// `kind`: `fal` | `hf`; `target`: model ya da depo kimliği.
    fn start(kind: &str, target: &str) -> Self {
        let id = format!("media-{}", NEXT_JOB.fetch_add(1, Ordering::Relaxed));
        let cancel = CancellationToken::new();
        let meta = json!({
            "jobId": id, "kind": kind, "target": target, "startedAt": kor_providers::catalog::now(),
        });
        jobs().insert(id.clone(), (cancel.clone(), meta));
        Self { id, cancel }
    }
}

fn list_jobs() -> Value {
    let mut v: Vec<Value> = jobs().values().map(|(_, m)| m.clone()).collect();
    v.sort_by_key(|m| m["startedAt"].as_u64().unwrap_or(0));
    Value::Array(v)
}

impl Drop for Job {
    fn drop(&mut self) {
        jobs().remove(&self.id);
    }
}

/// `media.cancel(jobId?)`: verilen işi (yoksa hepsini) iptal eder. fal işi fal tarafında da iptal
/// edilir (`Fal::run`, CLI'daki Ctrl-C ile aynı yol); indirme `.part` dosyasını bırakır ve sonra
/// kaldığı yerden sürer. Her iptal `kor:media/cancelled` olayıyla bildirilir.
fn cancel(job: Option<&str>, app: &AppHandle) -> Res {
    let hit: Vec<String> = jobs()
        .iter()
        .filter(|(id, _)| job.is_none_or(|j| j == id.as_str()))
        .map(|(id, (c, _))| {
            c.cancel();
            id.clone()
        })
        .collect();
    for id in &hit {
        let _ = app.emit("kor:media/cancelled", json!({ "jobId": id }));
    }
    match (job, hit.is_empty()) {
        (Some(j), true) => Err(format!("Çalışan medya işi yok: {j}")),
        _ => Ok(json!({ "cancelled": hit })),
    }
}

fn l() -> kor_i18n::Localizer {
    kor_i18n::Localizer::new(kor_i18n::Lang::En)
}

fn privacy(state: &AppState) -> bool {
    kor_runtime::privacy_mode()
        || root(state)
            .ok()
            .and_then(|r| crate::bridge::load_config(&r).ok())
            .is_some_and(|c| c.config.privacy.mode)
}

fn fal() -> Result<kor_providers::fal::Fal, String> {
    kor_runtime::fal_client(&l())
        .map(|(f, _)| f)
        .ok_or_else(|| "fal anahtarı yok: FAL_KEY ayarlayın veya `kor auth login fal`".into())
}

fn hf(endpoint: Option<&str>) -> Result<kor_providers::huggingface::Hf, String> {
    kor_runtime::hf_client(&l(), endpoint).map(|(h, _)| h)
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("media.{name}");
    let m = m.as_str();
    // İptal ağ kullanmaz; gizlilik modunda da çalışır.
    if name == "cancel" {
        return Some(cancel(opt_s(args, 0), app));
    }
    if name == "jobs" {
        return Some(Ok(list_jobs()));
    }
    // Galeri yereldir (§20.3): gizlilik modunda da listelenir, gösterilir, projeye kopyalanır.
    match name {
        "gallery" => return Some(root(state).map(|r| gallery(&r))),
        "item" => return Some(s(args, 0, m).and_then(|id| item(state, id))),
        "export" => {
            return Some(match (s(args, 0, m), s(args, 1, m), root(state)) {
                (Ok(id), Ok(dest), Ok(r)) => kor_agent::gallery::export(&r, id, dest)
                    .map(|p| json!({ "path": kor_tools::display_path(&p) })),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
            });
        }
        "pending" => return Some(root(state).map(|r| json!(kor_agent::gallery::jobs(&r)))),
        "pasteRef" => return Some(paste_ref(state)),
        "importRef" => return Some(s(args, 0, m).and_then(|p| import_ref(state, p))),
        _ => {}
    }
    if !matches!(
        name,
        "falModels"
            | "falRun"
            | "falImage"
            | "hfSearch"
            | "hfInfo"
            | "hfDownload"
            | "hfInfer"
            | "catalog"
            | "plan"
            | "generate"
            | "collect"
    ) {
        return None;
    }
    if privacy(state) {
        return Some(Err(
            "Gizlilik modu açık: medya sağlayıcılarına bağlanılmıyor".into(),
        ));
    }
    Some(match name {
        "falModels" => fal_models(opt_s(args, 0), opt_s(args, 1), opt_s(args, 2)).await,
        "falRun" => match (s(args, 0, m), args.get(1).cloned()) {
            (Ok(model), input) => {
                let input = input.filter(Value::is_object).unwrap_or_else(|| json!({}));
                fal_run(model, input, app, state).await
            }
            (Err(e), _) => Err(e),
        },
        "falImage" => match s(args, 0, m) {
            Ok(prompt) => {
                let model = opt_s(args, 1).unwrap_or(kor_providers::fal::DEFAULT_IMAGE_MODEL);
                fal_run(model, json!({ "prompt": prompt }), app, state).await
            }
            Err(e) => Err(e),
        },
        "hfSearch" => hf_search(opt_s(args, 0), opt_s(args, 1), opt_s(args, 2)).await,
        "hfInfo" => match s(args, 0, m) {
            Ok(repo) => hf_info(repo).await,
            Err(e) => Err(e),
        },
        "hfDownload" => match s(args, 0, m) {
            Ok(repo) => hf_download(repo, opt_s(args, 1), opt_s(args, 2), app).await,
            Err(e) => Err(e),
        },
        "catalog" => catalog(opt_s(args, 0), b(args, 1)).await,
        "plan" => match (s(args, 0, m), s(args, 1, m)) {
            (Ok(cap), Ok(model)) => {
                plan(
                    state,
                    cap,
                    model,
                    opt_s(args, 2).unwrap_or_default(),
                    args.get(3),
                )
                .await
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "generate" => match (s(args, 0, m), s(args, 1, m)) {
            (Ok(cap), Ok(model)) => {
                let req = GenRequest {
                    cap,
                    model,
                    prompt: opt_s(args, 2).unwrap_or_default(),
                    params: args.get(3),
                    reference: opt_s(args, 4),
                    options: args.get(5),
                };
                generate(req, app, state).await
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "collect" => collect(opt_s(args, 0), app, state).await,
        "hfInfer" => match (s(args, 0, m), s(args, 1, m), s(args, 2, m)) {
            (Ok(model), Ok(task), Ok(input)) => match (hf(opt_s(args, 3)), root(state)) {
                (Ok(h), Ok(r)) => kor_agent::media::hf_infer(&h, model, task, input, &r)
                    .await
                    .map(|text| json!({ "text": text })),
                (Err(e), _) | (_, Err(e)) => Err(e),
            },
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        },
        _ => return None,
    })
}

async fn fal_models(query: Option<&str>, category: Option<&str>, model: Option<&str>) -> Res {
    let f = fal()?;
    if let Some(id) = model {
        let schema = f.input_schema(id).await.map_err(|e| e.message)?;
        let price =
            f.price(id).await.ok().flatten().map(
                |p| json!({ "unitPrice": p.unit_price, "unit": p.unit, "currency": p.currency }),
            );
        return Ok(json!({ "model": id, "schema": schema, "price": price }));
    }
    let list = f.models(query, category, 40).await.map_err(|e| e.message)?;
    Ok(json!({
        "models": list.iter().map(|m| json!({
            "id": m.endpoint_id, "name": m.name, "category": m.category, "description": m.description,
        })).collect::<Vec<_>>()
    }))
}

async fn fal_run(model: &str, input: Value, app: &AppHandle, state: &AppState) -> Res {
    let f = fal()?;
    let r = root(state)?;
    // §20.2: eksik zorunlu girdiler önce arayüze döner; arayüz sorup tekrar çağırır.
    if let Ok(schema) = f.input_schema(model).await {
        let missing = kor_providers::fal::missing_inputs(&schema, &input);
        if !missing.is_empty() {
            return Ok(json!({ "missing": missing.iter().map(|m| json!({
                "name": m.name, "description": m.description, "options": m.options, "default": m.default,
            })).collect::<Vec<_>>() }));
        }
    }
    let prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let stem = kor_agent::media::stem(None, if prompt.is_empty() { model } else { &prompt });
    let job = Job::start("fal", model);
    let on = |st: &kor_providers::fal::QueueStatus| {
        let _ = app.emit(
            "kor:media/progress",
            json!({ "jobId": job.id, "model": model, "status": kor_providers::fal::status_line(st) }),
        );
    };
    let env = media_env(state);
    let save_to = kor_agent::media::FalSave {
        root: &r,
        stem: &stem,
        meta: kor_agent::gallery::Meta {
            provider: "fal".into(),
            model: model.to_owned(),
            prompt,
            params: input.clone(),
            ..kor_agent::gallery::Meta::default()
        },
        ledger: env.ledger.as_deref(),
    };
    let saved =
        kor_agent::media::fal_run_save(&f, model, &input, &save_to, &job.cancel, &on).await?;
    let _ = app.emit("kor:media/done", json!({ "jobId": job.id, "paths": saved.paths.iter().map(|p| kor_tools::display_path(p)).collect::<Vec<_>>() }));
    Ok(json!({
        "jobId": job.id,
        "paths": saved.paths.iter().map(|p| kor_tools::display_path(p)).collect::<Vec<_>>(),
        "seconds": saved.secs,
        "report": saved.report(model),
    }))
}

// ---------------------------------------------------------------------------------------------
// §20 yetenek kataloğu, parametre kartı, galeri ve devam eden işler (ADR 0060)
// ---------------------------------------------------------------------------------------------

fn media_env(state: &AppState) -> kor_agent::media::MediaEnv {
    let cfg = root(state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok());
    kor_runtime::media::env(cfg.as_ref().map(|c| &c.config.media))
}

fn capability(s: &str) -> Result<kor_providers::media_catalog::Capability, String> {
    kor_providers::media_catalog::Capability::parse(s)
        .ok_or_else(|| format!("Bilinmeyen yetenek: {s}"))
}

/// `media.catalog(capability?, refresh?)`: yapılandırılmış sağlayıcıların modelleri (günlük önbellek).
async fn catalog(cap: Option<&str>, refresh: bool) -> Res {
    let cap = cap.filter(|c| !c.is_empty()).map(capability).transpose()?;
    let c = kor_runtime::media::Clients::load(&l());
    let cat = kor_runtime::media::catalog(&c, refresh).await;
    Ok(json!({
        "fetched": cat.fetched,
        "providers": cat.providers,
        "errors": cat.errors,
        "models": cat.with(cap),
    }))
}

async fn find_model(
    c: &kor_runtime::media::Clients,
    cat: &kor_runtime::media::Catalog,
    cap: kor_providers::media_catalog::Capability,
    model: &str,
) -> Result<kor_providers::media_catalog::CatalogModel, String> {
    if let Some(m) = cat.find(model) {
        return Ok(m.clone());
    }
    let id = model.strip_prefix("fal:").unwrap_or(model);
    let f = c
        .fal
        .as_ref()
        .ok_or_else(|| format!("Katalogda yok: {model}"))?;
    let schema = f.input_schema(id).await.map_err(|e| e.message)?;
    let price = f.price(id).await.ok().flatten();
    Ok(kor_providers::media_catalog::fal_model(
        id,
        cap,
        Some(&schema),
        price.as_ref(),
    ))
}

/// `media.plan(capability, model, prompt, params?)`: §20.2 kartı — sorulacaklar (seçenekler ve
/// varsayılanlarla), modelin yapamadığı değerler (en yakını ve yapabilen modeller, ucuzdan
/// pahalıya), tahmini maliyet ve süre, `media.confirm_above` onayı.
async fn plan(
    state: &AppState,
    cap: &str,
    model: &str,
    prompt: &str,
    params: Option<&Value>,
) -> Res {
    use kor_providers::media_catalog as mc;
    let cap = capability(cap)?;
    let r = root(state)?;
    let env = media_env(state);
    let c = kor_runtime::media::Clients::load(&l());
    let cat = kor_runtime::media::catalog(&c, false).await;
    let m = find_model(&c, &cat, cap, model).await?;
    let given = params
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let p = mc::plan(
        &m,
        prompt,
        &given,
        &kor_runtime::media::defaults_for(&env, cap),
        !env.no_ask,
    );
    let alternatives: Vec<Value> = p
        .unsupported
        .iter()
        .map(|u| {
            let alts = mc::alternatives(&cat.models, cap, &m, mc::role_of(&m, &u.name), &u.asked, prompt);
            json!({
                "name": u.name,
                "models": alts.iter().map(|(a, e)| json!({ "model": a, "estimate": e })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let confirm = env.confirm_above > 0.0
        && p.estimate
            .as_ref()
            .is_some_and(|e| e.currency == "USD" && e.amount > env.confirm_above);
    Ok(json!({
        "model": m,
        "plan": p,
        "estimateText": p.estimate.as_ref().map(mc::Estimate::display),
        "alternatives": alternatives,
        "eta": kor_agent::gallery::eta(&r, &m.id),
        "confirm": confirm,
        "confirmAbove": env.confirm_above,
    }))
}

struct GenRequest<'a> {
    cap: &'a str,
    model: &'a str,
    prompt: &'a str,
    params: Option<&'a Value>,
    reference: Option<&'a str>,
    /// `{ name, saveAs, action, derivedFrom }`
    options: Option<&'a Value>,
}

/// `media.generate(...)`: planlanmış (kartta yanıtlanmış) üretimi çalıştırır; ilerleme
/// `kor:media/progress`, bitiş `kor:media/done` olaylarıyla ve bildirim merkezine gider.
async fn generate(req: GenRequest<'_>, app: &AppHandle, state: &AppState) -> Res {
    let cap = capability(req.cap)?;
    let r = root(state)?;
    let env = media_env(state);
    let c = kor_runtime::media::Clients::load(&l());
    let cat = kor_runtime::media::catalog(&c, false).await;
    let model = find_model(&c, &cat, cap, req.model).await?;
    let opt = |k: &str| {
        req.options
            .and_then(|o| o.get(k))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let run = kor_agent::media::CatalogRun {
        capability: Some(cap),
        prompt: req.prompt.to_owned(),
        params: req
            .params
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
        reference: req.reference.filter(|s| !s.is_empty()).map(str::to_owned),
        action: opt("action"),
        name: opt("name"),
        save_as: opt("saveAs"),
    };
    let (tool, mut input) = kor_agent::media::catalog_call(&model, &run)?;
    if let Some(d) = opt("derivedFrom") {
        input["derived_from"] = json!(d);
    }
    let tools = kor_runtime::media::tools(&c, &env);
    let t = tools
        .iter()
        .find(|t| t.name() == tool)
        .ok_or_else(|| format!("{tool}: {} yapılandırılmamış", model.provider))?;
    let mut tctx = kor_tools::ToolCtx::new(&r).map_err(|e| e.to_string())?;
    let job = Job::start("gen", &model.reference());
    tctx.cancel = job.cancel.clone();
    let (id, target) = (job.id.clone(), model.reference());
    let emitter = app.clone();
    let sink: kor_tools::ProgressSink = std::sync::Arc::new(move |line: String| {
        let _ = emitter.emit(
            "kor:media/progress",
            json!({ "jobId": id, "model": target, "status": line }),
        );
    });
    let out = kor_tools::with_progress(sink, t.call(input, &tctx)).await;
    if out.is_error {
        kor_runtime::media::notify(false, "Medya üretimi başarısız", &out.content, None);
        return Err(out.content);
    }
    let paths: Vec<String> = out
        .content
        .lines()
        .filter_map(|l| l.strip_prefix("saved: "))
        .map(|p| p.trim().to_owned())
        .collect();
    let items: Vec<Value> = paths
        .iter()
        .filter_map(|p| {
            std::path::Path::new(p)
                .file_name()?
                .to_str()
                .map(str::to_owned)
        })
        .filter_map(|id| kor_agent::gallery::get(&r, &id))
        .map(|i| json!(i))
        .collect();
    let body = json!({ "jobId": job.id, "paths": paths, "items": items, "report": out.content });
    let _ = app.emit("kor:media/done", body.clone());
    kor_runtime::media::notify(
        true,
        "Medya hazır",
        &format!("{}: {}", model.reference(), req.prompt),
        None,
    );
    Ok(body)
}

/// `media.collect(jobId?)`: önceki bir oturumun kuyruğa aldığı istekleri bekleyip kaydeder.
async fn collect(id: Option<&str>, app: &AppHandle, state: &AppState) -> Res {
    let r = root(state)?;
    let env = media_env(state);
    let c = kor_runtime::media::Clients::load(&l());
    let list: Vec<kor_agent::gallery::Job> = kor_agent::gallery::jobs(&r)
        .into_iter()
        .filter(|j| id.is_none_or(|i| i == j.id))
        .collect();
    if list.is_empty() {
        return Err("Bekleyen medya isteği yok".into());
    }
    let mut out = Vec::new();
    for j in &list {
        let job = Job::start("collect", &j.model);
        let fal = c.fal_for(&j.provider);
        let b = kor_agent::media::Backends {
            fal: fal.as_ref(),
            venice: c.venice.as_ref(),
        };
        let on = |s: &str| {
            let _ = app.emit(
                "kor:media/progress",
                json!({ "jobId": job.id, "model": j.model, "status": s }),
            );
        };
        let saved =
            kor_agent::media::collect(j, b, &r, env.ledger.as_deref(), &job.cancel, &on).await?;
        let paths: Vec<String> = saved
            .paths
            .iter()
            .map(|p| kor_tools::display_path(p))
            .collect();
        let _ = app.emit("kor:media/done", json!({ "jobId": job.id, "paths": paths }));
        kor_runtime::media::notify(
            true,
            "Medya hazır",
            &format!("{}: {}", j.model, j.prompt),
            j.cost.as_ref(),
        );
        out.push(json!({ "job": j.id, "paths": paths }));
    }
    Ok(json!({ "collected": out }))
}

/// `media.gallery()`: projedeki üretilmiş medya, yeniden eskiye; her öğe önizleme için mutlak yoluyla.
fn gallery(r: &std::path::Path) -> Value {
    Value::Array(
        kor_agent::gallery::list(r)
            .into_iter()
            .map(|i| {
                let mut v = json!(i);
                v["path"] = json!(kor_tools::display_path(&r.join(&i.file)));
                v
            })
            .collect(),
    )
}

/// `media.item(id)`: öğe, türediği zincir (yeniden eskiye) ve ondan türeyenler.
fn item(state: &AppState, id: &str) -> Res {
    let r = root(state)?;
    let chain = kor_agent::gallery::chain(&r, id);
    let first = chain
        .first()
        .cloned()
        .ok_or_else(|| format!("Galeride yok: {id}"))?;
    Ok(json!({
        "item": first,
        "path": kor_tools::display_path(&r.join(&first.file)),
        "chain": chain.iter().skip(1).map(|i| json!({ "item": i, "path": kor_tools::display_path(&r.join(&i.file)) })).collect::<Vec<_>>(),
        "children": kor_agent::gallery::children(&r, id),
    }))
}

/// Panodaki görseli galeriye referans olarak kaydeder (§20.3 Ctrl/Cmd+V).
fn paste_ref(state: &AppState) -> Res {
    let r = root(state)?;
    let pic = kor_runtime::visual::clipboard_image()?.ok_or("Panoda görsel yok")?;
    let ext = pic.media_type.rsplit('/').next().unwrap_or("png");
    let dir = kor_agent::gallery::dir(&r);
    let name = format!("clipboard-{}.{ext}", kor_runtime::cost::now_ms());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(&name), &pic.bytes).map_err(|e| e.to_string())?;
    Ok(json!({ "id": name, "path": kor_tools::display_path(&dir.join(&name)) }))
}

/// Sürüklenen bir dosyayı (görsel/ses/video) galeriye referans olarak kopyalar (§20.3).
fn import_ref(state: &AppState, path: &str) -> Res {
    const MAX: u64 = 100 * 1024 * 1024;
    let r = root(state)?;
    let src = std::path::Path::new(path);
    if kor_agent::gallery::kind_of(src) == "other" {
        return Err("Desteklenen türler: görsel, video, ses".into());
    }
    let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX {
        return Err("Dosya yok ya da 100 MB'tan büyük".into());
    }
    let dir = kor_agent::gallery::dir(&r);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = kor_agent::media::stem(src.file_stem().and_then(|s| s.to_str()), "ref");
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let name = format!("{stem}-{}.{ext}", kor_runtime::cost::now_ms());
    std::fs::copy(src, dir.join(&name)).map_err(|e| e.to_string())?;
    Ok(json!({ "id": name, "path": kor_tools::display_path(&dir.join(&name)) }))
}

async fn hf_search(query: Option<&str>, pipeline: Option<&str>, filter: Option<&str>) -> Res {
    let q = kor_providers::huggingface::SearchQuery {
        search: query.map(str::to_owned),
        pipeline_tag: pipeline.map(str::to_owned),
        filter: filter.map(|f| vec![f.to_owned()]).unwrap_or_default(),
        limit: 30,
        ..Default::default()
    };
    let list = hf(None)?.search(&q).await.map_err(|e| e.message)?;
    Ok(json!({ "models": list.iter().map(|m| json!({
        "id": m.id, "downloads": m.downloads, "likes": m.likes, "pipeline": m.pipeline_tag, "library": m.library,
    })).collect::<Vec<_>>() }))
}

async fn hf_info(repo: &str) -> Res {
    let h = hf(None)?;
    let info = h.info(repo, None).await.map_err(|e| e.message)?;
    let files = h.files(repo, None).await.map_err(|e| e.message)?;
    Ok(json!({
        "id": info.get("id"),
        "pipeline": info.get("pipeline_tag"),
        "library": info.get("library_name"),
        "license": info.pointer("/cardData/license"),
        "downloads": info.get("downloads"),
        "likes": info.get("likes"),
        "gated": kor_providers::huggingface::gated(&info),
        "files": files.iter().map(|f| json!({ "path": f.path, "size": f.size, "sha256": f.sha256 })).collect::<Vec<_>>(),
    }))
}

/// Boyut onayı arayüzde (`hfInfo` dosya boyutlarını verir); burası indirir ve doğrular.
async fn hf_download(
    repo: &str,
    file: Option<&str>,
    include: Option<&str>,
    app: &AppHandle,
) -> Res {
    let h = hf(None)?;
    let dest = kor_runtime::kor_home()
        .map(|d| d.join("models"))
        .ok_or("Modeller klasörü bulunamadı")?;
    let files: Vec<_> = h
        .files(repo, None)
        .await
        .map_err(|e| e.message)?
        .into_iter()
        .filter(|f| match (file, include) {
            (Some(n), _) => f.path == n,
            (None, Some(p)) => kor_providers::huggingface::glob_match(p, &f.path),
            (None, None) => true,
        })
        .collect();
    if files.is_empty() {
        return Err("Eşleşen dosya yok".into());
    }
    let job = Job::start("hf", repo);
    let mut out = Vec::new();
    for f in &files {
        let last = std::sync::atomic::AtomicU64::new(u64::MAX);
        let on = |done: u64, total: u64| {
            let pct = (done * 100).checked_div(total).unwrap_or(0);
            if last.swap(pct, std::sync::atomic::Ordering::Relaxed) == pct {
                return;
            }
            let _ = app.emit(
                "kor:media/hfProgress",
                json!({ "jobId": job.id, "repo": repo, "file": f.path, "done": done, "total": total }),
            );
        };
        let d = h
            .download(repo, None, f, &dest, &job.cancel, &on)
            .await
            .map_err(|e| e.message)?;
        out.push(json!({ "path": kor_tools::display_path(&d.path), "verified": d.verified }));
    }
    Ok(json!({ "jobId": job.id, "files": out }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_register_cancel_and_unregister() {
        let job = Job::start("hf", "org/repo");
        let id = job.id.clone();
        assert!(jobs().contains_key(&id));
        assert!(
            list_jobs()
                .as_array()
                .unwrap()
                .iter()
                .any(|j| j["jobId"] == id.as_str() && j["target"] == "org/repo")
        );
        // `cancel` itself needs an AppHandle; the token it flips is the job's own.
        jobs().get(&id).unwrap().0.cancel();
        assert!(job.cancel.is_cancelled());
        drop(job);
        assert!(!jobs().contains_key(&id));
    }
}
