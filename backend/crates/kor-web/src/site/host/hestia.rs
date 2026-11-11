//! HestiaCP API (`POST /api/`, `hash=ACCESS_KEY:SECRET_KEY`, one `v-*` command per call).

use std::sync::Arc;

use async_trait::async_trait;
use kor_agent::remote::Shell;
use serde_json::Value;

use super::{Applied, ApplyCtx, Disk, Domain, Inventory, Kind, Panel, Task, php_version, shell};
use crate::site::Result;
use crate::site::http::Http;

pub struct Hestia {
    name: String,
    http: Http,
    url: String,
    user: String,
    hash: String,
    sh: Option<Arc<dyn Shell>>,
}

impl Hestia {
    pub fn new(
        name: &str,
        http: Http,
        url: String,
        user: String,
        hash: String,
        sh: Option<Arc<dyn Shell>>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            http,
            url,
            user,
            hash,
            sh,
        }
    }

    /// `returncode=no` returns the command output; `json` as the last argument makes it JSON.
    async fn cmd(&self, cmd: &str, args: &[&str]) -> Result<String> {
        let names: Vec<String> = (1..=args.len()).map(|i| format!("arg{i}")).collect();
        let mut pairs: Vec<(&str, &str)> =
            vec![("hash", &self.hash), ("returncode", "no"), ("cmd", cmd)];
        pairs.extend(names.iter().map(String::as_str).zip(args.iter().copied()));
        let r = self
            .http
            .post_form(&format!("{}/api/", self.url), &[], &pairs)
            .await?;
        if !r.ok() {
            return Err(format!("{}: Hestia {cmd}: HTTP {}", self.name, r.status));
        }
        let b = r.body.trim();
        // Errors come back as "Error: …" text (or a non-zero code with returncode=yes).
        if b.starts_with("Error") || b.starts_with("ERROR") {
            return Err(format!("{cmd}: {b}"));
        }
        Ok(b.to_owned())
    }

    async fn json(&self, cmd: &str, args: &[&str]) -> Result<Value> {
        let mut a = args.to_vec();
        a.push("json");
        let t = self.cmd(cmd, &a).await?;
        serde_json::from_str(&t).map_err(|e| format!("{cmd}: {e}"))
    }

    fn u(&self) -> &str {
        &self.user
    }
}

#[async_trait]
impl Panel for Hestia {
    async fn inventory(&self) -> Result<Inventory> {
        let mut inv = Inventory {
            host: self.name.clone(),
            kind: Some(Kind::Hestia),
            ..Inventory::default()
        };
        let web = self.json("v-list-web-domains", &[self.u()]).await?;
        for (name, d) in web.as_object().into_iter().flatten() {
            inv.domains.push(Domain {
                name: name.clone(),
                kind: "site".into(),
                docroot: Some(
                    d["DOCUMENT_ROOT"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map_or_else(
                            || format!("/home/{}/web/{name}/public_html", self.user),
                            str::to_owned,
                        ),
                ),
                php: d["BACKEND"].as_str().and_then(php_version),
                account: None,
            });
        }
        match self.json("v-list-databases", &[self.u()]).await {
            Ok(v) => {
                inv.databases = v
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
            }
            Err(e) => inv.notes.push(e),
        }
        match self.json("v-list-user", &[self.u()]).await {
            Ok(v) => {
                let u = &v[&self.user];
                let used = u["U_DISK"].as_str().and_then(|x| x.parse().ok());
                let limit = u["DISK_QUOTA"].as_str().and_then(|x| x.parse().ok());
                if let Some(used) = used {
                    inv.disk = Some(Disk {
                        used_mb: used,
                        limit_mb: limit,
                    });
                }
                inv.email_accounts = u["U_MAIL_ACCOUNTS"].as_str().and_then(|x| x.parse().ok());
            }
            Err(e) => inv.notes.push(e),
        }
        match self.json("v-list-web-templates-backend", &[]).await {
            Ok(v) => {
                for t in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if let Some(p) = php_version(t)
                        && !inv.php_installed.contains(&p)
                    {
                        inv.php_installed.push(p);
                    }
                }
            }
            Err(e) => inv.notes.push(e),
        }
        Ok(inv)
    }

