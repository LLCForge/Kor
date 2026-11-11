//! `kor provider …`: test, status, add/remove custom endpoints, local discovery, key pool.
//! Thin client over `kor_providers::{probe, custom, health, pool}`; the desktop calls the same
//! functions (`provider.*`, `g9.testKey`, `g9.health`).

use std::io::{IsTerminal, Read};

use anyhow::Context;
use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_providers::custom::{self, AuthMethod};
use kor_providers::{Provider, ProviderManifest, Registry, health, probe};
use kor_secrets::{
    ExposeSecret, KeychainStore, MAX_POOL_KEYS, MAX_SECRET_LEN, SecretError, SecretStore,
    SecretString, mask, pool_account,
};

use crate::Ctx;
use crate::providers::{provider_error, registry, runtime};

#[derive(Debug, Subcommand)]
pub enum ProviderCmd {
    /// List models, then make one tiny streaming call.
    Test {
        id: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Latency, error rate and remaining quota per provider.
    Status { id: Option<String> },
    /// Add an `OpenAI`- or Anthropic-compatible endpoint, a local server or a subscription proxy.
    Add {
        id: String,
        #[arg(long)]
        url: String,
        #[arg(long, conflicts_with = "proxy")]
        anthropic: bool,
        #[arg(long)]
        no_key: bool,
        #[arg(long)]
        proxy: bool,
    },
    /// Remove a provider added with `kor provider add`.
    Remove { id: String },
    /// Find local model servers on their default ports.
    Discover {
        #[arg(long)]
        save: bool,
    },
    /// Keys in the failover pool (masked).
    Keys { id: String },
    /// Store one more key for failover.
    AddKey { id: String },
    /// Delete the key in pool slot `slot`.
    RemoveKey { id: String, slot: usize },
}

fn privacy(ctx: &Ctx) -> bool {
    ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode)
}

fn user_providers_dir() -> Option<std::path::PathBuf> {
    kor_config::user_dir().map(|d| d.join("providers"))
}

fn known<'a>(l: &Localizer, reg: &'a Registry, id: &str) -> Option<&'a ProviderManifest> {
    let m = reg.get(id);
    if m.is_none() {
        eprintln!(
            "{}",
            l.t_args("chat-unknown-provider", &args!("provider" => id.to_owned()))
        );
    }
    m
}

fn build(
    l: &Localizer,
    m: &ProviderManifest,
) -> anyhow::Result<Option<std::sync::Arc<dyn Provider>>> {
    let keys = kor_providers::pool::keys_for(m, &KeychainStore)?;
    Ok(match kor_providers::build_pool(m, keys) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("{}", provider_error(l, &e));
            None
        }
    })
}

fn health_file() -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|d| d.join("kor").join("provider-health.json"))
}

pub fn run(ctx: &Ctx, cmd: ProviderCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(reg) = registry(l) else {
        return Ok(false);
    };
    if let Some(f) = health_file() {
        health::set_store(f);
    }
    match cmd {
        ProviderCmd::Test { id, model } => test(ctx, &reg, &id, model.as_deref()),
        ProviderCmd::Status { id } => status(ctx, &reg, id.as_deref()),
        ProviderCmd::Add {
            id,
            url,
            anthropic,
            no_key,
            proxy,
        } => add(l, &id, url, anthropic, no_key, proxy),
        ProviderCmd::Remove { id } => remove(l, &id),
        ProviderCmd::Discover { save } => discover(l, &reg, save),
        ProviderCmd::Keys { id } => keys(l, &reg, &id),
        ProviderCmd::AddKey { id } => add_key(l, &reg, &id),
        ProviderCmd::RemoveKey { id, slot } => remove_key(l, &reg, &id, slot),
    }
}

fn blocked(ctx: &Ctx, m: &ProviderManifest) -> bool {
    let b = privacy(ctx) && !kor_runtime::is_local_provider(m);
    if b {
        eprintln!(
            "{}",
            ctx.l.t_args(
                "provider-privacy-blocked",
                &args!("provider" => m.id.clone())
            )
        );
    }
    b
}

