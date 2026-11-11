//! Shell tasks over SSH (any panel with `ssh = …`) and the plain-server adapter: nginx/Apache vhosts,
//! php-fpm, certbot, MySQL/MariaDB through `sudo -n` (passwordless sudo only; Kor never types a
//! password). Every command is built from validated words and `sh_quote`d paths; secrets go on stdin.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kor_agent::remote::{Shell, sh_quote};

use super::{Applied, ApplyCtx, Disk, Domain, Inventory, Kind, Panel, Task, php_version};
use crate::site::Result;

const T: Duration = Duration::from_secs(300);

pub(crate) fn check_path(p: &str) -> Result<()> {
    let bad = p.is_empty()
        || p.len() > 1024
        || p.starts_with('-')
        || p.chars().any(char::is_control)
        || p.split(['/', '\\']).any(|c| c == "..");
    if bad {
        Err(format!("invalid path `{p}`"))
    } else {
        Ok(())
    }
}

/// Five cron fields of `0-9 * / , -` and a one-line command.
pub(crate) fn check_cron(schedule: &str, command: &str) -> Result<()> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    let ok = fields.len() == 5
        && fields.iter().all(|f| {
            f.bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'*' | b'/' | b',' | b'-'))
        });
    if !ok {
        return Err(format!("invalid cron schedule `{schedule}` (five fields)"));
    }
    if command.trim().is_empty() || command.len() > 1000 || command.contains(['\n', '\r', '\0']) {
        return Err("invalid cron command".into());
    }
    Ok(())
}

pub(crate) fn check_ini(key: &str, value: &str) -> Result<()> {
    let k = (1..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.'));
    let v = value.len() <= 200
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-' | b' ' | b':' | b',')
        });
    if k && v {
        Ok(())
    } else {
        Err(format!("invalid php.ini setting `{key}={value}`"))
    }
}

/// A path for the remote shell: `~/x` stays relative to the login's home.
pub(crate) fn q(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("\"$HOME\"/{}", sh_quote(rest)),
        None => sh_quote(p),
    }
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M%S").to_string()
}

pub(crate) async fn exec(sh: &dyn Shell, cmd: &str, stdin: Option<&[u8]>) -> Result<String> {
    let out = sh.run(cmd, stdin, T).await?;
    if out.ok() {
        Ok(out.text())
    } else {
        let err = out.error();
        let msg = if err.trim().is_empty() {
            out.text()
        } else {
            err
        };
        Err(msg.trim().chars().take(2000).collect())
    }
}

fn docroot<'a>(inv: &'a Inventory, domain: &str) -> Result<&'a str> {
    inv.domain(domain)
        .and_then(|d| d.docroot.as_deref())
        .ok_or_else(|| format!("{domain}: document root unknown"))
}

/// Shell tasks every adapter with SSH can run. `None` = not a shell task.
pub(crate) async fn generic(
    sh: &dyn Shell,
    task: &Task,
    ctx: &ApplyCtx<'_>,
) -> Option<Result<Applied>> {
    Some(match task {
        Task::Perms { path } => perms(sh, path).await,
        Task::RestoreModes { path, list } => exec(
            sh,
            &format!(
                "cd {} && while read -r m p; do chmod \"$m\" \"$p\"; done < {} && rm -f {}",
                q(path),
                q(list),
                q(list)
            ),
            None,
        )
        .await
        .map(|_| Applied {
            summary: format!("modes restored: {path}"),
            ..Applied::default()
        }),
        Task::Clean { paths } => clean(sh, paths).await,
        Task::Cron { schedule, command } => cron(sh, schedule, command).await,
        Task::RemoveCron { id } => exec(
            sh,
            &format!(
                "crontab -l 2>/dev/null | grep -v -F {} | crontab -",
                sh_quote(&format!("# kor:{id}"))
            ),
            None,
        )
        .await
        .map(|_| Applied {
            summary: format!("cron {id} removed"),
            ..Applied::default()
        }),
        Task::PhpIni { domain, key, value } => match docroot(ctx.inv, domain) {
            Ok(root) => php_ini(sh, root, key, value).await,
            Err(e) => Err(e),
        },
        Task::RestoreFile {
            path,
            backup,
            reload,
        } => {
            let mut cmd = format!("cp -p {} {}", q(backup), q(path));
            if let Some(r) = reload {
                let _ = write!(cmd, " && {r}");
            }
            exec(sh, &cmd, None).await.map(|_| Applied {
                summary: format!("restored {path}"),
                ..Applied::default()
            })
        }
        Task::Backup { domain } => match domain.as_deref().map(|d| docroot(ctx.inv, d)).transpose()
        {
            Ok(root) => backup(sh, domain.as_deref().unwrap_or("home"), root).await,
            Err(e) => Err(e),
        },
        Task::Restore { archive, domain } => match docroot(ctx.inv, domain) {
            Ok(root) => restore(sh, archive, domain, root).await,
            Err(e) => Err(e),
        },
        _ => return None,
    })
}

