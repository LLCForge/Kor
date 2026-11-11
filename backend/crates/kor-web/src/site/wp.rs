//! `kor wp` (§29.3): WordPress/WooCommerce sites through the REST API (application password from the
//! vault) and/or WP-CLI over SSH.
//!
//! - inventory: WordPress version, plugins, themes, user count, WooCommerce product/order counts;
//! - health: Site Health REST tests, page checks, `wp core verify-checksums`;
//! - updates: backup first, optional staging copy (`<root>/kor-staging`, same database with its own
//!   table prefix), update one item at a time, smoke test (home, product, cart, checkout), roll the
//!   item back when the smoke test fails, report; live updates only after approval;
//! - security: core files compared with WordPress.org checksums, plugin checksums, suspicious code
//!   patterns, and an approved clean-up (reinstall core/plugins, new salts, password resets);
//! - scaffolding: child theme and plugin skeletons with nonces, sanitizing, escaping and i18n.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use kor_agent::remote::{Shell, sh_quote};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::host::shell::{exec, q};
use super::http::Http;
use super::{Result, check_name};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WpSite {
    /// `https://example.com` (the site's home URL).
    pub url: String,
    /// WordPress user for the REST API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Vault entry with the application password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// SSH connection id or `user@host` for WP-CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
    /// WordPress root on the server (where wp-config.php is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Host profile the site runs on (PHP compatibility checks in `kor host php`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

impl WpSite {
    pub fn validate(&mut self) -> Result<()> {
        let mut u = url::Url::parse(&self.url).map_err(|e| format!("invalid URL {}: {e}", self.url))?;
        let local = u.host_str().is_some_and(kor_core::net::is_loopback_host);
        if u.scheme() != "https" && !local && self.secret.is_some() {
            return Err("the site URL must use https (the application password is sent with every request)".into());
        }
        u.set_query(None);
        u.as_str().trim_end_matches('/').clone_into(&mut self.url);
        if self.secret.is_some() != self.user.is_some() {
            return Err("--user and --secret go together".into());
        }
        if self.secret.is_none() && self.ssh.is_none() {
            return Err("give --user/--secret (REST) or --ssh/--path (WP-CLI)".into());
        }
        if self.ssh.is_some() && self.path.is_none() {
            return Err("--ssh needs --path (the WordPress folder on the server)".into());
        }
        if let Some(p) = &self.path {
            super::host::shell::check_path(p)?;
        }
        if let Some(s) = &self.secret
            && !kor_secrets::vault::valid_name(s)
        {
            return Err(format!("invalid vault name `{s}`"));
        }
        for n in [&self.host, &self.client].into_iter().flatten() {
            check_name(n)?;
        }
        Ok(())
    }

    pub fn domain(&self) -> String {
        super::http::host_of(&self.url)
    }
}

pub fn sites_dir(base: &Path) -> PathBuf {
    base.join("sites")
}

pub fn save(base: &Path, name: &str, s: &WpSite) -> Result<PathBuf> {
    check_name(name)?;
    let p = sites_dir(base).join(format!("{name}.toml"));
    super::save_toml(&p, s)?;
    Ok(p)
}

pub fn load(base: &Path, name: &str) -> Result<WpSite> {
    check_name(name)?;
    let p = sites_dir(base).join(format!("{name}.toml"));
    if !p.is_file() {
        return Err(format!("no WordPress site `{name}` (kor wp connect {name} …)"));
    }
    super::load_toml(&p)
}

pub fn list(base: &Path) -> Vec<(String, Result<WpSite>)> {
    super::names_in(&sites_dir(base))
        .into_iter()
        .map(|n| {
            let s = load(base, &n);
            (n, s)
        })
        .collect()
}

pub fn remove(base: &Path, name: &str) -> Result<bool> {
    check_name(name)?;
    match std::fs::remove_file(sites_dir(base).join(format!("{name}.toml"))) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plugin {
    /// wordpress.org slug (the plugin folder).
    pub slug: String,
    pub name: String,
    pub version: String,
    pub active: bool,
    /// Newer version WordPress knows of (WP-CLI only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WpInventory {
    pub site: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_update: Option<String>,
    pub plugins: Vec<Plugin>,
    pub themes: Vec<Plugin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<u64>,
    pub woocommerce: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub products: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orders: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php: Option<String>,
    /// What could not be read and why.
    pub notes: Vec<String>,
}

/// A connected site: REST credentials and/or a WP-CLI shell.
pub struct Wp {
    pub name: String,
    pub site: WpSite,
    http: Http,
    auth: Option<String>,
    sh: Option<Arc<dyn Shell>>,
}

impl std::fmt::Debug for Wp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wp").field("name", &self.name).field("url", &self.site.url).finish_non_exhaustive()
    }
}

const T: Duration = Duration::from_secs(600);

/// `akismet/akismet.php` → `akismet`; `hello.php` → `hello`.
pub fn slug_of(plugin: &str) -> String {
    plugin
        .split('/')
        .next()
        .unwrap_or(plugin)
        .trim_end_matches(".php")
        .to_owned()
}

