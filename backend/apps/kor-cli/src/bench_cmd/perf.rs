//! `kor bench --ui --cli --memory` (§32.1, §32.3, ADR 0062): measure the performance budgets on
//! this machine, compare with `bench/history.json`, record, gate and write `PERFORMANCE.md`.
//! Budgets, history and the gate are `kor_runtime::budgets`; this module measures.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use kor_i18n::{Localizer, args};
use kor_runtime::budgets::{self, History, Run, Stat};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::Ctx;
use crate::tui::{self, Ev, Panel, Screen};

/// What to measure and what to do with it.
#[derive(Debug)]
pub struct Opts {
    pub ui: bool,
    pub cli: bool,
    pub memory: bool,
    pub history: PathBuf,
    pub record: bool,
    pub check: bool,
    pub update_baseline: bool,
    pub weak: bool,
    pub desktop: Option<PathBuf>,
    pub records: u64,
    pub performance_md: Option<PathBuf>,
}

pub type Results = BTreeMap<String, Stat>;

const MB: f64 = 1_048_576.0;

#[allow(clippy::cast_precision_loss)] // sizes in MB for display and budgets
fn mb(bytes: u64) -> f64 {
    bytes as f64 / MB
}

fn put(r: &mut Results, id: &str, s: Option<Stat>) {
    if let Some(s) = s {
        r.insert(id.to_owned(), s);
    }
}

/// Wall time of `n` runs of `exe args` (one warm-up run first, not counted).
fn spawn_times(exe: &Path, args: &[&str], dir: &Path, n: usize) -> Vec<Duration> {
    (0..=n)
        .filter_map(|i| {
            let t = Instant::now();
            let ok = Command::new(exe)
                .args(args)
                .current_dir(dir)
                .env("NO_COLOR", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            let d = t.elapsed();
            (ok && i > 0).then_some(d)
        })
        .collect()
}

/// `kor` in line mode, in an empty folder, privacy mode, a local model: started, then measured
/// until its first line on stdout (the banner: ready for the first command).
struct Repl {
    child: std::process::Child,
    ready: Duration,
}

impl Repl {
    fn start(exe: &Path, dir: &Path) -> Option<Self> {
        let t = Instant::now();
        let mut child = Command::new(exe)
            .args(["--privacy-mode", "--model", "ollama/kor-bench"])
            .current_dir(dir)
            .env("KOR_UI", "line")
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        let mut first = [0u8; 1];
        let got = out.read(&mut first).ok()?;
        let ready = t.elapsed();
        if got == 0 {
            let _ = child.wait();
            return None;
        }
        // Keep draining so the child never blocks on a full pipe.
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = out.read_to_end(&mut sink);
        });
        Some(Self { child, ready })
    }

    /// Close stdin (EOF ends the REPL) and wait up to 5 s, then kill.
    fn stop(mut self) {
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The children's share of this process tree: (rss bytes, cpu).
fn children_usage() -> Option<(u64, Duration)> {
    let tree = kor_platform::procmem::tree()?;
    let me_rss = kor_platform::procmem::rss().unwrap_or(0);
    let me_cpu = kor_platform::procmem::cpu_time().unwrap_or_default();
    Some((
        tree.rss.saturating_sub(me_rss),
        tree.cpu.saturating_sub(me_cpu),
    ))
}

/// §32.1 CLI rows.
fn cli(ctx: &Ctx, r: &mut Results, notes: &mut Vec<String>) -> anyhow::Result<()> {
    let l = &ctx.l;
    let exe = std::env::current_exe()?;
    let dir = tempdir()?;
    phase(l, "kor --version");
    put(
        r,
        "cli.version",
        Stat::of_durations(&spawn_times(&exe, &["--version"], dir.path(), 20)),
    );
    phase(l, "kor help");
    put(
        r,
        "cli.help",
        Stat::of_durations(&spawn_times(&exe, &["help"], dir.path(), 20)),
    );

    phase(l, "kor (ready)");
    let mut ready = Vec::new();
    for _ in 0..8 {
        match Repl::start(&exe, dir.path()) {
            Some(repl) => {
                ready.push(repl.ready);
                repl.stop();
            }
            None => break,
        }
    }
    if ready.is_empty() {
        notes.push(l.t_args(
            "pbench-unavailable",
            &args!("id" => "cli.ready_cold", "why" => "kor did not start"),
        ));
    }
    put(r, "cli.ready_cold", Stat::of_durations(&ready));

    // Idle: a started REPL waiting for input; memory, and CPU over 5 s (no polling: ~0).
    phase(l, "kor (idle)");
    if let Some(repl) = Repl::start(&exe, dir.path()) {
        std::thread::sleep(Duration::from_millis(1500));
        if let Some((rss0, cpu0)) = children_usage() {
            let window = Duration::from_secs(5);
            std::thread::sleep(window);
            if let Some((rss1, cpu1)) = children_usage() {
                put(r, "cli.memory_idle", Some(Stat::single(mb(rss0.max(rss1)))));
                let busy = cpu1.saturating_sub(cpu0).as_secs_f64() / window.as_secs_f64();
                put(r, "cli.cpu_idle", Some(Stat::single(busy * 100.0)));
            }
        }
        repl.stop();
    }

    put(
        r,
        "cli.binary",
        std::fs::metadata(&exe)
            .ok()
            .map(|m| Stat::single(mb(m.len()))),
    );

    phase(l, "token path");
    put(r, "cli.token_path", token_path()?);

    phase(l, "soak");
    put(r, "cli.leak", soak(l, dir.path()));
    Ok(())
}

fn tempdir() -> anyhow::Result<TempDir> {
    let p = std::env::temp_dir().join(format!(
        "kor-bench-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::create_dir_all(&p)?;
    Ok(TempDir(p))
}

/// A folder removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn phase(l: &Localizer, what: &str) {
    eprintln!(
        "{}",
        l.t_args("pbench-phase", &args!("what" => what.to_owned()))
    );
}

/// A loopback OpenAI-compatible server streaming `tokens` deltas 4 ms apart; returns its address
/// and, per token, the instant its bytes were written.
async fn sse_server(
    tokens: usize,
) -> anyhow::Result<(
    std::net::SocketAddr,
    tokio::sync::oneshot::Receiver<Vec<Instant>>,
)> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (done, sent) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        sock.set_nodelay(true).ok();
        // Read the request head and its body.
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let Ok(n) = sock.read(&mut buf).await else {
                return;
            };
            if n == 0 {
                return;
            }
            req.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&req);
            if let Some(head_end) = text.find("\r\n\r\n") {
                let len = text[..head_end]
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if req.len() >= head_end + 4 + len {
                    break;
                }
            }
        }
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ntransfer-encoding: chunked\r\n\r\n";
        if sock.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        let mut stamps = Vec::with_capacity(tokens);
        for i in 0..=tokens {
            let data = if i < tokens {
                format!(
                    "data: {{\"id\":\"b\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"t{i} \"}},\"finish_reason\":null}}]}}\n\n"
                )
            } else {
                "data: {\"id\":\"b\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_owned()
            };
            let chunk = format!("{:x}\r\n{data}\r\n", data.len());
            if sock.write_all(chunk.as_bytes()).await.is_err() || sock.flush().await.is_err() {
                return;
            }
            if i < tokens {
                stamps.push(Instant::now());
                tokio::time::sleep(Duration::from_millis(4)).await;
            }
        }
        let _ = sock.write_all(b"0\r\n\r\n").await;
        let _ = done.send(stamps);
    });
    Ok((addr, sent))
}

