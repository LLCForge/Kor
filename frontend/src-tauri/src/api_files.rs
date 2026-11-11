//! Dosya ve kod işleri: dosya seçme/arama, kod/Python/SQL çalıştırma (yerel onaylı), not defteri ve belge
//! okuma, bağlantı önizleme, PDF dışa aktarma, `/init` (AGENTS.md) ve proje haritası.
//! Her yol açık klasörün içinde kalır; kod yalnızca kullanıcının yerel onay penceresinden sonra çalışır.

use kor_platform::NoWindow as _;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::api::{Res, b, confirm, opt_s, root, s, save_dialog};
use crate::state::AppState;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "build",
];
const MAX_OUTPUT: usize = 200 * 1024;
const RUN_TIMEOUT: Duration = Duration::from_secs(60);
const SENSITIVE: &[&str] = &[".env", "id_rsa", "id_ed25519", ".npmrc", ".netrc"];

/// Kök içindeki yol (göreli ya da mutlak); dışarı kaçan `None`.
pub(crate) fn inside(root: &Path, raw: &str) -> Option<PathBuf> {
    crate::changes::inside(root, raw)
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sensitive(p: &Path) -> bool {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    SENSITIVE.iter().any(|s| name.starts_with(s))
        || ["pem", "key", "p12", "pfx"]
            .iter()
            .any(|e| name.ends_with(&format!(".{e}")))
}

/// Dosyayı ya da klasörü Geri Dönüşüm Kutusu'na taşır (Windows); başka sistemde kalıcı siler.
pub(crate) fn to_recycle_bin(p: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let script = if p.is_dir() {
            "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($env:KOR_TRASH, 'OnlyErrorDialogs', 'SendToRecycleBin')"
        } else {
            "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($env:KOR_TRASH, 'OnlyErrorDialogs', 'SendToRecycleBin')"
        };
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("KOR_TRASH", p);
        cmd.no_window();
        let st = cmd.status().map_err(|e| e.to_string())?;
        if st.success() {
            Ok(())
        } else {
            Err(format!(
                "Geri Dönüşüm Kutusu'na taşınamadı: {}",
                p.display()
            ))
        }
    } else if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(p).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------------------------
// Dosya arama
// ---------------------------------------------------------------------------------------------

/// Bulanık eşleşme puanı: sorgu harfleri sırayla geçmeli; ardışık ve dosya adındaki eşleşmeler öne çıkar.
fn fuzzy(query: &str, path: &str) -> Option<i64> {
    let q: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let p: Vec<char> = path.to_lowercase().chars().collect();
    let name_start = path.rfind('/').map_or(0, |i| path[..=i].chars().count());
    let (mut qi, mut score, mut last) = (0usize, 0i64, None::<usize>);
    for (i, c) in p.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            score += 1;
            if last == Some(i.wrapping_sub(1)) {
                score += 3;
            }
            if i >= name_start {
                score += 2;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then(|| score * 100 - i64::try_from(p.len()).unwrap_or(0))
}

fn walk(root: &Path, limit: usize) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            if out.len() >= limit {
                return out;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let p = e.path();
            if ft.is_dir() {
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                out.push((rel(root, &p), true));
                stack.push(p);
            } else {
                out.push((rel(root, &p), false));
            }
        }
    }
    out
}

/// Dosya araması için klasör listesi önbelleği (§32.1 "50k dosyada fuzzy < 50 ms", ADR 0062): arama
/// önbellekteki listeden hemen yanıt verir; liste 2 sn'den eskiyse arka planda yeniden taranır
/// (bayat-ama-hızlı). İlk arama taramayı bekler.
static FILE_INDEX: std::sync::LazyLock<std::sync::Mutex<Option<FileIndex>>> =
    std::sync::LazyLock::new(std::sync::Mutex::default);

struct FileIndex {
    root: PathBuf,
    files: std::sync::Arc<Vec<(String, bool)>>,
    at: Instant,
    refreshing: bool,
}

/// Aranan en çok girdi (50k dosyalı projeler tam aranır).
const SEARCH_LIMIT: usize = 200_000;
const INDEX_FRESH: Duration = Duration::from_secs(2);

