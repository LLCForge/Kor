//! DirectAdmin legacy API (`CMD_API_*`, HTTP Basic with a login key, `json=yes`).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use kor_agent::remote::Shell;
use serde_json::Value;

use super::{Applied, ApplyCtx, Disk, Domain, Inventory, Kind, Panel, Task, php_version, shell};
use crate::site::Result;
use crate::site::http::Http;

pub struct DirectAdmin {
    name: String,
    http: Http,
    url: String,
    auth: String,
    sh: Option<Arc<dyn Shell>>,
}

/// A JSON list, or an object whose keys are the items (DirectAdmin returns both shapes).
fn items(v: &Value) -> Vec<String> {
    if let Some(a) = v.get("list").and_then(Value::as_array).or(v.as_array()) {
        return a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
    }
    v.as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| !matches!(k.as_str(), "error" | "result" | "success" | "details" | "text"))
        .map(|(k, _)| k.clone())
        .collect()
}

impl DirectAdmin {
    pub fn new(name: &str, http: Http, url: String, user: &str, key: &str, sh: Option<Arc<dyn Shell>>) -> Self {
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{key}"))
        );
        Self { name: name.to_owned(), http, url, auth, sh }
    }

    async fn call(&self, cmd: &str, args: &[(&str, &str)]) -> Result<Value> {
        let mut pairs = vec![("json", "yes")];
        pairs.extend_from_slice(args);
        let r = self
            .http
            .post_form(&format!("{}/{cmd}", self.url), &[("authorization", self.auth.clone())], &pairs)
            .await?;
        if r.status == 401 || r.status == 403 {
            return Err(format!("{}: DirectAdmin refused the login key (HTTP {})", self.name, r.status));
        }
        let v = r.json()?;
        let err = v["error"].as_str().is_some_and(|e| e != "0") || v["error"] == Value::Bool(true);
        if err {
            let detail = v["details"].as_str().or(v["text"].as_str()).unwrap_or("error");
            return Err(format!("{cmd}: {detail}"));
        }
        Ok(v)
    }
}

