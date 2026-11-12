//! `file_search`: semantic search over the project's text files with the local embedding model
//! (`embed-e5-small` component, the default when installed; nothing leaves the machine) or
//! Venice / Hugging Face embeddings (see [`Embedder`]).
//!
//! Files are cut into line-aligned chunks, embedded once and cached in Kor's home (never inside
//! the project). Later searches embed only the query and the files that changed. Sending project
//! text to Venice is an upload, so the tool asks like a command unless a rule allows it; likely
//! secrets (`.env`, keys, chunks matching a secret pattern) are never sent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use base64::Engine as _;
use kor_core::ProviderError;
use kor_core::ToolSpec;
use kor_providers::huggingface::Hf;
use kor_providers::media::VeniceMedia;
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_MODEL: &str = "text-embedding-bge-m3";
const INDEX_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 3000;
const MAX_CHUNKS: usize = 6000;
const CHUNK_CHARS: usize = 1500;
const CHUNK_LINES: usize = 60;
const SNIPPET_CHARS: usize = 300;
const BATCH: usize = 64;
const MIN_SCORE: f32 = 0.2;
/// With a reranker, this many times `k` vector matches are read again with the query.
const RERANK_POOL: usize = 4;
const SKIP_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "composer.lock",
];
const SECRET_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "crt", "cer", "jks", "keystore"];

#[derive(Serialize, Deserialize, Clone)]
struct Chunk {
    start: usize,
    end: usize,
    snippet: String,
    vec: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct FileEntry {
    len: u64,
    mtime: u64,
    chunks: Vec<Chunk>,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    version: u32,
    model: String,
    files: HashMap<String, FileEntry>,
}

/// Where embeddings come from: the local model, Venice, or Hugging Face feature extraction.
#[async_trait]
pub trait Embedder: Send + Sync + std::fmt::Debug {
    /// Shown in the tool description ("sends file text to `<name>`").
    fn name(&self) -> &'static str;
    fn default_model(&self) -> &'static str;
    /// Embeds on this computer: nothing is uploaded, so the tool only reads.
    fn local(&self) -> bool {
        false
    }
    async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError>;
    /// A search query (models such as e5 embed questions differently from documents).
    async fn embed_query(
        &self,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        self.embed(model, texts).await
    }
    /// `(index into docs, relevance)` best first, when the provider has a reranker (§4.2).
    async fn rerank(
        &self,
        _query: &str,
        _docs: &[String],
    ) -> Option<Result<Vec<(usize, f64)>, ProviderError>> {
        None
    }
}

#[async_trait]
impl Embedder for kor_providers::extras::Cohere {
    fn name(&self) -> &'static str {
        "Cohere"
    }
    fn default_model(&self) -> &'static str {
        kor_providers::extras::COHERE_EMBED_MODEL
    }
    async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        kor_providers::extras::Cohere::embed(
            self,
            model,
            texts,
            kor_providers::extras::EmbedKind::Document,
        )
        .await
    }
    async fn embed_query(
        &self,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        kor_providers::extras::Cohere::embed(
            self,
            model,
            texts,
            kor_providers::extras::EmbedKind::Query,
        )
        .await
    }
    async fn rerank(
        &self,
        query: &str,
        docs: &[String],
    ) -> Option<Result<Vec<(usize, f64)>, ProviderError>> {
        Some(
            kor_providers::extras::Cohere::rerank(
                self,
                kor_providers::extras::COHERE_RERANK_MODEL,
                query,
                docs,
                docs.len(),
            )
            .await,
        )
    }
}

/// The installed local embedding model (shared by the process), if any. No network.
pub fn local_embedder() -> Option<std::sync::Arc<kor_memory::LocalEmbedder>> {
    kor_memory::LocalEmbedder::installed()
}

