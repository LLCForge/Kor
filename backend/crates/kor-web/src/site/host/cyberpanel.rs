//! CyberPanel through its own `cyberpanel` command-line tool over SSH (`sudo -n`), which covers more
//! than its small HTTP API. Passwords for new accounts go through a root-only temporary file.

use std::sync::Arc;

use async_trait::async_trait;
use kor_agent::remote::{Shell, sh_quote};
use serde_json::Value;

use super::shell::exec;
use super::{Applied, ApplyCtx, Domain, Inventory, Kind, Panel, Task, php_version, shell};
use crate::site::Result;

pub struct CyberPanel {
    name: String,
    sh: Arc<dyn Shell>,
}

impl CyberPanel {
    pub fn new(name: &str, sh: Arc<dyn Shell>) -> Self {
        Self { name: name.to_owned(), sh }
    }

    async fn cli(&self, args: &[&str]) -> Result<String> {
        let line: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
        let out = exec(self.sh.as_ref(), &format!("sudo -n cyberpanel {}", line.join(" ")), None).await?;
        // The tool prints `{"success": 0, "errorMessage": …}` with exit code 0 on failure.
        if let Ok(v) = serde_json::from_str::<Value>(out.trim())
            && v["success"].as_i64() == Some(0)
        {
            return Err(format!("cyberpanel {}: {}", args[0], v["errorMessage"].as_str().unwrap_or("failed")));
        }
        Ok(out)
    }

    /// Same, with `--<flag> <password>` read from stdin so it never appears in a command line we build.
    async fn cli_secret(&self, args: &[&str], flag: &str, password: &str) -> Result<String> {
        let line: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
        let cmd = format!("P=$(cat); sudo -n cyberpanel {} --{flag} \"$P\"", line.join(" "));
        let out = exec(self.sh.as_ref(), &cmd, Some(password.as_bytes())).await?;
        if let Ok(v) = serde_json::from_str::<Value>(out.trim())
            && v["success"].as_i64() == Some(0)
        {
            return Err(format!("cyberpanel {}: {}", args[0], v["errorMessage"].as_str().unwrap_or("failed")));
        }
        Ok(out)
    }
}

/// `listWebsitesJson` prints JSON, sometimes as a JSON string holding JSON.
fn parse_json(out: &str) -> Value {
    let v: Value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    match v {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
        other => other,
    }
}

#[async_trait]
impl Panel for CyberPanel {
    async fn inventory(&self) -> Result<Inventory> {
        let mut inv = Inventory { host: self.name.clone(), kind: Some(Kind::Cyberpanel), ..Inventory::default() };
        let sites = parse_json(&self.cli(&["listWebsitesJson"]).await?);
        for s in sites.as_array().into_iter().flatten() {
            let name = s["domain"].as_str().unwrap_or_default().to_owned();
            inv.domains.push(Domain {
                docroot: Some(format!("/home/{name}/public_html")),
                name,
                kind: "site".into(),
                ..Domain::default()
            });
        }
        let names: Vec<String> = inv.domains.iter().map(|d| d.name.clone()).collect();
        for d in names {
            match self.cli(&["listDatabasesJson", "--databaseWebsite", &d]).await {
                Ok(o) => inv.databases.extend(
                    parse_json(&o).as_array().into_iter().flatten().filter_map(|x| x["dbName"].as_str().map(str::to_owned)),
                ),
                Err(e) => inv.notes.push(e),
            }
        }
        // PHP per site from the OpenLiteSpeed vhost config; installed versions from /usr/local/lsws.
        if let Ok(o) = exec(
            self.sh.as_ref(),
            "ls -1d /usr/local/lsws/lsphp* 2>/dev/null; for f in /usr/local/lsws/conf/vhosts/*/vhost.conf; do \
             echo \"@ $(basename $(dirname $f)) $(grep -m1 -o 'lsphp[0-9]*' $f 2>/dev/null)\"; done",
            None,
        )
        .await
        {
            for l in o.lines() {
                if let Some(rest) = l.strip_prefix("@ ") {
                    let mut w = rest.split_whitespace();
                    if let (Some(site), Some(php)) = (w.next(), w.next())
                        && let Some(d) = inv.domains.iter_mut().find(|d| d.name == site)
                    {
                        d.php = php_version(php);
                    }
                } else if let Some(v) = php_version(l.rsplit('/').next().unwrap_or(l))
                    && !inv.php_installed.contains(&v)
                {
                    inv.php_installed.push(v);
                }
            }
        }
        if let Ok(o) = exec(self.sh.as_ref(), "df -Pm /home | tail -1", None).await {
            let f: Vec<&str> = o.split_whitespace().collect();
            if let (Some(t), Some(u)) = (f.get(1).and_then(|x| x.parse().ok()), f.get(2).and_then(|x| x.parse().ok())) {
                inv.disk = Some(super::Disk { used_mb: u, limit_mb: Some(t) });
            }
        }
        Ok(inv)
    }