    #[allow(clippy::too_many_lines)]
    async fn apply(&self, task: &Task, ctx: &ApplyCtx<'_>) -> Result<Applied> {
        if let Some(sh) = &self.sh
            && let Some(r) = shell::generic(sh.as_ref(), task, ctx).await
        {
            return r;
        }
        let u = self.u();
        let done = |summary: String, undo: Option<Task>| {
            Ok(Applied {
                summary,
                undo,
                ..Applied::default()
            })
        };
        match task {
            Task::Ssl { domain } => {
                self.cmd("v-add-letsencrypt-domain", &[u, domain]).await?;
                done(format!("Let's Encrypt certificate for {domain}"), None)
            }
            Task::Subdomain { sub, domain, .. } => {
                let fqdn = format!("{sub}.{domain}");
                self.cmd("v-add-web-domain", &[u, &fqdn]).await?;
                done(
                    fqdn,
                    Some(Task::RemoveSubdomain {
                        sub: sub.clone(),
                        domain: domain.clone(),
                    }),
                )
            }
            Task::RemoveSubdomain { sub, domain } => {
                self.cmd("v-delete-web-domain", &[u, &format!("{sub}.{domain}")])
                    .await?;
                done(format!("{sub}.{domain} removed"), None)
            }
            Task::Database { name, user } => {
                let pw = ctx.password.ok_or("no password")?;
                self.cmd("v-add-database", &[u, name, user, pw, "mysql"])
                    .await?;
                done(
                    format!("database {u}_{name}, user {u}_{user}"),
                    Some(Task::RemoveDatabase {
                        name: format!("{u}_{name}"),
                        user: user.clone(),
                    }),
                )
            }
            Task::RemoveDatabase { name, .. } => {
                self.cmd("v-delete-database", &[u, name]).await?;
                done(format!("database {name} removed"), None)
            }
            Task::Email { address, quota_mb } => {
                let pw = ctx.password.ok_or("no password")?;
                let (local, dom) = address.split_once('@').ok_or("invalid address")?;
                let q = quota_mb.to_string();
                self.cmd("v-add-mail-account", &[u, dom, local, pw, &q])
                    .await?;
                done(
                    format!("mailbox {address}"),
                    Some(Task::RemoveEmail {
                        address: address.clone(),
                    }),
                )
            }
            Task::RemoveEmail { address } => {
                let (local, dom) = address.split_once('@').ok_or("invalid address")?;
                self.cmd("v-delete-mail-account", &[u, dom, local]).await?;
                done(format!("mailbox {address} removed"), None)
            }
            Task::Backup { .. } => {
                self.cmd("v-backup-user", &[u]).await?;
                done(format!("Hestia backup of {u}"), None)
            }
            Task::Cron { schedule, command } => {
                let p: Vec<&str> = schedule.split_whitespace().collect();
                self.cmd(
                    "v-add-cron-job",
                    &[u, p[0], p[1], p[2], p[3], p[4], command],
                )
                .await?;
                let jobs = self
                    .json("v-list-cron-jobs", &[u])
                    .await
                    .unwrap_or_default();
                let id = jobs
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(_, j)| j["CMD"].as_str() == Some(command.as_str()))
                    .map(|(k, _)| k.clone())
                    .max_by_key(|k| k.parse::<u64>().unwrap_or(0));
                done(
                    format!("cron added: {schedule} {command}"),
                    id.map(|id| Task::RemoveCron { id }),
                )
            }
            Task::RemoveCron { id } => {
                self.cmd("v-delete-cron-job", &[u, id]).await?;
                done(format!("cron {id} removed"), None)
            }
            Task::Php { domain, version } => {
                let prev = ctx.inv.domain(domain).and_then(|d| d.php.clone());
                let tpl = format!("PHP-{}", version.replace('.', "_"));
                self.cmd("v-change-web-domain-backend-tpl", &[u, domain, &tpl])
                    .await?;
                done(
                    format!(
                        "{domain}: PHP {} → {version}",
                        prev.as_deref().unwrap_or("?")
                    ),
                    prev.map(|v| Task::Php {
                        domain: domain.clone(),
                        version: v,
                    }),
                )
            }
            Task::Sftp { user, path, domain } => {
                let pw = ctx.password.ok_or("no password")?;
                let dom = domain
                    .as_deref()
                    .or(ctx.inv.main_domain())
                    .ok_or("no domain")?;
                self.cmd("v-add-web-domain-ftp", &[u, dom, user, pw, path])
                    .await?;
                done(
                    format!("FTP/SFTP user {u}_{user} → {path}"),
                    Some(Task::RemoveSftp {
                        user: format!("{u}_{user}"),
                        domain: Some(dom.to_owned()),
                    }),
                )
            }
            Task::RemoveSftp { user, domain } => {
                let dom = domain
                    .as_deref()
                    .or(ctx.inv.main_domain())
                    .ok_or("no domain")?;
                self.cmd("v-delete-web-domain-ftp", &[u, dom, user]).await?;
                done(format!("FTP user {user} removed"), None)
            }
            other => Err(format!(
                "{}: needs SSH on this host (kor host connect … --ssh)",
                other.id()
            )),
        }
    }

    fn shell(&self) -> Option<&Arc<dyn Shell>> {
        self.sh.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn inventory_and_php_template() {
        let s = MockServer::start().await;
        let reply = |b: &str| ResponseTemplate::new(200).set_body_string(b.to_owned());
        Mock::given(path("/api/"))
            .and(body_string_contains("cmd=v-list-web-domains"))
            .respond_with(reply(r#"{"acme.com":{"BACKEND":"PHP-8_1","SSL":"yes"}}"#))
            .mount(&s)
            .await;
        Mock::given(path("/api/"))
            .and(body_string_contains("cmd=v-list-databases"))
            .respond_with(reply(r#"{"acme_wp":{"TYPE":"mysql"}}"#))
            .mount(&s)
            .await;
        Mock::given(path("/api/"))
            .and(body_string_contains("cmd=v-list-user"))
            .respond_with(reply(
                r#"{"acme":{"U_DISK":"250","DISK_QUOTA":"unlimited","U_MAIL_ACCOUNTS":"4"}}"#,
            ))
            .mount(&s)
            .await;
        Mock::given(path("/api/"))
            .and(body_string_contains("cmd=v-list-web-templates-backend"))
            .respond_with(reply(r#"["default","PHP-8_1","PHP-8_3"]"#))
            .mount(&s)
            .await;
        Mock::given(path("/api/"))
            .and(body_string_contains("arg3=PHP-8_3"))
            .respond_with(reply(""))
            .expect(1)
            .mount(&s)
            .await;
        let h = Hestia::new(
            "h",
            Http::new().unwrap(),
            s.uri(),
            "acme".into(),
            "A:S".into(),
            None,
        );
        let inv = h.inventory().await.unwrap();
        assert_eq!(inv.domains[0].php.as_deref(), Some("8.1"));
        assert_eq!(
            inv.domains[0].docroot.as_deref(),
            Some("/home/acme/web/acme.com/public_html")
        );
        assert_eq!(inv.php_installed, ["8.1", "8.3"]);
        assert_eq!(inv.disk.as_ref().unwrap().limit_mb, None);
        assert_eq!(inv.email_accounts, Some(4));
        let ctx = ApplyCtx {
            inv: &inv,
            email: None,
            password: None,
        };
        let a = h
            .apply(
                &Task::Php {
                    domain: "acme.com".into(),
                    version: "8.3".into(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(
            a.undo,
            Some(Task::Php {
                domain: "acme.com".into(),
                version: "8.1".into()
            })
        );
    }
}
