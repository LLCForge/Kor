//! `kor media` (§20, ADR 0011, ADR 0060): the capability catalog across fal.ai, Venice and
//! Hugging Face (`models --capability`), generation with the missing-parameter card
//! (`generate`), the project gallery with its derivation chain (`gallery`, `show`, `derive`,
//! `export`), resumable jobs (`jobs`, `collect`), inline previews (kitty / sixel, else ASCII and
//! the path), plus any fal model by id (`run`, `image`). The work lives in
//! `kor_providers::{fal, media_catalog}`, `kor_agent::{media, gallery}` and `kor_runtime::media`;
//! this file only talks to the user.

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use kor_agent::gallery;
use kor_agent::media::{CatalogRun, MediaEnv};
use kor_i18n::{Localizer, args};
use kor_providers::fal::{self, Fal};
use kor_providers::media_catalog::{self as catalog, Capability, CatalogModel, Plan};
use kor_runtime::media::{Catalog, Clients};
use serde_json::{Map, Value, json};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum MediaCmd {
    /// The capability catalog, or fal models by text/category, or one fal model's inputs.
    Models {
        query: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        capability: Option<String>,
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Run any fal model with a JSON input.
    Run {
        model: String,
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        hf: bool,
        #[arg(long)]
        no_ask: bool,
    },
    /// Text-to-image with the default (or given) fal model.
    Image {
        #[arg(required = true)]
        prompt: Vec<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        no_ask: bool,
    },
    /// Generate with a catalog model, asking for missing parameters first.
    Generate {
        capability: String,
        prompt: Vec<String>,
        #[command(flatten)]
        opts: GenOpts,
    },
    /// Generated media of this project, newest first.
    Gallery {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// One gallery item: preview, parameters and derivation chain.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// A quick action on a gallery item: variation, reseed, upscale, video, remove-bg, edit.
    Derive {
        id: String,
        action: String,
        prompt: Vec<String>,
        #[command(flatten)]
        opts: GenOpts,
    },
    /// Copy a gallery item into the project (e.g. public/logo.png).
    Export { id: String, dest: String },
    /// Queued requests not collected yet.
    Jobs {
        #[arg(long)]
        json: bool,
    },
    /// Wait for and save a queued request (all when no id).
    Collect { job: Option<String> },
}

/// Options shared by `generate` and `derive`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct GenOpts {
    #[arg(long)]
    model: Option<String>,
    #[arg(long = "param", short = 'p')]
    params: Vec<String>,
    #[arg(long = "ref")]
    reference: Option<String>,
    #[arg(long)]
    no_ask: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    save_as: Option<String>,
    #[arg(long)]
    background: bool,
    /// Set by `derive` and the background child (`--action`; `derive` has its own positional).
    #[arg(long = "action", hide = true)]
    step_action: Option<String>,
    #[arg(long, hide = true)]
    derived_from: Option<String>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    let gen_args = |s: clap::Command| {
        s.mut_arg("model", |a| a.help(l.t("arg-media-model")))
            .mut_arg("params", |a| a.help(l.t("arg-media-param")))
            .mut_arg("reference", |a| a.help(l.t("arg-media-ref")))
            .mut_arg("no_ask", |a| a.help(l.t("arg-media-no-ask")))
            .mut_arg("yes", |a| a.help(l.t("arg-media-yes")))
            .mut_arg("name", |a| a.help(l.t("arg-media-name")))
            .mut_arg("save_as", |a| a.help(l.t("arg-media-save-as")))
            .mut_arg("background", |a| a.help(l.t("arg-media-background")))
    };
    c.about(l.t("cmd-media"))
        .mut_subcommand("models", |s| {
            s.about(l.t("cmd-media-models"))
                .mut_arg("query", |a| a.help(l.t("arg-media-query")))
                .mut_arg("category", |a| a.help(l.t("arg-media-category")))
                .mut_arg("model", |a| a.help(l.t("arg-media-model")))
                .mut_arg("capability", |a| a.help(l.t("arg-media-capability")))
                .mut_arg("refresh", |a| a.help(l.t("arg-media-refresh")))
                .mut_arg("json", |a| a.help(l.t("arg-media-json")))
        })
        .mut_subcommand("run", |s| {
            s.about(l.t("cmd-media-run"))
                .mut_arg("model", |a| a.help(l.t("arg-media-model")))
                .mut_arg("input", |a| a.help(l.t("arg-media-input")))
                .mut_arg("hf", |a| a.help(l.t("arg-media-hf")))
                .mut_arg("no_ask", |a| a.help(l.t("arg-media-no-ask")))
        })
        .mut_subcommand("image", |s| {
            s.about(l.t("cmd-media-image"))
                .mut_arg("prompt", |a| a.help(l.t("arg-media-prompt")))
                .mut_arg("model", |a| a.help(l.t("arg-media-model")))
                .mut_arg("no_ask", |a| a.help(l.t("arg-media-no-ask")))
        })
        .mut_subcommand("generate", |s| {
            gen_args(s.about(l.t("cmd-media-generate")))
                .mut_arg("capability", |a| a.help(l.t("arg-media-capability")))
                .mut_arg("prompt", |a| a.help(l.t("arg-media-prompt")))
        })
        .mut_subcommand("gallery", |s| {
            s.about(l.t("cmd-media-gallery"))
                .mut_arg("json", |a| a.help(l.t("arg-media-json")))
                .mut_arg("limit", |a| a.help(l.t("arg-media-limit")))
        })
        .mut_subcommand("show", |s| {
            s.about(l.t("cmd-media-show"))
                .mut_arg("id", |a| a.help(l.t("arg-media-id")))
                .mut_arg("json", |a| a.help(l.t("arg-media-json")))
        })
        .mut_subcommand("derive", |s| {
            gen_args(s.about(l.t("cmd-media-derive")))
                .mut_arg("id", |a| a.help(l.t("arg-media-id")))
                .mut_arg("action", |a| a.help(l.t("arg-media-action")))
                .mut_arg("prompt", |a| a.help(l.t("arg-media-prompt")))
        })
        .mut_subcommand("export", |s| {
            s.about(l.t("cmd-media-export"))
                .mut_arg("id", |a| a.help(l.t("arg-media-id")))
                .mut_arg("dest", |a| a.help(l.t("arg-media-dest")))
        })
        .mut_subcommand("jobs", |s| {
            s.about(l.t("cmd-media-jobs"))
                .mut_arg("json", |a| a.help(l.t("arg-media-json")))
        })
        .mut_subcommand("collect", |s| {
            s.about(l.t("cmd-media-collect"))
                .mut_arg("job", |a| a.help(l.t("arg-media-job")))
        })
}

pub(crate) fn privacy_blocked(ctx: &Ctx) -> bool {
    let on =
        kor_runtime::privacy_mode() || ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
    if on {
        eprintln!("{}", ctx.l.t("media-privacy"));
    }
    on
}

fn client(ctx: &Ctx, via_hf: bool) -> Option<Fal> {
    if via_hf {
        return match kor_runtime::hf_client(&ctx.l, None).map(|(h, _)| h.fal()) {
            Ok(Ok(f)) => Some(f),
            Ok(Err(e)) => {
                eprintln!("{}", e.message);
                None
            }
            Err(e) => {
                eprintln!("{e}");
                None
            }
        };
    }
    let f = kor_runtime::fal_client(&ctx.l).map(|(f, _)| f);
    if f.is_none() {
        eprintln!("{}", ctx.l.t("media-no-fal-key"));
    }
    f
}

fn media_env(ctx: &Ctx) -> MediaEnv {
    kor_runtime::media::env(ctx.config.as_ref().ok().map(|c| &c.config.media))
}

fn clients(ctx: &Ctx) -> Option<Clients> {
    let c = Clients::load(&ctx.l);
    if c.is_empty() {
        eprintln!("{}", ctx.l.t("media-no-provider"));
        return None;
    }
    Some(c)
}

pub fn run(ctx: &Ctx, cmd: &MediaCmd) -> anyhow::Result<bool> {
    let root = kor_runtime::project_root()?;
    // The gallery is local: listing, showing and copying work in privacy mode too.
    match cmd {
        MediaCmd::Gallery { json, limit } => return Ok(gallery_list(ctx, &root, *json, *limit)),
        MediaCmd::Show { id, json } => return Ok(show(ctx, &root, id, *json)),
        MediaCmd::Export { id, dest } => {
            return Ok(match gallery::export(&root, id, dest) {
                Ok(p) => {
                    println!(
                        "{}",
                        ctx.l
                            .t_args("media-exported", &args!("path" => p.display().to_string()))
                    );
                    true
                }
                Err(e) => {
                    eprintln!("{}", ctx.l.t_args("media-error", &args!("error" => e)));
                    false
                }
            });
        }
        MediaCmd::Jobs { json } => return Ok(jobs(ctx, &root, *json)),
        _ => {}
    }
    if privacy_blocked(ctx) {
        return Ok(false);
    }
    let rt = crate::providers::runtime()?;
    match cmd {
        MediaCmd::Models {
            query,
            category,
            model,
            capability,
            refresh,
            json,
        } => {
            if capability.is_some()
                || *refresh
                || *json
                || (query.is_none() && category.is_none() && model.is_none())
            {
                return rt.block_on(catalog_list(ctx, capability.as_deref(), *refresh, *json));
            }
            let Some(fal) = client(ctx, false) else {
                return Ok(false);
            };
            rt.block_on(models(
                ctx,
                &fal,
                query.as_deref(),
                category.as_deref(),
                model.as_deref(),
            ))
        }
        MediaCmd::Run {
            model,
            input,
            hf,
            no_ask,
        } => {
            let input = match parse_input(input.as_deref()) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{}", ctx.l.t_args("media-bad-input", &args!("error" => e)));
                    return Ok(false);
                }
            };
            let Some(fal) = client(ctx, *hf) else {
                return Ok(false);
            };
            // Schemas and prices come from fal's own API; the HF route has no key for it.
            let schema_src = if *hf {
                kor_runtime::fal_client(&ctx.l).map(|(f, _)| f)
            } else {
                Some(fal.clone())
            };
            let provider = if *hf { "huggingface" } else { "fal" };
            rt.block_on(run_model(
                ctx,
                &fal,
                schema_src.as_ref(),
                model,
                input,
                *no_ask,
                provider,
            ))
        }
        MediaCmd::Image {
            prompt,
            model,
            no_ask,
        } => {
            let Some(fal) = client(ctx, false) else {
                return Ok(false);
            };
            let model = model.as_deref().unwrap_or(fal::DEFAULT_IMAGE_MODEL);
            let input = json!({ "prompt": prompt.join(" ") });
            rt.block_on(run_model(
                ctx,
                &fal,
                Some(&fal),
                model,
                input,
                *no_ask,
                "fal",
            ))
        }
        MediaCmd::Generate {
            capability,
            prompt,
            opts,
        } => {
            let Some(cap) = parse_cap(&ctx.l, capability) else {
                return Ok(false);
            };
            let Some(c) = clients(ctx) else {
                return Ok(false);
            };
            rt.block_on(generate(
                ctx,
                &c,
                &root,
                cap,
                &prompt.join(" "),
                opts.clone(),
                Map::new(),
            ))
        }
        MediaCmd::Derive {
            id,
            action,
            prompt,
            opts,
        } => {
            let Some(c) = clients(ctx) else {
                return Ok(false);
            };
            rt.block_on(derive(
                ctx,
                &c,
                &root,
                id,
                action,
                &prompt.join(" "),
                opts.clone(),
            ))
        }
        MediaCmd::Collect { job } => {
            let Some(c) = clients(ctx) else {
                return Ok(false);
            };
            rt.block_on(collect(ctx, &c, &root, job.as_deref()))
        }
        MediaCmd::Gallery { .. }
        | MediaCmd::Show { .. }
        | MediaCmd::Export { .. }
        | MediaCmd::Jobs { .. } => Ok(true),
    }
}

