//! Geliştirme araçları: denetim (`g27ListRuntimes`) ve eksikleri otomatik kurma (`g27InstallRuntime`).
//! İlk açılış ekranı gerekli araçları (node, python, php, git, rust) bununla kurar; Ayarlar > Araçlar da okur.
//! Tablo `frontend/src/core/runtimes-g27.ts` içindeki `RUNTIMES` ile aynıdır; dönen şekiller `RuntimeStatus` ve
//! `InstallTick & { id }` (olay: `kor:runtimeProgress`).
//!
//! Node ve Python her işletim sisteminde Kor'un kendi bileşeni olarak (`kor-components`, yönetici izni ve
//! PATH değişikliği olmadan) kurulur; diğer araçlar için winget (Windows) isteğe bağlı sistem kurulumudur (§24.3).

use kor_platform::NoWindow as _;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_opener::OpenerExt;

struct Spec {
    id: &'static str,
    name: &'static str,
    /// Sırayla denenir; ilk sürüm döndüren kuruludur.
    probes: &'static [(&'static str, &'static [&'static str])],
    winget_id: &'static str,
    /// winget yoksa açılan resmi kurulum sayfası. Sessiz indirme yok.
    page: &'static str,
}

#[rustfmt::skip]
const RUNTIMES: &[Spec] = &[
    Spec { id: "node", name: "Node.js", probes: &[("node", &["--version"])], winget_id: "OpenJS.NodeJS.LTS", page: "https://nodejs.org/en/download" },
    Spec { id: "python", name: "Python", probes: &[("py", &["-3", "--version"]), ("python", &["--version"]), ("python3", &["--version"])], winget_id: "Python.Python.3.12", page: "https://www.python.org/downloads/" },
    Spec { id: "rust", name: "Rust", probes: &[("rustc", &["--version"])], winget_id: "Rustlang.Rustup", page: "https://rustup.rs/" },
    Spec { id: "go", name: "Go", probes: &[("go", &["version"])], winget_id: "GoLang.Go", page: "https://go.dev/dl/" },
    Spec { id: "java", name: "Java", probes: &[("java", &["-version"])], winget_id: "Microsoft.OpenJDK.21", page: "https://learn.microsoft.com/java/openjdk/download" },
    Spec { id: "dotnet", name: ".NET", probes: &[("dotnet", &["--version"])], winget_id: "Microsoft.DotNet.SDK.8", page: "https://dotnet.microsoft.com/download" },
    Spec { id: "php", name: "PHP", probes: &[("php", &["--version"])], winget_id: "PHP.PHP.8.3", page: "https://windows.php.net/download/" },
    Spec { id: "ruby", name: "Ruby", probes: &[("ruby", &["--version"])], winget_id: "RubyInstallerTeam.Ruby.3.3", page: "https://rubyinstaller.org/" },
    Spec { id: "bun", name: "Bun", probes: &[("bun", &["--version"])], winget_id: "Oven-sh.Bun", page: "https://bun.sh/" },
    Spec { id: "deno", name: "Deno", probes: &[("deno", &["--version"])], winget_id: "DenoLand.Deno", page: "https://docs.deno.com/runtime/getting_started/installation/" },
    Spec { id: "git", name: "Git", probes: &[("git", &["--version"])], winget_id: "Git.Git", page: "https://git-scm.com/download/win" },
    Spec { id: "docker", name: "Docker", probes: &[("docker", &["--version"])], winget_id: "Docker.DockerDesktop", page: "https://docs.docker.com/desktop/setup/install/windows-install/" },
];

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const LINE_MAX: usize = 240;

static BUSY: AtomicBool = AtomicBool::new(false);
static PERCENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,3}(?:\.\d+)?)\s*%").expect("sabit desen"));
static RATIO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(b|kb|mb|gb)\s*/\s*(\d+(?:\.\d+)?)\s*(b|kb|mb|gb)")
        .expect("sabit desen")
});

