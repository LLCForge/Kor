//! Resumable HTTPS download with a hard size cap and SHA-256 verification.

use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use futures::StreamExt as _;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::Progress;

/// Progress callbacks at most once per this many bytes (plus the last one).
const STEP: u64 = 512 * 1024;

/// HTTPS only; plain HTTP is accepted for loopback hosts (tests, local mirrors).
pub(crate) fn url_allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    url.strip_prefix("http://").is_some_and(|rest| {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        host == "127.0.0.1" || host == "localhost" || rest.starts_with("[::1]")
    })
}

fn client() -> Result<reqwest::Client, String> {
    kor_core::net::client_builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent(format!("kor/{}", kor_core::VERSION))
        .build()
        .map_err(|e| e.to_string())
}

/// Download `url` into `part`, continuing an earlier partial file with a `Range` request.
/// Never writes more than `size` bytes.
pub(crate) async fn download(
    url: &str,
    part: &Path,
    size: u64,
    name: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<(), String> {
    if !url_allowed(url) {
        return Err(format!("refusing non-HTTPS download: {url}"));
    }
    let mut have = tokio::fs::metadata(part).await.map_or(0, |m| m.len());
    if have > size {
        let _ = tokio::fs::remove_file(part).await;
        have = 0;
    }
    if have == size {
        return Ok(());
    }
    let mut req = client()?.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await.map_err(|e| kor_core::net::describe(&e))?;
    let status = resp.status();
    let resumed = if status == reqwest::StatusCode::PARTIAL_CONTENT && have > 0 {
        let range = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if !range.starts_with(&format!("bytes {have}-")) {
            let _ = tokio::fs::remove_file(part).await;
            return Err(format!("server sent an unexpected range ({range})"));
        }
        true
    } else if status.is_success() {
        have = 0;
        false
    } else {
        return Err(format!("download failed: HTTP {status} for {url}"));
    };
    if resp.content_length().is_some_and(|n| have + n > size) {
        return Err(format!("download is larger than the expected {size} bytes"));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(part)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut reported = have;
    progress(Progress {
        name: name.to_owned(),
        done: have,
        total: Some(size),
    });
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        have += chunk.len() as u64;
        if have > size {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            return Err(format!("download is larger than the expected {size} bytes"));
        }
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        if have - reported >= STEP || have == size {
            reported = have;
            progress(Progress {
                name: name.to_owned(),
                done: have,
                total: Some(size),
            });
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    file.sync_all().await.map_err(|e| e.to_string())?;
    if have < size {
        return Err(format!("download ended early ({have} of {size} bytes)"));
    }
    Ok(())
}

/// Lowercase hex SHA-256 of a file.
pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_https_or_loopback() {
        assert!(super::url_allowed("https://nodejs.org/dist/x.zip"));
        assert!(super::url_allowed("http://127.0.0.1:4000/x"));
        assert!(!super::url_allowed("http://example.com/x"));
        assert!(!super::url_allowed("http://127.0.0.1.evil.com/x"));
        assert!(!super::url_allowed("ftp://x"));
    }
}