async fn perms(sh: &dyn Shell, path: &str) -> Result<Applied> {
    let list = format!("~/.kor-perms-{}.txt", stamp());
    let cmd = format!(
        "cd {p} && find . -printf '%m %p\\n' > {l} && find . -type d -exec chmod 755 {{}} + && \
         find . -type f -exec chmod 644 {{}} + && if [ -f wp-config.php ]; then chmod 640 wp-config.php; fi && \
         wc -l < {l}",
        p = q(path),
        l = q(&list)
    );
    let n = exec(sh, &cmd, None).await?;
    Ok(Applied {
        summary: format!("755/644 on {path} ({} entries)", n.trim()),
        undo: Some(Task::RestoreModes {
            path: path.to_owned(),
            list,
        }),
        ..Applied::default()
    })
}

const CLEAN_LIST: &str = "for d in \"$HOME/tmp\" \"$HOME/.cache\" \"$HOME/.trash\"; do [ -d \"$d\" ] && du -sm \"$d\" 2>/dev/null; done; \
find \"$HOME\" -xdev -type d -path '*/wp-content/cache' -prune -exec du -sm {} + 2>/dev/null | head -50; \
find \"$HOME\" -xdev -type f \\( -name '*.log' -o -name error_log -o -name 'core.[0-9]*' \\) -size +50M -mtime +30 \
-printf '%s %p\\n' 2>/dev/null | awk '{printf \"%d %s\\n\", $1/1048576, $2}' | head -200";

/// `MB path` lines.
fn candidates(text: &str) -> Vec<(u64, String)> {
    text.lines()
        .filter_map(|l| {
            let (n, p) = l.trim().split_once(char::is_whitespace)?;
            Some((n.parse().ok()?, p.trim().to_owned()))
        })
        .collect()
}

async fn clean(sh: &dyn Shell, paths: &[String]) -> Result<Applied> {
    let found = candidates(&exec(sh, CLEAN_LIST, None).await?);
    if paths.is_empty() {
        let total: u64 = found.iter().map(|(n, _)| n).sum();
        return Ok(Applied {
            summary: format!("{} candidates, {total} MB", found.len()),
            output: found.iter().map(|(n, p)| format!("{n} MB  {p}")).collect(),
            ..Applied::default()
        });
    }
    // Only paths from the fresh candidate list may be deleted: nothing else on the server.
    let chosen: Vec<&(u64, String)> = found.iter().filter(|(_, p)| paths.contains(p)).collect();
    if chosen.len() != paths.len() {
        return Err("some paths are not clean-up candidates (run the list again)".into());
    }
    let mut cmd = String::new();
    for (_, p) in &chosen {
        if p.ends_with("/tmp")
            || p.ends_with("/.cache")
            || p.ends_with("/.trash")
            || p.ends_with("/cache")
        {
            let _ = write!(cmd, "find {} -mindepth 1 -delete; ", sh_quote(p));
        } else {
            let _ = write!(cmd, "rm -f {}; ", sh_quote(p));
        }
    }
    exec(sh, &cmd, None).await?;
    let total: u64 = chosen.iter().map(|(n, _)| n).sum();
    Ok(Applied {
        summary: format!("{} paths cleaned, ≈{total} MB", chosen.len()),
        ..Applied::default()
    })
}

async fn cron(sh: &dyn Shell, schedule: &str, command: &str) -> Result<Applied> {
    let id = stamp();
    let line = format!("{schedule} {command} # kor:{id}");
    exec(
        sh,
        &format!(
            "(crontab -l 2>/dev/null; echo {}) | crontab -",
            sh_quote(&line)
        ),
        None,
    )
    .await?;
    Ok(Applied {
        summary: format!("cron added: {schedule} {command}"),
        undo: Some(Task::RemoveCron { id }),
        ..Applied::default()
    })
}