fn parse_cap(l: &Localizer, s: &str) -> Option<Capability> {
    let c = Capability::parse(s);
    if c.is_none() {
        let list: Vec<&str> = Capability::ALL.iter().map(|c| c.key()).collect();
        eprintln!(
            "{}",
            l.t_args(
                "media-unknown-capability",
                &args!("value" => s, "list" => list.join(", "))
            )
        );
    }
    c
}

/// `{...}` JSON, or `@path` to a JSON file.
fn parse_input(s: Option<&str>) -> Result<Value, String> {
    let Some(s) = s else {
        return Ok(json!({}));
    };
    let text = match s.strip_prefix('@') {
        Some(p) => std::fs::read_to_string(p).map_err(|e| e.to_string())?,
        None => s.to_owned(),
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if v.is_object() {
        Ok(v)
    } else {
        Err("the input must be a JSON object".into())
    }
}

/// `--param name=value`: the value as JSON when it is a number, boolean or array, else text.
fn parse_params(raw: &[String]) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for p in raw {
        let (k, v) = p
            .split_once('=')
            .filter(|(k, _)| !k.trim().is_empty())
            .ok_or_else(|| p.clone())?;
        let v = v.trim();
        let value = serde_json::from_str::<Value>(v)
            .ok()
            .filter(|x| !x.is_string() && !x.is_object())
            .unwrap_or_else(|| json!(v));
        out.insert(k.trim().to_owned(), value);
    }
    Ok(out)
}

