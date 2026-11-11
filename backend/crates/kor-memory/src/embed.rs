//! Local text embeddings and the in-memory vector index (ADR 0023).
//!
//! [`Local`] runs `intfloat/multilingual-e5-small` (MIT, 384 dimensions, Turkish/English/code)
//! on the CPU with `candle`, from the `embed-e5-small` component (downloaded and verified on
//! demand by `kor-components`, never bundled). Nothing leaves the machine.
//!
//! Vectors are stored and searched as int8 with one scale per vector ([`quantize`]): 388 bytes
//! per record instead of 1536, and [`VecIndex::top`] is an exact, multi-threaded scan.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

/// Component with the model weights (it requires [`TOKENIZER_COMPONENT`]).
pub const MODEL_COMPONENT: &str = "embed-e5-small";
pub const TOKENIZER_COMPONENT: &str = "embed-e5-small-tokenizer";
/// Model name stored next to every vector; a different model re-embeds.
pub const MODEL_NAME: &str = "multilingual-e5-small@614241f6";
const DIMS: usize = 384;
const MAX_TOKENS: usize = 512;

/// `config.json` of the pinned revision.
const CONFIG: &str = r#"{"architectures":["BertModel"],"attention_probs_dropout_prob":0.1,
"hidden_act":"gelu","hidden_dropout_prob":0.1,"hidden_size":384,"initializer_range":0.02,
"intermediate_size":1536,"layer_norm_eps":1e-12,"max_position_embeddings":512,"model_type":"bert",
"num_attention_heads":12,"num_hidden_layers":12,"pad_token_id":0,"position_embedding_type":"absolute",
"type_vocab_size":2,"use_cache":true,"vocab_size":250037}"#;

/// What a text is for (e5 embeds questions and documents with different prefixes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Query,
    Passage,
}

/// A text embedder. Vectors are L2-normalized.
pub trait Embed: Send + Sync {
    /// Stored with every vector; vectors of another model are not compared.
    fn model(&self) -> &str;
    /// Below this cosine similarity a vector match is noise and is dropped.
    fn min_similarity(&self) -> f32;
    /// Matches further than this below the best match of a query are dropped too.
    fn similarity_band(&self) -> f32 {
        1.0
    }
    /// # Errors
    /// When the model cannot be loaded or run.
    fn embed(&self, texts: &[String], kind: Kind) -> Result<Vec<Vec<f32>>, String>;
}

/// multilingual-e5-small on the CPU, loaded on first use.
pub struct Local {
    model: PathBuf,
    tokenizer: PathBuf,
    loaded: OnceLock<Result<(BertModel, Tokenizer), String>>,
    /// One forward pass at a time: the matmuls already use every core.
    run: Mutex<()>,
}

impl std::fmt::Debug for Local {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Local")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

static SHARED: OnceLock<Arc<Local>> = OnceLock::new();

impl Local {
    pub fn new(model: PathBuf, tokenizer: PathBuf) -> Self {
        Self {
            model,
            tokenizer,
            loaded: OnceLock::new(),
            run: Mutex::new(()),
        }
    }

    /// The installed model, shared by the whole process (loaded once); `None` when the
    /// `embed-e5-small` component is not installed. No network.
    pub fn installed() -> Option<Arc<Self>> {
        if let Some(l) = SHARED.get() {
            return Some(l.clone());
        }
        let model = kor_components::path(MODEL_COMPONENT)?;
        let tokenizer = kor_components::path(TOKENIZER_COMPONENT)?;
        Some(
            SHARED
                .get_or_init(|| Arc::new(Self::new(model, tokenizer)))
                .clone(),
        )
    }

    fn load(&self) -> Result<(BertModel, Tokenizer), String> {
        let config: Config = serde_json::from_str(CONFIG).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&self.model).map_err(|e| format!("embedding model: {e}"))?;
        let vb = VarBuilder::from_buffered_safetensors(bytes, DType::F32, &Device::Cpu)
            .map_err(|e| format!("embedding model: {e}"))?;
        let model = BertModel::load(vb, &config).map_err(|e| format!("embedding model: {e}"))?;
        let mut tok =
            Tokenizer::from_file(&self.tokenizer).map_err(|e| format!("tokenizer: {e}"))?;
        tok.with_padding(Some(PaddingParams {
            pad_id: 1,
            pad_token: "<pad>".into(),
            ..PaddingParams::default()
        }));
        tok.with_truncation(Some(TruncationParams {
            max_length: MAX_TOKENS,
            ..TruncationParams::default()
        }))
        .map_err(|e| format!("tokenizer: {e}"))?;
        Ok((model, tok))
    }

