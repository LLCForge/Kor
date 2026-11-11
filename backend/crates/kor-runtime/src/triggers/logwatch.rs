//! `log_match` sources: a file (followed like `tail -F`), `journald:<unit>` (`journalctl -f`) or
//! `docker:<container>` (`docker logs -f`). A matching line fires once, then the trigger stays
//! quiet for its debounce period.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use regex::Regex;
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncSeekExt as _, BufReader};
use tokio::sync::mpsc::UnboundedSender;

pub const DEFAULT_DEBOUNCE_SECS: u64 = 300;
const MAX_LINE: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Journald(String),
    Docker(String),
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-@:".contains(c))
}

impl Source {
    /// `journald:nginx.service`, `docker:web`, anything else is a file path (relative to `root`).
    ///
    /// # Errors
    /// An empty source or an invalid unit/container name.
    pub fn parse(text: &str, root: &std::path::Path) -> Result<Self, String> {
        let t = text.trim();
        if let Some(u) = t.strip_prefix("journald:") {
            return name_ok(u)
                .then(|| Self::Journald(u.to_owned()))
                .ok_or_else(|| format!("bad journald unit `{u}`"));
        }
        if let Some(c) = t.strip_prefix("docker:") {
            return name_ok(c)
                .then(|| Self::Docker(c.to_owned()))
                .ok_or_else(|| format!("bad container `{c}`"));
        }
        if t.is_empty() {
            return Err("log_match needs a source".into());
        }
        Ok(Self::File(root.join(t)))
    }
}

/// Debounce state of one trigger.
#[derive(Debug)]
pub struct Matcher {
    re: Regex,
    quiet: Duration,
    last: Option<Instant>,
}

impl Matcher {
    pub fn new(re: Regex, debounce_secs: u64) -> Self {
        let secs = if debounce_secs == 0 {
            DEFAULT_DEBOUNCE_SECS
        } else {
            debounce_secs
        };
        Self {
            re,
            quiet: Duration::from_secs(secs),
            last: None,
        }
    }

    /// `true` when `line` should fire now.
    pub fn hit(&mut self, line: &str, now: Instant) -> bool {
        if !self.re.is_match(line) {
            return false;
        }
        if self
            .last
            .is_some_and(|l| now.duration_since(l) < self.quiet)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// Every line of `r` to `tx`.
async fn forward<R: AsyncRead + Unpin>(r: R, tx: UnboundedSender<String>) {
    let mut r = BufReader::new(r);
    let mut line = String::new();
    loop {
        line.clear();
        match r.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if tx
                    .send(line.trim_end().chars().take(MAX_LINE).collect())
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

async fn follow_file(path: PathBuf, mut m: Matcher, tx: UnboundedSender<String>) {
    // Start at the end: only new lines count.
    let mut pos = tokio::fs::metadata(&path)
        .await
        .map(|md| md.len())
        .unwrap_or(0);
    let mut pending = String::new();
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if tx.is_closed() {
            return;
        }
        let Ok(md) = tokio::fs::metadata(&path).await else {
            continue;
        };
        if md.len() < pos {
            pos = 0; // truncated or rotated
        }
        if md.len() == pos {
            continue;
        }
        let Ok(mut f) = tokio::fs::File::open(&path).await else {
            continue;
        };
        if f.seek(std::io::SeekFrom::Start(pos)).await.is_err() {
            continue;
        }
        let mut buf = Vec::new();
        let Ok(n) = f.take(1024 * 1024).read_to_end(&mut buf).await else {
            continue;
        };
        pos += n as u64;
        pending.push_str(&String::from_utf8_lossy(&buf));
        while let Some(i) = pending.find('\n') {
            let line: String = pending[..i].trim_end().chars().take(MAX_LINE).collect();
            pending.drain(..=i);
            if m.hit(&line, Instant::now()) && tx.send(line).is_err() {
                return;
            }
        }
        if pending.len() > MAX_LINE {
            pending.clear();
        }
    }
}

async fn follow_command(
    program: &str,
    args: Vec<String>,
    mut m: Matcher,
    tx: UnboundedSender<String>,
) {
    use kor_platform::NoWindow as _;
    loop {
        let child = tokio::process::Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .no_window()
            .spawn();
        if let Ok(mut c) = child {
            let (out, err) = (c.stdout.take(), c.stderr.take());
            // Both streams through one matcher: docker writes many apps' errors to stderr.
            let (otx, mut orx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let a = out.map(|o| tokio::spawn(forward(o, otx.clone())));
            let b = err.map(|e| tokio::spawn(forward(e, otx.clone())));
            drop(otx);
            while let Some(line) = orx.recv().await {
                if m.hit(&line, Instant::now()) && tx.send(line).is_err() {
                    return;
                }
            }
            for h in [a, b].into_iter().flatten() {
                let _ = h.await;
            }
            let _ = c.wait().await;
        }
        if tx.is_closed() {
            return;
        }
        // The unit or container may come back; retry calmly.
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Follow `source`; matching lines (after debounce) go to `tx`. Ends when `tx` is closed.
pub fn spawn(
    source: Source,
    m: Matcher,
    tx: UnboundedSender<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match source {
            Source::File(p) => follow_file(p, m, tx).await,
            Source::Journald(u) => {
                let args = ["-f", "-n", "0", "-o", "cat", "-u"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .chain([u])
                    .collect();
                follow_command("journalctl", args, m, tx).await;
            }
            Source::Docker(c) => {
                let args = ["logs", "-f", "--tail", "0"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .chain([c])
                    .collect();
                follow_command("docker", args, m, tx).await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_and_debounce() {
        let root = std::path::Path::new("/p");
        assert_eq!(
            Source::parse("journald:nginx.service", root),
            Ok(Source::Journald("nginx.service".into()))
        );
        assert_eq!(
            Source::parse("docker:web-1", root),
            Ok(Source::Docker("web-1".into()))
        );
        assert!(Source::parse("docker:--privileged", root).is_err());
        assert!(Source::parse("docker:a b", root).is_err());
        assert_eq!(
            Source::parse("logs/x.log", root),
            Ok(Source::File(root.join("logs/x.log")))
        );

        let mut m = Matcher::new(Regex::new(r"\s5\d\d\s").unwrap(), 60);
        let t0 = Instant::now();
        assert!(!m.hit("GET / 200 ok", t0));
        assert!(m.hit("GET / 502 bad", t0));
        assert!(
            !m.hit("GET / 503 bad", t0 + Duration::from_secs(10)),
            "debounced"
        );
        assert!(m.hit("GET / 500 bad", t0 + Duration::from_secs(61)));
    }

    #[tokio::test]
    async fn follows_a_growing_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("app.log");
        std::fs::write(&f, "old 500 line\n").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let h = spawn(
            Source::File(f.clone()),
            Matcher::new(Regex::new(r"\b5\d\d\b").unwrap(), 1),
            tx,
        );
        tokio::time::sleep(Duration::from_millis(700)).await;
        {
            use std::io::Write as _;
            let mut w = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
            writeln!(w, "ok 200").unwrap();
            writeln!(w, "boom 502 upstream").unwrap();
        }
        let line = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(line, "boom 502 upstream");
        drop(rx);
        h.abort();
    }
}
