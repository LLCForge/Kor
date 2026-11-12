//! Yerel ad alanları: `git`, `terminal`, `logs`, `routines`, `notes`, `tags`, `connections`, `g10`.
//! Sözleşmeler: `frontend/src/desktop/ipc-{git,terminal,logs,routines,notes,tags,connections,g10}.ts`.

use kor_platform::NoWindow as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kor_core::{ContentPart, Role};
use kor_secrets::SecretStore;
use regex::Regex;
use serde_json::{Map, Value, json};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncRead, AsyncReadExt};
use toml_edit::{DocumentMut, Item, Table};

use crate::api::{Res, b, opt_s, root, s};
use crate::state::AppState;

/// Komut çıktısı üst sınırı (stdout ve stderr ayrı ayrı).
const OUTPUT_CAP: usize = 200_000;
const GIT_ENV: &[(&str, &str)] = &[("GIT_TERMINAL_PROMPT", "0"), ("GIT_EDITOR", "true")];

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    let m = format!("{ns}.{name}");
    match ns {
        "git" => git(name, args, app, state, &m).await,
        "terminal" if name == "run" => Some(terminal_run(args, state, &m).await),
        "logs" if name == "list" => Some(logs_list(args)),
        "routines" => routines(name, args, &m),
        "notes" => notes(name, args, &m),
        "tags" => tags(name, args),
        "connections" => connections(name, args, state, &m).await,
        "g10" => g10(name, args, app, state, &m).await,
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Ortak yardımcılar
// ---------------------------------------------------------------------------------------------

struct Out {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Out {
    fn text(&self) -> String {
        [self.stdout.trim_end(), self.stderr.trim_end()]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Akışı sonuna dek okur (süreç yazarken tıkanmasın), ilk `OUTPUT_CAP` baytı tutar.
async fn drain<R: AsyncRead + Unpin>(mut r: R) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut cut = false;
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = OUTPUT_CAP.saturating_sub(buf.len());
                cut |= n > room;
                buf.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    }
    (buf, cut)
}

fn lossy((bytes, cut): (Vec<u8>, bool)) -> String {
    let mut t = String::from_utf8_lossy(&bytes).into_owned();
    if cut {
        t.push_str("\n[çıktı kısaltıldı]");
    }
    t
}

/// Programı argv ile (kabuk yok) zaman aşımıyla çalıştırır; süre dolunca süreç öldürülür.
async fn exec(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[&str],
    cwd: &Path,
    secs: u64,
    env: &[(&str, &str)],
) -> Result<Out, String> {
    let program = program.as_ref();
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.no_window();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{} başlatılamadı: {e}", program.to_string_lossy()))?;
    let so = child.stdout.take().ok_or("Çıktı alınamadı")?;
    let se = child.stderr.take().ok_or("Çıktı alınamadı")?;
    let work = async {
        let (o, e) = tokio::join!(drain(so), drain(se));
        (o, e, child.wait().await)
    };
    match tokio::time::timeout(Duration::from_secs(secs), work).await {
        Ok((o, e, status)) => {
            let status = status.map_err(|e| e.to_string())?;
            Ok(Out {
                ok: status.success(),
                code: status.code(),
                stdout: lossy(o),
                stderr: lossy(e),
            })
        }
        Err(_) => Err(format!("Komut {secs} saniyede bitmedi ve durduruldu")),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn valid_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn need_id<'a>(args: &'a [Value], i: usize, m: &str) -> Result<&'a str, String> {
    let id = s(args, i, m)?;
    if valid_id(id) {
        Ok(id)
    } else {
        Err("Geçersiz kimlik".into())
    }
}

fn kor_home() -> Result<PathBuf, String> {
    kor_runtime::kor_home().ok_or_else(|| "Kor veri klasörü bulunamadı".into())
}

/// Geçici dosyaya yazıp yeniden adlandırır: yarım yazılmış dosya kalmaz.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("kor-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn read_json_or<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn obj<'a>(args: &'a [Value], i: usize, m: &str) -> Result<&'a Map<String, Value>, String> {
    args.get(i)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("`{m}`: {}. bağımsız değişken nesne olmalı", i + 1))
}

fn field<'a>(o: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    o.get(k).and_then(Value::as_str).filter(|v| !v.is_empty())
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut t: String = text.chars().take(max).collect();
        t.push('…');
        t
    }
}

// ---------------------------------------------------------------------------------------------
// git
// ---------------------------------------------------------------------------------------------

async fn git_out(dir: &Path, args: &[&str], secs: u64) -> Result<Out, String> {
    exec("git", args, dir, secs, GIT_ENV).await
}

/// Başarılıysa kırpılmış stdout; boşsa ya da hata varsa `None`.
async fn git_line(dir: &Path, args: &[&str]) -> Option<String> {
    let o = git_out(dir, args, 30).await.ok()?;
    let t = o.stdout.trim();
    (o.ok && !t.is_empty()).then(|| t.to_owned())
}

/// Kullanıcı eylemi: çıktıyı metin döner, başarısızsa çıktıyı hata yapar.
async fn git_action(dir: &Path, args: &[&str]) -> Res {
    let o = git_out(dir, args, 300).await?;
    let text = o.text();
    if o.ok {
        Ok(json!(text))
    } else if text.is_empty() {
        Err("git işlemi başarısız oldu".into())
    } else {
        Err(text)
    }
}

/// Dal adı: seçenek gibi okunamaz, yalnızca güvenli karakterler.
fn valid_ref(r: &str) -> bool {
    (1..=255).contains(&r.len())
        && !r.starts_with('-')
        && !r.contains("..")
        && r.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b'-'))
}

/// `https://user:token@host/x` → `https://host/x`: arayüze kimlik bilgisi dönmez.
fn redact_userinfo(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://")
        && let Some((auth, host)) = rest.split_once('@')
        && !auth.contains('/')
        && (scheme == "https" || scheme == "http")
    {
        return format!("{scheme}://{host}");
    }
    url.to_owned()
}