/// Model token → screen: bytes written by a loopback server → provider SSE parse → text event →
/// TUI reduce + a full frame (120×40, 5000-line scrollback). Per token, p50/p95.
fn token_path() -> anyhow::Result<Option<Stat>> {
    const TOKENS: usize = 300;
    let l = Localizer::new(kor_i18n::Lang::En);
    let rt = crate::providers::runtime()?;
    rt.block_on(async {
        use futures::StreamExt as _;
        let (addr, sent) = sse_server(TOKENS).await?;
        let manifest = kor_providers::ProviderManifest::parse(
            &format!(
                "id = \"kor-bench\"\nname = \"Kor bench\"\ncategory = \"local\"\nadapter = \"openai_compat\"\n\
                 base_url = \"http://{addr}/v1\"\nauth = {{ kind = \"bearer\", env = \"KOR_BENCH_KEY\" }}\n\
                 [privacy]\ntier = \"standard\"\nretention = \"test\"\n"
            ),
            "bench",
        )
        .map_err(|e| anyhow!("{e}"))?;
        let provider = kor_providers::build(
            &manifest,
            Some(kor_secrets::SecretString::from("kor-bench")),
        )
        .map_err(|e| anyhow!("{e}"))?;
        let req = kor_providers::ChatRequest {
            model: "bench".into(),
            system: None,
            messages: vec![kor_core::Message::text(kor_core::Role::User, "go")],
            tools: Vec::new(),
            max_tokens: Some(16),
            temperature: None,
            reasoning: None,
        };
        let mut screen = filled_screen(&l);
        let mut term = Terminal::new(TestBackend::new(120, 40))?;
        let mut stream = provider
            .complete(req, kor_providers::CancellationToken::new())
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let mut arrived = Vec::with_capacity(TOKENS);
        while let Some(ev) = stream.next().await {
            if let Ok(kor_core::StreamEvent::TextDelta { text }) = ev {
                tui::reduce(&mut screen, &l, Ev::Text(text));
                term.draw(|f| tui::draw(f, &screen, &l))?;
                arrived.push(Instant::now());
            }
        }
        let sent = sent.await.unwrap_or_default();
        let lat: Vec<Duration> = sent
            .iter()
            .zip(&arrived)
            .map(|(s, a)| a.saturating_duration_since(*s))
            .collect();
        Ok::<_, anyhow::Error>(Stat::of_durations(&lat))
    })
}

