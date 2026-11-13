//! Webmaster and agency commands (spec §29, ADR 0054); logic in `kor_web::site`.
//!
//! - `kor host connect|list|remove|inventory` and one subcommand per task (`ssl`, `php`, `php-ini`,
//!   `subdomain`, `db`, `email`, `backup`, `restore`, `clean`, `cron`, `perms`, `sftp`), each on one
//!   server or `--all`: inventory → plan → `[y/N]` → apply → verify → undo on failure (§29.1);
//! - `kor wp connect|list|remove|inventory|health|doctor|update|staging|integrity|cleanup|scaffold|detect`
//!   (§29.3, §29.4);
//! - `kor redirects build|export|check|slugs` (§29.11);
//! - `kor proposal init|new|revise|templates` (§29.12);
//! - `kor client add|list|log|ticket|check|report` (§29.13);
//! - `kor tr check|slug|kdv|kvkk|efatura|irsaliye|payment|market|cargo|dns|keywords` (§29.14).

#![allow(clippy::doc_markdown, clippy::many_single_char_names)] // product names; `l`, `t`, `p`… as in the other *_cmd files

use std::path::{Path, PathBuf};

use clap::Subcommand;
use kor_i18n::{Localizer, args};
use kor_secrets::{ExposeSecret as _, SecretStore};
use kor_web::site::{self as web, client, cms, docs, doctor, host, proposal, redirects, tr, wp};

use crate::Ctx;

fn fail(l: &Localizer, e: impl std::fmt::Display) -> bool {
    eprintln!(
        "{}",
        l.t_args("error-generic", &args!("error" => e.to_string()))
    );
    false
}

fn base(l: &Localizer) -> Option<PathBuf> {
    web::base().map_err(|e| fail(l, e)).ok()
}

/// The configured vault; the encrypted file backend asks for its passphrase unless it is set.
fn store(ctx: &Ctx) -> Option<Box<dyn SecretStore>> {
    let cfg = ctx
        .config
        .as_ref()
        .map(|c| c.config.vault.clone())
        .unwrap_or_default();
    let env_set = std::env::var(kor_runtime::secure::PASSPHRASE_ENV).is_ok_and(|v| !v.is_empty());
    let pass = if cfg.backend == kor_config::VaultBackend::File && !env_set {
        crate::auth::read_secret(&ctx.l.t("vault-passphrase"))
            .ok()
            .flatten()
    } else {
        None
    };
    kor_runtime::secure::vault_store(&cfg, pass).ok()
}

fn saved(l: &Localizer, files: &[PathBuf]) {
    for f in files {
        println!(
            "{}",
            l.t_args("web-saved", &args!("path" => f.display().to_string()))
        );
    }
}

// ───────────────────────────── kor host ─────────────────────────────

#[derive(Debug, Clone, clap::Args)]
pub struct Target {
    /// Host profile name (`kor host list`).
    pub name: Option<String>,
    /// Every connected server.
    #[arg(long)]
    pub all: bool,
    /// WHM: the cPanel account to act on.
    #[arg(long)]
    pub account: Option<String>,
    /// Apply without asking (the plan is still printed).
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum HostCmd {
    /// Connect a hosting panel or server; the token stays in the vault (`kor vault add NAME`).
    Connect {
        name: String,
        /// cpanel, whm, plesk, directadmin, cyberpanel, hestia, ssh.
        #[arg(long)]
        kind: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        user: Option<String>,
        /// Vault entry with the API token / key.
        #[arg(long)]
        secret: Option<String>,
        /// connections.toml id or user@host for shell tasks.
        #[arg(long)]
        ssh: Option<String>,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        client: Option<String>,
        /// Email for Let's Encrypt.
        #[arg(long)]
        email: Option<String>,
        /// Save without reading the inventory first.
        #[arg(long)]
        no_check: bool,
    },
    /// Connected servers.
    List,
    /// Forget a server (its vault entry stays).
    Remove { name: String },
    /// Read-only inventory: domains, PHP, databases, SSL expiry, disk.
    Inventory {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        json: bool,
    },
    /// Install or renew SSL (Let's Encrypt / AutoSSL).
    Ssl {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        domain: String,
    },
    /// Change a site's PHP version (warns about incompatible WordPress plugins).
    Php {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        domain: String,
        #[arg(long)]
        version: String,
    },
    /// Set a php.ini value for a site.
    PhpIni {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        domain: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
    },
    /// Create a subdomain.
    Subdomain {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        sub: String,
        #[arg(long)]
        domain: String,
        #[arg(long)]
        dir: Option<String>,
    },
    /// Create a database and user (password goes to the vault).
    Db {
        #[command(flatten)]
        t: Target,
        #[arg(long = "name", id = "db_name")]
        db: String,
        #[arg(long)]
        user: String,
    },
    /// Create a mailbox (password goes to the vault).
    Email {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        address: String,
        #[arg(long, default_value_t = 1024)]
        quota: u32,
    },
    /// Take a backup.
    Backup {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        domain: Option<String>,
    },
    /// Restore a backup Kor made (`~/kor-backups/…tar.gz`); the current state is saved first.
    Restore {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        archive: String,
        #[arg(long)]
        domain: String,
    },
    /// Disk clean-up: list candidates, or delete the `--path` ones.
    Clean {
        #[command(flatten)]
        t: Target,
        #[arg(long = "path")]
        paths: Vec<String>,
    },
    /// Add a cron job.
    Cron {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        schedule: String,
        #[arg(long)]
        command: String,
    },
    /// Fix permissions (folders 755, files 644, wp-config.php 640).
    Perms {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        path: String,
    },
    /// Create an SFTP/FTP user (password goes to the vault).
    Sftp {
        #[command(flatten)]
        t: Target,
        #[arg(long)]
        user: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        domain: Option<String>,
    },
}

pub fn localize_host(c: clap::Command, l: &Localizer) -> clap::Command {
    let mut c = c.about(l.t("cmd-host"));
    for s in [
        "connect",
        "list",
        "remove",
        "inventory",
        "ssl",
        "php",
        "php-ini",
        "subdomain",
        "db",
        "email",
        "backup",
        "restore",
        "clean",
        "cron",
        "perms",
        "sftp",
    ] {
        let key = format!("cmd-host-{s}");
        c = c.mut_subcommand(s, |x| x.about(l.t(&key)));
    }
    c
}

fn names(l: &Localizer, base: &Path, t: &Target) -> Vec<String> {
    if t.all {
        return host::list(base).into_iter().map(|(n, _)| n).collect();
    }
    if let Some(n) = &t.name {
        vec![n.clone()]
    } else {
        eprintln!("{}", l.t("web-host-needs-name"));
        Vec::new()
    }
}

fn print_plan(l: &Localizer, host_name: &str, rows: &[host::PlanRow]) {
    println!(
        "{}",
        l.t_args("web-plan-header", &args!("host" => host_name.to_owned()))
    );
    for r in rows {
        let undo = if r.reversible {
            l.t("web-reversible")
        } else {
            l.t("web-irreversible")
        };
        println!("  • {} — {} — {} ({undo})", r.step, r.target, r.impact);
    }
}

fn report_run(l: &Localizer, r: &host::RunReport) -> bool {
    if let Some(e) = &r.error
        && r.applied.is_none()
    {
        return fail(l, e);
    }
    if let Some(a) = &r.applied {
        println!(
            "{}",
            l.t_args("web-applied", &args!("summary" => a.clone()))
        );
    }
    for o in &r.output {
        println!("  {o}");
    }
    if let Some(v) = &r.verify {
        if v.ok {
            println!(
                "{}",
                l.t_args(
                    "web-verify-ok",
                    &args!("url" => v.url.clone(), "status" => v.status, "ms" => v.ms)
                )
            );
        } else {
            println!("{}", l.t_args("web-verify-fail", &args!("url" => v.url.clone(), "status" => v.status, "detail" => v.error.clone().unwrap_or_default())));
        }
    }
    if r.rolled_back {
        println!("{}", l.t("web-rolled-back"));
    }
    if let Some(c) = &r.credential {
        println!(
            "{}",
            l.t_args("web-credential", &args!("name" => c.clone()))
        );
    }
    if let Some(e) = &r.error {
        return fail(l, e);
    }
    r.verify.as_ref().is_none_or(|v| v.ok)
}

fn run_task(ctx: &Ctx, t: &Target, task: &host::Task) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(base) = base(l) else {
        return Ok(false);
    };
    let st = store(ctx);
    let rt = crate::providers::runtime()?;
    let mut ok = true;
    for name in names(l, &base, t) {
        let (prof, panel) = match host::connect(&base, &name, st.as_deref(), t.account.clone()) {
            Ok(x) => x,
            Err(e) => {
                ok = fail(l, format!("{name}: {e}"));
                continue;
            }
        };
        let warnings = rt.block_on(web::tools::php_warnings(l, &base, &name, task));
        let yes = t.yes;
        let lang = l.lang();
        let host_name = name.clone();
        let approve = move |rows: &[host::PlanRow]| {
            let l = Localizer::new(lang);
            print_plan(&l, &host_name, rows);
            crate::shell_cmd::confirm(&l, &l.t("web-approve-q"), yes)
        };
        let kind = format!("host-{}", task.id());
        let r = rt.block_on(host::run(
            l,
            &name,
            &prof,
            panel.as_ref(),
            task.clone(),
            &warnings,
            st.as_deref(),
            &approve,
        ));
        if r.approved && r.applied.is_some() {
            client::log_auto(
                &base,
                prof.client.as_deref(),
                &kind,
                &name,
                r.applied.as_deref().unwrap_or(""),
                r.secs,
            );
        }
        ok &= report_run(l, &r);
    }
    Ok(ok)
}