/// `<meta name="generator" content="WordPress 6.6.2">` or the feed's `?v=6.6.2`.
pub fn generator_version(html: &str) -> Option<String> {
    let re = regex::Regex::new(r#"(?i)(?:content="WordPress\s+|wordpress\.org/\?v=)([0-9][0-9.]*)"#).ok()?;
    re.captures(html).map(|c| c[1].to_owned())
}

impl Wp {
    pub fn new(name: &str, site: WpSite, password: Option<String>, sh: Option<Arc<dyn Shell>>) -> Result<Self> {
        let auth = match (&site.user, password) {
            (Some(u), Some(p)) => Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"))
            )),
            _ => None,
        };
        Ok(Self { name: name.to_owned(), site, http: Http::new()?, auth, sh })
    }

    pub fn shell(&self) -> Option<&Arc<dyn Shell>> {
        self.sh.as_ref()
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    fn root(&self) -> Result<&str> {
        self.site.path.as_deref().ok_or_else(|| "this site has no WP-CLI path (--ssh/--path)".into())
    }

    /// `wp --path=<root> …` (or another root, for staging).
    pub async fn cli_at(&self, root: &str, args: &str) -> Result<String> {
        let sh = self.sh.as_ref().ok_or("WP-CLI needs SSH (kor wp connect … --ssh … --path …)")?;
        let out = sh.run(&format!("wp --path={} {args}", q(root)), None, T).await?;
        if out.ok() {
            Ok(out.text())
        } else {
            let e = out.error();
            Err(format!("wp {}: {}", args.split_whitespace().next().unwrap_or(""), if e.trim().is_empty() { out.text() } else { e }.trim()))
        }
    }

    pub async fn cli(&self, args: &str) -> Result<String> {
        let root = self.root()?.to_owned();
        self.cli_at(&root, args).await
    }

    /// GET `/wp-json/<path>` → (JSON, X-WP-Total).
    pub async fn rest(&self, path: &str) -> Result<(Value, Option<u64>)> {
        let mut h = Vec::new();
        if let Some(a) = &self.auth {
            h.push(("authorization", a.clone()));
        }
        let url = format!("{}/wp-json/{}", self.site.url, path.trim_start_matches('/'));
        let r = self.http.get_with(&url, &h).await?;
        if r.status == 401 || r.status == 403 {
            return Err(format!("{path}: WordPress refused the application password (HTTP {})", r.status));
        }
        if !r.ok() {
            return Err(format!("{path}: HTTP {}", r.status));
        }
        let total = r.header("x-wp-total").and_then(|t| t.parse().ok());
        Ok((r.json()?, total))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn inventory(&self) -> Result<WpInventory> {
        let mut inv = WpInventory { site: self.name.clone(), url: self.site.url.clone(), ..WpInventory::default() };
        let mut got_any = false;
        if self.sh.is_some() && self.site.path.is_some() {
            match self.cli("core version").await {
                Ok(v) => {
                    inv.version = Some(v.trim().to_owned());
                    got_any = true;
                }
                Err(e) => inv.notes.push(e),
            }
            if let Ok(o) = self.cli("core check-update --format=json").await {
                inv.core_update = serde_json::from_str::<Value>(o.trim())
                    .ok()
                    .and_then(|v| v.get(0).and_then(|x| x["version"].as_str().map(str::to_owned)));
            }
            for (what, out) in [("plugin", &mut inv.plugins), ("theme", &mut inv.themes)] {
                match self.cli(&format!("{what} list --format=json --fields=name,title,status,version,update_version")).await {
                    Ok(o) => {
                        let v: Value = serde_json::from_str(o.trim()).unwrap_or(Value::Null);
                        *out = v
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|p| p["status"] != "must-use" && p["status"] != "dropin")
                            .map(|p| Plugin {
                                slug: p["name"].as_str().unwrap_or_default().to_owned(),
                                name: p["title"].as_str().or(p["name"].as_str()).unwrap_or_default().to_owned(),
                                version: p["version"].as_str().unwrap_or_default().to_owned(),
                                active: matches!(p["status"].as_str(), Some("active" | "active-network" | "parent")),
                                update: p["update_version"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
                            })
                            .collect();
                    }
                    Err(e) => inv.notes.push(e),
                }
            }
            if let Ok(o) = self.cli("user list --format=count").await {
                inv.users = o.trim().parse().ok();
            }
            if let Ok(o) = self.cli("eval 'echo PHP_VERSION;'").await {
                inv.php = super::host::php_version(o.trim());
            }
        }
        if self.auth.is_some() {
            if inv.plugins.is_empty() {
                match self.rest("wp/v2/plugins?context=edit").await {
                    Ok((v, _)) => {
                        got_any = true;
                        inv.plugins = v
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|p| Plugin {
                                slug: slug_of(p["plugin"].as_str().unwrap_or_default()),
                                name: p["name"].as_str().unwrap_or_default().to_owned(),
                                version: p["version"].as_str().unwrap_or_default().to_owned(),
                                active: p["status"] == "active" || p["status"] == "network-active",
                                update: None,
                            })
                            .collect();
                    }
                    Err(e) => inv.notes.push(e),
                }
            }
            if inv.themes.is_empty() {
                match self.rest("wp/v2/themes?context=edit").await {
                    Ok((v, _)) => {
                        inv.themes = v
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|t| Plugin {
                                slug: t["stylesheet"].as_str().unwrap_or_default().to_owned(),
                                name: t["name"]["raw"].as_str().or(t["name"]["rendered"].as_str()).unwrap_or_default().to_owned(),
                                version: t["version"].as_str().unwrap_or_default().to_owned(),
                                active: t["status"] == "active",
                                update: None,
                            })
                            .collect();
                    }
                    Err(e) => inv.notes.push(e),
                }
            }
            if inv.users.is_none() {
                match self.rest("wp/v2/users?per_page=1&context=edit").await {
                    Ok((_, t)) => inv.users = t,
                    Err(e) => inv.notes.push(e),
                }
            }
        }
        // WooCommerce: from the plugin list, then counts (REST, else WP-CLI).
        inv.woocommerce = inv.plugins.iter().any(|p| p.slug == "woocommerce" && p.active);
        if inv.woocommerce {
            if self.auth.is_some() {
                if let Ok((_, t)) = self.rest("wc/v3/products?per_page=1").await {
                    inv.products = t;
                }
                if let Ok((_, t)) = self.rest("wc/v3/orders?per_page=1").await {
                    inv.orders = t;
                }
            }
            if inv.products.is_none() && self.sh.is_some() {
                inv.products = self.cli("post list --post_type=product --post_status=any --format=count").await.ok().and_then(|o| o.trim().parse().ok());
            }
            if inv.orders.is_none() && self.sh.is_some() {
                // HPOS stores orders in wc_orders; older shops as shop_order posts.
                let prefix = self.cli("db prefix").await.unwrap_or_default();
                let prefix = prefix.trim();
                if prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !prefix.is_empty() {
                    inv.orders = self
                        .cli(&format!("db query \"SELECT COUNT(*) FROM {prefix}wc_orders\" --skip-column-names"))
                        .await
                        .ok()
                        .and_then(|o| o.trim().parse().ok());
                }
                if inv.orders.is_none() {
                    inv.orders = self.cli("post list --post_type=shop_order --post_status=any --format=count").await.ok().and_then(|o| o.trim().parse().ok());
                }
            }
        }
        if inv.version.is_none()
            && let Ok(r) = self.http.get(&format!("{}/feed/", self.site.url)).await
        {
            inv.version = generator_version(&r.body);
            got_any |= r.ok();
        }
        if inv.version.is_none()
            && let Ok((_, r)) = self.http.follow(&format!("{}/", self.site.url)).await
        {
            inv.version = generator_version(&r.body);
            got_any |= r.ok();
        }
        if !got_any {
            return Err(inv.notes.join("; "));
        }
        Ok(inv)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    /// `good`, `recommended`, `critical`.
    pub status: String,
    pub label: String,
    /// Where the result came from.
    pub source: String,
}

