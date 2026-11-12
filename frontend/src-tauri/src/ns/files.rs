//! Ad alanları: g4 (dosya ağacı, önizleme, harici editör, kontrol noktası, tanılama), g13 (sistem istemi
//! katmanları, .env yöneticisi, doküman kaynakları, bağlam dışa aktarma), g30 (otomatik hafıza onay kuyruğu),
//! g14 (yerel gözlem: kullanım defteri, süreçler, sağlayıcı denetimi). Sözleşmeler `frontend/src/desktop/ipc-g4.ts`,
//! `ipc-g13.ts`, `ipc-g14.ts` ve `ipc.ts` (g30).
//! Her yol açık klasörün içinde kalır (`kor_tools::resolve`, bağlar dahil); hassas dosyalar (.env, anahtarlar)
//! önizlenmez, düzenlenmez, taşınmaz. Hiçbir veri makineden çıkmaz; tek ağ isteği kullanıcının "Denetle"
//! dediği, kimlik bilgisi içermeyen sağlayıcı erişilebilirlik denetimidir.

use kor_platform::NoWindow as _;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use base64::Engine as _;
use kor_core::{ContentPart, Message, Role};
use regex::Regex;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::api::{Res, b, opt_s, root, s, save_dialog};
use crate::meta;
use crate::state::AppState;

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).expect("sabit desen"))
    }};
}

/// Bilinmeyen yöntemde `None`.
pub async fn call(
    ns: &str,
    name: &str,
    args: &[Value],
    app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    Some(match (ns, name) {
        ("g4", "listDir") => list_dir(args, state).await,
        ("g4", "listAllFiles") => list_all_files(args, state).await,
        ("g4", "preview") => preview(args, state),
        ("g4", "writeFile") => write_file(args, state),
        ("g4", "createFile") => create_entry(args, state, false),
        ("g4", "createDir") => create_entry(args, state, true),
        ("g4", "rename") => rename(args, state),
        ("g4", "trash") => trash(args, state).await,
        ("g4", "watch") => watch(app, state),
        ("g4", "detectEditors") => Ok(json!(
            EDITOR_IDS
                .iter()
                .filter(|id| find_editor(id).is_some())
                .collect::<Vec<_>>()
        )),
        ("g4", "openInEditor") => open_in_editor(args, app, state),
        ("g4", "listCheckpoints") => list_checkpoints(args, state),
        ("g4", "previewRewind") => preview_rewind(args, state),
        ("g4", "rewind") => rewind(args, app, state).await,
        ("g4", "runDiagnostics") => run_diagnostics(state).await,

        ("g13", "getLayers") => get_layers(args, state),
        ("g13", "setProjectPrompt") => set_project_prompt(args, state),
        ("g13", "setSessionPrompt") => set_session_prompt(args),
        ("g13", "envFiles") => env_files(state),
        ("g13", "envKeys") => env_keys(args, state),
        ("g13", "envReveal") => env_reveal(args, state),
        ("g13", "envSet") => env_set(args, state),
        ("g13", "envRemove") => env_remove(args, state),
        ("g13", "envTemplateReport") => env_template_report(state),
        ("g13", "envCreateTemplate") => env_create_template(state),
        ("g13", "getDocSources") => get_doc_sources(),
        ("g13", "setDocSources") => set_doc_sources(args),
        ("g13", "buildExport") => build_export(args, state),
        ("g13", "saveExport") => save_export(args, app, state).await,

        ("g30", "pendingList") => pending_list(),
        // `kor:g30/pending`: the list changed (other windows / the memory layer refresh).
        ("g30", "pendingResolve") => pending_resolve(args, state).inspect(|_| {
            let _ = app.emit("kor:g30/pending", Value::Null);
        }),
        ("g30", "getAutoMemory") => Ok(json!(kor_agent::memory::auto_memory_enabled(None))),
        ("g30", "setAutoMemory") => set_auto_memory(args),

        ("g14", "overview") => overview(state).await,
        ("g14", "cost") => cost(args, &crate::api::price_table(state).await),
        // Kor ham istek/yanıt gövdelerini saklamaz (yerel defterde yalnızca token sayıları var).
        ("g14", "raw") => Ok(json!([])),
        ("g14", "processes") => processes().await,
        ("g14", "probe") => probe(state).await,
        ("g14", "status") => status(args),
        ("g14", "getSettings") => obs_store(|v| Ok(obs_settings(v))),
        ("g14", "setSettings") => set_obs_settings(args),
        ("g14", "rate") => rate(args),
        ("g14", "getTags") => get_tags(args),
        ("g14", "setTags") => set_tags(args),
        _ => return None,
    })
}

// =============================================================================================
// Ortak yardımcılar
// =============================================================================================

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
const MAX_PATH_ARG: usize = 4096;
const MAX_DIR_ENTRIES: usize = 5_000;
const MAX_ALL_FILES: usize = 50_000;
const MAX_OUT: usize = 20_000_000;

/// Açık klasörün gerçek (bağları çözülmüş, `\\?\` öneksiz) yolu.
fn workspace(state: &AppState) -> Result<PathBuf, String> {
    let r = root(state)?;
    let c = r
        .canonicalize()
        .map_err(|e| format!("Çalışma klasörü açılamadı: {e}"))?;
    Ok(kor_tools::without_verbatim(&c))
}

/// Kökün içindeki yol; `..`, kök dışı mutlak yol ve dışarı çıkan bağlar reddedilir (CWE-22).
fn jail(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    if rel.is_empty() || rel.len() > MAX_PATH_ARG {
        return Err("Geçersiz yol".into());
    }
    kor_tools::resolve(root, rel).map_err(|_| "Yol açık klasörün dışında".into())
}

fn jail_or_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if matches!(rel.trim(), "" | "." | "/") {
        Ok(root.to_path_buf())
    } else {
        jail(root, rel)
    }
}

fn rel_of(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sensitive_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        ".env",
        "id_rsa",
        "id_ed25519",
        "id_ecdsa",
        "id_dsa",
        ".npmrc",
        ".netrc",
        ".pypirc",
    ]
    .iter()
    .any(|s| n.starts_with(s))
        || [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"]
            .iter()
            .any(|e| n.ends_with(e))
}

/// Hassas dosya: ad kuralı ya da `.ssh` klasörünün içi.
fn sensitive(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| sensitive_name(&n.to_string_lossy()))
        || p.components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case(".ssh"))
}

/// Tek bir dosya/klasör adı (ayraç, Windows'a özel yasak karakter ve ayrılmış ad yok).
fn valid_name(raw: &str) -> Result<&str, String> {
    let n = raw.trim();
    let stem = n.split('.').next().unwrap_or("");
    let bad = n.is_empty()
        || n.len() > 255
        || n.ends_with('.')
        || n.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        || re!(r"(?i)^(con|prn|aux|nul|com[1-9]|lpt[1-9])$").is_match(stem);
    if bad {
        Err("Geçersiz ad".into())
    } else {
        Ok(n)
    }
}

fn mtime_ms(m: &std::fs::Metadata) -> f64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_micros() as f64 / 1000.0)
}

/// Geçici dosya + taşıma: yazma yarıda kalırsa hedef bozulmaz.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("Geçersiz yol")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.kor-tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("Yazılamadı: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Yazılamadı: {e}")
    })
}

fn valid_session(id: &str) -> Result<&str, String> {
    if kor_runtime::valid_session_id(id) {
        Ok(id)
    } else {
        Err("Geçersiz oturum kimliği".into())
    }
}

/// `kor_home()` altındaki kendi JSON dosyalarımız (aynı süreçte eşzamanlı yazma kilitli).
fn own_file(name: &str) -> Result<PathBuf, String> {
    kor_runtime::kor_home()
        .map(|h| h.join(name))
        .ok_or_else(|| "Kullanıcı veri klasörü bulunamadı".into())
}

fn with_json<T>(name: &str, f: impl FnOnce(&mut Value) -> Result<T, String>) -> Result<T, String> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = own_file(name)?;
    let mut v = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} bozuk, düzeltin ya da silin: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.to_string()),
    };
    let before = v.clone();
    let out = f(&mut v)?;
    if v != before {
        let bytes = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
        atomic_write(&path, &bytes)?;
    }
    Ok(out)
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

struct Output {
    out: Vec<u8>,
    err: Vec<u8>,
    code: Option<i32>,
    truncated: bool,
}

/// Kabuksuz (argüman dizisi) alt süreç; zaman aşımında öldürülür. Uygulamanın gizli anahtarları
/// (adında KEY/TOKEN/SECRET/PASSWORD geçen ortam değişkenleri) alt sürece geçmez.
async fn run(
    program: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    input: Option<Vec<u8>>,
    limit: Duration,
    env: &[(&str, &str)],
) -> Result<Output, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    for (k, _) in std::env::vars_os() {
        let up = k.to_string_lossy().to_ascii_uppercase();
        if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
            .iter()
            .any(|w| up.contains(w))
        {
            cmd.env_remove(&k);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.no_window();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{} başlatılamadı: {e}", program.display()))?;
    let stdin = child.stdin.take();
    let mut so = child.stdout.take().ok_or("stdout yok")?;
    let mut se = child.stderr.take().ok_or("stderr yok")?;
    let work = async {
        let write = async {
            if let (Some(mut w), Some(data)) = (stdin, input) {
                let _ = w.write_all(&data).await;
            }
        };
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let mut lo = (&mut so).take(MAX_OUT as u64 + 1);
        let mut le = (&mut se).take(MAX_OUT as u64 + 1);
        let _ = tokio::join!(write, lo.read_to_end(&mut o), le.read_to_end(&mut e));
        let st = child.wait().await.ok();
        (o, e, st)
    };
    match tokio::time::timeout(limit, work).await {
        Ok((mut o, mut e, st)) => {
            let truncated = o.len() > MAX_OUT || e.len() > MAX_OUT;
            o.truncate(MAX_OUT);
            e.truncate(MAX_OUT);
            Ok(Output {
                out: o,
                err: e,
                code: st.and_then(|s| s.code()),
                truncated,
            })
        }
        Err(_) => Err(format!("Zaman aşımı ({} sn)", limit.as_secs())),
    }
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Kayıtlı sohbet dökümü; dosya yoksa `None`.
fn session_messages(id: &str) -> Result<Option<Vec<Message>>, String> {
    let dir = crate::store::sessions_dir().ok_or("Oturum klasörü bulunamadı")?;
    let path = crate::store::session_path(&dir, id)?;
    if !path.is_file() {
        return Ok(None);
    }
    kor_agent::session::load(&path)
        .map(Some)
        .map_err(|e| format!("Oturum okunamadı: {e}"))
}

/// Kullanıcının yazdığı metin (araç sonucu ve sıkıştırma sınırı değil); `chat.rs`'deki tur sayımıyla aynı.
fn user_text(m: &Message) -> Option<String> {
    if m.role != Role::User
        || m.parts
            .iter()
            .any(|p| matches!(p, ContentPart::ToolResult { .. }))
    {
        return None;
    }
    let text: Vec<&str> = m
        .parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let text = text.join("\n");
    (!text.is_empty()
        && !text.starts_with(kor_agent::compact::BOUNDARY)
        && !text.starts_with(kor_agent::compact::SESSION_BOUNDARY))
    .then_some(text)
}

// =============================================================================================
// g4: dosya ağacı ve dosya işlemleri
// =============================================================================================

/// Dizin yürüyüşü: (göreli yol, klasör mü, mtime ms, boyut); bağlar izlenmez. İkinci değer: sınır aşıldı.
type WalkEntry = (String, bool, f64, u64);

fn walk(root: &Path, skip_heavy: bool, limit: usize) -> (Vec<WalkEntry>, bool) {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            if out.len() >= limit {
                return (out, true);
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() || name == ".git" {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let p = e.path();
            if ft.is_dir() {
                if skip_heavy && SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push(p.clone());
            }
            out.push((rel_of(root, &p), ft.is_dir(), mtime_ms(&m), m.len()));
        }
    }
    (out, false)
}

/// `git check-ignore`: yok sayılan göreli yollar; depo değilse ya da git yoksa `None`.
async fn git_ignored(root: &Path, items: &[(bool, String, String)]) -> Option<HashSet<String>> {
    let git = which("git")?;
    let mut input = Vec::new();
    for (is_dir, _, rel) in items {
        input.extend_from_slice(rel.as_bytes());
        if *is_dir {
            input.push(b'/');
        }
        input.push(0);
    }
    let out = run(
        &git,
        &os(&["check-ignore", "--stdin", "-z"]),
        Some(root),
        Some(input),
        Duration::from_secs(10),
        &[],
    )
    .await
    .ok()?;
    // 0: bazıları yok sayıldı, 1: hiçbiri; diğerleri (128: depo değil) bilinmiyor.
    if !matches!(out.code, Some(0 | 1)) {
        return None;
    }
    Some(
        out.out
            .split(|c| *c == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).trim_end_matches('/').to_owned())
            .collect(),
    )
}

async fn list_dir(args: &[Value], state: &AppState) -> Res {
    let dir = s(args, 0, "g4.listDir")?;
    let respect = b(args, 1);
    let root = workspace(state)?;
    let abs = jail_or_root(&root, dir)?;
    let rd = std::fs::read_dir(&abs).map_err(|e| format!("Klasör okunamadı: {e}"))?;
    let mut items: Vec<(bool, String, String)> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != ".git")
        .take(MAX_DIR_ENTRIES)
        .map(|e| {
            (
                e.file_type().is_ok_and(|t| t.is_dir()),
                e.file_name().to_string_lossy().into_owned(),
                rel_of(&root, &e.path()),
            )
        })
        .collect();
    if respect {
        match git_ignored(&root, &items).await {
            Some(ignored) => items.retain(|(_, _, rel)| !ignored.contains(rel)),
            None => items.retain(|(d, name, _)| !(*d && SKIP_DIRS.contains(&name.as_str()))),
        }
    }
    items.sort_by(|a, c| {
        c.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&c.1.to_lowercase()))
    });
    Ok(Value::Array(
        items
            .into_iter()
            .map(|(is_dir, name, path)| {
                let sens = !is_dir && sensitive(Path::new(&path));
                json!({ "name": name, "path": path, "isDir": is_dir, "sensitive": sens })
            })
            .collect(),
    ))
}