async fn php_ini(sh: &dyn Shell, root: &str, key: &str, value: &str) -> Result<Applied> {
    let file = format!("{}/.user.ini", root.trim_end_matches('/'));
    let backup = format!("{file}.kor-{}", stamp());
    let pat = key.replace('.', "\\.");
    let cmd = format!(
        "touch {f} && cp -p {f} {b} && grep -v -E {re} {b} > {f}.new; echo {line} >> {f}.new && mv {f}.new {f}",
        f = q(&file),
        b = q(&backup),
        re = sh_quote(&format!("^[[:space:]]*{pat}[[:space:]]*=")),
        line = sh_quote(&format!("{key} = {value}")),
    );
    exec(sh, &cmd, None).await?;
    Ok(Applied {
        summary: format!("{file}: {key} = {value}"),
        undo: Some(Task::RestoreFile {
            path: file,
            backup,
            reload: None,
        }),
        ..Applied::default()
    })
}

/// Files (and the WordPress database when WP-CLI is there) into `~/kor-backups/`.
pub(crate) async fn backup(sh: &dyn Shell, label: &str, root: Option<&str>) -> Result<Applied> {
    let base = format!("~/kor-backups/{label}-{}", stamp());
    let src = root.unwrap_or("~");
    let cmd = format!(
        "mkdir -p \"$HOME/kor-backups\" && tar czf {a} --exclude=kor-backups -C {s} . && \
         if [ -f {s}/wp-config.php ] && command -v wp >/dev/null; then wp --path={s} db export {d} --quiet; fi; \
         ls -1 {a} {d} 2>/dev/null",
        a = q(&format!("{base}.tar.gz")),
        d = q(&format!("{base}.sql")),
        s = q(src),
    );
    let out = exec(sh, &cmd, None).await?;
    Ok(Applied {
        summary: format!("backup {base}.tar.gz"),
        output: out.lines().map(str::to_owned).collect(),
        ..Applied::default()
    })
}

async fn restore(sh: &dyn Shell, archive: &str, domain: &str, root: &str) -> Result<Applied> {
    if !(archive.starts_with("~/kor-backups/") && archive.ends_with(".tar.gz")) {
        return Err("only archives Kor made in ~/kor-backups/ can be restored".into());
    }
    // The current state is saved first, so the restore itself can be undone by hand.
    let safety = backup(sh, &format!("{domain}-before-restore"), Some(root)).await?;
    exec(sh, &format!("tar xzf {} -C {}", q(archive), q(root)), None).await?;
    let sql = archive.trim_end_matches(".tar.gz").to_owned() + ".sql";
    let _ = exec(
        sh,
        &format!(
            "if [ -f {s} ] && command -v wp >/dev/null; then wp --path={r} db import {s} --quiet; fi",
            s = q(&sql),
            r = q(root)
        ),
        None,
    )
    .await;
    Ok(Applied {
        summary: format!("{archive} → {root}"),
        output: safety.output,
        ..Applied::default()
    })
}

/// Plain Linux server: nginx or Apache, php-fpm, certbot, MySQL/MariaDB.
pub struct Plain {
    name: String,
    sh: Arc<dyn Shell>,
}

impl std::fmt::Debug for Plain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plain")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Plain {
    pub fn new(name: &str, sh: Arc<dyn Shell>) -> Self {
        Self {
            name: name.to_owned(),
            sh,
        }
    }
}

/// One inventory script; sections are separated by `@@name`.
const INVENTORY: &str = "echo @@php; ls -1 /etc/php 2>/dev/null; ls -1d /opt/remi/php* /opt/cpanel/ea-php* 2>/dev/null; \
echo @@nginx; (sudo -n nginx -T 2>/dev/null || nginx -T 2>/dev/null) | grep -E '^\\s*(server_name|root|fastcgi_pass)\\s' ; \
echo @@apache; (sudo -n apachectl -S 2>/dev/null || apache2ctl -S 2>/dev/null) | grep -E 'namevhost|alias' ; \
echo @@db; sudo -n mysql -N -e 'SHOW DATABASES' 2>/dev/null; \
echo @@certs; for c in /etc/letsencrypt/live/*/cert.pem; do [ -r \"$c\" ] && echo \"$(basename $(dirname $c)) $(openssl x509 -enddate -noout -in $c | cut -d= -f2)\"; done 2>/dev/null; \
echo @@disk; df -Pm / | tail -1";

