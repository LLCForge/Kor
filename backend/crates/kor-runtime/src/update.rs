//! Self-update (`kor update`, Master Build Prompt §15.17).
//!
//! Source: GitHub Releases of [`repo()`] (`branding.toml` `[urls] repository`). Channels: `stable` (non-prerelease), `beta` (prerelease),
//! `nightly` (a release whose tag contains `nightly`). Each release carries plain binaries named
//! `kor-<target>[.exe]` and a minisign signature `<asset>.minisig` (see the release workflow).
//!
//! Safety: a downloaded binary is installed only if its signature verifies against the public key
//! compiled into this build (`KOR_UPDATE_PUBKEY` at build time). A build without that key can
//! check and notify, never install: it fails closed. The running binary is replaced by renames
//! (atomic on one volume); the previous one is kept as `<exe>.previous` for `kor update rollback`.
//! Nothing runs in privacy mode or offline.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// `owner/name` on GitHub, from `branding.toml` `[urls] repository` (the one place to change it).
pub fn repo() -> &'static str {
    let url = kor_platform::brand().urls.repository.as_str();
    url.strip_prefix("https://github.com/")
        .unwrap_or(url)
        .trim_end_matches('/')
}
/// Minisign public key of the release signer, baked in by the release build.
pub const PUBLIC_KEY: Option<&str> = option_env!("KOR_UPDATE_PUBKEY");
/// The installed release asset is at most this large.
const MAX_BINARY: usize = 256 * 1024 * 1024;
/// Automatic checks happen at most once per this interval.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Beta,
    Nightly,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stable" => Some(Self::Stable),
            "beta" => Some(Self::Beta),
            "nightly" => Some(Self::Nightly),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// `1.2.3`, `v1.2.3-beta.1` → comparable parts. A pre-release sorts below its release.
fn version_key(v: &str) -> Option<(u64, u64, u64, bool, String)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
    let mut it = core.split('.').map(str::parse::<u64>);
    let (a, b, c) = (
        it.next()?.ok()?,
        it.next()?.ok()?,
        it.next().unwrap_or(Ok(0)).ok()?,
    );
    Some((a, b, c, pre.is_none(), pre.unwrap_or_default().to_owned()))
}

/// `candidate` is newer than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    match (version_key(candidate), version_key(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The newest release of `channel` among `releases` (drafts ignored).
pub fn pick(releases: &[Release], channel: Channel) -> Option<&Release> {
    let fits = |r: &&Release| {
        !r.draft
            && match channel {
                Channel::Stable => !r.prerelease && !r.tag_name.contains("nightly"),
                Channel::Beta => !r.tag_name.contains("nightly"),
                Channel::Nightly => true,
            }
    };
    let mut fitting: Vec<&Release> = releases.iter().filter(fits).collect();
    if channel == Channel::Nightly
        && let Some(n) = fitting.iter().find(|r| r.tag_name.contains("nightly"))
    {
        return Some(n);
    }
    fitting.sort_by(|a, b| version_key(&b.tag_name).cmp(&version_key(&a.tag_name)));
    fitting.first().copied()
}

/// The target triple of the release asset for this machine (Linux builds are static musl on `x86_64`).
pub fn target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

pub fn asset_name() -> Option<String> {
    target().map(|t| format!("kor-{t}{}", std::env::consts::EXE_SUFFIX))
}

/// Verify `data` against a minisign signature file's text with the built-in key.
///
/// # Errors
/// If this build has no key, or the signature does not verify.
pub fn verify(data: &[u8], signature: &str) -> Result<(), String> {
    let key = PUBLIC_KEY.ok_or("no update key in this build; only notifications are possible")?;
    verify_with(key, data, signature)
}

/// [`verify`] with an explicit public key (base64), e.g. a test key.
///
/// # Errors
/// If the key or signature cannot be read, or the signature does not verify.
pub fn verify_with(key: &str, data: &[u8], signature: &str) -> Result<(), String> {
    let pk = minisign_verify::PublicKey::from_base64(key.trim())
        .map_err(|e| format!("update key: {e}"))?;
    let sig =
        minisign_verify::Signature::decode(signature).map_err(|e| format!("signature: {e}"))?;
    pk.verify(data, &sig, false)
        .map_err(|e| format!("signature does not verify: {e}"))
}