async fn list_all_files(args: &[Value], state: &AppState) -> Res {
    let respect = b(args, 0);
    let root = workspace(state)?;
    if respect
        && let Some(git) = which("git")
        && let Ok(o) = run(
            &git,
            &os(&["ls-files", "-co", "--exclude-standard", "-z"]),
            Some(&root),
            None,
            Duration::from_secs(30),
            &[],
        )
        .await
        && o.code == Some(0)
    {
        let mut files: Vec<String> = o
            .out
            .split(|c| *c == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        let truncated = o.truncated || files.len() > MAX_ALL_FILES;
        files.truncate(MAX_ALL_FILES);
        return Ok(json!({ "files": files, "truncated": truncated }));
    }
    let (entries, truncated) =
        tokio::task::spawn_blocking(move || walk(&root, respect, MAX_ALL_FILES))
            .await
            .map_err(|e| e.to_string())?;
    let mut files: Vec<String> = entries.into_iter().filter(|e| !e.1).map(|e| e.0).collect();
    files.sort();
    Ok(json!({ "files": files, "truncated": truncated }))
}

const TEXT_LIMIT: u64 = 5 * 1024 * 1024;
const EDIT_LIMIT: u64 = 1024 * 1024;
const IMAGE_LIMIT: u64 = 10 * 1024 * 1024;
const PDF_LIMIT: u64 = 50 * 1024 * 1024;

fn image_mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        _ => return None,
    })
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn preview(args: &[Value], state: &AppState) -> Res {
    let root = workspace(state)?;
    let abs = jail(&root, s(args, 0, "g4.preview")?)?;
    if sensitive(&abs) {
        return Ok(json!({ "kind": "sensitive" }));
    }
    let m = std::fs::metadata(&abs).map_err(|e| format!("Dosya okunamadı: {e}"))?;
    if m.is_dir() {
        return Err("Bu bir klasör".into());
    }
    let size = m.len();
    let ext = abs
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let too_large = |limit: u64| json!({ "kind": "tooLarge", "size": size, "limit": limit });
    if let Some(mime) = image_mime(&ext) {
        if size > IMAGE_LIMIT {
            return Ok(too_large(IMAGE_LIMIT));
        }
        let bytes = std::fs::read(&abs).map_err(|e| format!("Dosya okunamadı: {e}"))?;
        return Ok(json!({ "kind": "image", "dataUrl": data_url(mime, &bytes), "size": size }));
    }
    if ext == "pdf" {
        if size > PDF_LIMIT {
            return Ok(too_large(PDF_LIMIT));
        }
        // shortcut: Kor'da PDF metin çıkarıcı yok; sayfaları arayüz (g21) kendisi çizer.
        return Ok(
            json!({ "kind": "pdf", "pages": null, "text": "", "truncated": false, "size": size }),
        );
    }
    if size > TEXT_LIMIT {
        return Ok(too_large(TEXT_LIMIT));
    }
    let bytes = std::fs::read(&abs).map_err(|e| format!("Dosya okunamadı: {e}"))?;
    if bytes.iter().take(8192).any(|c| *c == 0) {
        return Ok(json!({ "kind": "binary", "size": size }));
    }
    let Ok(content) = String::from_utf8(bytes) else {
        return Ok(json!({ "kind": "binary", "size": size }));
    };
    let mut v = json!({
        "kind": "text",
        "content": content,
        "size": size,
        "mtimeMs": mtime_ms(&m),
        "editable": size <= EDIT_LIMIT,
    });
    if ext == "svg" {
        // <img> içinde gösterilir: SVG'deki betik çalışmaz.
        v["imageDataUrl"] = json!(data_url("image/svg+xml", content.as_bytes()));
    }
    Ok(v)
}

fn write_file(args: &[Value], state: &AppState) -> Res {
    let root = workspace(state)?;
    let abs = jail(&root, s(args, 0, "g4.writeFile")?)?;
    let content = s(args, 1, "g4.writeFile")?;
    let expected = args.get(2).and_then(Value::as_f64);
    if sensitive(&abs) {
        return Err("Hassas dosyalar düzenlenemez".into());
    }
    if content.len() as u64 > TEXT_LIMIT {
        return Err("Dosya çok büyük (en fazla 5 MB)".into());
    }
    match std::fs::metadata(&abs) {
        Ok(m) if m.is_dir() => return Err("Bu bir klasör".into()),
        Ok(m) if expected.is_some_and(|e| (mtime_ms(&m) - e).abs() > 1.0) => {
            return Ok(json!({ "ok": false, "reason": "conflict" }));
        }
        Err(_) if expected.is_some() => return Ok(json!({ "ok": false, "reason": "conflict" })),
        _ => {}
    }
    atomic_write(&abs, content.as_bytes())?;
    let m = std::fs::metadata(&abs).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "mtimeMs": mtime_ms(&m) }))
}

fn create_entry(args: &[Value], state: &AppState, dir: bool) -> Res {
    let method = if dir { "g4.createDir" } else { "g4.createFile" };
    let root = workspace(state)?;
    let base = jail_or_root(&root, s(args, 0, method)?)?;
    let name = valid_name(s(args, 1, method)?)?;
    if !base.is_dir() {
        return Err("Hedef klasör bulunamadı".into());
    }
    let target = base.join(name);
    if !dir && sensitive(&target) {
        return Err("Hassas dosyalar buradan oluşturulamaz".into());
    }
    let r = if dir {
        std::fs::create_dir(&target)
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map(drop)
    };
    r.map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => "Bu adda bir öğe zaten var".to_owned(),
        _ => format!("Oluşturulamadı: {e}"),
    })?;
    Ok(json!(rel_of(&root, &target)))
}

fn rename(args: &[Value], state: &AppState) -> Res {
    let root = workspace(state)?;
    let src = jail(&root, s(args, 0, "g4.rename")?)?;
    let name = valid_name(s(args, 1, "g4.rename")?)?;
    if src == root {
        return Err("Çalışma klasörünün kendisi yeniden adlandırılamaz".into());
    }
    let dst = src.parent().ok_or("Geçersiz yol")?.join(name);
    if sensitive(&src) || sensitive(&dst) {
        return Err("Hassas dosyalarda dosya işlemleri kapalı".into());
    }
    if !src.exists() {
        return Err("Dosya bulunamadı".into());
    }
    // Yalnızca büyük/küçük harf değişimi (Windows'ta aynı dosya) dışında var olan hedef ezilmez.
    if dst.exists()
        && !src
            .to_string_lossy()
            .eq_ignore_ascii_case(&dst.to_string_lossy())
    {
        return Err("Bu adda bir öğe zaten var".into());
    }
    std::fs::rename(&src, &dst).map_err(|e| format!("Yeniden adlandırılamadı: {e}"))?;
    Ok(json!(rel_of(&root, &dst)))
}

async fn trash(args: &[Value], state: &AppState) -> Res {
    let root = workspace(state)?;
    let p = jail(&root, s(args, 0, "g4.trash")?)?;
    if p == root {
        return Err("Çalışma klasörünün kendisi silinemez".into());
    }
    if sensitive(&p) {
        return Err("Hassas dosyalarda dosya işlemleri kapalı".into());
    }
    if !p.exists() {
        return Err("Dosya bulunamadı".into());
    }
    if !cfg!(windows) {
        // Kalıcı silme yapılmaz (sözleşme: yalnızca çöp kutusu).
        return Err("Çöp kutusuna taşıma yalnızca Windows'ta destekleniyor".into());
    }
    // `api_files::to_recycle_bin` yalnızca dosya siler; klasörler için DeleteDirectory gerekir.
    let script = "Add-Type -AssemblyName Microsoft.VisualBasic; $p = $env:KOR_TRASH; \
        if (Test-Path -LiteralPath $p -PathType Container) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin') } \
        else { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }";
    let ps = which("powershell").unwrap_or_else(|| PathBuf::from("powershell"));
    let path = p.to_string_lossy().into_owned();
    let out = run(
        &ps,
        &os(&["-NoProfile", "-NonInteractive", "-Command", script]),
        None,
        None,
        Duration::from_secs(60),
        &[("KOR_TRASH", path.as_str())],
    )
    .await?;
    if out.code == Some(0) && !p.exists() {
        Ok(Value::Null)
    } else {
        Err(format!(
            "Geri Dönüşüm Kutusu'na taşınamadı: {}",
            rel_of(&root, &p)
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// İzleyici: bağımlılıksız yoklama (2 sn), en fazla WATCH_LIMIT girdi; yeni `watch` öncekini durdurur.
// ---------------------------------------------------------------------------------------------

const WATCH_LIMIT: usize = 20_000;
const WATCH_EVERY: Duration = Duration::from_secs(2);
const MAX_EVENT_PATHS: usize = 200;
static WATCH_GEN: AtomicU64 = AtomicU64::new(0);

type Snapshot = HashMap<String, (f64, u64)>;

fn snapshot(root: &Path) -> (Snapshot, bool) {
    let (entries, truncated) = walk(root, true, WATCH_LIMIT);
    (
        entries
            .into_iter()
            .map(|(p, _, t, n)| (p, (t, n)))
            .collect(),
        truncated,
    )
}

/// Eklenen, silinen ya da değişen yollar (sıralı).
fn diff(prev: &Snapshot, next: &Snapshot) -> Vec<String> {
    let mut out: BTreeSet<&String> = next
        .iter()
        .filter(|(k, v)| prev.get(*k) != Some(*v))
        .map(|(k, _)| k)
        .collect();
    out.extend(prev.keys().filter(|k| !next.contains_key(*k)));
    out.into_iter().cloned().collect()
}

fn watch(app: &AppHandle, state: &AppState) -> Res {
    let root = workspace(state)?;
    let shown = root.to_string_lossy().into_owned();
    let generation = WATCH_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let scan = |r: PathBuf| tokio::task::spawn_blocking(move || snapshot(&r));
        let Ok((mut prev, _)) = scan(root.clone()).await else {
            return;
        };
        loop {
            tokio::time::sleep(WATCH_EVERY).await;
            if WATCH_GEN.load(Ordering::SeqCst) != generation {
                return;
            }
            let Ok((next, truncated)) = scan(root.clone()).await else {
                return;
            };
            let mut paths = diff(&prev, &next);
            if !paths.is_empty() {
                let overflow = truncated || paths.len() > MAX_EVENT_PATHS;
                paths.truncate(MAX_EVENT_PATHS);
                let _ = app.emit(
                    "kor:g4/event",
                    json!({ "type": "fs", "root": shown, "paths": paths, "overflow": overflow }),
                );
            }
            prev = next;
        }
    });
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Harici editör
// ---------------------------------------------------------------------------------------------

const EDITOR_IDS: &[&str] = &["code", "cursor", "windsurf", "zed"];

fn editor_candidates(id: &str) -> Vec<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{id}.cmd"), format!("{id}.exe")]
    } else {
        vec![id.to_owned()]
    };
    let mut out: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .flat_map(|d| names.iter().map(move |n| d.join(n)))
                .collect()
        })
        .unwrap_or_default();
    // GUI'den başlatılan uygulamada PATH eksik olabilir: bilinen kurulum yerleri.
    if cfg!(windows) {
        let local =
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs");
        let pf = PathBuf::from(
            std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into()),
        );
        match id {
            "code" => out.extend([
                local.join(r"Microsoft VS Code\bin\code.cmd"),
                pf.join(r"Microsoft VS Code\bin\code.cmd"),
            ]),
            "cursor" => out.push(local.join(r"cursor\resources\app\bin\cursor.cmd")),
            "windsurf" => out.push(local.join(r"Windsurf\bin\windsurf.cmd")),
            _ => out.push(local.join(r"Zed\zed.exe")),
        }
    } else {
        for d in [
            "/usr/local/bin",
            "/opt/homebrew/bin",
            "/usr/bin",
            "/snap/bin",
        ] {
            out.push(Path::new(d).join(id));
        }
    }
    out
}

