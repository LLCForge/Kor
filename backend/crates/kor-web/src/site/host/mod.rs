//! `kor host` (§29.1): hosting panel and server connections.
//!
//! Profiles are `hosts/<name>.toml` in Kor's user folder. Adapters: cPanel (UAPI, user API token),
//! WHM (API 1 + `uapi_cpanel` for one account), Plesk (REST API key + CLI gateway), DirectAdmin
//! (login key, `CMD_API_*`), HestiaCP (access key, `/api/`), CyberPanel (its `cyberpanel` CLI over SSH)
//! and plain SSH servers (nginx/Apache, php-fpm, certbot). The token is a vault name, never a value.
//!
//! Every change runs the same way ([`run`]): read-only inventory → plan with an impact table →
//! the caller's approval → apply → verify (the site answers below 500 without a PHP fatal error, the
//! site's error log has no new fatal lines) → undo when verification fails and the change can be
//! undone. Each run is appended to the client's time log when the profile names a client.

mod cpanel;
mod cyberpanel;
mod directadmin;
mod hestia;
mod plesk;
pub mod shell;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use kor_agent::remote::Shell;
use kor_i18n::{Localizer, args};
use kor_secrets::SecretStore;
use serde::{Deserialize, Serialize};

use super::http::Http;
use super::{Result, check_domain, check_name};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Cpanel,
    Whm,
    Plesk,
    Directadmin,
    Cyberpanel,
    Hestia,
    Ssh,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Cpanel,
        Kind::Whm,
        Kind::Plesk,
        Kind::Directadmin,
        Kind::Cyberpanel,
        Kind::Hestia,
        Kind::Ssh,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Cpanel => "cpanel",
            Kind::Whm => "whm",
            Kind::Plesk => "plesk",
            Kind::Directadmin => "directadmin",
            Kind::Cyberpanel => "cyberpanel",
            Kind::Hestia => "hestia",
            Kind::Ssh => "ssh",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The panel's usual HTTPS port.
    pub fn default_port(self) -> Option<u16> {
        match self {
            Kind::Cpanel => Some(2083),
            Kind::Whm => Some(2087),
            Kind::Plesk => Some(8443),
            Kind::Directadmin => Some(2222),
            Kind::Hestia => Some(8083),
            Kind::Cyberpanel | Kind::Ssh => None,
        }
    }

    fn needs_api(self) -> bool {
        !matches!(self, Kind::Cyberpanel | Kind::Ssh)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostProfile {
    pub kind: Option<Kind>,
    /// Panel address, `https://server.example.com:2083`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Panel user (cPanel/DirectAdmin/Hestia user, `root` for WHM, `admin` for CyberPanel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Vault entry with the API token, key, login key or `ACCESS:SECRET` pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// SSH: a `connections.toml` id or `user@host[:port]`, for shell tasks and plain servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
    /// WHM: the cPanel account tasks act on when none is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Client workspace this server belongs to (time log, reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Contact email for Let's Encrypt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

impl HostProfile {
    pub fn kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Ssh)
    }

    /// Fill defaults and refuse what cannot work: a panel without URL or secret, a URL that is not
    /// HTTPS (tokens would travel in clear text; loopback is allowed for tests).
    pub fn validate(&mut self) -> Result<()> {
        let kind = self.kind();
        if kind.needs_api() {
            let url = self
                .url
                .as_deref()
                .ok_or_else(|| format!("{} needs --url", kind.as_str()))?;
            let mut u = url::Url::parse(url).map_err(|e| format!("invalid URL {url}: {e}"))?;
            let local = u.host_str().is_some_and(kor_core::net::is_loopback_host);
            if u.scheme() != "https" && !local {
                return Err(
                    "the panel URL must use https (credentials are sent with every request)".into(),
                );
            }
            if u.port().is_none()
                && let Some(p) = kind.default_port()
            {
                let _ = u.set_port(Some(p));
            }
            u.set_path("");
            self.url = Some(u.as_str().trim_end_matches('/').to_owned());
            if self.secret.is_none() {
                return Err(format!(
                    "{} needs --secret (a vault entry name)",
                    kind.as_str()
                ));
            }
        }
        if matches!(kind, Kind::Cyberpanel | Kind::Ssh) && self.ssh.is_none() {
            return Err(format!("{} needs --ssh", kind.as_str()));
        }
        if let Some(s) = &self.secret
            && !kor_secrets::vault::valid_name(s)
        {
            return Err(format!("invalid vault name `{s}`"));
        }
        if let Some(c) = &self.client {
            check_name(c)?;
        }
        Ok(())
    }
}