#[allow(clippy::many_single_char_names)]
fn test(ctx: &Ctx, reg: &Registry, id: &str, model: Option<&str>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(m) = known(l, reg, id) else {
        return Ok(false);
    };
    if blocked(ctx, m) {
        return Ok(false);
    }
    let Some(p) = build(l, m)? else {
        return Ok(false);
    };
    let r = runtime()?.block_on(probe::test(p.as_ref(), model));
    let c = &r.check;
    match &c.error {
        None => println!(
            "{}",
            l.t_args(
                "provider-test-models",
                &args!("provider" => id.to_owned(), "count" => c.models, "ms" => c.latency_ms)
            )
        ),
        Some(e) => println!(
            "{}",
            l.t_args(
                "provider-test-models-failed",
                &args!("provider" => id.to_owned(), "error" => provider_error(l, e))
            )
        ),
    }
    let (Some(model), Some(s)) = (&r.model, &r.stream) else {
        println!("{}", l.t("provider-test-no-model"));
        return Ok(false);
    };
    let base = args!("provider" => id.to_owned(), "model" => model.clone());
    let mut a = base;
    match &s.error {
        None => {
            a.set("first", s.first_token_ms.unwrap_or(s.total_ms));
            a.set("ms", s.total_ms);
            a.set("text", s.text.trim().to_owned());
            println!("{}", l.t_args("provider-test-stream", &a));
        }
        Some(e) => {
            a.set("error", provider_error(l, e));
            println!("{}", l.t_args("provider-test-stream-failed", &a));
        }
    }
    Ok(c.ok && s.ok)
}

fn opt(n: Option<u64>) -> String {
    n.map_or_else(|| "-".into(), |n| n.to_string())
}

fn status(ctx: &Ctx, reg: &Registry, only: Option<&str>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if let Some(id) = only
        && known(l, reg, id).is_none()
    {
        return Ok(false);
    }
    // Probe what is usable now: a key is present (or none is needed) and privacy allows it.
    let mut targets = Vec::new();
    for m in reg.iter().filter(|m| only.is_none_or(|id| id == m.id)) {
        if privacy(ctx) && !kor_runtime::is_local_provider(m) {
            continue;
        }
        let keys = kor_providers::pool::keys_for(m, &KeychainStore)?;
        if m.needs_key() && keys.is_empty() {
            continue;
        }
        // Local servers that are not running are only probed when asked for by name.
        if only.is_none() && !m.needs_key() {
            continue;
        }
        if let Ok(p) = kor_providers::build_pool(m, keys) {
            targets.push(p);
        }
    }
    runtime()?.block_on(futures::future::join_all(
        targets.iter().map(|p| probe::check(p.as_ref())),
    ));
    let snap = health::snapshot();
    let mut rows = vec![
        l.t("provider-status-columns")
            .split('|')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    ];
    for (id, s) in snap
        .iter()
        .filter(|(id, _)| only.is_none_or(|o| id.as_str() == o || id.starts_with(&format!("{o}#"))))
    {
        rows.push(vec![
            id.clone(),
            s.latency_ms()
                .map_or_else(|| "-".into(), |ms| format!("{ms} ms")),
            format!("{:.0}%", s.error_rate() * 100.0),
            s.recent.len().to_string(),
            opt(s.quota.remaining_requests),
            opt(s.quota.remaining_tokens),
            s.last_error
                .clone()
                .unwrap_or_default()
                .chars()
                .take(60)
                .collect(),
        ]);
    }
    if rows.len() == 1 {
        println!("{}", l.t("provider-status-empty"));
    } else {
        crate::providers::table(&rows);
    }
    Ok(true)
}