/// The local `embed-e5-small` model (`kor_memory::embed`), run on a blocking thread.
async fn local_embed(
    m: &std::sync::Arc<kor_memory::LocalEmbedder>,
    texts: &[String],
    kind: kor_memory::embed::Kind,
) -> Result<Vec<Vec<f32>>, ProviderError> {
    let (m, texts) = (m.clone(), texts.to_vec());
    let fail = |e: String| ProviderError::new("local", kor_core::ProviderErrorKind::Unavailable, e);
    tokio::task::spawn_blocking(move || kor_memory::Embed::embed(&*m, &texts, kind))
        .await
        .map_err(|e| fail(e.to_string()))?
        .map_err(fail)
}

#[async_trait]
impl Embedder for std::sync::Arc<kor_memory::LocalEmbedder> {
    fn name(&self) -> &'static str {
        "the local model"
    }
    fn default_model(&self) -> &'static str {
        kor_memory::embed::MODEL_NAME
    }
    fn local(&self) -> bool {
        true
    }
    async fn embed(&self, _: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        local_embed(self, texts, kor_memory::embed::Kind::Passage).await
    }
    async fn embed_query(&self, _: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        local_embed(self, texts, kor_memory::embed::Kind::Query).await
    }
}

#[async_trait]
impl Embedder for VeniceMedia {
    fn name(&self) -> &'static str {
        "Venice"
    }
    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }
    async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        VeniceMedia::embed(self, model, texts).await
    }
}

/// Hugging Face feature-extraction model used when HF is the embedder (the HF Inference
/// provider's example model: <https://huggingface.co/docs/inference-providers/providers/hf-inference>).
pub const HF_DEFAULT_MODEL: &str = "perplexity-ai/pplx-embed-v2-late-0.6b";

#[async_trait]
impl Embedder for Hf {
    fn name(&self) -> &'static str {
        "Hugging Face"
    }
    fn default_model(&self) -> &'static str {
        HF_DEFAULT_MODEL
    }
    async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        Hf::embed(self, model, texts).await
    }
}

#[derive(Debug)]
pub struct FileSearch {
    venice: Box<dyn Embedder>,
    /// Cache file for this project, under Kor's home.
    index_path: PathBuf,
    model: String,
    busy: tokio::sync::Mutex<()>,
}

impl FileSearch {
    /// `embedder` is the local model, a [`VeniceMedia`] or an [`Hf`] client; the index is per
    /// model, so switching re-embeds instead of mixing vectors.
    pub fn new(embedder: impl Embedder + 'static, index_dir: &Path, root: &Path) -> Self {
        let name = crate::memory::sanitize_path(&root.to_string_lossy().replace('\\', "/"));
        Self {
            model: embedder.default_model().to_owned(),
            venice: Box::new(embedder),
            index_path: index_dir.join(format!("{name}.json")),
            busy: tokio::sync::Mutex::new(()),
        }
    }
}

fn encode(v: &[f32]) -> String {
    let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(s: &str) -> Vec<f32> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .unwrap_or_default()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Files whose content must never leave the computer.
fn secret_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();
    name.starts_with(".env")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name == ".npmrc"
        || name == ".netrc"
        || name.contains("secret")
        || name.contains("credential")
        || Path::new(&name)
            .extension()
            .is_some_and(|e| SECRET_EXTENSIONS.iter().any(|x| e == *x))
}

/// Lock files and likely secrets, which no search returns (`walk` skips them as well).
pub(crate) fn walk_excluded(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    SKIP_NAMES.contains(&name) || secret_file(rel)
}

/// Line-aligned pieces `(first line, last line, text)`, 1-based.
pub(crate) fn chunk_text(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut start = 1;
    let mut lines = 0;
    for (i, line) in text.lines().enumerate() {
        if buf.is_empty() {
            start = i + 1;
        }
        let line: String = line.chars().take(CHUNK_CHARS).collect();
        buf.push_str(&line);
        buf.push('\n');
        lines += 1;
        if buf.len() >= CHUNK_CHARS || lines >= CHUNK_LINES {
            out.push((start, i + 1, std::mem::take(&mut buf)));
            lines = 0;
        }
    }
    if !buf.trim().is_empty() {
        out.push((start, start + lines - 1, buf));
    }
    out.retain(|(_, _, t)| !t.trim().is_empty());
    out
}

