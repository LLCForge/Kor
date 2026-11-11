//! `bench.*`: `kor bench` için ince köprü (parity.toml, ADR 0015). İş mantığı
//! `kor_runtime::bench` içinde; burası seçenekleri okur, bütçe korumasını uygular, ilerlemeyi
//! `kor:bench/progress` ve bitişi `kor:bench/done` olaylarıyla yayar.
//!
//! Seçenekler (`estimate`, `start`): `{ file?, models?: [..], baseline?, routes?: {kategori: model},
//! maxCost?, confirm? }`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use kor_providers::CancellationToken;
use kor_runtime::bench::{self, Job, Spec};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::api::{Res, root};
use crate::state::AppState;

/// Köprü ad alanı (`ns/mod.rs` bunu `bench::call`'a yönlendirir).
pub const NS: &str = "bench";

#[derive(Default)]
struct Run {
    cancel: Option<CancellationToken>,
    last: Option<Value>,
}

static RUN: LazyLock<Mutex<Run>> = LazyLock::new(Mutex::default);

fn run_state() -> std::sync::MutexGuard<'static, Run> {
    RUN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let opts = args.first().cloned().unwrap_or(Value::Null);
    Some(match name {
        "estimate" => plan(&opts, state).map(|p| p.estimate_json()),
        "start" => start(&opts, app, state),
        "cancel" => Ok(cancel()),
        "result" => Ok(result()),
        _ => return None,
    })
}

/// Doğrulanmış bir çalıştırma planı.
struct Plan {
    root: PathBuf,
    loaded: kor_config::Loaded,
    l: kor_i18n::Localizer,
    spec: Spec,
    jobs: Vec<Job>,
    prices: bench::Prices,
    estimate: bench::Estimate,
    max_cost: f64,
}

impl Plan {
    fn estimate_json(&self) -> Value {
        json!({
            "jobs": self.jobs.len(),
            "usd": self.estimate.usd,
            "estimated": true,
            "unknown": self.estimate.unknown,
            "maxCost": self.max_cost,
            "withinBudget": bench::within_budget(&self.estimate, self.max_cost),
        })
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect()
}

fn plan(opts: &Value, state: &AppState) -> Result<Plan, String> {
    let root = root(state)?;
    let loaded = crate::bridge::load_config(&root)?;
    let l = kor_i18n::Localizer::new(loaded.config.ui.language.resolve());
    let file = opts["file"]
        .as_str()
        .map_or_else(|| root.join(bench::SPEC_FILE), PathBuf::from);
    // Tanımdaki komutlar (ve modelin yazdığı kod) bu makinede çalışır.
    if !kor_runtime::is_trusted(&root) {
        return Err(l.t_args(
            "bench-untrusted",
            &kor_i18n::args!("file" => file.display().to_string()),
        ));
    }
    let spec = Spec::load(&file)
        .map_err(|e| l.t_args("bench-spec-error", &kor_i18n::args!("error" => e)))?;
    let jobs = if let Some(base) = opts["baseline"].as_str() {
        let mut routes: BTreeMap<String, String> = opts["routes"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect();
        if routes.is_empty() {
            routes.clone_from(&spec.routing);
        }
        if routes.is_empty() {
            return Err(l.t("bench-no-routes"));
        }
        bench::router_jobs(&spec, base, &routes)
    } else {
        let mut models = strings(&opts["models"]);
        if models.is_empty() {
            models.clone_from(&spec.models);
        }
        if models.is_empty() {
            return Err(l.t("bench-no-models"));
        }
        // `graph`: "on" | "off" | "both" (kor bench --graph), graf araçları açık/kapalı kolları.
        match opts["graph"].as_str() {
            Some(g @ ("on" | "off" | "both")) => {
                bench::graph_jobs(&spec, &models, g != "on", g != "off")
            }
            _ => bench::race_jobs(&spec, &models),
        }
    };
    let cache = kor_platform::cache_dir().map(|d| d.join("model-catalog.json"));
    let prices = bench::Prices::load(&l, cache.as_deref());
    let estimate = bench::estimate(&jobs, &prices);
    let max_cost = opts["maxCost"]
        .as_f64()
        .filter(|c| c.is_finite() && *c >= 0.0)
        .unwrap_or(bench::DEFAULT_MAX_COST_USD);
    Ok(Plan {
        root,
        loaded,
        l,
        spec,
        jobs,
        prices,
        estimate,
        max_cost,
    })
}

/// Bütçe içindeyse ya da `confirm` verildiyse arka planda başlatır; değilse `needsConfirm` döner.
fn start(opts: &Value, app: &AppHandle, state: &AppState) -> Res {
    let p = plan(opts, state)?;
    if !bench::within_budget(&p.estimate, p.max_cost) && opts["confirm"].as_bool() != Some(true) {
        return Ok(
            json!({ "started": false, "needsConfirm": true, "estimate": p.estimate_json() }),
        );
    }
    let cancel = CancellationToken::new();
    {
        let mut st = run_state();
        if st.cancel.as_ref().is_some_and(|c| !c.is_cancelled()) {
            return Err("Bir bench zaten çalışıyor".into());
        }
        st.cancel = Some(cancel.clone());
    }
    let estimate = p.estimate_json();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let make = bench::runtime_agent(&p.l, &p.loaded);
        let emit = |ev: bench::Progress| {
            let _ = app.emit("kor:bench/progress", &ev);
        };
        let rows = bench::run(
            &p.l, &p.root, &p.spec, &p.jobs, &make, &p.prices, &emit, &cancel,
        )
        .await;
        let report = serde_json::to_value(bench::report(rows)).unwrap_or(Value::Null);
        let done = json!({ "cancelled": cancel.is_cancelled(), "report": report });
        {
            let mut st = run_state();
            st.cancel = None;
            st.last = Some(done.clone());
        }
        let _ = app.emit("kor:bench/done", done);
    });
    Ok(json!({ "started": true, "estimate": estimate }))
}

fn cancel() -> Value {
    let st = run_state();
    let running = st.cancel.as_ref().is_some_and(|c| {
        c.cancel();
        true
    });
    json!({ "cancelled": running })
}

/// Son raporu (`{cancelled, report}`) ve çalışıp çalışmadığını döndürür.
fn result() -> Value {
    let st = run_state();
    json!({ "running": st.cancel.is_some(), "last": st.last })
}