/// GitHub uzak adresinden web adresi (`https://github.com/owner/repo`).
fn github_web(remote: &str) -> Option<String> {
    let r = remote.trim();
    let path = r
        .strip_prefix("git@github.com:")
        .or_else(|| r.strip_prefix("ssh://git@github.com/"))
        .or_else(|| r.strip_prefix("https://github.com/"))
        .or_else(|| r.strip_prefix("http://github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let ok = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    };
    (parts.next().is_none() && ok(owner) && ok(repo))
        .then(|| format!("https://github.com/{owner}/{repo}"))
}

/// `git rev-list --left-right --count @{u}...HEAD` → (geri, ileri).
fn parse_counts(text: &str) -> Option<(u64, u64)> {
    let mut it = text.split_whitespace().map(str::parse::<u64>);
    match (it.next(), it.next()) {
        (Some(Ok(behind)), Some(Ok(ahead))) => Some((behind, ahead)),
        _ => None,
    }
}

async fn gh_present(dir: &Path) -> bool {
    exec("gh", &["--version"], dir, 10, &[])
        .await
        .is_ok_and(|o| o.ok)
}

async fn git_path_exists(dir: &Path, name: &str) -> bool {
    match git_line(dir, &["rev-parse", "--git-path", name]).await {
        Some(p) => {
            let p = PathBuf::from(p);
            if p.is_absolute() { p } else { dir.join(p) }.exists()
        }
        None => false,
    }
}

async fn operation(dir: &Path) -> Option<&'static str> {
    if git_path_exists(dir, "rebase-merge").await || git_path_exists(dir, "rebase-apply").await {
        Some("rebase")
    } else if git_path_exists(dir, "MERGE_HEAD").await {
        Some("merge")
    } else {
        None
    }
}

async fn default_base(dir: &Path) -> Option<String> {
    if let Some(r) = git_line(
        dir,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .await
    {
        return Some(r.strip_prefix("origin/").unwrap_or(&r).to_owned());
    }
    for b in ["main", "master"] {
        let r = format!("refs/heads/{b}");
        if git_line(dir, &["rev-parse", "--verify", "--quiet", &r])
            .await
            .is_some()
        {
            return Some(b.to_owned());
        }
    }
    None
}

async fn current_branch(dir: &Path) -> Option<String> {
    git_line(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await
}

async fn git_status(dir: &Path) -> Value {
    let folder = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let Some(top) = git_line(dir, &["rev-parse", "--show-toplevel"]).await else {
        return json!({
            "isRepo": false, "repoName": folder(dir), "branch": null, "ahead": null, "behind": null,
            "dirty": 0, "remoteUrl": null, "webUrl": null, "pr": null, "hasGh": false,
            "operation": null, "conflicts": [], "defaultBase": null,
        });
    };
    let branch = current_branch(dir).await;
    let counts = git_line(
        dir,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    )
    .await
    .and_then(|t| parse_counts(&t));
    let dirty = git_out(dir, &["status", "--porcelain"], 30)
        .await
        .map_or(0, |o| {
            o.stdout.lines().filter(|l| !l.trim().is_empty()).count()
        });
    let remote = git_line(dir, &["remote", "get-url", "origin"])
        .await
        .map(|r| redact_userinfo(&r));
    let web = remote.as_deref().and_then(github_web);
    let has_gh = gh_present(dir).await;
    let mut pr = Value::Null;
    if has_gh && branch.is_some() && web.is_some() {
        // Ağ isteği yalnızca kullanıcı Git panelini açtığında ve gh kuruluyken yapılır (gh'nin kendi oturumu).
        if let Ok(o) = exec("gh", &["pr", "view", "--json", "number,url"], dir, 15, &[]).await
            && o.ok
            && let Ok(v) = serde_json::from_str::<Value>(&o.stdout)
            && let (Some(n), Some(u)) = (v["number"].as_u64(), v["url"].as_str())
        {
            pr = json!({ "number": n, "url": u });
        }
    }
    let conflicts: Vec<String> = git_out(dir, &["diff", "--name-only", "--diff-filter=U"], 30)
        .await
        .map(|o| {
            o.stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    json!({
        "isRepo": true,
        "repoName": folder(Path::new(&top)),
        "branch": branch,
        "ahead": counts.map(|c| c.1),
        "behind": counts.map(|c| c.0),
        "dirty": dirty,
        "remoteUrl": remote,
        "webUrl": web,
        "pr": pr,
        "hasGh": has_gh,
        "operation": operation(dir).await,
        "conflicts": conflicts,
        "defaultBase": default_base(dir).await,
    })
}

fn open_url(app: &AppHandle, url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Geçersiz adres".to_owned())?;
    if parsed.scheme() != "https" {
        return Err("Yalnızca https adresleri açılır".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

async fn git(
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
    m: &str,
) -> Option<Res> {
    let dir = match root(state) {
        Ok(d) => d,
        Err(e) => return Some(Err(e)),
    };
    let dir = dir.as_path();
    Some(match name {
        "status" => Ok(git_status(dir).await),
        "branches" => {
            let local: Vec<String> = git_out(
                dir,
                &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
                30,
            )
            .await
            .map(|o| {
                o.stdout
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
            Ok(json!({
                "current": current_branch(dir).await,
                "defaultBase": default_base(dir).await,
                "local": local,
            }))
        }
        "push" => match current_branch(dir).await {
            None => Err("Geçerli dal yok (ayrık HEAD)".into()),
            Some(branch) => {
                let upstream = git_line(
                    dir,
                    &[
                        "rev-parse",
                        "--abbrev-ref",
                        "--symbolic-full-name",
                        "@{upstream}",
                    ],
                )
                .await;
                if upstream.is_some() {
                    git_action(dir, &["push"]).await
                } else {
                    git_action(dir, &["push", "-u", "origin", &branch]).await
                }
            }
        },
        "createPr" => create_pr(args, app, dir, m).await,
        "openPr" => s(args, 0, m)
            .and_then(|u| open_url(app, u))
            .map(|()| Value::Null),
        "merge" | "rebase" => match s(args, 0, m) {
            Ok(base) if valid_ref(base) => {
                if name == "merge" {
                    git_action(dir, &["merge", "--no-edit", base]).await
                } else {
                    git_action(dir, &["rebase", base]).await
                }
            }
            Ok(_) => Err("Geçersiz dal adı".into()),
            Err(e) => Err(e),
        },
        "abortOperation" => match operation(dir).await {
            Some(op) => git_action(dir, &[op, "--abort"]).await,
            None => Err("Süren bir rebase ya da merge yok".into()),
        },
        "continueOperation" => match operation(dir).await {
            Some("rebase") => git_action(dir, &["rebase", "--continue"]).await,
            Some(_) => git_action(dir, &["commit", "--no-edit"]).await,
            None => Err("Süren bir rebase ya da merge yok".into()),
        },
        _ => return None,
    })
}

async fn create_pr(args: &[Value], app: &AppHandle, dir: &Path, m: &str) -> Res {
    let o = obj(args, 0, m)?;
    let title = o.get("title").and_then(Value::as_str).unwrap_or("").trim();
    let body = o.get("body").and_then(Value::as_str).unwrap_or("");
    let base = o.get("base").and_then(Value::as_str).unwrap_or("").trim();
    let draft = o.get("draft").and_then(Value::as_bool).unwrap_or(false);
    if title.is_empty() || title.chars().count() > 256 {
        return Err("Başlık 1-256 karakter olmalı".into());
    }
    if body.len() > 65_536 {
        return Err("Açıklama çok uzun".into());
    }
    if !valid_ref(base) {
        return Err("Geçersiz dal adı".into());
    }
    let branch = current_branch(dir)
        .await
        .ok_or("Geçerli dal yok (ayrık HEAD)")?;
    if gh_present(dir).await {
        let mut a = vec![
            "pr", "create", "--title", title, "--body", body, "--base", base, "--head", &branch,
        ];
        if draft {
            a.push("--draft");
        }
        let out = exec("gh", &a, dir, 120, &[]).await?;
        if !out.ok {
            let t = out.text();
            return Err(if t.is_empty() {
                "PR oluşturulamadı".into()
            } else {
                t
            });
        }
        let url = out
            .stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| l.starts_with("https://"))
            .unwrap_or_default()
            .to_owned();
        return Ok(json!({ "url": url, "created": true }));
    }
    let remote = git_line(dir, &["remote", "get-url", "origin"])
        .await
        .ok_or("origin uzak deposu yok")?;
    let web = github_web(&remote).ok_or("PR yalnızca GitHub depolarında açılabilir")?;
    let url = reqwest::Url::parse_with_params(
        &format!("{web}/compare/{base}...{branch}"),
        &[("expand", "1"), ("title", title), ("body", body)],
    )
    .map_err(|e| e.to_string())?;
    open_url(app, url.as_str())?;
    Ok(json!({ "url": url.as_str(), "created": false }))
}

// ---------------------------------------------------------------------------------------------
// terminal
// ---------------------------------------------------------------------------------------------

/// Terminalin geçerli klasörü (uygulama boyunca); kök değişip dışarıda kalırsa köke döner.
static TERM_CWD: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `p`yi `cur`a göre çözer; sonuç kökün içinde bir klasör olmalı.
fn resolve_dir(base: &Path, cur: &Path, p: &str) -> Result<PathBuf, String> {
    let p = p.trim().trim_matches(|c| c == '"' || c == '\'');
    let cand = if p.is_empty() {
        base.to_path_buf()
    } else if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        cur.join(p)
    };
    let full = cand
        .canonicalize()
        .map_err(|_| format!("Klasör bulunamadı: {p}"))?;
    if !full.starts_with(base) || !full.is_dir() {
        return Err("Klasör çalışma klasörünün dışında".into());
    }
    Ok(full)
}

/// Yalnız `cd <yol>` / `Set-Location <yol>` komutu: geçerli klasörü değiştirir (zincirli komut değil).
fn cd_target(cmd: &str) -> Option<&str> {
    let c = cmd.trim();
    if c.contains([';', '&', '|', '\n']) {
        return None;
    }
    let lower = c.to_ascii_lowercase();
    for kw in ["cd", "set-location", "chdir"] {
        if lower == kw {
            return Some("");
        }
        if lower.starts_with(kw) && c[kw.len()..].starts_with([' ', '\t']) {
            return Some(c[kw.len()..].trim());
        }
    }
    None
}

async fn terminal_run(args: &[Value], state: &AppState, m: &str) -> Res {
    let command = s(args, 0, m)?;
    if command.trim().is_empty() || command.len() > 10_000 {
        return Err("Komut boş ya da çok uzun".into());
    }
    let base = root(state)?
        .canonicalize()
        .map_err(|e| format!("Çalışma klasörü açılamadı: {e}"))?;
    let stored = lock(&TERM_CWD)
        .clone()
        .filter(|p| p.starts_with(&base) && p.is_dir());
    let mut cwd = match opt_s(args, 1).filter(|c| !c.is_empty()) {
        Some(c) => resolve_dir(&base, &base, c)?,
        None => stored.unwrap_or_else(|| base.clone()),
    };
    let shown = |p: &Path| {
        kor_tools::without_verbatim(p)
            .to_string_lossy()
            .into_owned()
    };
    if let Some(target) = cd_target(command) {
        cwd = resolve_dir(&base, &cwd, target)?;
        *lock(&TERM_CWD) = Some(cwd.clone());
        return Ok(json!({ "output": "", "code": 0, "cwd": shown(&cwd) }));
    }
    *lock(&TERM_CWD) = Some(cwd.clone());
    let run_dir = kor_tools::without_verbatim(&cwd);
    // Kullanıcının yazdığı komut kabuğa tek argv olarak gider; arayüz bunu kullanıcı eylemi olarak çalıştırır.
    let out = if cfg!(windows) {
        let script = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {command}");
        exec(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ],
            &run_dir,
            120,
            &[],
        )
        .await?
    } else {
        exec("sh", &["-c", command], &run_dir, 120, &[]).await?
    };
    Ok(json!({ "output": out.text(), "code": out.code, "cwd": shown(&cwd) }))
}

// ---------------------------------------------------------------------------------------------
// logs: Kor ayrı bir araç günlüğü tutmaz; oturum dökümlerindeki araç çağrıları/sonuçları okunur.
// ---------------------------------------------------------------------------------------------

const MAX_LOG_FILES: usize = 50;

fn decision_of(result: Option<(&str, bool)>) -> &'static str {
    match result {
        Some((c, true)) if c.starts_with("permission denied") || c.starts_with("blocked by") => {
            "deny"
        }
        Some((c, true)) if c.starts_with("unknown tool") || c.starts_with("invalid input") => {
            "invalid"
        }
        _ => "allow",
    }
}

fn log_entries(messages: &[kor_core::Message], ts: &str) -> Vec<Value> {
    let results: HashMap<&str, (&str, bool)> = messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match p {
            ContentPart::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some((tool_use_id.as_str(), (content.as_str(), *is_error))),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for m in messages.iter().filter(|m| m.role == Role::Assistant) {
        for p in &m.parts {
            if let ContentPart::ToolUse { id, name, input } = p {
                let r = results.get(id.as_str()).copied();
                let decision = decision_of(r);
                let mut e = json!({
                    "ts": ts,
                    "tool": name,
                    "decision": decision,
                    "args": clip(&input.to_string(), 300),
                });
                if let Some((content, is_error)) = r {
                    e["ok"] = json!(!is_error);
                    if decision != "allow" {
                        e["reason"] = json!(clip(content, 200));
                    }
                }
                out.push(e);
            }
        }
    }
    out
}

fn logs_list(args: &[Value]) -> Res {
    let limit = args
        .get(1)
        .and_then(Value::as_u64)
        .map_or(500, |n| usize::try_from(n).unwrap_or(2000).clamp(1, 2000));
    let Some(dir) = crate::store::sessions_dir() else {
        return Ok(json!([]));
    };
    // En yeni dosya önce okunur; sonuçta en yeni kayıt sonda olur.
    let files = match opt_s(args, 0) {
        Some(id) => vec![crate::store::session_path(&dir, id)?],
        None => kor_agent::session::list(&dir)
            .into_iter()
            .take(MAX_LOG_FILES)
            .collect(),
    };
    let mut groups: Vec<Vec<Value>> = Vec::new();
    let mut total = 0;
    for f in files {
        if total >= limit {
            break;
        }
        let Ok(messages) = kor_agent::session::load(&f) else {
            continue;
        };
        // Mesajların kendi zamanı yok: dosyanın son yazılma zamanı kullanılır.
        let ts = std::fs::metadata(&f)
            .and_then(|m| m.modified())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
            .unwrap_or_default();
        let entries = log_entries(&messages, &ts);
        total += entries.len();
        groups.push(entries);
    }
    let all: Vec<Value> = groups.into_iter().rev().flatten().collect();
    let skip = all.len().saturating_sub(limit);
    Ok(Value::Array(all.into_iter().skip(skip).collect()))
}

// ---------------------------------------------------------------------------------------------
// routines (zamanlanmış görevler): depo + uygulama açıkken dakikada bir tetikleme.
// ---------------------------------------------------------------------------------------------

/// Vadesi gelen etkin görevleri yeni bir sohbet olarak başlatır. Onay isteyen araçlar arayüzde
/// her zamanki gibi onay ister; görev kendiliğinden hiçbir şeyi onaylamaz.
pub fn start_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            for t in take_due(now_ms()) {
                fire(&app, &t);
            }
            // §27.15 haftalık bilgi özeti (ADR 0063); `kor daemon` çalışıyorsa onu o yazar.
            if !kor_runtime::triggers::daemon::is_running() {
                let _ =
                    tokio::task::spawn_blocking(kor_runtime::routines::knowledge_digest_tick).await;
                let _ =
                    tokio::task::spawn_blocking(kor_runtime::routines::knowledge_folder_tick).await;
            }
        }
    });
}

/// Vadesi gelenler (çekirdek depo: `kor_runtime::routines`, `kor routines` ile aynı dosya).
/// `kor daemon` çalışırken klasörlü görevleri o çalıştırır (ADR 0039); burada yalnızca klasörsüzler.
fn take_due(now: u64) -> Vec<Value> {
    let daemon = kor_runtime::triggers::daemon::is_running();
    kor_runtime::routines::file()
        .map(|p| {
            kor_runtime::routines::take_due_where(&p, now, |t| {
                !daemon || t["root"].as_str().is_none_or(str::is_empty)
            })
        })
        .unwrap_or_default()
}

fn fire(app: &AppHandle, t: &Value) {
    let name = t["name"].as_str().unwrap_or_default();
    let Some(prompt) = t["prompt"].as_str().filter(|p| !p.trim().is_empty()) else {
        return;
    };
    let id = crate::meta::new_uuid();
    let _ = crate::meta::update(|m| {
        m.sessions.entry(id.clone()).or_default().custom_title =
            Some(format!("Zamanlanmış: {name}").chars().take(120).collect());
        Ok(())
    });
    let args = crate::chat::SendArgs {
        session_id: id,
        text: prompt.to_owned(),
        attachments: Vec::new(),
        open_file: None,
    };
    let root = t["root"]
        .as_str()
        .filter(|r| !r.is_empty())
        .and_then(|r| PathBuf::from(r).canonicalize().ok())
        .filter(|p| p.is_dir());
    let started = match root {
        Some(r) => crate::chat::start_in(app, r, args),
        None => crate::chat::send(app, args),
    };
    if let Err(e) = started {
        eprintln!("kor: zamanlanmış görev başlatılamadı ({name}): {e}");
    }
}

fn routines(name: &str, args: &[Value], m: &str) -> Option<Res> {
    use kor_runtime::routines as r;
    let run = || -> Res {
        let path = r::file().ok_or("Kor veri klasörü bulunamadı")?;
        match name {
            "list" => Ok(json!(r::list(&path))),
            "add" => r::add(&path, obj(args, 0, m)?, now_ms()),
            "remove" => r::remove(&path, need_id(args, 0, m)?).map(|ok| json!(ok)),
            "setEnabled" => r::set_enabled(&path, need_id(args, 0, m)?, b(args, 1), now_ms())
                .map(|ok| json!(ok)),
            _ => Err(String::new()),
        }
    };
    matches!(name, "list" | "add" | "remove" | "setEnabled").then(run)
}

// ---------------------------------------------------------------------------------------------
// notes / tags
// ---------------------------------------------------------------------------------------------

const MAX_NOTE: usize = 100_000;

fn notes(name: &str, args: &[Value], m: &str) -> Option<Res> {
    let file = |id: &str| -> Result<PathBuf, String> {
        Ok(kor_home()?.join("notes").join(format!("{id}.md")))
    };
    Some(match name {
        "get" => need_id(args, 0, m)
            .and_then(file)
            .map(|p| json!(std::fs::read_to_string(p).unwrap_or_default())),
        "set" => (|| {
            let p = file(need_id(args, 0, m)?)?;
            let text = s(args, 1, m)?;
            if text.len() > MAX_NOTE {
                return Err("Not çok uzun (en fazla 100000 karakter)".into());
            }
            if text.is_empty() {
                let _ = std::fs::remove_file(&p);
            } else {
                write_atomic(&p, text)?;
            }
            Ok(Value::Null)
        })(),
        _ => return None,
    })
}

static TAG_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}][\p{L}\p{N}_-]{0,63}$").expect("sabit desen"));

fn check_tags(list: &[Value]) -> Result<Vec<Value>, String> {
    if list.len() > 200 {
        return Err("En fazla 200 etiket".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in list {
        let name = t["name"].as_str().unwrap_or("");
        let content = t["content"].as_str().unwrap_or("");
        if !TAG_NAME.is_match(name) {
            return Err(format!("Geçersiz etiket adı: {}", clip(name, 64)));
        }
        if content.len() > 20_000 {
            return Err(format!("Etiket içeriği çok uzun: {name}"));
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("Aynı etiket adı iki kez: {name}"));
        }
        out.push(json!({ "name": name, "content": content }));
    }
    Ok(out)
}

fn tags(name: &str, args: &[Value]) -> Option<Res> {
    let path = || kor_home().map(|h| h.join("tags.json"));
    Some(match name {
        "list" => path().map(|p| json!(read_json_or::<Vec<Value>>(&p))),
        "save" => (|| {
            let list = args
                .first()
                .and_then(Value::as_array)
                .ok_or("Etiket listesi bekleniyor")?;
            let clean = check_tags(list)?;
            write_atomic(
                &path()?,
                &serde_json::to_string_pretty(&clean).map_err(|e| e.to_string())?,
            )?;
            Ok(json!(clean))
        })(),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// connections: kullanıcının `connections.toml` dosyası (Kor ajanının okuduğu dosyanın aynısı)
// ---------------------------------------------------------------------------------------------

static CONN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static FINGERPRINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"SHA256:[A-Za-z0-9+/]{20,64}").expect("sabit desen"));

fn conn_file() -> Result<PathBuf, String> {
    kor_config::user_dir()
        .map(|d| d.join("connections.toml"))
        .ok_or_else(|| "Kullanıcı yapılandırma klasörü bulunamadı".into())
}

fn read_text(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn load_conns(path: &Path) -> Result<kor_agent::remote::Connections, String> {
    kor_agent::remote::Connections::parse(&read_text(path)?)
}

/// Ajanın kuralıyla aynı: küçük harf, rakam ve `-`, en fazla 40.
fn valid_conn_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn db_account(id: &str) -> String {
    format!("db-{id}")
}

fn db_has_secret(id: &str) -> bool {
    let env = format!(
        "KOR_DB_{}_PASSWORD",
        id.to_ascii_uppercase().replace('-', "_")
    );
    kor_secrets::resolve_key(
        &db_account(id),
        &env,
        |k| std::env::var(k).ok(),
        &kor_secrets::KeychainStore,
    )
    .ok()
    .flatten()
    .is_some()
}

fn openssh(name: &str) -> std::ffi::OsString {
    if cfg!(windows)
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        let p = PathBuf::from(root).join(format!(r"System32\OpenSSH\{name}.exe"));
        if p.is_file() {
            return p.into_os_string();
        }
    }
    name.into()
}

fn host_pattern(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

fn tmp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// `known_hosts` içinde bu sunucu için kayıtlı anahtarın parmak izi.
async fn known_fingerprint(host: &str, port: u16) -> Option<String> {
    let pat = host_pattern(host, port);
    let o = exec(
        openssh("ssh-keygen"),
        &["-l", "-F", &pat],
        &tmp_dir(),
        10,
        &[],
    )
    .await
    .ok()?;
    FINGERPRINT.find(&o.stdout).map(|m| m.as_str().to_owned())
}

/// Sunucunun şu anki anahtarları: `(known_hosts satırı, parmak izi)`, tercih sırasıyla (ed25519, ecdsa, rsa).
async fn scan_keys(host: &str, port: u16) -> Result<Vec<(String, String)>, String> {
    let port_s = port.to_string();
    let o = exec(
        openssh("ssh-keyscan"),
        &["-T", "10", "-p", &port_s, "-t", "ed25519,ecdsa,rsa", host],
        &tmp_dir(),
        30,
        &[],
    )
    .await?;
    let lines: Vec<String> = o
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect();
    if lines.is_empty() {
        return Err("Sunucu anahtarı okunamadı (adres, port ve ağ erişimini kontrol et)".into());
    }
    let tmp = tmp_dir().join(format!("kor-hostkeys-{}-{}", std::process::id(), now_ms()));
    std::fs::write(&tmp, format!("{}\n", lines.join("\n"))).map_err(|e| e.to_string())?;
    let fp = exec(
        openssh("ssh-keygen"),
        &["-l", "-f", &tmp.to_string_lossy()],
        &tmp_dir(),
        10,
        &[],
    )
    .await;
    let _ = std::fs::remove_file(&tmp);
    let fps: Vec<String> = fp?
        .stdout
        .lines()
        .filter_map(|l| FINGERPRINT.find(l).map(|m| m.as_str().to_owned()))
        .collect();
    let rank = |line: &str| {
        let t = line.split_whitespace().nth(1).unwrap_or("");
        if t.contains("ed25519") {
            0
        } else if t.starts_with("ecdsa") {
            1
        } else {
            2
        }
    };
    let mut pairs: Vec<(String, String)> = lines.into_iter().zip(fps).collect();
    pairs.sort_by_key(|(l, _)| rank(l));
    Ok(pairs)
}

fn put(o: &mut Map<String, Value>, k: &str, v: Option<Value>) {
    if let Some(v) = v {
        o.insert(k.into(), v);
    }
}

async fn conn_view(path: &Path) -> Res {
    let c = load_conns(path)?;
    let mut ssh = Vec::new();
    for (id, h) in &c.ssh {
        let port = h.port.unwrap_or(22);
        let mut o = Map::new();
        o.insert("id".into(), json!(id));
        // shortcut: connections.toml ek alan kabul etmez; görünen ad kimliktir. Ad gerekirse ayrı dosyada tutulur.
        o.insert("name".into(), json!(id));
        o.insert("host".into(), json!(h.host));
        o.insert("port".into(), json!(port));
        o.insert("username".into(), json!(h.user.clone().unwrap_or_default()));
        o.insert(
            "auth".into(),
            json!(if h.identity.is_some() { "key" } else { "agent" }),
        );
        put(&mut o, "keyPath", h.identity.clone().map(Value::from));
        put(
            &mut o,
            "hostKey",
            known_fingerprint(&h.host, port).await.map(Value::from),
        );
        o.insert("hasSecret".into(), json!(false));
        ssh.push(Value::Object(o));
    }
    let databases: Vec<Value> =
        c.db.iter()
            .map(|(id, d)| {
                let mut o = Map::new();
                o.insert("id".into(), json!(id));
                o.insert("name".into(), json!(id));
                let kind = match d.kind {
                    kor_agent::remote::DbKind::Postgres => "postgres",
                    kor_agent::remote::DbKind::Mysql => "mysql",
                    kor_agent::remote::DbKind::Sqlite => "sqlite",
                    kor_agent::remote::DbKind::Mssql => "mssql",
                };
                o.insert("kind".into(), json!(kind));
                put(&mut o, "host", d.host.clone().map(Value::from));
                put(&mut o, "port", d.port.map(Value::from));
                put(&mut o, "database", d.database.clone().map(Value::from));
                put(&mut o, "user", d.user.clone().map(Value::from));
                put(&mut o, "file", d.file.clone().map(Value::from));
                if d.sslmode.as_deref().is_some_and(is_tls_mode) {
                    o.insert("ssl".into(), json!(true));
                }
                put(&mut o, "sshHost", d.ssh.clone().map(Value::from));
                o.insert("hasSecret".into(), json!(db_has_secret(id)));
                Value::Object(o)
            })
            .collect();
    Ok(json!({ "ssh": ssh, "databases": databases, "canStoreSecrets": true }))
}

fn is_tls_mode(m: &str) -> bool {
    matches!(m, "require" | "verify-ca" | "verify-full")
}

fn parse_doc(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|e| format!("connections.toml okunamadı: {e}"))
}

/// `[kind.id]` tablosu (yoksa açılır); `kind` örtük tablo kalır, yorumlar korunur.
fn section<'a>(doc: &'a mut DocumentMut, kind: &str, id: &str) -> Result<&'a mut Table, String> {
    let top = doc.as_table_mut();
    if !top.contains_key(kind) {
        let mut t = Table::new();
        t.set_implicit(true);
        top.insert(kind, Item::Table(t));
    }
    let parent = top
        .get_mut(kind)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("connections.toml: `{kind}` tablo değil"))?;
    if !parent.contains_key(id) {
        parent.insert(id, Item::Table(Table::new()));
    }
    parent
        .get_mut(id)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("connections.toml: `{kind}.{id}` tablo değil"))
}