    fn forward(&self, texts: Vec<String>) -> candle_core::Result<Vec<Vec<f32>>> {
        let (model, tok) = match self.loaded.get_or_init(|| self.load()) {
            Ok(m) => m,
            Err(e) => return Err(candle_core::Error::Msg(e.clone())),
        };
        let enc = tok
            .encode_batch(texts, true)
            .map_err(|e| candle_core::Error::Msg(e.to_string()))?;
        let ids: Vec<Vec<u32>> = enc.iter().map(|e| e.get_ids().to_vec()).collect();
        let mask: Vec<Vec<u32>> = enc
            .iter()
            .map(|e| e.get_attention_mask().to_vec())
            .collect();
        let dev = &Device::Cpu;
        let ids = Tensor::new(ids, dev)?;
        let mask = Tensor::new(mask, dev)?;
        let types = ids.zeros_like()?;
        let _one = self.run.lock();
        let hidden = model.forward(&ids, &types, Some(&mask))?;
        // Mean of the real tokens (sentence-transformers pooling), then L2 normalization.
        let m = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let sum = hidden.broadcast_mul(&m)?.sum(1)?;
        let mean = sum.broadcast_div(&m.sum(1)?)?;
        let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
        mean.broadcast_div(&norm)?.to_vec2()
    }
}

impl Embed for Local {
    fn model(&self) -> &str {
        MODEL_NAME
    }

    /// e5 similarities sit high: unrelated record-sized texts score 0.75-0.82, related ones
    /// 0.83-0.93 (measured on Turkish/English records, ADR 0023).
    fn min_similarity(&self) -> f32 {
        0.80
    }

    fn similarity_band(&self) -> f32 {
        0.04
    }

    fn embed(&self, texts: &[String], kind: Kind) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prefix = match kind {
            Kind::Query => "query: ",
            Kind::Passage => "passage: ",
        };
        let texts = texts.iter().map(|t| format!("{prefix}{t}")).collect();
        let out = self.forward(texts).map_err(|e| e.to_string())?;
        if out.iter().any(|v| v.len() != DIMS) {
            return Err("embedding model returned a wrong size".into());
        }
        Ok(out)
    }
}

/// Per-vector symmetric int8: `v ≈ q / scale`. Stored as 4 bytes of little-endian `1/scale`
/// followed by the int8 values.
pub fn quantize(v: &[f32]) -> Vec<u8> {
    let (inv, q) = to_i8(v);
    let mut out = Vec::with_capacity(4 + v.len());
    out.extend_from_slice(&inv.to_le_bytes());
    out.extend(q.iter().map(|x| x.to_le_bytes()[0]));
    out
}

fn to_i8(v: &[f32]) -> (f32, Vec<i8>) {
    let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    let scale = if max > 0.0 { 127.0 / max } else { 1.0 };
    // In range by construction: |x * scale| <= 127.
    #[allow(clippy::cast_possible_truncation)]
    let q = v.iter().map(|x| (x * scale).round() as i8).collect();
    (1.0 / scale, q)
}

/// `(1/scale, values)` of a stored vector, or `None` if it is malformed.
fn split(blob: &[u8]) -> Option<(f32, &[u8])> {
    let (head, rest) = blob.split_first_chunk::<4>()?;
    let inv = f32::from_le_bytes(*head);
    (inv.is_finite() && !rest.is_empty()).then_some((inv, rest))
}

fn dot(a: &[i8], b: &[i8]) -> i32 {
    // Pairs of i8 products summed in i16 (|x|, |y| <= 127, so a pair is <= 32258), then
    // widened: the shape compilers turn into multiply-add instructions (`pmaddwd`).
    let mut acc = [0i32; 8];
    let (ca, cb) = (a.chunks_exact(16), b.chunks_exact(16));
    let rest: i32 = ca
        .remainder()
        .iter()
        .zip(cb.remainder())
        .map(|(x, y)| i32::from(*x) * i32::from(*y))
        .sum();
    for (x, y) in ca.zip(cb) {
        for j in 0..8 {
            let p = i16::from(x[2 * j]) * i16::from(y[2 * j])
                + i16::from(x[2 * j + 1]) * i16::from(y[2 * j + 1]);
            acc[j] += i32::from(p);
        }
    }
    acc.iter().sum::<i32>() + rest
}

/// Exact top-k cosine search over int8 vectors held in one contiguous buffer.
#[derive(Debug, Default)]
pub struct VecIndex {
    dims: usize,
    ids: Vec<i64>,
    inv: Vec<f32>,
    data: Vec<i8>,
}