/// Çıktıdaki ilk `a.b` veya `a.b.c` sürüm dizisi.
pub fn first_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // "go1.23.4" gibi harf bitişik sürümler de sayılır; rakam/nokta ortasından başlanmaz.
        if bytes[i].is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b'.'))
        {
            let start = i;
            let mut dots = 0;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || (bytes[i] == b'.' && dots < 2)) {
                if bytes[i] == b'.' {
                    dots += 1;
                }
                i += 1;
            }
            let v = text[start..i].trim_end_matches('.');
            if v.contains('.') {
                return Some(v.to_owned());
            }
        }
        i += 1;
    }
    None
}

fn hidden(cmd: &str) -> Command {
    let mut c = Command::new(cmd);
    c.no_window();
    c
}

/// Süreci zaman aşımıyla bekler; süre dolarsa öldürür.
fn wait_with_timeout(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let mut child = hidden(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text);
        // `java -version` sürümü stderr'e yazar.
        let _ = err.read_to_string(&mut text);
        text
    });
    let status = wait_with_timeout(&mut child, PROBE_TIMEOUT)?;
    let text = reader.join().ok()?;
    if status.success() {
        first_version(&text)
    } else {
        None
    }
}

fn probe(spec: &Spec) -> Option<String> {
    spec.probes
        .iter()
        .find_map(|(cmd, args)| run(cmd, args))
        .or_else(|| {
            // Sistemde yoksa Kor'un kendi bileşeni (yalnız node ve python).
            let exe = kor_components::path(component_of(spec.id)?)?;
            run(exe.to_str()?, &["--version"])
        })
}

/// Bileşen olarak kurulan araçlar.
fn component_of(id: &str) -> Option<&'static str> {
    match id {
        "node" => Some("node"),
        "python" => Some("python"),
        _ => None,
    }
}

/// Gizlilik modu: açık klasörün ayarı ya da oturumun bayrağı. `kor-components` da bununla kilitlenir.
fn privacy(app: &AppHandle) -> bool {
    let state = app.state::<crate::state::AppState>();
    let on = crate::api::root(&state)
        .ok()
        .and_then(|r| crate::bridge::load_config(&r).ok())
        .is_some_and(|c| c.config.privacy.mode)
        || kor_runtime::privacy_mode();
    kor_components::set_privacy(on);
    on
}

pub(crate) fn mb(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let v = bytes as f64 / 1_048_576.0;
    format!("{v:.1} MB")
}

/// Kor bileşenini (`python`, `node`, `pdfium`, `chromium`…) hazırlar: kuruluysa yolunu döndürür, değilse
/// `ask` ise bir kez onay ister, indirir, doğrular ve ilerlemeyi `kor:runtimeProgress` (`id` = bileşen) ile yayar.
/// Gizlilik modunda indirmez; çevrimdışı kurulum yolunu söyler.
pub(crate) async fn component(app: &AppHandle, name: &str, ask: bool) -> Result<PathBuf, String> {
    if let Some(p) = kor_components::path(name) {
        return Ok(p);
    }
    let info = kor_components::info(name).ok_or_else(|| format!("Bilinmeyen bileşen: {name}"))?;
    let Some(size) = info.size else {
        return Err(format!("{name} için bu sistemde yayımlanmış derleme yok"));
    };
    if privacy(app) {
        return Err(format!(
            "Gizlilik modu açık; Kor bileşen indirmez. İnternetsiz kurulum: kor component add {name} --from <arşiv>"
        ));
    }
    if ask {
        let msg = format!(
            "{} gerekiyor ve bu bilgisayarda yok.\n\n{} indirilip Kor'un kendi klasörüne kurulsun mu? Yönetici izni gerekmez, sisteme dokunulmaz.",
            info.about,
            mb(size)
        );
        if !crate::api::confirm(app, "Bileşen indir", &msg, "İndir").await {
            return Err(format!("{} indirilmedi", info.about));
        }
    }
    let (app2, id) = (app.clone(), name.to_owned());
    let tick = move |p: kor_components::Progress| {
        let percent = p
            .total
            .filter(|t| *t > 0)
            .map(|t| (p.done * 100 / t).min(99));
        emit(
            &app2,
            &id,
            json!({ "phase": "indiriliyor", "percent": percent, "line": format!("{} / {}", mb(p.done), mb(size)) }),
        );
    };
    match kor_components::ensure(name, &tick).await {
        Ok(p) => {
            emit(
                app,
                name,
                json!({ "phase": "bitti", "percent": 100, "line": format!("Kuruldu: {}", info.version) }),
            );
            Ok(p)
        }
        Err(e) => {
            let line: String = e.chars().take(LINE_MAX).collect();
            emit(
                app,
                name,
                json!({ "phase": "hata", "percent": null, "line": line }),
            );
            Err(e)
        }
    }
}