/// Site Health REST tests, page checks and core checksums.
pub async fn health(wp: &Wp) -> Vec<Check> {
    let mut out = Vec::new();
    let page = |id: &str, ok: bool, label: String| Check {
        id: id.into(),
        status: if ok { "good" } else { "critical" }.into(),
        label,
        source: "http".into(),
    };
    for (id, path) in [("home", "/"), ("login", "/wp-login.php"), ("rest", "/wp-json/")] {
        match wp.http.follow(&format!("{}{path}", wp.site.url)).await {
            Ok((hops, r)) => {
                let fatal = super::host::fatal_text(&r.body);
                out.push(page(
                    id,
                    r.status < 400 && !fatal,
                    format!("{path}: HTTP {} · {} ms{}{}", r.status, r.ms, if hops.is_empty() { String::new() } else { format!(" · {} redirect(s)", hops.len()) }, if fatal { " · fatal error" } else { "" }),
                ));
            }
            Err(e) => out.push(page(id, false, format!("{path}: {e}"))),
        }
    }
    if wp.auth.is_some() {
        for t in ["background-updates", "loopback-requests", "https-status", "dotorg-communication", "authorization-header", "page-cache"] {
            match wp.rest(&format!("wp-site-health/v1/tests/{t}")).await {
                Ok((v, _)) => out.push(Check {
                    id: t.into(),
                    status: v["status"].as_str().unwrap_or("recommended").into(),
                    label: v["label"].as_str().unwrap_or(t).into(),
                    source: "site-health".into(),
                }),
                Err(e) => out.push(Check { id: t.into(), status: "recommended".into(), label: e, source: "site-health".into() }),
            }
        }
    }
    if wp.sh.is_some() && wp.site.path.is_some() {
        let r = wp.cli("core verify-checksums").await;
        out.push(Check {
            id: "core-checksums".into(),
            status: if r.is_ok() { "good" } else { "critical" }.into(),
            label: r.unwrap_or_else(|e| e).lines().take(5).collect::<Vec<_>>().join(" "),
            source: "wp-cli".into(),
        });
    }
    out
}

/// What one smoke test page answered.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Smoke {
    pub page: String,
    pub url: String,
    pub status: u16,
    pub ok: bool,
    pub note: String,
}