fn file_list(root: &Path) -> std::sync::Arc<Vec<(String, bool)>> {
    let lock = || FILE_INDEX.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cached = lock().as_mut().filter(|i| i.root == root).map(|i| {
        let stale = i.at.elapsed() > INDEX_FRESH && !i.refreshing;
        if stale {
            i.refreshing = true;
        }
        (i.files.clone(), stale)
    });
    match cached {
        Some((files, stale)) => {
            if stale {
                let root = root.to_path_buf();
                std::thread::spawn(move || {
                    let files = std::sync::Arc::new(walk(&root, SEARCH_LIMIT));
                    *lock() = Some(FileIndex { root, files, at: Instant::now(), refreshing: false });
                });
            }
            files
        }
        None => {
            let files = std::sync::Arc::new(walk(root, SEARCH_LIMIT));
            *lock() = Some(FileIndex { root: root.to_path_buf(), files: files.clone(), at: Instant::now(), refreshing: false });
            files
        }
    }
}

/// `g2SearchFiles`: en iyi 50 eşleşme (puan, yol, klasör mü). Engelleyici; `spawn_blocking` içinde çağrılır.
pub(crate) fn search_files(root: &Path, q: &str) -> Vec<(i64, String, bool)> {
    let mut scored: Vec<(i64, String, bool)> = file_list(root)
        .iter()
        .filter_map(|(p, dir)| fuzzy(q, p).map(|sc| (sc, p.clone(), *dir)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(50);
    scored
}

/// `g21ReadDoc` ve `kor_bytes`: proje içindeki bir belgenin baytları (hassas dosyalar ve 30 MB üstü hayır).
pub(crate) fn read_doc(root: &Path, raw: &str) -> Result<Vec<u8>, String> {
    let p = inside(root, raw).ok_or("Dosya açık klasörün dışında")?;
    if sensitive(&p) {
        return Err("Hassas dosyalar önizlenmez".into());
    }
    if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 30 * 1024 * 1024 {
        return Err("Dosya çok büyük".into());
    }
    std::fs::read(&p).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Süreç çalıştırma
// ---------------------------------------------------------------------------------------------

struct Run {
    out: String,
    err: String,
    code: Option<i32>,
    timed_out: bool,
    truncated: bool,
    ms: u128,
}

async fn run_process(
    program: &Path,
    args: &[&str],
    stdin: &str,
    cwd: &Path,
) -> Result<Run, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.no_window();
    let start = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{} başlatılamadı: {e}", program.display()))?;
    let mut input = child.stdin.take().ok_or("stdin yok")?;
    let mut out = child.stdout.take().ok_or("stdout yok")?;
    let mut err = child.stderr.take().ok_or("stderr yok")?;
    let data = stdin.to_owned();
    let work = async {
        let _ = input.write_all(data.as_bytes()).await;
        drop(input);
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let mut lo = (&mut out).take(MAX_OUTPUT as u64 + 1);
        let mut le = (&mut err).take(MAX_OUTPUT as u64 + 1);
        let _ = tokio::join!(lo.read_to_end(&mut o), le.read_to_end(&mut e));
        let status = child.wait().await.ok();
        (o, e, status)
    };
    match tokio::time::timeout(RUN_TIMEOUT, work).await {
        Ok((o, e, status)) => Ok(Run {
            truncated: o.len() > MAX_OUTPUT || e.len() > MAX_OUTPUT,
            out: String::from_utf8_lossy(&o[..o.len().min(MAX_OUTPUT)]).into_owned(),
            err: String::from_utf8_lossy(&e[..e.len().min(MAX_OUTPUT)]).into_owned(),
            code: status.and_then(|s| s.code()),
            timed_out: false,
            ms: start.elapsed().as_millis(),
        }),
        Err(_) => Ok(Run {
            out: String::new(),
            err: String::new(),
            code: None,
            timed_out: true,
            truncated: false,
            ms: start.elapsed().as_millis(),
        }),
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ""]
    } else {
        &[""]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|d| {
        exts.iter()
            .map(|e| d.join(format!("{name}{e}")))
            .find(|p| p.is_file())
    })
}

/// Projenin sanal ortamı varsa onun Python'u.
fn python(root: &Path) -> Option<PathBuf> {
    for v in [".venv", "venv"] {
        let p = if cfg!(windows) {
            root.join(v).join("Scripts").join("python.exe")
        } else {
            root.join(v).join("bin").join("python")
        };
        if p.is_file() {
            return Some(p);
        }
    }
    which("python")
        .or_else(|| which("python3"))
        .or_else(|| which("py"))
}

/// Proje/sistem Python'u; yoksa Kor'un Python bileşeni (ilk kullanımda bir kez onayla indirilir).
async fn python_or_component(app: &AppHandle, root: &Path) -> Result<PathBuf, String> {
    match python(root) {
        Some(p) => Ok(p),
        None => crate::runtimes::component(app, "python", true).await,
    }
}

/// Sistem Node.js'i; yoksa Kor'un Node bileşeni (ilk kullanımda bir kez onayla indirilir).
async fn node_or_component(app: &AppHandle) -> Result<PathBuf, String> {
    match which("node") {
        Some(p) => Ok(p),
        None => crate::runtimes::component(app, "node", true).await,
    }
}

fn bash() -> Option<PathBuf> {
    // Windows'ta WSL'nin bash.exe'si değil, Git Bash.
    if cfg!(windows) {
        let git = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
        return git.is_file().then_some(git);
    }
    which("bash")
}

fn preview(code: &str) -> String {
    let head: String = code.chars().take(600).collect();
    if code.chars().count() > 600 {
        format!("{head}\n…")
    } else {
        head
    }
}

async fn run_snippet(app: &AppHandle, state: &AppState, code: &str, language: &str) -> Res {
    if code.len() > 200_000 {
        return Err("Kod çok uzun".into());
    }
    let cwd = state.root().unwrap_or_else(std::env::temp_dir);
    let lang = language.to_ascii_lowercase();
    let tmp_ts = std::env::temp_dir().join(format!("kor-snippet-{}.ts", std::process::id()));
    let (program, args): (PathBuf, Vec<String>) = match lang.as_str() {
        "python" | "py" => (python_or_component(app, &cwd).await?, vec!["-".into()]),
        "javascript" | "js" | "node" | "mjs" => (node_or_component(app).await?, vec!["-".into()]),
        "typescript" | "ts" => {
            let node = node_or_component(app).await?;
            std::fs::write(&tmp_ts, code).map_err(|e| e.to_string())?;
            (
                node,
                vec![
                    "--experimental-strip-types".into(),
                    tmp_ts.to_string_lossy().into_owned(),
                ],
            )
        }
        "bash" | "sh" | "shell" => (
            bash().ok_or("Bash bulunamadı (Git for Windows)")?,
            vec!["-s".into()],
        ),
        "powershell" | "ps1" | "pwsh" => (
            which("powershell")
                .or_else(|| which("pwsh"))
                .ok_or("PowerShell bulunamadı")?,
            vec!["-NoProfile".into(), "-Command".into(), "-".into()],
        ),
        other => return Err(format!("`{other}` dili çalıştırılamıyor")),
    };
    let msg = format!(
        "Bu {language} kodu {} klasöründe çalıştırılsın mı?\n\n{}",
        cwd.display(),
        preview(code)
    );
    if !confirm(app, "Kodu çalıştır", &msg, "Çalıştır").await {
        let _ = std::fs::remove_file(&tmp_ts);
        return Ok(Value::Null);
    }
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let r = run_process(
        &program,
        &argv,
        if lang == "typescript" || lang == "ts" {
            ""
        } else {
            code
        },
        &cwd,
    )
    .await;
    let _ = std::fs::remove_file(&tmp_ts);
    let r = r?;
    let mut output = r.out;
    if !r.err.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&r.err);
    }
    Ok(
        json!({ "output": output, "exitCode": r.code, "timedOut": r.timed_out, "truncated": r.truncated }),
    )
}

