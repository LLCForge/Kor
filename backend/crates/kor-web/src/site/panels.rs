//! Working inside hosting panels from the Chrome side panel (§29.2).
//!
//! Panel maps live in `assets/panels/*.json` (embedded here, open to contributions). When the user
//! is on cPanel, Plesk, DirectAdmin, CyberPanel, HestiaCP, Cloudflare or a registrar/provider client
//! area, the extension asks the desktop bridge (`{"type":"panel", …}`, see
//! `kor_tools::chrome::set_panel_handler`) and this module answers:
//! - `detect`: which panel this is, the matching `kor host` profile and the contextual commands;
//! - `run`: an API-backed command through `kor host` — first the plan, then (after the user clicks
//!   approve in the side panel) the change with verification and undo;
//! - `fill`: form values for the page, which the extension types into fields matched by label
//!   (never password fields; the user presses submit);
//! - `explain`: known error messages and risky settings found on the screen.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use kor_i18n::{Lang, Localizer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Result;
use super::host::{self, Task};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Match {
    pub ports: Vec<u16>,
    pub hosts: Vec<String>,
    pub paths: Vec<String>,
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Field {
    pub labels: Vec<String>,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Command {
    pub id: String,
    pub label: BTreeMap<String, String>,
    /// `api`, `fill`, `explain`.
    pub action: String,
    pub task: Option<String>,
    pub needs: Vec<String>,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelMap {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    #[serde(rename = "match")]
    pub matcher: Match,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Note {
    #[serde(rename = "match")]
    matcher: String,
    #[serde(default)]
    risky: Vec<String>,
    text: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct Common {
    errors: Vec<Note>,
    settings: Vec<Note>,
}

macro_rules! maps {
    ($($id:literal),*) => { &[$(include_str!(concat!("../../../../../assets/panels/", $id, ".json"))),*] };
}

const MAPS: &[&str] = maps!(
    "cpanel", "whm", "plesk", "directadmin", "cyberpanel", "hestia", "cloudflare", "natro", "turhost", "guzelhosting", "hostinger",
    "namecheap", "godaddy"
);

static PANELS: LazyLock<Vec<PanelMap>> = LazyLock::new(|| MAPS.iter().filter_map(|t| serde_json::from_str(t).ok()).collect());
static COMMON: LazyLock<Common> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../../../../assets/panels/_common.json")).unwrap_or_default());

pub fn maps() -> &'static [PanelMap] {
    &PANELS
}

/// The panel the page belongs to: host first (provider sites), then port, path and title markers.
pub fn detect(url: &str, title: &str) -> Option<&'static PanelMap> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    let port = u.port_or_known_default();
    let path = u.path().to_owned() + "?" + u.query().unwrap_or("");
    let title = title.to_lowercase();
    let by_host = PANELS
        .iter()
        .find(|p| p.matcher.hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))));
    by_host.or_else(|| {
        PANELS.iter().find(|p| {
            let m = &p.matcher;
            let port_hit = port.is_some_and(|x| m.ports.contains(&x));
            let path_hit = m.paths.iter().any(|x| path.contains(x.as_str()));
            let marker_hit = m.markers.iter().any(|x| title.contains(x.as_str()));
            (port_hit && (path_hit || marker_hit || m.paths.is_empty())) || (marker_hit && path_hit)
        })
    })
}

/// The `kor host` profile whose panel URL has the page's host and port.
pub fn profile_for(base: &Path, url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    host::list(base).into_iter().find_map(|(name, p)| {
        let p = p.ok()?;
        let pu = url::Url::parse(p.url.as_deref()?).ok()?;
        (pu.host_str() == u.host_str() && pu.port_or_known_default() == u.port_or_known_default()).then_some(name)
    })
}

fn pick(texts: &BTreeMap<String, String>, lang: &str) -> String {
    texts.get(lang).or_else(|| texts.get("en")).cloned().unwrap_or_default()
}

/// Notes for what the screen shows: known errors and risky settings.
pub fn explain(text: &str, lang: &str) -> Vec<String> {
    let t = text.to_lowercase();
    let mut out: Vec<String> = COMMON.errors.iter().filter(|n| t.contains(&n.matcher)).map(|n| pick(&n.text, lang)).collect();
    for s in &COMMON.settings {
        if t.contains(&s.matcher) && s.risky.iter().any(|r| t.contains(r.as_str())) {
            let note = pick(&s.text, lang);
            if !out.contains(&note) {
                out.push(note);
            }
        }
    }
    out
}

fn val<'a>(values: &'a BTreeMap<String, String>, k: &str) -> Result<&'a str> {
    values.get(k).map(|s| s.trim()).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing value: {k}"))
}