/// A TUI screen with a full scrollback (5000 lines kept of a 10k-message thread).
fn filled_screen(l: &Localizer) -> Screen {
    let mut s = Screen::new("default", "ollama/kor-bench");
    for i in 0..10_000 {
        tui::reduce(
            &mut s,
            l,
            Ev::Text(format!(
                "line {i}: the quick brown fox jumps over the lazy dog, {i} times\n"
            )),
        );
    }
    s
}

/// §32.1 TUI rows, measured in process on a headless 120×40 terminal.
fn tui_rows(l: &Localizer, r: &mut Results) -> anyhow::Result<()> {
    phase(l, "tui");
    let first: Vec<Duration> = (0..20)
        .map(|_| -> anyhow::Result<Duration> {
            let t = Instant::now();
            let s = Screen::new("default", "ollama/kor-bench");
            let mut term = Terminal::new(TestBackend::new(120, 40))?;
            term.draw(|f| tui::draw(f, &s, l))?;
            Ok(t.elapsed())
        })
        .collect::<anyhow::Result<_>>()?;
    put(r, "tui.first_draw", Stat::of_durations(&first));

    let mut screen = filled_screen(l);
    let mut term = Terminal::new(TestBackend::new(120, 40))?;
    term.draw(|f| tui::draw(f, &screen, l))?;
    let mut frame = |screen: &mut Screen, ev: Ev| -> anyhow::Result<Duration> {
        let t = Instant::now();
        tui::reduce(screen, l, ev);
        term.draw(|f| tui::draw(f, screen, l))?;
        Ok(t.elapsed())
    };
    let keys = "the quick brown fox jumps over the lazy dog "
        .chars()
        .cycle()
        .take(200)
        .map(|c| frame(&mut screen, Ev::Char(c)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    put(r, "tui.keypress", Stat::of_durations(&keys));
    let stream = (0..300)
        .map(|i| frame(&mut screen, Ev::Text(format!("tok{i} "))))
        .collect::<anyhow::Result<Vec<_>>>()?;
    put(r, "tui.frame", Stat::of_durations(&stream));
    let scroll = (0..300)
        .map(|i| frame(&mut screen, Ev::Wheel(Panel::Stream, i % 100 < 50)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    put(r, "tui.scroll", Stat::of_durations(&scroll));
    Ok(())
}

/// Accelerated long session: the same work as hours of use (TUI stream + redraws, knowledge
/// writes and searches), resident memory after a warm-up round against after the last round.
fn soak(l: &Localizer, dir: &Path) -> Option<Stat> {
    let kb = kor_memory::Knowledge::open(&dir.join("soak.sqlite")).ok()?;
    let mut screen = filled_screen(l);
    let mut term = Terminal::new(TestBackend::new(120, 40)).ok()?;
    let mut round = |n: usize| -> Option<()> {
        for i in 0..2000 {
            tui::reduce(&mut screen, l, Ev::Text(format!("round {n} token {i} ")));
            if i % 20 == 0 {
                term.draw(|f| tui::draw(f, &screen, l)).ok()?;
            }
        }
        for i in 0..200 {
            kb.record(kor_memory::Record {
                project: "/soak".into(),
                task: format!("soak task {n} {i}"),
                summary: "fixed the auth error in the build config".into(),
                ..kor_memory::Record::default()
            })
            .ok()?;
        }
        kb.flush().ok()?;
        kb.search("auth error", None, None, 10).ok()?;
        Some(())
    };
    for n in 0..3 {
        round(n)?;
    }
    let before = kor_platform::procmem::rss()?;
    for n in 3..40 {
        round(n)?;
    }
    let after = kor_platform::procmem::rss()?;
    #[allow(clippy::cast_precision_loss)]
    let growth = (after.saturating_sub(before)) as f64 / before as f64 * 100.0;
    Some(Stat::single(growth))
}

/// §31.3.1 rows from `kor_memory::bench`.
fn memory(l: &Localizer, records: u64, r: &mut Results) -> anyhow::Result<()> {
    let dir = tempdir()?;
    let vectors = usize::try_from(records).unwrap_or(usize::MAX);
    let rep = kor_memory::bench::run(dir.path(), records, vectors, &|p| {
        phase(l, &format!("knowledge: {p}"));
    })
    .map_err(|e| anyhow!("{e}"))?;
    for row in &rep.rows {
        r.insert(
            row.id.to_owned(),
            Stat {
                p50: row.p50_ms,
                p95: row.p95_ms,
                n: row.samples,
            },
        );
    }
    r.insert("memory.disk".into(), Stat::single(mb(rep.disk_bytes)));
    Ok(())
}

fn git_commit() -> Option<String> {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        return Some(sha.chars().take(12).collect());
    }
    let out = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn fmt(v: f64, unit: budgets::Unit) -> String {
    let n = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 1.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    };
    format!("{n} {}", unit.as_str())
}

fn print_table(l: &Localizer, run: &Run, history: &History) {
    println!("{}", l.t("pbench-head"));
    let base = history.baseline.get(&run.machine);
    for b in budgets::BUDGETS {
        let Some(s) = run.results.get(b.id) else {
            continue;
        };
        let target = if run.weak {
            b.target * budgets::WEAK_FACTOR
        } else {
            b.target
        };
        let mark = if s.p95 <= target { "ok  " } else { "OVER" };
        let baseline = base
            .and_then(|m| m.get(b.id))
            .map_or_else(|| "–".to_owned(), |v| fmt(*v, b.unit));
        println!(
            "  {mark} {:<18} {:>11} {:>11}  < {:<10} {:<11} {}",
            b.id,
            fmt(s.p95, b.unit),
            fmt(s.p50, b.unit),
            fmt(target, b.unit),
            baseline,
            b.what
        );
    }
}

/// Run the selected groups, then report, record and gate.
pub fn run(ctx: &Ctx, o: &Opts) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let mut history = History::load(&o.history).map_err(|e| anyhow!(e))?;
    let mut results = Results::new();
    let mut notes = Vec::new();
    if o.cli {
        cli(ctx, &mut results, &mut notes)?;
    }
    if o.ui {
        tui_rows(l, &mut results)?;
        super::desktop::run(l, o.desktop.as_deref(), &mut results, &mut notes);
    }
    if o.memory {
        memory(l, o.records, &mut results)?;
    }
    let measured = o.cli || o.ui || o.memory;
    let mut passed = true;
    if measured {
        let run = Run {
            ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            commit: git_commit(),
            machine: budgets::machine_key(o.weak),
            weak: o.weak,
            results,
        };
        print_table(l, &run, &history);
        for n in &notes {
            println!("{n}");
        }
        let v = budgets::check(&history, &run);
        for x in &v.over_budget {
            println!(
                "{}",
                l.t_args(
                    "pbench-over",
                    &args!("id" => x.id.clone(), "p95" => format!("{:.3}", x.p95), "target" => format!("{:.3}", x.target))
                )
            );
        }
        for x in &v.regressions {
            println!(
                "{}",
                l.t_args(
                    "pbench-regression",
                    &args!("id" => x.id.clone(), "now" => format!("{:.3}", x.now),
                           "base" => format!("{:.3}", x.baseline), "pct" => format!("{:+.1}%", x.change * 100.0))
                )
            );
        }
        // A budget that could not be measured fails the gate too: nothing passes by not running.
        passed = v.passed() && notes.is_empty();
        println!(
            "{}",
            l.t(if passed { "pbench-pass" } else { "pbench-fail" })
        );
        if o.update_baseline {
            history.set_baseline(&run);
            println!(
                "{}",
                l.t_args(
                    "pbench-baseline",
                    &args!("machine" => run.machine.clone(), "path" => o.history.display().to_string())
                )
            );
        }
        if o.record {
            history.push(run);
            println!(
                "{}",
                l.t_args(
                    "pbench-recorded",
                    &args!("path" => o.history.display().to_string())
                )
            );
        }
        if o.record || o.update_baseline {
            history.save(&o.history).map_err(|e| anyhow!(e))?;
        }
    }
    if let Some(md) = &o.performance_md {
        std::fs::write(md, budgets::performance_md(&history))?;
        println!(
            "{}",
            l.t_args(
                "pbench-md-written",
                &args!("path" => md.display().to_string())
            )
        );
    }
    std::io::stdout().flush()?;
    Ok(!o.check || passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_rows_are_measured_headless() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let mut r = Results::new();
        tui_rows(&l, &mut r).unwrap();
        for id in ["tui.first_draw", "tui.keypress", "tui.frame", "tui.scroll"] {
            assert!(r[id].n > 0 && r[id].p95 >= r[id].p50, "{id}");
        }
    }

    #[test]
    fn token_path_measures_every_token() {
        let s = token_path().unwrap().unwrap();
        assert_eq!(s.n, 300);
        assert!(s.p95 < 1000.0, "{s:?}");
    }

    #[test]
    fn soak_reports_growth_percent() {
        let l = Localizer::new(kor_i18n::Lang::En);
        let dir = tempdir().unwrap();
        let s = soak(&l, dir.path()).unwrap();
        assert!(s.p95.is_finite());
    }
}