impl VecIndex {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Add a stored ([`quantize`]d) vector. Vectors of another size than the first are skipped.
    pub fn push(&mut self, id: i64, blob: &[u8]) {
        let Some((inv, q)) = split(blob) else {
            return;
        };
        if self.dims == 0 {
            self.dims = q.len();
        }
        if q.len() != self.dims {
            return;
        }
        self.ids.push(id);
        self.inv.push(inv);
        // -128 is never written by `quantize`; clamping keeps `dot`'s i16 pairs in range.
        self.data
            .extend(q.iter().map(|b| i8::from_le_bytes([*b]).max(-127)));
    }

    pub fn remove(&mut self, id: i64) {
        let Some(i) = self.ids.iter().position(|x| *x == id) else {
            return;
        };
        let last = self.ids.len() - 1;
        self.ids.swap_remove(i);
        self.inv.swap_remove(i);
        if i != last {
            let (d, src) = (self.dims, last * self.dims);
            self.data.copy_within(src..src + d, i * d);
        }
        self.data.truncate(last * self.dims);
    }

    /// The `k` most similar ids to `query` (an unquantized, normalized vector), with their
    /// approximate cosine similarity, best first.
    #[allow(clippy::many_single_char_names)]
    pub fn top(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dims || k == 0 {
            return Vec::new();
        }
        let (qinv, q) = to_i8(query);
        let q = &q[..];
        let n = self.ids.len();
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(n.div_ceil(16_384))
            .max(1);
        let per = n.div_ceil(threads);
        let scan = |from: usize| {
            let to = (from + per).min(n);
            let mut best = Top::new(k);
            for i in from..to {
                let v = &self.data[i * self.dims..(i + 1) * self.dims];
                // i32 → f32 rounding is far below int8 quantization error.
                #[allow(clippy::cast_precision_loss)]
                let s = dot(q, v) as f32 * self.inv[i];
                best.offer(s, i);
            }
            best.0
        };
        let mut all: Vec<(f32, usize)> = if threads == 1 {
            scan(0)
        } else {
            std::thread::scope(|s| {
                let hs: Vec<_> = (0..threads)
                    .map(|t| s.spawn(move || scan(t * per)))
                    .collect();
                hs.into_iter()
                    .flat_map(|h| h.join().unwrap_or_default())
                    .collect()
            })
        };
        all.sort_by(|a, b| b.0.total_cmp(&a.0));
        all.truncate(k);
        all.into_iter()
            .map(|(s, i)| (self.ids[i], s * qinv))
            .collect()
    }
}

/// Bounded best-k list (k is small, so a sorted insert beats a heap).
struct Top(Vec<(f32, usize)>, usize);

impl Top {
    fn new(k: usize) -> Self {
        Self(Vec::with_capacity(k + 1), k)
    }

    fn offer(&mut self, s: f32, i: usize) {
        if self.0.len() == self.1 && self.0.last().is_some_and(|w| s <= w.0) {
            return;
        }
        let at = self.0.partition_point(|x| x.0 >= s);
        self.0.insert(at, (s, i));
        self.0.truncate(self.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    #[test]
    fn quantized_dot_approximates_cosine() {
        let a = unit(&[0.3, -0.2, 0.9, 0.05]);
        let b = unit(&[0.25, -0.1, 0.8, 0.3]);
        let exact: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let mut ix = VecIndex::default();
        ix.push(7, &quantize(&b));
        let got = ix.top(&a, 3);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 7);
        assert!((got[0].1 - exact).abs() < 0.02, "{} vs {exact}", got[0].1);
        assert!(split(&[0, 0]).is_none());
    }

    #[test]
    fn top_k_is_exact_order_and_remove_keeps_rows_aligned() {
        let mut ix = VecIndex::default();
        // 40k rows so the scan is split across threads.
        for i in 0..40_000i64 {
            #[allow(clippy::cast_precision_loss)]
            let angle = i as f32 / 40_000.0 * 1.5;
            ix.push(i, &quantize(&[angle.cos(), angle.sin(), 0.0]));
        }
        let top = ix.top(&[1.0, 0.0, 0.0], 3);
        assert_eq!(top.len(), 3);
        assert!(top[0].0 < 50 && top.iter().all(|t| t.0 < 200), "{top:?}");
        assert!(top[0].1 >= top[1].1 && top[1].1 >= top[2].1);
        ix.remove(top[0].0);
        ix.remove(39_999);
        assert_eq!(ix.len(), 39_998);
        assert!(ix.top(&[1.0, 0.0, 0.0], 3).iter().all(|t| t.0 != top[0].0));
        let far = ix.top(&[0.0707, 0.9975, 0.0], 1);
        assert!((39_000..39_999).contains(&far[0].0), "{far:?}");
        ix.push(1, &quantize(&[1.0, 2.0]));
        assert_eq!(ix.len(), 39_998, "a vector of another size is skipped");
    }
}