fn find_editor(id: &str) -> Option<PathBuf> {
    editor_candidates(id).into_iter().find(|p| p.is_file())
}

/// `code.cmd` sarmalayıcısı: `"%~dp0..\Code.exe" "%~dp0..\resources\app\out\cli.js" %*`. .cmd kabuk ister ve
/// dosya adındaki `&` gibi karakterlerle komut enjeksiyonuna açıktır; aynısını argüman dizisiyle yaparız.
fn parse_cmd_shim(content: &str, dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let c = re!(r#"(?i)"%~dp0([^"]+?\.exe)"\s+"%~dp0([^"]+?\.js)""#).captures(content)?;
    Some((dir.join(&c[1]), dir.join(&c[2])))
}

const RISKY_EXT: &[&str] = &[
    "exe",
    "bat",
    "cmd",
    "com",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "mjs",
    "cjs",
    "wsf",
    "wsh",
    "msi",
    "msp",
    "lnk",
    "scr",
    "hta",
    "cpl",
    "reg",
    "jar",
    "app",
    "sh",
    "command",
    "desktop",
    "appimage",
    "run",
    "py",
    "pyw",
    "url",
    "inf",
    "application",
    "gadget",
    "pif",
    "scf",
    "dll",
    "sys",
];

fn launch_editor(found: &Path, file: &Path, line: Option<u64>, zed: bool) -> Result<(), String> {
    let f = file.to_string_lossy().into_owned();
    let loc = line.map_or_else(|| f.clone(), |l| format!("{f}:{l}"));
    let target: Vec<String> = if zed {
        vec![loc]
    } else if line.is_some() {
        vec!["-g".into(), loc]
    } else {
        vec![f]
    };
    let is_cmd = found
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd"));
    let mut cmd = if is_cmd {
        let text = std::fs::read_to_string(found).map_err(|e| e.to_string())?;
        let (exe, cli) = parse_cmd_shim(&text, found.parent().ok_or("Geçersiz yol")?)
            .ok_or("Editör başlatıcısı tanınmadı")?;
        let mut c = std::process::Command::new(exe);
        c.arg(cli).env("ELECTRON_RUN_AS_NODE", "1");
        c
    } else {
        std::process::Command::new(found)
    };
    cmd.args(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.no_window();
    cmd.spawn()
        .map(drop)
        .map_err(|e| format!("Editör başlatılamadı: {e}"))
}

fn open_in_editor(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let root = workspace(state)?;
    let abs = jail(&root, s(args, 0, "g4.openInEditor")?)?;
    let line = args.get(1).and_then(Value::as_u64).filter(|l| *l > 0);
    if sensitive(&abs) {
        return Err("Hassas dosyalarda dosya işlemleri kapalı".into());
    }
    if !abs.exists() {
        return Err("Dosya bulunamadı".into());
    }
    let mut editor = opt_s(args, 2).unwrap_or("preferred").to_owned();
    if editor == "preferred" {
        editor = meta::read()
            .ok()
            .and_then(|m| m.settings.editor["type"].as_str().map(str::to_owned))
            .unwrap_or_default();
    }
    if let Some(id) = EDITOR_IDS.iter().find(|i| **i == editor)
        && let Some(found) = find_editor(id)
    {
        launch_editor(&found, &abs, line, *id == "zed")?;
        return Ok(json!(id));
    }
    let ext = abs
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if abs.is_file() && RISKY_EXT.contains(&ext.as_str()) {
        // Çalıştırılabilir tür varsayılan uygulamayla açılmaz; klasörde gösterilir.
        app.opener()
            .reveal_item_in_dir(&abs)
            .map_err(|e| e.to_string())?;
        return Ok(json!("folder"));
    }
    app.opener()
        .open_path(abs.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(json!("system"))
}

// ---------------------------------------------------------------------------------------------
// Kontrol noktaları: Kor dosya değişikliklerini `crate::changes`te sohbet başına (ilk hâl) tutar; tur başına
// ara kayıt yoktur. Bu yüzden tek kontrol noktası vardır: sohbetin ilk mesajından önceki hâl.
// ---------------------------------------------------------------------------------------------

const CHECKPOINT_ID: &str = "session-start";
const FILE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Delete"];

struct Checkpoint {
    index: usize,
    text: String,
    created: u64,
    files: Vec<String>,
}

/// Dosya araçlarının hedeflediği, kök içindeki göreli yollar (tekil, sıralı).
fn touched_files(messages: &[Message], root: &Path) -> Vec<String> {
    let mut out = BTreeSet::new();
    for m in messages {
        for p in &m.parts {
            if let ContentPart::ToolUse { name, input, .. } = p
                && FILE_TOOLS.contains(&name.as_str())
                && let Some(raw) = ["file_path", "path", "notebook_path"]
                    .iter()
                    .find_map(|k| input.get(*k).and_then(Value::as_str))
                && let Some(abs) = crate::changes::inside(root, raw)
            {
                out.insert(rel_of(root, &abs));
            }
        }
    }
    out.into_iter().collect()
}

fn checkpoint(state: &AppState, id: &str) -> Result<Option<Checkpoint>, String> {
    let Some(messages) = session_messages(id)? else {
        return Ok(None);
    };
    let Some((index, text)) = messages
        .iter()
        .enumerate()
        .find_map(|(i, m)| user_text(m).map(|t| (i, t)))
    else {
        return Ok(None);
    };
    let root = root(state)?;
    let files: Vec<String> = touched_files(&messages, &root)
        .into_iter()
        .filter(|f| crate::changes::file_change(id, &root, f).is_some())
        .collect();
    if files.is_empty() {
        return Ok(None);
    }
    let created = crate::store::sessions_dir()
        .and_then(|d| crate::store::session_path(&d, id).ok())
        .and_then(|p| std::fs::metadata(p).ok())
        .map_or(0.0, |m| {
            m.created()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0.0, |d| d.as_millis() as f64)
        });
    Ok(Some(Checkpoint {
        index,
        text,
        created: created as u64,
        files,
    }))
}

fn checkpoint_info(c: &Checkpoint) -> Value {
    json!({
        "id": CHECKPOINT_ID,
        "messageIndex": c.index,
        "text": c.text,
        "createdAt": c.created,
        "files": c.files,
        "available": true,
    })
}

fn list_checkpoints(args: &[Value], state: &AppState) -> Res {
    let id = valid_session(s(args, 0, "g4.listCheckpoints")?)?;
    Ok(json!(
        checkpoint(state, id)?
            .iter()
            .map(checkpoint_info)
            .collect::<Vec<_>>()
    ))
}

/// Dosyanın hedefe göre durumu: `None` = zaten hedefte, yoksa eylem adı.
fn rewind_action(target: Option<&str>, disk: Option<&str>) -> Option<&'static str> {
    match (target, disk) {
        (None, None) => None,
        (None, Some(_)) => Some("delete"),
        (Some(_), None) => Some("create"),
        (Some(t), Some(d)) if t == d => None,
        _ => Some("restore"),
    }
}

type RewindPlan = (Checkpoint, Vec<(String, &'static str)>);

/// Plan: (yol, eylem) — yalnızca diskteki hali hedeften farklı olanlar.
fn rewind_plan(state: &AppState, id: &str, cp_id: &str) -> Result<RewindPlan, String> {
    let cp = checkpoint(state, id)?
        .filter(|_| cp_id == CHECKPOINT_ID)
        .ok_or("Kontrol noktası bulunamadı (bellek sınırı nedeniyle silinmiş olabilir).")?;
    let root = root(state)?;
    let mut plan = Vec::new();
    for f in &cp.files {
        let Some(change) = crate::changes::file_change(id, &root, f) else {
            continue;
        };
        let abs = crate::changes::inside(&root, f).ok_or("Yol açık klasörün dışında")?;
        let disk = match std::fs::read_to_string(&abs) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            // Okunamayan (ikili vb.) dosya: geri yüklenecek sayılır.
            Err(_) => Some(String::from("\0")),
        };
        if let Some(action) = rewind_action(change["before"].as_str(), disk.as_deref()) {
            plan.push((f.clone(), action));
        }
    }
    Ok((cp, plan))
}

fn preview_rewind(args: &[Value], state: &AppState) -> Res {
    let id = valid_session(s(args, 0, "g4.previewRewind")?)?;
    let (cp, plan) = rewind_plan(state, id, s(args, 1, "g4.previewRewind")?)?;
    Ok(json!({
        "checkpoint": checkpoint_info(&cp),
        // shortcut: oturum dışı (harici) değişiklik takibi yok; Kor'un değişiklik deposu bunu tutmuyor.
        "files": plan.iter().map(|(p, a)| json!({ "path": p, "action": a, "external": false })).collect::<Vec<_>>(),
        "running": state.chats.running_ids().iter().any(|r| r == id),
    }))
}

async fn rewind(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let id = valid_session(s(args, 0, "g4.rewind")?)?;
    let cut = b(args, 2);
    if state.chats.running_ids().iter().any(|r| r == id) {
        return Err("Sohbet çalışırken geri sarılamaz".into());
    }
    let (cp, plan) = rewind_plan(state, id, s(args, 1, "g4.rewind")?)?;
    if cut {
        crate::chat::rewind(app, id, Some(0), &cp.text).await?;
    }
    let root = root(state)?;
    let mut failures = Vec::new();
    // Planda olmayanlar zaten hedefte; yine de deposundan düşülsün diye hepsi geri alınır.
    for f in &cp.files {
        if let Err(e) = crate::changes::revert(id, &root, f) {
            failures.push(format!("{f} ({e})"));
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "Bazı dosyalar geri yüklenemedi: {}",
            failures.join(", ")
        ));
    }
    Ok(json!({
        "restored": plan.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        "changes": [],
        "conversationCut": cut,
    }))
}

// ---------------------------------------------------------------------------------------------
// Tanılama: projenin kendi tsc / eslint kurulumu (node_modules), Node ile, kabuksuz.
// ---------------------------------------------------------------------------------------------

const DIAG_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_DIAGNOSTICS: usize = 2000;

/// Kökün içindeyse '/' ayraçlı göreli yol; değilse ham metin.
fn locate(root: &Path, file: &str) -> (String, bool) {
    match crate::changes::inside(root, file) {
        Some(p) if p != root => (rel_of(root, &p), true),
        _ => (file.to_owned(), false),
    }
}

fn diag(
    tool: &str,
    file: (String, bool),
    line: u64,
    col: u64,
    sev: &str,
    code: Option<&str>,
    msg: &str,
) -> Value {
    json!({
        "tool": tool, "file": file.0, "inWorkspace": file.1, "line": line, "column": col,
        "severity": sev, "code": code, "message": msg,
    })
}

/// `tsc --pretty false` çıktısı; girintili devam satırları önceki mesaja eklenir.
fn parse_tsc(output: &str, root: &Path) -> Vec<Value> {
    let line_re = re!(r"^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.*)$");
    let global_re = re!(r"^(error|warning) (TS\d+): (.*)$");
    let mut out: Vec<Value> = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(c) = line_re.captures(line) {
            out.push(diag(
                "tsc",
                locate(root, &c[1]),
                c[2].parse().unwrap_or(0),
                c[3].parse().unwrap_or(0),
                if &c[4] == "warning" {
                    "warning"
                } else {
                    "error"
                },
                Some(&c[5]),
                &c[6],
            ));
        } else if let Some(c) = global_re.captures(line) {
            out.push(diag(
                "tsc",
                (String::new(), false),
                0,
                0,
                if &c[1] == "warning" {
                    "warning"
                } else {
                    "error"
                },
                Some(&c[2]),
                &c[3],
            ));
        } else if raw.starts_with(char::is_whitespace)
            && let Some(last) = out.last_mut()
        {
            let m = format!(
                "{}\n{}",
                last["message"].as_str().unwrap_or(""),
                line.trim()
            );
            last["message"] = json!(m);
        }
    }
    out
}

