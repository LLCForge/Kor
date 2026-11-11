//! `kor providers`, `kor models`, `kor chat`.

use std::io::Write;
use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use kor_core::{Message, ModelRef, ProviderError, Role, StreamEvent};
use kor_i18n::{Localizer, args};
use kor_providers::catalog::{self, Catalog};
use kor_providers::{CancellationToken, ChatRequest, ModelInfo, ProviderManifest, Registry};
use kor_secrets::{KeySource, SecretString};

use crate::Ctx;

const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Built-in manifests plus the user's `<config>/providers/*.toml` (see [`kor_runtime::registry`]).
pub fn registry(l: &Localizer) -> Option<Registry> {
    let mut notes = Vec::new();
    let r = kor_runtime::registry(l, &mut notes);
    for n in &notes {
        eprintln!("{n}");
    }
    match r {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("{e}");
            None
        }
    }
}

pub use kor_runtime::{key_for, provider_error};

pub(crate) fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")
}

pub(crate) fn table(rows: &[Vec<String>]) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for r in rows {
        let line: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{s:<w$}", w = widths[i]))
            .collect();
        println!("{}", line.join("  ").trim_end());
    }
}

pub fn list_providers(ctx: &Ctx) -> bool {
    let l = &ctx.l;
    let Some(reg) = registry(l) else {
        return false;
    };
    let mut rows = vec![
        l.t("providers-columns")
            .split('|')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    ];
    for m in reg.iter() {
        let key = if m.needs_key() {
            match key_for(m) {
                Ok(Some((_, KeySource::Env(v)))) => format!("{} ({v})", l.t("key-env")),
                Ok(Some((_, KeySource::Keychain))) => l.t("key-keychain"),
                Ok(None) => l.t("key-missing"),
                Err(e) => format!("{}: {e}", l.t("key-missing")),
            }
        } else {
            l.t("key-not-needed")
        };
        rows.push(vec![
            m.id.clone(),
            m.name.clone(),
            serde_json::to_value(m.category)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            m.adapter.as_str().to_owned(),
            m.privacy.tier.to_string(),
            l.t(if m.verified {
                "verified-yes"
            } else {
                "verified-no"
            }),
            key,
        ]);
    }
    table(&rows);
    true
}

fn human_ctx(n: Option<u64>) -> String {
    match n {
        Some(n) if n >= 1_000_000 && n % 1_000_000 == 0 => format!("{}M", n / 1_000_000),
        Some(n) if n >= 1000 => format!("{}k", n / 1000),
        Some(n) => n.to_string(),
        None => "-".into(),
    }
}

pub fn list_models(
    ctx: &Ctx,
    only: Option<&str>,
    refresh: bool,
    offline: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(reg) = registry(l) else {
        return Ok(false);
    };
    if let Some(id) = only
        && reg.get(id).is_none()
    {
        eprintln!(
            "{}",
            l.t_args("chat-unknown-provider", &args!("provider" => id.to_owned()))
        );
        return Ok(false);
    }
    let cache_path = dirs::cache_dir().map(|d| d.join("kor").join("models.json"));
    let mut cat = cache_path.as_deref().map(Catalog::load).unwrap_or_default();
    let now = catalog::now();

    let privacy = ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
    let no_auto = ctx.config.as_ref().is_ok_and(|c| !c.config.auto.catalog_refresh);
    let mut skipped = Vec::new();
    let mut to_fetch = Vec::new();
    let mut shown: Vec<(&ProviderManifest, Vec<ModelInfo>)> = Vec::new();
    for m in reg.iter().filter(|m| only.is_none_or(|id| id == m.id)) {
        let key = match key_for(m)? {
            Some((k, _)) => Some(k),
            None if m.needs_key() => {
                skipped.push(m.id.clone());
                continue;
            }
            None => None,
        };
        // Privacy mode: remote catalogs come only from the local cache.
        let offline = offline || (privacy && !kor_runtime::is_local_provider(m));
        let cached = if offline {
            cat.providers.get(&m.id)
        } else if refresh {
            None
        } else if no_auto {
            // `[auto] catalog_refresh = false`: stale lists stay until `--refresh` (§28.14).
            cat.providers.get(&m.id)
        } else {
            cat.fresh(&m.id, now)
        };
        match cached {
            Some(e) => shown.push((m, e.models.clone())),
            None if offline => {}
            None => to_fetch.push((m, key)),
        }
    }

    let ok = to_fetch.is_empty()
        || fetch_into(
            l,
            to_fetch,
            &mut cat,
            now,
            &mut shown,
            cache_path.as_deref(),
        )?;
    shown.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let mut rows = Vec::new();
    for (m, models) in &shown {
        let mut models = models.clone();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        for model in models {
            rows.push(vec![
                format!("{}/{}", m.id, model.id),
                human_ctx(model.context_window.or(m.max_context_default)),
                m.privacy.tier.to_string(),
            ]);
        }
    }
    if rows.is_empty() {
        println!("{}", l.t("models-none"));
    } else {
        table(&rows);
    }
    if !skipped.is_empty() {
        eprintln!(
            "{}",
            l.t_args("models-skipped", &args!("providers" => skipped.join(", ")))
        );
    }
    Ok(ok)
}