/// A `kor host` task from a command id and form values (side panel, desktop).
pub fn task_from(id: &str, values: &BTreeMap<String, String>) -> Result<Task> {
    let v = |k| val(values, k).map(str::to_owned);
    Ok(match id {
        "ssl" => Task::Ssl { domain: v("domain")? },
        "php" => Task::Php { domain: v("domain")?, version: v("version")? },
        "php-ini" => Task::PhpIni { domain: v("domain")?, key: v("key")?, value: v("value")? },
        "backup" => Task::Backup { domain: v("domain").ok() },
        "restore" => Task::Restore { archive: v("archive")?, domain: v("domain")? },
        "subdomain" => Task::Subdomain { sub: v("sub")?, domain: v("domain")?, dir: v("dir").ok() },
        "db" => Task::Database { name: v("name")?, user: v("user")? },
        "email" => Task::Email {
            address: v("address")?,
            quota_mb: values.get("quota").and_then(|q| q.trim().parse().ok()).unwrap_or(1024),
        },
        "cron" => Task::Cron { schedule: v("schedule")?, command: v("command")? },
        "perms" => Task::Perms { path: v("path")? },
        "clean" => Task::Clean {
            paths: values.get("paths").map(|p| p.lines().map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
        },
        "sftp" => Task::Sftp { user: v("user")?, path: v("path")?, domain: v("domain").ok() },
        other => return Err(format!("unknown task `{other}`")),
    })
}

pub fn lang_of(code: &str) -> Lang {
    match code {
        "en" => Lang::En,
        "fr" => Lang::Fr,
        _ => Lang::Tr,
    }
}

fn values_of(v: &Value) -> BTreeMap<String, String> {
    v["values"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.chars().take(500).collect())))
        .collect()
}

fn find(panel: &str, command: &str) -> Result<(&'static PanelMap, &'static Command)> {
    let p = PANELS.iter().find(|p| p.id == panel).ok_or_else(|| format!("unknown panel `{panel}`"))?;
    let c = p.commands.iter().find(|c| c.id == command).ok_or_else(|| format!("unknown command `{command}`"))?;
    Ok((p, c))
}

/// Form values for a `fill` command: field labels to match and the value to type.
pub fn fill(panel: &str, command: &str, values: &BTreeMap<String, String>) -> Result<Vec<Field>> {
    let (_, c) = find(panel, command)?;
    if c.action != "fill" {
        return Err(format!("`{command}` is not a form command"));
    }
    c.fields
        .iter()
        .filter_map(|f| values.get(&f.value).filter(|v| !v.trim().is_empty()).map(|v| Ok(Field { labels: f.labels.clone(), value: v.trim().to_owned() })))
        .collect()
}

async fn run(base: &Path, v: &Value) -> Result<Value> {
    let lang = v["lang"].as_str().unwrap_or("tr");
    let l = Localizer::new(lang_of(lang));
    let (_, c) = find(v["panel"].as_str().unwrap_or(""), v["command"].as_str().unwrap_or(""))?;
    if c.action != "api" {
        return Err(format!("`{}` does not run through the panel API", c.id));
    }
    let profile = v["profile"].as_str().ok_or_else(|| l.t("web-panel-no-profile"))?;
    let store = super::vault().ok();
    let (prof, panel) = host::connect(base, profile, store.as_deref(), None)?;
    let Some(task_id) = &c.task else {
        let inv = host::inventory(panel.as_ref()).await?;
        return Ok(json!({ "markdown": host::inventory_markdown(&l, &inv, super::now()), "inventory": inv }));
    };
    let task = task_from(task_id, &values_of(v))?;
    task.validate()?;
    if v["approve"] != Value::Bool(true) {
        let inv = host::inventory(panel.as_ref()).await?;
        return Ok(json!({ "needsApproval": true, "plan": host::plan(&l, &task, &inv, &[]) }));
    }
    let rep = host::run(&l, profile, &prof, panel.as_ref(), task, &[], store.as_deref(), &|_| true).await;
    super::client::log_auto(base, prof.client.as_deref(), &format!("host-{task_id}"), profile, rep.applied.as_deref().unwrap_or(""), rep.secs);
    serde_json::to_value(&rep).map_err(|e| e.to_string())
}