    async fn apply(&self, task: &Task, ctx: &ApplyCtx<'_>) -> Result<Applied> {
        if let Some(r) = shell::generic(self.sh.as_ref(), task, ctx).await {
            return r;
        }
        let done = |summary: String, undo: Option<Task>| Ok(Applied { summary, undo, ..Applied::default() });
        match task {
            Task::Ssl { domain } => {
                self.cli(&["issueSSL", "--domainName", domain]).await?;
                done(format!("Let's Encrypt certificate for {domain}"), None)
            }
            Task::Subdomain { sub, domain, .. } => {
                let fqdn = format!("{sub}.{domain}");
                let php = ctx.inv.domain(domain).and_then(|d| d.php.clone()).unwrap_or_else(|| "8.1".into());
                self.cli(&["createChild", "--masterDomain", domain, "--childDomain", &fqdn, "--owner", "admin", "--php", &php]).await?;
                done(fqdn, Some(Task::RemoveSubdomain { sub: sub.clone(), domain: domain.clone() }))
            }
            Task::RemoveSubdomain { sub, domain } => {
                self.cli(&["deleteChild", "--childDomain", &format!("{sub}.{domain}")]).await?;
                done(format!("{sub}.{domain} removed"), None)
            }
            Task::Database { name, user } => {
                let pw = ctx.password.ok_or("no password")?;
                let site = ctx.inv.main_domain().ok_or("no website on this server")?;
                self.cli_secret(&["createDatabase", "--databaseWebsite", site, "--dbName", name, "--dbUsername", user], "dbPassword", pw).await?;
                done(format!("database {name}, user {user}"), Some(Task::RemoveDatabase { name: name.clone(), user: user.clone() }))
            }
            Task::RemoveDatabase { name, .. } => {
                self.cli(&["deleteDatabase", "--dbName", name]).await?;
                done(format!("database {name} removed"), None)
            }
            Task::Email { address, .. } => {
                let pw = ctx.password.ok_or("no password")?;
                let (local, dom) = address.split_once('@').ok_or("invalid address")?;
                self.cli_secret(&["createEmail", "--domainName", dom, "--userName", local], "password", pw).await?;
                done(format!("mailbox {address}"), Some(Task::RemoveEmail { address: address.clone() }))
            }
            Task::RemoveEmail { address } => {
                self.cli(&["deleteEmail", "--email", address]).await?;
                done(format!("mailbox {address} removed"), None)
            }
            Task::Backup { domain } => {
                let d = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.cli(&["createBackup", "--domainName", d]).await?;
                done(format!("CyberPanel backup of {d}"), None)
            }
            Task::Php { domain, version } => {
                let prev = ctx.inv.domain(domain).and_then(|d| d.php.clone());
                self.cli(&["changePHP", "--domainName", domain, "--php", version]).await?;
                done(
                    format!("{domain}: PHP {} → {version}", prev.as_deref().unwrap_or("?")),
                    prev.map(|v| Task::Php { domain: domain.clone(), version: v }),
                )
            }
            Task::Sftp { user, path, domain } => {
                let pw = ctx.password.ok_or("no password")?;
                let d = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.cli_secret(&["createFTPAccount", "--domainName", d, "--userName", user, "--path", path, "--owner", "admin"], "password", pw).await?;
                done(format!("FTP user {user} → {path}"), Some(Task::RemoveSftp { user: user.clone(), domain: Some(d.to_owned()) }))
            }
            Task::RemoveSftp { user, .. } => {
                self.cli(&["deleteFTPAccount", "--userName", user]).await?;
                done(format!("FTP user {user} removed"), None)
            }
            other => Err(format!("{} is not available on CyberPanel", other.id())),
        }
    }

    fn shell(&self) -> Option<&Arc<dyn Shell>> {
        Some(&self.sh)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::FakeShell;
    use super::*;

    #[tokio::test]
    async fn inventory_and_secret_on_stdin() {
        let sh = Arc::new(FakeShell {
            answers: vec![
                ("listWebsitesJson", 0, r#""[{\"domain\": \"acme.com\"}]""#.into()),
                ("listDatabasesJson", 0, r#"[{"dbName": "acme_wp"}]"#.into()),
                ("lsws", 0, "/usr/local/lsws/lsphp81\n/usr/local/lsws/lsphp83\n@ acme.com lsphp81\n".into()),
                ("createEmail", 0, r#"{"success": 0, "errorMessage": "exists"}"#.into()),
            ],
            ..FakeShell::default()
        });
        let c = CyberPanel::new("cp", sh.clone());
        let inv = c.inventory().await.unwrap();
        assert_eq!(inv.domains[0].php.as_deref(), Some("8.1"));
        assert_eq!(inv.databases, ["acme_wp"]);
        assert_eq!(inv.php_installed, ["8.1", "8.3"]);
        let ctx = ApplyCtx { inv: &inv, email: None, password: Some("Pw1") };
        let e = c.apply(&Task::Email { address: "a@acme.com".into(), quota_mb: 0 }, &ctx).await.unwrap_err();
        assert!(e.contains("exists"));
        let log = sh.log.lock().unwrap().clone();
        assert!(log.iter().all(|(c, _)| !c.contains("Pw1")));
        assert_eq!(log.last().unwrap().1.as_deref(), Some("Pw1"));
    }
}
