//! The gateway behind `kor proxy` (only in builds that include kor-proxy, see build.rs).

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use kor_i18n::args;
use kor_secrets::{ExposeSecret, KeychainStore};

use super::{ProxyAuthCmd, ProxyCmd};
use crate::Ctx;

fn settings(
    path: Option<&PathBuf>,
) -> anyhow::Result<(kor_proxy::config::Settings, Option<PathBuf>)> {
    let path = path.cloned().or_else(kor_proxy::default_config_path);
    let st = match &path {
        Some(p) => kor_proxy::config::load(p).map_err(|e| anyhow!(e))?,
        None => kor_proxy::config::Settings::default(),
    };
    Ok((st, path))
}

fn addr(host: Option<&str>, port: u16) -> anyhow::Result<SocketAddr> {
    let host = host.unwrap_or("127.0.0.1");
    let ip: IpAddr = if host.eq_ignore_ascii_case("localhost") {
        IpAddr::from([127, 0, 0, 1])
    } else {
        host.parse()
            .with_context(|| format!("--host {host}: an IP address is expected"))?
    };
    Ok(SocketAddr::new(ip, port))
}

fn auth_dir(st: &kor_proxy::config::Settings) -> std::path::PathBuf {
    st.auth_dir
        .clone()
        .unwrap_or_else(kor_proxy::oauth::default_auth_dir)
}

pub fn run(ctx: &Ctx, cmd: ProxyCmd) -> anyhow::Result<bool> {
    match cmd {
        ProxyCmd::Serve {
            port,
            host,
            config,
            no_auth,
            background,
        } => serve(
            ctx,
            port,
            host.as_deref(),
            config.as_ref(),
            no_auth,
            background,
        ),
        ProxyCmd::Status { port, host } => status(ctx, port, host.as_deref()),
        ProxyCmd::Key { rotate, show } => access_key(ctx, rotate, show),
        ProxyCmd::Auth(command) => auth(ctx, command),
    }
}

fn auth(ctx: &Ctx, cmd: ProxyAuthCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let (st, _) = settings(None)?;
    let dir = auth_dir(&st);
    let opts = kor_proxy::oauth::options_from_settings(&st, true);
    match cmd {
        ProxyAuthCmd::Login { provider } => {
            let Some(name) = kor_proxy::oauth::Provider::parse(&provider) else {
                anyhow::bail!(
                    "unknown provider `{provider}`; try claude, codex, codex-device, antigravity, kimi, kimi-ai, devin, copilot"
                );
            };
            let saved = crate::providers::runtime()?
                .block_on(kor_proxy::oauth::login_with(name, &opts, |prompt| {
                    println!("{}", prompt.instructions());
                }))
                .map_err(|e| anyhow!(e.to_string()))?;
            println!(
                "{}",
                l.t_args(
                    "proxy-auth-saved",
                    &kor_i18n::args!("provider" => saved.provider.to_string(), "label" => saved.label, "file" => saved.file_name),
                )
            );
            println!(
                "{}",
                l.t_args(
                    "proxy-auth-dir",
                    &kor_i18n::args!("dir" => dir.display().to_string())
                )
            );
            Ok(true)
        }
        ProxyAuthCmd::List => {
            let accounts = kor_proxy::oauth::list_accounts(&dir);
            if accounts.is_empty() {
                println!("{}", l.t("proxy-auth-none"));
                println!(
                    "{}",
                    l.t_args(
                        "proxy-auth-dir",
                        &kor_i18n::args!("dir" => dir.display().to_string())
                    )
                );
                return Ok(false);
            }
            println!(
                "{}",
                l.t_args(
                    "proxy-auth-dir",
                    &kor_i18n::args!("dir" => dir.display().to_string())
                )
            );
            for entry in accounts {
                println!(
                    "  {}  {}  ({})",
                    entry.file_name, entry.label, entry.provider
                );
            }
            Ok(true)
        }
        ProxyAuthCmd::Refresh { file } => {
            let path = dir.join(&file);
            let saved = crate::providers::runtime()?
                .block_on(kor_proxy::oauth::refresh_file(&path, &opts))
                .map_err(|e| anyhow!(e.to_string()))?;
            println!(
                "{}",
                l.t_args(
                    "proxy-auth-refreshed",
                    &kor_i18n::args!("file" => saved.file_name, "label" => saved.label),
                )
            );
            Ok(true)
        }
        ProxyAuthCmd::Remove { file } => {
            kor_proxy::oauth::remove_account(&dir, &file).map_err(|e| anyhow!(e.to_string()))?;
            println!(
                "{}",
                l.t_args("proxy-auth-removed", &kor_i18n::args!("file" => file)),
            );
            Ok(true)
        }
    }
}