fn add(
    l: &Localizer,
    id: &str,
    url: String,
    anthropic: bool,
    no_key: bool,
    proxy: bool,
) -> anyhow::Result<bool> {
    if Registry::builtin().get(id).is_some() {
        eprintln!(
            "{}",
            l.t_args("provider-add-builtin", &args!("provider" => id.to_owned()))
        );
        return Ok(false);
    }
    let proxy = proxy || id == "cliproxy";
    let loopback = url::is_loopback(&url);
    let method = if proxy {
        AuthMethod::SubscriptionProxy { endpoint: url }
    } else if loopback && no_key && !anthropic {
        AuthMethod::Local { base_url: url }
    } else {
        AuthMethod::CustomEndpoint {
            base_url: url,
            anthropic,
            key: !no_key,
        }
    };
    let (m, dir) = match (custom::custom_manifest(id, &method), user_providers_dir()) {
        (Ok(m), Some(dir)) => (m, dir),
        (Err(e), _) => {
            eprintln!(
                "{}",
                l.t_args(
                    "manifest-invalid",
                    &args!("origin" => e.origin, "error" => e.message)
                )
            );
            return Ok(false);
        }
        (_, None) => anyhow::bail!("no config directory"),
    };
    let path = match custom::save_user_manifest(&dir, &m) {
        Ok(p) => p,
        Err(e) => anyhow::bail!("{e}"),
    };
    println!(
        "{}",
        l.t_args(
            "provider-added",
            &args!("provider" => id.to_owned(), "path" => path.display().to_string())
        )
    );
    if m.needs_key() {
        println!(
            "{}",
            l.t_args(
                "provider-add-key-hint",
                &args!("provider" => id.to_owned(), "var" => m.key_env())
            )
        );
    }
    if let AuthMethod::SubscriptionProxy { endpoint } = &method {
        eprintln!("{}", l.t("provider-proxy-warning"));
        println!(
            "{}",
            l.t_args(
                "provider-proxy-panel",
                &args!("url" => custom::proxy_panel(endpoint))
            )
        );
        // The proxy runs on this machine: listing its models needs no outbound network.
        if url::is_loopback(endpoint)
            && let Some(p) = build(l, &m)?
        {
            let c = runtime()?.block_on(probe::check(p.as_ref()));
            match c.error {
                None => println!(
                    "{}",
                    l.t_args("provider-proxy-models", &args!("count" => c.models))
                ),
                Some(e) => eprintln!("{}", provider_error(l, &e)),
            }
        }
    }
    Ok(true)
}

fn remove(l: &Localizer, id: &str) -> anyhow::Result<bool> {
    let Some(dir) = user_providers_dir() else {
        anyhow::bail!("no config directory");
    };
    let a = args!("provider" => id.to_owned());
    match custom::remove_user_manifest(&dir, id) {
        Ok(true) => println!("{}", l.t_args("provider-removed", &a)),
        Ok(false) => {
            eprintln!("{}", l.t_args("provider-not-user", &a));
            return Ok(false);
        }
        Err(e) => anyhow::bail!("{e}"),
    }
    Ok(true)
}

fn discover(l: &Localizer, reg: &Registry, save: bool) -> anyhow::Result<bool> {
    let found = runtime()?.block_on(probe::discover(reg));
    if found.is_empty() {
        println!("{}", l.t("provider-discover-none"));
        return Ok(true);
    }
    for f in &found {
        println!(
            "{}",
            l.t_args(
                "provider-discover-found",
                &args!("name" => f.name.clone(), "url" => f.base_url.clone(), "count" => f.models.len())
            )
        );
        // A server on a non-default port is remembered as a user override of that manifest.
        if save
            && f.moved
            && let (Some(m), Some(dir)) = (reg.get(&f.provider), user_providers_dir())
        {
            let mut m = m.clone();
            m.base_url.clone_from(&f.base_url);
            if custom::save_user_manifest(&dir, &m).is_ok() {
                println!(
                    "{}",
                    l.t_args(
                        "provider-discover-saved",
                        &args!("provider" => f.provider.clone(), "url" => f.base_url.clone())
                    )
                );
            }
        }
    }
    Ok(true)
}