#[allow(clippy::too_many_lines)]
pub fn host(ctx: &Ctx, cmd: HostCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(base) = base(l) else {
        return Ok(false);
    };
    let task = match cmd {
        HostCmd::Connect {
            name,
            kind,
            url,
            user,
            secret,
            ssh,
            account,
            client,
            email,
            no_check,
        } => {
            let Some(k) = host::Kind::parse(&kind) else {
                return Ok(fail(l, l.t_args("web-bad-kind", &args!("kind" => kind))));
            };
            let mut p = host::HostProfile {
                kind: Some(k),
                url,
                user,
                secret,
                ssh,
                account,
                client,
                email,
            };
            if let Err(e) = p.validate() {
                return Ok(fail(l, e));
            }
            if !no_check {
                let st = store(ctx);
                let secret = match &p.secret {
                    Some(s) => match web::secret(st.as_deref(), s) {
                        Ok(v) => Some(v.expose_secret().to_owned()),
                        Err(e) => return Ok(fail(l, e)),
                    },
                    None => None,
                };
                let panel = match host::ssh_shell(&p)
                    .and_then(|sh| host::open(&name, &p, secret.as_deref(), sh, None))
                {
                    Ok(x) => x,
                    Err(e) => return Ok(fail(l, e)),
                };
                let rt = crate::providers::runtime()?;
                match rt.block_on(host::inventory(panel.as_ref())) {
                    Ok(inv) => println!("{}", host::inventory_markdown(l, &inv, web::now())),
                    Err(e) => {
                        return Ok(fail(
                            l,
                            l.t_args("web-connect-failed", &args!("error" => e)),
                        ));
                    }
                }
            }
            return Ok(match host::save(&base, &name, &p) {
                Ok(path) => {
                    println!(
                        "{}",
                        l.t_args(
                            "web-connected",
                            &args!("name" => name, "path" => path.display().to_string())
                        )
                    );
                    true
                }
                Err(e) => fail(l, e),
            });
        }
        HostCmd::List => {
            let all = host::list(&base);
            if all.is_empty() {
                println!("{}", l.t("web-host-none"));
            }
            for (n, p) in all {
                match p {
                    Ok(p) => println!(
                        "{n}\t{}\t{}\t{}",
                        p.kind().as_str(),
                        p.url.as_deref().or(p.ssh.as_deref()).unwrap_or(""),
                        p.client.as_deref().unwrap_or("")
                    ),
                    Err(e) => println!("{n}\t{e}"),
                }
            }
            return Ok(true);
        }
        HostCmd::Remove { name } => {
            return Ok(match host::remove(&base, &name) {
                Ok(true) => {
                    println!("{}", l.t_args("web-removed", &args!("name" => name)));
                    true
                }
                Ok(false) => fail(l, l.t_args("web-not-found", &args!("name" => name))),
                Err(e) => fail(l, e),
            });
        }
        HostCmd::Inventory { t, json } => {
            let st = store(ctx);
            let rt = crate::providers::runtime()?;
            let mut ok = true;
            for name in names(l, &base, &t) {
                let r = host::connect(&base, &name, st.as_deref(), t.account.clone())
                    .and_then(|(_, p)| rt.block_on(host::inventory(p.as_ref())));
                match r {
                    Ok(inv) if json => println!("{}", serde_json::to_string_pretty(&inv)?),
                    Ok(inv) => println!("{}", host::inventory_markdown(l, &inv, web::now())),
                    Err(e) => ok = fail(l, format!("{name}: {e}")),
                }
            }
            return Ok(ok);
        }
        HostCmd::Ssl { t, domain } => (t, host::Task::Ssl { domain }),
        HostCmd::Php { t, domain, version } => (t, host::Task::Php { domain, version }),
        HostCmd::PhpIni {
            t,
            domain,
            key,
            value,
        } => (t, host::Task::PhpIni { domain, key, value }),
        HostCmd::Subdomain {
            t,
            sub,
            domain,
            dir,
        } => (t, host::Task::Subdomain { sub, domain, dir }),
        HostCmd::Db { t, db, user } => (t, host::Task::Database { name: db, user }),
        HostCmd::Email { t, address, quota } => (
            t,
            host::Task::Email {
                address,
                quota_mb: quota,
            },
        ),
        HostCmd::Backup { t, domain } => (t, host::Task::Backup { domain }),
        HostCmd::Restore { t, archive, domain } => (t, host::Task::Restore { archive, domain }),
        HostCmd::Clean { t, paths } => (t, host::Task::Clean { paths }),
        HostCmd::Cron {
            t,
            schedule,
            command,
        } => (t, host::Task::Cron { schedule, command }),
        HostCmd::Perms { t, path } => (t, host::Task::Perms { path }),
        HostCmd::Sftp {
            t,
            user,
            path,
            domain,
        } => (t, host::Task::Sftp { user, path, domain }),
    };
    run_task(ctx, &task.0, &task.1)
}

// ───────────────────────────── kor wp ─────────────────────────────