// ---------------------------------------------------------------------------------------------
// SQL (SQLite, salt okunur)
// ---------------------------------------------------------------------------------------------

fn sql_query(path: &Path, sql: &str) -> Result<Value, String> {
    let start = Instant::now();
    let s = sql.trim().trim_end_matches(';').trim();
    if s.is_empty() || s.contains(';') {
        return Err("Tek bir SQL ifadesi gerekli".into());
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let c = rusqlite::Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    c.pragma_update(None, "query_only", true)
        .map_err(|e| e.to_string())?;
    let mut stmt = c.prepare(s).map_err(|e| e.to_string())?;
    if !stmt.readonly() {
        return Err("Yalnızca okuma sorguları çalıştırılır".into());
    }
    let columns: Vec<String> = stmt.column_names().map(str::to_owned).collect();
    let n = columns.len();
    let mut rows = Vec::new();
    let mut truncated = false;
    let mut it = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = it.next().map_err(|e| e.to_string())? {
        if rows.len() >= 1000 {
            truncated = true;
            break;
        }
        let r: Vec<String> = (0..n)
            .map(|i| match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) | Err(_) => "NULL".into(),
                Ok(rusqlite::types::ValueRef::Integer(v)) => v.to_string(),
                Ok(rusqlite::types::ValueRef::Real(v)) => v.to_string(),
                Ok(rusqlite::types::ValueRef::Text(t)) => {
                    String::from_utf8_lossy(t).chars().take(500).collect()
                }
                Ok(rusqlite::types::ValueRef::Blob(b)) => format!("<blob {} bayt>", b.len()),
            })
            .collect();
        rows.push(r);
    }
    Ok(
        json!({ "columns": columns, "rows": rows, "truncated": truncated, "durationMs": start.elapsed().as_millis() }),
    )
}