pub fn hosts_dir(base: &Path) -> PathBuf {
    base.join("hosts")
}

pub fn save(base: &Path, name: &str, p: &HostProfile) -> Result<PathBuf> {
    check_name(name)?;
    let path = hosts_dir(base).join(format!("{name}.toml"));
    super::save_toml(&path, p)?;
    Ok(path)
}

pub fn load(base: &Path, name: &str) -> Result<HostProfile> {
    check_name(name)?;
    let path = hosts_dir(base).join(format!("{name}.toml"));
    if !path.is_file() {
        return Err(format!("no host `{name}` (kor host connect {name} …)"));
    }
    super::load_toml(&path)
}

pub fn list(base: &Path) -> Vec<(String, Result<HostProfile>)> {
    super::names_in(&hosts_dir(base))
        .into_iter()
        .map(|n| {
            let p = load(base, &n);
            (n, p)
        })
        .collect()
}

pub fn remove(base: &Path, name: &str) -> Result<bool> {
    check_name(name)?;
    let path = hosts_dir(base).join(format!("{name}.toml"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Domain {
    pub name: String,
    /// `main`, `addon`, `sub`, `parked`, `site`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docroot: Option<String>,
    /// `8.3` style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php: Option<String>,
    /// WHM: owning cPanel account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cert {
    pub domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_after: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// `panel` or `tls` (read from the live handshake).
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Disk {
    pub used_mb: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_mb: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Inventory {
    pub host: String,
    pub kind: Option<Kind>,
    pub domains: Vec<Domain>,
    pub databases: Vec<String>,
    /// Installed PHP versions, `8.3` style.
    pub php_installed: Vec<String>,
    pub certs: Vec<Cert>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<Disk>,
    /// WHM accounts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_accounts: Option<u64>,
    /// What could not be read (permissions, missing tools), so gaps are visible.
    pub notes: Vec<String>,
    pub scanned_at: i64,
}

impl Inventory {
    pub fn domain(&self, name: &str) -> Option<&Domain> {
        self.domains.iter().find(|d| d.name == name)
    }

    /// The domain changes are verified against when the task names none.
    pub fn main_domain(&self) -> Option<&str> {
        self.domains
            .iter()
            .find(|d| d.kind == "main")
            .or_else(|| self.domains.first())
            .map(|d| d.name.as_str())
    }
}

/// `ea-php83`, `PHP-8_3`, `plesk-php83-fastcgi`, `php8.3`, `8.3` → `8.3`.
#[allow(clippy::many_single_char_names)]
pub fn php_version(s: &str) -> Option<String> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '_'))
        .collect();
    let d = digits.replace('_', ".");
    if d.contains('.') {
        let mut it = d.split('.').filter(|p| !p.is_empty());
        let (a, b) = (it.next()?, it.next()?);
        return Some(format!("{a}.{b}"));
    }
    let mut c = d.chars();
    let (a, b) = (c.next()?, c.next()?);
    Some(format!("{a}.{b}"))
}

fn valid_php(v: &str) -> bool {
    let mut p = v.split('.');
    matches!((p.next(), p.next(), p.next()), (Some(a), Some(b), None)
        if (1..=2).contains(&a.len()) && (1..=2).contains(&b.len())
            && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit()))
}

/// One change on a server. The `Remove*`/`Restore*` variants are the inverses Kor records to undo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "task", rename_all = "kebab-case")]
pub enum Task {
    Ssl {
        domain: String,
    },
    Subdomain {
        sub: String,
        domain: String,
        dir: Option<String>,
    },
    Database {
        name: String,
        user: String,
    },
    Email {
        address: String,
        quota_mb: u32,
    },
    Backup {
        domain: Option<String>,
    },
    Restore {
        archive: String,
        domain: String,
    },
    /// Empty `paths`: list candidates only. Otherwise delete those candidates.
    Clean {
        paths: Vec<String>,
    },
    Cron {
        schedule: String,
        command: String,
    },
    Php {
        domain: String,
        version: String,
    },
    PhpIni {
        domain: String,
        key: String,
        value: String,
    },
    Perms {
        path: String,
    },
    Sftp {
        user: String,
        path: String,
        domain: Option<String>,
    },
    RemoveSubdomain {
        sub: String,
        domain: String,
    },
    RemoveDatabase {
        name: String,
        user: String,
    },
    RemoveEmail {
        address: String,
    },
    RemoveCron {
        id: String,
    },
    RemoveSftp {
        user: String,
        domain: Option<String>,
    },
    RestoreModes {
        path: String,
        list: String,
    },
    RestoreFile {
        path: String,
        backup: String,
        reload: Option<String>,
    },
}

impl Task {
    pub fn id(&self) -> &'static str {
        match self {
            Task::Ssl { .. } => "ssl",
            Task::Subdomain { .. } => "subdomain",
            Task::Database { .. } => "db",
            Task::Email { .. } => "email",
            Task::Backup { .. } => "backup",
            Task::Restore { .. } => "restore",
            Task::Clean { .. } => "clean",
            Task::Cron { .. } => "cron",
            Task::Php { .. } => "php",
            Task::PhpIni { .. } => "php-ini",
            Task::Perms { .. } => "perms",
            Task::Sftp { .. } => "sftp",
            Task::RemoveSubdomain { .. } => "remove-subdomain",
            Task::RemoveDatabase { .. } => "remove-db",
            Task::RemoveEmail { .. } => "remove-email",
            Task::RemoveCron { .. } => "remove-cron",
            Task::RemoveSftp { .. } => "remove-sftp",
            Task::RestoreModes { .. } => "restore-modes",
            Task::RestoreFile { .. } => "restore-file",
        }
    }

    /// The domain a change is verified against.
    pub fn domain(&self) -> Option<&str> {
        match self {
            Task::Ssl { domain }
            | Task::Restore { domain, .. }
            | Task::Php { domain, .. }
            | Task::PhpIni { domain, .. } => Some(domain),
            Task::Backup { domain } | Task::Sftp { domain, .. } => domain.as_deref(),
            _ => None,
        }
    }

    /// Reject anything that could be read as a shell option, path escape or a second command.
    pub fn validate(&self) -> Result<()> {
        let word = |s: &str, what: &str| -> Result<()> {
            let ok = (1..=64).contains(&s.len())
                && !s.starts_with('-')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
            if ok {
                Ok(())
            } else {
                Err(format!("invalid {what} `{s}`"))
            }
        };
        match self {
            Task::Ssl { domain }
            | Task::Backup {
                domain: Some(domain),
            } => check_domain(domain),
            Task::Subdomain { sub, domain, dir } => {
                word(sub, "subdomain")?;
                check_domain(domain)?;
                dir.as_deref().map_or(Ok(()), shell::check_path)
            }
            Task::Database { name, user } => {
                word(name, "database")?;
                word(user, "database user")
            }
            Task::Email { address, .. } => {
                let (local, dom) = address
                    .split_once('@')
                    .ok_or_else(|| format!("invalid email `{address}`"))?;
                word(local, "mailbox")?;
                check_domain(dom)
            }
            Task::Restore { archive, domain } => {
                check_domain(domain)?;
                shell::check_path(archive)
            }
            Task::Clean { paths } => paths.iter().try_for_each(|p| shell::check_path(p)),
            Task::Cron { schedule, command } => shell::check_cron(schedule, command),
            Task::Php { domain, version } => {
                check_domain(domain)?;
                if valid_php(version) {
                    Ok(())
                } else {
                    Err(format!("invalid PHP version `{version}` (use 8.3)"))
                }
            }
            Task::PhpIni { domain, key, value } => {
                check_domain(domain)?;
                shell::check_ini(key, value)
            }
            Task::Perms { path } => shell::check_path(path),
            Task::Sftp { user, path, domain } => {
                word(user, "user")?;
                shell::check_path(path)?;
                domain.as_deref().map_or(Ok(()), check_domain)
            }
            _ => Ok(()),
        }
    }

    /// Whether Kor can undo the change by itself.
    pub fn reversible(&self) -> bool {
        !matches!(
            self,
            Task::Clean { .. } | Task::Ssl { .. } | Task::Backup { .. }
        )
    }
}

