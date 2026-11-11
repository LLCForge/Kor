//! Plesk REST API (`X-API-Key`): `/api/v2/domains`, `/api/v2/databases`, and the CLI gateway
//! `/api/v2/cli/<utility>/call` for what REST does not cover (PHP handlers, subdomains, mail,
//! FTP users, Let's Encrypt). Certificates are read from the live TLS handshake.

use std::sync::Arc;

use async_trait::async_trait;
use kor_agent::remote::Shell;
use serde_json::{Value, json};

use super::{Applied, ApplyCtx, Domain, Inventory, Kind, Panel, Task, php_version, shell};
use crate::site::Result;
use crate::site::http::Http;

pub struct Plesk {
    name: String,
    http: Http,
    url: String,
    key: String,
    sh: Option<Arc<dyn Shell>>,
}

impl Plesk {
    pub fn new(name: &str, http: Http, url: String, key: String, sh: Option<Arc<dyn Shell>>) -> Self {
        Self { name: name.to_owned(), http, url, key, sh }
    }

    fn headers(&self) -> [(&'static str, String); 2] {
        [("x-api-key", self.key.clone()), ("accept", "application/json".into())]
    }

    async fn get(&self, p: &str) -> Result<Value> {
        let r = self.http.get_with(&format!("{}/api/v2/{p}", self.url), &self.headers()).await?;
        if !r.ok() {
            return Err(format!("{}: Plesk {p}: HTTP {}", self.name, r.status));
        }
        r.json()
    }

    /// `plesk bin <utility> <params…>` through the REST gateway → stdout.
    async fn cli(&self, utility: &str, params: &[&str]) -> Result<String> {
        let r = self
            .http
            .post_json(
                &format!("{}/api/v2/cli/{utility}/call", self.url),
                &self.headers(),
                &json!({ "params": params }),
            )
            .await?;
        let v = r.json()?;
        if r.ok() && v["code"].as_i64() == Some(0) {
            Ok(v["stdout"].as_str().unwrap_or_default().to_owned())
        } else {
            let msg = v["stderr"].as_str().or(v["message"].as_str()).unwrap_or_default();
            Err(format!("plesk {utility}: {}", msg.trim()))
        }
    }

    /// Installed PHP handlers: (version, handler id).
    async fn handlers(&self) -> Result<Vec<(String, String)>> {
        let out = self.cli("php_handler", &["--list"]).await?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let id = l.split_whitespace().find(|w| w.starts_with("plesk-php") || w.contains("-php"))?;
                Some((php_version(id)?, id.to_owned()))
            })
            .collect())
    }
}

/// `PHP handler: plesk-php83-fastcgi` style lines in `domain --info`.
fn info_php(out: &str) -> Option<String> {
    out.lines()
        .filter(|l| l.to_ascii_lowercase().contains("php"))
        .find_map(|l| l.split_whitespace().find(|w| w.contains("php") && w.chars().any(|c| c.is_ascii_digit())).and_then(php_version))
}

/// Disk usage in MB from `domain --info` (`Disk space used by …: 12.3 M`).
fn info_disk(out: &str) -> Option<f64> {
    let l = out.lines().find(|l| l.contains("Disk space used") || l.contains("Total"))?;
    let v = l.rsplit(':').next()?.trim();
    let (num, unit) = v.split_at(v.find(|c: char| !(c.is_ascii_digit() || c == '.'))?);
    let n: f64 = num.parse().ok()?;
    Some(match unit.trim().chars().next()? {
        'G' => n * 1024.0,
        'K' => n / 1024.0,
        'B' | 'b' => n / 1_048_576.0,
        _ => n,
    })
}