// ---------------------------------------------------------------------------------------------
// Bağlantı önizleme
// ---------------------------------------------------------------------------------------------

fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64)
        }
        std::net::IpAddr::V6(v) => {
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (v.segments()[0] & 0xfe00) == 0xfc00
                || (v.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

fn meta_tag(html: &str, key: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(
        r#"(?is)<meta[^>]+(?:property|name)\s*=\s*["']{key}["'][^>]*content\s*=\s*["']([^"']*)["']"#
    ))
    .ok()?;
    let alt = regex::Regex::new(&format!(
        r#"(?is)<meta[^>]+content\s*=\s*["']([^"']*)["'][^>]*(?:property|name)\s*=\s*["']{key}["']"#
    ))
    .ok()?;
    re.captures(html)
        .or_else(|| alt.captures(html))
        .map(|c| decode_entities(c[1].trim()))
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

async fn link_preview(url: &str) -> Res {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Geçersiz adres".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Ok(Value::Null);
    }
    let host = parsed.host_str().ok_or("Adreste sunucu yok")?.to_owned();
    // Gizlilik/çevrimdışı modda DNS sorgusu bile yapılmaz.
    if kor_core::net::check_host(&host).is_err() {
        return Ok(Value::Null);
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| e.to_string())?
        .collect();
    // SSRF: yerel ağ, localhost ve bulut meta veri adresleri önizlenmez.
    if addrs.is_empty() || addrs.iter().any(|a| !public_ip(a.ip())) {
        return Ok(Value::Null);
    }
    let client = kor_core::net::client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(6))
        .user_agent(concat!("kor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let Ok(resp) = client.get(parsed.clone()).send().await else {
        return Ok(Value::Null);
    };
    if !resp.status().is_success() {
        return Ok(Value::Null);
    }
    let is_html = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("html"));
    if !is_html {
        return Ok(Value::Null);
    }
    let mut body = Vec::new();
    let mut stream = resp;
    while let Ok(Some(chunk)) = stream.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > 512 * 1024 {
            break;
        }
    }
    let html = String::from_utf8_lossy(&body);
    let title = meta_tag(&html, "og:title").or_else(|| {
        regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>")
            .ok()?
            .captures(&html)
            .map(|c| decode_entities(c[1].trim()))
    });
    let description = meta_tag(&html, "og:description").or_else(|| meta_tag(&html, "description"));
    let site = meta_tag(&html, "og:site_name").or(Some(host));
    let clip = |s: Option<String>, n: usize| {
        s.map(|x| x.chars().take(n).collect::<String>())
            .filter(|x| !x.is_empty())
    };
    Ok(
        json!({ "url": url, "title": clip(title, 200), "description": clip(description, 400), "siteName": clip(site, 100) }),
    )
}

// ---------------------------------------------------------------------------------------------
// /init ve proje haritası
// ---------------------------------------------------------------------------------------------

fn stack_summary(root: &Path) -> (Vec<(String, usize)>, Vec<&'static str>, Vec<String>) {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (p, dir) in walk(root, 20_000) {
        if dir {
            continue;
        }
        if let Some(ext) = Path::new(&p).extension() {
            *counts
                .entry(ext.to_string_lossy().to_ascii_lowercase())
                .or_default() += 1;
        }
    }
    let mut langs: Vec<(String, usize)> = counts.into_iter().collect();
    langs.sort_by(|a, b| b.1.cmp(&a.1));
    langs.truncate(6);
    let markers: Vec<&str> = [
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "composer.json",
        "Gemfile",
        "Makefile",
        "Dockerfile",
        "tsconfig.json",
    ]
    .into_iter()
    .filter(|f| root.join(f).exists())
    .collect();
    let mut top: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_str()))
        .collect();
    top.sort();
    (langs, markers, top)
}

