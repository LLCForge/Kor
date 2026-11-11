//! File watcher: debounced incremental updates on save (`notify`). Stop with
//! [`WatchHandle::stop`] or by dropping the handle.

use crate::graph::{Graph, UpdateStats};
use crate::{Error, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Quiet time after the last event before the graph is updated.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
/// Low-power mode (§32.3, ADR 0062): updates wait ten times longer, so a burst of saves costs one
/// re-index instead of several.
static LOW_POWER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_low_power(on: bool) {
    LOW_POWER.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The debounce in force.
pub fn debounce() -> Duration {
    if LOW_POWER.load(std::sync::atomic::Ordering::Relaxed) {
        DEBOUNCE * 10
    } else {
        DEBOUNCE
    }
}

enum Msg {
    Paths(Vec<PathBuf>),
    Stop,
}

/// Running watcher. Dropping it stops the watcher and joins its thread.
#[derive(Debug)]
pub struct WatchHandle {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    watcher: Option<notify::RecommendedWatcher>,
    graph: Arc<Mutex<Graph>>,
}

impl WatchHandle {
    /// The watched graph (lock it to query between updates).
    pub fn graph(&self) -> Arc<Mutex<Graph>> {
        Arc::clone(&self.graph)
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.watcher.take();
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn lock(g: &Mutex<Graph>) -> Result<std::sync::MutexGuard<'_, Graph>> {
    g.lock().map_err(|_| Error::Poisoned)
}

fn internal(root: &Path, p: &Path) -> bool {
    p.strip_prefix(root)
        .ok()
        .and_then(|r| r.components().next())
        .is_some_and(|c| matches!(c.as_os_str().to_str(), Some(".git" | ".kor")))
}

/// Open the graph at `root`, bring it up to date, then keep it updated on file changes.
/// `callback` gets the result of every update that changed something (the first full update included).
pub fn watch(
    root: impl AsRef<Path>,
    callback: impl FnMut(Result<UpdateStats>) + Send + 'static,
) -> Result<WatchHandle> {
    let graph = Arc::new(Mutex::new(Graph::open(root)?));
    start(graph, callback, true)
}

/// Watch an already opened graph (no initial full update).
pub fn watch_graph(
    graph: Arc<Mutex<Graph>>,
    callback: impl FnMut(Result<UpdateStats>) + Send + 'static,
) -> Result<WatchHandle> {
    start(graph, callback, false)
}

fn start(
    graph: Arc<Mutex<Graph>>,
    mut callback: impl FnMut(Result<UpdateStats>) + Send + 'static,
    initial: bool,
) -> Result<WatchHandle> {
    let root = graph
        .lock()
        .map_err(|_| Error::Poisoned)?
        .root()
        .to_path_buf();
    let (tx, rx) = mpsc::channel::<Msg>();
    let ev_tx = tx.clone();
    let ev_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if matches!(ev.kind, EventKind::Access(_)) {
            return;
        }
        let paths: Vec<PathBuf> = ev
            .paths
            .into_iter()
            .filter(|p| !internal(&ev_root, p))
            .collect();
        if !paths.is_empty() {
            let _ = ev_tx.send(Msg::Paths(paths));
        }
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;
    let g = Arc::clone(&graph);
    let thread = std::thread::Builder::new()
        .name("kor-graph-watch".into())
        .spawn(move || {
            if initial {
                callback(lock(&g).and_then(|mut g| g.update()));
            }
            loop {
                let mut pending: Vec<PathBuf> = match rx.recv() {
                    Ok(Msg::Paths(p)) => p,
                    Ok(Msg::Stop) | Err(_) => return,
                };
                loop {
                    match rx.recv_timeout(debounce()) {
                        Ok(Msg::Paths(p)) => pending.extend(p),
                        Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
                pending.sort();
                pending.dedup();
                let res = lock(&g).and_then(|mut g| g.update_files(&pending));
                if !matches!(res, Ok(UpdateStats { changed: false, .. })) {
                    callback(res);
                }
            }
        })?;
    Ok(WatchHandle {
        tx,
        thread: Some(thread),
        watcher: Some(watcher),
        graph,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::fixture;
    use std::time::Instant;

    #[test]
    fn watcher_updates_and_stops() -> Result<()> {
        let dir = fixture();
        let (tx, rx) = mpsc::channel();
        let handle = watch(dir.path(), move |r| {
            let _ = tx.send(r.map(|s| s.parsed));
        })?;
        // Initial full build.
        let first = rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|e| Error::Git(e.to_string()))??;
        assert_eq!(first, 6);
        std::fs::write(
            dir.path().join("src/extra.rs"),
            "pub fn added_by_watcher() {}\n",
        )?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = false;
        while Instant::now() < deadline && !seen {
            if let Ok(Ok(_)) = rx.recv_timeout(Duration::from_millis(500)) {
                seen = handle
                    .graph()
                    .lock()
                    .map_err(|_| Error::Poisoned)?
                    .node("src/extra.rs#added_by_watcher")
                    .is_some();
            }
        }
        assert!(seen, "watcher did not pick up the new file");
        handle.stop();
        Ok(())
    }
}