#[derive(Debug, Subcommand)]
pub enum WpCmd {
    /// Connect a WordPress site: REST (user + application password in the vault) and/or WP-CLI over SSH.
    Connect {
        name: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        user: Option<String>,
        /// Vault entry with the application password.
        #[arg(long)]
        secret: Option<String>,
        #[arg(long)]
        ssh: Option<String>,
        /// WordPress folder on the server.
        #[arg(long)]
        path: Option<String>,
        /// Host profile the site runs on.
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        client: Option<String>,
    },
    /// Connected sites.
    List,
    /// Forget a site.
    Remove { name: String },
    /// Version, plugins, themes, users, WooCommerce products and orders.
    Inventory {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Site Health tests, page checks, core checksums.
    Health { name: String },
    /// Plugin doctor: conflicts, outdated/abandoned/vulnerable plugins, A–F grades.
    Doctor {
        name: String,
        /// Warn about plugins that may not run on this PHP version.
        #[arg(long)]
        php: Option<String>,
        /// Measure each plugin's cost on the staging copy (needs `kor wp staging create`).
        #[arg(long)]
        measure: bool,
        #[arg(long)]
        json: bool,
    },
    /// Update core/plugins/themes: backup first, smoke test, roll back what breaks.
    Update {
        name: String,
        /// Try on the staging copy first, then ask before live.
        #[arg(long)]
        staging: bool,
        /// Only these slugs.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Staging copy in `<root>/kor-staging`: `create` or `drop`.
    Staging {
        name: String,
        action: String,
        #[arg(long)]
        yes: bool,
    },
    /// Core files vs WordPress.org checksums, plugin checksums, suspicious code.
    Integrity {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// After a compromise: reinstall core/plugins, new salts, password resets (asks first).
    Cleanup {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Skeletons: child-theme, plugin, opencart, prestashop.
    Scaffold {
        kind: String,
        /// Name (`Acme Child`, `Kargo Takip`).
        #[arg(long)]
        name: String,
        /// Folder/slug (`acme-child`).
        #[arg(long)]
        slug: String,
        /// Child theme: the parent theme folder.
        #[arg(long)]
        parent: Option<String>,
        /// Plugin: WooCommerce hooks.
        #[arg(long)]
        woocommerce: bool,
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Recognise the platform of a folder or URL (WordPress, OpenCart, PrestaShop, Ticimax, İdeasoft, T-Soft…).
    Detect { target: String },
}

pub fn localize_wp(c: clap::Command, l: &Localizer) -> clap::Command {
    let mut c = c.about(l.t("cmd-wp"));
    for s in [
        "connect",
        "list",
        "remove",
        "inventory",
        "health",
        "doctor",
        "update",
        "staging",
        "integrity",
        "cleanup",
        "scaffold",
        "detect",
    ] {
        let key = format!("cmd-wp-{s}");
        c = c.mut_subcommand(s, |x| x.about(l.t(&key)));
    }
    c
}

fn print_update(l: &Localizer, r: &wp::UpdateReport) -> bool {
    println!(
        "{}",
        l.t_args("wp-update-target", &args!("url" => r.target.clone()))
    );
    for b in &r.backup {
        println!("  {}", l.t_args("wp-backup", &args!("path" => b.clone())));
    }
    if r.items.is_empty() {
        println!("{}", l.t("wp-nothing-to-update"));
    }
    for i in &r.items {
        println!(
            "  {} {} {} → {}: {} {}",
            i.kind,
            i.slug,
            i.from,
            i.to,
            l.t(&format!("wp-result-{}", i.result)),
            i.note
        );
        for s in i.smoke.iter().filter(|s| !s.ok) {
            println!("    ✗ {} {} HTTP {} {}", s.page, s.url, s.status, s.note);
        }
    }
    r.items.iter().all(|i| i.result == "updated")
}

#[allow(clippy::too_many_lines)]
pub fn wp(ctx: &Ctx, cmd: WpCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(base) = base(l) else {
        return Ok(false);
    };
    let rt = crate::providers::runtime()?;
    let open = |name: &str| web::tools::open_wp(&base, name);
    Ok(match cmd {
        WpCmd::Connect {
            name,
            url,
            user,
            secret,
            ssh,
            path,
            host,
            client,
        } => {
            let mut s = wp::WpSite {
                url,
                user,
                secret,
                ssh,
                path,
                host,
                client,
            };
            if let Err(e) = s.validate() {
                return Ok(fail(l, e));
            }
            if let Err(e) = wp::save(&base, &name, &s) {
                return Ok(fail(l, e));
            }
            match open(&name).map(|w| rt.block_on(w.inventory())) {
                Ok(Ok(inv)) => {
                    println!("{}", l.t_args("wp-connected", &args!("name" => name, "version" => inv.version.unwrap_or_default(), "plugins" => inv.plugins.len())));
                    true
                }
                Ok(Err(e)) | Err(e) => {
                    let _ = wp::remove(&base, &name);
                    fail(l, l.t_args("web-connect-failed", &args!("error" => e)))
                }
            }
        }
        WpCmd::List => {
            for (n, s) in wp::list(&base) {
                match s {
                    Ok(s) => println!(
                        "{n}\t{}\t{}\t{}",
                        s.url,
                        if s.ssh.is_some() { "wp-cli" } else { "rest" },
                        s.client.unwrap_or_default()
                    ),
                    Err(e) => println!("{n}\t{e}"),
                }
            }
            true
        }
        WpCmd::Remove { name } => match wp::remove(&base, &name) {
            Ok(true) => {
                println!("{}", l.t_args("web-removed", &args!("name" => name)));
                true
            }
            Ok(false) => fail(l, l.t_args("web-not-found", &args!("name" => name))),
            Err(e) => fail(l, e),
        },
        WpCmd::Inventory { name, json } => match open(&name).map(|w| rt.block_on(w.inventory())) {
            Ok(Ok(inv)) if json => {
                println!("{}", serde_json::to_string_pretty(&inv)?);
                true
            }
            Ok(Ok(inv)) => {
                println!("{}", wp_inventory_text(l, &inv));
                true
            }
            Ok(Err(e)) | Err(e) => fail(l, e),
        },
        WpCmd::Health { name } => match open(&name) {
            Ok(w) => {
                let checks = rt.block_on(wp::health(&w));
                for c in &checks {
                    println!(
                        "{} {} — {} [{}]",
                        if c.status == "good" {
                            "✓"
                        } else if c.status == "critical" {
                            "✗"
                        } else {
                            "!"
                        },
                        c.id,
                        c.label,
                        c.source
                    );
                }
                checks.iter().all(|c| c.status != "critical")
            }
            Err(e) => fail(l, e),
        },
        WpCmd::Doctor {
            name,
            php,
            measure,
            json,
        } => {
            let w = match open(&name) {
                Ok(w) => w,
                Err(e) => return Ok(fail(l, e)),
            };
            let inv = match rt.block_on(w.inventory()) {
                Ok(i) => i,
                Err(e) => return Ok(fail(l, e)),
            };
            let st = store(ctx);
            let token = web::secret(st.as_deref(), "WPSCAN_API_TOKEN")
                .ok()
                .map(|t| t.expose_secret().to_owned());
            let mut rep = rt.block_on(doctor::diagnose(
                w.http(),
                &name,
                &inv.plugins,
                inv.version.as_deref(),
                token.as_deref().map(|t| ("https://wpscan.com", t)),
                None,
            ));
            if measure {
                let root = w.site.path.clone().unwrap_or_default();
                let st_root = format!("{}/kor-staging", root.trim_end_matches('/'));
                let st_url = format!("{}/kor-staging", w.site.url);
                if let Err(e) = rt.block_on(doctor::measure(&w, &st_root, &st_url, &mut rep, 5)) {
                    fail(l, e);
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                println!("{}", doctor::render(l, &rep));
            }
            if let Some(v) = php {
                let infos: Vec<_> = rep
                    .rows
                    .iter()
                    .map(|r| (r.plugin.clone(), r.org.clone()))
                    .collect();
                for warn in doctor::php_warnings(l, &infos, &v) {
                    println!("- {warn}");
                }
            }
            client::log_auto(
                &base,
                w.site.client.as_deref(),
                "doctor",
                &name,
                &format!("{} plugins", rep.rows.len()),
                0,
            );
            true
        }
        WpCmd::Update {
            name,
            staging,
            only,
            yes,
        } => {
            let w = match open(&name) {
                Ok(w) => w,
                Err(e) => return Ok(fail(l, e)),
            };
            let inv = match rt.block_on(w.inventory()) {
                Ok(i) => i,
                Err(e) => return Ok(fail(l, e)),
            };
            let pending = wp::pending(&inv);
            for (k, s, f, t) in &pending {
                println!("  {k} {s}: {f} → {t}");
            }
            if pending.is_empty() {
                println!("{}", l.t("wp-nothing-to-update"));
                return Ok(true);
            }
            let Some(root) = w.site.path.clone() else {
                return Ok(fail(l, "this site has no WP-CLI path (--ssh/--path)"));
            };
            let start = std::time::Instant::now();
            if staging {
                let url = match rt.block_on(wp::create_staging(&w)) {
                    Ok(u) => u,
                    Err(e) => return Ok(fail(l, e)),
                };
                println!(
                    "{}",
                    l.t_args("wp-staging-ready", &args!("url" => url.clone()))
                );
                let st_root = format!("{}/kor-staging", root.trim_end_matches('/'));
                match rt.block_on(wp::update(&w, &st_root, &url, &inv, &only)) {
                    Ok(r) => {
                        print_update(l, &r);
                    }
                    Err(e) => return Ok(fail(l, e)),
                }
            }
            if !crate::shell_cmd::confirm(l, &l.t("wp-live-q"), yes) {
                return Ok(false);
            }
            match rt.block_on(wp::update(&w, &root, &w.site.url, &inv, &only)) {
                Ok(r) => {
                    let ok = print_update(l, &r);
                    let done = r.items.iter().filter(|i| i.result == "updated").count();
                    client::log_auto(
                        &base,
                        w.site.client.as_deref(),
                        "wp-update",
                        &name,
                        &l.t_args("wp-update-log", &args!("count" => done)),
                        start.elapsed().as_secs(),
                    );
                    ok
                }
                Err(e) => fail(l, e),
            }
        }
        WpCmd::Staging { name, action, yes } => {
            let w = match open(&name) {
                Ok(w) => w,
                Err(e) => return Ok(fail(l, e)),
            };
            match action.as_str() {
                "create" => match rt.block_on(wp::create_staging(&w)) {
                    Ok(u) => {
                        println!("{}", l.t_args("wp-staging-ready", &args!("url" => u)));
                        true
                    }
                    Err(e) => fail(l, e),
                },
                "drop" if crate::shell_cmd::confirm(l, &l.t("wp-staging-drop-q"), yes) => {
                    match rt.block_on(wp::drop_staging(&w)) {
                        Ok(()) => {
                            println!("{}", l.t("wp-staging-dropped"));
                            true
                        }
                        Err(e) => fail(l, e),
                    }
                }
                "drop" => false,
                other => fail(l, format!("create | drop, not `{other}`")),
            }
        }
        WpCmd::Integrity { name, json } => {
            match open(&name).map(|w| (rt.block_on(wp::integrity(&w)), w)) {
                Ok((Ok(i), w)) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&i)?);
                    } else {
                        println!("{}", integrity_text(l, &i));
                    }
                    client::log_auto(
                        &base,
                        w.site.client.as_deref(),
                        "wp-integrity",
                        &name,
                        &format!(
                            "{} modified, {} unknown, {} suspicious",
                            i.modified.len(),
                            i.unknown.len(),
                            i.suspicious.len()
                        ),
                        0,
                    );
                    i.modified.is_empty()
                        && i.unknown.is_empty()
                        && i.suspicious.is_empty()
                        && i.uploads_php.is_empty()
                        && i.plugins.is_empty()
                }
                Ok((Err(e), _)) | Err(e) => fail(l, e),
            }
        }
        WpCmd::Cleanup { name, yes } => {
            let w = match open(&name) {
                Ok(w) => w,
                Err(e) => return Ok(fail(l, e)),
            };
            let found = match rt.block_on(wp::integrity(&w)) {
                Ok(i) => i,
                Err(e) => return Ok(fail(l, e)),
            };
            println!("{}", integrity_text(l, &found));
            println!("{}", l.t("wp-cleanup-plan"));
            if !crate::shell_cmd::confirm(l, &l.t("web-approve-q"), yes) {
                return Ok(false);
            }
            match rt.block_on(wp::cleanup(&w, &found)) {
                Ok(done) => {
                    for d in &done {
                        println!("  ✓ {d}");
                    }
                    println!("{}", l.t("wp-cleanup-next"));
                    client::log_auto(
                        &base,
                        w.site.client.as_deref(),
                        "wp-cleanup",
                        &name,
                        &done.join("; "),
                        0,
                    );
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        WpCmd::Scaffold {
            kind,
            name,
            slug,
            parent,
            woocommerce,
            out,
        } => {
            if !web::valid_name(&slug) {
                return Ok(fail(l, format!("invalid slug `{slug}`")));
            }
            let files = match kind.as_str() {
                "child-theme" => wp::child_theme(
                    parent.as_deref().unwrap_or("twentytwentyfive"),
                    &name,
                    &slug,
                ),
                "plugin" => wp::plugin_skeleton(&name, &slug, woocommerce),
                "opencart" => cms::opencart_module(&slug, &name),
                "prestashop" => cms::prestashop_module(&slug, &name),
                other => {
                    return Ok(fail(
                        l,
                        format!("child-theme | plugin | opencart | prestashop, not `{other}`"),
                    ));
                }
            };
            match wp::write_scaffold(&out, &files) {
                Ok(paths) => {
                    saved(l, &paths);
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        WpCmd::Detect { target } => {
            let found = if target.starts_with("http") {
                match web::http::Http::new().and_then(|h| rt.block_on(h.follow(&target))) {
                    Ok((_, r)) => cms::detect_html(&r.body),
                    Err(e) => return Ok(fail(l, e)),
                }
            } else {
                cms::detect_dir(Path::new(&target))
            };
            if found.is_empty() {
                println!("{}", l.t("wp-detect-none"));
            }
            for d in &found {
                println!(
                    "{} {} [{}]{}",
                    d.platform,
                    d.version.as_deref().unwrap_or(""),
                    d.evidence,
                    if d.modules { "" } else { " (SaaS)" }
                );
            }
            !found.is_empty()
        }
    })
}

fn wp_inventory_text(l: &Localizer, inv: &wp::WpInventory) -> String {
    use std::fmt::Write as _;
    let mut o = l.t_args(
        "wp-inventory",
        &args!("site" => inv.site.clone(), "version" => inv.version.clone().unwrap_or_else(|| "?".into()),
               "plugins" => inv.plugins.len(), "themes" => inv.themes.len(),
               "users" => inv.users.map_or_else(|| "?".into(), |u| u.to_string())),
    );
    o.push('\n');
    if inv.woocommerce {
        let _ = writeln!(o, "{}", l.t_args("wp-woo", &args!("products" => inv.products.map_or_else(|| "?".into(), |u| u.to_string()), "orders" => inv.orders.map_or_else(|| "?".into(), |u| u.to_string()))));
    }
    for p in &inv.plugins {
        let _ = writeln!(
            o,
            "  {} {} {}{}",
            if p.active { "●" } else { "○" },
            p.slug,
            p.version,
            p.update
                .as_deref()
                .map(|u| format!(" → {u}"))
                .unwrap_or_default()
        );
    }
    for n in &inv.notes {
        let _ = writeln!(o, "  ! {n}");
    }
    o
}

fn integrity_text(l: &Localizer, i: &wp::Integrity) -> String {
    use std::fmt::Write as _;
    let mut o = l.t_args("wp-integrity", &args!("version" => i.version.clone(), "modified" => i.modified.len(), "unknown" => i.unknown.len(), "uploads" => i.uploads_php.len(), "plugins" => i.plugins.len()));
    o.push('\n');
    for f in i
        .modified
        .iter()
        .map(|f| ("M", f))
        .chain(i.unknown.iter().map(|f| ("?", f)))
        .chain(i.uploads_php.iter().map(|f| ("U", f)))
    {
        let _ = writeln!(o, "  {} {}", f.0, f.1);
    }
    for p in &i.plugins {
        let _ = writeln!(o, "  P {p}");
    }
    for (f, line, why) in &i.suspicious {
        let _ = writeln!(o, "  ! {f}:{line} {why}");
    }
    let _ = write!(o, "[{}]", i.source);
    o
}

// ───────────────────────────── kor redirects ─────────────────────────────

#[derive(Debug, Subcommand)]
pub enum RedirectsCmd {
    /// Match old URLs to new ones into a CSV map (sitemap, crawl, file or access log on each side).
    Build {
        /// `sitemap:<url>`, `crawl:<url>`, a URL, or a file (URL list, CSV, access log).
        #[arg(long)]
        old: String,
        #[arg(long)]
        new: String,
        /// Lowest similarity accepted (0–1).
        #[arg(long, default_value_t = 0.34)]
        min: f64,
        #[arg(long, default_value_t = 2000)]
        limit: usize,
        #[arg(long, default_value = "redirects.csv")]
        out: PathBuf,
    },
    /// Server snippets from a (corrected) map: apache, nginx, cloudflare, redirection, csv.
    Export {
        map: PathBuf,
        #[arg(long)]
        format: String,
        /// Old domain (Cloudflare needs it).
        #[arg(long, default_value = "")]
        host: String,
        /// Point chained redirects straight at their final target.
        #[arg(long)]
        collapse: bool,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// After launch: request every old URL and report 404s, chains and loops.
    Check {
        map: PathBuf,
        #[arg(long)]
        base: String,
    },
    /// Redirects for URLs with Turkish letters to ASCII slugs.
    Slugs {
        source: String,
        #[arg(long, default_value = "redirects-slugs.csv")]
        out: PathBuf,
    },
}

pub fn localize_redirects(c: clap::Command, l: &Localizer) -> clap::Command {
    let mut c = c.about(l.t("cmd-redirects"));
    for s in ["build", "export", "check", "slugs"] {
        let key = format!("cmd-redirects-{s}");
        c = c.mut_subcommand(s, |x| x.about(l.t(&key)));
    }
    c
}

fn print_issues(l: &Localizer, rows: &[redirects::Row]) {
    let i = redirects::issues(rows);
    println!("{}", l.t_args("redirects-issues", &args!("chains" => i.chains.len(), "loops" => i.loops.len(), "unmatched" => i.unmatched.len())));
    for c in i.chains.iter().chain(&i.loops) {
        println!("  {}", c.join(" → "));
    }
}

#[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
pub fn redirects_run(ctx: &Ctx, cmd: RedirectsCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let rt = crate::providers::runtime()?;
    let http = match web::http::Http::new() {
        Ok(h) => h,
        Err(e) => return Ok(fail(l, e)),
    };
    Ok(match cmd {
        RedirectsCmd::Build {
            old,
            new,
            min,
            limit,
            out,
        } => {
            let r = rt.block_on(async {
                let (o, mut ot) = redirects::collect(&http, &old, limit).await?;
                let (n, nt) = redirects::collect(&http, &new, limit).await?;
                ot.extend(nt);
                Ok::<_, String>((o, n, ot))
            });
            match r {
                Ok((o, n, titles)) => {
                    let rows = redirects::build(&o, &n, &titles, min);
                    println!(
                        "{}",
                        l.t_args(
                            "redirects-built",
                            &args!("old" => o.len(), "new" => n.len(), "rows" => rows.len())
                        )
                    );
                    print_issues(l, &rows);
                    match std::fs::write(&out, redirects::to_csv(&rows)) {
                        Ok(()) => {
                            saved(l, &[out]);
                            true
                        }
                        Err(e) => fail(l, e),
                    }
                }
                Err(e) => fail(l, e),
            }
        }
        RedirectsCmd::Export {
            map,
            format,
            host,
            collapse,
            out,
        } => {
            let Some(fmt) = redirects::Format::parse(&format) else {
                return Ok(fail(
                    l,
                    format!("apache | nginx | cloudflare | redirection | csv, not `{format}`"),
                ));
            };
            let rows = match std::fs::read_to_string(&map)
                .map_err(|e| e.to_string())
                .and_then(|t| redirects::from_csv(&t))
            {
                Ok(r) => r,
                Err(e) => return Ok(fail(l, e)),
            };
            print_issues(l, &rows);
            let rows = if collapse {
                redirects::collapse(&rows)
            } else {
                rows
            };
            let text = redirects::render(&rows, fmt, &host);
            if let Some(p) = out {
                match std::fs::write(&p, text) {
                    Ok(()) => {
                        saved(l, &[p]);
                        true
                    }
                    Err(e) => fail(l, e),
                }
            } else {
                print!("{text}");
                true
            }
        }
        RedirectsCmd::Check { map, base } => {
            let rows = match std::fs::read_to_string(&map)
                .map_err(|e| e.to_string())
                .and_then(|t| redirects::from_csv(&t))
            {
                Ok(r) => r,
                Err(e) => return Ok(fail(l, e)),
            };
            let checked = rt.block_on(redirects::verify(&http, &base, &rows));
            for c in checked.iter().filter(|c| c.verdict != "ok") {
                println!(
                    "{} {} → {} (HTTP {}, {} hops)",
                    l.t(&format!("redirects-verdict-{}", c.verdict)),
                    c.old,
                    c.final_url,
                    c.status,
                    c.hops
                );
            }
            let bad = checked.iter().filter(|c| c.verdict != "ok").count();
            println!(
                "{}",
                l.t_args(
                    "redirects-checked",
                    &args!("total" => checked.len(), "bad" => bad)
                )
            );
            bad == 0
        }
        RedirectsCmd::Slugs { source, out } => {
            match rt.block_on(redirects::collect(&http, &source, 5000)) {
                Ok((paths, _)) => {
                    let rows: Vec<redirects::Row> = tr::slug_redirects(&paths)
                        .into_iter()
                        .map(|(old, new)| redirects::Row {
                            old,
                            new,
                            score: 1.0,
                            method: "slug".into(),
                        })
                        .collect();
                    println!(
                        "{}",
                        l.t_args("redirects-slugs", &args!("count" => rows.len()))
                    );
                    match std::fs::write(&out, redirects::to_csv(&rows)) {
                        Ok(()) => {
                            saved(l, &[out]);
                            true
                        }
                        Err(e) => fail(l, e),
                    }
                }
                Err(e) => fail(l, e),
            }
        }
    })
}

// ───────────────────────────── kor proposal ─────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ProposalCmd {
    /// Write a starter business.toml (rates, company, payment plan).
    Init,
    /// Proposal (and with --contracts: service contract, KVKK annex, NDA) from a job description and/or items.
    New {
        #[arg(long)]
        client: String,
        #[arg(long)]
        project: String,
        /// Job description, meeting notes or client email (file, or - for stdin).
        #[arg(long)]
        from: Option<PathBuf>,
        /// `Title=hours`, repeatable.
        #[arg(long = "item")]
        items: Vec<String>,
        /// Package name from business.toml, repeatable.
        #[arg(long = "package")]
        packages: Vec<String>,
        #[arg(long, default_value = "tr")]
        lang: String,
        #[arg(long)]
        contracts: bool,
        /// md, html, pdf, docx (comma-separated).
        #[arg(long, value_delimiter = ',', default_value = "html,pdf")]
        format: Vec<String>,
        #[arg(long, default_value_t = 15)]
        valid_days: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// New version from the client's reply (`çıkar: …`, `ekle: …=saat`, `İş = saat`, `not: …`).
    Revise {
        dir: PathBuf,
        #[arg(long)]
        reply: PathBuf,
        #[arg(long)]
        contracts: bool,
        #[arg(long, value_delimiter = ',', default_value = "html,pdf")]
        format: Vec<String>,
    },
    /// Where the editable templates are (created on first use).
    Templates,
}

pub fn localize_proposal(c: clap::Command, l: &Localizer) -> clap::Command {
    let mut c = c.about(l.t("cmd-proposal"));
    for s in ["init", "new", "revise", "templates"] {
        let key = format!("cmd-proposal-{s}");
        c = c.mut_subcommand(s, |x| x.about(l.t(&key)));
    }
    c
}

fn read_input(p: &Path) -> std::io::Result<String> {
    if p == Path::new("-") {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(p)
    }
}

#[allow(clippy::too_many_lines)]
pub fn proposal_run(ctx: &Ctx, cmd: ProposalCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(base) = base(l) else {
        return Ok(false);
    };
    let biz = match proposal::business(&base) {
        Ok(b) => b,
        Err(e) => return Ok(fail(l, e)),
    };
    Ok(match cmd {
        ProposalCmd::Init => match proposal::init_business(&base) {
            Ok(p) => {
                saved(l, &[p]);
                true
            }
            Err(e) => fail(l, e),
        },
        ProposalCmd::New {
            client,
            project,
            from,
            items,
            packages,
            lang,
            contracts,
            format,
            valid_days,
            out,
        } => {
            let Some(lang) = proposal::Lang::parse(&lang) else {
                return Ok(fail(l, "--lang tr | en"));
            };
            let mut list = Vec::new();
            if let Some(f) = from {
                match read_input(&f) {
                    Ok(t) => list.extend(proposal::scope_from_text(&t)),
                    Err(e) => return Ok(fail(l, e)),
                }
            }
            for i in &items {
                match proposal::parse_item(i) {
                    Ok(x) => list.push(x),
                    Err(e) => return Ok(fail(l, e)),
                }
            }
            for p in &packages {
                match biz.packages.iter().find(|x| &x.name == p) {
                    Some(x) => list.push(proposal::Item {
                        title: x.name.clone(),
                        hours: x.hours,
                        estimated: false,
                        price: Some(x.price),
                    }),
                    None => {
                        return Ok(fail(
                            l,
                            l.t_args("proposal-no-package", &args!("name" => p.clone())),
                        ));
                    }
                }
            }
            if list.is_empty() {
                return Ok(fail(l, l.t("proposal-empty")));
            }
            if biz.hourly_rate <= 0.0 && list.iter().any(|i| i.price.is_none()) {
                eprintln!("{}", l.t("proposal-no-rate"));
            }
            let p = proposal::Proposal {
                client: client.clone(),
                project,
                lang,
                items: list,
                version: 1,
                date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                valid_days,
                notes: Vec::new(),
            };
            let dir = out.unwrap_or_else(|| PathBuf::from(format!("teklif-{}", tr::slug(&client))));
            match proposal::write(&base, &dir, &p, &biz, contracts, &format) {
                Ok(files) => {
                    let t = proposal::totals(&p, &biz);
                    println!("{}", l.t_args("proposal-total", &args!("hours" => t.hours, "net" => tr::try_format(t.net), "gross" => tr::try_format(t.gross))));
                    if t.estimated {
                        println!("{}", l.t("proposal-estimated"));
                    }
                    saved(l, &files);
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        ProposalCmd::Revise {
            dir,
            reply,
            contracts,
            format,
        } => {
            let mut p = match proposal::load_state(&dir) {
                Ok(p) => p,
                Err(e) => return Ok(fail(l, e)),
            };
            let text = match read_input(&reply) {
                Ok(t) => t,
                Err(e) => return Ok(fail(l, e)),
            };
            let changes = proposal::revise(&mut p, &text);
            if changes.is_empty() {
                println!("{}", l.t("proposal-no-changes"));
                return Ok(false);
            }
            for c in &changes {
                println!("  {c}");
            }
            match proposal::write(&base, &dir, &p, &biz, contracts, &format) {
                Ok(files) => {
                    saved(l, &files);
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        ProposalCmd::Templates => {
            for k in proposal::KINDS {
                for lang in [proposal::Lang::Tr, proposal::Lang::En] {
                    if let Err(e) = proposal::template(&base, k, lang) {
                        return Ok(fail(l, e));
                    }
                }
            }
            println!("{}", proposal::templates_dir(&base).display());
            true
        }
    })
}

// ───────────────────────────── kor client ─────────────────────────────

#[derive(Debug, Subcommand)]
pub enum ClientCmd {
    /// Create or update a client workspace.
    Add {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        telegram: Option<String>,
        #[arg(long = "site")]
        sites: Vec<String>,
        #[arg(long = "host")]
        hosts: Vec<String>,
        #[arg(long = "url")]
        urls: Vec<String>,
        /// Hourly rate (TL, without KDV) for this client.
        #[arg(long)]
        rate: Option<f64>,
    },
    /// Client workspaces.
    List,
    /// Log manual work time.
    Log {
        id: String,
        #[arg(long)]
        hours: f64,
        #[arg(long)]
        note: String,
    },
    /// Tickets: add `<title>`, close `<n>`, list.
    Ticket {
        id: String,
        action: String,
        value: Option<String>,
    },
    /// Take an uptime/SSL sample of every site (schedule it with a routine).
    Check { id: String },
    /// Monthly report: work, uptime/SSL, updates, security, SEO, tickets, time sheet; Telegram and email drafts.
    Report {
        id: String,
        /// YYYY-MM (default: this month).
        #[arg(long)]
        month: Option<String>,
        #[arg(long, value_delimiter = ',', default_value = "html,pdf")]
        format: Vec<String>,
    },
}

pub fn localize_client(c: clap::Command, l: &Localizer) -> clap::Command {
    let mut c = c.about(l.t("cmd-client"));
    for s in ["add", "list", "log", "ticket", "check", "report"] {
        let key = format!("cmd-client-{s}");
        c = c.mut_subcommand(s, |x| x.about(l.t(&key)));
    }
    c
}

#[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
pub fn client_run(ctx: &Ctx, cmd: ClientCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let Some(base) = base(l) else {
        return Ok(false);
    };
    let rt = crate::providers::runtime()?;
    Ok(match cmd {
        ClientCmd::Add {
            id,
            name,
            email,
            telegram,
            sites,
            hosts,
            urls,
            rate,
        } => {
            let mut c = client::load(&base, &id).unwrap_or_default();
            if let Some(v) = name {
                c.name = v;
            }
            if let Some(v) = email {
                c.email = v;
            }
            if let Some(v) = telegram {
                c.telegram = v;
            }
            if let Some(v) = rate {
                c.rate = v;
            }
            for (list, add) in [
                (&mut c.sites, sites),
                (&mut c.hosts, hosts),
                (&mut c.urls, urls),
            ] {
                for x in add {
                    if !list.contains(&x) {
                        list.push(x);
                    }
                }
            }
            match client::save(&base, &id, &c) {
                Ok(p) => {
                    saved(l, &[p]);
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        ClientCmd::List => {
            for id in client::list(&base) {
                let c = client::load(&base, &id).unwrap_or_default();
                println!(
                    "{id}\t{}\t{} site(s)\t{} server(s)",
                    c.name,
                    c.sites.len() + c.urls.len(),
                    c.hosts.len()
                );
            }
            true
        }
        ClientCmd::Log { id, hours, note } => {
            if client::load(&base, &id).is_err() || !(0.0..=24.0).contains(&hours) {
                return Ok(fail(l, l.t_args("web-not-found", &args!("name" => id))));
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let secs = (hours * 3600.0).round() as u64;
            match client::log(
                &base,
                &id,
                &client::Entry {
                    ts: web::now(),
                    kind: "manual".into(),
                    target: String::new(),
                    summary: note,
                    secs,
                    source: "manual".into(),
                },
            ) {
                Ok(()) => true,
                Err(e) => fail(l, e),
            }
        }
        ClientCmd::Ticket { id, action, value } => match (action.as_str(), value) {
            ("add", Some(t)) => match client::ticket_add(&base, &id, &t) {
                Ok(t) => {
                    println!("#{} {}", t.id, t.title);
                    true
                }
                Err(e) => fail(l, e),
            },
            ("close", Some(n)) => match n
                .parse()
                .map_err(|_| "ticket number".to_owned())
                .and_then(|n| client::ticket_close(&base, &id, n))
            {
                Ok(true) => true,
                Ok(false) => fail(l, l.t_args("web-not-found", &args!("name" => n))),
                Err(e) => fail(l, e),
            },
            ("list", _) => {
                for t in client::tickets(&base, &id) {
                    println!("#{} {} {}", t.id, if t.open { "○" } else { "●" }, t.title);
                }
                true
            }
            _ => fail(l, "add <title> | close <n> | list"),
        },
        ClientCmd::Check { id } => {
            match web::http::Http::new().and_then(|h| rt.block_on(client::check(&base, &id, &h))) {
                Ok(samples) => {
                    for s in &samples {
                        println!(
                            "{} {} HTTP {} {} ms{}",
                            if s.up { "✓" } else { "✗" },
                            s.url,
                            s.status,
                            s.ms,
                            s.ssl_days
                                .map(|d| format!(" · SSL {d} d"))
                                .unwrap_or_default()
                        );
                    }
                    samples.iter().all(|s| s.up)
                }
                Err(e) => fail(l, e),
            }
        }
        ClientCmd::Report { id, month, format } => {
            let month = month.unwrap_or_else(|| chrono::Local::now().format("%Y-%m").to_string());
            let live = match web::http::Http::new()
                .and_then(|h| rt.block_on(client::check(&base, &id, &h)))
            {
                Ok(s) => s,
                Err(e) => return Ok(fail(l, e)),
            };
            let biz = proposal::business(&base).unwrap_or_default();
            match client::report(l, &base, &id, &month, &live, biz.hourly_rate)
                .and_then(|r| client::write_report(&base, &id, &month, &r, &biz.accent, &format))
            {
                Ok(files) => {
                    saved(l, &files);
                    true
                }
                Err(e) => fail(l, e),
            }
        }
    })
}

// ───────────────────────────── kor tr ─────────────────────────────

#[derive(Debug, Subcommand)]
pub enum TrCmd {
    /// KVKK text, cookie notice, VERBİS, Turkish locale, payment/cargo checks of a site.
    Check {
        url: String,
        /// Also look at this connected WordPress site's plugins.
        #[arg(long)]
        site: Option<String>,
    },
    /// Turkish-aware URL slug.
    Slug { text: String },
    /// KDV: net → gross, or with --gross the other way.
    Kdv {
        amount: String,
        #[arg(long, default_value_t = 20)]
        rate: u32,
        #[arg(long)]
        gross: bool,
    },
    /// KVKK disclosure text and cookie policy from business.toml and options.
    Kvkk {
        #[arg(long = "data")]
        data: Vec<String>,
        #[arg(long = "purpose")]
        purposes: Vec<String>,
        #[arg(long = "recipient")]
        recipients: Vec<String>,
        #[arg(long = "abroad")]
        abroad: Vec<String>,
        /// `name|purpose|duration`, repeatable.
        #[arg(long = "cookie")]
        cookies: Vec<String>,
        #[arg(long, value_delimiter = ',', default_value = "html")]
        format: Vec<String>,
        #[arg(long, default_value = "kvkk")]
        out: PathBuf,
    },
    /// UBL-TR e-Fatura/e-Arşiv XML from a TOML/JSON invoice; --send posts it to the integrator.
    Efatura {
        file: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        send: bool,
    },
    /// UBL-TR e-İrsaliye XML from a TOML/JSON despatch; --send posts it to the integrator.
    Irsaliye {
        file: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        send: bool,
    },
    /// Payment integration templates: iyzico, paytr, param, sipay.
    Payment {
        provider: String,
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Product/order counts from Trendyol, Hepsiburada and N11 (credentials in business.toml + vault).
    Market,
    /// Yurtiçi Kargo shipment status by cargo key.
    Cargo { key: String },
    /// Domain Name API: domain details and DNS zone; --set / --ns change them after a yes.
    Dns {
        domain: String,
        /// `name TYPE value[,value]` record set to write, e.g. `@ TXT v=spf1 -all`.
        #[arg(long)]
        set: Option<String>,
        #[arg(long, default_value_t = 3600)]
        ttl: u64,
        /// Name servers to set, comma separated.
        #[arg(long, value_delimiter = ',')]
        ns: Vec<String>,
        #[arg(long, short)]
        yes: bool,
    },
    /// Most frequent Turkish keyword stems of a file or page.
    Keywords {
        source: String,
        #[arg(long, default_value_t = 20)]
        top: usize,
    },
}

pub fn localize_tr(c: clap::Command, l: &Localizer) -> clap::Command {
    let mut c = c.about(l.t("cmd-tr"));
    for s in [
        "check", "slug", "kdv", "kvkk", "efatura", "irsaliye", "payment", "market", "cargo", "dns",
        "keywords",
    ] {
        let key = format!("cmd-tr-{s}");
        c = c.mut_subcommand(s, |x| x.about(l.t(&key)));
    }
    c
}

fn read_doc<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    let t = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if p.extension().is_some_and(|x| x == "json") {
        serde_json::from_str(&t).map_err(|e| e.to_string())
    } else {
        toml::from_str(&t).map_err(|e| e.to_string())
    }
}

fn send_ubl(ctx: &Ctx, xml: &str) -> Result<String, String> {
    let base = web::base()?;
    let biz = proposal::business(&base)?;
    if biz.efatura.url.is_empty() || biz.efatura.secret.is_empty() {
        return Err(ctx.l.t("tr-efatura-no-integrator"));
    }
    let st = store(ctx);
    let token = web::secret(st.as_deref(), &biz.efatura.secret)?;
    let rt = crate::providers::runtime().map_err(|e| e.to_string())?;
    rt.block_on(tr::efatura::send(
        &web::http::Http::new()?,
        &biz.efatura.url,
        token.expose_secret(),
        xml,
    ))
}

/// `kor tr dns`: show details and zone, or change one record set / the name servers after a yes.
fn dns(
    ctx: &Ctx,
    rt: &tokio::runtime::Runtime,
    domain: &str,
    set: Option<&str>,
    ttl: u64,
    ns: &[String],
    yes: bool,
) -> bool {
    let l = &ctx.l;
    let Ok(b) = web::base() else { return false };
    let biz = proposal::business(&b).unwrap_or_default();
    if biz.dna.id.is_empty() || biz.dna.key.is_empty() {
        println!("{}", l.t("tr-dns-none"));
        return false;
    }
    let st = store(ctx);
    let key = match web::secret(st.as_deref(), &biz.dna.key) {
        Ok(k) => k,
        Err(e) => return fail(l, e),
    };
    let http = match web::http::Http::new() {
        Ok(h) => h,
        Err(e) => return fail(l, e),
    };
    let d = tr::registrar::Dna {
        http: &http,
        base: tr::registrar::DNA,
        reseller: &biz.dna.id,
        key: key.expose_secret(),
    };
    let change = |what: String| {
        crate::shell_cmd::confirm(
            l,
            &l.t_args(
                "tr-dns-confirm",
                &args!("change" => what, "domain" => domain.to_owned()),
            ),
            yes,
        )
    };
    if let Some(spec) = set {
        let mut parts = spec.splitn(3, ' ');
        let (Some(name), Some(kind), Some(values)) = (parts.next(), parts.next(), parts.next())
        else {
            return fail(l, format!("--set \"name TYPE value\", not `{spec}`"));
        };
        let values: Vec<String> = values.split(',').map(|v| v.trim().to_owned()).collect();
        if !change(format!("{name} {kind} {}", values.join(", "))) {
            return false;
        }
        return match rt.block_on(d.set_records(domain, name, kind, &values, ttl)) {
            Ok(()) => {
                println!("{}", l.t("tr-dns-done"));
                true
            }
            Err(e) => fail(l, e),
        };
    }
    if !ns.is_empty() {
        if !change(format!("NS {}", ns.join(", "))) {
            return false;
        }
        return match rt.block_on(d.set_name_servers(domain, ns)) {
            Ok(()) => {
                println!("{}", l.t("tr-dns-done"));
                true
            }
            Err(e) => fail(l, e),
        };
    }
    match rt.block_on(async { Ok::<_, String>((d.info(domain).await?, d.zone(domain).await?)) }) {
        Ok((i, zone)) => {
            println!(
                "{}  {}  {}  NS: {}",
                i.domain,
                i.status,
                i.expiration,
                i.name_servers.join(", ")
            );
            for r in zone {
                println!("{:<24} {:<6} {:>6}  {}", r.name, r.kind, r.ttl, r.content);
            }
            true
        }
        Err(e) => fail(l, e),
    }
}

#[allow(clippy::too_many_lines)]
pub fn tr_run(ctx: &Ctx, cmd: TrCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let rt = crate::providers::runtime()?;
    Ok(match cmd {
        TrCmd::Check { url, site } => {
            let plugins: Vec<String> = match (&site, web::base()) {
                (Some(s), Ok(b)) => web::tools::open_wp(&b, s)
                    .ok()
                    .and_then(|w| rt.block_on(w.inventory()).ok())
                    .map(|i| i.plugins.into_iter().map(|p| p.slug).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            match web::http::Http::new().and_then(|h| rt.block_on(tr::check(&h, &url, &plugins))) {
                Ok(r) => {
                    println!("{}", tr::render(l, &r));
                    if let (Some(s), Ok(b)) = (&site, web::base()) {
                        let c = wp::load(&b, s).ok().and_then(|w| w.client);
                        client::log_auto(
                            &b,
                            c.as_deref(),
                            "tr-check",
                            &url,
                            &format!("{} checks", r.items.len()),
                            0,
                        );
                    }
                    !r.items.iter().any(|i| i.status == "fail")
                }
                Err(e) => fail(l, e),
            }
        }
        TrCmd::Slug { text } => {
            println!("{}", tr::slug(&text));
            true
        }
        TrCmd::Kdv {
            amount,
            rate,
            gross,
        } => match tr::parse_try(&amount) {
            Some(k) => {
                let (net, tax, total) = if gross {
                    let (n, t) = tr::kdv_from_gross(k, rate);
                    (n, t, k)
                } else {
                    let (t, g) = tr::kdv(k, rate);
                    (k, t, g)
                };
                println!("{}", l.t_args("tr-kdv", &args!("net" => tr::try_format(net), "rate" => rate, "kdv" => tr::try_format(tax), "gross" => tr::try_format(total))));
                true
            }
            None => fail(l, format!("`{amount}`")),
        },
        TrCmd::Kvkk {
            data,
            purposes,
            recipients,
            abroad,
            cookies,
            format,
            out,
        } => {
            let biz = web::base()
                .and_then(|b| proposal::business(&b))
                .unwrap_or_default();
            let input = tr::KvkkInput {
                company: biz.name.clone(),
                address: format!("{} {}", biz.address, biz.city).trim().to_owned(),
                email: biz.email.clone(),
                website: biz.website.clone(),
                data,
                purposes,
                recipients,
                abroad,
                cookies: cookies
                    .iter()
                    .map(|c| {
                        let mut p = c.splitn(3, '|').map(str::to_owned);
                        (
                            p.next().unwrap_or_default(),
                            p.next().unwrap_or_default(),
                            p.next().unwrap_or_default(),
                        )
                    })
                    .collect(),
            };
            let (disclosure, cookie) = tr::kvkk_text(&input);
            let mut files = Vec::new();
            for (stem, md, title) in [
                ("aydinlatma-metni", disclosure, "Aydınlatma Metni"),
                ("cerez-politikasi", cookie, "Çerez Politikası"),
            ] {
                match docs::write_all(&out, stem, &md, title, &biz.accent, &format) {
                    Ok(f) => files.extend(f),
                    Err(e) => return Ok(fail(l, e)),
                }
            }
            saved(l, &files);
            println!("{}", l.t("tr-kvkk-review"));
            true
        }
        TrCmd::Efatura { file, out, send } => {
            let inv: tr::efatura::Invoice = match read_doc(&file) {
                Ok(i) => i,
                Err(e) => return Ok(fail(l, e)),
            };
            let xml = match tr::efatura::uuid4().and_then(|u| tr::efatura::invoice_xml(&inv, &u)) {
                Ok(x) => x,
                Err(e) => return Ok(fail(l, e)),
            };
            let out = out.unwrap_or_else(|| file.with_extension("xml"));
            if let Err(e) = std::fs::write(&out, &xml) {
                return Ok(fail(l, e));
            }
            saved(l, &[out]);
            if send {
                return Ok(match send_ubl(ctx, &xml) {
                    Ok(r) => {
                        println!("{r}");
                        true
                    }
                    Err(e) => fail(l, e),
                });
            }
            true
        }
        TrCmd::Irsaliye { file, out, send } => {
            let d: tr::efatura::Despatch = match read_doc(&file) {
                Ok(i) => i,
                Err(e) => return Ok(fail(l, e)),
            };
            let xml = match tr::efatura::uuid4().and_then(|u| tr::efatura::despatch_xml(&d, &u)) {
                Ok(x) => x,
                Err(e) => return Ok(fail(l, e)),
            };
            let out = out.unwrap_or_else(|| file.with_extension("xml"));
            if let Err(e) = std::fs::write(&out, &xml) {
                return Ok(fail(l, e));
            }
            saved(l, &[out]);
            if send {
                return Ok(match send_ubl(ctx, &xml) {
                    Ok(r) => {
                        println!("{r}");
                        true
                    }
                    Err(e) => fail(l, e),
                });
            }
            true
        }
        TrCmd::Payment { provider, out } => match tr::templates::payment(&provider) {
            Some(files) => match wp::write_scaffold(&out, &files) {
                Ok(p) => {
                    saved(l, &p);
                    true
                }
                Err(e) => fail(l, e),
            },
            None => fail(
                l,
                format!("{}, not `{provider}`", tr::templates::PROVIDERS.join(" | ")),
            ),
        },
        TrCmd::Market => {
            let Ok(b) = web::base() else { return Ok(false) };
            let biz = proposal::business(&b).unwrap_or_default();
            let st = store(ctx);
            let http = match web::http::Http::new() {
                Ok(h) => h,
                Err(e) => return Ok(fail(l, e)),
            };
            let get = |n: &str| web::secret(st.as_deref(), n).map(|s| s.expose_secret().to_owned());
            let mut any = false;
            if !biz.trendyol.id.is_empty() {
                any = true;
                match get(&biz.trendyol.key)
                    .and_then(|k| Ok((k, get(&biz.trendyol.secret)?)))
                    .and_then(|(k, s)| {
                        rt.block_on(tr::market::trendyol(
                            &http,
                            tr::market::TRENDYOL,
                            &biz.trendyol.id,
                            &k,
                            &s,
                        ))
                    }) {
                    Ok(c) => println!("{}: {:?} / {:?}", c.market, c.products, c.orders),
                    Err(e) => {
                        fail(l, e);
                    }
                }
            }
            if !biz.hepsiburada.id.is_empty() {
                any = true;
                match get(&biz.hepsiburada.key)
                    .and_then(|k| Ok((k, get(&biz.hepsiburada.secret)?)))
                    .and_then(|(k, s)| {
                        rt.block_on(tr::market::hepsiburada(
                            &http,
                            tr::market::HEPSIBURADA,
                            &biz.hepsiburada.id,
                            &k,
                            &s,
                        ))
                    }) {
                    Ok(c) => println!("{}: {:?}", c.market, c.products),
                    Err(e) => {
                        fail(l, e);
                    }
                }
            }
            if !biz.n11.key.is_empty() {
                any = true;
                match get(&biz.n11.key)
                    .and_then(|k| Ok((k, get(&biz.n11.secret)?)))
                    .and_then(|(k, s)| rt.block_on(tr::market::n11(&http, tr::market::N11, &k, &s)))
                {
                    Ok(c) => println!("{}: {:?} / {:?}", c.market, c.products, c.orders),
                    Err(e) => {
                        fail(l, e);
                    }
                }
            }
            if !any {
                println!("{}", l.t("tr-market-none"));
            }
            any
        }
        TrCmd::Cargo { key } => {
            let Ok(b) = web::base() else { return Ok(false) };
            let biz = proposal::business(&b).unwrap_or_default();
            if biz.yurtici.key.is_empty() {
                println!("{}", l.t("tr-cargo-none"));
                return Ok(false);
            }
            let st = store(ctx);
            let get = |n: &str| web::secret(st.as_deref(), n).map(|s| s.expose_secret().to_owned());
            let endpoint = if biz.yurtici.id == "test" {
                tr::cargo::YURTICI_TEST
            } else {
                tr::cargo::YURTICI
            };
            match web::http::Http::new()
                .and_then(|h| Ok((h, get(&biz.yurtici.key)?, get(&biz.yurtici.secret)?)))
                .and_then(|(h, u, p)| rt.block_on(tr::cargo::yurtici(&h, endpoint, &u, &p, &key)))
            {
                Ok(s) => {
                    println!("{}  {}  {}  {}", s.key, s.status, s.tracking, s.message);
                    true
                }
                Err(e) => fail(l, e),
            }
        }
        TrCmd::Dns {
            domain,
            set,
            ttl,
            ns,
            yes,
        } => dns(ctx, &rt, &domain, set.as_deref(), ttl, &ns, yes),
        TrCmd::Keywords { source, top } => {
            let text = if source.starts_with("http") {
                match web::http::Http::new().and_then(|h| rt.block_on(h.follow(&source))) {
                    Ok((_, r)) => r.body,
                    Err(e) => return Ok(fail(l, e)),
                }
            } else {
                match std::fs::read_to_string(&source) {
                    Ok(t) => t,
                    Err(e) => return Ok(fail(l, e)),
                }
            };
            let plain = if text.contains('<') {
                tr::strip_tags(&text)
            } else {
                text
            };
            for (w, n) in tr::keywords(&plain, top) {
                println!("{n}\t{w}");
            }
            true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    enum T {
        Host {
            #[command(subcommand)]
            c: HostCmd,
        },
        Wp {
            #[command(subcommand)]
            c: WpCmd,
        },
        Redirects {
            #[command(subcommand)]
            c: RedirectsCmd,
        },
        Proposal {
            #[command(subcommand)]
            c: ProposalCmd,
        },
        Client {
            #[command(subcommand)]
            c: ClientCmd,
        },
        Tr {
            #[command(subcommand)]
            c: TrCmd,
        },
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one linear sequence; splitting it would scatter the steps
    fn commands_parse() {
        let t = T::try_parse_from([
            "kor",
            "host",
            "php",
            "prod",
            "--domain",
            "a.com",
            "--version",
            "8.3",
            "--yes",
        ])
        .unwrap();
        assert!(matches!(t, T::Host { c: HostCmd::Php { ref domain, .. } } if domain == "a.com"));
        assert!(T::try_parse_from(["kor", "host", "inventory", "--all", "--json"]).is_ok());
        assert!(
            T::try_parse_from([
                "kor",
                "host",
                "connect",
                "srv",
                "--kind",
                "cpanel",
                "--url",
                "https://x:2083",
                "--user",
                "u",
                "--secret",
                "CP"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "kor",
                "host",
                "clean",
                "prod",
                "--path",
                "/home/u/tmp",
                "--path",
                "/home/u/x.log"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "kor",
                "wp",
                "update",
                "shop",
                "--staging",
                "--only",
                "akismet,woocommerce"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "kor",
                "wp",
                "scaffold",
                "plugin",
                "--name",
                "X",
                "--slug",
                "x",
                "--woocommerce"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "kor",
                "redirects",
                "build",
                "--old",
                "sitemap:https://a/s.xml",
                "--new",
                "crawl:https://b/"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "kor",
                "redirects",
                "export",
                "m.csv",
                "--format",
                "nginx",
                "--collapse"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from([
                "kor",
                "proposal",
                "new",
                "--client",
                "A",
                "--project",
                "B",
                "--item",
                "Tasarım=12",
                "--contracts",
                "--format",
                "pdf,docx"
            ])
            .is_ok()
        );
        assert!(
            T::try_parse_from(["kor", "client", "report", "acme", "--month", "2026-11"]).is_ok()
        );
        assert!(T::try_parse_from(["kor", "client", "ticket", "acme", "add", "Logo"]).is_ok());
        assert!(T::try_parse_from(["kor", "tr", "kdv", "1.000,00", "--rate", "10"]).is_ok());
        assert!(T::try_parse_from(["kor", "tr", "kvkk", "--cookie", "_ga|Analiz|2 yıl"]).is_ok());
        assert!(T::try_parse_from(["kor", "tr", "efatura", "f.toml", "--send"]).is_ok());
    }
}