/// One side-panel request (see the module comment). Errors come back as `{"error": "…"}`.
pub async fn handle(v: Value) -> Value {
    let base = match super::base() {
        Ok(b) => b,
        Err(e) => return json!({ "error": e }),
    };
    let lang = v["lang"].as_str().unwrap_or("tr").to_owned();
    let r: Result<Value> = match v["op"].as_str().unwrap_or("") {
        "detect" => {
            let url = v["url"].as_str().unwrap_or("");
            Ok(match detect(url, v["title"].as_str().unwrap_or("")) {
                Some(p) => json!({ "panel": {
                    "id": p.id, "name": p.name, "kind": p.kind,
                    "profile": if p.kind.is_some() { profile_for(&base, url) } else { None },
                    "commands": p.commands.iter().map(|c| json!({
                        "id": c.id, "label": pick(&c.label, &lang), "action": c.action, "needs": c.needs,
                        "fields": c.fields.iter().map(|f| f.value.clone()).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                }}),
                None => json!({ "panel": null }),
            })
        }
        "explain" => {
            let text = format!("{}\n{}", v["title"].as_str().unwrap_or(""), v["text"].as_str().unwrap_or(""));
            let notes = explain(&text.chars().take(20_000).collect::<String>(), &lang);
            Ok(json!({ "notes": notes, "none": Localizer::new(lang_of(&lang)).t("web-panel-explain-none") }))
        }
        "fill" => fill(v["panel"].as_str().unwrap_or(""), v["command"].as_str().unwrap_or(""), &values_of(&v))
            .map(|f| json!({ "fields": f })),
        "run" => run(&base, &v).await,
        other => Err(format!("unknown op `{other}`")),
    };
    r.unwrap_or_else(|e| json!({ "error": e }))
}

/// For `kor_tools::chrome::set_panel_handler`.
pub fn handler(v: Value) -> futures::future::BoxFuture<'static, Value> {
    Box::pin(handle(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_map_parses_and_has_explain() {
        assert_eq!(maps().len(), MAPS.len());
        for p in maps() {
            assert!(p.commands.iter().any(|c| c.action == "explain"), "{}", p.id);
            for c in &p.commands {
                assert!(c.label.contains_key("tr") && c.label.contains_key("en") && c.label.contains_key("fr"), "{}/{}", p.id, c.id);
                if c.action == "api" && let Some(t) = &c.task {
                    let values: BTreeMap<String, String> = c.needs.iter().map(|n| (n.clone(), "x".into())).collect();
                    assert!(task_from(t, &values).is_ok(), "{}/{}", p.id, c.id);
                }
            }
        }
        assert!(!COMMON.errors.is_empty() && !COMMON.settings.is_empty());
    }

    #[test]
    fn detects_panels() {
        let id = |u: &str, t: &str| detect(u, t).map(|p| p.id.clone());
        assert_eq!(id("https://srv.example.com:2083/cpsess123/frontend/jupiter/index.html", "cPanel"), Some("cpanel".into()));
        assert_eq!(id("https://srv.example.com:8443/smb/web/view", "Plesk Obsidian"), Some("plesk".into()));
        assert_eq!(id("https://dash.cloudflare.com/abc/example.com/dns", ""), Some("cloudflare".into()));
        assert_eq!(id("https://www.natro.com/musteri/alan-adi", ""), Some("natro".into()));
        assert_eq!(id("https://srv.example.com:2222/evo/user", "DirectAdmin"), Some("directadmin".into()));
        assert_eq!(id("https://example.com/", "Blog"), None);
    }

    #[test]
    fn explains_screens() {
        let n = explain("Error 522 Connection timed out — SSL/TLS: Flexible", "tr");
        assert_eq!(n.len(), 2, "{n:?}");
        assert!(explain("display_errors On", "en")[0].contains("display_errors"));
        assert!(explain("all good", "en").is_empty());
    }

    #[test]
    fn fill_values_and_tasks() {
        let mut v = BTreeMap::new();
        v.insert("ns1".to_owned(), "ns1.example.net".to_owned());
        v.insert("ns2".to_owned(), " ".to_owned());
        let f = fill("natro", "nameservers", &v).unwrap();
        assert_eq!(f.len(), 1);
        assert!(f[0].labels.iter().any(|l| l == "ad sunucusu 1"));
        assert!(fill("cpanel", "ssl", &v).is_err());
        assert!(task_from("php", &v).is_err());
    }

    #[tokio::test]
    async fn handler_answers_detect_and_errors() {
        let r = handle(json!({"op": "detect", "url": "https://dash.cloudflare.com/x", "lang": "en"})).await;
        assert_eq!(r["panel"]["id"], "cloudflare");
        assert_eq!(r["panel"]["commands"][0]["label"], "Fill in the DNS record form");
        let r = handle(json!({"op": "nope"})).await;
        assert!(r["error"].is_string());
    }
}
