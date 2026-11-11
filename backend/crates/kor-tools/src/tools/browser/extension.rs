//! The `browser_*` tools on the user's own browser, through the Kor extension (§18.2, ADR 0058).
//!
//! Same tools and output as the headless backend (the extension injects the same `snapshot.js`, so
//! refs, `browser_read` and `browser_find` behave alike). Differences, all deliberate:
//! - every agent session (task) has its own tab group, named after the project, in Kor's ember
//!   colour; Kor lists, reads and acts only on tabs of its group. A tab outside it is reached only
//!   when the user hands it over from the side panel ("Bu sekmeyi Kor'a ver");
//! - the user's domain lists (`kor chrome allow|deny`) are checked before every navigation and,
//!   by the extension, before every action on a page;
//! - privacy and offline mode refuse the user's browser entirely (it is online by nature);
//! - input events are DOM events from a content script (no debugger attached), so trusted-input
//!   checks of a site may differ from a person's clicks. Password, card, CVV and one-time-code
//!   fields are refused by the extension.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use kor_core::ImageSource;
use serde_json::{Value, json};

use super::{Args, Op, UNTRUSTED, checked_url, find_nodes, format_node, format_snapshot, need};
use crate::tools::chrome::{Bridge, policy};
use crate::{ToolCtx, ToolOutput, lock};

const NAV_TIMEOUT: Duration = Duration::from_secs(40);
const ACT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_UPLOAD: u64 = 10 * 1024 * 1024;

/// The tab group of one agent session.
#[derive(Debug)]
pub(super) struct Group {
    key: String,
    current: Mutex<Option<i64>>,
}

impl Default for Group {
    fn default() -> Self {
        let mut b = [0u8; 8];
        let _ = getrandom::fill(&mut b);
        Self {
            key: b.iter().fold(String::new(), |mut s, x| {
                let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{x:02x}"));
                s
            }),
            current: Mutex::new(None),
        }
    }
}

fn title(ctx: &ToolCtx) -> String {
    let name = ctx
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        "Kor".into()
    } else {
        format!("Kor · {name}")
    }
}

struct Run<'a> {
    bridge: &'a Bridge,
    group: &'a Group,
    task: Value,
}

impl Run<'_> {
    async fn cmd(&self, name: &str, mut body: Value, timeout: Duration) -> Result<Value, String> {
        body["cmd"] = json!(name);
        body["task"] = self.task.clone();
        self.bridge.request(body, timeout).await
    }

    fn tab(&self) -> Result<i64, String> {
        lock(&self.group.current)
            .ok_or_else(|| "No Kor tab yet; start with browser_open.".to_owned())
    }

    async fn act(&self, action: &str, mut body: Value) -> Result<Value, String> {
        body["action"] = json!(action);
        body["tabId"] = json!(self.tab()?);
        self.cmd("act", body, ACT_TIMEOUT).await
    }

    async fn snapshot(&self) -> Result<Value, String> {
        self.cmd("snapshot", json!({"tabId": self.tab()?}), ACT_TIMEOUT)
            .await
    }

    async fn open(&self, url: &str, new_tab: bool) -> Result<(), String> {
        let current = *lock(&self.group.current);
        if let Some(tab) = current.filter(|_| !new_tab) {
            self.cmd("navigate", json!({"tabId": tab, "url": url}), NAV_TIMEOUT)
                .await?;
        } else {
            let r = self
                .cmd("tabs.create", json!({"url": url}), NAV_TIMEOUT)
                .await?;
            let id = r["tabId"]
                .as_i64()
                .ok_or("the extension returned no tab id")?;
            *lock(&self.group.current) = Some(id);
        }
        Ok(())
    }

    async fn tabs(&self, a: &Args) -> Result<String, String> {
        let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
        let mut tabs = list(&self.cmd("tabs.list", json!({}), ACT_TIMEOUT).await?);
        let pick = |tabs: &[Value], id: Option<usize>| -> Result<i64, String> {
            let id = id.ok_or("id is required")?;
            tabs.get(id.wrapping_sub(1))
                .and_then(|t| t["tabId"].as_i64())
                .ok_or_else(|| format!("no tab {id}"))
        };
        match a.action.as_deref().unwrap_or("list") {
            "list" => {}
            "new" => {
                let url = a
                    .url
                    .as_deref()
                    .map_or_else(|| Ok("about:blank".to_owned()), checked_url)?;
                if url != "about:blank" {
                    policy::current().check(&url)?;
                }
                self.open(&url, true).await?;
                tabs = list(&self.cmd("tabs.list", json!({}), ACT_TIMEOUT).await?);
            }
            "switch" => {
                let tab = pick(&tabs, a.id)?;
                self.cmd("tabs.select", json!({"tabId": tab}), ACT_TIMEOUT)
                    .await?;
                *lock(&self.group.current) = Some(tab);
            }
            "close" => {
                let tab = match a.id {
                    Some(_) => pick(&tabs, a.id)?,
                    None => self.tab()?,
                };
                self.cmd("tabs.close", json!({"tabId": tab}), ACT_TIMEOUT)
                    .await?;
                tabs.retain(|t| t["tabId"].as_i64() != Some(tab));
                let mut cur = lock(&self.group.current);
                if *cur == Some(tab) {
                    *cur = tabs.last().and_then(|t| t["tabId"].as_i64());
                }
            }
            other => return Err(format!("unknown action `{other}`")),
        }
        let current = *lock(&self.group.current);
        let lines: Vec<String> = tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mark = if t["tabId"].as_i64() == current {
                    "*"
                } else {
                    " "
                };
                format!(
                    "{mark}{} {} — {}",
                    i + 1,
                    t["title"].as_str().unwrap_or(""),
                    t["url"].as_str().unwrap_or("")
                )
            })
            .collect();
        Ok(format!(
            "{UNTRUSTED}\nTabs of Kor's group (* current):\n{}",
            lines.join("\n")
        ))
    }
}