fn text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}

// ---------------------------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------------------------

fn price_text(m: &CatalogModel) -> String {
    m.price.as_ref().map_or_else(
        || "?".to_owned(),
        |p| {
            if p.currency == "USD" {
                format!("${}/{}", p.amount, p.unit)
            } else {
                format!("{} {}/{}", p.amount, p.currency, p.unit)
            }
        },
    )
}

/// `prompt* image_size[square_hd|…]=landscape_4_3 num_images(1–4)=1 …`
fn params_text(m: &CatalogModel) -> String {
    m.params
        .iter()
        .map(|p| {
            let star = if p.required { "*" } else { "" };
            let opts = if p.options.is_empty() {
                match (p.min, p.max) {
                    (Some(a), Some(b)) => format!("({a}–{b})"),
                    _ => String::new(),
                }
            } else {
                let o: Vec<String> = p.options.iter().take(6).map(text).collect();
                let more = if p.options.len() > 6 { "|…" } else { "" };
                format!("[{}{more}]", o.join("|"))
            };
            let d = p
                .default
                .as_ref()
                .map(|d| format!("={}", text(d)))
                .unwrap_or_default();
            format!("{}{star}{opts}{d}", p.name)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn catalog_list(
    ctx: &Ctx,
    cap: Option<&str>,
    refresh: bool,
    as_json: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let cap = match cap {
        Some(s) => match parse_cap(l, s) {
            Some(c) => Some(c),
            None => return Ok(false),
        },
        None => None,
    };
    let Some(c) = clients(ctx) else {
        return Ok(false);
    };
    let cat = kor_runtime::media::catalog(&c, refresh).await;
    let list = cat.with(cap);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(true);
    }
    let cap_name = cap.map_or_else(|| "*".to_owned(), |c| c.key().to_owned());
    if list.is_empty() {
        println!(
            "{}",
            l.t_args("media-catalog-none", &args!("capability" => cap_name))
        );
    } else {
        let date = chrono::DateTime::from_timestamp_millis(i64::try_from(cat.fetched).unwrap_or(0))
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        println!(
            "{}",
            l.t_args(
                "media-catalog-header",
                &args!("count" => list.len(), "capability" => cap_name, "date" => date)
            )
        );
        for m in &list {
            let caps: Vec<&str> = m.capabilities.iter().map(|c| c.key()).collect();
            println!(
                "  {:<56} {:>18}  {}",
                m.reference(),
                price_text(m),
                caps.join(",")
            );
            println!("      {}", params_text(m));
            println!("      {}", m.source);
        }
    }
    if !cat.errors.is_empty() {
        eprintln!(
            "{}",
            l.t_args(
                "media-catalog-errors",
                &args!("errors" => cat.errors.join("; "))
            )
        );
    }
    Ok(true)
}

async fn models(
    ctx: &Ctx,
    fal: &Fal,
    query: Option<&str>,
    category: Option<&str>,
    model: Option<&str>,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if let Some(m) = model {
        let schema = match fal.input_schema(m).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", l.t_args("media-error", &args!("error" => e.message)));
                return Ok(false);
            }
        };
        println!("{}", l.t_args("media-inputs", &args!("model" => m)));
        for p in catalog::params_from_schema(&schema) {
            let star = if p.required { "*" } else { " " };
            let en = if p.options.is_empty() {
                String::new()
            } else {
                format!(" {}", Value::Array(p.options.clone()))
            };
            let range = match (p.min, p.max) {
                (Some(a), Some(b)) => format!(" ({a}–{b})"),
                _ => String::new(),
            };
            let de = p
                .default
                .as_ref()
                .map(|d| format!(" = {d}"))
                .unwrap_or_default();
            println!("  {star} {} ({:?}){en}{range}{de}", p.name, p.kind);
        }
        if let Ok(Some(p)) = fal.price(m).await {
            println!(
                "{}",
                l.t_args(
                    "media-price",
                    &args!("price" => p.unit_price, "currency" => p.currency, "unit" => p.unit)
                )
            );
        }
        return Ok(true);
    }
    match fal.models(query, category, 40).await {
        Ok(list) if list.is_empty() => {
            println!("{}", l.t("media-no-models"));
            Ok(true)
        }
        Ok(list) => {
            for m in list {
                println!("{:<48} {:<16} {}", m.endpoint_id, m.category, m.name);
            }
            Ok(true)
        }
        Err(e) => {
            eprintln!("{}", l.t_args("media-error", &args!("error" => e.message)));
            Ok(false)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Asking (§20.2)
// ---------------------------------------------------------------------------------------------

fn read_answer() -> Option<String> {
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok()?;
    Some(line.trim().to_owned())
}

/// The answer to one field: Enter = default, `n` = the n-th option, else a value the parameter
/// accepts. `None`: no valid answer.
fn answer_value(p: &catalog::Param, default: Option<&Value>, answer: &str) -> Option<Value> {
    if answer.is_empty() {
        return default.cloned();
    }
    if !p.options.iter().any(|o| text(o) == answer)
        && let Ok(n) = answer.parse::<usize>()
        && (1..=p.options.len()).contains(&n)
    {
        return p.options.get(n - 1).cloned();
    }
    let v = catalog::coerce(p, &json!(answer));
    catalog::check(p, &v).is_ok().then_some(v)
}

/// Ask every open choice at once (one line each), the defaults marked; `None` = aborted.
fn ask_plan(ctx: &Ctx, model: &CatalogModel, plan: &Plan) -> Option<Map<String, Value>> {
    let l = &ctx.l;
    let mut fields: Vec<(catalog::Param, Option<Value>)> = plan
        .questions
        .iter()
        .filter_map(|q| Some((model.param(&q.name)?.clone(), q.default.clone())))
        .collect();
    for u in &plan.unsupported {
        if let Some(p) = model.param(&u.name) {
            fields.push((p.clone(), u.nearest.clone().or_else(|| p.default.clone())));
        }
    }
    let mut out = Map::new();
    if fields.is_empty() {
        return Some(out);
    }
    if !std::io::stdin().is_terminal() {
        let missing: Vec<&str> = fields
            .iter()
            .filter(|(_, d)| d.is_none())
            .map(|(p, _)| p.name.as_str())
            .collect();
        if !missing.is_empty() {
            eprintln!(
                "{}",
                l.t_args("media-missing", &args!("names" => missing.join(", ")))
            );
            return None;
        }
        return Some(
            fields
                .into_iter()
                .filter_map(|(p, d)| Some((p.name, d?)))
                .collect(),
        );
    }
    eprintln!("{}", l.t("media-card-hint"));
    for (p, default) in &fields {
        let opts = if p.options.is_empty() {
            match (p.min, p.max) {
                (Some(a), Some(b)) => format!(" ({a}–{b})"),
                _ => String::new(),
            }
        } else {
            let o: Vec<String> = p
                .options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let mark = if Some(o) == default.as_ref() { "*" } else { "" };
                    format!("{}:{}{mark}", i + 1, text(o))
                })
                .collect();
            format!(" [{}]", o.join(" | "))
        };
        let d = default
            .as_ref()
            .map(|d| format!(" <{}>", text(d)))
            .unwrap_or_default();
        let mut value = None;
        for _ in 0..3 {
            eprint!("  {}{opts}{d}: ", p.name);
            let a = read_answer()?;
            value = answer_value(p, default.as_ref(), &a);
            if value.is_some() {
                break;
            }
            eprintln!("{}", l.t_args("media-not-an-option", &args!("value" => a)));
        }
        out.insert(p.name.clone(), value?);
    }
    Some(out)
}

/// The model to use: `--model`, `[media.defaults.<cap>] model`, else a choice from the catalog
/// (cheapest first; without a terminal the cheapest).
async fn pick_model(
    ctx: &Ctx,
    c: &Clients,
    cat: &Catalog,
    cap: Capability,
    wanted: Option<String>,
) -> Option<CatalogModel> {
    let l = &ctx.l;
    if let Some(w) = wanted {
        if let Some(m) = cat.find(&w) {
            return Some(m.clone());
        }
        // Any fal endpoint works, also one outside the catalog's top list.
        let id = w.strip_prefix("fal:").unwrap_or(&w);
        if let Some(f) = &c.fal
            && (w.starts_with("fal:") || !w.contains(':'))
            && let Ok(schema) = f.input_schema(id).await
        {
            let price = f.price(id).await.ok().flatten();
            return Some(catalog::fal_model(id, cap, Some(&schema), price.as_ref()));
        }
        eprintln!(
            "{}",
            l.t_args(
                "media-unknown-model",
                &args!("model" => w, "capability" => cap.key())
            )
        );
        return None;
    }
    let list = cat.with(Some(cap));
    if list.is_empty() {
        eprintln!(
            "{}",
            l.t_args("media-catalog-none", &args!("capability" => cap.key()))
        );
        return None;
    }
    if !std::io::stdin().is_terminal() {
        return list.first().map(|m| (*m).clone());
    }
    for (i, m) in list.iter().take(9).enumerate() {
        eprintln!("  {} {:<56} {}", i + 1, m.reference(), price_text(m));
    }
    eprint!("{} ", l.t("media-choose-model"));
    let a = read_answer()?;
    let i = a
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=list.len().min(9)).contains(n))
        .unwrap_or(1);
    list.get(i - 1).map(|m| (*m).clone())
}