/// `eslint --format json` çıktısı; önünde başka metin varsa ilk '[' karakterinden okunur.
fn parse_eslint(output: &str, root: &Path) -> Result<Vec<Value>, String> {
    let start = output.find('[').ok_or("ESLint çıktısı JSON değil")?;
    let files: Vec<Value> =
        serde_json::from_str(&output[start..]).map_err(|_| "ESLint çıktısı okunamadı.")?;
    let mut out = Vec::new();
    for f in &files {
        let path = f["filePath"].as_str().ok_or("ESLint çıktısı okunamadı.")?;
        for m in f["messages"].as_array().into_iter().flatten() {
            out.push(diag(
                "eslint",
                locate(root, path),
                m["line"].as_u64().unwrap_or(0),
                m["column"].as_u64().unwrap_or(0),
                if m["severity"].as_u64().unwrap_or(0) >= 2 {
                    "error"
                } else {
                    "warning"
                },
                m["ruleId"].as_str(),
                m["message"].as_str().unwrap_or(""),
            ));
        }
    }
    Ok(out)
}

/// Projenin `node_modules/<pkg>` içindeki komutun JS giriş dosyası (.cmd sarmalayıcısı kabuk ister).
fn local_bin(root: &Path, pkg: &str, bin: &str) -> Option<PathBuf> {
    let dir = root.join("node_modules").join(pkg);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(dir.join("package.json")).ok()?).ok()?;
    let rel = manifest["bin"]
        .as_str()
        .or_else(|| manifest["bin"][bin].as_str())?;
    let script = crate::changes::inside(&dir, rel)?;
    script.is_file().then_some(script)
}

fn tool_report(
    tool: &str,
    started: Instant,
    status: &str,
    reason: Option<String>,
    diags: Vec<Value>,
) -> Value {
    let truncated = diags.len() > MAX_DIAGNOSTICS;
    let diags: Vec<Value> = diags.into_iter().take(MAX_DIAGNOSTICS).collect();
    json!({
        "tool": tool, "status": status, "reason": reason, "diagnostics": diags,
        "truncated": truncated, "durationMs": started.elapsed().as_millis() as u64,
    })
}

async fn run_node(
    node: &Path,
    script: &Path,
    args: &[&str],
    root: &Path,
) -> Result<Output, String> {
    let mut argv = vec![script.as_os_str().to_owned()];
    argv.extend(args.iter().map(OsString::from));
    let out = run(node, &argv, Some(root), None, DIAG_TIMEOUT, &[]).await?;
    if out.truncated {
        return Err("Çıktı çok büyük".into());
    }
    Ok(out)
}

async fn run_tsc(root: &Path, node: Option<&Path>) -> Value {
    let started = Instant::now();
    let skip = |r: &str| tool_report("tsc", started, "skipped", Some(r.into()), Vec::new());
    if !root.join("tsconfig.json").is_file() {
        return skip("tsconfig.json bulunamadı.");
    }
    let Some(script) = local_bin(root, "typescript", "tsc") else {
        return skip("TypeScript bu projede kurulu değil (node_modules/typescript).");
    };
    let Some(node) = node else {
        return skip("Node.js bulunamadı.");
    };
    match run_node(node, &script, &["--noEmit", "--pretty", "false"], root).await {
        Err(e) => tool_report("tsc", started, "failed", Some(e), Vec::new()),
        Ok(o) => {
            let d = parse_tsc(&String::from_utf8_lossy(&o.out), root);
            let err = String::from_utf8_lossy(&o.err).trim().to_owned();
            if d.is_empty() && !err.is_empty() {
                let r: String = err.chars().take(500).collect();
                return tool_report("tsc", started, "failed", Some(r), Vec::new());
            }
            tool_report("tsc", started, "ok", None, d)
        }
    }
}

const ESLINT_CONFIGS: &[&str] = &[
    "eslint.config.js",
    "eslint.config.mjs",
    "eslint.config.cjs",
    "eslint.config.ts",
    "eslint.config.mts",
    "eslint.config.cts",
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.json",
    ".eslintrc.yml",
    ".eslintrc.yaml",
];

fn has_eslint_config(root: &Path) -> bool {
    ESLINT_CONFIGS.iter().any(|n| root.join(n).is_file())
        || std::fs::read(root.join("package.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some_and(|p| p.get("eslintConfig").is_some())
}

async fn run_eslint(root: &Path, node: Option<&Path>) -> Value {
    let started = Instant::now();
    let skip = |r: &str| tool_report("eslint", started, "skipped", Some(r.into()), Vec::new());
    if !has_eslint_config(root) {
        return skip("ESLint yapılandırması bulunamadı.");
    }
    let Some(script) = local_bin(root, "eslint", "eslint") else {
        return skip("ESLint bu projede kurulu değil (node_modules/eslint).");
    };
    let Some(node) = node else {
        return skip("Node.js bulunamadı.");
    };
    match run_node(node, &script, &["--format", "json", "."], root).await {
        Err(e) => tool_report("eslint", started, "failed", Some(e), Vec::new()),
        Ok(o) => match parse_eslint(&String::from_utf8_lossy(&o.out), root) {
            Ok(d) => tool_report("eslint", started, "ok", None, d),
            Err(e) => {
                let err: String = String::from_utf8_lossy(&o.err)
                    .trim()
                    .chars()
                    .take(500)
                    .collect();
                let reason = if err.is_empty() { e } else { err };
                tool_report("eslint", started, "failed", Some(reason), Vec::new())
            }
        },
    }
}

async fn run_diagnostics(state: &AppState) -> Res {
    let root = workspace(state)?;
    let node = which("node");
    let (tsc, eslint) = tokio::join!(
        run_tsc(&root, node.as_deref()),
        run_eslint(&root, node.as_deref())
    );
    Ok(json!({ "tsc": tsc, "eslint": eslint }))
}

// =============================================================================================
// g13: sistem istemi katmanları
// =============================================================================================

const MAX_LAYER_CHARS: usize = 16_000;
/// Proje katmanı: Kor'a özgü proje talimat dosyası (`kor_compat` güvenilen klasörde isteme ekler).
const PROJECT_PROMPT_FILE: &str = "KOR.md";

fn check_layer(text: &str) -> Result<(), String> {
    if text.chars().count() > MAX_LAYER_CHARS {
        // Konumsal argüman: arayüzün çeviri kalıbı `{0}` yer tutucusuyla eşleşir.
        #[allow(clippy::uninlined_format_args)]
        return Err(format!(
            "Katman en fazla {} karakter olabilir",
            MAX_LAYER_CHARS
        ));
    }
    Ok(())
}

fn session_prompt_file(id: &str) -> Result<PathBuf, String> {
    Ok(own_file("session-prompts")?.join(format!("{}.md", valid_session(id)?)))
}

fn read_trimmed(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

/// Oturum katmanı sistem istemi bloğu; `chat.rs` her turda ekler.
pub(crate) fn session_prompt_block(id: &str) -> Option<String> {
    let text = read_trimmed(&session_prompt_file(id).ok()?)?;
    let clipped: String = text.chars().take(MAX_LAYER_CHARS).collect();
    Some(format!(
        "# SESSION INSTRUCTIONS\nThe user set these instructions for this chat only; they take precedence over every layer above, including the project layer.\n{clipped}"
    ))
}

/// Masaüstü sohbetine giden global istem: temel istem (ortam, AGENTS.md/KOR.md/CLAUDE.md, hafıza) ve
/// `chat.rs::apply_desktop_prefs` eklemeleri (çıktı stili, kullanıcı talimatı, proje haritası).
fn global_prompt(root: &Path, m: &meta::Meta) -> String {
    let compat = kor_runtime::load_compat(root);
    let shell = kor_tools::Shell::detect();
    let mut p = kor_agent::prompt::build(
        root,
        shell.name,
        None,
        &compat,
        kor_agent::prompt::Surface::Desktop,
    );
    if kor_agent::memory::auto_memory_enabled(None)
        && let Some(lay) = kor_runtime::memory_layout(root)
    {
        let raw = kor_agent::memory::read_entrypoint(&lay).unwrap_or_default();
        p.push_str(&kor_agent::memory::prompt_section(&raw, &lay.auto_dir));
    }
    let mut extra = String::new();
    if let Some(st) = meta::style_prompt(&m.output_style) {
        extra.push_str(st);
        extra.push('\n');
    }
    if let Some(u) = m.system_prompt.as_deref().filter(|u| !u.trim().is_empty()) {
        extra.push_str("## User instructions\n");
        extra.push_str(u.trim());
        extra.push('\n');
    }
    if let Some(map) = crate::api_files::project_map_text(root) {
        extra.push_str("## Project map\n");
        extra.push_str(&map);
        extra.push('\n');
    }
    if !extra.is_empty() {
        p.push_str("\n\n");
        p.push_str(&extra);
    }
    p
}

fn get_layers(args: &[Value], state: &AppState) -> Res {
    let id = valid_session(s(args, 0, "g13.getLayers")?)?;
    let root = root(state)?;
    let m = meta::read()?;
    let global = global_prompt(&root, &m);
    let project = std::fs::read_to_string(root.join(PROJECT_PROMPT_FILE)).unwrap_or_default();
    let session = session_prompt_file(id)
        .ok()
        .and_then(|p| read_trimmed(&p))
        .unwrap_or_default();
    let mut preview = global.clone();
    if let Some(block) = session_prompt_block(id) {
        preview.push_str("\n\n");
        preview.push_str(&block);
    }
    Ok(json!({
        "root": root.to_string_lossy(),
        "global": { "custom": m.system_prompt.as_deref().is_some_and(|p| !p.trim().is_empty()), "chars": global.chars().count() },
        "project": { "file": PROJECT_PROMPT_FILE, "text": project, "trusted": kor_runtime::is_trusted(&root) },
        "session": { "text": session },
        "preview": preview,
    }))
}

fn set_project_prompt(args: &[Value], state: &AppState) -> Res {
    valid_session(s(args, 0, "g13.setProjectPrompt")?)?;
    let text = s(args, 1, "g13.setProjectPrompt")?;
    check_layer(text)?;
    let path = root(state)?.join(PROJECT_PROMPT_FILE);
    if text.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Silinemedi: {e}")),
        }
    } else {
        atomic_write(&path, text.as_bytes())?;
    }
    Ok(Value::Null)
}

fn set_session_prompt(args: &[Value]) -> Res {
    let id = valid_session(s(args, 0, "g13.setSessionPrompt")?)?;
    let text = s(args, 1, "g13.setSessionPrompt")?;
    check_layer(text)?;
    let path = session_prompt_file(id)?;
    if text.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        atomic_write(&path, text.as_bytes())?;
    }
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// .env yöneticisi: değerler yalnızca `envReveal` ile, tek tek döner; günlüğe/hataya yazılmaz.
// ---------------------------------------------------------------------------------------------

const MAX_ENV_BYTES: usize = 256_000;
const MAX_ENV_VALUE_CHARS: usize = 10_000;
const ENV_TEMPLATE_FILES: &[&str] = &[".env.example", ".env.template", ".env.sample", ".env.dist"];
const ACTUAL_ENV_FILES: &[&str] = &[".env", ".env.local"];

struct EnvEntry {
    key: String,
    value: String,
    line: usize,
}

fn env_lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

fn unquote(raw: &str) -> String {
    let v = raw.trim_start();
    if let Some(rest) = v.strip_prefix('"') {
        let mut out = String::new();
        let mut it = rest.chars();
        while let Some(c) = it.next() {
            match c {
                '"' => return out,
                '\\' => match it.next() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => out.push('\\'),
                },
                o => out.push(o),
            }
        }
        return out; // Kapanmayan tırnak: satırın kalanı değerdir.
    }
    if let Some(rest) = v.strip_prefix('\'') {
        return rest.split('\'').next().unwrap_or("").to_owned();
    }
    // Tırnaksız değerde boşlukla başlayan # satır içi yorumdur.
    re!(r"\s+#.*$").replace(v, "").trim().to_owned()
}

