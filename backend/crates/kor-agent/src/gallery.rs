//! Media gallery, derivation chain, media cost and resumable jobs (§20.3, §20.4, ADR 0060).
//!
//! Every file a media tool saves under `<project>/kor-media/` gets a sidecar `<file>.kor.json`
//! (plain JSON next to the output): prompt, provider, model, parameters, seed, cost, time, and the
//! inputs it came from. A reference that is itself in the gallery is the item's `parent`; the
//! chain of parents is the derivation history (also a graph layer, `kor_graph::MediaItem`).
//! A paid generation also goes to the usage ledger as a media line (`kor cost` shows media
//! spending separately). A queued fal or Venice request is written to `kor-media/.jobs/` while
//! it runs, so a request outliving its process can be collected later (`kor media collect`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::media::MEDIA_DIR;

/// Suffix of a gallery sidecar.
pub const SIDECAR: &str = ".kor.json";
const JOBS: &str = ".jobs";
/// Parents followed at most (a cycle cannot loop forever).
const MAX_CHAIN: usize = 64;

/// One generated file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    /// File name inside `kor-media/` (unique there).
    pub id: String,
    /// Project-relative path with `/`.
    pub file: String,
    /// `image`, `video`, `audio` or `other`.
    pub kind: String,
    /// Catalog capability key (`image`, `upscale`, `video`...), when known.
    pub capability: Option<String>,
    pub provider: String,
    pub model: String,
    pub prompt: String,
    pub params: Value,
    pub seed: Option<i64>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    /// The cost is an estimate from list prices.
    pub estimated: bool,
    pub secs: u64,
    /// Unix milliseconds.
    pub created: u64,
    /// References used: gallery ids, or project paths for files outside the gallery.
    pub inputs: Vec<String>,
    /// The first input that is a gallery item.
    pub parent: Option<String>,
    /// Quick action that made it (`variation`, `reseed`, `upscale`, `video`, `remove-bg`, `edit`).
    pub action: Option<String>,
}

/// What a generation knew about itself, for [`record`].
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub capability: Option<String>,
    pub provider: String,
    pub model: String,
    pub prompt: String,
    pub params: Value,
    pub seed: Option<i64>,
    /// Amount, currency, estimated.
    pub cost: Option<(f64, String, bool)>,
    pub secs: u64,
    /// Reference files (absolute).
    pub inputs: Vec<PathBuf>,
    pub action: Option<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

pub fn dir(root: &Path) -> PathBuf {
    root.join(MEDIA_DIR)
}

/// `image` / `video` / `audio` / `other` by extension.
pub fn kind_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "webp" | "gif") => "image",
        Some("mp4" | "webm" | "mov") => "video",
        Some("mp3" | "wav" | "ogg" | "flac" | "m4a") => "audio",
        _ => "other",
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The gallery id of `p` when it is a file directly in `kor-media/`.
fn gallery_id(root: &Path, p: &Path) -> Option<String> {
    let d = dir(root);
    let parent = p.parent()?;
    let same = parent == d || std::fs::canonicalize(parent).ok() == std::fs::canonicalize(&d).ok();
    same.then(|| p.file_name()?.to_str().map(str::to_owned))?
}

/// The seed a provider reports in its result (`seed`), else the one that was asked for.
pub fn seed_of(result: &Value, input: &Value) -> Option<i64> {
    result
        .get("seed")
        .and_then(Value::as_i64)
        .or_else(|| input.get("seed").and_then(Value::as_i64))
}

/// Write a sidecar for each of `paths`; returns the items. Errors are ignored: a missing
/// sidecar must not lose the generated file.
pub fn record(root: &Path, paths: &[PathBuf], m: &Meta) -> Vec<Item> {
    // Relative inputs are project paths (`kor-media/<id>` for "derived from" without upload).
    let abs: Vec<PathBuf> = m
        .inputs
        .iter()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                root.join(p)
            }
        })
        .collect();
    let inputs: Vec<String> = abs
        .iter()
        .map(|p| gallery_id(root, p).unwrap_or_else(|| rel(root, p)))
        .collect();
    let parent = abs.iter().find_map(|p| gallery_id(root, p));
    let n = paths.len().max(1);
    let mut out = Vec::new();
    for p in paths {
        let Some(id) = p.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        #[allow(clippy::cast_precision_loss)] // a handful of files
        let share = |c: f64| c / n as f64;
        let item = Item {
            id: id.clone(),
            file: rel(root, p),
            kind: kind_of(p).into(),
            capability: m.capability.clone(),
            provider: m.provider.clone(),
            model: m.model.clone(),
            prompt: m.prompt.clone(),
            params: without_inline_files(&m.params),
            seed: m.seed,
            cost: m.cost.as_ref().map(|c| share(c.0)),
            currency: m.cost.as_ref().map(|c| c.1.clone()),
            estimated: m.cost.as_ref().is_some_and(|c| c.2),
            secs: m.secs,
            created: now_ms(),
            inputs: inputs.clone(),
            parent: parent.clone(),
            action: m.action.clone(),
        };
        if let Ok(s) = serde_json::to_string_pretty(&item) {
            let _ = std::fs::write(sidecar(p), s);
        }
        out.push(item);
    }
    out
}