fn data_url_image(url: &str) -> Result<ImageSource, String> {
    let (head, data) = url
        .split_once(',')
        .ok_or("the extension returned no image")?;
    let media_type = head
        .strip_prefix("data:")
        .and_then(|h| h.strip_suffix(";base64"))
        .filter(|m| matches!(*m, "image/png" | "image/jpeg"))
        .ok_or("the extension returned an unexpected image type")?;
    Ok(ImageSource::Base64 {
        media_type: media_type.into(),
        data: data.to_owned(),
    })
}

fn upload_files(ctx: &ToolCtx, paths: &[String]) -> Result<Vec<Value>, String> {
    if paths.is_empty() || paths.len() > 10 {
        return Err("paths: 1 to 10 project files".into());
    }
    paths
        .iter()
        .map(|p| {
            let path = ctx.resolve(p)?;
            let meta = std::fs::metadata(&path).map_err(|e| format!("{p}: {e}"))?;
            if !meta.is_file() || meta.len() > MAX_UPLOAD {
                return Err(format!("{p}: not a file or larger than 10 MB"));
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{p}: {e}"))?;
            if !crate::secret_labels(&String::from_utf8_lossy(&bytes)).is_empty() {
                return Err(format!("{p} looks like it contains secrets; Kor does not upload it"));
            }
            Ok(json!({
                "name": Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "mime": super::mime_of(&path),
                "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
            }))
        })
        .collect()
}

/// Run one browser tool call in the user's browser.
#[allow(clippy::too_many_lines)]
pub(super) async fn run(
    op: Op,
    a: Args,
    ctx: &ToolCtx,
    group: &Group,
    bridge: &Bridge,
) -> Result<ToolOutput, String> {
    if kor_core::net::policy() != kor_core::net::Policy::Open {
        return Err("Privacy or offline mode is on: Kor does not drive the user's browser. Use browser_open with the headless backend for local pages (`kor config browser.backend headless`).".into());
    }
    let r = Run {
        bridge,
        group,
        task: json!({"key": group.key, "title": title(ctx)}),
    };
    if lock(&group.current).is_none() && !matches!(op, Op::Open | Op::Tabs) {
        return Err("No Kor tab yet; start with browser_open.".into());
    }
    let text = match op {
        Op::Open => {
            let url = checked_url(need(a.url.as_ref(), "url")?)?;
            policy::current().check(&url)?;
            r.open(&url, a.new_tab).await?;
            format_snapshot(&r.snapshot().await?, "interactive", 0, 4000)
        }
        Op::Read => {
            let mode = a.mode.as_deref().unwrap_or("interactive");
            let limit = a.limit.unwrap_or(12_000).clamp(500, 60_000);
            format_snapshot(&r.snapshot().await?, mode, a.offset.unwrap_or(0), limit)
        }
        Op::Find => {
            let query = need(a.query.as_ref(), "query")?;
            let snap = r.snapshot().await?;
            let nodes = snap["nodes"].as_array().cloned().unwrap_or_default();
            let found = find_nodes(&nodes, query, a.limit.unwrap_or(8).clamp(1, 20));
            if found.is_empty() {
                format!("No element matches `{query}`. Try browser_read.")
            } else {
                let lines: Vec<String> = found.into_iter().map(format_node).collect();
                format!("{UNTRUSTED}\n{}", lines.join("\n"))
            }
        }
        Op::Click => {
            let rf = need(a.reference.as_ref(), "ref")?;
            let button = a.button.as_deref().unwrap_or("left");
            if !["left", "right", "middle"].contains(&button) {
                return Err("button must be left, right or middle".into());
            }
            let v = r
                .act(
                    "click",
                    json!({"ref": rf, "button": button, "double": a.double}),
                )
                .await?;
            format!("clicked {rf}; now at {}", v["url"].as_str().unwrap_or(""))
        }
        Op::Fill => {
            let rf = need(a.reference.as_ref(), "ref")?;
            let text = a.text.clone().unwrap_or_default();
            r.act("fill", json!({"ref": rf, "text": text, "clear": a.clear.unwrap_or(true), "submit": a.submit}))
                .await?;
            format!(
                "typed {} characters into {rf}{}",
                text.chars().count(),
                if a.submit { " and pressed Enter" } else { "" }
            )
        }
        Op::Press => {
            let key = need(a.key.as_ref(), "key")?;
            r.act("press", json!({"key": key})).await?;
            format!("pressed {key}")
        }
        Op::Scroll => {
            let v = r
                .act("scroll", json!({"ref": a.reference, "direction": a.direction.as_deref().unwrap_or("down"), "amount": a.amount}))
                .await?;
            v["message"].as_str().unwrap_or("scrolled").to_owned()
        }
        Op::Select => {
            let rf = need(a.reference.as_ref(), "ref")?;
            let v = r
                .act(
                    "select",
                    json!({"ref": rf, "value": a.value.clone().unwrap_or_default()}),
                )
                .await?;
            format!("{rf}: {}", v["message"].as_str().unwrap_or("selected"))
        }
        Op::Hover => {
            let rf = need(a.reference.as_ref(), "ref")?;
            r.act("hover", json!({"ref": rf})).await?;
            format!("hovering {rf}")
        }
        Op::Wait => {
            let ms = a.ms.unwrap_or(10_000).min(60_000);
            if let Some(t) = a.text.as_deref().filter(|t| !t.is_empty()) {
                let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
                loop {
                    if r.act("hasText", json!({"text": t})).await?["found"] == true {
                        break format!("`{t}` is on the page");
                    }
                    if tokio::time::Instant::now() > deadline || ctx.cancel.is_cancelled() {
                        return Err(format!("`{t}` did not appear within {ms} ms"));
                    }
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            } else {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(ms)) => {}
                    () = ctx.cancel.cancelled() => return Err("cancelled".into()),
                }
                format!("waited {ms} ms")
            }
        }
        Op::Back => {
            let action = a.action.as_deref().unwrap_or("back");
            if !["back", "forward", "reload"].contains(&action) {
                return Err("action must be back, forward or reload".into());
            }
            let v = r
                .cmd(
                    "history",
                    json!({"tabId": r.tab()?, "action": action}),
                    NAV_TIMEOUT,
                )
                .await?;
            format!("{action}; now at {}", v["url"].as_str().unwrap_or(""))
        }
        Op::Tabs => r.tabs(&a).await?,
        Op::Console => {
            let v = r
                .cmd(
                    "console",
                    json!({"tabId": r.tab()?, "clear": a.clear == Some(true)}),
                    ACT_TIMEOUT,
                )
                .await?;
            let lines: Vec<&str> = v
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if lines.is_empty() {
                "No console output.".to_owned()
            } else {
                format!("{UNTRUSTED}\n{}", lines.join("\n"))
            }
        }
        Op::Network => {
            let v = r
                .cmd(
                    "network",
                    json!({"tabId": r.tab()?, "clear": a.clear == Some(true)}),
                    ACT_TIMEOUT,
                )
                .await?;
            let filter = a.filter.as_deref().unwrap_or("").to_lowercase();
            let failed = |e: &Value| {
                e["error"].is_string() || e["status"].as_u64().is_some_and(|s| s >= 400)
            };
            let picked: Vec<&Value> = v
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| !a.failed || failed(e))
                .filter(|e| {
                    filter.is_empty()
                        || e["url"]
                            .as_str()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&filter)
                })
                .collect();
            let limit = a.limit.unwrap_or(50).clamp(1, 300);
            let skipped = picked.len().saturating_sub(limit);
            let lines: Vec<String> = picked[skipped..]
                .iter()
                .map(|e| {
                    let status = e["error"].as_str().map_or_else(
                        || {
                            e["status"]
                                .as_u64()
                                .map_or_else(|| "pending".to_owned(), |s| s.to_string())
                        },
                        |err| format!("failed {err}"),
                    );
                    format!(
                        "{} {status} {} {}",
                        e["method"].as_str().unwrap_or("GET"),
                        e["type"].as_str().unwrap_or(""),
                        e["url"].as_str().unwrap_or("")
                    )
                })
                .collect();
            if lines.is_empty() {
                "No network requests recorded for this tab.".to_owned()
            } else {
                let failures = picked.iter().filter(|e| failed(e)).count();
                format!(
                    "{UNTRUSTED}\n{} request(s), {failures} failed:\n{}",
                    picked.len(),
                    lines.join("\n")
                )
            }
        }
        Op::Screenshot => {
            let v = r
                .cmd(
                    "screenshot",
                    json!({"tabId": r.tab()?, "fullPage": a.full_page}),
                    NAV_TIMEOUT,
                )
                .await?;
            let mut out =
                ToolOutput::ok(format!("Screenshot of {}", v["url"].as_str().unwrap_or("")));
            out.image = Some(data_url_image(v["dataUrl"].as_str().unwrap_or(""))?);
            return Ok(out);
        }
        Op::Eval => {
            let code = need(a.code.as_ref(), "code")?;
            let v = r
                .cmd(
                    "eval",
                    json!({"tabId": r.tab()?, "code": code}),
                    ACT_TIMEOUT,
                )
                .await?;
            format!("{UNTRUSTED}\n{}", crate::truncate(&v.to_string(), 20_000))
        }
        Op::Upload => {
            let rf = need(a.reference.as_ref(), "ref")?;
            let files = upload_files(ctx, a.paths.as_deref().unwrap_or_default())?;
            let n = files.len();
            r.act("upload", json!({"ref": rf, "files": files})).await?;
            format!("attached {n} file(s) to {rf}")
        }
        Op::Gif => {
            let count = a.count.unwrap_or(10).clamp(2, 50);
            let interval = a.interval_ms.unwrap_or(600).clamp(550, 3000);
            let v = r
                .cmd(
                    "frames",
                    json!({"tabId": r.tab()?, "count": count, "intervalMs": interval}),
                    Duration::from_millis(u64::from(count) * (interval + 1500) + 10_000),
                )
                .await?;
            let frames: Vec<Vec<u8>> = v
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|u| u.split_once(',').map(|(_, d)| d))
                .filter_map(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
                .collect();
            return super::save_gif(ctx, &frames, interval);
        }
    };
    Ok(ToolOutput::ok(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::chrome::server::tests::{paired, recv, send};

    #[tokio::test]
    async fn tools_run_in_the_task_group_and_refuse_without_a_tab() {
        let (bridge, mut ws, _dir) = paired().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let group = Group::default();
        let none = run(Op::Read, Args::default(), &ctx, &group, &bridge).await;
        assert!(none.unwrap_err().contains("browser_open"));
        let ext = tokio::spawn(async move {
            // tabs.create, then snapshot.
            let c = recv(&mut ws).await;
            assert_eq!(c["cmd"], "tabs.create");
            assert!(c["task"]["title"].as_str().unwrap().starts_with("Kor · "));
            send(
                &mut ws,
                json!({"id": c["id"], "ok": true, "result": {"tabId": 42}}),
            )
            .await;
            let c = recv(&mut ws).await;
            assert_eq!(
                (c["cmd"].as_str(), c["tabId"].as_i64()),
                (Some("snapshot"), Some(42))
            );
            send(&mut ws, json!({"id": c["id"], "ok": true, "result": {
                "url": "https://example.com/", "title": "Example", "scroll": {"y": 0, "height": 900, "viewport": 800},
                "nodes": [{"ref": "e1", "role": "button", "name": "Sign in"}], "text": "Hello"}})).await;
            let c = recv(&mut ws).await;
            assert_eq!(
                (c["action"].as_str(), c["ref"].as_str()),
                (Some("fill"), Some("e1"))
            );
            send(
                &mut ws,
                json!({"id": c["id"], "ok": false, "error": "Güvenlik: parola alanına yazılmaz"}),
            )
            .await;
            ws
        });
        let a = Args {
            url: Some("https://example.com".into()),
            ..Args::default()
        };
        let out = run(Op::Open, a, &ctx, &group, &bridge).await.unwrap();
        assert!(out.content.contains("Sign in") && out.content.starts_with(UNTRUSTED));
        let a = Args {
            reference: Some("e1".into()),
            text: Some("x".into()),
            ..Args::default()
        };
        assert!(
            run(Op::Fill, a, &ctx, &group, &bridge)
                .await
                .unwrap_err()
                .contains("parola")
        );
        drop(ext.await.unwrap());
    }

    #[test]
    fn uploads_stay_in_the_project_and_skip_secrets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cv.pdf"), b"%PDF-1.4 plain").unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "GH_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n",
        )
        .unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let ok = upload_files(&ctx, &["cv.pdf".into()]).unwrap();
        assert_eq!(ok[0]["mime"], "application/pdf");
        assert!(upload_files(&ctx, &["../x".into()]).is_err());
        assert!(upload_files(&ctx, &[".env".into()]).is_err());
        assert!(upload_files(&ctx, &[]).is_err());
        assert!(data_url_image("data:image/svg+xml;base64,AAAA").is_err());
        assert!(data_url_image("data:image/png;base64,AAAA").is_ok());
    }

    const PAGE: &str = "<!doctype html><title>Kor e2e</title><h1>Form</h1><form><label>E-posta <input id=m name=email></label><label>Parola <input type=password id=p></label><button type=button id=b onclick=\"console.log('kor-e2e-click')\">Gönder</button></form>";

    async fn serve() -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        port
    }

    /// The built extension in a real browser, driven through the bridge like the agent does:
    /// pairing with a one-time code from the side panel page, a task tab group, snapshot, fill,
    /// the password refusal, click + console, network, tabs, and the refusal of a foreign tab.
    /// `cd frontend && npm run build:extension`, then
    /// `KOR_BROWSER=<Chrome for Testing or Chromium> cargo test -p kor-tools -- --ignored real_extension`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the built extension, KOR_BROWSER = Chromium / Chrome for Testing, and a free 127.0.0.1:49201"]
    #[allow(clippy::too_many_lines)]
    async fn real_extension_runs_browser_tools_in_its_tab_group() {
        use crate::tools::browser::cdp::{Cdp, NetRule};
        use crate::tools::chrome::{PORT, pairing, server::Auth};
        let dist =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../frontend/extension/dist/chrome");
        assert!(
            dist.join("manifest.json").is_file(),
            "build the extension first: cd frontend && npm run build:extension"
        );
        let state = tempfile::tempdir().unwrap();
        let auth = Auth {
            dir: state.path().to_path_buf(),
            store: std::sync::Arc::new(kor_secrets::MemoryStore::default()),
        };
        let bridge = Bridge::bind(PORT, auth)
            .await
            .expect("127.0.0.1:49201 must be free (close Kor)");
        let (code, _) = pairing::new_code(state.path()).unwrap();
        let cdp = Cdp::launch_with(false, &NetRule::Open, Some(&dist))
            .await
            .unwrap();

        // The extension's service worker is up: open its side panel page and pair from there.
        let mut id = String::new();
        for _ in 0..50 {
            let t = cdp
                .call(None, "Target.getTargets", json!({}))
                .await
                .unwrap();
            if let Some(u) = t["targetInfos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x["url"].as_str())
                .find(|u| u.ends_with("/background.js"))
            {
                id = u
                    .trim_start_matches("chrome-extension://")
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(
            id,
            kor_platform::native_messaging::CHROME_EXTENSION_ID,
            "the manifest key fixes the id"
        );
        let panel = cdp
            .call(
                None,
                "Target.createTarget",
                json!({"url": format!("chrome-extension://{id}/sidepanel.html")}),
            )
            .await
            .unwrap();
        let session = cdp
            .call(
                None,
                "Target.attachToTarget",
                json!({"targetId": panel["targetId"], "flatten": true}),
            )
            .await
            .unwrap()["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        tokio::time::sleep(Duration::from_millis(800)).await;
        let js = format!(
            "chrome.runtime.connect({{name: 'kor-panel'}}).postMessage({{op: 'pair', code: '{code}', rid: 1}})"
        );
        cdp.call(
            Some(&session),
            "Runtime.evaluate",
            json!({"expression": js}),
        )
        .await
        .unwrap();
        for _ in 0..50 {
            if bridge.connected() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(
            bridge.connected(),
            "the extension paired over the WebSocket fallback"
        );

        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let group = Group::default();
        let url = format!("http://127.0.0.1:{}/", serve().await);
        let out = run(
            Op::Open,
            Args {
                url: Some(url.clone()),
                ..Args::default()
            },
            &ctx,
            &group,
            &bridge,
        )
        .await
        .unwrap();
        assert!(out.content.contains("Kor e2e"), "{}", out.content);
        let find = |q: &str| {
            let q = q.to_owned();
            let (ctx, group, bridge) = (&ctx, &group, &bridge);
            async move {
                run(
                    Op::Find,
                    Args {
                        query: Some(q),
                        ..Args::default()
                    },
                    ctx,
                    group,
                    bridge,
                )
                .await
                .unwrap()
                .content
            }
        };
        let email = find("email field").await;
        let r_email = email
            .lines()
            .nth(1)
            .and_then(|l| l.split_whitespace().next())
            .unwrap()
            .to_owned();
        let filled = run(
            Op::Fill,
            Args {
                reference: Some(r_email),
                text: Some("ada@example.com".into()),
                ..Args::default()
            },
            &ctx,
            &group,
            &bridge,
        )
        .await;
        assert!(filled.is_ok(), "{filled:?}");
        let pw = find("parola password").await;
        let r_pw = pw
            .lines()
            .nth(1)
            .and_then(|l| l.split_whitespace().next())
            .unwrap()
            .to_owned();
        let refused = run(
            Op::Fill,
            Args {
                reference: Some(r_pw),
                text: Some("x".into()),
                ..Args::default()
            },
            &ctx,
            &group,
            &bridge,
        )
        .await;
        assert!(refused.unwrap_err().contains("Güvenlik"));
        let button = find("gönder button").await;
        let r_btn = button
            .lines()
            .nth(1)
            .and_then(|l| l.split_whitespace().next())
            .unwrap()
            .to_owned();
        run(
            Op::Click,
            Args {
                reference: Some(r_btn),
                ..Args::default()
            },
            &ctx,
            &group,
            &bridge,
        )
        .await
        .unwrap();
        let console = run(Op::Console, Args::default(), &ctx, &group, &bridge)
            .await
            .unwrap();
        assert!(
            console.content.contains("kor-e2e-click"),
            "{}",
            console.content
        );
        let network = run(Op::Network, Args::default(), &ctx, &group, &bridge)
            .await
            .unwrap();
        assert!(network.content.contains(&url), "{}", network.content);
        let tabs = run(
            Op::Tabs,
            Args {
                action: Some("list".into()),
                ..Args::default()
            },
            &ctx,
            &group,
            &bridge,
        )
        .await
        .unwrap();
        assert!(tabs.content.contains("Kor e2e"));
        let shot = run(Op::Screenshot, Args::default(), &ctx, &group, &bridge)
            .await
            .unwrap();
        assert!(shot.image.is_some());
        // Tabs outside the task's group are refused.
        let foreign = bridge
            .request(
                json!({"cmd": "snapshot", "task": {"key": "other", "title": "x"}, "tabId": 1}),
                Duration::from_secs(10),
            )
            .await;
        assert!(foreign.unwrap_err().contains("grubunda değil"));
        drop(cdp);
    }
}