fn mtime_secs(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Indexable text files under `root`: `(relative path, absolute path, len, mtime)`.
pub(crate) fn walk(
    root: &Path,
    ai_ignore: Option<&ignore::gitignore::Gitignore>,
) -> Vec<(String, PathBuf, u64, u64)> {
    let mut out = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
    {
        if out.len() >= MAX_FILES {
            break;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        if ai_ignore.is_some_and(|g| g.matched_path_or_any_parents(path, false).is_ignore()) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        if SKIP_NAMES.contains(&name) || secret_file(&rel) {
            continue;
        }
        out.push((rel, path.to_owned(), meta.len(), mtime_secs(&meta)));
    }
    out
}

/// UTF-8 text without NUL bytes, else `None` (binary).
pub(crate) fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

impl FileSearch {
    fn load(&self) -> Index {
        std::fs::read(&self.index_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .filter(|i| i.version == INDEX_VERSION && i.model == self.model)
            .unwrap_or_else(|| Index {
                version: INDEX_VERSION,
                model: self.model.clone(),
                files: HashMap::new(),
            })
    }

    fn save(&self, index: &Index) -> std::io::Result<()> {
        if let Some(dir) = self.index_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.index_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(index)?)?;
        std::fs::rename(&tmp, &self.index_path)
    }

    /// Bring the index up to date with the files on disk. Returns how many files were embedded.
    async fn refresh(&self, ctx: &ToolCtx, index: &mut Index) -> Result<usize, String> {
        let (root, ai) = (ctx.root.clone(), ctx.ai_ignore.clone());
        let files = tokio::task::spawn_blocking(move || walk(&root, ai.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
        let mut keep: HashMap<String, FileEntry> = HashMap::new();
        // Texts to embed: (file, start, end, snippet, text sent).
        let mut todo: Vec<(String, usize, usize, String, String)> = Vec::new();
        let mut fresh: HashMap<String, (u64, u64)> = HashMap::new();
        let mut total = 0;
        for (rel, abs, len, mtime) in files {
            if let Some(old) = index.files.get(&rel)
                && old.len == len
                && old.mtime == mtime
            {
                total += old.chunks.len();
                if total > MAX_CHUNKS {
                    break;
                }
                keep.insert(rel, old.clone());
                continue;
            }
            let Some(text) = read_text(&abs) else {
                continue;
            };
            let chunks = chunk_text(&text);
            if total + chunks.len() > MAX_CHUNKS {
                break;
            }
            for (start, end, body) in chunks {
                if !kor_tools::scan_secrets(&body).is_empty() {
                    continue;
                }
                total += 1;
                let snippet: String = body.chars().take(SNIPPET_CHARS).collect();
                todo.push((rel.clone(), start, end, snippet, format!("{rel}\n{body}")));
            }
            fresh.insert(rel.clone(), (len, mtime));
            keep.entry(rel).or_insert(FileEntry {
                len,
                mtime,
                chunks: Vec::new(),
            });
        }
        let embedded = fresh.len();
        for batch in todo.chunks(BATCH) {
            if ctx.cancel.is_cancelled() {
                return Err("cancelled".into());
            }
            let texts: Vec<String> = batch.iter().map(|t| t.4.clone()).collect();
            let vecs = self
                .venice
                .embed(&self.model, &texts)
                .await
                .map_err(|e| e.to_string())?;
            for ((rel, start, end, snippet, _), v) in batch.iter().zip(vecs) {
                if let Some(entry) = keep.get_mut(rel) {
                    entry.chunks.push(Chunk {
                        start: *start,
                        end: *end,
                        snippet: snippet.clone(),
                        vec: encode(&v),
                    });
                }
            }
        }
        index.files = keep;
        Ok(embedded)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchIn {
    query: String,
    #[serde(default)]
    k: Option<usize>,
}

#[async_trait]
impl Tool for FileSearch {
    fn name(&self) -> &'static str {
        "file_search"
    }
    fn rule_name(&self) -> &'static str {
        "file_search"
    }
    // Project text is uploaded to Venice / Hugging Face for embedding; the local model only reads.
    fn kind(&self) -> ToolKind {
        if self.venice.local() {
            ToolKind::Read
        } else {
            ToolKind::Execute
        }
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: format!(
                "Semantic search over the project's text and code files: finds places related in meaning (\"where are sessions saved\", \"how is auth checked\"), returning `path:start-end` with a short excerpt. Use Grep for exact text and names; use this when you do not know the words the code uses. The first call indexes the project ({} for embedding, skipping .gitignored files and likely secrets) and is slow; later calls only embed changed files.",
                if self.venice.local() {
                    format!(
                        "uses {} on this computer, nothing is uploaded,",
                        self.venice.name()
                    )
                } else {
                    format!("sends file text to {}", self.venice.name())
                }
            ),
            input_schema: json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "What you are looking for, in plain words"},
                "k": {"type": "integer", "minimum": 1, "maximum": 20, "description": "Results (default 8)"}
            }, "required": ["query"], "additionalProperties": false}),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        input["query"].as_str().map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let a: SearchIn = match serde_json::from_value(input) {
            Ok(a) => a,
            Err(e) => return ToolOutput::err(format!("invalid input: {e}")),
        };
        let query = a.query.trim();
        if query.chars().count() < 2 || query.len() > 1000 {
            return ToolOutput::err("query must be 2 to 1000 characters");
        }
        let k = a.k.unwrap_or(8).clamp(1, 20);
        let found = match self.search(ctx, query, k).await {
            Ok(f) => f,
            Err(e) => return ToolOutput::err(e),
        };
        if found.chunks == 0 {
            return ToolOutput::ok("No text files to index in this project.");
        }
        if found.hits.is_empty() {
            return ToolOutput::ok("Nothing related in meaning was found.");
        }
        let note = if found.embedded > 0 {
            format!(
                "(indexed {} changed file(s); {} chunks in total)

",
                found.embedded, found.chunks
            )
        } else {
            String::new()
        };
        let body: Vec<String> = found
            .hits
            .iter()
            .map(|h| {
                format!(
                    "{}:{}-{} (similarity {:.2})
{}",
                    h.file,
                    h.start,
                    h.end,
                    h.score,
                    h.snippet.trim_end()
                )
            })
            .collect();
        ToolOutput::ok(format!(
            "{note}{}",
            body.join(
                "

---

"
            )
        ))
    }
}