#[async_trait]
impl Panel for Plesk {
    async fn inventory(&self) -> Result<Inventory> {
        let mut inv = Inventory { host: self.name.clone(), kind: Some(Kind::Plesk), ..Inventory::default() };
        for d in self.get("domains").await?.as_array().into_iter().flatten() {
            let name = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = if d["base_domain_id"].as_i64().unwrap_or(0) > 0 { "sub" } else { "site" };
            inv.domains.push(Domain { name, kind: kind.into(), ..Domain::default() });
        }
        match self.get("databases").await {
            Ok(v) => inv.databases = v.as_array().into_iter().flatten().filter_map(|d| d["name"].as_str().map(str::to_owned)).collect(),
            Err(e) => inv.notes.push(e),
        }
        match self.handlers().await {
            Ok(h) => {
                for (v, _) in h {
                    if !inv.php_installed.contains(&v) {
                        inv.php_installed.push(v);
                    }
                }
            }
            Err(e) => inv.notes.push(e),
        }
        let mut used = 0.0;
        for d in &mut inv.domains {
            match self.cli("domain", &["--info", &d.name]).await {
                Ok(out) => {
                    d.php = info_php(&out);
                    used += info_disk(&out).unwrap_or(0.0);
                    d.docroot = out
                        .lines()
                        .find(|l| l.contains("Document root") || l.contains("WWW-Root"))
                        .and_then(|l| l.split_once(':'))
                        .map(|(_, v)| v.trim().to_owned());
                }
                Err(e) => inv.notes.push(e),
            }
        }
        if used > 0.0 {
            inv.disk = Some(super::Disk { used_mb: used, limit_mb: None });
        }
        Ok(inv)
    }

    async fn apply(&self, task: &Task, ctx: &ApplyCtx<'_>) -> Result<Applied> {
        if let Some(sh) = &self.sh
            && let Some(r) = shell::generic(sh.as_ref(), task, ctx).await
        {
            return r;
        }
        let done = |summary: String, undo: Option<Task>| Ok(Applied { summary, undo, ..Applied::default() });
        match task {
            Task::Ssl { domain } => {
                let email = ctx.email.ok_or("Let's Encrypt needs the host's email (--email)")?;
                self.cli("extension", &["--exec", "letsencrypt", "cli.php", "-d", domain, "-m", email]).await?;
                done(format!("Let's Encrypt certificate for {domain}"), None)
            }
            Task::Subdomain { sub, domain, dir } => {
                let mut p = vec!["--create", sub.as_str(), "-domain", domain.as_str()];
                if let Some(d) = dir {
                    p.extend(["-www-root", d.as_str()]);
                }
                self.cli("subdomain", &p).await?;
                done(format!("{sub}.{domain}"), Some(Task::RemoveSubdomain { sub: sub.clone(), domain: domain.clone() }))
            }
            Task::RemoveSubdomain { sub, domain } => {
                self.cli("subdomain", &["--remove", sub, "-domain", domain]).await?;
                done(format!("{sub}.{domain} removed"), None)
            }
            Task::Database { name, user } => {
                let pw = ctx.password.ok_or("no password")?;
                let dom = ctx.inv.main_domain().ok_or("no domain on this server")?;
                self.cli("database", &["--create", name, "-domain", dom, "-type", "mysql"]).await?;
                self.cli("database", &["--create-dbuser", user, "-passwd", pw, "-domain", dom, "-database", name]).await?;
                done(format!("database {name}, user {user}"), Some(Task::RemoveDatabase { name: name.clone(), user: user.clone() }))
            }
            Task::RemoveDatabase { name, .. } => {
                self.cli("database", &["--remove", name]).await?;
                done(format!("database {name} removed"), None)
            }
            Task::Email { address, quota_mb } => {
                let pw = ctx.password.ok_or("no password")?;
                let quota = format!("{quota_mb}M");
                self.cli("mail", &["--create", address, "-mailbox", "true", "-passwd", pw, "-mbox_quota", &quota]).await?;
                done(format!("mailbox {address}"), Some(Task::RemoveEmail { address: address.clone() }))
            }
            Task::RemoveEmail { address } => {
                self.cli("mail", &["--remove", address]).await?;
                done(format!("mailbox {address} removed"), None)
            }
            Task::Php { domain, version } => {
                let h = self.handlers().await?;
                let id = |v: &str| h.iter().find(|(x, id)| x == v && id.contains("fpm")).or_else(|| h.iter().find(|(x, _)| x == v)).map(|(_, id)| id.clone());
                let target = id(version).ok_or_else(|| format!("PHP {version} is not installed on {}", self.name))?;
                let prev = ctx.inv.domain(domain).and_then(|d| d.php.clone());
                self.cli("domain", &["--update", domain, "-php_handler_id", &target]).await?;
                done(
                    format!("{domain}: PHP {} → {version}", prev.as_deref().unwrap_or("?")),
                    prev.map(|v| Task::Php { domain: domain.clone(), version: v }),
                )
            }
            Task::Sftp { user, path, domain } => {
                let pw = ctx.password.ok_or("no password")?;
                let dom = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.cli("ftpsubaccount", &["--create", user, "-passwd", pw, "-home", path, "-domain", dom]).await?;
                done(format!("FTP/SFTP user {user} → {path}"), Some(Task::RemoveSftp { user: user.clone(), domain: Some(dom.to_owned()) }))
            }
            Task::RemoveSftp { user, domain } => {
                let dom = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.cli("ftpsubaccount", &["--remove", user, "-domain", dom]).await?;
                done(format!("FTP user {user} removed"), None)
            }
            Task::Backup { domain } => {
                let d = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.cli("pleskbackup", &["domains-name", d]).await?;
                done(format!("Plesk backup of {d}"), None)
            }
            other => Err(format!("{}: needs SSH on this host (kor host connect … --ssh)", other.id())),
        }
    }

