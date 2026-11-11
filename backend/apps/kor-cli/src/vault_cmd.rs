//! `kor vault add|list|remove|import` (spec §6.6). Storage and `.env` parsing live in
//! `kor_secrets::vault`; values go to the OS keychain, or to the encrypted vault file with
//! `[vault] backend = "file"` (ADR 0036), and are never printed. There is deliberately no
//! `get --reveal`: §6.6 keeps secrets out of files and prompts, and the agent only receives them
//! as command environment variables (`[vault.scope]` limits which commands).

use std::path::PathBuf;

use anyhow::Context as _;
use clap::Subcommand;
use kor_config::VaultBackend;
use kor_i18n::{Localizer, args};
use kor_secrets::{SecretError, SecretStore, SecretString, vault};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum VaultCmd {
    /// Store a secret. Without VALUE it is read without echo (or from stdin).
    Add { name: String, value: Option<String> },
    /// Names of the stored secrets (never the values) and which commands get them.
    List,
    /// Delete a secret.
    Remove { name: String },
    /// Store every `KEY=value` of a `.env` file.
    Import {
        #[arg(default_value = ".env")]
        path: PathBuf,
        /// Do not ask before storing.
        #[arg(long)]
        yes: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-vault"))
        .mut_subcommand("add", |s| s.about(l.t("cmd-vault-add")))
        .mut_subcommand("list", |s| s.about(l.t("cmd-vault-list")))
        .mut_subcommand("remove", |s| s.about(l.t("cmd-vault-remove")))
        .mut_subcommand("import", |s| s.about(l.t("cmd-vault-import")))
}

fn report(l: &Localizer, e: &SecretError) -> bool {
    let msg = match e {
        SecretError::InvalidName(n) => l.t_args("vault-bad-name", &args!("name" => n.clone())),
        SecretError::InvalidSecret => l.t("vault-bad-value"),
        other => l.t_args("error-generic", &args!("error" => other.to_string())),
    };
    eprintln!("{msg}");
    false
}

/// The configured store; the vault file asks for its passphrase unless `KOR_VAULT_PASSPHRASE`
/// is set.
fn open(ctx: &Ctx, cfg: &kor_config::Vault) -> anyhow::Result<Box<dyn SecretStore>> {
    let env_set = std::env::var(kor_runtime::secure::PASSPHRASE_ENV).is_ok_and(|v| !v.is_empty());
    let pass = if cfg.backend == VaultBackend::File && !env_set {
        crate::auth::read_secret(&ctx.l.t("vault-passphrase"))?
    } else {
        None
    };
    kor_runtime::secure::vault_store(cfg, pass).map_err(|e| anyhow::anyhow!(e.to_string()))
}

#[allow(clippy::too_many_lines)]
pub fn run(ctx: &Ctx, cmd: VaultCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let cfg = ctx
        .config
        .as_ref()
        .map(|c| c.config.vault.clone())
        .unwrap_or_default();
    let store = open(ctx, &cfg)?;
    let store = store.as_ref();
    match cmd {
        VaultCmd::Add { name, value } => {
            if !vault::valid_name(&name) {
                return Ok(report(l, &SecretError::InvalidName(name)));
            }
            let secret = if let Some(v) = value {
                SecretString::from(v)
            } else {
                let prompt = l.t_args("vault-prompt", &args!("name" => name.clone()));
                let Some(s) = crate::auth::read_secret(&prompt)? else {
                    eprintln!("{}", l.t("vault-bad-value"));
                    return Ok(false);
                };
                s
            };
            match vault::add(store, &name, &secret) {
                Ok(replaced) => {
                    let key = if replaced {
                        "vault-replaced"
                    } else {
                        "vault-stored"
                    };
                    println!("{}", l.t_args(key, &args!("name" => name)));
                    Ok(true)
                }
                Err(e) => Ok(report(l, &e)),
            }
        }
        VaultCmd::List => {
            let names = vault::names(store).context("vault")?;
            println!(
                "{}",
                match cfg.backend {
                    VaultBackend::Keychain => l.t("vault-backend-keychain"),
                    VaultBackend::File => l.t_args(
                        "vault-backend-file",
                        &args!("path" => kor_runtime::secure::vault_file()
                            .map(|p| kor_tools::display_path(&p))
                            .unwrap_or_default())
                    ),
                }
            );
            if names.is_empty() {
                println!("{}", l.t("vault-empty"));
            }
            for n in names {
                let scope = match cfg.scope.get(&n) {
                    None => l.t("vault-scope-all"),
                    Some(p) if p.is_empty() => l.t("vault-scope-none"),
                    Some(p) => p.join(", "),
                };
                println!("  {n}  → {scope}");
            }
            Ok(true)
        }
        VaultCmd::Remove { name } => match vault::remove(store, &name) {
            Ok(true) => {
                println!("{}", l.t_args("vault-removed", &args!("name" => name)));
                Ok(true)
            }
            Ok(false) => {
                eprintln!("{}", l.t_args("vault-not-found", &args!("name" => name)));
                Ok(false)
            }
            Err(e) => Ok(report(l, &e)),
        },
        VaultCmd::Import { path, yes } => {
            let shown = kor_tools::display_path(&path);
            let text = std::fs::read_to_string(&path).with_context(|| shown.clone())?;
            let parsed = vault::parse_dotenv(&text);
            let found: Vec<&str> = parsed.entries.iter().map(|(k, _)| k.as_str()).collect();
            if !parsed.skipped.is_empty() {
                println!(
                    "{}",
                    l.t_args(
                        "vault-import-skipped",
                        &args!("names" => parsed.skipped.join(", "))
                    )
                );
            }
            if found.is_empty() {
                eprintln!("{}", l.t_args("vault-import-none", &args!("path" => shown)));
                return Ok(false);
            }
            let question = l.t_args(
                "vault-import-confirm",
                &args!("count" => found.len(), "path" => shown, "names" => found.join(", ")),
            );
            if !crate::shell_cmd::confirm(l, &question, yes) {
                return Ok(false);
            }
            for (k, v) in &parsed.entries {
                if let Err(e) = vault::add(store, k, v) {
                    return Ok(report(l, &e));
                }
            }
            println!(
                "{}",
                l.t_args("vault-imported", &args!("count" => found.len()))
            );
            Ok(true)
        }
    }
}