fn parse_env(text: &str) -> Vec<EnvEntry> {
    let line_re = re!(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_.-]*)\s*=\s?(.*)$");
    env_lines(text)
        .into_iter()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with('#'))
        .filter_map(|(i, l)| {
            let c = line_re.captures(l)?;
            Some(EnvEntry {
                key: c[1].to_owned(),
                value: unquote(&c[2]),
                line: i,
            })
        })
        .collect()
}

fn serialize_env_value(v: &str) -> String {
    if re!(r"^[A-Za-z0-9_.,:/@+%=-]*$").is_match(v) {
        return v.to_owned();
    }
    let esc = v
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{esc}\"")
}

fn check_env_key(key: &str) -> Result<(), String> {
    if re!(r"^[A-Za-z_][A-Za-z0-9_.-]{0,127}$").is_match(key) {
        Ok(())
    } else {
        Err("Geçersiz değişken adı (harf veya _ ile başlamalı; harf, rakam, _ . -)".into())
    }
}

fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Anahtarın ilk satırını günceller, sonraki tekrarlarını siler; yoksa sona ekler. Diğer satırlar korunur.
fn set_env_value(text: &str, key: &str, value: &str) -> Result<String, String> {
    check_env_key(key)?;
    if value.chars().count() > MAX_ENV_VALUE_CHARS {
        #[allow(clippy::uninlined_format_args)]
        return Err(format!(
            "Değer en fazla {} karakter olabilir",
            MAX_ENV_VALUE_CHARS
        ));
    }
    let entry = format!("{key}={}", serialize_env_value(value));
    let mut lines: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        env_lines(text).into_iter().map(str::to_owned).collect()
    };
    let hits: Vec<usize> = parse_env(text)
        .into_iter()
        .filter(|e| e.key == key)
        .map(|e| e.line)
        .collect();
    let Some((&first, rest)) = hits.split_first() else {
        if lines.last().is_some_and(String::is_empty) {
            let n = lines.len();
            lines[n - 1] = entry;
            lines.push(String::new());
        } else {
            lines.push(entry);
        }
        return Ok(lines.join(eol(text)));
    };
    let out: Vec<String> = lines
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !rest.contains(i))
        .map(|(i, l)| if i == first { entry.clone() } else { l })
        .collect();
    Ok(out.join(eol(text)))
}

fn remove_env_key(text: &str, key: &str) -> Result<String, String> {
    check_env_key(key)?;
    let hits: HashSet<usize> = parse_env(text)
        .into_iter()
        .filter(|e| e.key == key)
        .map(|e| e.line)
        .collect();
    let kept: Vec<&str> = env_lines(text)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !hits.contains(i))
        .map(|(_, l)| l)
        .collect();
    Ok(kept.join(eol(text)))
}

/// .env'den şablon: her değer silinir; yorumlar, boş satırlar ve sıra korunur.
fn template_from_env(text: &str) -> String {
    let keyed: HashMap<usize, String> = parse_env(text)
        .into_iter()
        .map(|e| (e.line, e.key))
        .collect();
    let export = re!(r"^\s*export\s");
    env_lines(text)
        .into_iter()
        .enumerate()
        .map(|(i, l)| match keyed.get(&i) {
            Some(k) => format!("{}{k}=", if export.is_match(l) { "export " } else { "" }),
            None => l.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(eol(text))
}

/// Yalnızca kökteki `.env`, `.env.local`, `.env.example` ... (alt klasör ve yol ayracı yok).
fn env_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !re!(r"^\.env(\.[A-Za-z0-9_-]{1,40})*$").is_match(name) {
        return Err("Yalnızca kök klasördeki .env dosyaları yönetilebilir".into());
    }
    Ok(root.join(name))
}

fn read_env(root: &Path, name: &str) -> Result<String, String> {
    let p = env_path(root, name)?;
    match std::fs::metadata(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e.to_string()),
        Ok(m) if !m.is_file() => return Err(format!("{name} bir dosya değil")),
        Ok(m) if m.len() as usize > MAX_ENV_BYTES => {
            return Err(format!("{name} çok büyük (sınır 256 KB)"));
        }
        Ok(_) => {}
    }
    std::fs::read_to_string(&p).map_err(|e| format!("{name} okunamadı: {e}"))
}

fn write_env(root: &Path, name: &str, text: &str) -> Result<(), String> {
    let p = env_path(root, name)?;
    if text.len() > MAX_ENV_BYTES {
        return Err(format!("{name} çok büyük (sınır 256 KB)"));
    }
    atomic_write(&p, text.as_bytes())
}

fn list_env_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = std::fs::read_dir(root)
        .map_err(|e| format!("Klasör okunamadı: {e}"))?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| env_path(Path::new(""), n).is_ok())
        .collect();
    out.sort();
    Ok(out)
}

fn env_files(state: &AppState) -> Res {
    Ok(json!(list_env_files(&workspace(state)?)?))
}

fn env_keys(args: &[Value], state: &AppState) -> Res {
    let text = read_env(&workspace(state)?, s(args, 0, "g13.envKeys")?)?;
    let mut seen = HashSet::new();
    Ok(json!(
        parse_env(&text)
            .into_iter()
            .filter(|e| seen.insert(e.key.clone()))
            .map(|e| json!({ "key": e.key, "empty": e.value.is_empty() }))
            .collect::<Vec<_>>()
    ))
}

fn env_reveal(args: &[Value], state: &AppState) -> Res {
    let key = s(args, 1, "g13.envReveal")?;
    check_env_key(key)?;
    let text = read_env(&workspace(state)?, s(args, 0, "g13.envReveal")?)?;
    parse_env(&text)
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| json!(e.value))
        .ok_or_else(|| "Değişken bulunamadı".into())
}

fn env_set(args: &[Value], state: &AppState) -> Res {
    let root = workspace(state)?;
    let file = s(args, 0, "g13.envSet")?;
    let text = read_env(&root, file)?;
    let next = set_env_value(&text, s(args, 1, "g13.envSet")?, s(args, 2, "g13.envSet")?)?;
    write_env(&root, file, &next)?;
    Ok(Value::Null)
}

fn env_remove(args: &[Value], state: &AppState) -> Res {
    let root = workspace(state)?;
    let file = s(args, 0, "g13.envRemove")?;
    let text = read_env(&root, file)?;
    write_env(
        &root,
        file,
        &remove_env_key(&text, s(args, 1, "g13.envRemove")?)?,
    )?;
    Ok(Value::Null)
}

/// Yalnızca ADLAR döner; değer hiçbir koşulda rapora girmez.
fn env_template_report(state: &AppState) -> Res {
    let root = workspace(state)?;
    let files = list_env_files(&root)?;
    let Some(template) = ENV_TEMPLATE_FILES
        .iter()
        .find(|t| files.iter().any(|f| f == *t))
    else {
        return Ok(json!({ "template": null, "missing": [], "empty": [], "extra": [] }));
    };
    let mut wanted: Vec<String> = Vec::new();
    for e in parse_env(&read_env(&root, template)?) {
        if !wanted.contains(&e.key) {
            wanted.push(e.key);
        }
    }
    let mut actual: Vec<(String, String)> = Vec::new();
    for name in ACTUAL_ENV_FILES
        .iter()
        .filter(|n| files.iter().any(|f| f == *n))
    {
        for e in parse_env(&read_env(&root, name)?) {
            match actual.iter_mut().find(|(k, _)| *k == e.key) {
                Some(slot) => slot.1 = e.value,
                None => actual.push((e.key, e.value)),
            }
        }
    }
    let get = |k: &str| actual.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    Ok(json!({
        "template": template,
        "missing": wanted.iter().filter(|k| get(k).is_none()).collect::<Vec<_>>(),
        "empty": wanted.iter().filter(|k| get(k) == Some("")).collect::<Vec<_>>(),
        "extra": actual.iter().map(|(k, _)| k).filter(|k| !wanted.contains(k)).collect::<Vec<_>>(),
    }))
}

fn env_create_template(state: &AppState) -> Res {
    let root = workspace(state)?;
    let target = ENV_TEMPLATE_FILES[0];
    if root.join(target).exists() {
        return Err(format!("{target} zaten var"));
    }
    if !root.join(".env").is_file() {
        return Err(".env bulunamadı".into());
    }
    let text = read_env(&root, ".env")?;
    write_env(&root, target, &template_from_env(&text))?;
    Ok(json!(target))
}

// ---------------------------------------------------------------------------------------------
// Doküman kaynakları (yalnızca kullanıcı dosyası) ve bağlam dışa aktarma
// ---------------------------------------------------------------------------------------------

const G13_FILE: &str = "g13.json";

fn get_doc_sources() -> Res {
    with_json(G13_FILE, |v| {
        Ok(json!(
            v["docSources"].as_array().cloned().unwrap_or_default()
        ))
    })
}

fn set_doc_sources(args: &[Value]) -> Res {
    let list = args
        .first()
        .and_then(Value::as_array)
        .ok_or("`g13.setDocSources`: 1. bağımsız değişken liste olmalı")?;
    if list.len() > 50 {
        return Err("En fazla 50 kaynak eklenebilir".into());
    }
    let url_re = re!(r"^https?://[^\s/?#]+[^\s?#]*$");
    let mut out: Vec<String> = Vec::new();
    for v in list {
        let src = v.as_str().map(str::trim).unwrap_or("");
        if src.len() > 500 || !url_re.is_match(src) {
            return Err("Kaynak http(s) adresi olmalı (sorgu ve # olmadan)".into());
        }
        if !out.iter().any(|o| o == src) {
            out.push(src.to_owned());
        }
    }
    with_json(G13_FILE, |v| {
        if !v.is_object() {
            *v = json!({});
        }
        v["docSources"] = json!(out);
        Ok(Value::Null)
    })
}

/// Bilinen anahtar biçimleri ve `anahtar=değer` atamaları maskelenir.
fn mask_secrets(text: &str) -> String {
    let tokens = re!(
        r"\b(sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,})"
    );
    let assigns = re!(r"(?i)\b(api[_-]?key|token|secret|password|passwd)(\s*[:=]\s*)\S+");
    let t = tokens.replace_all(text, "[MASKED]");
    assigns.replace_all(&t, "$1$2[MASKED]").into_owned()
}

fn clip(text: &str, max: usize) -> String {
    let mut t: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        t.push('…');
    }
    t
}