fn set_opt(t: &mut Table, k: &str, v: Option<Item>) {
    match v {
        Some(i) => {
            t.insert(k, i);
        }
        None => {
            t.remove(k);
        }
    }
}

fn remove_entry(doc: &mut DocumentMut, kind: &str, id: &str) -> bool {
    doc.get_mut(kind)
        .and_then(Item::as_table_mut)
        .is_some_and(|t| t.remove(id).is_some())
}

fn has_entry(doc: &DocumentMut, kind: &str, id: &str) -> bool {
    doc.get(kind)
        .and_then(Item::as_table)
        .is_some_and(|t| t.contains_key(id))
}

/// Değiştirilmiş belgeyi ajanın ayrıştırıcısıyla doğrular, sonra atomik yazar.
fn commit_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    let text = doc.to_string();
    kor_agent::remote::Connections::parse(&text)
        .map_err(|e| format!("Bağlantı kaydedilemedi: {e}"))?;
    write_atomic(path, &text)
}

fn port_of(o: &Map<String, Value>) -> Result<Option<u16>, String> {
    match o.get("port") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p > 0)
            .map(Some)
            .ok_or_else(|| "Port 1-65535 olmalı".into()),
    }
}

fn conn_id(o: &Map<String, Value>) -> Result<&str, String> {
    let id = o.get("id").and_then(Value::as_str).unwrap_or("");
    if valid_conn_id(id) {
        Ok(id)
    } else {
        Err("Kimlik: küçük harf, rakam ve - (en fazla 40)".into())
    }
}