    fn shell(&self) -> Option<&Arc<dyn Shell>> {
        self.sh.as_ref()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn inventory_and_php_switch() {
        let s = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/v2/domains")).and(header("x-api-key", "K"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": 1, "name": "acme.com.tr", "base_domain_id": 0}])))
            .mount(&s).await;
        Mock::given(method("GET")).and(path("/api/v2/databases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"name": "acme_wp"}]))).mount(&s).await;
        Mock::given(path("/api/v2/cli/php_handler/call"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code": 0, "stdout": "plesk-php74-fastcgi 7.4\nplesk-php83-fpm 8.3\n"})))
            .mount(&s).await;
        Mock::given(path("/api/v2/cli/domain/call")).and(body_string_contains("--info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code": 0, "stdout": "WWW-Root: /var/www/vhosts/acme.com.tr/httpdocs\nPHP handler: plesk-php74-fastcgi\nDisk space used by httpdocs: 1.5 G\n"})))
            .mount(&s).await;
        Mock::given(path("/api/v2/cli/domain/call")).and(body_string_contains("plesk-php83-fpm"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code": 0, "stdout": ""})))
            .expect(1).mount(&s).await;
        let p = Plesk::new("plesk", Http::new().unwrap(), s.uri(), "K".into(), None);
        let inv = p.inventory().await.unwrap();
        let d = &inv.domains[0];
        assert_eq!(d.php.as_deref(), Some("7.4"));
        assert_eq!(d.docroot.as_deref(), Some("/var/www/vhosts/acme.com.tr/httpdocs"));
        assert_eq!(inv.disk.as_ref().unwrap().used_mb, 1536.0);
        assert_eq!(inv.php_installed, ["7.4", "8.3"]);
        let ctx = ApplyCtx { inv: &inv, email: None, password: None };
        let a = p.apply(&Task::Php { domain: "acme.com.tr".into(), version: "8.3".into() }, &ctx).await.unwrap();
        assert!(matches!(a.undo, Some(Task::Php { .. })));
        assert!(p.apply(&Task::Php { domain: "acme.com.tr".into(), version: "5.6".into() }, &ctx).await.is_err());
    }
}