fn sections(text: &str) -> std::collections::HashMap<&str, Vec<&str>> {
    let mut out = std::collections::HashMap::new();
    let mut cur = "";
    for l in text.lines() {
        if let Some(name) = l.strip_prefix("@@") {
            cur = name.trim();
            out.entry(cur).or_insert_with(Vec::new);
        } else if !cur.is_empty() && !l.trim().is_empty() {
            out.entry(cur).or_insert_with(Vec::new).push(l.trim());
        }
    }
    out
}

pub(crate) fn parse_plain(host: &str, text: &str) -> Inventory {
    let s = sections(text);
    let get = |k: &str| s.get(k).cloned().unwrap_or_default();
    let mut inv = Inventory {
        host: host.to_owned(),
        kind: Some(Kind::Ssh),
        ..Inventory::default()
    };
    for l in get("php") {
        if let Some(v) = php_version(l.rsplit('/').next().unwrap_or(l))
            && !inv.php_installed.contains(&v)
        {
            inv.php_installed.push(v);
        }
    }
    // nginx: server_name … root … fastcgi_pass, in file order.
    let mut current: Vec<String> = Vec::new();
    for l in get("nginx") {
        let l = l.trim_end_matches(';');
        let mut w = l.split_whitespace();
        match w.next() {
            Some("server_name") => {
                current = w
                    .filter(|n| super::super::valid_domain(n) && !n.starts_with("www."))
                    .map(str::to_owned)
                    .collect();
                for n in &current {
                    if inv.domain(n).is_none() {
                        inv.domains.push(Domain {
                            name: n.clone(),
                            kind: "site".into(),
                            ..Domain::default()
                        });
                    }
                }
            }
            Some("root") => {
                let r = w.next().map(str::to_owned);
                for d in inv
                    .domains
                    .iter_mut()
                    .filter(|d| current.contains(&d.name) && d.docroot.is_none())
                {
                    d.docroot.clone_from(&r);
                }
            }
            Some("fastcgi_pass") => {
                let v = w.next().and_then(php_version);
                for d in inv
                    .domains
                    .iter_mut()
                    .filter(|d| current.contains(&d.name) && d.php.is_none())
                {
                    d.php.clone_from(&v);
                }
            }
            _ => {}
        }
    }
    for l in get("apache") {
        let mut w = l.split_whitespace();
        let name = match (w.next(), w.next()) {
            (Some("port"), Some(_)) => w.nth(1),
            (Some("alias"), n) => n,
            _ => None,
        };
        if let Some(n) = name.filter(|n| super::super::valid_domain(n) && !n.starts_with("www."))
            && inv.domain(n).is_none()
        {
            inv.domains.push(Domain {
                name: n.to_owned(),
                kind: "site".into(),
                ..Domain::default()
            });
        }
    }
    inv.databases = get("db")
        .into_iter()
        .filter(|d| {
            !matches!(
                *d,
                "information_schema" | "performance_schema" | "mysql" | "sys"
            )
        })
        .map(str::to_owned)
        .collect();
    if s.get("db").is_none_or(Vec::is_empty) {
        inv.notes
            .push("databases: needs passwordless `sudo -n mysql`".into());
    }
    for l in get("certs") {
        if let Some((d, date)) = l.split_once(' ') {
            let t = chrono::NaiveDateTime::parse_from_str(
                date.trim().trim_end_matches(" GMT"),
                "%b %e %H:%M:%S %Y",
            )
            .ok()
            .map(|t| t.and_utc().timestamp());
            inv.certs.push(super::Cert {
                domain: d.to_owned(),
                not_after: t,
                issuer: Some("Let's Encrypt (certbot)".into()),
                source: "panel".into(),
                error: None,
            });
        }
    }
    if let Some(l) = get("disk").first() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if let (Some(total), Some(used)) = (
            f.get(1).and_then(|x| x.parse().ok()),
            f.get(2).and_then(|x| x.parse().ok()),
        ) {
            inv.disk = Some(Disk {
                used_mb: used,
                limit_mb: Some(total),
            });
        }
    }
    inv
}