/// Home, a product page, cart and checkout (WooCommerce), all must answer below 400 without a
/// fatal error page. `base` is the live or staging URL.
pub async fn smoke(wp: &Wp, base: &str, woocommerce: bool) -> Vec<Smoke> {
    let mut pages = vec![("home".to_owned(), format!("{base}/"))];
    if woocommerce {
        let product = if wp.auth.is_some() {
            wp.rest("wc/v3/products?per_page=1&status=publish").await.ok().and_then(|(v, _)| v.get(0)?["permalink"].as_str().map(str::to_owned))
        } else {
            None
        };
        let product = match product {
            Some(p) => Some(p.replacen(&wp.site.url, base, 1)),
            None if wp.sh.is_some() => wp
                .cli("post list --post_type=product --post_status=publish --posts_per_page=1 --field=url")
                .await
                .ok()
                .map(|u| u.trim().replacen(&wp.site.url, base, 1))
                .filter(|u| !u.is_empty()),
            None => None,
        };
        if let Some(p) = product {
            pages.push(("product".into(), p));
        }
        pages.push(("cart".into(), format!("{base}/?wc-ajax=get_refreshed_fragments")));
        pages.push(("cart-page".into(), format!("{base}/cart/")));
        pages.push(("checkout".into(), format!("{base}/checkout/")));
    }
    let mut out = Vec::new();
    for (page, url) in pages {
        let s = match wp.http.follow(&url).await {
            Ok((_, r)) => {
                let fatal = super::host::fatal_text(&r.body);
                Smoke {
                    page,
                    url,
                    status: r.status,
                    ok: r.status < 400 && !fatal,
                    note: if fatal { "fatal error on page".into() } else { format!("{} ms", r.ms) },
                }
            }
            Err(e) => Smoke { page, url, ok: false, note: e, ..Smoke::default() },
        };
        out.push(s);
    }
    out
}

/// Staging copy in `<root>/kor-staging` with table prefix `kst_` in the same database.
pub async fn create_staging(wp: &Wp) -> Result<String> {
    let root = wp.root()?.trim_end_matches('/').to_owned();
    let st = format!("{root}/kor-staging");
    let url = format!("{}/kor-staging", wp.site.url);
    let prefix = wp.cli("db prefix").await?.trim().to_owned();
    if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("unexpected table prefix `{prefix}`"));
    }
    let sh = wp.sh.as_ref().ok_or("staging needs SSH")?;
    let script = format!(
        "set -e; R={r}; S={s}; P={p}; \
         rm -rf \"$S\"; mkdir -p \"$S\"; \
         tar -C \"$R\" --exclude=./kor-staging --exclude=./wp-content/cache --exclude=./wp-content/updraft -cf - . | tar -C \"$S\" -xf -; \
         T=$(wp --path=\"$R\" db tables --all-tables-with-prefix --format=csv); \
         wp --path=\"$R\" db export - --tables=\"$T\" | sed \"s/\\`$P/\\`kst_/g\" | wp --path=\"$R\" db import -; \
         sed -i \"s/\\$table_prefix *= *['\\\"]$P['\\\"]/\\$table_prefix = 'kst_'/\" \"$S/wp-config.php\"; \
         wp --path=\"$S\" db query \"UPDATE kst_options SET option_name='kst_user_roles' WHERE option_name='${{P}}user_roles'; UPDATE kst_usermeta SET meta_key=CONCAT('kst_', SUBSTRING(meta_key, LENGTH('$P')+1)) WHERE meta_key LIKE '${{P}}%';\"; \
         wp --path=\"$S\" search-replace {u} {su} --all-tables-with-prefix --skip-columns=guid --quiet; \
         wp --path=\"$S\" option update blog_public 0 --quiet; echo ok",
        r = sh_quote(&root),
        s = sh_quote(&st),
        p = sh_quote(&prefix),
        u = sh_quote(&wp.site.url),
        su = sh_quote(&url),
    );
    exec(sh.as_ref(), &script, None).await?;
    Ok(url)
}

/// Remove the staging copy and its tables.
pub async fn drop_staging(wp: &Wp) -> Result<()> {
    let root = wp.root()?.trim_end_matches('/').to_owned();
    let sh = wp.sh.as_ref().ok_or("staging needs SSH")?;
    let script = format!(
        "S={s}; T=$(wp --path=\"$S\" db tables 'kst_*' --all-tables --format=csv 2>/dev/null); \
         [ -n \"$T\" ] && wp --path={r} db query \"DROP TABLE $T\"; rm -rf \"$S\"",
        s = sh_quote(&format!("{root}/kor-staging")),
        r = sh_quote(&root),
    );
    exec(sh.as_ref(), &script, None).await.map(|_| ())
}

/// One item of an update run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Updated {
    /// `core`, `plugin`, `theme`.
    pub kind: String,
    pub slug: String,
    pub from: String,
    pub to: String,
    /// `updated`, `rolled-back`, `failed`, `skipped`.
    pub result: String,
    pub smoke: Vec<Smoke>,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateReport {
    pub target: String,
    pub backup: Vec<String>,
    pub items: Vec<Updated>,
}

/// Files + database into `~/kor-backups/` before anything changes.
pub async fn backup(wp: &Wp, root: &str, label: &str) -> Result<Vec<String>> {
    let sh = wp.sh.as_ref().ok_or("updates need SSH (WP-CLI)")?;
    super::host::shell::backup(sh.as_ref(), label, Some(root)).await.map(|a| a.output)
}