/// Plan, show unsupported values with the models that can do them, ask, confirm the cost.
/// Returns the model (possibly switched) and its final inputs.
#[allow(clippy::too_many_arguments)] // the request, its context and the user's options
async fn plan_and_ask(
    ctx: &Ctx,
    cat: &Catalog,
    cap: Capability,
    mut model: CatalogModel,
    prompt: &str,
    given: &Map<String, Value>,
    env: &MediaEnv,
    opts: &GenOpts,
) -> Option<(CatalogModel, Map<String, Value>)> {
    let l = &ctx.l;
    let ask = !opts.no_ask && !env.no_ask;
    let defaults = kor_runtime::media::defaults_for(env, cap);
    let root = kor_runtime::project_root().ok()?;
    for _ in 0..3 {
        let plan = catalog::plan(&model, prompt, given, &defaults, ask);
        eprintln!("{}: {prompt}", model.reference());
        if let Some(e) = &plan.estimate {
            eprintln!(
                "  {}",
                l.t_args("media-card-cost", &args!("cost" => e.display()))
            );
        }
        if let Some(s) = gallery::eta(&root, &model.id) {
            eprintln!("  {}", l.t_args("media-card-eta", &args!("secs" => s)));
        }
        let mut switched = false;
        for u in &plan.unsupported {
            let near = u.nearest.as_ref().map(text).unwrap_or_else(|| "-".into());
            eprintln!(
                "  {}",
                l.t_args(
                    "media-unsupported",
                    &args!("name" => u.name.clone(), "value" => text(&u.asked), "nearest" => near, "allowed" => u.allowed.clone())
                )
            );
            let alts = catalog::alternatives(
                &cat.models,
                cap,
                &model,
                catalog::role_of(&model, &u.name),
                &u.asked,
                prompt,
            );
            if alts.is_empty() || !std::io::stdin().is_terminal() || opts.no_ask {
                continue;
            }
            eprintln!(
                "  {}",
                l.t_args("media-switch", &args!("model" => model.id.clone()))
            );
            for (i, (m, e)) in alts.iter().enumerate() {
                let cost = e
                    .as_ref()
                    .map_or_else(|| "?".to_owned(), catalog::Estimate::display);
                eprintln!("    {} {:<56} {cost}", i + 1, m.reference());
            }
            eprint!("  > ");
            let a = read_answer()?;
            if let Some(m) = a
                .parse::<usize>()
                .ok()
                .and_then(|n| alts.get(n.wrapping_sub(1)))
            {
                model = m.0.clone();
                switched = true;
                break;
            }
        }
        if switched {
            continue;
        }
        let answers = ask_plan(ctx, &model, &plan)?;
        let mut input = plan.input.clone();
        input.extend(answers);
        let estimate = catalog::estimate(&model, prompt, &input);
        if let Some(e) = &estimate
            && env.confirm_above > 0.0
            && e.currency == "USD"
            && e.amount > env.confirm_above
            && !opts.yes
        {
            if !std::io::stdin().is_terminal() {
                eprintln!(
                    "{}",
                    l.t_args("media-confirm-needed", &args!("cost" => e.display()))
                );
                return None;
            }
            eprint!(
                "{} ",
                l.t_args("media-confirm", &args!("cost" => e.display()))
            );
            let a = read_answer()?.to_lowercase();
            if !matches!(a.as_str(), "y" | "yes" | "e" | "evet" | "o" | "oui") {
                eprintln!("{}", l.t("media-declined"));
                return None;
            }
        }
        return Some((model, input));
    }
    None
}

