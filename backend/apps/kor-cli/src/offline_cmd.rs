//! `kor offline init|status|run|bundle|import` (§6.5, ADR 0034). Logic in
//! `kor_runtime::offline`; this file asks, prints and wires the REPL.

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use kor_i18n::{Localizer, args};
use kor_runtime::offline::{self, Event};

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum OfflineCmd {
    /// Prepare this computer for work without internet: runtime, models, embeddings, catalog.
    Init {
        /// llamacpp (portable, default when nothing runs), ollama, lmstudio.
        #[arg(long)]
        runtime: Option<String>,
        /// Models to download: GGUF names (qwen3-4b) or Ollama tags (qwen3:4b), comma separated.
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        /// Model used in offline mode (default: the first chosen).
        #[arg(long = "default")]
        default_model: Option<String>,
        #[arg(long)]
        no_embedding: bool,
        #[arg(long)]
        no_catalog: bool,
        #[arg(long, short)]
        yes: bool,
    },
    /// What is ready and whether a full task can run offline now.
    Status,
    /// Run Kor offline: privacy and offline mode on, the local model, no update checks.
    Run {
        #[arg(short, long)]
        model: Option<String>,
        #[arg(short = 'p', long)]
        print: bool,
        prompt: Vec<String>,
    },
    /// Write one air-gap archive: CLI, runtime, models, embedding model, catalog.
    Bundle {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        /// win-x64, win-arm64, linux-x64, linux-arm64, macos-x64, macos-arm64.
        #[arg(long)]
        platform: Option<String>,
        /// The kor executable to include (default: this one).
        #[arg(long)]
        cli: Option<PathBuf>,
        /// A desktop installer to include.
        #[arg(long)]
        desktop: Option<PathBuf>,
        #[arg(long)]
        no_embedding: bool,
    },
    /// Verify a bundle and install it into Kor's own folders.
    Import {
        file: PathBuf,
        #[arg(long, short)]
        yes: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-offline"))
        .mut_subcommand("init", |s| s.about(l.t("cmd-offline-init")))
        .mut_subcommand("status", |s| s.about(l.t("cmd-offline-status")))
        .mut_subcommand("run", |s| s.about(l.t("cmd-offline-run")))
        .mut_subcommand("bundle", |s| s.about(l.t("cmd-offline-bundle")))
        .mut_subcommand("import", |s| s.about(l.t("cmd-offline-import")))
}

pub fn run(ctx: &Ctx, cmd: OfflineCmd) -> anyhow::Result<bool> {
    match cmd {
        OfflineCmd::Init {
            runtime,
            models,
            default_model,
            no_embedding,
            no_catalog,
            yes,
        } => init(
            ctx,
            &offline::InitOptions {
                runtime,
                models,
                default_model,
                embedding: !no_embedding,
                catalog: !no_catalog,
            },
            yes,
        ),
        OfflineCmd::Status => status(ctx),
        OfflineCmd::Run {
            model,
            print,
            prompt,
        } => run_offline(ctx, model, print, &prompt),
        OfflineCmd::Bundle {
            out,
            models,
            platform,
            cli,
            desktop,
            no_embedding,
        } => bundle(
            ctx,
            &offline::BundleOptions {
                out,
                platform,
                models,
                cli,
                desktop,
                embedding: !no_embedding,
            },
        ),
        OfflineCmd::Import { file, yes } => Ok(import(ctx, &file, yes)),
    }
}

/// `1.2 GB` / `45.9 MB`
fn size(bytes: u64) -> String {
    // Display only; f64 is exact far beyond any download size.
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else {
        format!("{:.1} MB", b / 1e6)
    }
}

fn cfg(ctx: &Ctx) -> kor_config::Offline {
    ctx.config
        .as_ref()
        .map(|c| c.config.offline.clone())
        .unwrap_or_default()
}

fn registry(ctx: &Ctx) -> anyhow::Result<kor_providers::Registry> {
    kor_runtime::registry(&ctx.l, &mut Vec::new()).map_err(anyhow::Error::msg)
}

fn user_file(ctx: &Ctx) -> anyhow::Result<PathBuf> {
    ctx.paths
        .user
        .clone()
        .ok_or_else(|| anyhow::anyhow!(ctx.l.t("config-set-no-user-dir")))
}

/// `[Y/n]` on a terminal; `true` without one (scripts pass `--yes` anyway).
fn confirm(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return true;
    }
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    let a = line.trim().to_lowercase();
    a.is_empty() || ["y", "yes", "e", "evet", "o", "oui"].contains(&a.as_str())
}

fn read_line(question: &str) -> String {
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_owned()
}