fn export_text(id: &str, state: &AppState, format: &str) -> Result<String, String> {
    if !matches!(format, "markdown" | "json") {
        return Err("Geçersiz biçim".into());
    }
    let messages = session_messages(id)?.ok_or("Oturum bulunamadı")?;
    let root = root(state)?;
    let users: Vec<String> = messages.iter().filter_map(user_text).collect();
    let summary = messages
        .iter()
        .rev()
        .filter(|m| m.role == Role::Assistant)
        .find_map(|m| {
            m.parts.iter().rev().find_map(|p| match p {
                ContentPart::Text { text } if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();
    let first = mask_secrets(&clip(users.first().map_or("", String::as_str), 8000));
    let last = mask_secrets(&clip(users.last().map_or("", String::as_str), 8000));
    let summary = mask_secrets(&clip(&summary, 6000));
    let files = touched_files(&messages, &root);
    let model = meta::read()
        .ok()
        .and_then(|m| m.sessions.get(id).and_then(|s| s.model.clone()))
        .unwrap_or_default();
    if format == "json" {
        let v = json!({
            "version": 1, "sessionId": id, "root": root.to_string_lossy(), "model": model,
            "firstPrompt": first, "lastRequest": last, "summary": summary, "files": files,
        });
        return serde_json::to_string_pretty(&v).map_err(|e| e.to_string());
    }
    let mut md = format!(
        "# Kor bağlam paketi\n\n- Oturum: {id}\n- Klasör: {}\n- Model: {model}\n\n## İlk istem\n\n{first}\n\n## Son istek\n\n{last}\n\n## Özet (son yanıt)\n\n{summary}\n\n## Değişen dosyalar\n\n",
        root.display()
    );
    if files.is_empty() {
        md.push_str("(yok)\n");
    }
    for f in &files {
        md.push_str(&format!("- {f}\n"));
    }
    Ok(md)
}

fn build_export(args: &[Value], state: &AppState) -> Res {
    let id = valid_session(s(args, 0, "g13.buildExport")?)?;
    Ok(json!(export_text(
        id,
        state,
        s(args, 1, "g13.buildExport")?
    )?))
}

async fn save_export(args: &[Value], app: &AppHandle, state: &AppState) -> Res {
    let id = valid_session(s(args, 0, "g13.saveExport")?)?;
    let format = s(args, 1, "g13.saveExport")?;
    let text = export_text(id, state, format)?;
    let ext = if format == "json" { "json" } else { "md" };
    let Some(path) = save_dialog(app, None, &format!("kor-context-{id}.{ext}")).await else {
        return Ok(Value::Null);
    };
    std::fs::write(&path, text).map_err(|e| format!("Kaydedilemedi: {e}"))?;
    Ok(json!(path.to_string_lossy()))
}

// =============================================================================================
// g30: otomatik hafıza onay kuyruğu
// =============================================================================================

const PENDING_FILE: &str = "memory-pending.json";

fn pending_items(v: &Value) -> Vec<Value> {
    v["items"].as_array().cloned().unwrap_or_default()
}

/// shortcut: Kor tur sonunda aday çıkarmaz (ajan `remember` aracıyla doğrudan yazar), bu kuyruğa yazan yok;
/// başka bir süreç `memory-pending.json`'a aday koyarsa burada çözülür.
fn pending_list() -> Res {
    with_json(PENDING_FILE, |v| Ok(json!(pending_items(v))))
}

fn pending_resolve(args: &[Value], state: &AppState) -> Res {
    let id = s(args, 0, "g30.pendingResolve")?;
    let action = s(args, 1, "g30.pendingResolve")?;
    if !re!(r"^[A-Za-z0-9_-]{1,40}$").is_match(id) || !matches!(action, "save" | "skip") {
        return Err("Geçersiz istek".into());
    }
    let edited = opt_s(args, 2);
    let root = root(state)?;
    with_json(PENDING_FILE, |v| {
        let mut items = pending_items(v);
        let Some(pos) = items.iter().position(|i| i["id"] == id) else {
            return Ok(json!(items));
        };
        if action == "save" {
            let raw = edited.or_else(|| items[pos]["text"].as_str()).unwrap_or("");
            let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.chars().count() > 1000 {
                return Err("Hafıza kaydı en fazla 1000 karakter olabilir".into());
            }
            if !kor_tools::secret_labels(&text).is_empty() {
                return Err("Metin gizli bilgi içeriyor; kaydedilmedi".into());
            }
            if !text.is_empty() {
                let lay = kor_runtime::memory_layout(&root).ok_or("Hafıza klasörü bulunamadı")?;
                let title: String = text.chars().take(60).collect();
                kor_agent::memory::save_memory_as(
                    &lay,
                    &format!("auto-{id}.md"),
                    &title,
                    "user",
                    &text,
                )
                .map_err(|e| format!("Hafızaya yazılamadı: {e}"))?;
            }
        }
        items.remove(pos);
        *v = json!({ "version": 1, "items": items });
        Ok(json!(items))
    })
}

fn set_auto_memory(args: &[Value]) -> Res {
    let on = args
        .first()
        .and_then(Value::as_bool)
        .ok_or("`g30.setAutoMemory`: 1. bağımsız değişken true/false olmalı")?;
    if on == kor_agent::memory::auto_memory_enabled(None) {
        return Ok(Value::Null);
    }
    // Kor'da bu kapı yalnızca ortam değişkeninden okunur (`auto_memory_enabled(None)`); ayar dosyası anahtarı yok.
    Err("Kor'da otomatik hafıza KOR_DISABLE_AUTO_MEMORY ortam değişkeniyle açılıp kapatılır".into())
}

// =============================================================================================
// g14: yerel gözlem (sıfır telemetri)
// =============================================================================================

const G14_FILE: &str = "g14.json";

fn obs_store<T>(f: impl FnOnce(&mut Value) -> Result<T, String>) -> Result<T, String> {
    with_json(G14_FILE, |v| {
        if !v.is_object() {
            *v = json!({});
        }
        f(v)
    })
}

fn obs_settings(store: &Value) -> Value {
    let mut out = json!({
        "traceExport": false,
        "otlpEndpoint": "",
        "prometheus": false,
        "prometheusPort": 9464,
        "slowToolMs": 30_000,
        "turnBudgetMs": 600_000,
        "memoryBudgetMb": 2048,
        "alerts": true,
    });
    if let Some(saved) = store["settings"].as_object() {
        for (k, v) in saved {
            if out
                .get(k)
                .is_some_and(|d| std::mem::discriminant(d) == std::mem::discriminant(v))
            {
                out[k] = v.clone();
            }
        }
    }
    out
}

fn set_obs_settings(args: &[Value]) -> Res {
    let patch = args
        .first()
        .and_then(Value::as_object)
        .ok_or("`g14.setSettings`: 1. bağımsız değişken nesne olmalı")?;
    let ranges: &[(&str, u64, u64)] = &[
        ("prometheusPort", 1024, 65_535),
        ("slowToolMs", 500, 3_600_000),
        ("turnBudgetMs", 5_000, 86_400_000),
        ("memoryBudgetMb", 128, 65_536),
    ];
    let mut clean = Map::new();
    for (k, v) in patch {
        match k.as_str() {
            "otlpEndpoint" => {
                let e = v.as_str().ok_or(format!("{k} geçersiz"))?.trim();
                // SECURITY: iz verisi makineden çıkmaz; yalnızca loopback toplayıcıya gider.
                if !e.is_empty() && kor_runtime::obs::check_endpoint(e).is_err() {
                    return Err(
                        "OTLP adresi yalnızca 127.0.0.1, localhost veya [::1] olabilir".into(),
                    );
                }
                clean.insert(k.clone(), json!(e));
            }
            "alerts" | "traceExport" | "prometheus" => {
                clean.insert(
                    k.clone(),
                    json!(v.as_bool().ok_or(format!("{k} geçersiz"))?),
                );
            }
            _ => {
                let (_, lo, hi) = ranges
                    .iter()
                    .find(|(n, _, _)| n == k)
                    .ok_or(format!("{k} geçersiz"))?;
                match v.as_u64() {
                    Some(n) if (*lo..=*hi).contains(&n) => {
                        clean.insert(k.clone(), json!(n));
                    }
                    _ => return Err(format!("{k} geçersiz")),
                }
            }
        }
    }
    let s = obs_store(|v| {
        let mut saved = v["settings"].as_object().cloned().unwrap_or_default();
        saved.extend(clean);
        v["settings"] = Value::Object(saved);
        Ok(obs_settings(v))
    })?;
    apply_prometheus(&s);
    Ok(s)
}

/// Starts, moves or stops `/metrics` to match the settings; a bind error shows in `exportStatus`.
fn apply_prometheus(s: &Value) {
    let port = (s["prometheus"].as_bool() == Some(true))
        .then(|| {
            s["prometheusPort"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
        })
        .flatten();
    let _ = kor_runtime::obs::set_prometheus(port);
}

fn ratings_of(store: &Value) -> Map<String, Value> {
    store["ratings"].as_object().cloned().unwrap_or_default()
}

/// Son kullanıcı turunun kimliği: `<oturum>.<tur sayısı>` (Kor tur izi tutmaz; tur sayısı kararlıdır).
fn last_trace(id: &str) -> Option<String> {
    let n = session_messages(id)
        .ok()??
        .iter()
        .filter_map(user_text)
        .count();
    (n > 0).then(|| format!("{id}.{n}"))
}

fn session_model(id: &str) -> String {
    meta::read()
        .ok()
        .and_then(|m| m.sessions.get(id).and_then(|s| s.model.clone()))
        .unwrap_or_default()
}

fn rate(args: &[Value]) -> Res {
    let trace = s(args, 0, "g14.rate")?;
    let Some(c) = re!(r"^([A-Za-z0-9-]{1,128})\.\d{1,7}$").captures(trace) else {
        return Err("Geçersiz iz kimliği".into());
    };
    let rating = args.get(1).and_then(Value::as_i64).unwrap_or(9);
    if ![1, -1, 0].contains(&rating) {
        return Err("Geçersiz değerlendirme".into());
    }
    let model = session_model(&c[1]);
    obs_store(|v| {
        let mut r = ratings_of(v);
        if rating == 0 {
            r.remove(trace);
        } else {
            r.insert(
                trace.to_owned(),
                json!({ "rating": rating, "model": model, "at": meta::now_ms() }),
            );
        }
        v["ratings"] = Value::Object(r);
        Ok(Value::Null)
    })
}

fn get_tags(args: &[Value]) -> Res {
    let id = valid_session(s(args, 0, "g14.getTags")?)?;
    obs_store(|v| Ok(json!(v["tags"][id].as_array().cloned().unwrap_or_default())))
}

fn set_tags(args: &[Value]) -> Res {
    let id = valid_session(s(args, 0, "g14.setTags")?)?;
    let list = args
        .get(1)
        .and_then(Value::as_array)
        .ok_or("`g14.setTags`: 2. bağımsız değişken liste olmalı")?;
    let mut tags: Vec<String> = Vec::new();
    for t in list {
        let t = t.as_str().map(str::trim).unwrap_or("");
        if t.is_empty()
            || t.chars().count() > 40
            || !t.chars().all(|c| c.is_alphanumeric() || "_-.:".contains(c))
        {
            return Err("Etiket 1-40 karakter olmalı (harf, rakam, _ - . :)".into());
        }
        if !tags.iter().any(|x| x == t) {
            tags.push(t.to_owned());
        }
    }
    if tags.len() > 20 {
        return Err("En fazla 20 etiket eklenebilir".into());
    }
    obs_store(|v| {
        let mut all = v["tags"].as_object().cloned().unwrap_or_default();
        if tags.is_empty() {
            all.remove(id);
        } else {
            all.insert(id.to_owned(), json!(tags));
        }
        v["tags"] = Value::Object(all);
        Ok(Value::Null)
    })
}

fn status(args: &[Value]) -> Res {
    let last = match opt_s(args, 0).filter(|id| kor_runtime::valid_session_id(id)) {
        Some(id) => match last_trace(id) {
            Some(trace) => {
                let rating =
                    obs_store(|v| Ok(ratings_of(v).get(&trace).map(|r| r["rating"].clone())))?;
                json!({ "traceId": trace, "model": session_model(id), "rating": rating })
            }
            None => Value::Null,
        },
        None => Value::Null,
    };
    Ok(json!({ "online": true, "provider": provider_state(), "lastTurn": last }))
}

// ---------------------------------------------------------------------------------------------
// Sağlık: yerel defter ve son etkin denetim
// ---------------------------------------------------------------------------------------------

static LAST_PROBE: Mutex<Option<Value>> = Mutex::new(None);

fn last_probe() -> Option<Value> {
    LAST_PROBE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn provider_state() -> &'static str {
    match last_probe() {
        Some(p) if p["ok"] == true => "ok",
        Some(_) => "down",
        None => "unknown",
    }
}

/// Etkin modelin sağlayıcı adresi (`base_url`).
fn provider_url(state: &AppState) -> Result<String, String> {
    let model = match state.model() {
        Some(m) => m,
        None => {
            let cwd = state.root().unwrap_or_default();
            crate::bridge::load_config(&cwd)?.config.agent.model
        }
    };
    let provider = model.split_once('/').map_or(model.as_str(), |(p, _)| p);
    let l = kor_i18n::Localizer::new(kor_i18n::Lang::En);
    let reg = kor_runtime::registry(&l, &mut Vec::new())?;
    let url = reg
        .get(provider)
        .map(|m| m.base_url.clone())
        .ok_or("Sağlayıcı bulunamadı")?;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Sağlayıcı adresi geçersiz".into());
    }
    Ok(url)
}

/// Bu sürecin bellek kullanımı (MB); ölçülemezse 0.
async fn own_memory_mb() -> f64 {
    let pid = std::process::id().to_string();
    let filter = format!("PID eq {pid}");
    let (prog, argv) = if cfg!(windows) {
        (
            "tasklist",
            os(&["/FI", filter.as_str(), "/FO", "CSV", "/NH"]),
        )
    } else {
        ("ps", os(&["-o", "rss=", "-p", pid.as_str()]))
    };
    let Some(p) = which(prog) else { return 0.0 };
    let Ok(o) = run(&p, &argv, None, None, Duration::from_secs(5), &[]).await else {
        return 0.0;
    };
    let text = String::from_utf8_lossy(&o.out);
    // tasklist: son sütun "123.456 K" (yerel ayraçlı); ps: KB.
    let field = if cfg!(windows) {
        text.trim().rsplit("\",\"").next().unwrap_or("")
    } else {
        text.trim()
    };
    let kb: String = field.chars().filter(char::is_ascii_digit).collect();
    kb.parse::<f64>()
        .map_or(0.0, |k| (k / 1024.0 * 10.0).round() / 10.0)
}

async fn health(state: &AppState) -> Result<Value, String> {
    let m = meta::read()?;
    let now = meta::now_ms();
    let host = provider_url(state)
        .ok()
        .and_then(|u| reqwest::Url::parse(&u).ok())
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let budget = obs_store(|v| Ok(obs_settings(v)))?["memoryBudgetMb"].clone();
    Ok(json!({
        // shortcut: işletim sisteminin ağ durumu sorulmuyor (istek atmadan bilinmiyor); çevrimiçi varsayılır.
        "online": true,
        "provider": {
            "state": provider_state(),
            "host": host,
            "lastCallAt": m.usage.last().map(|u| u.at),
            "lastStatus": null,
            "errorRate": 0,
            "recentCalls": m.usage.iter().filter(|u| u.at + 600_000 >= now).count(),
            "p95Ms": null,
        },
        "probe": last_probe(),
        "memoryMb": own_memory_mb().await,
        "memoryBudgetMb": budget,
    }))
}

async fn probe(state: &AppState) -> Res {
    let url = provider_url(state)?;
    let client = kor_core::net::client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("kor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    // Kimlik bilgisi eklenmez: yalnızca erişilebilirlik.
    let res = client.get(&url).send().await;
    let latency = started.elapsed().as_millis() as u64;
    let p = match res {
        Ok(r) => {
            let code = r.status().as_u16();
            json!({ "at": meta::now_ms(), "ok": code < 500, "latencyMs": latency, "status": code, "error": null })
        }
        Err(e) => {
            let msg = if e.is_timeout() {
                "Zaman aşımı"
            } else {
                "Bağlantı kurulamadı"
            };
            json!({ "at": meta::now_ms(), "ok": false, "latencyMs": latency, "status": null, "error": msg })
        }
    };
    *LAST_PROBE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(p);
    health(state).await
}

async fn overview(state: &AppState) -> Res {
    let (settings, ratings) = obs_store(|v| {
        let r: Map<String, Value> = ratings_of(v)
            .into_iter()
            .map(|(k, r)| (k, r["rating"].clone()))
            .collect();
        Ok((obs_settings(v), r))
    })?;
    // Uygulama yeniden açıldığında açık kalan Prometheus burada başlar.
    apply_prometheus(&settings);
    let export_status = kor_runtime::obs::status();
    // Kor tur/araç/model izi (süre, hata) tutmaz: bu listeler dürüstçe boş; maliyet ve tokenlar `cost`ta.
    Ok(json!({
        "now": meta::now_ms(),
        "health": health(state).await?,
        "tools": [], "models": [], "errors": [], "slow": [], "turns": [],
        "ratings": ratings,
        "events": [], "live": [],
        "settings": settings,
        "exportStatus": export_status,
    }))
}

// ---------------------------------------------------------------------------------------------
// Maliyet: yerel kullanım defteri (`meta.usage`)
// ---------------------------------------------------------------------------------------------

fn local_day(ms: u64) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp_millis(i64::try_from(ms).unwrap_or(0))
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_default()
}

fn slice(key: &str, usd: f64, tokens: f64, calls: f64) -> Value {
    json!({ "key": key, "usd": usd, "tokens": tokens, "calls": calls })
}

/// Ay sonu tahmini: ay başından bugüne + kalan günler x son 7 gün ortalaması (eksik gün = 0).
fn forecast(daily: &BTreeMap<chrono::NaiveDate, f64>, today: chrono::NaiveDate) -> Value {
    use chrono::Datelike;
    let sum = |n: i64| -> f64 {
        (0..n)
            .map(|i| {
                daily
                    .get(&(today - chrono::Duration::days(i)))
                    .copied()
                    .unwrap_or(0.0)
            })
            .sum()
    };
    let avg7 = sum(7) / 7.0;
    let avg30 = sum(30) / 30.0;
    let next_month = if today.month() == 12 {
        chrono::NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
    };
    let days_in_month = next_month.map_or(30, |n| (n - chrono::Duration::days(1)).day());
    let mtd = sum(i64::from(today.day()));
    let left = days_in_month - today.day();
    let oldest = today - chrono::Duration::days(29);
    json!({
        "avg7": avg7, "avg30": avg30, "nextWeekUsd": avg7 * 7.0, "monthToDateUsd": mtd,
        "monthProjectionUsd": mtd + f64::from(left) * avg7, "daysLeftInMonth": left,
        "sampleDays": daily.iter().filter(|(d, u)| **u > 0.0 && **d >= oldest).count(),
    })
}

/// En az 3 değerlendirmesi olan modellerden en yüksek (Laplace) beğeni puanlı olan.
fn recommend(ratings: &Map<String, Value>) -> Value {
    let mut by: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for r in ratings.values() {
        let e = by
            .entry(r["model"].as_str().unwrap_or("").to_owned())
            .or_default();
        if r["rating"] == 1 { e.0 += 1 } else { e.1 += 1 }
    }
    let mut stats: Vec<(String, u32, u32, f64)> = by
        .into_iter()
        .filter(|(m, _)| !m.is_empty())
        .map(|(m, (l, d))| (m, l, d, f64::from(l + 1) / f64::from(l + d + 2)))
        .collect();
    stats.sort_by(|a, c| c.3.total_cmp(&a.3).then_with(|| a.0.cmp(&c.0)));
    let pick = stats.iter().find(|s| s.1 + s.2 >= 3);
    json!({
        "stats": stats.iter().map(|(m, l, d, sc)| json!({ "model": m, "likes": l, "dislikes": d, "score": sc, "avgTurnCostUsd": null })).collect::<Vec<_>>(),
        "pick": pick.map(|p| p.0.clone()),
        "reason": match pick {
            Some(p) => format!("En yüksek beğeni puanı ({}%).", (p.3 * 100.0).round()),
            None => "Öneri için en az bir modelde 3 değerlendirme gerekir.".to_owned(),
        },
    })
}

/// `prices`: model → (girdi, çıktı) USD / 1M token.
fn cost(args: &[Value], prices: &BTreeMap<String, (f64, f64)>) -> Res {
    let days = args
        .first()
        .and_then(Value::as_u64)
        .filter(|d| (1..=365).contains(d))
        .ok_or("Gün sayısı 1-365 olmalı")?;
    let m = meta::read()?;
    let now = meta::now_ms();
    let since = now.saturating_sub(days * 86_400_000);
    // (usd, token, çağrı)
    let mut by_model: BTreeMap<String, (f64, f64, f64)> = BTreeMap::new();
    let mut by_day: BTreeMap<chrono::NaiveDate, (f64, f64, f64)> = BTreeMap::new();
    let (mut usd, mut tokens, mut calls, mut unpriced) = (0.0, 0.0, 0.0, 0u64);
    for u in m.usage.iter().filter(|u| u.at >= since) {
        let t = (u.prompt + u.completion) as f64;
        let c = match prices.get(&u.model) {
            Some((i, o)) => (u.prompt as f64 * i + u.completion as f64 * o) / 1_000_000.0,
            None => {
                unpriced += 1;
                0.0
            }
        };
        for e in [
            by_model.entry(u.model.clone()).or_default(),
            by_day.entry(local_day(u.at)).or_default(),
        ] {
            e.0 += c;
            e.1 += t;
            e.2 += 1.0;
        }
        usd += c;
        tokens += t;
        calls += 1.0;
    }
    let mut models: Vec<Value> = by_model
        .iter()
        .map(|(k, (u, t, c))| slice(k, *u, *t, *c))
        .collect();
    models.sort_by(|a, c| {
        c["tokens"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["tokens"].as_f64().unwrap_or(0.0))
    });
    // shortcut: fiyat yalnızca Venice kataloğundan; diğer sağlayıcıların çağrıları "fiyatsız" sayılır.
    let attribution = json!({
        "totalUsd": usd,
        "unpricedCalls": unpriced,
        "byModel": models,
        "byTool": if calls > 0.0 { vec![slice("(yanıt)", usd, tokens, calls)] } else { vec![] },
        "byProject": if calls > 0.0 { vec![slice("(klasörsüz)", usd, tokens, calls)] } else { vec![] },
        "byTag": if calls > 0.0 { vec![slice("(etiketsiz)", usd, tokens, calls)] } else { vec![] },
        "byDay": by_day.iter().map(|(d, (u, t, c))| slice(&d.format("%Y-%m-%d").to_string(), *u, *t, *c)).collect::<Vec<_>>(),
    });
    let daily: BTreeMap<chrono::NaiveDate, f64> =
        by_day.iter().map(|(d, (u, _, _))| (*d, *u)).collect();
    let (ratings, tags) = obs_store(|v| Ok((ratings_of(v), v["tags"].clone())))?;
    Ok(json!({
        "days": days,
        "attribution": attribution,
        "forecast": forecast(&daily, chrono::Local::now().date_naive()),
        "recommendation": recommend(&ratings),
        "tags": if tags.is_object() { tags } else { json!({}) },
    }))
}

// ---------------------------------------------------------------------------------------------
// Süreçler: bu uygulamanın alt süreçleri (Windows: CIM, diğerleri: ps), kabuksuz.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ProcRow {
    pid: u32,
    ppid: u32,
    name: String,
    cmd: String,
    mem: u64,
    /// Toplam CPU süresi (ms; Windows) ya da anlık yüzde (ps).
    cpu_ms: Option<f64>,
    pcpu: Option<f64>,
}

fn parse_cim(json_text: &str) -> Vec<ProcRow> {
    let v: Value = serde_json::from_str(json_text.trim()).unwrap_or(Value::Null);
    let list = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|r| {
            let n = |k: &str| r[k].as_f64();
            Some(ProcRow {
                pid: u32::try_from(r["ProcessId"].as_u64()?).ok()?,
                ppid: r["ParentProcessId"]
                    .as_u64()
                    .and_then(|p| u32::try_from(p).ok())
                    .unwrap_or(0),
                name: r["Name"].as_str().unwrap_or("?").to_owned(),
                cmd: r["CommandLine"].as_str().unwrap_or("").to_owned(),
                mem: r["WorkingSetSize"].as_u64().unwrap_or(0),
                cpu_ms: Some(
                    (n("KernelModeTime").unwrap_or(0.0) + n("UserModeTime").unwrap_or(0.0))
                        / 10_000.0,
                ),
                pcpu: None,
            })
        })
        .collect()
}

fn parse_ps(text: &str) -> Vec<ProcRow> {
    let line = re!(r"^\s*(\d+)\s+(\d+)\s+(\d+)\s+([\d.]+)\s+(.*)$");
    text.lines()
        .filter_map(|l| {
            let c = line.captures(l)?;
            let args = c[5].to_owned();
            Some(ProcRow {
                pid: c[1].parse().ok()?,
                ppid: c[2].parse().ok()?,
                name: args
                    .split_whitespace()
                    .next()
                    .unwrap_or("?")
                    .rsplit('/')
                    .next()
                    .unwrap_or("?")
                    .to_owned(),
                cmd: args,
                mem: c[3].parse::<u64>().ok()? * 1024,
                cpu_ms: None,
                pcpu: c[4].parse().ok(),
            })
        })
        .collect()
}

/// `root` altındaki tüm süreçler (kendisi dahil), döngüye karşı korumalı.
fn subtree(rows: &[ProcRow], root: u32) -> Vec<u32> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        for r in rows {
            if r.ppid == p && r.pid != p && !out.contains(&r.pid) {
                out.push(r.pid);
            }
        }
        i += 1;
    }
    out
}

