//! cPanel (UAPI with a user API token: `Authorization: cpanel user:TOKEN`) and WHM (API 1 with
//! `Authorization: whm root:TOKEN`; one account's UAPI through `uapi_cpanel`). Calls are POSTs so
//! passwords never sit in a URL or the server's access log.

use std::sync::Arc;

use async_trait::async_trait;
use kor_agent::remote::Shell;
use serde_json::Value;

use super::{
    Applied, ApplyCtx, Cert, Disk, Domain, Inventory, Kind, Panel, Task, php_version, shell,
};
use crate::site::Result;
use crate::site::http::Http;

pub struct Cpanel {
    name: String,
    http: Http,
    url: String,
    auth: String,
    /// WHM mode: `Some(account)` (empty = whole server, inventory only).
    whm: Option<String>,
    sh: Option<Arc<dyn Shell>>,
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn f(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// Unix seconds as a number or a numeric string.
fn ts(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

impl Cpanel {
    pub fn new(
        name: &str,
        http: Http,
        url: String,
        user: &str,
        token: &str,
        whm: Option<String>,
        sh: Option<Arc<dyn Shell>>,
    ) -> Self {
        let scheme = if whm.is_some() { "whm" } else { "cpanel" };
        Self {
            name: name.to_owned(),
            http,
            url,
            auth: format!("{scheme} {user}:{token}"),
            whm,
            sh,
        }
    }

    fn headers(&self) -> [(&'static str, String); 1] {
        [("authorization", self.auth.clone())]
    }

    fn account(&self) -> Result<&str> {
        match self.whm.as_deref() {
            Some("") => Err("WHM: name the cPanel account (--account)".into()),
            Some(a) => Ok(a),
            None => Ok(""),
        }
    }

    /// UAPI call → `data`.
    async fn uapi(&self, module: &str, func: &str, args: &[(&str, &str)]) -> Result<Value> {
        let (url, mut pairs): (String, Vec<(&str, &str)>) = if self.whm.is_none() {
            (format!("{}/execute/{module}/{func}", self.url), Vec::new())
        } else {
            let acc = self.account()?;
            (
                format!("{}/json-api/uapi_cpanel", self.url),
                vec![
                    ("api.version", "1"),
                    ("cpanel.user", acc),
                    ("cpanel.module", module),
                    ("cpanel.function", func),
                ],
            )
        };
        pairs.extend_from_slice(args);
        let r = self.http.post_form(&url, &self.headers(), &pairs).await?;
        if r.status == 401 || r.status == 403 {
            return Err(format!(
                "{}: the panel refused the token (HTTP {})",
                self.name, r.status
            ));
        }
        let v = r.json()?;
        let res = if self.whm.is_some() {
            &v["data"]["uapi"]
        } else {
            &v
        };
        if res["status"].as_i64() == Some(1) || res["status"] == Value::Bool(true) {
            Ok(res["data"].clone())
        } else {
            let errs: Vec<String> = res["errors"]
                .as_array()
                .map(|a| a.iter().map(s).collect())
                .unwrap_or_default();
            Err(format!(
                "{module}::{func}: {}",
                if errs.is_empty() {
                    format!("HTTP {}", r.status)
                } else {
                    errs.join("; ")
                }
            ))
        }
    }

    /// cPanel API 2 (subdomain removal, cron), for functions UAPI does not have.
    async fn api2(&self, module: &str, func: &str, args: &[(&str, &str)]) -> Result<Value> {
        let acc = self.account()?;
        let mut pairs = vec![
            ("cpanel_jsonapi_apiversion", "2"),
            ("cpanel_jsonapi_module", module),
            ("cpanel_jsonapi_func", func),
        ];
        if self.whm.is_some() {
            pairs.push(("cpanel_jsonapi_user", acc));
        }
        pairs.extend_from_slice(args);
        let r = self
            .http
            .post_form(
                &format!("{}/json-api/cpanel", self.url),
                &self.headers(),
                &pairs,
            )
            .await?;
        let v = r.json()?;
        let res = &v["cpanelresult"];
        if res["event"]["result"].as_i64() == Some(1) && res["error"].is_null() {
            Ok(res["data"].clone())
        } else {
            Err(format!("{module}::{func}: {}", s(&res["error"])))
        }
    }

    /// WHM API 1 → `data`.
    async fn whm(&self, func: &str, args: &[(&str, &str)]) -> Result<Value> {
        let mut pairs = vec![("api.version", "1")];
        pairs.extend_from_slice(args);
        let r = self
            .http
            .post_form(
                &format!("{}/json-api/{func}", self.url),
                &self.headers(),
                &pairs,
            )
            .await?;
        let v = r.json()?;
        if v["metadata"]["result"].as_i64() == Some(1) {
            Ok(v["data"].clone())
        } else {
            Err(format!("WHM {func}: {}", s(&v["metadata"]["reason"])))
        }
    }

    async fn whm_inventory(&self) -> Result<Inventory> {
        let mut inv = Inventory {
            host: self.name.clone(),
            kind: Some(Kind::Whm),
            ..Inventory::default()
        };
        let mut used = 0.0;
        for a in self.whm("listaccts", &[]).await?["acct"]
            .as_array()
            .into_iter()
            .flatten()
        {
            inv.accounts.push(s(&a["user"]));
            used += a["diskused"]
                .as_str()
                .and_then(|d| d.trim_end_matches('M').parse::<f64>().ok())
                .unwrap_or(0.0);
        }
        inv.disk = Some(Disk {
            used_mb: used,
            limit_mb: None,
        });
        match self.whm("get_domain_info", &[]).await {
            Ok(d) => {
                for x in d["domains"].as_array().into_iter().flatten() {
                    inv.domains.push(Domain {
                        name: s(&x["domain"]),
                        kind: s(&x["domain_type"]),
                        docroot: x["docroot"].as_str().map(str::to_owned),
                        php: x["php_version"].as_str().and_then(php_version),
                        account: x["user"].as_str().map(str::to_owned),
                    });
                }
            }
            Err(e) => inv.notes.push(e),
        }
        match self.whm("fetch_ssl_vhosts", &[]).await {
            Ok(d) => {
                for v in d["vhosts"].as_array().into_iter().flatten() {
                    inv.certs.push(Cert {
                        domain: s(&v["servername"]),
                        not_after: ts(&v["crt"]["not_after"]),
                        issuer: v["crt"]["issuer.organizationName"]
                            .as_str()
                            .map(str::to_owned),
                        source: "panel".into(),
                        error: None,
                    });
                }
            }
            Err(e) => inv.notes.push(e),
        }
        match self.whm("php_get_installed_versions", &[]).await {
            Ok(d) => {
                inv.php_installed = d["versions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().and_then(php_version))
                    .collect();
            }
            Err(e) => inv.notes.push(e),
        }
        Ok(inv)
    }
}

#[allow(clippy::too_many_lines)] // one provider: each panel API call in sequence
#[async_trait]
impl Panel for Cpanel {
    async fn inventory(&self) -> Result<Inventory> {
        if self.whm.as_deref() == Some("") {
            return self.whm_inventory().await;
        }
        let mut inv = Inventory {
            host: self.name.clone(),
            kind: Some(if self.whm.is_some() {
                Kind::Whm
            } else {
                Kind::Cpanel
            }),
            ..Inventory::default()
        };
        let data = self.uapi("DomainInfo", "domains_data", &[]).await?;
        let mut push = |x: &Value, kind: &str| {
            let name = s(&x["domain"]);
            if !name.is_empty() {
                inv.domains.push(Domain {
                    name,
                    kind: kind.into(),
                    docroot: x["documentroot"].as_str().map(str::to_owned),
                    ..Domain::default()
                });
            }
        };
        push(&data["main_domain"], "main");
        for (k, kind) in [
            ("addon_domains", "addon"),
            ("sub_domains", "sub"),
            ("parked_domains", "parked"),
        ] {
            for x in data[k].as_array().into_iter().flatten() {
                if x.is_string() {
                    push(&serde_json::json!({ "domain": x }), kind);
                } else {
                    push(x, kind);
                }
            }
        }
        match self.uapi("LangPHP", "php_get_vhost_versions", &[]).await {
            Ok(v) => {
                for x in v.as_array().into_iter().flatten() {
                    let vh = s(&x["vhost"]);
                    let ver = x["version"].as_str().and_then(php_version);
                    for d in inv.domains.iter_mut().filter(|d| d.name == vh) {
                        d.php.clone_from(&ver);
                    }
                }
            }
            Err(e) => inv.notes.push(e),
        }
        match self
            .uapi("LangPHP", "php_get_installed_versions", &[])
            .await
        {
            Ok(v) => {
                inv.php_installed = v["versions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().and_then(php_version))
                    .collect();
            }
            Err(e) => inv.notes.push(e),
        }
        match self.uapi("Mysql", "list_databases", &[]).await {
            Ok(v) => {
                inv.databases = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|d| s(&d["database"]))
                    .collect();
            }
            Err(e) => inv.notes.push(e),
        }
        match self.uapi("SSL", "installed_hosts", &[]).await {
            Ok(v) => {
                for h in v.as_array().into_iter().flatten() {
                    let c = &h["certificate"];
                    let not_after = ts(&c["not_after"]);
                    let issuer = c["issuer.organizationName"].as_str().map(str::to_owned);
                    for d in c["domains"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        inv.certs.push(Cert {
                            domain: d.to_owned(),
                            not_after,
                            issuer: issuer.clone(),
                            source: "panel".into(),
                            error: None,
                        });
                    }
                }
            }
            Err(e) => inv.notes.push(e),
        }
        match self.uapi("Quota", "get_quota_info", &[]).await {
            Ok(v) => {
                if let Some(used) = f(&v["megabytes_used"]) {
                    inv.disk = Some(Disk {
                        used_mb: used,
                        limit_mb: f(&v["megabyte_limit"]).filter(|x| *x > 0.0),
                    });
                }
            }
            Err(e) => inv.notes.push(e),
        }
        match self.uapi("Email", "count_pops", &[]).await {
            Ok(v) => {
                inv.email_accounts = v
                    .as_u64()
                    .or_else(|| v.as_str().and_then(|x| x.parse().ok()));
            }
            Err(e) => inv.notes.push(e),
        }
        Ok(inv)
    }

    #[allow(clippy::too_many_lines)]
    async fn apply(&self, task: &Task, ctx: &ApplyCtx<'_>) -> Result<Applied> {
        // Shell tasks, and .user.ini through SSH when there is one.
        if let Some(sh) = &self.sh
            && let Some(r) = shell::generic(sh.as_ref(), task, ctx).await
        {
            return r;
        }
        let done = |summary: String, undo: Option<Task>| {
            Ok(Applied {
                summary,
                undo,
                ..Applied::default()
            })
        };
        match task {
            Task::Ssl { domain } => {
                self.uapi("SSL", "start_autossl_check", &[]).await?;
                done(format!("AutoSSL started (covers {domain})"), None)
            }
            Task::Subdomain { sub, domain, dir } => {
                let dir = dir.clone().unwrap_or_else(|| format!("public_html/{sub}"));
                self.uapi(
                    "SubDomain",
                    "addsubdomain",
                    &[("domain", sub), ("rootdomain", domain), ("dir", &dir)],
                )
                .await?;
                done(
                    format!("{sub}.{domain} → {dir}"),
                    Some(Task::RemoveSubdomain {
                        sub: sub.clone(),
                        domain: domain.clone(),
                    }),
                )
            }
            Task::RemoveSubdomain { sub, domain } => {
                self.api2(
                    "SubDomain",
                    "delsubdomain",
                    &[("domain", &format!("{sub}.{domain}"))],
                )
                .await?;
                done(format!("{sub}.{domain} removed"), None)
            }
            Task::Database { name, user } => {
                let pw = ctx.password.ok_or("no password")?;
                self.uapi("Mysql", "create_database", &[("name", name)])
                    .await?;
                if let Err(e) = self
                    .uapi("Mysql", "create_user", &[("name", user), ("password", pw)])
                    .await
                {
                    let _ = self
                        .uapi("Mysql", "delete_database", &[("name", name)])
                        .await;
                    return Err(e);
                }
                self.uapi(
                    "Mysql",
                    "set_privileges_on_database",
                    &[
                        ("user", user),
                        ("database", name),
                        ("privileges", "ALL PRIVILEGES"),
                    ],
                )
                .await?;
                done(
                    format!("database {name}, user {user}"),
                    Some(Task::RemoveDatabase {
                        name: name.clone(),
                        user: user.clone(),
                    }),
                )
            }
            Task::RemoveDatabase { name, user } => {
                self.uapi("Mysql", "delete_database", &[("name", name)])
                    .await?;
                self.uapi("Mysql", "delete_user", &[("name", user)]).await?;
                done(format!("database {name} removed"), None)
            }
            Task::Email { address, quota_mb } => {
                let pw = ctx.password.ok_or("no password")?;
                let (local, dom) = address.split_once('@').ok_or("invalid address")?;
                let quota = quota_mb.to_string();
                self.uapi(
                    "Email",
                    "add_pop",
                    &[
                        ("email", local),
                        ("domain", dom),
                        ("password", pw),
                        ("quota", &quota),
                    ],
                )
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
                self.uapi("Email", "delete_pop", &[("email", local), ("domain", dom)])
                    .await?;
                done(format!("mailbox {address} removed"), None)
            }
            Task::Backup { .. } => {
                let mut args = Vec::new();
                if let Some(e) = ctx.email {
                    args.push(("email", e));
                }
                self.uapi("Backup", "fullbackup_to_homedir", &args).await?;
                done("full backup started into the home folder".into(), None)
            }
            Task::Cron { schedule, command } => {
                let p: Vec<&str> = schedule.split_whitespace().collect();
                let d = self
                    .api2(
                        "Cron",
                        "add_line",
                        &[
                            ("command", command),
                            ("minute", p[0]),
                            ("hour", p[1]),
                            ("day", p[2]),
                            ("month", p[3]),
                            ("weekday", p[4]),
                        ],
                    )
                    .await?;
                let key = d.get(0).map(|x| s(&x["linekey"])).unwrap_or_default();
                done(
                    format!("cron added: {schedule} {command}"),
                    (!key.is_empty()).then_some(Task::RemoveCron { id: key }),
                )
            }
            Task::RemoveCron { id } => {
                self.api2("Cron", "remove_line", &[("linekey", id)]).await?;
                done(format!("cron {id} removed"), None)
            }
            Task::Php { domain, version } => {
                let prev = ctx.inv.domain(domain).and_then(|d| d.php.clone());
                let ea = format!("ea-php{}", version.replace('.', ""));
                self.uapi(
                    "LangPHP",
                    "php_set_vhost_versions",
                    &[("vhost", domain), ("version", &ea)],
                )
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
            Task::PhpIni { domain, key, value } => {
                let old = self
                    .uapi(
                        "LangPHP",
                        "php_ini_get_user_basic_directives",
                        &[("type", "vhost"), ("vhost", domain)],
                    )
                    .await
                    .ok()
                    .and_then(|v| {
                        v["directives"]
                            .as_array()?
                            .iter()
                            .find(|d| d["key"] == key.as_str())
                            .map(|d| s(&d["value"]))
                    });
                let dir = format!("{key}:{value}");
                self.uapi(
                    "LangPHP",
                    "php_ini_set_user_basic_directives",
                    &[("type", "vhost"), ("vhost", domain), ("directive-1", &dir)],
                )
                .await?;
                done(
                    format!("{domain}: {key} = {value}"),
                    old.map(|v| Task::PhpIni {
                        domain: domain.clone(),
                        key: key.clone(),
                        value: v,
                    }),
                )
            }
            Task::Sftp { user, path, .. } => {
                let pw = ctx.password.ok_or("no password")?;
                self.uapi(
                    "Ftp",
                    "add_ftp",
                    &[
                        ("user", user),
                        ("pass", pw),
                        ("homedir", path),
                        ("quota", "0"),
                    ],
                )
                .await?;
                done(
                    format!("FTP/SFTP account {user} → {path}"),
                    Some(Task::RemoveSftp {
                        user: user.clone(),
                        domain: None,
                    }),
                )
            }
            Task::RemoveSftp { user, .. } => {
                self.uapi("Ftp", "delete_ftp", &[("user", user), ("destroy", "0")])
                    .await?;
                done(format!("FTP account {user} removed"), None)
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
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ok(data: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(json!({ "status": 1, "data": data, "errors": null }))
    }

    async fn uapi(s: &MockServer, m: &str, f: &str, data: Value) {
        Mock::given(method("POST"))
            .and(path(format!("/execute/{m}/{f}")))
            .and(header("authorization", "cpanel acme:TOKEN"))
            .respond_with(ok(&data))
            .mount(s)
            .await;
    }

    #[tokio::test]
    async fn inventory_and_php_change_with_undo() {
        let s = MockServer::start().await;
        uapi(
            &s,
            "DomainInfo",
            "domains_data",
            json!({
                "main_domain": {"domain": "acme.com.tr", "documentroot": "/home/acme/public_html"},
                "addon_domains": [{"domain": "shop.com", "documentroot": "/home/acme/shop"}],
                "sub_domains": [], "parked_domains": ["acme.net"]
            }),
        )
        .await;
        uapi(
            &s,
            "LangPHP",
            "php_get_vhost_versions",
            json!([{"vhost": "acme.com.tr", "version": "ea-php74"}]),
        )
        .await;
        uapi(
            &s,
            "LangPHP",
            "php_get_installed_versions",
            json!({"versions": ["ea-php74", "ea-php83"]}),
        )
        .await;
        uapi(
            &s,
            "Mysql",
            "list_databases",
            json!([{"database": "acme_wp"}]),
        )
        .await;
        uapi(&s, "SSL", "installed_hosts", json!([{"certificate": {"not_after": 1_805_112_000, "domains": ["acme.com.tr"], "issuer.organizationName": "Sectigo"}}])).await;
        uapi(
            &s,
            "Quota",
            "get_quota_info",
            json!({"megabytes_used": "512.5", "megabyte_limit": 10240}),
        )
        .await;
        uapi(&s, "Email", "count_pops", json!(3)).await;
        Mock::given(method("POST"))
            .and(path("/execute/LangPHP/php_set_vhost_versions"))
            .and(body_string_contains("version=ea-php83"))
            .respond_with(ok(&json!(null)))
            .expect(1)
            .mount(&s)
            .await;
        let c = Cpanel::new(
            "acme",
            Http::new().unwrap(),
            s.uri(),
            "acme",
            "TOKEN",
            None,
            None,
        );
        let inv = c.inventory().await.unwrap();
        assert_eq!(inv.domains.len(), 3);
        assert_eq!(
            inv.domain("acme.com.tr").unwrap().php.as_deref(),
            Some("7.4")
        );
        assert_eq!(inv.php_installed, ["7.4", "8.3"]);
        assert_eq!(inv.databases, ["acme_wp"]);
        assert_eq!(inv.certs[0].issuer.as_deref(), Some("Sectigo"));
        assert_eq!(inv.disk.as_ref().unwrap().used_mb, 512.5);
        assert_eq!(inv.email_accounts, Some(3));
        let ctx = ApplyCtx {
            inv: &inv,
            email: None,
            password: None,
        };
        let a = c
            .apply(
                &Task::Php {
                    domain: "acme.com.tr".into(),
                    version: "8.3".into(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(
            a.undo,
            Some(Task::Php {
                domain: "acme.com.tr".into(),
                version: "7.4".into()
            })
        );
    }

    #[tokio::test]
    async fn database_failure_cleans_up_and_errors_are_reported() {
        let s = MockServer::start().await;
        uapi(&s, "Mysql", "create_database", json!(null)).await;
        Mock::given(method("POST"))
            .and(path("/execute/Mysql/create_user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status": 0, "errors": ["Password too weak"]})),
            )
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/execute/Mysql/delete_database"))
            .respond_with(ok(&json!(null)))
            .expect(1)
            .mount(&s)
            .await;
        let c = Cpanel::new(
            "acme",
            Http::new().unwrap(),
            s.uri(),
            "acme",
            "TOKEN",
            None,
            None,
        );
        let inv = Inventory::default();
        let ctx = ApplyCtx {
            inv: &inv,
            email: None,
            password: Some("pw"),
        };
        let e = c
            .apply(
                &Task::Database {
                    name: "acme_x".into(),
                    user: "acme_x".into(),
                },
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(e.contains("Password too weak"), "{e}");
        assert!(
            c.apply(&Task::Perms { path: "x".into() }, &ctx)
                .await
                .unwrap_err()
                .contains("SSH")
        );
    }

    #[tokio::test]
    async fn whm_server_inventory() {
        let s = MockServer::start().await;
        let meta = |data: Value| {
            ResponseTemplate::new(200)
                .set_body_json(json!({"metadata": {"result": 1}, "data": data}))
        };
        Mock::given(path("/json-api/listaccts"))
            .and(header("authorization", "whm root:T"))
            .respond_with(meta(
                json!({"acct": [{"user": "acme", "domain": "acme.com", "diskused": "100M"}]}),
            ))
            .mount(&s)
            .await;
        Mock::given(path("/json-api/get_domain_info"))
            .respond_with(meta(json!({"domains": [{"domain": "acme.com", "domain_type": "main", "docroot": "/home/acme/public_html", "php_version": "ea-php81", "user": "acme"}]}))).mount(&s).await;
        Mock::given(path("/json-api/fetch_ssl_vhosts"))
            .respond_with(meta(json!({"vhosts": []})))
            .mount(&s)
            .await;
        Mock::given(path("/json-api/php_get_installed_versions"))
            .respond_with(meta(json!({"versions": ["ea-php81"]})))
            .mount(&s)
            .await;
        let c = Cpanel::new(
            "srv",
            Http::new().unwrap(),
            s.uri(),
            "root",
            "T",
            Some(String::new()),
            None,
        );
        let inv = c.inventory().await.unwrap();
        assert_eq!(inv.accounts, ["acme"]);
        assert_eq!(inv.domains[0].php.as_deref(), Some("8.1"));
        assert_eq!(inv.disk.unwrap().used_mb, 100.0);
    }
}