fn package_scripts(root: &Path) -> Vec<String> {
    let Ok(t) = std::fs::read_to_string(root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&t) else {
        return Vec::new();
    };
    v["scripts"]
        .as_object()
        .map(|o| o.keys().take(12).cloned().collect())
        .unwrap_or_default()
}

fn agents_draft(root: &Path) -> String {
    let (langs, markers, top) = stack_summary(root);
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut s = format!(
        "# {name}\n\nKor ve diğer kodlama ajanları için proje notları. Taslak Kor tarafından taranarak oluşturuldu; düzeltip genişletin.\n\n## Yapı\n\n"
    );
    for d in &top {
        s.push_str(&format!("- `{d}/`\n"));
    }
    if !langs.is_empty() {
        s.push_str("\n## Diller\n\n");
        for (ext, n) in &langs {
            s.push_str(&format!("- .{ext}: {n} dosya\n"));
        }
    }
    if !markers.is_empty() {
        s.push_str("\n## Derleme ve araçlar\n\n");
        for m in &markers {
            s.push_str(&format!("- `{m}`\n"));
        }
    }
    let scripts = package_scripts(root);
    if !scripts.is_empty() {
        s.push_str("\n## Komutlar\n\n");
        for sc in scripts {
            s.push_str(&format!("- `npm run {sc}`\n"));
        }
    }
    if markers.contains(&"Cargo.toml") {
        s.push_str("- `cargo build`, `cargo test`, `cargo clippy`\n");
    }
    s.push_str("\n## Kurallar\n\n- Mevcut kod stiline ve klasör yapısına uyun.\n- Değişiklikten sonra testleri çalıştırın.\n");
    s
}

pub(crate) fn map_file(root: &Path) -> Option<PathBuf> {
    let slug = kor_agent::memory::sanitize_path(&root.to_string_lossy().replace('\\', "/"));
    kor_runtime::kor_home().map(|h| h.join("maps").join(format!("{slug}.md")))
}

/// Haritanın metni (sistem istemine eklenir); yoksa `None`.
pub(crate) fn project_map_text(root: &Path) -> Option<String> {
    let t = std::fs::read_to_string(map_file(root)?).ok()?;
    Some(t.chars().take(12_000).collect())
}

fn build_map(root: &Path) -> Result<(), String> {
    let entries = walk(root, 4000);
    let mut s = format!(
        "Project map of `{}` (directories and files; generated by Kor).\n",
        root.display()
    );
    let mut shown = 0;
    for (p, dir) in &entries {
        if p.matches('/').count() > 3 {
            continue;
        }
        if shown >= 600 {
            s.push_str("…\n");
            break;
        }
        s.push_str(&format!("{p}{}\n", if *dir { "/" } else { "" }));
        shown += 1;
    }
    let f = map_file(root).ok_or("Kullanıcı veri klasörü yok")?;
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(f, s).map_err(|e| e.to_string())
}

fn map_info(state: &AppState) -> Value {
    let root = state.root();
    let file = root.as_deref().and_then(map_file);
    let meta = file.as_ref().and_then(|f| std::fs::metadata(f).ok());
    let generated = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis());
    let text = file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default();
    let count = text.lines().count().saturating_sub(1);
    let state_name = if meta.is_none() {
        "none"
    } else if generated.is_some_and(|g| meta::now_ms() as u128 - g > 7 * 86_400_000) {
        "stale"
    } else {
        "fresh"
    };
    json!({
        "state": state_name,
        "generatedAt": generated,
        "fileCount": count,
        "changedAreas": [],
        "buildTokens": text.len() / 4,
        "truncated": text.ends_with("…\n"),
        "root": root.as_ref().map(|r| r.to_string_lossy().into_owned()),
        "building": Value::Null,
        "error": Value::Null,
        "autoCreate": "never",
        "agentsFileExists": root.as_ref().is_some_and(|r| r.join("AGENTS.md").is_file()),
    })
}

use crate::meta;