fn winget_present() -> bool {
    cfg!(windows) && run("winget", &["--version"]).is_some()
}

/// `RuntimeStatus[]`. Yoklamalar paralel çalışır; biri asılı kalsa bile en fazla `PROBE_TIMEOUT` bekler.
pub fn list() -> Value {
    let (versions, winget) = std::thread::scope(|s| {
        let handles: Vec<_> = RUNTIMES
            .iter()
            .map(|spec| s.spawn(move || probe(spec)))
            .collect();
        let winget = s.spawn(winget_present);
        let versions: Vec<Option<String>> = handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect();
        (versions, winget.join().unwrap_or(false))
    });
    Value::Array(
        RUNTIMES
            .iter()
            .zip(versions)
            .map(|(spec, version)| {
                json!({ "id": spec.id, "name": spec.name, "installed": version.is_some(), "version": version, "winget": winget })
            })
            .collect(),
    )
}

/// winget satırından aşama ve varsa gerçek yüzde (`InstallTick`). Yüzde uydurmaz; 100 yalnız başarıda verilir.
pub fn tick_from_line(line: &str) -> Value {
    let text = line.trim();
    let lower = text.to_lowercase();
    let phase = if lower.contains("download") || lower.contains("indir") {
        "indiriliyor"
    } else if ["verif", "doğrula", "hash"]
        .iter()
        .any(|k| lower.contains(k))
    {
        "dogrulaniyor"
    } else {
        "kuruluyor"
    };
    let short: String = text.chars().take(LINE_MAX).collect();
    let unit = |u: &str| match u.to_lowercase().as_str() {
        "kb" => 1024.0,
        "mb" => 1024.0 * 1024.0,
        "gb" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    let percent = if let Some(c) = PERCENT.captures(text) {
        c[1].parse::<f64>().ok().map(|n| n.round().clamp(0.0, 99.0))
    } else if let Some(c) = RATIO.captures(text) {
        let done = c[1].parse::<f64>().unwrap_or(0.0) * unit(&c[2]);
        let total = c[3].parse::<f64>().unwrap_or(0.0) * unit(&c[4]);
        (total > 0.0).then(|| (done / total * 100.0).round().min(99.0))
    } else {
        None
    };
    json!({ "phase": phase, "percent": percent, "line": short })
}

/// Kurulum sürerken başka kurulum başlatılamaz; bırakılınca bayrak düşer (panik dahil).
struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

fn emit(app: &AppHandle, id: &str, mut tick: Value) {
    tick["id"] = Value::String(id.to_owned());
    let _ = app.emit("kor:runtimeProgress", tick);
}

/// Node/Python: Kor bileşeni olarak kurar (her OS). Diğerleri: winget ile kurar ve ilerlemeyi
/// `kor:runtimeProgress` olarak yayar; winget yoksa resmi sayfayı açar.
/// Dönüş: `{ installed, version, opened }`. Kimlik sabit tablodan gelir; arayüzden komut satırına serbest metin geçmez.
pub fn install(app: &AppHandle, id: &str) -> Result<Value, String> {
    let spec = RUNTIMES
        .iter()
        .find(|s| s.id == id)
        .ok_or("Bilinmeyen araç")?;
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("Başka bir kurulum sürüyor".into());
    }
    let _guard = BusyGuard;
    if let Some(version) = probe(spec) {
        return Ok(json!({ "installed": true, "version": version, "opened": false }));
    }
    if let Some(name) = component_of(id) {
        // Kullanıcı kurulum düğmesine bastı: onay budur.
        tauri::async_runtime::block_on(component(app, name, false))?;
        return Ok(json!({ "installed": true, "version": probe(spec), "opened": false }));
    }
    if !winget_present() {
        app.opener()
            .open_url(spec.page, None::<&str>)
            .map_err(|e| e.to_string())?;
        return Ok(json!({ "installed": false, "version": null, "opened": true }));
    }
    emit(
        app,
        id,
        json!({ "phase": "indiriliyor", "percent": null, "line": "winget başlatıldı" }),
    );
    let mut child = hidden("winget")
        .args([
            "install",
            "--id",
            spec.winget_id,
            "--exact",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("winget başlatılamadı: {e}"))?;

    // winget ilerleme çubuğunu `\r` ile yeniden çizer; satırlar hem `\r` hem `\n` ile bölünür.
    let pump = |mut src: Box<dyn Read + Send>, app: AppHandle, id: String| {
        std::thread::spawn(move || {
            let mut last = String::new();
            let mut buf = [0u8; 4096];
            let mut pending = Vec::new();
            while let Ok(n) = src.read(&mut buf) {
                if n == 0 {
                    break;
                }
                pending.extend_from_slice(&buf[..n]);
                while let Some(pos) = pending.iter().position(|b| *b == b'\n' || *b == b'\r') {
                    let line = String::from_utf8_lossy(&pending[..pos]).trim().to_owned();
                    pending.drain(..=pos);
                    if !line.is_empty() {
                        emit(&app, &id, tick_from_line(&line));
                        last = line;
                    }
                }
            }
            last
        })
    };
    let out = child
        .stdout
        .take()
        .map(|s| pump(Box::new(s), app.clone(), id.to_owned()));
    let err = child
        .stderr
        .take()
        .map(|s| pump(Box::new(s), app.clone(), id.to_owned()));
    let status = wait_with_timeout(&mut child, INSTALL_TIMEOUT);
    let last_out = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let last_err = err.and_then(|h| h.join().ok()).unwrap_or_default();

    let version = probe(spec);
    let ok = status.is_some_and(|s| s.success());
    if ok || version.is_some() {
        let line = version
            .as_ref()
            .map_or_else(|| "Kurulum bitti".to_owned(), |v| format!("Kuruldu: {v}"));
        emit(
            app,
            id,
            json!({ "phase": "bitti", "percent": 100, "line": line }),
        );
        return Ok(json!({ "installed": true, "version": version, "opened": false }));
    }
    let reason = if status.is_none() {
        "Kurulum 20 dakikada bitmedi ve durduruldu".to_owned()
    } else {
        [last_err, last_out]
            .into_iter()
            .find(|l| !l.is_empty())
            .unwrap_or_else(|| "Kurulum başarısız".into())
    };
    let reason: String = reason.chars().take(LINE_MAX).collect();
    emit(
        app,
        id,
        json!({ "phase": "hata", "percent": null, "line": reason }),
    );
    Err(reason)
}

#[cfg(test)]
mod tests {
    use super::{first_version, tick_from_line};
    use serde_json::Value;

    #[test]
    fn parses_tool_version_lines() {
        assert_eq!(first_version("v22.11.0\n").as_deref(), Some("22.11.0"));
        assert_eq!(first_version("Python 3.14.0").as_deref(), Some("3.14.0"));
        assert_eq!(
            first_version("rustc 1.94.0 (abc 2026-01-01)").as_deref(),
            Some("1.94.0")
        );
        assert_eq!(
            first_version("go version go1.23.4 windows/amd64").as_deref(),
            Some("1.23.4")
        );
        assert_eq!(
            first_version("openjdk version \"21.0.5\" 2024-10-15").as_deref(),
            Some("21.0.5")
        );
        assert_eq!(
            first_version("git version 2.47.1.windows.1").as_deref(),
            Some("2.47.1")
        );
        assert_eq!(first_version("komut bulunamadı"), None);
    }

    #[test]
    fn winget_lines_become_ticks() {
        let t = tick_from_line("  Downloading https://x/node.msi   ██████  12.5 MB / 25 MB");
        assert_eq!(t["phase"], "indiriliyor");
        assert_eq!(t["percent"], 50.0);
        let t = tick_from_line("Successfully verified installer hash");
        assert_eq!(t["phase"], "dogrulaniyor");
        assert_eq!(t["percent"], Value::Null);
        // %100 bile 99'a kırpılır: tamamlandı yalnız süreç başarıyla bitince söylenir.
        assert_eq!(tick_from_line("Installing 100%")["percent"], 99.0);
    }
}