fn save_ssh(doc: &mut DocumentMut, o: &Map<String, Value>) -> Result<(), String> {
    let id = conn_id(o)?;
    if has_entry(doc, "db", id) {
        return Err("Aynı kimlik iki bağlantıda kullanılamaz.".into());
    }
    let auth = o.get("auth").and_then(Value::as_str).unwrap_or("agent");
    if auth == "password" {
        return Err(
            "Kor SSH'ye parolayla bağlanmaz; anahtar dosyası veya ssh-agent kullan.".into(),
        );
    }
    if field(o, "password").is_some() {
        return Err("Anahtar parolası kaydedilemez: Kor SSH'yi etkileşimsiz çalıştırır. Anahtarı ssh-agent'a ekleyip ssh-agent seç.".into());
    }
    let host = field(o, "host").ok_or("Sunucu adresi gerekli")?;
    let user = field(o, "username");
    let port = port_of(o)?.filter(|p| *p != 22);
    let key = if auth == "key" {
        let k = field(o, "keyPath").ok_or("Anahtar dosyası gerekli")?;
        if k.len() > 1024 || k.contains(['\0', '\n']) {
            return Err("Geçersiz anahtar dosyası yolu".into());
        }
        Some(k)
    } else {
        None
    };
    let t = section(doc, "ssh", id)?;
    t.insert("host", toml_edit::value(host));
    set_opt(t, "user", user.map(toml_edit::value));
    set_opt(t, "port", port.map(|p| toml_edit::value(i64::from(p))));
    set_opt(t, "identity", key.map(toml_edit::value));
    Ok(())
}