#[async_trait]
impl Panel for DirectAdmin {
    async fn inventory(&self) -> Result<Inventory> {
        let mut inv = Inventory { host: self.name.clone(), kind: Some(Kind::Directadmin), ..Inventory::default() };
        for d in items(&self.call("CMD_API_SHOW_DOMAINS", &[]).await?) {
            let php = self
                .call("CMD_API_ADDITIONAL_DOMAINS", &[("action", "view"), ("domain", &d)])
                .await
                .ok()
                .and_then(|v| v["php1_ver"].as_str().or(v["php_version"].as_str()).and_then(php_version));
            let docroot = self
                .call("CMD_API_SHOW_USER_CONFIG", &[])
                .await
                .ok()
                .and_then(|v| v["username"].as_str().map(|u| format!("/home/{u}/domains/{d}/public_html")));
            inv.domains.push(Domain { name: d, kind: "site".into(), php, docroot, account: None });
        }
        match self.call("CMD_API_DATABASES", &[]).await {
            Ok(v) => inv.databases = items(&v),
            Err(e) => inv.notes.push(e),
        }
        match self.call("CMD_API_SHOW_USER_USAGE", &[]).await {
            Ok(u) => {
                let used = u["quota"].as_str().and_then(|q| q.parse().ok()).or_else(|| u["quota"].as_f64());
                let limit = self
                    .call("CMD_API_SHOW_USER_CONFIG", &[])
                    .await
                    .ok()
                    .and_then(|c| c["quota"].as_str().and_then(|q| q.parse().ok()));
                if let Some(used) = used {
                    inv.disk = Some(Disk { used_mb: used, limit_mb: limit });
                }
                inv.email_accounts = u["nemails"].as_str().and_then(|n| n.parse().ok());
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
        let done = |summary: String, undo: Option<Task>| Ok(Applied { summary, undo, ..Applied::default() });
        match task {
            Task::Ssl { domain } => {
                self.call(
                    "CMD_API_SSL",
                    &[("domain", domain), ("action", "save"), ("type", "create"), ("request", "letsencrypt"),
                      ("name", domain), ("le_select0", domain), ("keysize", "secp384r1"), ("encryption", "sha256")],
                )
                .await?;
                done(format!("Let's Encrypt certificate for {domain}"), None)
            }
            Task::Subdomain { sub, domain, .. } => {
                self.call("CMD_API_SUBDOMAINS", &[("action", "create"), ("domain", domain), ("subdomain", sub)]).await?;
                done(format!("{sub}.{domain}"), Some(Task::RemoveSubdomain { sub: sub.clone(), domain: domain.clone() }))
            }
            Task::RemoveSubdomain { sub, domain } => {
                self.call("CMD_API_SUBDOMAINS", &[("action", "delete"), ("domain", domain), ("select0", sub), ("contents", "no")]).await?;
                done(format!("{sub}.{domain} removed"), None)
            }
            Task::Database { name, user } => {
                let pw = ctx.password.ok_or("no password")?;
                self.call("CMD_API_DATABASES", &[("action", "create"), ("name", name), ("user", user), ("passwd", pw), ("passwd2", pw)]).await?;
                done(format!("database {name}, user {user}"), Some(Task::RemoveDatabase { name: name.clone(), user: user.clone() }))
            }
            Task::RemoveDatabase { name, .. } => {
                self.call("CMD_API_DATABASES", &[("action", "delete"), ("select0", name)]).await?;
                done(format!("database {name} removed"), None)
            }
            Task::Email { address, quota_mb } => {
                let pw = ctx.password.ok_or("no password")?;
                let (local, dom) = address.split_once('@').ok_or("invalid address")?;
                let q = quota_mb.to_string();
                self.call("CMD_API_POP", &[("action", "create"), ("domain", dom), ("user", local), ("passwd", pw), ("passwd2", pw), ("quota", &q)]).await?;
                done(format!("mailbox {address}"), Some(Task::RemoveEmail { address: address.clone() }))
            }
            Task::RemoveEmail { address } => {
                let (local, dom) = address.split_once('@').ok_or("invalid address")?;
                self.call("CMD_API_POP", &[("action", "delete"), ("domain", dom), ("user", local)]).await?;
                done(format!("mailbox {address} removed"), None)
            }
            Task::Cron { schedule, command } => {
                let p: Vec<&str> = schedule.split_whitespace().collect();
                let before = items(&self.call("CMD_API_CRON_JOBS", &[]).await.unwrap_or_default());
                self.call(
                    "CMD_API_CRON_JOBS",
                    &[("action", "create"), ("minute", p[0]), ("hour", p[1]), ("dayofmonth", p[2]), ("month", p[3]), ("dayofweek", p[4]), ("command", command)],
                )
                .await?;
                let after = items(&self.call("CMD_API_CRON_JOBS", &[]).await.unwrap_or_default());
                let id = after.into_iter().find(|x| !before.contains(x));
                done(format!("cron added: {schedule} {command}"), id.map(|id| Task::RemoveCron { id }))
            }
            Task::RemoveCron { id } => {
                self.call("CMD_API_CRON_JOBS", &[("action", "delete"), ("select0", id)]).await?;
                done(format!("cron {id} removed"), None)
            }
            Task::Php { domain, version } => {
                // php1_select is the index (1-based) of the version in the server's PHP list.
                let v = self.call("CMD_API_SYSTEM_INFO", &[]).await.unwrap_or_default();
                let list: Vec<String> = ["php1_release", "php2_release", "php3_release", "php4_release"]
                    .iter()
                    .filter_map(|k| v[*k].as_str().and_then(php_version))
                    .collect();
                let idx = list
                    .iter()
                    .position(|x| x == version)
                    .ok_or_else(|| format!("PHP {version} is not configured on {}", self.name))?;
                let prev = ctx.inv.domain(domain).and_then(|d| d.php.clone());
                let sel = (idx + 1).to_string();
                self.call("CMD_API_DOMAIN", &[("action", "php_selector"), ("domain", domain), ("php1_select", &sel)]).await?;
                done(
                    format!("{domain}: PHP {} → {version}", prev.as_deref().unwrap_or("?")),
                    prev.map(|v| Task::Php { domain: domain.clone(), version: v }),
                )
            }
            Task::Sftp { user, path, domain } => {
                let pw = ctx.password.ok_or("no password")?;
                let dom = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.call(
                    "CMD_API_FTP",
                    &[("action", "create"), ("domain", dom), ("user", user), ("type", "custom"), ("custom_val", path), ("passwd", pw), ("passwd2", pw)],
                )
                .await?;
                done(format!("FTP/SFTP user {user} → {path}"), Some(Task::RemoveSftp { user: user.clone(), domain: Some(dom.to_owned()) }))
            }
            Task::RemoveSftp { user, domain } => {
                let dom = domain.as_deref().or(ctx.inv.main_domain()).ok_or("no domain")?;
                self.call("CMD_API_FTP", &[("action", "delete"), ("domain", dom), ("select0", user)]).await?;
                done(format!("FTP user {user} removed"), None)
            }
            Task::Backup { .. } => {
                self.call(
                    "CMD_API_SITE_BACKUP",
                    &[("action", "backup"), ("select0", "domain"), ("select1", "subdomain"), ("select2", "email"), ("select3", "ftp"), ("select4", "database")],
                )
                .await?;
                done("DirectAdmin backup started".into(), None)
            }
            other => Err(format!("{}: needs SSH on this host (kor host connect … --ssh)", other.id())),
        }
    }

    fn shell(&self) -> Option<&Arc<dyn Shell>> {
        self.sh.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn inventory_and_subdomain_with_undo() {
        let s = MockServer::start().await;
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("acme:KEY"));
        Mock::given(path("/CMD_API_SHOW_DOMAINS")).and(header("authorization", auth.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"list": ["acme.com"]}))).mount(&s).await;
        Mock::given(path("/CMD_API_ADDITIONAL_DOMAINS"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"php1_ver": "8.1"}))).mount(&s).await;
        Mock::given(path("/CMD_API_SHOW_USER_CONFIG"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"username": "acme", "quota": "2048"}))).mount(&s).await;
        Mock::given(path("/CMD_API_DATABASES")).and(body_string_contains("action=create"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error": "0"}))).expect(1).mount(&s).await;
        Mock::given(path("/CMD_API_DATABASES"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"list": ["acme_wp"]}))).mount(&s).await;
        Mock::given(path("/CMD_API_SHOW_USER_USAGE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"quota": "300", "nemails": "2"}))).mount(&s).await;
        Mock::given(path("/CMD_API_SUBDOMAINS"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error": "1", "details": "Subdomain exists"}))).mount(&s).await;
        let d = DirectAdmin::new("da", Http::new().unwrap(), s.uri(), "acme", "KEY", None);
        let inv = d.inventory().await.unwrap();
        assert_eq!(inv.domains[0].php.as_deref(), Some("8.1"));
        assert_eq!(inv.domains[0].docroot.as_deref(), Some("/home/acme/domains/acme.com/public_html"));
        assert_eq!(inv.databases, ["acme_wp"]);
        assert_eq!(inv.disk.as_ref().unwrap().limit_mb, Some(2048.0));
        let ctx = ApplyCtx { inv: &inv, email: None, password: Some("pw") };
        let e = d.apply(&Task::Subdomain { sub: "blog".into(), domain: "acme.com".into(), dir: None }, &ctx).await.unwrap_err();
        assert!(e.contains("Subdomain exists"));
        let a = d.apply(&Task::Database { name: "acme_x".into(), user: "acme_x".into() }, &ctx).await.unwrap();
        assert!(a.undo.is_some());
    }
}