fn keyed<'a>(l: &Localizer, reg: &'a Registry, id: &str) -> Option<&'a ProviderManifest> {
    known(l, reg, id).filter(|m| m.needs_key())
}

fn keys(l: &Localizer, reg: &Registry, id: &str) -> anyhow::Result<bool> {
    let Some(m) = keyed(l, reg, id) else {
        return Ok(false);
    };
    let store = KeychainStore;
    let mut any = false;
    for slot in 1..=MAX_POOL_KEYS {
        if let Some(k) = store.get(&pool_account(&m.id, slot)?)? {
            any = true;
            println!(
                "{}",
                l.t_args(
                    "provider-key-slot",
                    &args!("slot" => slot, "masked" => mask(k.expose_secret()))
                )
            );
        }
    }
    if !any {
        println!(
            "{}",
            l.t_args("provider-keys-none", &args!("provider" => id.to_owned()))
        );
    }
    Ok(true)
}

/// Hidden prompt on a terminal; otherwise one bounded read of stdin (for scripts).
fn read_secret(prompt: &str) -> anyhow::Result<Option<SecretString>> {
    let raw = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!("{prompt} ")).context("terminal")?
    } else {
        let mut buf = String::new();
        std::io::stdin()
            .take(MAX_SECRET_LEN as u64 + 2)
            .read_to_string(&mut buf)
            .context("stdin")?;
        buf
    };
    let trimmed = raw.trim_end_matches(['\r', '\n']).trim();
    Ok((!trimmed.is_empty()).then(|| SecretString::from(trimmed.to_owned())))
}

fn add_key(l: &Localizer, reg: &Registry, id: &str) -> anyhow::Result<bool> {
    let Some(m) = keyed(l, reg, id) else {
        return Ok(false);
    };
    let a = || args!("provider" => m.id.clone());
    let Some(secret) = read_secret(&l.t_args("auth-prompt", &a()))? else {
        eprintln!("{}", l.t("auth-empty"));
        return Ok(false);
    };
    match kor_secrets::add_pool_key(&m.id, &secret, &KeychainStore) {
        Ok(Some(slot)) => {
            let mut x = a();
            x.set("slot", slot);
            println!("{}", l.t_args("provider-key-added", &x));
            Ok(true)
        }
        Ok(None) => {
            let mut x = a();
            x.set("max", MAX_POOL_KEYS);
            eprintln!("{}", l.t_args("provider-keys-full", &x));
            Ok(false)
        }
        Err(SecretError::InvalidSecret) => {
            eprintln!(
                "{}",
                l.t_args("auth-key-too-long", &args!("max" => MAX_SECRET_LEN))
            );
            Ok(false)
        }
        Err(e) => Err(e).context("keychain"),
    }
}

fn remove_key(l: &Localizer, reg: &Registry, id: &str, slot: usize) -> anyhow::Result<bool> {
    let Some(m) = keyed(l, reg, id) else {
        return Ok(false);
    };
    let a = args!("provider" => m.id.clone(), "slot" => slot);
    if !(1..=MAX_POOL_KEYS).contains(&slot) || !KeychainStore.delete(&pool_account(&m.id, slot)?)? {
        eprintln!("{}", l.t_args("provider-key-not-found", &a));
        return Ok(false);
    }
    println!("{}", l.t_args("provider-key-removed", &a));
    Ok(true)
}

mod url {
    /// `http(s)://localhost|127.x|[::1]`: no outbound network.
    pub fn is_loopback(raw: &str) -> bool {
        reqwest::Url::parse(raw).is_ok_and(|u| {
            matches!(u.host_str(), Some("localhost" | "[::1]" | "::1"))
                || u.host_str().is_some_and(|h| h.starts_with("127."))
        })
    }
}