// ---------------------------------------------------------------------------------------------
// Yönlendirici
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
pub async fn call(method: &str, args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let arg = |i: usize| s(args, i, method);
    match method {
        "pickFiles" => {
            let r = root(state)?;
            let (tx, rx) = std::sync::mpsc::channel();
            app.dialog().file().set_directory(&r).pick_files(move |p| {
                let _ = tx.send(p);
            });
            let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
                .await
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let rels: Vec<String> = picked
                .into_iter()
                .filter_map(|p| p.into_path().ok())
                .filter_map(|p| {
                    p.strip_prefix(&r)
                        .ok()
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                })
                .collect();
            Ok(json!(rels))
        }
        "g2SearchFiles" => {
            let r = root(state)?;
            let q = arg(0)?.to_owned();
            let hits = tauri::async_runtime::spawn_blocking(move || search_files(&r, &q))
            .await
            .map_err(|e| e.to_string())?;
            Ok(json!(
                hits.into_iter()
                    .map(|(_, p, d)| json!({ "path": p, "isDir": d }))
                    .collect::<Vec<_>>()
            ))
        }
        "g2StatFiles" => {
            let r = root(state)?;
            let list: Vec<Value> = args
                .first()
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(500)
                .map(|p| {
                    let m = inside(&r, p).and_then(|f| std::fs::metadata(f).ok());
                    json!({
                        "path": p,
                        "size": m.as_ref().filter(|m| m.is_file()).map(std::fs::Metadata::len),
                        "isDir": m.as_ref().is_some_and(std::fs::Metadata::is_dir),
                    })
                })
                .collect();
            Ok(json!(list))
        }
        "g2ResolveDropped" => {
            let r = root(state)?;
            let (mut accepted, mut rejected) = (Vec::new(), Vec::new());
            for raw in args
                .first()
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(200)
            {
                let p = Path::new(raw);
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| raw.to_owned());
                let canon = p.canonicalize().ok();
                match canon.as_ref().and_then(|c| {
                    c.strip_prefix(r.canonicalize().unwrap_or(r.clone()))
                        .ok()
                        .map(Path::to_path_buf)
                }) {
                    Some(relp) if !relp.components().any(|c| matches!(c, Component::ParentDir)) => {
                        accepted.push(relp.to_string_lossy().replace('\\', "/"));
                    }
                    _ => rejected.push(json!({ "name": name, "reason": "Açık klasörün dışında" })),
                }
            }
            Ok(json!({ "accepted": accepted, "rejected": rejected }))
        }
        "g3LinkPreview" => link_preview(arg(0)?).await,
        "g3SaveCode" => {
            let (content, language) = (arg(0)?, arg(1)?);
            let ext = match language.to_ascii_lowercase().as_str() {
                "python" | "py" => "py",
                "javascript" | "js" => "js",
                "typescript" | "ts" => "ts",
                "tsx" => "tsx",
                "rust" | "rs" => "rs",
                "bash" | "sh" | "shell" => "sh",
                "powershell" | "ps1" => "ps1",
                "json" => "json",
                "html" => "html",
                "css" => "css",
                "sql" => "sql",
                "markdown" | "md" => "md",
                "csharp" | "cs" => "cs",
                "cpp" | "c++" => "cpp",
                "go" => "go",
                _ => "txt",
            };
            let r = state.root();
            match save_dialog(app, r.as_deref(), &format!("kod.{ext}")).await {
                Some(p) => {
                    std::fs::write(&p, content).map_err(|e| e.to_string())?;
                    let shown = r.as_ref().and_then(|r| p.strip_prefix(r).ok()).map_or_else(
                        || p.to_string_lossy().into_owned(),
                        |x| x.to_string_lossy().replace('\\', "/"),
                    );
                    Ok(json!(shown))
                }
                None => Ok(Value::Null),
            }
        }
        "g3RunCode" => run_snippet(app, state, arg(0)?, arg(1)?).await,
        "g20StripTs" => {
            let node = node_or_component(app).await?;
            let script = "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{const m=require('node:module');if(!m.stripTypeScriptTypes){console.error('Node 22.13+ gerekli');process.exit(2)}process.stdout.write(m.stripTypeScriptTypes(s))})";
            let r = run_process(&node, &["-e", script], arg(0)?, &std::env::temp_dir()).await?;
            if r.code == Some(0) {
                Ok(json!(r.out))
            } else {
                Err(r.err.trim().to_owned())
            }
        }
        "g20RunPython" => {
            let code = arg(0)?;
            let cwd = state.root().unwrap_or_else(std::env::temp_dir);
            let py = python_or_component(app, &cwd).await?;
            let msg = format!(
                "Bu Python kodu {} ile çalıştırılsın mı?\n\n{}",
                py.display(),
                preview(code)
            );
            if !confirm(app, "Python çalıştır", &msg, "Çalıştır").await {
                return Ok(Value::Null);
            }
            let r = run_process(&py, &["-"], code, &cwd).await?;
            let mut chunks = Vec::new();
            if !r.out.is_empty() {
                chunks.push(json!({ "stream": "out", "text": r.out }));
            }
            if !r.err.is_empty() {
                chunks.push(json!({ "stream": "err", "text": r.err }));
            }
            Ok(json!({
                "chunks": chunks, "exitCode": r.code, "timedOut": r.timed_out, "truncated": r.truncated,
                "durationMs": r.ms, "interpreter": py.to_string_lossy(), "images": [], "vars": [],
            }))
        }
        "g20RunSql" => {
            let r = root(state)?;
            let path = inside(&r, arg(0)?).ok_or("Veritabanı açık klasörün dışında")?;
            if !path.is_file() {
                return Err("Veritabanı dosyası bulunamadı".into());
            }
            let sql = arg(1)?.to_owned();
            let msg = format!(
                "{} üzerinde salt okunur sorgu çalıştırılsın mı?\n\n{}",
                rel(&r, &path),
                preview(&sql)
            );
            if !confirm(app, "SQL çalıştır", &msg, "Çalıştır").await {
                return Ok(Value::Null);
            }
            tauri::async_runtime::spawn_blocking(move || sql_query(&path, &sql))
                .await
                .map_err(|e| e.to_string())?
        }
        "g20ReadNotebook" => {
            let r = root(state)?;
            let p = inside(&r, arg(0)?).ok_or("Dosya açık klasörün dışında")?;
            if p.extension().is_none_or(|e| e != "ipynb") {
                return Err("Yalnızca .ipynb dosyaları".into());
            }
            if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 20 * 1024 * 1024 {
                return Err("Not defteri çok büyük".into());
            }
            std::fs::read_to_string(&p)
                .map(Value::String)
                .map_err(|e| e.to_string())
        }
        // İkili veri JSON'da base64 taşınır; köprü (kor-bridge.ts) Uint8Array'e çevirir.
        // Arayüz bunu `kor_bytes` ile ham bayt olarak alır (ipc_bytes.rs); bu yol eski çağıranlar için.
        "g21ReadDoc" => {
            let (r, raw) = (root(state)?, arg(0)?.to_owned());
            let bytes = tauri::async_runtime::spawn_blocking(move || read_doc(&r, &raw))
                .await
                .map_err(|e| e.to_string())??;
            Ok(json!({ "$bytes": base64::engine::general_purpose::STANDARD.encode(bytes) }))
        }
        "g17ExportPdf" => {
            let (html, name) = (arg(0)?, opt_s(args, 1).unwrap_or("belge"));
            let browser = kor_tools::find_browser().ok_or("PDF için Chrome ya da Edge gerekli")?;
            let dir = std::env::temp_dir().join(format!("kor-pdf-{}", meta::now_ms()));
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let page = dir.join("page.html");
            let out = dir.join("out.pdf");
            std::fs::write(&page, html).map_err(|e| e.to_string())?;
            let url = format!("file:///{}", page.to_string_lossy().replace('\\', "/"));
            let arg_out = format!("--print-to-pdf={}", out.display());
            let profile = format!("--user-data-dir={}", dir.join("profile").display());
            let r = run_process(
                &browser,
                &[
                    "--headless=new",
                    "--disable-gpu",
                    "--no-pdf-header-footer",
                    "--disable-extensions",
                    &profile,
                    &arg_out,
                    &url,
                ],
                "",
                &dir,
            )
            .await;
            let result = match r {
                Ok(_) if out.is_file() => {
                    let file_name = format!(
                        "{}.pdf",
                        name.trim_end_matches(".pdf")
                            .chars()
                            .filter(|c| c.is_alphanumeric() || " -_".contains(*c))
                            .collect::<String>()
                    );
                    match save_dialog(app, state.root().as_deref(), &file_name).await {
                        Some(dest) => std::fs::copy(&out, &dest)
                            .map(|_| json!(dest.to_string_lossy()))
                            .map_err(|e| e.to_string()),
                        None => Ok(Value::Null),
                    }
                }
                Ok(_) => Err("PDF oluşturulamadı".into()),
                Err(e) => Err(e),
            };
            let _ = std::fs::remove_dir_all(&dir);
            result
        }
        "initProject" => {
            let r = root(state)?;
            let path = r.join("AGENTS.md");
            if path.exists() && !b(args, 0) {
                return Ok(json!({ "status": "exists", "path": path.to_string_lossy() }));
            }
            let backup = if path.exists() {
                let bk = r.join(format!("AGENTS.md.bak-{}", meta::now_ms()));
                std::fs::copy(&path, &bk).map_err(|e| e.to_string())?;
                Some(bk.to_string_lossy().into_owned())
            } else {
                None
            };
            let rr = r.clone();
            let draft = tauri::async_runtime::spawn_blocking(move || agents_draft(&rr))
                .await
                .map_err(|e| e.to_string())?;
            std::fs::write(&path, &draft).map_err(|e| e.to_string())?;
            Ok(
                json!({ "status": "created", "path": path.to_string_lossy(), "backup": backup, "chars": draft.chars().count() }),
            )
        }
        "getProjectMap" => Ok(map_info(state)),
        "buildProjectMap" => {
            let r = root(state)?;
            tauri::async_runtime::spawn_blocking(move || build_map(&r))
                .await
                .map_err(|e| e.to_string())??;
            Ok(Value::Null)
        }
        // Üretim eşzamanlı ve kısa; iptal edilecek bir arka plan işi yok.
        "cancelProjectMap" => Ok(Value::Null),
        _ => crate::ns::call(method, args, app, state).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_file_names_and_needs_all_letters() {
        let a = fuzzy("chat", "src/chat.rs").unwrap();
        let b = fuzzy("chat", "c/h/a/t/x.rs").unwrap();
        assert!(a > b);
        assert!(fuzzy("zz", "src/chat.rs").is_none());
        assert_eq!(fuzzy("", "x"), Some(0));
    }

    #[test]
    fn private_addresses_are_not_previewed() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fd00::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn meta_tags_parse_both_orders() {
        let h = r#"<meta property="og:title" content="Kor &amp; co"><meta content="Açıklama" name="description">"#;
        assert_eq!(meta_tag(h, "og:title").unwrap(), "Kor & co");
        assert_eq!(meta_tag(h, "description").unwrap(), "Açıklama");
    }

    #[test]
    fn sql_is_read_only_and_single() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        rusqlite::Connection::open(&p)
            .unwrap()
            .execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (1),(2);")
            .unwrap();
        let v = sql_query(&p, "SELECT a FROM t;").unwrap();
        assert_eq!(v["rows"], json!([["1"], ["2"]]));
        assert!(sql_query(&p, "DELETE FROM t").is_err());
        assert!(sql_query(&p, "SELECT 1; SELECT 2").is_err());
    }

    #[test]
    fn agents_draft_lists_the_stack() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main(){}").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let d = agents_draft(dir.path());
        assert!(
            d.contains("- `src/`") && d.contains("- .rs: 1 dosya") && d.contains("cargo test"),
            "{d}"
        );
    }

    #[test]
    fn file_search_uses_the_cached_list_and_finds_late_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/deep")).unwrap();
        std::fs::write(dir.path().join("src/deep/chat_view.rs"), "").unwrap();
        let hits = search_files(dir.path(), "chatview");
        assert_eq!(hits.first().map(|h| h.1.as_str()), Some("src/deep/chat_view.rs"));
        // A new file shows up once the list is refreshed (at most INDEX_FRESH later).
        std::fs::write(dir.path().join("src/zeta.rs"), "").unwrap();
        FILE_INDEX.lock().unwrap().as_mut().unwrap().at -= INDEX_FRESH * 2;
        let _ = search_files(dir.path(), "zeta");
        let t = Instant::now();
        while search_files(dir.path(), "zeta").is_empty() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!search_files(dir.path(), "zeta").is_empty());
    }

    #[test]
    fn docs_outside_or_sensitive_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.pdf"), b"%PDF").unwrap();
        std::fs::write(dir.path().join(".env"), b"X=1").unwrap();
        assert_eq!(read_doc(dir.path(), "a.pdf").unwrap(), b"%PDF");
        assert!(read_doc(dir.path(), ".env").is_err());
        assert!(read_doc(dir.path(), "../x.pdf").is_err());
    }

    #[test]
    fn sensitive_files_are_refused() {
        assert!(sensitive(Path::new("a/.env.local")) && sensitive(Path::new("k.pem")));
        assert!(!sensitive(Path::new("docs/plan.pdf")));
    }
}