/// Inputs as kept in a sidecar: inline files (`data:` URLs, megabytes) become a marker; the
/// file itself is in `inputs`.
fn without_inline_files(v: &Value) -> Value {
    match v {
        Value::String(s) if s.starts_with("data:") => Value::String("(inline file)".into()),
        Value::Array(a) => Value::Array(a.iter().map(without_inline_files).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), without_inline_files(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn sidecar(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(SIDECAR);
    PathBuf::from(s)
}

/// Every file in the gallery, newest first. Files without a sidecar (copied in by hand) are
/// listed with what the file itself says.
pub fn list(root: &Path) -> Vec<Item> {
    let Ok(rd) = std::fs::read_dir(dir(root)) else {
        return Vec::new();
    };
    let mut out: Vec<Item> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                let lower = n.to_ascii_lowercase();
                !n.starts_with('.') && !lower.ends_with(SIDECAR) && !lower.ends_with(".part")
            })
        })
        .map(|p| read_item(root, &p))
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then(a.id.cmp(&b.id)));
    out
}

fn read_item(root: &Path, p: &Path) -> Item {
    let stored = std::fs::read_to_string(sidecar(p))
        .ok()
        .and_then(|s| serde_json::from_str::<Item>(&s).ok());
    stored.unwrap_or_else(|| Item {
        id: p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file: rel(root, p),
        kind: kind_of(p).into(),
        created: std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0)),
        ..Item::default()
    })
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains(['/', '\\']) && !id.contains("..")
}

/// One item by id.
pub fn get(root: &Path, id: &str) -> Option<Item> {
    if !valid_id(id) {
        return None;
    }
    let p = dir(root).join(id);
    p.is_file().then(|| read_item(root, &p))
}

/// The item and its ancestors, newest first.
pub fn chain(root: &Path, id: &str) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut next = Some(id.to_owned());
    while let Some(id) = next.take() {
        if out.len() >= MAX_CHAIN || out.iter().any(|i| i.id == id) {
            break;
        }
        let Some(item) = get(root, &id) else { break };
        next.clone_from(&item.parent);
        out.push(item);
    }
    out
}

/// Items made from `id`.
pub fn children(root: &Path, id: &str) -> Vec<Item> {
    list(root)
        .into_iter()
        .filter(|i| i.parent.as_deref() == Some(id) || i.inputs.iter().any(|x| x == id))
        .collect()
}

/// A reference given by the user: a gallery id, or a path inside the project.
///
/// # Errors
/// When it names nothing, or a path outside the project.
pub fn resolve(root: &Path, s: &str) -> Result<PathBuf, String> {
    if valid_id(s) {
        let p = dir(root).join(s);
        if p.is_file() {
            return Ok(p);
        }
    }
    let p = kor_tools::resolve(root, s)?;
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("no such file or gallery item: {s}"))
    }
}

/// Average seconds of earlier runs of `model` in this gallery (§20.2 time estimate).
pub fn eta(root: &Path, model: &str) -> Option<u64> {
    let secs: Vec<u64> = list(root)
        .into_iter()
        .filter(|i| i.model == model && i.secs > 0)
        .map(|i| i.secs)
        .collect();
    let n = u64::try_from(secs.len()).ok().filter(|n| *n > 0)?;
    Some(secs.iter().sum::<u64>() / n)
}

/// Copy a gallery item into the project (`public/logo.png`): never over an existing file.
///
/// # Errors
/// Unknown item, a destination outside the project, or an existing destination.
pub fn export(root: &Path, id: &str, dest: &str) -> Result<PathBuf, String> {
    let item = get(root, id).ok_or_else(|| format!("no such gallery item: {id}"))?;
    let to = kor_tools::resolve(root, dest)?;
    if to.exists() {
        return Err(format!("{dest} already exists"));
    }
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::copy(root.join(&item.file), &to).map_err(|e| e.to_string())?;
    Ok(to)
}

// ---------------------------------------------------------------------------------------------
// Media cost in the usage ledger
// ---------------------------------------------------------------------------------------------

/// One ledger line for a paid generation. Same fields as `kor_runtime::usage::Entry` (which
/// reads them back), with `media` set: `kor cost` lists media spending on its own line.
pub fn ledger_line(model: &str, usd: f64, estimated: bool) -> Value {
    json!({
        "at": now_ms(),
        "model": model,
        "prompt": 0,
        "completion": 0,
        "cost_usd": usd,
        "media": true,
        "estimated": estimated,
    })
}