/// Start/done lines on stdout, a percentage line on stderr while downloading.
fn printer(l: &Localizer) -> impl Fn(Event) + Send + Sync + '_ {
    move |e: Event| match e.state {
        "progress" => {
            if let Some(t) = e.total.filter(|t| *t > 0) {
                let pct = e.done.saturating_mul(100) / t;
                eprint!(
                    "\r  {} {pct:>3}%  {} / {}   ",
                    e.name,
                    size(e.done),
                    size(t)
                );
            } else if let Some(m) = &e.message {
                eprint!("\r  {} {m}   ", e.name);
            }
        }
        "start" => eprintln!("{}", l.t_args("offline-step", &args!("name" => e.name))),
        "done" => eprintln!(
            "\r{}",
            l.t_args("offline-step-done", &args!("name" => e.name))
        ),
        "error" => eprintln!(
            "\r{}",
            l.t_args(
                "error-generic",
                &args!("error" => e.message.unwrap_or_default())
            )
        ),
        _ => {}
    }
}

fn fail(l: &Localizer, e: &str) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_owned()))
    );
    false
}

fn choose_models(l: &Localizer, runtime: &str) -> Vec<String> {
    let choices: Vec<(String, String)> = if runtime == offline::LLAMACPP {
        offline::gguf_models()
            .into_iter()
            .map(|m| {
                let state = if m.installed {
                    l.t("offline-installed")
                } else {
                    m.size.map(size).unwrap_or_default()
                };
                (
                    offline::alias(&m.name).to_owned(),
                    format!("{state}  {}", m.about),
                )
            })
            .collect()
    } else if runtime == offline::OLLAMA {
        offline::OLLAMA_MODELS
            .iter()
            .map(|t| ((*t).to_owned(), String::new()))
            .collect()
    } else {
        return Vec::new();
    };
    println!("{}", l.t("offline-models-header"));
    for (i, (name, about)) in choices.iter().enumerate() {
        println!("  {}) {name}  {about}", i + 1);
    }
    read_line(&l.t("offline-pick"))
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            s.parse::<usize>()
                .ok()
                .and_then(|n| choices.get(n.wrapping_sub(1)))
                .map(|c| c.0.clone())
                .or_else(|| (!s.is_empty()).then(|| s.to_owned()))
        })
        .collect()
}

fn init(ctx: &Ctx, opts: &offline::InitOptions, yes: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let reg = registry(ctx)?;
    let file = user_file(ctx)?;
    let rt = crate::providers::runtime()?;
    let cfg = cfg(ctx);
    let mut plan = match rt.block_on(offline::plan(opts, &cfg, &reg)) {
        Ok(p) => p,
        Err(e) => return Ok(fail(l, &e)),
    };
    if plan.models.is_empty() && plan.default_model.is_empty() {
        if !std::io::stdin().is_terminal() || yes {
            let names: Vec<_> = offline::gguf_models()
                .into_iter()
                .map(|m| offline::alias(&m.name).to_owned())
                .collect();
            return Ok(fail(
                l,
                &l.t_args("offline-no-models", &args!("names" => names.join(", "))),
            ));
        }
        let picked = choose_models(l, &plan.runtime);
        if picked.is_empty() {
            return Ok(fail(
                l,
                &l.t_args("offline-no-models", &args!("names" => "")),
            ));
        }
        let again = offline::InitOptions {
            runtime: Some(plan.runtime.clone()),
            models: picked,
            ..opts.clone()
        };
        plan = match rt.block_on(offline::plan(&again, &cfg, &reg)) {
            Ok(p) => p,
            Err(e) => return Ok(fail(l, &e)),
        };
    }
    if plan.runtime == offline::OLLAMA && plan.endpoint.is_empty() {
        println!("{}", l.t("offline-ollama-hint"));
    }
    println!(
        "{}",
        l.t_args(
            "offline-plan",
            &args!(
                "runtime" => plan.runtime.clone(),
                "endpoint" => plan.endpoint.clone(),
                "model" => plan.default_model.clone()
            )
        )
    );
    if plan.downloads.is_empty() {
        println!("{}", l.t("offline-nothing-to-download"));
    } else {
        for d in &plan.downloads {
            println!("  {}  {}", d.name, d.size.map_or_else(|| "?".into(), size));
        }
        if !yes && !confirm(&l.t_args("offline-ask", &args!("size" => size(plan.total())))) {
            return Ok(false);
        }
    }
    let emit = printer(l);
    let report = match rt.block_on(offline::init(&plan, &file, &emit)) {
        Ok(r) => r,
        Err(e) => return Ok(fail(l, &e)),
    };
    for w in &report.warnings {
        eprintln!(
            "{}",
            l.t_args("offline-warning", &args!("text" => w.clone()))
        );
    }
    println!(
        "{}",
        l.t_args(
            "offline-init-done",
            &args!("runtime" => report.runtime, "model" => report.model)
        )
    );
    Ok(true)
}