/// Dönüş: parola eylemi (`Some(Some(p))` kaydet, `Some(None)` sil, `None` dokunma).
fn save_db<'a>(
    doc: &mut DocumentMut,
    o: &'a Map<String, Value>,
) -> Result<Option<Option<&'a str>>, String> {
    let id = conn_id(o)?;
    if has_entry(doc, "ssh", id) {
        return Err("Aynı kimlik iki bağlantıda kullanılamaz.".into());
    }
    let kind = o.get("kind").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "postgres" | "mysql" | "sqlite") {
        return Err("Bilinmeyen veritabanı türü".into());
    }
    let sqlite = kind == "sqlite";
    let file = field(o, "file");
    if sqlite && file.is_none_or(|f| f.len() > 1024) {
        return Err("SQLite için dosya yolu gerekli".into());
    }
    let port = port_of(o)?;
    let ssl = o.get("ssl").and_then(Value::as_bool).unwrap_or(false);
    let t = section(doc, "db", id)?;
    let old_mode = t.get("sslmode").and_then(Item::as_str).map(str::to_owned);
    let sslmode = match (ssl && !sqlite, old_mode) {
        (true, Some(m)) if is_tls_mode(&m) => Some(m),
        (true, _) => Some("verify-full".to_owned()),
        (false, _) => None,
    };
    let server = |k: &str| {
        (!sqlite)
            .then(|| field(o, k))
            .flatten()
            .map(toml_edit::value)
    };
    t.insert("kind", toml_edit::value(kind));
    set_opt(t, "host", server("host"));
    set_opt(
        t,
        "port",
        port.filter(|_| !sqlite)
            .map(|p| toml_edit::value(i64::from(p))),
    );
    set_opt(t, "database", server("database"));
    set_opt(t, "user", server("user"));
    set_opt(t, "file", file.filter(|_| sqlite).map(toml_edit::value));
    set_opt(t, "sslmode", sslmode.map(toml_edit::value));
    set_opt(t, "ssh", server("sshHost"));
    Ok(if let Some(p) = field(o, "password") {
        Some(Some(p))
    } else if o.get("clearSecret").and_then(Value::as_bool) == Some(true) {
        Some(None)
    } else {
        None
    })
}