type Fetch<'a> = (&'a ProviderManifest, Option<SecretString>);

/// Fetch model lists concurrently, update the cache, fall back to stale entries on error.
fn fetch_into<'a>(
    l: &Localizer,
    to_fetch: Vec<Fetch<'a>>,
    cat: &mut Catalog,
    now: u64,
    shown: &mut Vec<(&'a ProviderManifest, Vec<ModelInfo>)>,
    cache_path: Option<&std::path::Path>,
) -> anyhow::Result<bool> {
    let mut ok = true;
    let results = runtime()?.block_on(futures::future::join_all(to_fetch.into_iter().map(
        |(m, key)| async move {
            let r = match kor_providers::build(m, key) {
                Ok(p) => match tokio::time::timeout(FETCH_TIMEOUT, p.list_models()).await {
                    Ok(r) => r,
                    Err(_) => Err(ProviderError::new(
                        &m.id,
                        kor_core::ProviderErrorKind::Timeout,
                        "timed out",
                    )),
                },
                Err(e) => Err(e),
            };
            (m, r)
        },
    )));
    for (m, r) in results {
        match r {
            Ok(models) => {
                cat.put(&m.id, models.clone(), now);
                shown.push((m, models));
            }
            Err(e) => {
                ok = false;
                eprintln!(
                    "{}",
                    l.t_args(
                        "models-fetch-failed",
                        &args!("error" => provider_error(l, &e))
                    )
                );
                if let Some(stale) = cat.providers.get(&m.id) {
                    let hours = now.saturating_sub(stale.fetched_at) / 3600;
                    eprintln!(
                        "{}",
                        l.t_args(
                            "models-stale",
                            &args!("provider" => m.id.clone(), "hours" => hours)
                        )
                    );
                    shown.push((m, stale.models.clone()));
                }
            }
        }
    }
    if let Some(p) = cache_path
        && let Err(e) = cat.save(p)
    {
        eprintln!(
            "{}",
            l.t_args("error-generic", &args!("error" => e.to_string()))
        );
    }
    Ok(ok)
}

pub struct ChatArgs {
    pub model: String,
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: Option<u32>,
}

pub fn chat(ctx: &Ctx, input: ChatArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Ok(model) = input.model.parse::<ModelRef>() else {
        eprintln!(
            "{}",
            l.t_args("chat-bad-model", &args!("model" => input.model))
        );
        return Ok(false);
    };
    let Some(reg) = registry(l) else {
        return Ok(false);
    };
    let Some(manifest) = reg.get(model.provider()) else {
        eprintln!(
            "{}",
            l.t_args(
                "chat-unknown-provider",
                &args!("provider" => model.provider().to_owned())
            )
        );
        return Ok(false);
    };
    let provider = match kor_providers::build(manifest, key_for(manifest)?.map(|(k, _)| k)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", provider_error(l, &e));
            return Ok(false);
        }
    };
    let req = ChatRequest {
        model: model.model().to_owned(),
        system: input.system,
        messages: vec![Message::text(Role::User, input.prompt)],
        max_tokens: input.max_tokens,
        ..ChatRequest::default()
    };

    runtime()?.block_on(async {
        let cancel = CancellationToken::new();
        let on_signal = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                on_signal.cancel();
            }
        });
        let mut stream = match provider.complete(req, cancel).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", provider_error(l, &e));
                return Ok(false);
            }
        };
        let mut out = std::io::stdout().lock();
        while let Some(ev) = stream.next().await {
            match ev {
                Ok(StreamEvent::TextDelta { text }) => {
                    out.write_all(text.as_bytes())?;
                    out.flush()?;
                }
                Ok(StreamEvent::ThinkingDelta { text }) => eprint!("{text}"),
                Ok(StreamEvent::Usage { usage }) => {
                    writeln!(out)?;
                    eprintln!(
                        "{}",
                        l.t_args(
                            "chat-usage",
                            &args!("input" => usage.input, "output" => usage.output)
                        )
                    );
                }
                Ok(StreamEvent::Finish { reason }) => {
                    if reason != kor_core::FinishReason::Stop {
                        eprintln!(
                            "[{}]",
                            serde_json::to_value(reason)?.as_str().unwrap_or_default()
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    writeln!(out)?;
                    eprintln!("{}", provider_error(l, &e));
                    return Ok(false);
                }
            }
        }
        Ok(true)
    })
}