/// One semantic match: `file` is project-relative, lines are 1-based.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticHit {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub score: f32,
    pub snippet: String,
}

/// [`FileSearch::search`] result: hits, files embedded by this call, chunks in the index.
#[derive(Debug, Clone, Default)]
pub struct SemanticFound {
    pub hits: Vec<SemanticHit>,
    pub embedded: usize,
    pub chunks: usize,
}

impl FileSearch {
    /// Bring the index up to date, then the `k` chunks closest to `query` (used by `file_search`
    /// and `kor find`).
    pub async fn search(
        &self,
        ctx: &ToolCtx,
        query: &str,
        k: usize,
    ) -> Result<SemanticFound, String> {
        let _guard = self.busy.lock().await;
        let mut index = self.load();
        let embedded = self
            .refresh(ctx, &mut index)
            .await
            .map_err(|e| format!("indexing failed: {e}"))?;
        if embedded > 0 {
            self.save(&index)
                .map_err(|e| format!("could not save the index: {e}"))?;
        }
        let chunks: usize = index.files.values().map(|f| f.chunks.len()).sum();
        if chunks == 0 {
            return Ok(SemanticFound {
                hits: Vec::new(),
                embedded,
                chunks,
            });
        }
        let q = match self
            .venice
            .embed_query(&self.model, &[query.to_owned()])
            .await
        {
            Ok(mut v) if !v.is_empty() => v.swap_remove(0),
            Ok(_) => return Err("the query could not be embedded".into()),
            Err(e) => return Err(e.to_string()),
        };
        let mut hits: Vec<SemanticHit> = index
            .files
            .iter()
            .flat_map(|(rel, f)| f.chunks.iter().map(move |c| (rel, c)))
            .map(|(rel, c)| SemanticHit {
                file: rel.clone(),
                start: c.start,
                end: c.end,
                score: cosine(&q, &decode(&c.vec)),
                snippet: c.snippet.clone(),
            })
            .filter(|h| h.score >= MIN_SCORE)
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        // A reranker (Cohere) reorders the closest candidates by reading them with the query; a
        // failed rerank keeps the vector order.
        hits.truncate(k.saturating_mul(RERANK_POOL));
        let docs: Vec<String> = hits
            .iter()
            .map(|h| format!("{}\n{}", h.file, h.snippet))
            .collect();
        if hits.len() > 1
            && let Some(Ok(order)) = self.venice.rerank(query, &docs).await
        {
            let mut by_rank: Vec<SemanticHit> = Vec::with_capacity(order.len());
            for (i, score) in order {
                if let Some(h) = hits.get(i) {
                    #[allow(clippy::cast_possible_truncation)] // relevance in 0–1
                    by_rank.push(SemanticHit {
                        score: score as f32,
                        ..h.clone()
                    });
                }
            }
            hits = by_rank;
        }
        hits.truncate(k);
        Ok(SemanticFound {
            hits,
            embedded,
            chunks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors favour `alpha.rs`; the reranker prefers `beta.rs`.
    #[derive(Debug)]
    struct Reranking;

    #[async_trait]
    impl Embedder for Reranking {
        fn name(&self) -> &'static str {
            "test"
        }
        fn default_model(&self) -> &'static str {
            "test"
        }
        async fn embed(&self, _: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
            Ok(texts
                .iter()
                .map(|t| {
                    if t.contains("alpha") || t == "login" {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.8, 0.6]
                    }
                })
                .collect())
        }
        async fn rerank(
            &self,
            _: &str,
            docs: &[String],
        ) -> Option<Result<Vec<(usize, f64)>, ProviderError>> {
            let mut order: Vec<(usize, f64)> = docs
                .iter()
                .enumerate()
                .map(|(i, d)| (i, if d.contains("beta") { 0.9 } else { 0.1 }))
                .collect();
            order.sort_by(|a, b| b.1.total_cmp(&a.1));
            Some(Ok(order))
        }
    }