fn serve(
    ctx: &Ctx,
    port: Option<u16>,
    host: Option<&str>,
    config: Option<&PathBuf>,
    no_auth: bool,
    background: bool,
) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let privacy = ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
    let (st, path) = settings(config)?;
    let bind = addr(host.or(st.host.as_deref()), port.unwrap_or(st.port))?;
    let key = if no_auth {
        None
    } else {
        kor_proxy::access_key(&KeychainStore, true).map_err(|e| anyhow!(e))?
    };
    let mut notes = Vec::new();
    let sources = kor_proxy::Sources::user(l, privacy, &mut notes).map_err(|e| anyhow!(e))?;
    for n in notes {
        eprintln!("{n}");
    }
    let access = kor_proxy::Access {
        key: key.clone(),
        no_auth,
    };
    let gw = kor_proxy::Gateway::new(st, path, sources, access, l);
    for w in &gw.warnings {
        eprintln!("{}", l.t_args("proxy-warning", &args!("text" => w.clone())));
    }
    let banner = |addr: SocketAddr| {
        let base = format!("http://{addr}");
        println!(
            "{}",
            l.t_args("proxy-listening", &args!("url" => base.clone()))
        );
        match &key {
            Some(k) => println!(
                "{}",
                l.t_args("proxy-key-line", &args!("key" => kor_proxy::masked(k)))
            ),
            None => println!("{}", l.t("proxy-no-auth")),
        }
        if privacy {
            println!("{}", l.t("proxy-privacy"));
        }
        println!(
            "{}",
            l.t_args(
                "proxy-panel",
                &args!("url" => format!("{base}/management.html"))
            )
        );
    };
    if background {
        // Lives on its own thread and runtime until this process exits.
        // kor-lint: allow(unbounded_channel) one message: the started gateway's address or its error
        let (tx, rx) = std::sync::mpsc::channel();
        let rt = crate::providers::runtime()?;
        std::thread::spawn(move || {
            rt.block_on(async move {
                match kor_proxy::start(Arc::new(gw), bind).await {
                    Ok(running) => {
                        let _ = tx.send(Ok(running.addr));
                        std::future::pending::<()>().await;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            });
        });
        let addr = rx.recv().map_err(|e| anyhow!(e))?.map_err(|e| anyhow!(e))?;
        banner(addr);
        return Ok(true);
    }
    crate::providers::runtime()?.block_on(async move {
        let running = kor_proxy::start(Arc::new(gw), bind)
            .await
            .map_err(|e| anyhow!(e))?;
        banner(running.addr);
        println!("{}", l.t("proxy-stop-hint"));
        let _ = tokio::signal::ctrl_c().await;
        running.stop().await;
        println!("{}", l.t("proxy-stopped"));
        Ok(true)
    })
}

fn status(ctx: &Ctx, port: Option<u16>, host: Option<&str>) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let (st, _) = settings(None)?;
    let bind = addr(host.or(st.host.as_deref()), port.unwrap_or(st.port))?;
    let host = if bind.ip().is_unspecified() {
        IpAddr::from([127, 0, 0, 1])
    } else {
        bind.ip()
    };
    let base = format!("http://{}", SocketAddr::new(host, bind.port()));
    let key = kor_proxy::access_key(&KeychainStore, false).map_err(|e| anyhow!(e))?;
    let usage = crate::providers::runtime()?
        .block_on(kor_proxy::remote_usage(&base, key.as_ref()))
        .map_err(|e| anyhow!(e))?;
    let Some(report) = usage else {
        println!("{}", l.t_args("proxy-not-running", &args!("url" => base)));
        return Ok(false);
    };
    let u = &report["usage"];
    let n = |x: &serde_json::Value| x.as_u64().unwrap_or(0);
    println!(
        "{}",
        l.t_args(
            "proxy-status-head",
            &args!("url" => base, "requests" => n(&u["total_requests"]),
                           "failed" => n(&u["failure_count"]), "tokens" => n(&u["total_tokens"]))
        )
    );
    for c in report["credentials"].as_array().into_iter().flatten() {
        let s = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
        let secs = n(&c["cooling_for_secs"]);
        let cooling = if secs > 0 {
            l.t_args("proxy-status-cooling", &args!("secs" => secs))
        } else {
            String::new()
        };
        println!(
                    "  {}",
                    l.t_args(
                        "proxy-status-cred",
                        &args!("id" => s("id"), "provider" => s("provider"), "key" => s("key"),
                               "ok" => n(&c["success"]), "failed" => n(&c["failed"]), "cooling" => cooling)
                    )
                );
    }
    Ok(true)
}

fn access_key(ctx: &Ctx, rotate: bool, show: bool) -> anyhow::Result<bool> {
    let l = &ctx.l;
    if rotate {
        let k = kor_proxy::rotate_key(&KeychainStore).map_err(|e| anyhow!(e))?;
        println!(
            "{}",
            l.t_args("proxy-key-rotated", &args!("key" => kor_proxy::masked(&k)))
        );
        return Ok(true);
    }
    match kor_proxy::access_key(&KeychainStore, false).map_err(|e| anyhow!(e))? {
        // `--show` is the user's explicit request for their own local gateway key.
        Some(k) if show => println!("{}", k.expose_secret()),
        Some(k) => println!(
            "{}",
            l.t_args("proxy-key-current", &args!("key" => kor_proxy::masked(&k)))
        ),
        None => {
            println!("{}", l.t("proxy-key-none"));
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_parse_and_localhost_maps_to_loopback() {
        assert_eq!(addr(None, 8317).unwrap().to_string(), "127.0.0.1:8317");
        assert_eq!(
            addr(Some("localhost"), 1).unwrap().ip(),
            IpAddr::from([127, 0, 0, 1])
        );
        assert!(addr(Some("::1"), 1).unwrap().ip().is_loopback());
        assert!(addr(Some("not a host"), 1).is_err());
    }
}