fn is_webview(r: &ProcRow) -> bool {
    let n = r.name.to_ascii_lowercase();
    n.starts_with("msedgewebview2")
        || n.starts_with("webkitwebprocess")
        || n.starts_with("webkitnetworkprocess")
}

type CpuSample = (Instant, HashMap<u32, f64>);
static CPU_PREV: Mutex<Option<CpuSample>> = Mutex::new(None);

async fn processes() -> Res {
    let at = meta::now_ms();
    let rows = if cfg!(windows) {
        let ps = which("powershell").unwrap_or_else(|| PathBuf::from("powershell"));
        let script = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CommandLine,WorkingSetSize,KernelModeTime,UserModeTime | ConvertTo-Json -Compress";
        run(
            &ps,
            &os(&["-NoProfile", "-NonInteractive", "-Command", script]),
            None,
            None,
            Duration::from_secs(20),
            &[],
        )
        .await
        .map(|o| parse_cim(&String::from_utf8_lossy(&o.out)))
    } else {
        let ps = which("ps").unwrap_or_else(|| PathBuf::from("ps"));
        run(
            &ps,
            &os(&["-A", "-o", "pid=,ppid=,rss=,pcpu=,args="]),
            None,
            None,
            Duration::from_secs(10),
            &[],
        )
        .await
        .map(|o| parse_ps(&String::from_utf8_lossy(&o.out)))
    };
    let rows = match rows {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => {
            return Ok(
                json!({ "at": at, "app": [], "groups": [], "shells": [], "error": "Süreç listesi okunamadı" }),
            );
        }
        Err(e) => {
            return Ok(json!({ "at": at, "app": [], "groups": [], "shells": [], "error": e }));
        }
    };
    let me = std::process::id();
    let cpus = std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64);
    let now = Instant::now();
    let current: HashMap<u32, f64> = rows
        .iter()
        .filter_map(|r| r.cpu_ms.map(|c| (r.pid, c)))
        .collect();
    let prev = CPU_PREV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .replace((now, current.clone()));
    let pct = |pids: &[u32]| -> Option<f64> {
        let rows_pc: Vec<f64> = pids
            .iter()
            .filter_map(|p| rows.iter().find(|r| r.pid == *p).and_then(|r| r.pcpu))
            .collect();
        if !rows_pc.is_empty() {
            return Some(rows_pc.iter().sum());
        }
        let (t, before) = prev.as_ref()?;
        let elapsed = now.duration_since(*t).as_secs_f64() * 1000.0;
        if elapsed <= 0.0 {
            return None;
        }
        let used: f64 = pids
            .iter()
            .filter_map(|p| Some(current.get(p)? - before.get(p)?))
            .filter(|d| *d >= 0.0)
            .sum();
        Some((used / (elapsed * cpus) * 1000.0).round() / 10.0)
    };
    let by_pid = |p: u32| rows.iter().find(|r| r.pid == p);
    let mb = |b: u64| (b as f64 / 1_048_576.0 * 10.0).round() / 10.0;
    let mut app = Vec::new();
    if let Some(r) = by_pid(me) {
        app.push(json!({ "pid": me, "type": "main", "cpuPercent": pct(&[me]).unwrap_or(0.0), "memMb": mb(r.mem) }));
    }
    let mut groups = Vec::new();
    for child in rows.iter().filter(|r| r.ppid == me && r.pid != me) {
        let pids = subtree(&rows, child.pid);
        if is_webview(child) {
            for p in pids {
                if let Some(r) = by_pid(p) {
                    app.push(json!({ "pid": p, "type": "webview", "cpuPercent": pct(&[p]).unwrap_or(0.0), "memMb": mb(r.mem) }));
                }
            }
            continue;
        }
        let mem: u64 = pids.iter().filter_map(|p| by_pid(*p)).map(|r| r.mem).sum();
        let cmd = mask_secrets(&clip(child.cmd.trim(), 160));
        groups.push(json!({
            "shellId": null,
            "label": if cmd.is_empty() { child.name.clone() } else { cmd },
            "rootPid": child.pid,
            "pids": pids,
            "memBytes": mem,
            "cpuPercent": pct(&subtree(&rows, child.pid)),
        }));
    }
    // shortcut: Kor'un arka plan kabuk kaydı bu modüle açık değil; kabuklar süreç grubu olarak görünür.
    Ok(json!({ "at": at, "app": app, "groups": groups, "shells": [], "error": null }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_sensitive_files() {
        assert!(valid_name("a.txt").is_ok());
        for bad in ["", "..", "a/b", "a\\b", "x:y", "CON", "nul.txt", "a."] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
        for s in [".env", ".env.local", "id_rsa", "server.pem", "a.KEY"] {
            assert!(sensitive(Path::new(s)), "{s}");
        }
        assert!(sensitive(Path::new("home/.ssh/config")));
        assert!(!sensitive(Path::new("src/main.rs")));
    }

    #[test]
    fn jail_blocks_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = kor_tools::without_verbatim(&dir.path().canonicalize().unwrap());
        assert!(jail(&root, "a/b.txt").is_ok());
        assert!(jail(&root, "../x").is_err());
        assert!(jail(&root, "").is_err());
        assert_eq!(jail_or_root(&root, "").unwrap(), root);
    }

    #[test]
    fn watch_diff_reports_added_changed_removed() {
        let prev: Snapshot = [("a".to_owned(), (1.0, 1)), ("b".to_owned(), (1.0, 1))].into();
        let next: Snapshot = [("a".to_owned(), (2.0, 1)), ("c".to_owned(), (1.0, 1))].into();
        assert_eq!(diff(&prev, &next), ["a", "b", "c"]);
        assert!(diff(&next, &next).is_empty());
    }

    #[test]
    fn cmd_shim_is_parsed_without_a_shell() {
        let text =
            "@echo off\r\n\"%~dp0..\\Code.exe\" \"%~dp0..\\resources\\app\\out\\cli.js\" %*\r\n";
        let (exe, cli) = parse_cmd_shim(text, Path::new(r"C:\VS\bin")).unwrap();
        assert!(exe.to_string_lossy().ends_with(r"..\Code.exe"));
        assert!(cli.to_string_lossy().ends_with("cli.js"));
        assert!(parse_cmd_shim("code.exe %*", Path::new("x")).is_none());
    }

    #[test]
    fn rewind_actions() {
        assert_eq!(rewind_action(None, None), None);
        assert_eq!(rewind_action(None, Some("x")), Some("delete"));
        assert_eq!(rewind_action(Some("x"), None), Some("create"));
        assert_eq!(rewind_action(Some("x"), Some("x")), None);
        assert_eq!(rewind_action(Some("x"), Some("y")), Some("restore"));
    }

    #[test]
    fn touched_files_stay_inside_the_root() {
        let root = Path::new(r"C:\proj");
        let msg = Message {
            role: Role::Assistant,
            parts: vec![
                ContentPart::ToolUse {
                    id: "1".into(),
                    name: "Edit".into(),
                    input: json!({ "file_path": "src/a.rs" }),
                },
                ContentPart::ToolUse {
                    id: "2".into(),
                    name: "Write".into(),
                    input: json!({ "path": "../x" }),
                },
                ContentPart::ToolUse {
                    id: "3".into(),
                    name: "Read".into(),
                    input: json!({ "file_path": "b.rs" }),
                },
            ],
        };
        assert_eq!(touched_files(&[msg], root), ["src/a.rs"]);
    }

    #[test]
    fn tsc_and_eslint_output_is_parsed() {
        let root = Path::new(r"C:\proj");
        let out = "src/a.ts(3,5): error TS2322: Type 'x' is not assignable.\n  Details here\nerror TS5083: Cannot read file.\n";
        let d = parse_tsc(out, root);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["file"], "src/a.ts");
        assert_eq!(d[0]["inWorkspace"], true);
        assert_eq!(d[0]["line"], 3);
        assert!(
            d[0]["message"]
                .as_str()
                .unwrap()
                .ends_with("\nDetails here")
        );
        assert_eq!(d[1]["inWorkspace"], false);

        let js = r#"noise [{"filePath":"C:\\proj\\b.js","messages":[{"ruleId":"no-undef","severity":2,"message":"x","line":1,"column":2}]}]"#;
        let e = parse_eslint(js, root).unwrap();
        assert_eq!(e[0]["file"], "b.js");
        assert_eq!(e[0]["severity"], "error");
        assert!(parse_eslint("nope", root).is_err());
    }

    #[test]
    fn env_parse_set_remove_template() {
        let text = "# c\nexport A=1\nB=\"x y\\n\" \nC='q' # z\nD=v # yorum\nA=2\n";
        let e = parse_env(text);
        let get = |k: &str| e.iter().find(|x| x.key == k).map(|x| x.value.clone());
        assert_eq!(get("A").as_deref(), Some("1"));
        assert_eq!(get("B").as_deref(), Some("x y\n"));
        assert_eq!(get("C").as_deref(), Some("q"));
        assert_eq!(get("D").as_deref(), Some("v"));

        let set = set_env_value(text, "A", "a b").unwrap();
        assert!(!set.contains("export A") && set.contains("A=\"a b\""));
        assert_eq!(set.matches("A=").count(), 1, "later duplicates are removed");
        assert_eq!(set_env_value("", "K", "v").unwrap(), "K=v");
        assert_eq!(
            set_env_value("X=1\r\n", "K", "v").unwrap(),
            "X=1\r\nK=v\r\n"
        );
        assert!(set_env_value("", "1bad", "v").is_err());

        assert!(!remove_env_key(text, "A").unwrap().contains("A="));
        let t = template_from_env("# c\nexport A=1\nB=secret\n");
        assert_eq!(t, "# c\nexport A=\nB=\n");
        assert!(env_path(Path::new("r"), ".env.local").is_ok());
        assert!(env_path(Path::new("r"), "../.env").is_err());
        assert!(env_path(Path::new("r"), "x.env").is_err());
    }

    #[test]
    fn secrets_are_masked_in_exports() {
        let t = mask_secrets("key sk-abcdefghijklmnop1234 and api_key = hunter2 ok");
        assert!(!t.contains("sk-abc") && !t.contains("hunter2"));
        assert!(t.contains("api_key = [MASKED]"));
    }

    #[test]
    fn process_tree_and_parsers() {
        let rows = parse_ps(
            "  1 0 100 0.0 /sbin/init\n 10 1 2048 1.5 /usr/bin/node server.js\n 11 10 10 0.0 sh -c x\n 12 11 10 0.0 sleep 1\n",
        );
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].name, "node");
        assert_eq!(rows[1].mem, 2048 * 1024);
        assert_eq!(subtree(&rows, 10), [10, 11, 12]);
        let cim = parse_cim(
            r#"{"ProcessId":5,"ParentProcessId":1,"Name":"a.exe","CommandLine":null,"WorkingSetSize":100,"KernelModeTime":10000,"UserModeTime":20000}"#,
        );
        assert_eq!(cim[0].cpu_ms, Some(3.0));
    }

    #[test]
    fn forecast_and_recommendation() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 2, 10).unwrap();
        let daily: BTreeMap<_, _> = [(today, 7.0)].into();
        let f = forecast(&daily, today);
        assert_eq!(f["avg7"], 1.0);
        assert_eq!(f["daysLeftInMonth"], 18);
        assert_eq!(f["sampleDays"], 1);
        let mut r = Map::new();
        for i in 0..3 {
            r.insert(format!("s.{i}"), json!({ "rating": 1, "model": "m" }));
        }
        assert_eq!(recommend(&r)["pick"], "m");
        assert!(recommend(&Map::new())["pick"].is_null());
    }

    #[test]
    fn layer_limit() {
        assert!(check_layer(&"x".repeat(MAX_LAYER_CHARS)).is_ok());
        assert!(check_layer(&"x".repeat(MAX_LAYER_CHARS + 1)).is_err());
    }
}