/// What can be updated, from WP-CLI.
pub fn pending(inv: &WpInventory) -> Vec<(String, String, String, String)> {
    let mut v = Vec::new();
    if let (Some(cur), Some(new)) = (&inv.version, &inv.core_update) {
        v.push(("core".into(), "wordpress".into(), cur.clone(), new.clone()));
    }
    for (kind, list) in [("plugin", &inv.plugins), ("theme", &inv.themes)] {
        for p in list {
            if let Some(u) = &p.update {
                v.push((kind.into(), p.slug.clone(), p.version.clone(), u.clone()));
            }
        }
    }
    v
}

/// Update items one by one on `root` (live or staging) and smoke-test `base` after each; a failing
/// plugin or theme is put back at its old version (`--version=<old> --force`). `only` limits the run.
pub async fn update(wp: &Wp, root: &str, base: &str, inv: &WpInventory, only: &[String]) -> Result<UpdateReport> {
    let label = format!("{}{}", wp.site.domain(), if root.ends_with("kor-staging") { "-staging" } else { "" });
    let mut rep = UpdateReport { target: base.to_owned(), backup: backup(wp, root, &label).await?, items: Vec::new() };
    for (kind, slug, from, to) in pending(inv) {
        if !only.is_empty() && !only.contains(&slug) {
            continue;
        }
        if !slug.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
            continue;
        }
        let cmd = match kind.as_str() {
            "core" => format!("core update --version={to}"),
            k => format!("{k} update {slug}"),
        };
        let mut item = Updated { kind: kind.clone(), slug: slug.clone(), from: from.clone(), to, ..Updated::default() };
        if let Err(e) = wp.cli_at(root, &cmd).await {
            item.result = "failed".into();
            item.note = e;
        } else {
            {
                if kind == "core" {
                    let _ = wp.cli_at(root, "core update-db").await;
                }
                item.smoke = smoke(wp, base, inv.woocommerce).await;
                if item.smoke.iter().all(|s| s.ok) {
                    item.result = "updated".into();
                } else if kind == "core" {
                    // Core is not downgraded automatically: the backup is the way back.
                    item.result = "failed".into();
                    item.note = "smoke test failed after the core update; restore the backup".into();
                } else {
                    let back = wp.cli_at(root, &format!("{kind} install {slug} --version={from} --force")).await;
                    item.result = if back.is_ok() { "rolled-back" } else { "failed" }.into();
                    item.note = back.err().unwrap_or_default();
                }
            }
        }
        rep.items.push(item);
    }
    Ok(rep)
}

/// Core files that differ from WordPress.org, extra PHP files in core folders and PHP in uploads.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Integrity {
    pub version: String,
    pub modified: Vec<String>,
    pub unknown: Vec<String>,
    pub uploads_php: Vec<String>,
    /// `plugin: file` lines from `wp plugin verify-checksums`.
    pub plugins: Vec<String>,
    /// File, line, pattern for suspicious code.
    pub suspicious: Vec<(String, u32, String)>,
    pub source: String,
}

/// Patterns common in injected PHP (deterministic; the model only explains them).
const SUSPICIOUS: &[(&str, &str)] = &[
    (r"eval\s*\(\s*(base64_decode|gzinflate|gzuncompress|str_rot13|strrev)", "eval of decoded data"),
    (r"(base64_decode|gzinflate)\s*\(\s*['\x22][A-Za-z0-9+/=]{200,}", "long encoded payload"),
    (r"\$_(POST|GET|REQUEST|COOKIE)\s*\[[^\]]+\]\s*\(", "request value called as a function"),
    (r"assert\s*\(\s*\$_(POST|GET|REQUEST)", "assert on request data"),
    (r"preg_replace\s*\(\s*['\x22].*/e['\x22]", "preg_replace /e"),
    (r"(shell_exec|passthru|system|proc_open|popen)\s*\(\s*\$_(POST|GET|REQUEST)", "shell call with request data"),
    (r"create_function\s*\(", "create_function"),
    (r"\\x[0-9a-fA-F]{2}(\\x[0-9a-fA-F]{2}){30,}", "hex-escaped string"),
];

pub fn scan_code(file: &str, text: &str) -> Vec<(String, u32, String)> {
    let res: Vec<(regex::Regex, &str)> = SUSPICIOUS.iter().filter_map(|(p, why)| regex::Regex::new(p).ok().map(|r| (r, *why))).collect();
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (re, why) in &res {
            if re.is_match(line) {
                out.push((file.to_owned(), u32::try_from(i + 1).unwrap_or(u32::MAX), (*why).to_owned()));
            }
        }
    }
    out
}