/// Append a media cost (USD only; the ledger is in dollars). Errors are ignored.
pub fn charge(ledger: Option<&Path>, model: &str, cost: Option<&(f64, String, bool)>) {
    let (Some(path), Some((usd, cur, est))) = (ledger, cost) else {
        return;
    };
    if cur != "USD" || !usd.is_finite() || *usd < 0.0 {
        return;
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = std::io::Write::write_all(
            &mut f,
            format!("{}\n", ledger_line(model, *usd, *est)).as_bytes(),
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Resumable jobs
// ---------------------------------------------------------------------------------------------

/// A queued request that has not been collected yet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Job {
    pub id: String,
    /// `fal` or `venice`.
    pub provider: String,
    pub model: String,
    /// fal: request id and the request path; Venice: the queue id in `request_id`.
    pub request_id: String,
    pub path: String,
    pub prompt: String,
    pub stem: String,
    pub input: Value,
    pub capability: Option<String>,
    pub inputs: Vec<PathBuf>,
    pub action: Option<String>,
    pub cost: Option<(f64, String, bool)>,
    pub started: u64,
}

fn jobs_dir(root: &Path) -> PathBuf {
    dir(root).join(JOBS)
}

/// Remember a running request; returns its id.
pub fn job_start(root: &Path, mut job: Job) -> String {
    job.id = format!(
        "{}-{}",
        job.provider,
        job.request_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(32)
            .collect::<String>()
    );
    job.started = now_ms();
    let d = jobs_dir(root);
    if std::fs::create_dir_all(&d).is_ok()
        && let Ok(s) = serde_json::to_string_pretty(&job)
    {
        let _ = std::fs::write(d.join(format!("{}.json", job.id)), s);
    }
    job.id
}

/// The request was collected (or failed for good).
pub fn job_done(root: &Path, id: &str) {
    if valid_id(id) {
        let _ = std::fs::remove_file(jobs_dir(root).join(format!("{id}.json")));
    }
}

/// Requests not collected yet, oldest first.
pub fn jobs(root: &Path) -> Vec<Job> {
    let Ok(rd) = std::fs::read_dir(jobs_dir(root)) else {
        return Vec::new();
    };
    let mut out: Vec<Job> = rd
        .filter_map(Result::ok)
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect();
    out.sort_by_key(|j| j.started);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, name: &str) -> PathBuf {
        let p = dir(root).join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    }

    #[test]
    fn records_chain_children_and_eta() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let a = write(root, "cat-a.png");
        let meta = Meta {
            capability: Some("image".into()),
            provider: "fal".into(),
            model: "fal-ai/flux/schnell".into(),
            prompt: "a cat".into(),
            params: json!({"seed": 7, "image_url": "data:image/png;base64,AAAA"}),
            seed: Some(7),
            cost: Some((0.006, "USD".into(), true)),
            secs: 4,
            ..Meta::default()
        };
        let items = record(root, std::slice::from_ref(&a), &meta);
        assert_eq!(items[0].id, "cat-a.png");
        assert_eq!(
            items[0].params["image_url"], "(inline file)",
            "no megabytes in sidecars"
        );
        assert!(dir(root).join("cat-a.png.kor.json").is_file());
        let b = write(root, "cat-b.png");
        record(
            root,
            &[b],
            &Meta {
                inputs: vec![a.clone()],
                action: Some("upscale".into()),
                secs: 8,
                ..meta.clone()
            },
        );
        let c = chain(root, "cat-b.png");
        assert_eq!(
            c.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["cat-b.png", "cat-a.png"]
        );
        assert_eq!(c[0].action.as_deref(), Some("upscale"));
        assert_eq!(children(root, "cat-a.png").len(), 1);
        assert_eq!(eta(root, "fal-ai/flux/schnell"), Some(6));
        assert_eq!(list(root).len(), 2, "sidecars are not items");
        assert_eq!(resolve(root, "cat-a.png").unwrap(), a);
        assert!(get(root, "../x").is_none());
        let out = export(root, "cat-a.png", "public/logo.png").unwrap();
        assert!(out.is_file());
        assert!(
            export(root, "cat-a.png", "public/logo.png").is_err(),
            "never overwrites"
        );
    }

    #[test]
    fn ledger_and_jobs() {
        let t = tempfile::tempdir().unwrap();
        let ledger = t.path().join("usage.jsonl");
        charge(Some(&ledger), "fal/x", Some(&(0.25, "USD".into(), true)));
        charge(Some(&ledger), "fal/x", Some(&(0.25, "EUR".into(), false)));
        let text = std::fs::read_to_string(&ledger).unwrap();
        assert_eq!(text.lines().count(), 1, "only dollars go to the ledger");
        let v: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(
            (v["media"].as_bool(), v["estimated"].as_bool()),
            (Some(true), Some(true))
        );

        let root = t.path();
        let id = job_start(
            root,
            Job {
                provider: "fal".into(),
                request_id: "r-1".into(),
                ..Job::default()
            },
        );
        assert_eq!(id, "fal-r1");
        assert_eq!(jobs(root).len(), 1);
        job_done(root, &id);
        assert!(jobs(root).is_empty());
    }
}