/// A reference given on the command line: `clipboard` (saved into the gallery), a gallery id or
/// a project path.
fn reference(ctx: &Ctx, root: &Path, r: &str) -> Option<String> {
    if r == "clipboard" {
        let pic = match kor_runtime::visual::clipboard_image() {
            Ok(Some(p)) => p,
            Ok(None) => {
                eprintln!("{}", ctx.l.t("media-clipboard-empty"));
                return None;
            }
            Err(e) => {
                eprintln!("{}", ctx.l.t_args("media-error", &args!("error" => e)));
                return None;
            }
        };
        let ext = pic.media_type.rsplit('/').next().unwrap_or("png");
        let dir = gallery::dir(root);
        let name = format!("clipboard-{}.{ext}", kor_runtime::cost::now_ms());
        let saved = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(&name), &pic.bytes));
        if let Err(e) = saved {
            eprintln!(
                "{}",
                ctx.l
                    .t_args("media-error", &args!("error" => e.to_string()))
            );
            return None;
        }
        return Some(name);
    }
    match gallery::resolve(root, r) {
        Ok(_) => Some(r.to_owned()),
        Err(e) => {
            eprintln!("{}", ctx.l.t_args("media-error", &args!("error" => e)));
            None
        }
    }
}

#[allow(clippy::too_many_lines)] // check, pick, plan and ask, background or run, preview: one flow
async fn generate(
    ctx: &Ctx,
    c: &Clients,
    root: &Path,
    cap: Capability,
    prompt: &str,
    opts: GenOpts,
    preset: Map<String, Value>,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let env = media_env(ctx);
    let mut given = preset;
    match parse_params(&opts.params) {
        Ok(p) => given.extend(p),
        Err(bad) => {
            eprintln!(
                "{}",
                l.t_args("media-invalid-param", &args!("value" => bad))
            );
            return Ok(false);
        }
    }
    let needs_prompt = !matches!(
        cap,
        Capability::Upscale | Capability::RemoveBg | Capability::Asr
    );
    if needs_prompt && prompt.trim().is_empty() {
        eprintln!(
            "{}",
            l.t_args("media-prompt-required", &args!("capability" => cap.key()))
        );
        return Ok(false);
    }
    let reference_id = match opts.reference.as_deref() {
        Some(r) => match reference(ctx, root, r) {
            Some(id) => Some(id),
            None => return Ok(false),
        },
        None => None,
    };
    if cap.needs_input() && reference_id.is_none() {
        eprintln!(
            "{}",
            l.t_args("media-ref-required", &args!("capability" => cap.key()))
        );
        return Ok(false);
    }
    let cat = kor_runtime::media::catalog(c, false).await;
    let wanted = opts
        .model
        .clone()
        .or_else(|| kor_runtime::media::default_model(&env, cap));
    let Some(model) = pick_model(ctx, c, &cat, cap, wanted).await else {
        return Ok(false);
    };
    let Some((model, params)) =
        plan_and_ask(ctx, &cat, cap, model, prompt, &given, &env, &opts).await
    else {
        return Ok(false);
    };
    if opts.background {
        return Ok(background(
            ctx,
            root,
            cap,
            prompt,
            &model,
            &params,
            reference_id.as_deref(),
            &opts,
        ));
    }
    let run = CatalogRun {
        capability: Some(cap),
        prompt: prompt.to_owned(),
        params,
        reference: reference_id,
        action: opts.step_action.clone(),
        name: opts.name.clone(),
        save_as: opts.save_as.clone(),
    };
    let tools = kor_runtime::media::tools(c, &env);
    let tctx = kor_tools::ToolCtx::new(root)?;
    let cancel = tctx.cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel.cancel();
        }
    });
    let prefix = l.t("media-status");
    let sink: kor_tools::ProgressSink = std::sync::Arc::new(move |line: String| {
        eprintln!("{prefix} {line}");
    });
    let mut out = kor_tools::with_progress(sink, async {
        let (name, mut input) = match kor_agent::media::catalog_call(&model, &run) {
            Ok(x) => x,
            Err(e) => return kor_tools::ToolOutput::err(e),
        };
        // Not an input file, only the gallery link (variation, same seed).
        if let Some(d) = &opts.derived_from {
            input["derived_from"] = json!(d);
        }
        match tools.iter().find(|t| t.name() == name) {
            Some(t) => t.call(input, &tctx).await,
            None => {
                kor_tools::ToolOutput::err(format!("{name}: {} is not configured", model.provider))
            }
        }
    })
    .await;
    let background_child = std::env::var_os("KOR_MEDIA_BACKGROUND").is_some();
    if out.is_error {
        eprintln!(
            "{}",
            l.t_args("media-error", &args!("error" => out.content.clone()))
        );
        if background_child {
            kor_runtime::media::notify(false, &l.t("media-failed-title"), &out.content, None);
        }
        return Ok(false);
    }
    out.content = out.content.trim_end().to_owned();
    println!("{}", out.content);
    for p in saved_paths(&out.content) {
        preview(ctx, &p);
    }
    crate::status_cmd::call_me_done();
    if background_child {
        kor_runtime::media::notify(
            true,
            &l.t("media-done-title"),
            &format!("{}: {prompt}", model.reference()),
            None,
        );
    }
    Ok(true)
}

fn saved_paths(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .filter_map(|l| l.strip_prefix("saved: "))
        .map(|p| PathBuf::from(p.trim()))
        .collect()
}