fn set_db_secret(id: &str, secret: Option<&str>) -> Result<(), String> {
    match secret {
        Some(p) => kor_secrets::store_key(
            &db_account(id),
            &kor_secrets::SecretString::from(p.to_owned()),
            &kor_secrets::KeychainStore,
        )
        .map_err(|e| format!("Parola kasaya kaydedilemedi: {e}")),
        None => {
            let account = kor_secrets::account_for(&db_account(id)).map_err(|e| e.to_string())?;
            kor_secrets::KeychainStore
                .delete(&account)
                .map(|_| ())
                .map_err(|e| format!("Parola kasadan silinemedi: {e}"))
        }
    }
}

/// Ajanın kendi aracını (ssh_exec / db_query) çalıştırır: test, ajanla birebir aynı yolu dener.
async fn run_remote_tool(
    path: &Path,
    state: &AppState,
    tool: &str,
    input: Value,
) -> Result<kor_tools::ToolOutput, String> {
    let ctx = kor_tools::ToolCtx::new(&root(state)?).map_err(|e| e.to_string())?;
    let tools = kor_agent::remote::tools(Some(path.to_path_buf()));
    let t = tools
        .iter()
        .find(|t| t.name() == tool)
        .ok_or("Uzak bağlantı aracı bulunamadı")?;
    Ok(t.call(input, &ctx).await)
}

async fn test_ssh(path: &Path, state: &AppState, id: &str) -> Res {
    let c = load_conns(path)?;
    let h = c.ssh.get(id).ok_or("SSH sunucusu bulunamadı")?.clone();
    let out = run_remote_tool(
        path,
        state,
        "ssh_exec",
        json!({"host": id, "command": "exit 0", "timeout_seconds": 30}),
    )
    .await?;
    if !out.is_error && out.content.contains("\nexit 0\n") {
        return Ok(json!({ "ok": true }));
    }
    let text = out.content;
    let mismatch = text.contains("REMOTE HOST IDENTIFICATION HAS CHANGED");
    if mismatch || text.contains("Host key verification failed") {
        return Ok(match scan_keys(&h.host, h.port.unwrap_or(22)).await {
            Ok(keys) => match keys.into_iter().next() {
                Some((_, fp)) => {
                    json!({ "ok": false, "hostKey": { "fingerprint": fp, "mismatch": mismatch } })
                }
                None => json!({ "ok": false, "error": "Sunucu anahtarı okunamadı" }),
            },
            Err(e) => json!({ "ok": false, "error": e }),
        });
    }
    let msg = text
        .strip_prefix(kor_untrusted_header(&text))
        .unwrap_or(&text)
        .trim()
        .to_owned();
    Ok(json!({ "ok": false, "error": clip(&msg, 2000) }))
}

/// Uzak çıktı başındaki "[Output below comes from a remote system…]" satırı.
fn kor_untrusted_header(text: &str) -> &str {
    match text.split_once('\n') {
        Some((first, _)) if first.starts_with("[Output below") => &text[..=first.len()],
        _ => "",
    }
}

async fn trust_host_key(path: &Path, id: &str, fingerprint: &str) -> Result<(), String> {
    if !FINGERPRINT.is_match(fingerprint) || fingerprint.len() > 80 {
        return Err("Geçersiz parmak izi".into());
    }
    let c = load_conns(path)?;
    let h = c.ssh.get(id).ok_or("SSH sunucusu bulunamadı")?;
    let port = h.port.unwrap_or(22);
    let keys = scan_keys(&h.host, port).await?;
    // Kullanıcının gördüğü anahtar hâlâ sunucunun anahtarı olmalı (arada değiştiyse güvenme).
    let (line, _) = keys
        .into_iter()
        .find(|(_, fp)| fp == fingerprint)
        .ok_or("Sunucu anahtarı onaylanan parmak iziyle eşleşmiyor; bağlantıyı yeniden test et.")?;
    let ssh_dir = dirs::home_dir()
        .ok_or("Ev klasörü bulunamadı")?
        .join(".ssh");
    let known = ssh_dir.join("known_hosts");
    // Eski (değişmiş) kayıt kaldırılır; kullanıcı yeni anahtarı açıkça onayladı.
    if known.is_file() && known_fingerprint(&h.host, port).await.is_some() {
        let pat = host_pattern(&h.host, port);
        let kp = known.to_string_lossy().into_owned();
        exec(
            openssh("ssh-keygen"),
            &["-R", &pat, "-f", &kp],
            &tmp_dir(),
            10,
            &[],
        )
        .await?;
    }
    std::fs::create_dir_all(&ssh_dir).map_err(|e| e.to_string())?;
    let mut existing = read_text(&known)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(&line);
    existing.push('\n');
    std::fs::write(&known, existing).map_err(|e| format!("known_hosts yazılamadı: {e}"))
}