/// Compare core files with `api.wordpress.org/core/checksums/1.0`, plugin checksums, and scan the
/// changed/unknown files and PHP in uploads.
pub async fn integrity(wp: &Wp) -> Result<Integrity> {
    let sh = wp.sh.as_ref().ok_or("the integrity check needs SSH (WP-CLI)")?;
    let root = wp.root()?.trim_end_matches('/').to_owned();
    let version = wp.cli("core version").await?.trim().to_owned();
    let locale = wp.cli("eval 'echo get_locale();'").await.unwrap_or_else(|_| "en_US".into()).trim().to_owned();
    let url = format!(
        "https://api.wordpress.org/core/checksums/1.0/?{}",
        super::http::form(&[("version", &version), ("locale", &locale)])
    );
    let r = wp.http.get(&url).await?;
    let sums = r.json()?;
    let sums = sums["checksums"].get(&version).unwrap_or(&sums["checksums"]).clone();
    let sums = sums.as_object().ok_or("WordPress.org has no checksums for this version")?;
    let listing = exec(
        sh.as_ref(),
        &format!(
            "cd {} && find wp-admin wp-includes -type f -exec md5sum {{}} + ; for f in *.php; do md5sum \"$f\"; done",
            q(&root)
        ),
        None,
    )
    .await?;
    let mut out = Integrity { version, source: "api.wordpress.org/core/checksums".into(), ..Integrity::default() };
    for l in listing.lines() {
        let Some((sum, file)) = l.split_once("  ") else { continue };
        match sums.get(file).and_then(Value::as_str) {
            Some(s) if s == sum => {}
            Some(_) => out.modified.push(file.to_owned()),
            None if std::path::Path::new(file).extension().is_some_and(|e| e.eq_ignore_ascii_case("php")) && file != "wp-config.php" => out.unknown.push(file.to_owned()),
            None => {}
        }
    }
    out.uploads_php = exec(sh.as_ref(), &format!("cd {} && find wp-content/uploads -type f -name '*.php' 2>/dev/null | head -200", q(&root)), None)
        .await
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    if let Ok(o) = sh.run(&format!("wp --path={} plugin verify-checksums --all --format=json 2>/dev/null", q(&root)), None, T).await {
        let v: Value = serde_json::from_str(o.text().trim()).unwrap_or(Value::Null);
        out.plugins = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| format!("{}: {} ({})", x["plugin_name"].as_str().unwrap_or("?"), x["file"].as_str().unwrap_or("?"), x["message"].as_str().unwrap_or("")))
            .collect();
    }
    let mut look: Vec<String> = out.modified.iter().chain(&out.unknown).chain(&out.uploads_php).take(80).cloned().collect();
    look.dedup();
    for f in look {
        if let Ok(text) = exec(sh.as_ref(), &format!("cd {} && head -c 200000 {}", q(&root), sh_quote(&f)), None).await {
            out.suspicious.extend(scan_code(&f, &text));
        }
    }
    Ok(out)
}

/// The approved clean-up after a compromise: core reinstalled from WordPress.org (content kept),
/// plugins whose checksums failed reinstalled, new salts (everyone is logged out), PHP removed from
/// uploads, administrators get password reset emails.
pub async fn cleanup(wp: &Wp, found: &Integrity) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let root = wp.root()?.trim_end_matches('/').to_owned();
    backup(wp, &root, &format!("{}-before-cleanup", wp.site.domain())).await?;
    wp.cli(&format!("core download --force --skip-content --version={}", found.version)).await?;
    done.push(format!("core {} reinstalled", found.version));
    let mut plugins: Vec<String> = found.plugins.iter().filter_map(|l| l.split(':').next().map(str::to_owned)).collect();
    plugins.sort();
    plugins.dedup();
    for p in plugins.iter().filter(|p| p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))) {
        let v = wp.cli(&format!("plugin get {p} --field=version")).await.unwrap_or_default();
        let ver = v.trim();
        let arg = if ver.is_empty() { String::new() } else { format!(" --version={ver}") };
        wp.cli(&format!("plugin install {p} --force{arg}")).await?;
        done.push(format!("plugin {p} reinstalled"));
    }
    if let Some(sh) = &wp.sh {
        for f in &found.uploads_php {
            exec(sh.as_ref(), &format!("cd {} && rm -f {}", q(&root), sh_quote(f)), None).await?;
            done.push(format!("removed {f}"));
        }
    }
    wp.cli("config shuffle-salts").await?;
    done.push("salts renewed (all sessions logged out)".into());
    let admins = wp.cli("user list --role=administrator --field=user_login").await?;
    for a in admins.lines().map(str::trim).filter(|a| !a.is_empty()) {
        wp.cli(&format!("user reset-password {}", sh_quote(a))).await?;
        done.push(format!("password reset email sent to {a}"));
    }
    Ok(done)
}

/// Child theme files: style.css with the Template header, functions.php enqueuing the parent style.
pub fn child_theme(parent: &str, name: &str, text_domain: &str) -> Vec<(String, String)> {
    let dir = text_domain.to_owned();
    vec![
        (
            format!("{dir}/style.css"),
            format!(
                "/*\nTheme Name: {name}\nTemplate: {parent}\nVersion: 1.0.0\nText Domain: {text_domain}\nLicense: GPL-2.0-or-later\n*/\n"
            ),
        ),
        (
            format!("{dir}/functions.php"),
            format!(
                "<?php\n/**\n * {name} functions.\n *\n * @package {text_domain}\n */\n\ndefined( 'ABSPATH' ) || exit;\n\n\
add_action( 'wp_enqueue_scripts', function () {{\n\twp_enqueue_style( '{parent}-style', get_template_directory_uri() . '/style.css', array(), wp_get_theme( '{parent}' )->get( 'Version' ) );\n\
\twp_enqueue_style( '{text_domain}-style', get_stylesheet_uri(), array( '{parent}-style' ), wp_get_theme()->get( 'Version' ) );\n}} );\n\n\
add_action( 'after_setup_theme', function () {{\n\tload_child_theme_textdomain( '{text_domain}', get_stylesheet_directory() . '/languages' );\n}} );\n"
            ),
        ),
        (format!("{dir}/languages/.gitkeep"), String::new()),
    ]
}

