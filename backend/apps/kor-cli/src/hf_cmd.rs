//! `kor hf search|info|download|infer`: the Hugging Face Hub (model search, model card metadata,
//! file listing, resumable checksum-verified downloads into Kor's models folder) and HF task
//! inference (router or a dedicated Inference Endpoint). Logic: `kor_providers::huggingface`.

use std::io::{BufRead as _, IsTerminal as _, Write as _};

use clap::Subcommand;
use kor_i18n::args;
use kor_providers::huggingface::{self as hf, Hf, HubFile, SearchQuery};
use serde_json::Value;

use crate::Ctx;

/// Downloads above this ask first (unless `--yes`).
const CONFIRM_ABOVE: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Subcommand)]
pub enum HfCmd {
    /// Search Hub models.
    Search {
        query: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        filter: Vec<String>,
        #[arg(long)]
        pipeline: Option<String>,
        #[arg(long)]
        sort: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Model card metadata and files.
    Info {
        repo: String,
        #[arg(long)]
        revision: Option<String>,
    },
    /// Download a repo's files (all, one, or `--include` pattern) into Kor's models folder.
    Download {
        repo: String,
        file: Option<String>,
        #[arg(long)]
        include: Option<String>,
        #[arg(long)]
        revision: Option<String>,
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        #[arg(long)]
        yes: bool,
    },
    /// Run an HF task (text-to-image, speech-to-text, feature-extraction, image-classification,
    /// object-detection) on the router or a dedicated endpoint.
    Infer {
        model: String,
        task: String,
        input: String,
        #[arg(long)]
        endpoint: Option<String>,
    },
}

/// `<kor home>/models`.
pub fn models_dir() -> Option<std::path::PathBuf> {
    kor_runtime::kor_home().map(|h| h.join("models"))
}

fn gb(n: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let g = n as f64 / 1_073_741_824.0;
    format!("{g:.2}")
}

/// Files to fetch: `file` exactly, else those matching `include`, else all.
pub fn select(files: Vec<HubFile>, file: Option<&str>, include: Option<&str>) -> Vec<HubFile> {
    files
        .into_iter()
        .filter(|f| match (file, include) {
            (Some(name), _) => f.path == name,
            (None, Some(p)) => hf::glob_match(p, &f.path),
            (None, None) => true,
        })
        .collect()
}

#[allow(clippy::too_many_lines)] // one arm per subcommand
pub fn run(ctx: &Ctx, cmd: &HfCmd) -> anyhow::Result<bool> {
    if crate::media_cmd::privacy_blocked(ctx) {
        return Ok(false);
    }
    let l = &ctx.l;
    let endpoint = match cmd {
        HfCmd::Infer { endpoint, .. } => endpoint.as_deref(),
        _ => None,
    };
    let (client, token) = match kor_runtime::hf_client(l, endpoint) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{}", l.t_args("hf-error", &args!("error" => e)));
            return Ok(false);
        }
    };
    let rt = crate::providers::runtime()?;
    let fail = |e: String| -> anyhow::Result<bool> {
        eprintln!("{}", l.t_args("hf-error", &args!("error" => e)));
        Ok(false)
    };
    match cmd {
        HfCmd::Search {
            query,
            author,
            filter,
            pipeline,
            sort,
            limit,
        } => {
            let q = SearchQuery {
                search: query.clone(),
                author: author.clone(),
                filter: filter.clone(),
                pipeline_tag: pipeline.clone(),
                sort: sort.clone(),
                limit: *limit,
            };
            match rt.block_on(client.search(&q)) {
                Ok(list) if list.is_empty() => {
                    println!("{}", l.t("hf-none"));
                    Ok(true)
                }
                Ok(list) => {
                    for m in list {
                        println!(
                            "{:<56} {:>10} dl {:>6} likes  {}",
                            m.id,
                            m.downloads,
                            m.likes,
                            m.pipeline_tag.unwrap_or_default()
                        );
                    }
                    Ok(true)
                }
                Err(e) => fail(e.message),
            }
        }
        HfCmd::Info { repo, revision } => {
            let info = match rt.block_on(client.info(repo, revision.as_deref())) {
                Ok(v) => v,
                Err(e) => return fail(e.message),
            };
            let s = |k: &str| {
                info.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned()
            };
            println!(
                "{}",
                l.t_args(
                    "hf-info",
                    &args!(
                        "id" => s("id"),
                        "pipeline" => s("pipeline_tag"),
                        "library" => s("library_name"),
                        "license" => info.pointer("/cardData/license").and_then(Value::as_str).unwrap_or("-").to_owned(),
                        "downloads" => info.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                        "likes" => info.get("likes").and_then(Value::as_u64).unwrap_or(0)
                    )
                )
            );
            if let Some(g) = hf::gated(&info) {
                println!(
                    "{}",
                    l.t_args(
                        "hf-gated",
                        &args!("mode" => g, "url" => format!("{}/{repo}", hf::HUB))
                    )
                );
            }
            match rt.block_on(client.files(repo, revision.as_deref())) {
                Ok(files) => {
                    for f in files {
                        println!("  {:<60} {:>10}", f.path, f.size);
                    }
                    Ok(true)
                }
                Err(e) => fail(e.message),
            }
        }
        HfCmd::Download {
            repo,
            file,
            include,
            revision,
            dir,
            yes,
        } => {
            let Some(dest) = dir.clone().or_else(models_dir) else {
                return fail("no models folder".into());
            };
            rt.block_on(download(
                ctx,
                &client,
                token.is_some(),
                repo,
                revision.as_deref(),
                file.as_deref(),
                include.as_deref(),
                &dest,
                *yes,
            ))
        }
        HfCmd::Infer {
            model, task, input, ..
        } => {
            let root = kor_runtime::project_root()?;
            match rt.block_on(kor_agent::media::hf_infer(
                &client, model, task, input, &root,
            )) {
                Ok(out) => {
                    println!("{out}");
                    Ok(true)
                }
                Err(e) => fail(e),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn download(
    ctx: &Ctx,
    client: &Hf,
    has_token: bool,
    repo: &str,
    rev: Option<&str>,
    file: Option<&str>,
    include: Option<&str>,
    dest: &std::path::Path,
    yes: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let files = match client.files(repo, rev).await {
        Ok(f) => select(f, file, include),
        Err(e) => {
            eprintln!("{}", l.t_args("hf-error", &args!("error" => e.message)));
            return Ok(false);
        }
    };
    if files.is_empty() {
        eprintln!("{}", l.t("hf-no-files"));
        return Ok(false);
    }
    if !has_token
        && let Ok(info) = client.info(repo, rev).await
        && let Some(g) = hf::gated(&info)
    {
        eprintln!(
            "{}",
            l.t_args(
                "hf-gated",
                &args!("mode" => g, "url" => format!("{}/{repo}", hf::HUB))
            )
        );
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    let plan = l.t_args(
        "hf-plan",
        &args!(
            "count" => files.len(),
            "size" => gb(total),
            "dir" => kor_tools::display_path(dest)
        ),
    );
    if total > CONFIRM_ABOVE && !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("{plan}\n{}", l.t("hf-use-yes"));
            return Ok(false);
        }
        eprint!("{plan} [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if !matches!(line.trim(), "y" | "Y" | "e" | "E" | "o" | "O") {
            return Ok(false);
        }
    } else {
        eprintln!("{plan}");
    }
    let cancel = kor_providers::CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            c2.cancel();
        }
    });
    for f in &files {
        let shown = std::sync::atomic::AtomicU64::new(u64::MAX);
        let progress = |done: u64, size: u64| {
            let pct = (done * 100).checked_div(size).unwrap_or(0);
            if shown.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                eprint!("\r{} {pct}%", f.path);
                let _ = std::io::stderr().flush();
            }
        };
        match client
            .download(repo, rev, f, dest, &cancel, &progress)
            .await
        {
            Ok(d) => {
                eprintln!();
                let key = if d.verified {
                    "hf-saved-verified"
                } else {
                    "hf-saved"
                };
                println!(
                    "{}",
                    l.t_args(key, &args!("path" => kor_tools::display_path(&d.path)))
                );
            }
            Err(e) => {
                eprintln!();
                eprintln!("{}", l.t_args("hf-error", &args!("error" => e.message)));
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_by_name_pattern_or_all() {
        let f = |p: &str| HubFile {
            path: p.into(),
            size: 1,
            sha256: None,
        };
        let all = vec![f("a.Q4_K_M.gguf"), f("a.Q8_0.gguf"), f("README.md")];
        assert_eq!(select(all.clone(), Some("README.md"), None).len(), 1);
        assert_eq!(select(all.clone(), None, Some("*.gguf")).len(), 2);
        assert_eq!(select(all.clone(), None, Some("*Q4_K_M*")).len(), 1);
        assert_eq!(select(all, None, None).len(), 3);
    }
}