/// Start the planned generation as a detached `kor media generate` (§20.4: the job goes on when
/// this terminal closes); it reports to the notification center when done.
#[allow(clippy::too_many_arguments)] // the planned call, as the child's command line
fn background(
    ctx: &Ctx,
    root: &Path,
    cap: Capability,
    prompt: &str,
    model: &CatalogModel,
    params: &Map<String, Value>,
    reference_id: Option<&str>,
    opts: &GenOpts,
) -> bool {
    let l = &ctx.l;
    let mut a: Vec<String> = vec!["media".into(), "generate".into(), cap.key().into()];
    if !prompt.is_empty() {
        a.push(prompt.to_owned());
    }
    a.extend([
        "--model".into(),
        model.reference(),
        "--no-ask".into(),
        "--yes".into(),
    ]);
    for (k, v) in params {
        a.extend(["--param".into(), format!("{k}={}", text(v))]);
    }
    for (flag, v) in [
        ("--ref", reference_id.map(str::to_owned)),
        ("--name", opts.name.clone()),
        ("--save-as", opts.save_as.clone()),
        ("--action", opts.step_action.clone()),
        ("--derived-from", opts.derived_from.clone()),
    ] {
        if let Some(v) = v {
            a.extend([flag.to_owned(), v]);
        }
    }
    let dir = gallery::dir(root).join(".jobs");
    let log = dir.join(format!("bg-{}.log", kor_runtime::cost::now_ms()));
    let spawned = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::File::create(&log))
        .and_then(|f| {
            let f2 = f.try_clone()?;
            std::process::Command::new(std::env::current_exe()?)
                .args(&a)
                .current_dir(root)
                .env("KOR_MEDIA_BACKGROUND", "1")
                .stdin(std::process::Stdio::null())
                .stdout(f)
                .stderr(f2)
                .spawn()
        });
    match spawned {
        Ok(_) => {
            println!(
                "{}",
                l.t_args(
                    "media-background",
                    &args!("log" => log.display().to_string())
                )
            );
            true
        }
        Err(e) => {
            eprintln!(
                "{}",
                l.t_args("media-error", &args!("error" => e.to_string()))
            );
            false
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Gallery (§20.3) and preview (§20.4)
// ---------------------------------------------------------------------------------------------

/// Inline in kitty / sixel terminals (`tui.images`), else an ASCII sketch; always the path.
fn preview(ctx: &Ctx, path: &Path) {
    let setting = ctx
        .config
        .as_ref()
        .map_or(kor_config::ImageProtocol::Auto, |c| c.config.tui.images);
    if gallery::kind_of(path) == "image" {
        let shown = crate::tui::term::show_image(path, crate::tui::term::image_protocol(setting));
        if !shown
            && std::io::stdout().is_terminal()
            && let Some(art) = ascii_preview(path, 48)
        {
            println!("{art}");
        }
    }
    println!(
        "{}",
        ctx.l.t_args(
            "media-preview-path",
            &args!("path" => path.display().to_string())
        )
    );
}

/// A small grey-ramp sketch of an image for terminals without inline images.
fn ascii_preview(path: &Path, width: u32) -> Option<String> {
    const RAMP: &[u8] = b" .:-=+*#%@";
    let img = image::open(path).ok()?;
    let (w, h) = (img.width().max(1), img.height().max(1));
    // Terminal cells are about twice as tall as wide.
    let rows = (u64::from(h) * u64::from(width) / u64::from(w) / 2).clamp(1, 64);
    let small = img
        .resize_exact(
            width,
            u32::try_from(rows).ok()?,
            image::imageops::FilterType::Triangle,
        )
        .to_luma8();
    let mut out = String::new();
    for y in 0..small.height() {
        for x in 0..small.width() {
            let v = usize::from(small.get_pixel(x, y)[0]);
            out.push(char::from(RAMP[v * (RAMP.len() - 1) / 255]));
        }
        out.push('\n');
    }
    Some(out.trim_end().to_owned())
}

fn item_line(i: &gallery::Item) -> String {
    let cost = i
        .cost
        .map_or_else(String::new, |c| kor_runtime::cost::fmt_usd(c, i.estimated));
    let from = i
        .parent
        .as_deref()
        .map_or_else(String::new, |p| format!(" ← {p}"));
    let action = i
        .action
        .as_deref()
        .map_or_else(String::new, |a| format!(" [{a}]"));
    let prompt: String = i.prompt.chars().take(60).collect();
    format!(
        "{:<44} {:<6} {:<36} {cost:>8}{action}{from}  {prompt}",
        i.id,
        i.kind,
        format!("{}:{}", i.provider, i.model)
    )
}

fn gallery_list(ctx: &Ctx, root: &Path, as_json: bool, limit: usize) -> bool {
    let items = gallery::list(root);
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&items).unwrap_or_default()
        );
        return true;
    }
    if items.is_empty() {
        println!("{}", ctx.l.t("media-gallery-empty"));
        return true;
    }
    println!(
        "{}",
        ctx.l.t_args(
            "media-gallery-header",
            &args!("count" => items.len(), "dir" => gallery::dir(root).display().to_string())
        )
    );
    for i in items.iter().take(limit) {
        println!("  {}", item_line(i));
    }
    true
}

fn show(ctx: &Ctx, root: &Path, id: &str, as_json: bool) -> bool {
    let l = &ctx.l;
    let chain = gallery::chain(root, id);
    let Some(item) = chain.first() else {
        eprintln!("{}", l.t_args("media-not-found", &args!("id" => id)));
        return false;
    };
    let children = gallery::children(root, id);
    if as_json {
        let v = json!({ "item": item, "chain": &chain[1..], "children": children });
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return true;
    }
    preview(ctx, &root.join(&item.file));
    println!("{}", item_line(item));
    if !item.prompt.is_empty() {
        println!("  {}", item.prompt);
    }
    if item.params.as_object().is_some_and(|o| !o.is_empty()) {
        println!("  {}", item.params);
    }
    if let Some(s) = item.seed {
        println!("  seed {s}");
    }
    if chain.len() > 1 {
        println!("{}", l.t("media-chain"));
        for i in &chain[1..] {
            println!("  {}", item_line(i));
        }
    }
    if !children.is_empty() {
        println!("{}", l.t("media-children"));
        for i in &children {
            println!("  {}", item_line(i));
        }
    }
    true
}

/// What a quick action does with a gallery item.
#[derive(Debug, PartialEq)]
struct Derived {
    cap: Capability,
    /// Sent as the input file (edit, upscale, video, background removal).
    reference: bool,
    /// The item's model and parameters are kept (variation, same seed).
    same_model: bool,
    keep_seed: bool,
    /// A new prompt is required.
    new_prompt: bool,
}

fn derived(action: &str, item_cap: Option<Capability>) -> Option<Derived> {
    let d = |cap, reference, same_model, keep_seed, new_prompt| Derived {
        cap,
        reference,
        same_model,
        keep_seed,
        new_prompt,
    };
    let own = item_cap.unwrap_or(Capability::Image);
    Some(match action {
        "variation" => d(own, false, true, false, false),
        "reseed" => d(own, false, true, true, true),
        "upscale" => d(Capability::Upscale, true, false, false, false),
        "video" => d(Capability::ImageToVideo, true, false, false, false),
        "remove-bg" => d(Capability::RemoveBg, true, false, false, false),
        "edit" => d(Capability::Edit, true, false, false, true),
        _ => return None,
    })
}