/// Installs managed by a package manager update through it; Kor only notifies there.
pub fn package_manager(exe: &Path) -> Option<&'static str> {
    let p = exe.to_string_lossy().replace('\\', "/").to_lowercase();
    if p.contains("/winget/") || p.contains("/microsoft/winget") {
        Some("winget")
    } else if p.contains("/cellar/") || p.contains("/homebrew/") {
        Some("brew")
    } else if p.starts_with("/usr/bin/") || p.starts_with("/usr/lib/") {
        Some("apt")
    } else if p.contains("/flatpak/") {
        Some("flatpak")
    } else {
        None
    }
}

fn client() -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!("kor-update/{}", kor_core::VERSION))
        .build()
        .map_err(|e| e.to_string())
}

/// Releases from GitHub (newest first, up to 30).
///
/// # Errors
/// On network or HTTP errors.
pub async fn releases() -> Result<Vec<Release>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page=30",
        repo()
    );
    let res = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    if !res.status().is_success() {
        return Err(format!("GitHub: HTTP {}", res.status().as_u16()));
    }
    let text = res.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

async fn download(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let res = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| kor_core::net::describe(&e))?;
    if !res.status().is_success() {
        return Err(format!("download: HTTP {}", res.status().as_u16()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err("download too large".into());
    }
    Ok(bytes.to_vec())
}

/// Result of a check.
#[derive(Debug, Clone)]
pub struct Check {
    pub current: String,
    pub channel: Channel,
    /// A newer release, if any.
    pub available: Option<Release>,
}

/// Look for a newer release of `channel` than `current`.
///
/// # Errors
/// On network errors.
pub async fn check(current: &str, channel: Channel) -> Result<Check, String> {
    let all = releases().await?;
    let available = pick(&all, channel)
        .filter(|r| r.tag_name.contains("nightly") || newer(&r.tag_name, current))
        .cloned();
    Ok(Check {
        current: current.to_owned(),
        channel,
        available,
    })
}

/// Download, verify and install `release` over the running executable.
///
/// # Errors
/// If the asset is missing, the signature fails, or the files cannot be replaced.
pub async fn install(release: &Release) -> Result<PathBuf, String> {
    let name = asset_name().ok_or("no release build for this platform")?;
    let find = |n: &str| release.assets.iter().find(|a| a.name == n);
    let bin = find(&name).ok_or_else(|| format!("release has no `{name}`"))?;
    let sig = find(&format!("{name}.minisig")).ok_or("release has no signature")?;
    let data = download(&bin.browser_download_url, MAX_BINARY).await?;
    let sig = download(&sig.browser_download_url, 64 * 1024).await?;
    let sig = String::from_utf8(sig).map_err(|_| "signature is not text".to_owned())?;
    verify(&data, &sig)?;
    replace_current(&data)
}

/// `kor update --from <file>`: the signature must sit next to it as `<file>.minisig`.
///
/// # Errors
/// As [`install`].
pub fn install_from(file: &Path) -> Result<PathBuf, String> {
    // An air-gap bundle (`kor offline bundle`, ADR 0034) carries `bin/kor[.exe]` and its
    // `.minisig`; the bundle manifest's sha256 is checked first, the signature as always.
    if kor_components::bundle::is_bundle(file) {
        let (data, sig) = bundled_cli(file)?;
        verify(&data, &sig)?;
        return replace_current(&data);
    }
    let data = std::fs::read(file).map_err(|e| e.to_string())?;
    let sig_path = PathBuf::from(format!("{}.minisig", file.display()));
    let sig = std::fs::read_to_string(&sig_path)
        .map_err(|_| format!("missing signature {}", sig_path.display()))?;
    verify(&data, &sig)?;
    replace_current(&data)
}

/// The CLI binary and its signature text from an air-gap bundle for this machine.
fn bundled_cli(file: &Path) -> Result<(Vec<u8>, String), String> {
    use kor_components::bundle;
    let m = bundle::read_manifest(file)?;
    let here = kor_components::target();
    if m.platform != here.trim_end_matches("-musl") {
        return Err(format!(
            "this bundle is for {}, this computer is {here}",
            m.platform
        ));
    }
    let name = format!("bin/kor{}", std::env::consts::EXE_SUFFIX);
    let data = bundle::read_file(file, &name, MAX_BINARY as u64)?;
    let sig = bundle::read_file(file, &format!("{name}.minisig"), 64 * 1024)
        .map_err(|_| "the bundle has no signature for its kor binary".to_owned())?;
    let sig = String::from_utf8(sig).map_err(|_| "signature is not text".to_owned())?;
    Ok((data, sig))
}

fn previous_path(exe: &Path) -> PathBuf {
    PathBuf::from(format!("{}.previous", exe.display()))
}

/// Replace `exe` with `data` by renames; the old binary becomes `<exe>.previous`.
fn replace_at(exe: &Path, data: &[u8]) -> Result<PathBuf, String> {
    let new = PathBuf::from(format!("{}.new", exe.display()));
    std::fs::write(&new, data).map_err(|e| e.to_string())?;
    let _ = kor_platform::make_executable(&new);
    let prev = previous_path(exe);
    let _ = std::fs::remove_file(&prev);
    // A running executable can be renamed on every platform (not overwritten on Windows).
    std::fs::rename(exe, &prev).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        e.to_string()
    })?;
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&prev, exe);
        return Err(e.to_string());
    }
    Ok(exe.to_owned())
}