async fn connections(name: &str, args: &[Value], state: &AppState, m: &str) -> Option<Res> {
    if !matches!(
        name,
        "list" | "saveSsh" | "saveDb" | "remove" | "testSsh" | "testDb" | "trustHostKey"
    ) {
        return None;
    }
    let path = match conn_file() {
        Ok(p) => p,
        Err(e) => return Some(Err(e)),
    };
    let _g = CONN_LOCK.lock().await;
    let r: Res = async {
        match name {
            "list" => {}
            "saveSsh" => {
                let mut doc = parse_doc(&read_text(&path)?)?;
                save_ssh(&mut doc, obj(args, 0, m)?)?;
                commit_doc(&path, &doc)?;
            }
            "saveDb" => {
                let o = obj(args, 0, m)?;
                let mut doc = parse_doc(&read_text(&path)?)?;
                let secret = save_db(&mut doc, o)?;
                commit_doc(&path, &doc)?;
                if let Some(secret) = secret {
                    set_db_secret(conn_id(o)?, secret)?;
                }
            }
            "remove" => {
                let id = s(args, 0, m)?;
                if !valid_conn_id(id) {
                    return Err("Bağlantı bulunamadı".into());
                }
                let mut doc = parse_doc(&read_text(&path)?)?;
                let c = load_conns(&path)?;
                if let Some((db, _)) = c.db.iter().find(|(_, d)| d.ssh.as_deref() == Some(id)) {
                    return Err(format!(
                        "Bu sunucu {db} veritabanının tüneli; önce onu değiştir."
                    ));
                }
                let was_db = remove_entry(&mut doc, "db", id);
                if !was_db && !remove_entry(&mut doc, "ssh", id) {
                    return Err("Bağlantı bulunamadı".into());
                }
                commit_doc(&path, &doc)?;
                if was_db {
                    let _ = set_db_secret(id, None);
                }
            }
            "testSsh" => return test_ssh(&path, state, s(args, 0, m)?).await,
            "testDb" => {
                let id = s(args, 0, m)?;
                if !load_conns(&path)?.db.contains_key(id) {
                    return Err("Veritabanı bağlantısı bulunamadı".into());
                }
                let out = run_remote_tool(
                    &path,
                    state,
                    "db_query",
                    json!({"connection": id, "sql": "SELECT 1"}),
                )
                .await?;
                return Ok(if out.is_error {
                    json!({ "ok": false, "error": clip(&out.content, 2000) })
                } else {
                    json!({ "ok": true })
                });
            }
            _ => trust_host_key(&path, s(args, 0, m)?, s(args, 1, m)?).await?,
        }
        conn_view(&path).await
    }
    .await;
    Some(r)
}

// ---------------------------------------------------------------------------------------------
// g10: Kor tur günlüğü ve ölü mektup kuyruğu tutmaz → boş listeler; öz-denetim ve git kurulumu gerçek.
// ---------------------------------------------------------------------------------------------

fn check(id: &str, label: &str, status: &str, message: impl Into<String>) -> Value {
    json!({ "id": id, "label": label, "status": status, "message": message.into() })
}