#[async_trait]
impl Panel for Plain {
    async fn inventory(&self) -> Result<Inventory> {
        Ok(parse_plain(
            &self.name,
            &exec(self.sh.as_ref(), INVENTORY, None).await?,
        ))
    }

    #[allow(clippy::too_many_lines)]
    async fn apply(&self, task: &Task, ctx: &ApplyCtx<'_>) -> Result<Applied> {
        let sh = self.sh.as_ref();
        if let Some(r) = generic(sh, task, ctx).await {
            return r;
        }
        match task {
            Task::Ssl { domain } => {
                let email = ctx
                    .email
                    .ok_or("Let's Encrypt needs the host's email (kor host connect … --email)")?;
                let cmd = format!(
                    "if command -v nginx >/dev/null; then P=--nginx; else P=--apache; fi; \
                     sudo -n certbot $P --non-interactive --agree-tos -m {} -d {}",
                    sh_quote(email),
                    sh_quote(domain)
                );
                exec(sh, &cmd, None).await?;
                Ok(Applied {
                    summary: format!("certificate issued for {domain}"),
                    ..Applied::default()
                })
            }
            Task::Database { name, user } => {
                let pw = ctx.password.ok_or("no password")?;
                let sql = format!(
                    "CREATE DATABASE `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
                     CREATE USER '{user}'@'localhost' IDENTIFIED BY '{pw}';\n\
                     GRANT ALL PRIVILEGES ON `{name}`.* TO '{user}'@'localhost';\nFLUSH PRIVILEGES;\n"
                );
                exec(sh, "sudo -n mysql", Some(sql.as_bytes())).await?;
                Ok(Applied {
                    summary: format!("database {name}, user {user}"),
                    undo: Some(Task::RemoveDatabase {
                        name: name.clone(),
                        user: user.clone(),
                    }),
                    ..Applied::default()
                })
            }
            Task::RemoveDatabase { name, user } => {
                let sql = format!(
                    "DROP DATABASE IF EXISTS `{name}`;\nDROP USER IF EXISTS '{user}'@'localhost';\n"
                );
                exec(sh, "sudo -n mysql", Some(sql.as_bytes())).await?;
                Ok(Applied {
                    summary: format!("database {name} removed"),
                    ..Applied::default()
                })
            }
            Task::Subdomain { sub, domain, dir } => {
                let fqdn = format!("{sub}.{domain}");
                let root = dir.clone().unwrap_or_else(|| format!("/var/www/{fqdn}"));
                let conf = format!(
                    "server {{\n    listen 80;\n    server_name {fqdn};\n    root {root};\n    index index.php index.html;\n    \
                     location / {{ try_files $uri $uri/ /index.php?$args; }}\n    \
                     location ~ \\.php$ {{ include snippets/fastcgi-php.conf; fastcgi_pass unix:$SOCK; }}\n}}\n"
                );
                let cmd = format!(
                    "command -v nginx >/dev/null || {{ echo 'only nginx servers'; exit 3; }}; \
                     SOCK=$(ls -1 /run/php/php*-fpm.sock 2>/dev/null | sort -V | tail -1); \
                     sudo -n mkdir -p {r} && printf '%s' \"$(cat)\" | sed \"s#\\$SOCK#$SOCK#\" | sudo -n tee {a} >/dev/null && \
                     sudo -n ln -sf {a} {e} && sudo -n nginx -t && sudo -n systemctl reload nginx",
                    r = sh_quote(&root),
                    a = sh_quote(&format!("/etc/nginx/sites-available/{fqdn}")),
                    e = sh_quote(&format!("/etc/nginx/sites-enabled/{fqdn}")),
                );
                exec(sh, &cmd, Some(conf.as_bytes())).await?;
                Ok(Applied {
                    summary: format!("nginx site {fqdn} → {root}"),
                    undo: Some(Task::RemoveSubdomain {
                        sub: sub.clone(),
                        domain: domain.clone(),
                    }),
                    ..Applied::default()
                })
            }
            Task::RemoveSubdomain { sub, domain } => {
                let fqdn = format!("{sub}.{domain}");
                exec(
                    sh,
                    &format!(
                        "sudo -n rm -f {} {} && sudo -n nginx -t && sudo -n systemctl reload nginx",
                        sh_quote(&format!("/etc/nginx/sites-enabled/{fqdn}")),
                        sh_quote(&format!("/etc/nginx/sites-available/{fqdn}"))
                    ),
                    None,
                )
                .await?;
                Ok(Applied {
                    summary: format!("nginx site {fqdn} removed"),
                    ..Applied::default()
                })
            }
            Task::Php { domain, version } => {
                let cur = ctx
                    .inv
                    .domain(domain)
                    .and_then(|d| d.php.clone())
                    .ok_or_else(|| format!("{domain}: no php-fpm vhost found (nginx only)"))?;
                let backup = format!("/root/kor-{domain}-{}.conf", stamp());
                let cmd = format!(
                    "test -S /run/php/php{v}-fpm.sock || {{ echo 'php{v}-fpm is not running'; exit 3; }}; \
                     F=$(sudo -n grep -lE 'server_name[^;]*[[:space:]]{d}[[:space:];]' /etc/nginx/sites-enabled/* /etc/nginx/conf.d/*.conf 2>/dev/null | head -1); \
                     [ -n \"$F\" ] || {{ echo 'vhost not found'; exit 3; }}; F=$(readlink -f \"$F\"); \
                     sudo -n cp -p \"$F\" {b} && sudo -n sed -i 's#php{c}-fpm.sock#php{v}-fpm.sock#g' \"$F\" && \
                     sudo -n nginx -t && sudo -n systemctl reload nginx && echo \"$F\"",
                    v = version,
                    c = cur,
                    d = domain.replace('.', "\\."),
                    b = sh_quote(&backup),
                );
                let file = exec(sh, &cmd, None)
                    .await?
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or_default()
                    .to_owned();
                Ok(Applied {
                    summary: format!("{domain}: PHP {cur} → {version}"),
                    undo: Some(Task::RestoreFile {
                        path: file,
                        backup,
                        reload: Some("sudo -n nginx -t && sudo -n systemctl reload nginx".into()),
                    }),
                    ..Applied::default()
                })
            }
            Task::RestoreFile {
                path,
                backup,
                reload,
            } => {
                let mut cmd = format!("sudo -n cp -p {} {}", sh_quote(backup), sh_quote(path));
                if let Some(r) = reload {
                    let _ = write!(cmd, " && {r}");
                }
                exec(sh, &cmd, None).await?;
                Ok(Applied {
                    summary: format!("restored {path}"),
                    ..Applied::default()
                })
            }
            Task::Sftp { user, path, .. } => {
                let pw = ctx.password.ok_or("no password")?;
                let conf = "Match Group kor-sftp\n    ForceCommand internal-sftp\n    AllowTcpForwarding no\n    X11Forwarding no\n";
                let cmd = format!(
                    "sudo -n groupadd -f kor-sftp && sudo -n useradd -M -d {p} -s /usr/sbin/nologin -G kor-sftp {u} && \
                     printf '%s:%s\\n' {u} \"$(cat)\" | sudo -n chpasswd && \
                     if [ ! -f /etc/ssh/sshd_config.d/kor-sftp.conf ]; then printf {c} | sudo -n tee /etc/ssh/sshd_config.d/kor-sftp.conf >/dev/null && \
                     sudo -n sshd -t && (sudo -n systemctl reload ssh 2>/dev/null || sudo -n systemctl reload sshd); fi",
                    p = sh_quote(path),
                    u = sh_quote(user),
                    c = sh_quote(conf),
                );
                exec(sh, &cmd, Some(pw.as_bytes())).await?;
                Ok(Applied {
                    summary: format!("SFTP user {user} → {path}"),
                    undo: Some(Task::RemoveSftp {
                        user: user.clone(),
                        domain: None,
                    }),
                    ..Applied::default()
                })
            }
            Task::RemoveSftp { user, .. } => {
                exec(sh, &format!("sudo -n userdel {}", sh_quote(user)), None).await?;
                Ok(Applied {
                    summary: format!("SFTP user {user} removed"),
                    ..Applied::default()
                })
            }
            Task::Email { .. } | Task::RemoveEmail { .. } => Err(
                "plain servers have no mail system Kor manages; use the panel or mail provider"
                    .into(),
            ),
            other => Err(format!(
                "{} is not available on plain SSH servers",
                other.id()
            )),
        }
    }