/// What applying a task did.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub summary: String,
    pub undo: Option<Task>,
    /// Vault entry holding a password Kor generated (database, mailbox, SFTP user).
    pub credential: Option<String>,
    /// Command output worth showing (clean candidates, backup file).
    pub output: Vec<String>,
}

#[derive(Debug)]
pub struct ApplyCtx<'a> {
    pub inv: &'a Inventory,
    pub email: Option<&'a str>,
    /// A password generated for this task, already stored in the vault under `credential`.
    pub password: Option<&'a str>,
}

/// One server adapter.
#[async_trait]
pub trait Panel: Send + Sync {
    async fn inventory(&self) -> Result<Inventory>;
    async fn apply(&self, task: &Task, ctx: &ApplyCtx<'_>) -> Result<Applied>;
    /// The SSH shell, when the profile has one (error log checks, shell tasks).
    fn shell(&self) -> Option<&Arc<dyn Shell>>;
}

/// Open the adapter for a profile. `secret` is the resolved token, `ssh` the shell for `ssh = …`.
pub fn open(
    name: &str,
    p: &HostProfile,
    secret: Option<&str>,
    ssh: Option<Arc<dyn Shell>>,
    account: Option<String>,
) -> Result<Box<dyn Panel>> {
    let http = Http::new()?;
    let url = p.url.clone().unwrap_or_default();
    let user = p.user.clone().unwrap_or_default();
    let need = || {
        secret
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: no API secret"))
    };
    let sh = || {
        ssh.clone()
            .ok_or_else(|| format!("{name}: no SSH connection"))
    };
    Ok(match p.kind() {
        Kind::Cpanel => Box::new(cpanel::Cpanel::new(
            name,
            http,
            url,
            &user,
            &need()?,
            None,
            ssh,
        )),
        Kind::Whm => Box::new(cpanel::Cpanel::new(
            name,
            http,
            url,
            &user,
            &need()?,
            account
                .or_else(|| p.account.clone())
                .or(Some(String::new())),
            ssh,
        )),
        Kind::Plesk => Box::new(plesk::Plesk::new(name, http, url, need()?, ssh)),
        Kind::Directadmin => Box::new(directadmin::DirectAdmin::new(
            name,
            http,
            url,
            &user,
            &need()?,
            ssh,
        )),
        Kind::Hestia => Box::new(hestia::Hestia::new(name, http, url, user, need()?, ssh)),
        Kind::Cyberpanel => Box::new(cyberpanel::CyberPanel::new(name, sh()?)),
        Kind::Ssh => Box::new(shell::Plain::new(name, sh()?)),
    })
}

/// The SSH shell for a profile's `ssh` setting.
pub fn ssh_shell(p: &HostProfile) -> Result<Option<Arc<dyn Shell>>> {
    let Some(target) = &p.ssh else {
        return Ok(None);
    };
    let file = kor_config::user_dir().map(|d| d.join("connections.toml"));
    let h = kor_agent::remote::resolve_target(target, file.as_deref())?;
    Ok(Some(Arc::new(kor_agent::remote::SshShell::new(&h))))
}

/// Profile → adapter with secrets from the vault (or environment) and the SSH shell.
pub fn connect(
    base: &Path,
    name: &str,
    store: Option<&dyn SecretStore>,
    account: Option<String>,
) -> Result<(HostProfile, Box<dyn Panel>)> {
    use kor_secrets::ExposeSecret as _;
    let p = load(base, name)?;
    let secret = match &p.secret {
        Some(s) => Some(super::secret(store, s)?.expose_secret().to_owned()),
        None => None,
    };
    let panel = open(name, &p, secret.as_deref(), ssh_shell(&p)?, account)?;
    Ok((p, panel))
}

/// Inventory plus live certificate expiry for domains the panel did not report a certificate for.
pub async fn inventory(panel: &dyn Panel) -> Result<Inventory> {
    let mut inv = panel.inventory().await?;
    let missing: Vec<String> = inv
        .domains
        .iter()
        .filter(|d| !inv.certs.iter().any(|c| c.domain == d.name))
        .map(|d| d.name.clone())
        .take(60)
        .collect();
    for chunk in missing.chunks(8) {
        let found =
            futures::future::join_all(chunk.iter().map(|d| super::tls::probe(d, 443))).await;
        for (d, r) in chunk.iter().zip(found) {
            inv.certs.push(match r {
                Ok(c) => Cert {
                    domain: d.clone(),
                    not_after: Some(c.not_after),
                    issuer: Some(c.issuer),
                    source: "tls".into(),
                    error: None,
                },
                Err(e) => Cert {
                    domain: d.clone(),
                    source: "tls".into(),
                    error: Some(e),
                    ..Cert::default()
                },
            });
        }
    }
    inv.scanned_at = super::now();
    Ok(inv)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanRow {
    pub step: String,
    pub target: String,
    pub impact: String,
    pub reversible: bool,
}

/// The plan and impact table shown before approval. `warnings` come from linked WordPress sites
/// (plugins that may not run on the new PHP version).
pub fn plan(l: &Localizer, task: &Task, inv: &Inventory, warnings: &[String]) -> Vec<PlanRow> {
    let row = |key: &str, target: String, impact: String| PlanRow {
        step: l.t(key),
        target,
        impact,
        reversible: task.reversible(),
    };
    let mut rows = Vec::new();
    match task {
        Task::Php { domain, version } => {
            let from = inv
                .domain(domain)
                .and_then(|d| d.php.clone())
                .unwrap_or_else(|| "?".into());
            rows.push(row(
                "web-step-php",
                domain.clone(),
                l.t_args(
                    "web-impact-php",
                    &args!("from" => from, "to" => version.clone()),
                ),
            ));
            if !inv.php_installed.is_empty() && !inv.php_installed.iter().any(|v| v == version) {
                rows.push(row(
                    "web-step-check",
                    domain.clone(),
                    l.t_args(
                        "web-impact-php-missing",
                        &args!("to" => version.clone(), "have" => inv.php_installed.join(", ")),
                    ),
                ));
            }
        }
        Task::Ssl { domain } => {
            let cur = inv
                .certs
                .iter()
                .find(|c| &c.domain == domain)
                .and_then(|c| c.not_after)
                .map_or_else(
                    || l.t("web-none"),
                    |t| (t - super::now()).div_euclid(86_400).to_string(),
                );
            rows.push(row(
                "web-step-ssl",
                domain.clone(),
                l.t_args("web-impact-ssl", &args!("days" => cur)),
            ));
        }
        Task::Clean { paths } if paths.is_empty() => rows.push(row(
            "web-step-clean-list",
            inv.host.clone(),
            l.t("web-impact-none"),
        )),
        Task::Clean { paths } => {
            for p in paths {
                rows.push(row("web-step-clean", p.clone(), l.t("web-impact-delete")));
            }
        }
        Task::Perms { path } => {
            rows.push(row("web-step-perms", path.clone(), l.t("web-impact-perms")))
        }
        Task::Restore { archive, domain } => {
            rows.push(row(
                "web-step-backup",
                domain.clone(),
                l.t("web-impact-none"),
            ));
            rows.push(row(
                "web-step-restore",
                archive.clone(),
                l.t("web-impact-overwrite"),
            ));
        }
        other => rows.push(row(
            &format!("web-step-{}", other.id()),
            describe(other),
            l.t("web-impact-create"),
        )),
    }
    for w in warnings {
        rows.push(PlanRow {
            step: l.t("web-step-check"),
            target: inv.host.clone(),
            impact: w.clone(),
            reversible: true,
        });
    }
    rows
}

/// Short target text for plan rows and logs (never a password).
pub fn describe(t: &Task) -> String {
    match t {
        Task::Ssl { domain }
        | Task::Backup {
            domain: Some(domain),
        } => domain.clone(),
        Task::Backup { domain: None } => "*".into(),
        Task::Subdomain { sub, domain, .. } | Task::RemoveSubdomain { sub, domain } => {
            format!("{sub}.{domain}")
        }
        Task::Database { name, user } | Task::RemoveDatabase { name, user } => {
            format!("{name} ({user})")
        }
        Task::Email { address, .. } | Task::RemoveEmail { address } => address.clone(),
        Task::Restore { archive, .. } => archive.clone(),
        Task::Clean { paths } => paths.join(", "),
        Task::Cron { schedule, command } => format!("{schedule} {command}"),
        Task::Php { domain, version } => format!("{domain} → PHP {version}"),
        Task::PhpIni { domain, key, value } => format!("{domain}: {key}={value}"),
        Task::Perms { path } | Task::RestoreModes { path, .. } | Task::RestoreFile { path, .. } => {
            path.clone()
        }
        Task::Sftp { user, path, .. } => format!("{user} → {path}"),
        Task::RemoveCron { id } => id.clone(),
        Task::RemoveSftp { user, .. } => user.clone(),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Verify {
    pub url: String,
    pub status: u16,
    pub ms: u64,
    /// A PHP fatal error or WordPress "critical error" page.
    pub fatal: bool,
    /// New fatal lines in the site's error log since the change started.
    pub new_log_errors: Option<u64>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn fatal_text(body: &str) -> bool {
    [
        "Fatal error</b>",
        "PHP Fatal error",
        "There has been a critical error",
        "Parse error</b>",
        "Error establishing a database connection",
    ]
    .iter()
    .any(|m| body.contains(m))
}

/// The site answers below 500 and shows no fatal error page.
pub async fn check_site(http: &Http, domain: &str) -> Verify {
    let url = format!("https://{domain}/");
    match http.follow(&url).await {
        Ok((_, r)) => {
            let fatal = fatal_text(&r.body);
            Verify {
                url,
                status: r.status,
                ms: r.ms,
                fatal,
                ok: r.status < 500 && r.status != 0 && !fatal,
                ..Verify::default()
            }
        }
        Err(e) => Verify {
            url,
            error: Some(e),
            ..Verify::default()
        },
    }
}

/// Fatal lines in the site's `error_log` (cPanel and most panels keep it in the document root).
async fn fatal_count(sh: &dyn Shell, docroot: &str) -> Option<u64> {
    let cmd = format!(
        "tail -n 400 {}/error_log 2>/dev/null | grep -c 'PHP Fatal' || true",
        kor_agent::remote::sh_quote(docroot)
    );
    let out = sh
        .run(&cmd, None, std::time::Duration::from_secs(30))
        .await
        .ok()?;
    out.text().trim().parse().ok()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunReport {
    pub host: String,
    pub task: Option<Task>,
    pub plan: Vec<PlanRow>,
    pub approved: bool,
    pub applied: Option<String>,
    pub output: Vec<String>,
    pub verify: Option<Verify>,
    pub rolled_back: bool,
    pub credential: Option<String>,
    pub error: Option<String>,
    pub secs: u64,
}

/// Approval callback: sees the plan, answers yes or no.
pub type Approve<'a> = dyn Fn(&[PlanRow]) -> bool + Send + Sync + 'a;

/// Inventory → plan → approval → apply → verify → undo on failure (see the module comment).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub async fn run(
    l: &Localizer,
    host: &str,
    profile: &HostProfile,
    panel: &dyn Panel,
    task: Task,
    warnings: &[String],
    store: Option<&dyn SecretStore>,
    approve: &Approve<'_>,
) -> RunReport {
    let start = Instant::now();
    let mut rep = RunReport {
        host: host.to_owned(),
        task: Some(task.clone()),
        ..RunReport::default()
    };
    if let Err(e) = task.validate() {
        rep.error = Some(e);
        return rep;
    }
    let inv = match inventory(panel).await {
        Ok(i) => i,
        Err(e) => {
            rep.error = Some(e);
            return rep;
        }
    };
    rep.plan = plan(l, &task, &inv, warnings);
    // Listing clean-up candidates changes nothing: no approval needed.
    let read_only = matches!(&task, Task::Clean { paths } if paths.is_empty());
    rep.approved = read_only || approve(&rep.plan);
    if !rep.approved {
        rep.error = Some(l.t("web-not-approved"));
        return rep;
    }
    // A generated password goes to the vault before the account exists, so it is never lost.
    let cred = match &task {
        Task::Database { name, .. } => {
            Some(super::credential_name("db", &format!("{host}_{name}")))
        }
        Task::Email { address, .. } => Some(super::credential_name("mail", address)),
        Task::Sftp { user, .. } => Some(super::credential_name("sftp", &format!("{host}_{user}"))),
        _ => None,
    };
    let password = match &cred {
        Some(name) => {
            let pw = match super::random_password() {
                Ok(p) => p,
                Err(e) => {
                    rep.error = Some(e);
                    return rep;
                }
            };
            let Some(st) = store else {
                rep.error = Some(l.t("web-needs-vault"));
                return rep;
            };
            if let Err(e) =
                kor_secrets::vault::add(st, name, &kor_secrets::SecretString::from(pw.clone()))
            {
                rep.error = Some(e.to_string());
                return rep;
            }
            Some(pw)
        }
        None => None,
    };
    let http = match Http::new() {
        Ok(h) => h,
        Err(e) => {
            rep.error = Some(e);
            return rep;
        }
    };
    let domain = task
        .domain()
        .map(str::to_owned)
        .or_else(|| inv.main_domain().map(str::to_owned));
    let docroot = domain
        .as_deref()
        .and_then(|d| inv.domain(d))
        .and_then(|d| d.docroot.clone());
    let before = match (panel.shell(), &docroot) {
        (Some(sh), Some(root)) => fatal_count(sh.as_ref(), root).await,
        _ => None,
    };
    let ctx = ApplyCtx {
        inv: &inv,
        email: profile.email.as_deref(),
        password: password.as_deref(),
    };
    let applied = match panel.apply(&task, &ctx).await {
        Ok(a) => a,
        Err(e) => {
            rep.error = Some(e);
            rep.secs = start.elapsed().as_secs();
            return rep;
        }
    };
    rep.applied = Some(applied.summary.clone());
    rep.output = applied.output.clone();
    rep.credential = applied.credential.clone().or(cred);
    if read_only {
        rep.secs = start.elapsed().as_secs();
        return rep;
    }
    if let Some(d) = &domain {
        let mut v = check_site(&http, d).await;
        if let (Some(sh), Some(root), Some(b)) = (panel.shell(), &docroot, before)
            && let Some(a) = fatal_count(sh.as_ref(), root).await
        {
            let new = a.saturating_sub(b);
            v.new_log_errors = Some(new);
            v.ok &= new == 0;
        }
        let failed = !v.ok;
        rep.verify = Some(v);
        if failed && let Some(undo) = &applied.undo {
            match panel.apply(undo, &ctx).await {
                Ok(_) => rep.rolled_back = true,
                Err(e) => rep.error = Some(l.t_args("web-undo-failed", &args!("error" => e))),
            }
        }
    }
    rep.secs = start.elapsed().as_secs();
    rep
}

/// Markdown summary of an inventory.
pub fn inventory_markdown(l: &Localizer, inv: &Inventory, now: i64) -> String {
    use std::fmt::Write as _;
    let cell = super::cell;
    let mut o = format!(
        "# {} ({})\n\n",
        cell(&inv.host),
        inv.kind.map_or("?", Kind::as_str)
    );
    let _ = writeln!(
        o,
        "| {} | {} | {} | {} | {} |\n|---|---|---|---|---|",
        l.t("web-col-domain"),
        l.t("web-col-kind"),
        l.t("web-col-php"),
        l.t("web-col-ssl"),
        l.t("web-col-docroot")
    );
    for d in &inv.domains {
        let ssl = inv.certs.iter().find(|c| c.domain == d.name).map_or_else(
            || "—".to_owned(),
            |c| match (c.not_after, &c.error) {
                (Some(t), _) => l.t_args(
                    "web-days-left",
                    &args!("days" => (t - now).div_euclid(86_400)),
                ),
                (None, Some(e)) => cell(e),
                _ => "—".into(),
            },
        );
        let _ = writeln!(
            o,
            "| {} | {} | {} | {} | {} |",
            cell(&d.name),
            cell(&d.kind),
            d.php.as_deref().unwrap_or("—"),
            ssl,
            cell(d.docroot.as_deref().unwrap_or("—"))
        );
    }
    let _ = writeln!(
        o,
        "\n- {}: {}\n- {}: {}",
        l.t("web-col-databases"),
        if inv.databases.is_empty() {
            "—".into()
        } else {
            inv.databases.join(", ")
        },
        l.t("web-col-php-installed"),
        if inv.php_installed.is_empty() {
            "—".into()
        } else {
            inv.php_installed.join(", ")
        }
    );
    if let Some(d) = &inv.disk {
        let _ = writeln!(
            o,
            "- {}: {:.0} MB{}",
            l.t("web-col-disk"),
            d.used_mb,
            d.limit_mb
                .map(|x| format!(" / {x:.0} MB"))
                .unwrap_or_default()
        );
    }
    if let Some(n) = inv.email_accounts {
        let _ = writeln!(o, "- {}: {n}", l.t("web-col-mail"));
    }
    if !inv.accounts.is_empty() {
        let _ = writeln!(
            o,
            "- {}: {}",
            l.t("web-col-accounts"),
            inv.accounts.join(", ")
        );
    }
    for n in &inv.notes {
        let _ = writeln!(o, "- {n}");
    }
    o
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Fake SSH: answers by the first matching substring, records every command.
    #[derive(Debug, Default)]
    pub(crate) struct FakeShell {
        pub answers: Vec<(&'static str, i32, String)>,
        pub log: Mutex<Vec<(String, Option<String>)>>,
    }

    #[async_trait]
    impl Shell for FakeShell {
        async fn run(
            &self,
            command: &str,
            stdin: Option<&[u8]>,
            _t: Duration,
        ) -> std::result::Result<kor_agent::remote::Output, String> {
            self.log.lock().unwrap().push((
                command.to_owned(),
                stdin.map(|s| String::from_utf8_lossy(s).into_owned()),
            ));
            let (code, out) = self
                .answers
                .iter()
                .find(|(k, _, _)| command.contains(k))
                .map_or((0, String::new()), |(_, c, o)| (*c, o.clone()));
            Ok(kor_agent::remote::Output {
                code: Some(code),
                stdout: out.into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    impl FakeShell {
        pub(crate) fn commands(&self) -> Vec<String> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .map(|(c, _)| c.clone())
                .collect()
        }
    }

    #[test]
    fn php_versions_from_every_panel() {
        for (s, v) in [
            ("ea-php83", "8.3"),
            ("PHP-8_2", "8.2"),
            ("plesk-php74-fastcgi", "7.4"),
            ("php8.1", "8.1"),
            ("8.3", "8.3"),
            ("alt-php56", "5.6"),
        ] {
            assert_eq!(php_version(s).as_deref(), Some(v), "{s}");
        }
        assert!(php_version("none").is_none());
        assert!(valid_php("8.3") && !valid_php("8") && !valid_php("8.3;rm"));
    }

    #[test]
    fn profiles_round_trip_and_validate() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = HostProfile {
            kind: Some(Kind::Cpanel),
            url: Some("https://srv.example.com".into()),
            user: Some("acme".into()),
            secret: Some("CPANEL_ACME".into()),
            ..HostProfile::default()
        };
        p.validate().unwrap();
        assert_eq!(p.url.as_deref(), Some("https://srv.example.com:2083"));
        save(dir.path(), "acme", &p).unwrap();
        assert_eq!(load(dir.path(), "acme").unwrap(), p);
        assert_eq!(list(dir.path()).len(), 1);
        let mut bad = HostProfile {
            url: Some("http://srv.example.com".into()),
            ..p.clone()
        };
        assert!(bad.validate().is_err());
        let mut no_ssh = HostProfile {
            kind: Some(Kind::Ssh),
            ..HostProfile::default()
        };
        assert!(no_ssh.validate().is_err());
        assert!(remove(dir.path(), "acme").unwrap());
        assert!(load(dir.path(), "../x").is_err());
    }

    #[test]
    fn tasks_refuse_shell_tricks() {
        assert!(
            Task::Database {
                name: "shop".into(),
                user: "u;rm".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Task::Cron {
                schedule: "* * * * *".into(),
                command: "a\nb".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Task::Php {
                domain: "a.com".into(),
                version: "8.3".into()
            }
            .validate()
            .is_ok()
        );
        assert!(
            Task::Perms {
                path: "../etc".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Task::Email {
                address: "info@a.com".into(),
                quota_mb: 100
            }
            .validate()
            .is_ok()
        );
    }

    struct Fake {
        inv: Inventory,
        applied: Mutex<Vec<Task>>,
    }

    #[async_trait]
    impl Panel for Fake {
        async fn inventory(&self) -> Result<Inventory> {
            Ok(self.inv.clone())
        }
        async fn apply(&self, task: &Task, _: &ApplyCtx<'_>) -> Result<Applied> {
            self.applied.lock().unwrap().push(task.clone());
            Ok(Applied {
                summary: "ok".into(),
                undo: Some(Task::Php {
                    domain: "127.0.0.1.nip.io".into(),
                    version: "7.4".into(),
                }),
                ..Applied::default()
            })
        }
        fn shell(&self) -> Option<&Arc<dyn Shell>> {
            None
        }
    }

    #[tokio::test]
    async fn refused_plans_change_nothing() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let f = Fake {
            inv: Inventory::default(),
            applied: Mutex::default(),
        };
        let t = Task::Php {
            domain: "example.com".into(),
            version: "8.3".into(),
        };
        let r = run(&l, "h", &HostProfile::default(), &f, t, &[], None, &|_| {
            false
        })
        .await;
        assert!(!r.approved && r.applied.is_none());
        assert!(f.applied.lock().unwrap().is_empty());
        assert_eq!(r.plan.len(), 1);
    }

    #[tokio::test]
    async fn a_failing_site_is_rolled_back() {
        let l = Localizer::new(kor_i18n::Lang::En);
        // Nothing listens on this name's port 443 locally; verification fails and the undo runs.
        let f = Fake {
            inv: Inventory {
                domains: vec![Domain {
                    name: "localhost.invalid".into(),
                    kind: "main".into(),
                    ..Domain::default()
                }],
                ..Inventory::default()
            },
            applied: Mutex::default(),
        };
        let t = Task::Php {
            domain: "localhost.invalid".into(),
            version: "8.3".into(),
        };
        let r = run(&l, "h", &HostProfile::default(), &f, t, &[], None, &|_| {
            true
        })
        .await;
        assert!(r.approved && r.applied.is_some());
        assert!(!r.verify.as_ref().unwrap().ok);
        assert!(r.rolled_back, "{r:?}");
        assert_eq!(f.applied.lock().unwrap().len(), 2);
    }

    #[test]
    fn fatal_pages_are_detected() {
        assert!(fatal_text("<b>Fatal error</b>: Uncaught Error"));
        assert!(fatal_text(
            "There has been a critical error on this website."
        ));
        assert!(!fatal_text("<html>Hoş geldiniz</html>"));
    }
}