async fn derive(
    ctx: &Ctx,
    c: &Clients,
    root: &Path,
    id: &str,
    action: &str,
    prompt: &str,
    mut opts: GenOpts,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(item) = gallery::get(root, id) else {
        eprintln!("{}", l.t_args("media-not-found", &args!("id" => id)));
        return Ok(false);
    };
    let item_cap = item.capability.as_deref().and_then(Capability::parse);
    let Some(d) = derived(action, item_cap) else {
        eprintln!(
            "{}",
            l.t_args("media-unknown-action", &args!("value" => action))
        );
        return Ok(false);
    };
    if d.new_prompt && prompt.trim().is_empty() {
        eprintln!(
            "{}",
            l.t_args("media-new-prompt-required", &args!("action" => action))
        );
        return Ok(false);
    }
    let prompt = if prompt.trim().is_empty() {
        item.prompt.clone()
    } else {
        prompt.to_owned()
    };
    let mut preset = Map::new();
    if d.same_model {
        if opts.model.is_none() && !item.model.is_empty() {
            opts.model = Some(format!("{}:{}", item.provider, item.model));
        }
        if let Some(o) = item.params.as_object() {
            for (k, v) in o {
                let skip = matches!(
                    k.as_str(),
                    "prompt"
                        | "text"
                        | "model"
                        | "name"
                        | "ask"
                        | "capability"
                        | "action"
                        | "save_as"
                        | "derived_from"
                        | "provider"
                        | "input"
                        | "image"
                ) || k.ends_with("_url")
                    || k.ends_with("_urls")
                    || (k == "seed" && !d.keep_seed);
                if !skip {
                    preset.insert(k.clone(), v.clone());
                }
            }
            // Hugging Face / fal tools keep their model inputs under `input`.
            if let Some(inner) = o.get("input").and_then(Value::as_object) {
                for (k, v) in inner {
                    if !(k.ends_with("_url") || k == "prompt" || (k == "seed" && !d.keep_seed)) {
                        preset.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        if d.keep_seed
            && let Some(s) = item.seed
        {
            preset.insert("seed".into(), json!(s));
        }
        opts.derived_from = Some(item.id.clone());
    }
    if d.reference {
        opts.reference = Some(item.id.clone());
    }
    opts.step_action = Some(action.to_owned());
    generate(ctx, c, root, d.cap, &prompt, opts, preset).await
}

// ---------------------------------------------------------------------------------------------
// Jobs (§20.4)
// ---------------------------------------------------------------------------------------------

fn jobs(ctx: &Ctx, root: &Path, as_json: bool) -> bool {
    let list = gallery::jobs(root);
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&list).unwrap_or_default()
        );
        return true;
    }
    if list.is_empty() {
        println!("{}", ctx.l.t("media-jobs-none"));
        return true;
    }
    let now = kor_runtime::cost::now_ms();
    for j in &list {
        let mins = now.saturating_sub(j.started) / 60_000;
        let prompt: String = j.prompt.chars().take(60).collect();
        println!(
            "  {:<40} {:<9} {:<36} {mins:>4} min  {prompt}",
            j.id, j.provider, j.model
        );
    }
    true
}

async fn collect(ctx: &Ctx, c: &Clients, root: &Path, id: Option<&str>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let list: Vec<gallery::Job> = gallery::jobs(root)
        .into_iter()
        .filter(|j| id.is_none_or(|i| i == j.id))
        .collect();
    if list.is_empty() {
        println!("{}", l.t("media-jobs-none"));
        return Ok(id.is_none());
    }
    let env = media_env(ctx);
    let cancel = kor_providers::CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            c2.cancel();
        }
    });
    let prefix = l.t("media-status");
    let mut ok = true;
    for j in &list {
        eprintln!(
            "{}",
            l.t_args(
                "media-collecting",
                &args!("id" => j.id.clone(), "model" => j.model.clone())
            )
        );
        let fal = c.fal_for(&j.provider);
        let b = kor_agent::media::Backends {
            fal: fal.as_ref(),
            venice: c.venice.as_ref(),
        };
        let on = |s: &str| eprintln!("{prefix} {s}");
        match kor_agent::media::collect(j, b, root, env.ledger.as_deref(), &cancel, &on).await {
            Ok(s) => {
                print!("{}", s.report(&j.model));
                for p in &s.paths {
                    preview(ctx, p);
                }
                kor_runtime::media::notify(
                    true,
                    &l.t("media-done-title"),
                    &format!("{}: {}", j.model, j.prompt),
                    j.cost.as_ref(),
                );
            }
            Err(e) => {
                ok = false;
                eprintln!("{}", l.t_args("media-error", &args!("error" => e)));
            }
        }
    }
    Ok(ok)
}

// ---------------------------------------------------------------------------------------------
// `run` / `image`: any fal model by id
// ---------------------------------------------------------------------------------------------

/// Ask on the terminal for each missing input (Enter takes the default); `None` = aborted.
fn ask(ctx: &Ctx, missing: &[fal::MissingInput], input: &mut Value) -> Option<()> {
    if !std::io::stdin().is_terminal() {
        let names: Vec<&str> = missing.iter().map(|m| m.name.as_str()).collect();
        eprintln!(
            "{}",
            ctx.l
                .t_args("media-missing", &args!("names" => names.join(", ")))
        );
        return None;
    }
    for m in missing {
        let opts: Vec<String> = m.options.iter().map(text).collect();
        let default = m.default.as_ref().map(text);
        eprint!(
            "{} {}{}{}: ",
            m.name,
            if m.description.is_empty() {
                String::new()
            } else {
                format!("({}) ", m.description)
            },
            if opts.is_empty() {
                String::new()
            } else {
                format!("[{}] ", opts.join(" | "))
            },
            default
                .as_deref()
                .map(|d| format!("<{d}>"))
                .unwrap_or_default()
        );
        let answer = read_answer()?;
        let value = if answer.is_empty() {
            let Some(d) = m.default.clone() else {
                eprintln!(
                    "{}",
                    ctx.l
                        .t_args("media-missing", &args!("names" => m.name.clone()))
                );
                return None;
            };
            d
        } else if !opts.is_empty() && !opts.iter().any(|o| *o == answer) {
            eprintln!(
                "{}",
                ctx.l
                    .t_args("media-not-an-option", &args!("value" => answer))
            );
            return None;
        } else {
            // Keep the schema's type: an option that is a number stays a number.
            m.options
                .iter()
                .find(|o| text(o) == answer)
                .cloned()
                .or_else(|| {
                    serde_json::from_str::<Value>(&answer)
                        .ok()
                        .filter(|v| !v.is_string())
                })
                .unwrap_or_else(|| json!(answer))
        };
        input[m.name.as_str()] = value;
    }
    Some(())
}