fn replace_current(data: &[u8]) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(pm) = package_manager(&exe) {
        return Err(format!("installed by {pm}; update it with {pm}"));
    }
    // The CLI bundled with the desktop app moves with the app's signed updates (ADR 0032).
    if exe
        .with_file_name(kor_platform::exe("kor-desktop"))
        .is_file()
    {
        return Err("installed with Kor desktop; the desktop app updates it".into());
    }
    replace_at(&exe, data)
}

/// `kor update rollback`: swap the current binary with `<exe>.previous`.
///
/// # Errors
/// If there is no previous version or the files cannot be swapped.
pub fn rollback() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let prev = previous_path(&exe);
    let data =
        std::fs::read(&prev).map_err(|_| "no previous version to roll back to".to_owned())?;
    replace_at(&exe, &data)
}

/// When the last automatic check ran (`<kor home>/update-check`, Unix seconds).
pub fn due(now_secs: u64) -> bool {
    let Some(f) = crate::kor_home().map(|h| h.join("update-check")) else {
        return false;
    };
    let last = std::fs::read_to_string(&f)
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if now_secs.saturating_sub(last) < CHECK_INTERVAL.as_secs() {
        return false;
    }
    let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
    let _ = std::fs::write(&f, now_secs.to_string());
    true
}

/// A note left by the background check for the next start (`<kor home>/update-notice`).
fn notice_file() -> Option<PathBuf> {
    crate::kor_home().map(|h| h.join("update-notice"))
}