fn can_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".g10-write-{}", now_ms()));
    let ok = std::fs::create_dir_all(dir).is_ok() && std::fs::write(&probe, "ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

async fn self_check(state: &AppState) -> Value {
    let mut items = Vec::new();
    let cwd = state
        .root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    match crate::bridge::load_config(&cwd) {
        Ok(loaded) => {
            let model = state.model().unwrap_or(loaded.config.agent.model);
            let provider = model.split_once('/').map_or(model.as_str(), |p| p.0);
            if kor_core::is_valid_provider_id(provider) {
                let found = kor_secrets::resolve_key(
                    provider,
                    &kor_secrets::env_var_for(provider),
                    |k| std::env::var(k).ok(),
                    &kor_secrets::KeychainStore,
                )
                .ok()
                .flatten()
                .is_some();
                items.push(if found {
                    check("apiKey", "API anahtarı", "ok", format!("{provider} anahtarı kayıtlı."))
                } else {
                    check("apiKey", "API anahtarı", "warn", format!("{provider} için kayıtlı API anahtarı yok (yerel sağlayıcıysa gerekmez)."))
                });
            }
            items.push(check(
                "config",
                "Yapılandırma",
                "ok",
                "Ayar dosyaları geçerli.",
            ));
        }
        Err(e) => items.push(check("config", "Yapılandırma", "fail", e)),
    }
    match kor_runtime::kor_home() {
        Some(home) if can_write(&home) => items.push(check("disk", "Disk", "ok", "Yazılabilir.")),
        Some(home) => items.push(check(
            "disk",
            "Disk",
            "fail",
            format!("Veri klasörüne yazılamıyor: {}", home.display()),
        )),
        None => items.push(check("disk", "Disk", "fail", "Kor veri klasörü bulunamadı")),
    }
    let git = exec("git", &["--version"], &tmp_dir(), 5, &[])
        .await
        .ok()
        .filter(|o| o.ok)
        .and_then(|o| o.stdout.lines().next().map(|l| l.trim().to_owned()));
    items.push(match git {
        Some(v) => check("git", "git", "ok", v),
        None => check(
            "git",
            "git",
            "warn",
            "git bulunamadı: git işlemleri ve yıkıcı git komutları öncesi kontrol noktası çalışmaz.",
        ),
    });
    Value::Array(items)
}

async fn g10(
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
    m: &str,
) -> Option<Res> {
    Some(match name {
        "listRecoverable" | "listDeadLetters" => Ok(json!([])),
        "recoveryInfo" | "dismiss" | "removeDeadLetter" => s(args, 0, m).map(|_| Value::Null),
        "restore" => Err("Kor tur günlüğü tutmuyor; geri yüklenecek adım yok.".into()),
        "retryDeadLetter" => Err("Kayıt bulunamadı".into()),
        "selfCheck" => Ok(self_check(state).await),
        "installGit" => {
            let app = app.clone();
            let r = tokio::task::spawn_blocking(move || crate::runtimes::install(&app, "git"))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            Ok(match r {
                Ok(v) if v["installed"] == true => json!({
                    "ok": true,
                    "message": v["version"].as_str().map_or_else(|| "git kuruldu.".to_owned(), |ver| format!("git kuruldu: {ver}")),
                }),
                Ok(_) => {
                    json!({ "ok": false, "message": "winget bulunamadı; git indirme sayfası tarayıcıda açıldı." })
                }
                Err(e) => json!({ "ok": false, "message": e }),
            })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn due_routines_fire_once_and_reschedule() {
        let mut list = vec![
            serde_json::json!({"id":"a","enabled":true,"intervalMinutes":5,"nextRun":1000}),
            serde_json::json!({"id":"b","enabled":false,"intervalMinutes":5,"nextRun":1000}),
            serde_json::json!({"id":"c","enabled":true,"intervalMinutes":5,"nextRun":9000}),
        ];
        let due = kor_runtime::routines::mark_due(&mut list, 2000);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0]["id"], "a");
        assert_eq!(list[0]["nextRun"], 2000 + 5 * 60_000);
        assert_eq!(list[0]["lastRun"], 2000);
        assert!(
            kor_runtime::routines::mark_due(&mut list, 2000).is_empty(),
            "not twice"
        );
    }

    use super::*;

    #[test]
    fn git_helpers() {
        assert_eq!(
            github_web("git@github.com:acme/app.git").as_deref(),
            Some("https://github.com/acme/app")
        );
        assert_eq!(
            github_web("https://github.com/acme/app/").as_deref(),
            Some("https://github.com/acme/app")
        );
        assert_eq!(github_web("https://gitlab.com/acme/app"), None);
        assert_eq!(github_web("https://github.com/a/b/c"), None);
        assert_eq!(
            redact_userinfo("https://user:ghp_secret@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(
            redact_userinfo("git@github.com:a/b.git"),
            "git@github.com:a/b.git"
        );
        assert_eq!(parse_counts("3\t5"), Some((3, 5)));
        assert_eq!(parse_counts("x"), None);
        assert!(valid_ref("feature/x-1.2"));
        for bad in ["-x", "a..b", "a b", "", "a;b"] {
            assert!(!valid_ref(bad), "{bad}");
        }
    }

    #[test]
    fn terminal_cd_parsing_and_jail() {
        assert_eq!(cd_target("cd src"), Some("src"));
        assert_eq!(cd_target("  CD  "), Some(""));
        assert_eq!(cd_target("Set-Location ..\\x"), Some("..\\x"));
        assert_eq!(cd_target("cd x && ls"), None);
        assert_eq!(cd_target("cdx"), None);
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir(base.join("sub")).unwrap();
        let sub = resolve_dir(&base, &base, "sub").unwrap();
        assert_eq!(resolve_dir(&base, &sub, "..").unwrap(), base);
        assert!(resolve_dir(&base, &base, "..").is_err());
        assert!(resolve_dir(&base, &base, "missing").is_err());
    }

    #[test]
    fn log_entries_pair_calls_with_results() {
        use kor_core::Message;
        let msgs: Vec<Message> = serde_json::from_value(json!([
            {"role": "assistant", "parts": [
                {"type": "tool_use", "id": "a", "name": "bash", "input": {"command": "ls"}},
                {"type": "tool_use", "id": "b", "name": "write_file", "input": {"path": "x"}}
            ]},
            {"role": "user", "parts": [
                {"type": "tool_result", "tool_use_id": "a", "content": "ok", "is_error": false},
                {"type": "tool_result", "tool_use_id": "b", "content": "permission denied; do not retry this call", "is_error": true}
            ]}
        ]))
        .unwrap();
        let e = log_entries(&msgs, "t");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0]["decision"], "allow");
        assert_eq!(e[0]["ok"], true);
        assert_eq!(e[1]["decision"], "deny");
        assert!(e[1]["reason"].as_str().unwrap().starts_with("permission"));
    }

    #[test]
    fn triggers_and_tags_are_validated() {
        use kor_runtime::routines::{MAX_INTERVAL, add};
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("triggers.json");
        let o = json!({"prompt": "özet çıkar", "intervalMinutes": 0.2, "root": "C:/x"});
        let t = add(&p, o.as_object().unwrap(), 1000).unwrap();
        assert_eq!(t["intervalMinutes"], 1);
        assert_eq!(t["nextRun"], 61_000);
        assert_eq!(t["name"], "özet çıkar");
        let empty = json!({"prompt": "", "intervalMinutes": 5});
        assert!(add(&p, empty.as_object().unwrap(), 0).is_err());
        let big = json!({"prompt": "x", "intervalMinutes": 1e9});
        assert_eq!(
            add(&p, big.as_object().unwrap(), 0).unwrap()["intervalMinutes"],
            MAX_INTERVAL
        );

        assert!(check_tags(&[json!({"name": "imza", "content": "x"})]).is_ok());
        assert!(check_tags(&[json!({"name": "-x", "content": ""})]).is_err());
        assert!(
            check_tags(&[
                json!({"name": "a", "content": ""}),
                json!({"name": "A", "content": ""})
            ])
            .is_err()
        );
    }

    #[test]
    fn connections_round_trip_keeps_comments_and_validates() {
        let text =
            "# benim sunucularım\n[ssh.prod]\nhost = \"203.0.113.5\" # üretim\nuser = \"deploy\"\n";
        let mut doc = parse_doc(text).unwrap();
        let ssh = json!({"id": "prod", "name": "P", "host": "203.0.113.9", "port": 2222, "username": "deploy", "auth": "key", "keyPath": "~/.ssh/id_ed25519"});
        save_ssh(&mut doc, ssh.as_object().unwrap()).unwrap();
        let db = json!({"id": "app", "name": "A", "kind": "postgres", "host": "127.0.0.1", "database": "app", "ssl": true, "sshHost": "prod", "password": "p"});
        let secret = save_db(&mut doc, db.as_object().unwrap()).unwrap();
        assert_eq!(secret, Some(Some("p")));
        let out = doc.to_string();
        assert!(out.contains("# benim sunucularım"), "{out}");
        let c = kor_agent::remote::Connections::parse(&out).unwrap();
        assert_eq!(c.ssh["prod"].port, Some(2222));
        assert_eq!(c.ssh["prod"].identity.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(c.db["app"].sslmode.as_deref(), Some("verify-full"));
        assert_eq!(c.db["app"].ssh.as_deref(), Some("prod"));
        assert!(!out.contains("\"p\""), "parola dosyaya yazılmaz: {out}");

        // Parola ile SSH ve geçersiz kimlik reddedilir; aynı kimlik iki türde olamaz.
        let pw = json!({"id": "x", "host": "h", "auth": "password"});
        assert!(save_ssh(&mut doc, pw.as_object().unwrap()).is_err());
        let bad = json!({"id": "Bad_Id", "host": "h", "auth": "agent"});
        assert!(save_ssh(&mut doc, bad.as_object().unwrap()).is_err());
        let dup = json!({"id": "app", "host": "h", "auth": "agent"});
        assert!(save_ssh(&mut doc, dup.as_object().unwrap()).is_err());
        assert!(remove_entry(&mut doc, "db", "app") && !has_entry(&doc, "db", "app"));
        assert_eq!(
            kor_untrusted_header("[Output below x]\nexit 1\n"),
            "[Output below x]\n"
        );
    }
    /// Git paneli açılışta `git.status` çağırır: depo yok, ilk commit yok, ayrık HEAD ve çakışmalı merge
    /// durumlarında da panik olmadan arayüzün okuduğu biçimi (`conflicts` dizisi, `isRepo`) dönmeli; işleyici
    /// küçük bir yığında da (Windows ana iş parçacığı 1 MiB) taşmadan bitmeli.
    #[test]
    fn git_status_covers_panel_states_on_small_stack() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // git yoksa panel zaten "git bulunamadı" der; burada sınanacak bir şey yok.
        }
        let run = |dir: PathBuf| {
            std::thread::Builder::new()
                .stack_size(1024 * 1024)
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(git_status(&dir))
                })
                .unwrap()
                .join()
                .expect("git_status paniklemeden ve yığını taşırmadan bitmeli")
        };
        let g = |dir: &Path, a: &[&str]| {
            let o = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "-c",
                    "core.autocrlf=false",
                ])
                .args(a)
                .current_dir(dir)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .unwrap();
            o.status.success()
        };
        let shape = |v: &Value| {
            assert!(v["isRepo"].is_boolean(), "{v}");
            assert!(v["conflicts"].is_array(), "{v}");
            assert!(v["dirty"].is_u64(), "{v}");
        };

        let plain = tempfile::tempdir().unwrap();
        let v = run(plain.path().to_path_buf());
        shape(&v);
        assert_eq!(v["isRepo"], false);

        let repo = tempfile::tempdir().unwrap();
        let d = repo.path();
        assert!(g(d, &["init", "-q", "-b", "main"]));
        let v = run(d.to_path_buf());
        shape(&v);
        assert_eq!(v["isRepo"], true, "ilk commit öncesi de depo: {v}");

        std::fs::write(d.join("a.txt"), "1\n").unwrap();
        assert!(g(d, &["add", "."]) && g(d, &["commit", "-qm", "init"]));
        assert!(g(d, &["checkout", "-qb", "feature"]));
        std::fs::write(d.join("a.txt"), "feature\n").unwrap();
        assert!(g(d, &["commit", "-qam", "f"]));
        assert!(g(d, &["checkout", "-q", "main"]));
        std::fs::write(d.join("a.txt"), "main\n").unwrap();
        assert!(g(d, &["commit", "-qam", "m"]));
        assert!(!g(d, &["merge", "feature"]), "çakışma beklenir");
        let v = run(d.to_path_buf());
        shape(&v);
        assert_eq!(v["operation"], "merge", "{v}");
        assert_eq!(v["conflicts"], json!(["a.txt"]), "{v}");
        assert_eq!(v["defaultBase"], "main");

        assert!(g(d, &["merge", "--abort"]) && g(d, &["checkout", "-q", "--detach"]));
        let v = run(d.to_path_buf());
        shape(&v);
        assert!(v["branch"].is_null(), "ayrık HEAD: {v}");
        assert!(v["operation"].is_null(), "{v}");
    }
}