async fn run_model(
    ctx: &Ctx,
    fal: &Fal,
    schema_src: Option<&Fal>,
    model: &str,
    mut input: Value,
    no_ask: bool,
    provider: &str,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let mut cost = None;
    if let Some(src) = schema_src
        && let Ok(schema) = src.input_schema(model).await
    {
        let mut missing = fal::missing_inputs(&schema, &input);
        if no_ask {
            missing.retain(|m| m.default.is_none());
        }
        if !missing.is_empty() && ask(ctx, &missing, &mut input).is_none() {
            return Ok(false);
        }
        if let Ok(Some(p)) = src.price(model).await {
            eprintln!(
                "{}",
                l.t_args(
                    "media-price",
                    &args!("price" => p.unit_price, "currency" => p.currency.clone(), "unit" => p.unit.clone())
                )
            );
            let cm = catalog::fal_model(model, Capability::Image, Some(&schema), Some(&p));
            let prompt = input
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            cost = input
                .as_object()
                .and_then(|o| catalog::estimate(&cm, prompt, o))
                .map(|e| (e.amount, e.currency, e.estimated));
        }
    }
    let root = kor_runtime::project_root()?;
    let cancel = kor_providers::CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            c2.cancel();
        }
    });
    let last = std::sync::Mutex::new(String::new());
    let prefix = l.t("media-status");
    let on_status = |s: &fal::QueueStatus| {
        let line = fal::status_line(s);
        if let Ok(mut prev) = last.lock()
            && *prev != line
        {
            eprintln!("{prefix} {line}");
            *prev = line;
        }
    };
    let prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let stem = kor_agent::media::stem(None, if prompt.is_empty() { model } else { &prompt });
    let env = media_env(ctx);
    let save_to = kor_agent::media::FalSave {
        root: &root,
        stem: &stem,
        meta: gallery::Meta {
            provider: provider.to_owned(),
            model: model.to_owned(),
            prompt,
            params: input.clone(),
            cost,
            ..gallery::Meta::default()
        },
        ledger: env.ledger.as_deref(),
    };
    match kor_agent::media::fal_run_save(fal, model, &input, &save_to, &cancel, &on_status).await {
        Ok(saved) => {
            print!("{}", saved.report(model));
            for p in &saved.paths {
                preview(ctx, p);
            }
            crate::status_cmd::call_me_done();
            Ok(true)
        }
        Err(e) => {
            eprintln!("{}", l.t_args("media-error", &args!("error" => e)));
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Debug, Parser)]
    struct T {
        #[command(subcommand)]
        c: MediaCmd,
    }

    #[test]
    fn input_must_be_a_json_object() {
        assert_eq!(parse_input(None).unwrap(), json!({}));
        assert_eq!(parse_input(Some(r#"{"a":1}"#)).unwrap()["a"], 1);
        assert!(parse_input(Some("[1]")).is_err());
        assert!(parse_input(Some("{")).is_err());
    }

    #[test]
    fn parses_and_localizes() {
        let t = T::try_parse_from([
            "t",
            "generate",
            "video",
            "a",
            "cat",
            "--model",
            "fal:x",
            "-p",
            "duration=5s",
            "--param",
            "seed=3",
            "--ref",
            "cat.png",
            "--no-ask",
            "--yes",
        ])
        .unwrap();
        let MediaCmd::Generate {
            capability,
            prompt,
            opts,
        } = t.c
        else {
            panic!("generate")
        };
        assert_eq!((capability.as_str(), prompt.len()), ("video", 2));
        assert_eq!(opts.reference.as_deref(), Some("cat.png"));
        let p = parse_params(&opts.params).unwrap();
        assert_eq!(
            (p["duration"].clone(), p["seed"].clone()),
            (json!("5s"), json!(3))
        );
        assert!(parse_params(&["=x".into()]).is_err());
        assert!(matches!(
            T::try_parse_from(["t", "models", "--capability", "upscale"])
                .unwrap()
                .c,
            MediaCmd::Models {
                capability: Some(_),
                ..
            }
        ));
        assert!(T::try_parse_from(["t", "derive", "a.png", "upscale"]).is_ok());
        assert!(T::try_parse_from(["t", "gallery", "--json"]).is_ok());
        assert!(T::try_parse_from(["t", "collect"]).is_ok());
        let l = Localizer::new(kor_i18n::Lang::Tr);
        localize(T::command(), &l).debug_assert();
    }

    #[test]
    fn quick_actions() {
        let up = derived("upscale", Some(Capability::Image)).unwrap();
        assert_eq!((up.cap, up.reference), (Capability::Upscale, true));
        let re = derived("reseed", Some(Capability::Video)).unwrap();
        assert!(re.same_model && re.keep_seed && re.new_prompt);
        assert_eq!(re.cap, Capability::Video);
        assert!(derived("nope", None).is_none());
    }

    #[test]
    fn answers_take_defaults_numbers_and_values() {
        let p = catalog::Param {
            name: "duration".into(),
            kind: catalog::ParamKind::Enum,
            role: catalog::Role::Duration,
            required: false,
            options: vec![json!("5s"), json!("8s")],
            min: None,
            max: None,
            default: Some(json!("5s")),
            description: String::new(),
        };
        assert_eq!(answer_value(&p, p.default.as_ref(), ""), Some(json!("5s")));
        assert_eq!(answer_value(&p, None, "2"), Some(json!("8s")));
        assert_eq!(answer_value(&p, None, "8s"), Some(json!("8s")));
        assert_eq!(answer_value(&p, None, "10s"), None);
        assert_eq!(
            saved_paths("x\nsaved: /a/b.png\n"),
            vec![PathBuf::from("/a/b.png")]
        );
    }

    #[test]
    fn ascii_sketch_of_an_image() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.png");
        image::RgbImage::from_fn(8, 4, |x, _| {
            if x < 4 {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([255, 255, 255])
            }
        })
        .save(&p)
        .unwrap();
        let art = ascii_preview(&p, 8).unwrap();
        assert!(art.starts_with(' ') && art.contains('@'), "{art:?}");
    }
}