/// Plugin skeleton following the WordPress coding standards: settings page with a nonce,
/// capability check, sanitizing on save, escaping on output, text domain, uninstall cleanup; with
/// `woocommerce`, a WooCommerce dependency check and an order-status hook.
pub fn plugin_skeleton(name: &str, slug: &str, woocommerce: bool) -> Vec<(String, String)> {
    let func = slug.replace('-', "_");
    let mut main = format!(
        "<?php\n/**\n * Plugin Name: {name}\n * Description: {name}.\n * Version: 1.0.0\n * Requires at least: 6.0\n * Requires PHP: 7.4\n\
 * Text Domain: {slug}\n * Domain Path: /languages\n * License: GPL-2.0-or-later\n{wc}*/\n\ndefined( 'ABSPATH' ) || exit;\n\n\
const {up}_OPTION = '{func}_settings';\n\n\
add_action( 'init', function () {{\n\tload_plugin_textdomain( '{slug}', false, dirname( plugin_basename( __FILE__ ) ) . '/languages' );\n}} );\n\n\
add_action( 'admin_menu', function () {{\n\tadd_options_page( __( '{name}', '{slug}' ), __( '{name}', '{slug}' ), 'manage_options', '{slug}', '{func}_settings_page' );\n}} );\n\n\
function {func}_settings_page() {{\n\tif ( ! current_user_can( 'manage_options' ) ) {{\n\t\treturn;\n\t}}\n\
\tif ( isset( $_POST['{func}_nonce'] ) && wp_verify_nonce( sanitize_text_field( wp_unslash( $_POST['{func}_nonce'] ) ), '{func}_save' ) ) {{\n\
\t\t$value = isset( $_POST['{func}_text'] ) ? sanitize_text_field( wp_unslash( $_POST['{func}_text'] ) ) : '';\n\
\t\tupdate_option( {up}_OPTION, array( 'text' => $value ) );\n\t\techo '<div class=\"updated\"><p>' . esc_html__( 'Saved.', '{slug}' ) . '</p></div>';\n\t}}\n\
\t$settings = get_option( {up}_OPTION, array( 'text' => '' ) );\n\t?>\n\t<div class=\"wrap\">\n\t\t<h1><?php echo esc_html( get_admin_page_title() ); ?></h1>\n\
\t\t<form method=\"post\">\n\t\t\t<?php wp_nonce_field( '{func}_save', '{func}_nonce' ); ?>\n\
\t\t\t<label for=\"{func}_text\"><?php esc_html_e( 'Text', '{slug}' ); ?></label>\n\
\t\t\t<input type=\"text\" id=\"{func}_text\" name=\"{func}_text\" value=\"<?php echo esc_attr( $settings['text'] ); ?>\" class=\"regular-text\" />\n\
\t\t\t<?php submit_button(); ?>\n\t\t</form>\n\t</div>\n\t<?php\n}}\n",
        wc = if woocommerce { " * Requires Plugins: woocommerce\n * WC requires at least: 8.0\n" } else { "" },
        up = func.to_ascii_uppercase(),
    );
    if woocommerce {
        let _ = write!(
            main,
            "\n// WooCommerce: declare HPOS compatibility and react to completed orders.\n\
add_action( 'before_woocommerce_init', function () {{\n\tif ( class_exists( \\Automattic\\WooCommerce\\Utilities\\FeaturesUtil::class ) ) {{\n\
\t\t\\Automattic\\WooCommerce\\Utilities\\FeaturesUtil::declare_compatibility( 'custom_order_tables', __FILE__, true );\n\t}}\n}} );\n\n\
add_action( 'woocommerce_order_status_completed', function ( $order_id ) {{\n\t$order = wc_get_order( $order_id );\n\tif ( ! $order ) {{\n\t\treturn;\n\t}}\n\
\t$order->add_order_note( esc_html__( 'Handled by {name}.', '{slug}' ) );\n}} );\n"
        );
    }
    vec![
        (format!("{slug}/{slug}.php"), main),
        (
            format!("{slug}/uninstall.php"),
            format!("<?php\n// Removes the plugin's options when it is deleted.\ndefined( 'WP_UNINSTALL_PLUGIN' ) || exit;\ndelete_option( '{func}_settings' );\n"),
        ),
        (
            format!("{slug}/readme.txt"),
            format!("=== {name} ===\nRequires at least: 6.0\nTested up to: 6.7\nRequires PHP: 7.4\nStable tag: 1.0.0\nLicense: GPLv2 or later\n\n{name}.\n"),
        ),
        (format!("{slug}/languages/.gitkeep"), String::new()),
    ]
}