fn status(ctx: &Ctx) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let reg = registry(ctx)?;
    let s = crate::providers::runtime()?.block_on(offline::status(&cfg(ctx), &reg));
    let yes_no = |b: bool| l.t(if b { "offline-yes" } else { "offline-no" });
    let dash = |s: &str| {
        if s.is_empty() {
            "-".to_owned()
        } else {
            s.to_owned()
        }
    };
    let mut rows = vec![
        vec![
            l.t("offline-col-runtime"),
            format!("{} {}", dash(&s.runtime), s.endpoint),
        ],
        vec![l.t("offline-col-runtime-ready"), yes_no(s.runtime_ready)],
        vec![l.t("offline-col-model"), dash(&s.model)],
        vec![
            l.t("offline-col-llama"),
            s.llama_server.clone().unwrap_or_else(|| "-".into()),
        ],
        vec![l.t("offline-col-embedding"), yes_no(s.embedding)],
        vec![
            l.t("offline-col-catalog"),
            s.catalog_age_secs.map_or_else(
                || "-".into(),
                |a| l.t_args("offline-age-hours", &args!("hours" => a / 3600)),
            ),
        ],
        vec![l.t("offline-col-mode"), yes_no(s.enabled)],
    ];
    for srv in &s.servers {
        rows.push(vec![
            l.t("offline-col-server"),
            format!("{} {} ({})", srv.name, srv.base_url, srv.models.len()),
        ]);
    }
    for m in s.models.iter().filter(|m| m.installed) {
        rows.push(vec![l.t("offline-col-gguf"), m.model_ref.clone()]);
    }
    crate::providers::table(&rows);
    if s.ready {
        println!("{}", l.t("offline-ready"));
    } else {
        let missing: Vec<String> = s
            .missing
            .iter()
            .map(|m| l.t(&format!("offline-missing-{m}")))
            .collect();
        println!(
            "{}",
            l.t_args("offline-not-ready", &args!("missing" => missing.join(", ")))
        );
    }
    Ok(s.ready)
}

fn run_offline(
    ctx: &Ctx,
    model: Option<String>,
    print: bool,
    prompt: &[String],
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    // Same layers, with offline (implies privacy) switched on for this run.
    let overrides = [
        ("offline.enabled", kor_config::Value::Boolean(true)),
        ("privacy.mode", kor_config::Value::Boolean(true)),
        (
            "ui.language",
            kor_config::Value::String(l.lang().code().into()),
        ),
    ];
    let config = kor_config::load(&ctx.paths, std::env::vars(), &overrides);
    let cfg = config
        .as_ref()
        .map(|c| c.config.offline.clone())
        .unwrap_or_default();
    if model.is_none() && cfg.model.is_empty() {
        return Ok(fail(l, &l.t("offline-run-no-model")));
    }
    offline::restore_catalog();
    let rt = crate::providers::runtime()?;
    let server = match rt.block_on(offline::serve(&cfg)) {
        Ok(s) => s,
        Err(e) => return Ok(fail(l, &e)),
    };
    drop(rt);
    if let Some(s) = &server {
        eprintln!(
            "{}",
            l.t_args(
                "offline-serving",
                &args!("model" => s.model.clone(), "port" => offline::LLAMA_PORT)
            )
        );
    }
    let sub = Ctx {
        l: Localizer::new(l.lang()),
        paths: ctx.paths.clone(),
        config,
    };
    let result = crate::repl::run(
        &sub,
        &crate::repl::RunArgs {
            prompt: (!prompt.is_empty()).then(|| prompt.join(" ")),
            print,
            model,
            ..crate::repl::RunArgs::default()
        },
    );
    drop(server); // stops llama-server
    result
}

fn bundle(ctx: &Ctx, opts: &offline::BundleOptions) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let emit = printer(l);
    let r = crate::providers::runtime()?.block_on(offline::create_bundle(opts, &cfg(ctx), &emit));
    let m = match r {
        Ok(m) => m,
        Err(e) => return Ok(fail(l, &e)),
    };
    if !m
        .files
        .iter()
        .any(|f| f.kind == kor_components::bundle::Kind::Cli)
    {
        println!(
            "{}",
            l.t_args(
                "offline-bundle-no-cli",
                &args!("platform" => m.platform.clone())
            )
        );
    }
    println!(
        "{}",
        l.t_args(
            "offline-bundle-done",
            &args!(
                "path" => opts.out.display().to_string(),
                "size" => size(m.total()),
                "files" => m.files.len()
            )
        )
    );
    Ok(true)
}

fn import(ctx: &Ctx, file: &Path, yes: bool) -> bool {
    let l = &ctx.l;
    let m = match offline::inspect(file) {
        Ok(m) => m,
        Err(e) => return fail(l, &e),
    };
    let q = l.t_args(
        "offline-import-ask",
        &args!(
            "files" => m.files.len(),
            "size" => size(m.total()),
            "platform" => m.platform.clone(),
            "version" => m.kor_version.clone()
        ),
    );
    if !yes && !confirm(&q) {
        return false;
    }
    let user = ctx.paths.user.clone();
    let emit = printer(l);
    let r = match offline::import(file, user.as_deref(), &emit) {
        Ok(r) => r,
        Err(e) => return fail(l, &e),
    };
    println!(
        "{}",
        l.t_args(
            "offline-import-done",
            &args!("components" => r.components.join(", "), "model" => r.model.clone())
        )
    );
    if let Some(p) = r.cli {
        println!(
            "{}",
            l.t_args(
                "offline-import-cli",
                &args!("path" => p, "bundle" => file.display().to_string())
            )
        );
    }
    if let Some(p) = r.desktop {
        println!(
            "{}",
            l.t_args("offline-import-desktop", &args!("path" => p))
        );
    }
    true
}