/// The note from the last background check, removed once read.
pub fn take_notice() -> Option<String> {
    let f = notice_file()?;
    let text = std::fs::read_to_string(&f).ok()?;
    let _ = std::fs::remove_file(&f);
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Daily background check for interactive sessions: never blocks the caller, never prints. A newer
/// signed release is installed when `auto` is on (the next start runs it); otherwise, or when this
/// build has no update key, a one-line note is left for the next start.
pub fn spawn_background_check(current: &'static str, channel: Channel, auto: bool) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if crate::privacy_mode() || std::env::var_os("CI").is_some() || !due(now) {
        return;
    }
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let note = rt.block_on(async {
            let found = check(current, channel).await.ok()?.available?;
            let tag = found.tag_name.trim_start_matches('v').to_owned();
            if auto && PUBLIC_KEY.is_some() && install(&found).await.is_ok() {
                return Some(format!("installed {tag}"));
            }
            Some(format!("available {tag} {}", found.html_url))
        });
        if let (Some(note), Some(f)) = (note, notice_file()) {
            let _ = std::fs::write(f, note);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_is_owner_slash_name_from_branding() {
        let r = repo();
        assert_eq!(r.split('/').filter(|s| !s.is_empty()).count(), 2, "{r}");
        assert!(kor_platform::brand().urls.repository.ends_with(r));
    }

    #[test]
    fn bundled_cli_needs_matching_platform_and_signature() {
        use kor_components::bundle::{self, Kind, Source};
        let dir = tempfile::tempdir().unwrap();
        let exe = format!("kor{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(dir.path().join("k"), b"binary").unwrap();
        std::fs::write(dir.path().join("s"), b"untrusted comment: x\nsig").unwrap();
        let make = |platform: &str, with_sig: bool| {
            let out = dir.path().join(format!("{platform}-{with_sig}.zip"));
            let mut files = vec![Source {
                path: format!("bin/{exe}"),
                file: dir.path().join("k"),
                kind: Kind::Cli,
                component: None,
            }];
            if with_sig {
                files.push(Source {
                    path: format!("bin/{exe}.minisig"),
                    file: dir.path().join("s"),
                    kind: Kind::Signature,
                    component: None,
                });
            }
            let meta = bundle::Manifest {
                format: bundle::FORMAT,
                kor_version: "0".into(),
                platform: platform.into(),
                created: 1,
                runtime: String::new(),
                model: String::new(),
                files: Vec::new(),
            };
            bundle::write(&out, meta, &files, &|_| {}).unwrap();
            out
        };
        let here = kor_components::target()
            .trim_end_matches("-musl")
            .to_owned();
        let (data, sig) = bundled_cli(&make(&here, true)).unwrap();
        assert_eq!(data, b"binary");
        assert!(sig.ends_with("sig"));
        assert!(
            bundled_cli(&make(&here, false))
                .unwrap_err()
                .contains("signature")
        );
        assert!(
            bundled_cli(&make("plan9-mips", true))
                .unwrap_err()
                .contains("bundle is for")
        );
        // The signature check itself still runs: a fake one never installs.
        assert!(install_from(&make(&here, true)).is_err());
    }

    fn rel(tag: &str, pre: bool) -> Release {
        Release {
            tag_name: tag.into(),
            prerelease: pre,
            draft: false,
            body: None,
            html_url: String::new(),
            assets: Vec::new(),
        }
    }

    #[test]
    fn versions_and_channels() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "1.0.0-beta.2"));
        assert!(!newer("1.0.0-beta.2", "1.0.0"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("garbage", "0.1.0"));
        let all = [
            rel("v0.3.0-beta.1", true),
            rel("v0.2.0", false),
            rel("nightly", true),
            rel("v0.1.0", false),
        ];
        assert_eq!(pick(&all, Channel::Stable).unwrap().tag_name, "v0.2.0");
        assert_eq!(pick(&all, Channel::Beta).unwrap().tag_name, "v0.3.0-beta.1");
        assert_eq!(pick(&all, Channel::Nightly).unwrap().tag_name, "nightly");
        assert!(asset_name().is_some());
    }

    #[test]
    fn signatures_must_verify_and_unsigned_builds_never_install() {
        // A real key pair and signature, as the release workflow produces them.
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, &b"kor binary"[..], None, None)
            .unwrap()
            .to_string();
        let pk = kp.pk.to_base64();
        assert!(verify_with(&pk, b"kor binary", &sig).is_ok());
        assert!(verify_with(&pk, b"tampered", &sig).is_err());
        let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        assert!(verify_with(&other.pk.to_base64(), b"kor binary", &sig).is_err());
        if PUBLIC_KEY.is_none() {
            assert!(verify(b"kor binary", &sig).is_err(), "no key, no install");
        }
    }

    #[test]
    fn replace_keeps_previous_for_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("kor-test-bin");
        std::fs::write(&exe, b"old").unwrap();
        replace_at(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(previous_path(&exe)).unwrap(), b"old");
        assert_eq!(
            package_manager(Path::new(
                "C:/Users/user/AppData/Local/Microsoft/WinGet/Packages/kor.exe"
            )),
            Some("winget")
        );
        assert_eq!(package_manager(Path::new("/home/x/.local/bin/kor")), None);
    }
}