/// Write scaffold files under `dir`; existing files are not overwritten.
pub fn write_scaffold(dir: &Path, files: &[(String, String)]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for (rel, text) in files {
        let p = dir.join(rel);
        if p.exists() {
            return Err(format!("{} already exists", p.display()));
        }
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::host::tests::FakeShell;
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn helpers() {
        assert_eq!(slug_of("akismet/akismet.php"), "akismet");
        assert_eq!(slug_of("hello.php"), "hello");
        assert_eq!(generator_version(r#"<meta name="generator" content="WordPress 6.6.2" />"#).as_deref(), Some("6.6.2"));
        assert_eq!(generator_version("<generator>https://wordpress.org/?v=6.5</generator>").as_deref(), Some("6.5"));
        let hits = scan_code("x.php", "<?php\n$a=1;\neval(base64_decode($x));\n");
        assert_eq!(hits, vec![("x.php".into(), 3, "eval of decoded data".into())]);
    }

    #[test]
    fn site_profiles_validate() {
        let mut s = WpSite { url: "http://shop.example.com/".into(), user: Some("admin".into()), secret: Some("WP_SHOP".into()), ..WpSite::default() };
        assert!(s.validate().is_err());
        s.url = "https://shop.example.com/".into();
        s.validate().unwrap();
        assert_eq!(s.url, "https://shop.example.com");
        let mut cli = WpSite { url: "https://a.com".into(), ssh: Some("prod".into()), ..WpSite::default() };
        assert!(cli.validate().is_err());
    }

    #[tokio::test]
    async fn rest_inventory_with_woocommerce() {
        let s = MockServer::start().await;
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("admin:abcd efgh"));
        Mock::given(path("/wp-json/wp/v2/plugins")).and(header("authorization", auth.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"plugin": "woocommerce/woocommerce.php", "name": "WooCommerce", "version": "9.1.0", "status": "active"},
                {"plugin": "iyzico-woocommerce/iyzico.php", "name": "iyzico", "version": "3.5", "status": "inactive"}
            ]))).mount(&s).await;
        Mock::given(path("/wp-json/wp/v2/themes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"stylesheet": "storefront", "name": {"raw": "Storefront"}, "version": "4.6", "status": "active"}]))).mount(&s).await;
        Mock::given(path("/wp-json/wp/v2/users"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-wp-total", "7").set_body_json(json!([]))).mount(&s).await;
        Mock::given(path("/wp-json/wc/v3/products")).and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-wp-total", "120").set_body_json(json!([]))).mount(&s).await;
        Mock::given(path("/wp-json/wc/v3/orders"))
            .respond_with(ResponseTemplate::new(200).insert_header("x-wp-total", "45").set_body_json(json!([]))).mount(&s).await;
        Mock::given(path("/feed/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<generator>https://wordpress.org/?v=6.6.2</generator>")).mount(&s).await;
        let site = WpSite { url: s.uri(), user: Some("admin".into()), secret: Some("X".into()), ..WpSite::default() };
        let wp = Wp::new("shop", site, Some("abcd efgh".into()), None).unwrap();
        let inv = wp.inventory().await.unwrap();
        assert_eq!(inv.version.as_deref(), Some("6.6.2"));
        assert_eq!(inv.plugins.len(), 2);
        assert!(inv.woocommerce);
        assert_eq!((inv.users, inv.products, inv.orders), (Some(7), Some(120), Some(45)));
        assert_eq!(inv.themes[0].name, "Storefront");
    }

    #[tokio::test]
    async fn updates_back_up_first_and_roll_back_a_breaking_plugin() {
        let s = MockServer::start().await;
        // Home fails after the update: the plugin goes back to its old version.
        Mock::given(path("/")).respond_with(ResponseTemplate::new(500)).mount(&s).await;
        let sh = Arc::new(FakeShell { answers: vec![("tar czf", 0, "/home/u/kor-backups/x.tar.gz\n".into())], ..FakeShell::default() });
        let site = WpSite { url: s.uri(), ssh: Some("prod".into()), path: Some("/home/u/www".into()), ..WpSite::default() };
        let wp = Wp::new("shop", site, None, Some(sh.clone())).unwrap();
        let inv = WpInventory {
            plugins: vec![Plugin { slug: "contact-form-7".into(), version: "5.8".into(), update: Some("5.9".into()), active: true, ..Plugin::default() }],
            ..WpInventory::default()
        };
        let rep = update(&wp, "/home/u/www", &s.uri(), &inv, &[]).await.unwrap();
        assert_eq!(rep.backup, ["/home/u/kor-backups/x.tar.gz"]);
        assert_eq!(rep.items[0].result, "rolled-back");
        let cmds = sh.commands();
        let first = cmds.iter().position(|c| c.contains("tar czf")).unwrap();
        let upd = cmds.iter().position(|c| c.contains("plugin update contact-form-7")).unwrap();
        assert!(first < upd, "backup must run before the update");
        assert!(cmds.iter().any(|c| c.contains("plugin install contact-form-7 --version=5.8 --force")));
    }

    #[test]
    fn scaffolds_are_safe_by_default() {
        let files = plugin_skeleton("Kargo Takip", "kargo-takip", true);
        let main = &files[0].1;
        for must in ["wp_verify_nonce", "sanitize_text_field", "esc_attr", "current_user_can", "load_plugin_textdomain", "declare_compatibility"] {
            assert!(main.contains(must), "{must}");
        }
        let theme = child_theme("storefront", "Acme Child", "acme-child");
        assert!(theme[0].1.contains("Template: storefront"));
        let dir = tempfile::tempdir().unwrap();
        write_scaffold(dir.path(), &theme).unwrap();
        assert!(write_scaffold(dir.path(), &theme).is_err());
    }
}