    #[tokio::test]
    async fn a_reranker_reorders_the_vector_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.rs"), "fn alpha_login() {}\n").unwrap();
        std::fs::write(dir.path().join("beta.rs"), "fn beta_sign_in() {}\n").unwrap();
        let index = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let s = FileSearch::new(Reranking, index.path(), &ctx.root);
        let found = s.search(&ctx, "login", 2).await.unwrap();
        let files: Vec<&str> = found.hits.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(files, ["beta.rs", "alpha.rs"], "{found:?}");
        assert!((found.hits[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn chunks_cover_every_line_once() {
        let text = (1..=130)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 3);
        assert_eq!((chunks[0].0, chunks[0].1), (1, 60));
        assert_eq!((chunks[2].0, chunks[2].1), (121, 130));
        assert!(chunk_text("\n\n  \n").is_empty());
    }

    #[test]
    fn vectors_round_trip_and_compare() {
        let v = vec![0.5f32, -1.25, 3.0];
        assert_eq!(decode(&encode(&v)), v);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!(cosine(&[1.0], &[1.0, 2.0]) == 0.0);
    }

    #[test]
    fn secret_files_are_never_indexed() {
        for f in [
            ".env",
            "app/.env.local",
            "certs/server.pem",
            "id_rsa",
            "config/secrets.yml",
        ] {
            assert!(secret_file(f), "{f}");
        }
        for f in ["src/main.rs", "README.md", "docs/environment.md"] {
            assert!(!secret_file(f), "{f}");
        }
    }

    #[test]
    fn walk_skips_ignored_binary_and_secret_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        std::fs::create_dir(root.join("build")).unwrap();
        std::fs::write(root.join("build/out.txt"), "x").unwrap();
        std::fs::write(root.join(".env"), "KEY=1").unwrap();
        std::fs::write(root.join("Cargo.lock"), "x").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        let ctx = ToolCtx::new(root).unwrap();
        let files: Vec<String> = walk(&ctx.root, None).into_iter().map(|f| f.0).collect();
        assert_eq!(files, vec!["main.rs"]);
    }
}
