use std::io::{IsTerminal, Read};

use anyhow::Context;
use kor_i18n::{Localizer, args};
use kor_secrets::{
    ExposeSecret, KeySource, KeychainStore, MAX_SECRET_LEN, SecretError, SecretStore, SecretString,
    account_for, env_var_for, mask, resolve_key, store_key,
};

use kor_providers::{ProviderManifest, Registry};

use crate::{AuthCmd, Ctx};

/// Env var for a provider: from its manifest when known, else `<ID>_API_KEY`.
fn env_var(reg: &Registry, id: &str) -> String {
    // Forges (`kor auth login github|gitlab|gitea|bitbucket`, ADR 0040) use their usual token var.
    if let Some(k) = kor_runtime::forge::Kind::for_auth_id(id) {
        return k.token_vars()[0].to_owned();
    }
    reg.get(id)
        .map_or_else(|| env_var_for(id), ProviderManifest::key_env)
}

#[allow(clippy::too_many_lines)]
pub fn run(ctx: &Ctx, cmd: AuthCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let store = KeychainStore;
    let Some(reg) = crate::providers::registry(l) else {
        return Ok(false);
    };
    match cmd {
        AuthCmd::Login { provider, claudeai } => {
            if claudeai {
                return oauth_login(&provider);
            }
            if !check_id(l, &provider) {
                return Ok(false);
            }
            let prompt = l.t_args("auth-prompt", &args!("provider" => provider.clone()));
            let Some(secret) = read_secret(&prompt)? else {
                eprintln!("{}", l.t("auth-empty"));
                return Ok(false);
            };
            match store_key(&provider, &secret, &store) {
                Err(SecretError::InvalidSecret) => {
                    eprintln!(
                        "{}",
                        l.t_args("auth-key-too-long", &args!("max" => MAX_SECRET_LEN))
                    );
                    return Ok(false);
                }
                r => r.context("keychain")?,
            }
            println!(
                "{}",
                l.t_args("auth-stored", &args!("provider" => provider.clone()))
            );
            let var = env_var(&reg, &provider);
            if std::env::var_os(&var).is_some_and(|v| !v.is_empty()) {
                println!("{}", l.t_args("auth-env-shadow", &args!("var" => var)));
            }
            Ok(true)
        }
        AuthCmd::Logout { provider } => {
            if !check_id(l, &provider) {
                return Ok(false);
            }
            let id = args!("provider" => provider.clone());
            if store.delete(&account_for(&provider)?)? {
                println!("{}", l.t_args("auth-removed", &id));
            } else {
                println!("{}", l.t_args("auth-not-found", &id));
            }
            Ok(true)
        }
        AuthCmd::Status { provider } => {
            if let Some(p) = &provider
                && !check_id(l, p)
            {
                return Ok(false);
            }
            let single = provider.is_some();
            let mut any = false;
            if provider.as_deref().unwrap_or("anthropic") == "anthropic"
                && crate::oauth::load_tokens(&store).ok().flatten().is_some()
            {
                any = true;
                println!("anthropic  claude.ai oauth token stored");
            }
            let ids: Vec<String> = provider.map_or_else(
                || {
                    reg.iter()
                        .filter(|m| m.needs_key())
                        .map(|m| m.id.clone())
                        .collect()
                },
                |p| vec![p],
            );
            for id in ids {
                let var = env_var(&reg, &id);
                let resolved = resolve_key(&id, &var, |v| std::env::var(v).ok(), &store)?;
                let a = |extra: (&'static str, String)| {
                    let mut a = args!("provider" => id.clone());
                    a.set(extra.0, extra.1);
                    a
                };
                match resolved {
                    Some(k) => {
                        any = true;
                        let masked = mask(k.secret.expose_secret());
                        let line = match k.source {
                            KeySource::Env(var) => {
                                let mut x = a(("masked", masked));
                                x.set("var", var);
                                l.t_args("auth-status-env", &x)
                            }
                            KeySource::Keychain => {
                                l.t_args("auth-status-keychain", &a(("masked", masked)))
                            }
                        };
                        println!("{line}");
                    }
                    None if single => {
                        println!("{}", l.t_args("auth-status-none", &a(("var", var))));
                    }
                    None => {}
                }
            }
            if !any && !single {
                println!("{}", l.t("auth-status-empty"));
            }
            Ok(true)
        }
    }
}

fn check_id(l: &Localizer, provider: &str) -> bool {
    let ok = kor_core::is_valid_provider_id(provider);
    if !ok {
        eprintln!(
            "{}",
            l.t_args(
                "auth-invalid-provider",
                &args!("provider" => provider.to_owned())
            )
        );
    }
    ok
}

/// Hidden prompt on a terminal; otherwise one bounded read of stdin (for scripts).
fn oauth_login(provider: &str) -> anyhow::Result<bool> {
    if provider != "anthropic" {
        eprintln!("Claude.ai OAuth is only for the anthropic provider");
        return Ok(false);
    }
    crate::providers::runtime()?.block_on(async {
        let (listener, port) = crate::oauth::bind_callback()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        let (verifier, challenge) = crate::oauth::pkce();
        let state = format!("kor{port}");
        println!("{}", crate::oauth::authorize_url(&challenge, &state, port));
        let code = crate::oauth::wait_for_code(listener, &state)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        let tokens = crate::oauth::exchange(&code, &verifier, &state, port)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        crate::oauth::store_tokens(&kor_secrets::KeychainStore, &tokens)
            .map_err(|e| anyhow::anyhow!(e))?;
        println!("stored Claude.ai OAuth token in the OS keychain");
        Ok(true)
    })
}

pub(crate) fn read_secret(prompt: &str) -> anyhow::Result<Option<SecretString>> {
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