    fn shell(&self) -> Option<&Arc<dyn Shell>> {
        Some(&self.sh)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::super::tests::FakeShell;
    use super::*;

    const OUT: &str = "@@php\n8.1\n8.3\n@@nginx\nserver_name shop.example.com www.shop.example.com;\nroot /var/www/shop;\n\
fastcgi_pass unix:/run/php/php8.1-fpm.sock;\n@@apache\n@@db\nshop\nmysql\n@@certs\nshop.example.com Mar 15 12:00:00 2027 GMT\n@@disk\n/dev/sda1 40000 12000 28000 30% /\n";

    #[test]
    fn plain_inventory_parses() {
        let inv = parse_plain("vps", OUT);
        assert_eq!(inv.php_installed, ["8.1", "8.3"]);
        let d = inv.domain("shop.example.com").unwrap();
        assert_eq!(d.docroot.as_deref(), Some("/var/www/shop"));
        assert_eq!(d.php.as_deref(), Some("8.1"));
        assert_eq!(inv.databases, ["shop"]);
        assert_eq!(inv.certs[0].not_after, Some(1_805_112_000));
        assert_eq!(inv.disk.as_ref().unwrap().used_mb, 12000.0);
    }

    #[tokio::test]
    async fn database_password_goes_on_stdin_and_perms_can_be_undone() {
        let sh = Arc::new(FakeShell {
            answers: vec![("@@php", 0, OUT.into()), ("wc -l", 0, "42\n".into())],
            ..FakeShell::default()
        });
        let p = Plain::new("vps", sh.clone());
        let inv = p.inventory().await.unwrap();
        let ctx = ApplyCtx {
            inv: &inv,
            email: None,
            password: Some("S3cret"),
        };
        let a = p
            .apply(
                &Task::Database {
                    name: "shop2".into(),
                    user: "shop2".into(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert!(matches!(a.undo, Some(Task::RemoveDatabase { .. })));
        let log = sh.log.lock().unwrap().clone();
        let (cmd, stdin) = log.last().unwrap();
        assert_eq!(cmd, "sudo -n mysql");
        assert!(stdin.as_deref().unwrap().contains("S3cret"));
        assert!(!log.iter().any(|(c, _)| c.contains("S3cret")));
        let a = p
            .apply(
                &Task::Perms {
                    path: "~/public_html".into(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert!(a.summary.contains("42"));
        assert!(matches!(a.undo, Some(Task::RestoreModes { .. })));
        assert!(sh.commands().iter().any(|c| c.contains("chmod 755")));
    }

    #[tokio::test]
    async fn clean_only_deletes_listed_candidates() {
        let sh = Arc::new(FakeShell {
            answers: vec![(
                "du -sm",
                0,
                "120 /home/u/tmp\n300 /home/u/logs/old.log\n".into(),
            )],
            ..FakeShell::default()
        });
        let inv = Inventory::default();
        let ctx = ApplyCtx {
            inv: &inv,
            email: None,
            password: None,
        };
        let list = generic(sh.as_ref(), &Task::Clean { paths: vec![] }, &ctx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(list.output.len(), 2);
        assert!(
            generic(
                sh.as_ref(),
                &Task::Clean {
                    paths: vec!["/etc".into()]
                },
                &ctx
            )
            .await
            .unwrap()
            .is_err()
        );
        let done = generic(
            sh.as_ref(),
            &Task::Clean {
                paths: vec!["/home/u/logs/old.log".into()],
            },
            &ctx,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(done.summary.contains("300"));
        assert!(
            sh.commands()
                .last()
                .unwrap()
                .contains("rm -f '/home/u/logs/old.log'")
        );
    }

    #[test]
    fn validators() {
        assert!(check_cron("*/5 * * * *", "php ~/cron.php").is_ok());
        assert!(check_cron("* * *", "x").is_err());
        assert!(check_ini("memory_limit", "256M").is_ok());
        assert!(check_ini("memory_limit", "1;\nx").is_err());
        assert_eq!(q("~/a b"), "\"$HOME\"/'a b'");
    }
}
