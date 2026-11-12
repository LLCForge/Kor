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

/// Rows from which [`VecIndex::top`] first narrows by sign bits: below it the exact scan is fast
/// enough everywhere (§31.3.1), above it a weak CPU would miss the 30 ms budget.
pub const PREFILTER_FROM: usize = 100_000;

/// Sign bits of a vector, 64 dimensions per word: the binary sketch whose Hamming distance
/// tracks the angle between two vectors.
fn signs(v: &[i8], out: &mut Vec<u64>) {
    for chunk in v.chunks(64) {
        let mut w = 0u64;
        for (j, x) in chunk.iter().enumerate() {
            if *x > 0 {
                w |= 1 << j;
            }
        }
        out.push(w);
    }
}

/// Top-k cosine search over int8 vectors held in one contiguous buffer: an exact scan, or for
/// large indexes a Hamming pass over the sign bits that keeps the best candidates for the exact
/// int8 rescoring (binary pre-filter).
#[derive(Debug, Default)]
pub struct VecIndex {
    dims: usize,
    ids: Vec<i64>,
    inv: Vec<f32>,
    data: Vec<i8>,
    /// `dims.div_ceil(64)` sign words per row.
    bits: Vec<u64>,
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
        let from = self.data.len();
        self.data
            .extend(q.iter().map(|b| i8::from_le_bytes([*b]).max(-127)));
        signs(&self.data[from..], &mut self.bits);
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
            let w = self.dims.div_ceil(64);
            self.bits.copy_within(last * w..(last + 1) * w, i * w);
        }
        self.data.truncate(last * self.dims);
        self.bits.truncate(last * self.dims.div_ceil(64));
    }

    /// The `k` most similar ids to `query` (an unquantized, normalized vector), with their
    /// approximate cosine similarity, best first.
    /// From [`PREFILTER_FROM`] rows the result is approximate (the best `64·k`, at least 4096,
    /// candidates by sign bits are rescored exactly).
    pub fn top(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if self.len() < PREFILTER_FROM {
            self.top_exact(query, k)
        } else {
            self.top_prefiltered(query, k, (k * 64).max(4096))
        }
    }

    /// `candidates` rows nearest by sign-bit Hamming distance, rescored with the int8 dot. The
    /// distances go to a `u16` array with a histogram per thread; the histogram gives the cutoff
    /// distance, so no sort or selection over all rows is needed.
    #[allow(clippy::many_single_char_names)] // same short names as `top_exact`
    fn top_prefiltered(&self, query: &[f32], k: usize, candidates: usize) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dims || k == 0 {
            return Vec::new();
        }
        let (qinv, q) = to_i8(query);
        let mut qb = Vec::new();
        signs(&q, &mut qb);
        let w = qb.len();
        let n = self.ids.len();
        let max = w * 64;
        let mut dist = vec![0u16; n];
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(n.div_ceil(65_536))
            .max(1);
        let per = n.div_ceil(threads);
        let hist: Vec<Vec<u32>> = std::thread::scope(|s| {
            let hs: Vec<_> = dist
                .chunks_mut(per)
                .enumerate()
                .map(|(t, part)| {
                    let (bits, qb) = (&self.bits, &qb);
                    s.spawn(move || {
                        let mut h = vec![0u32; max + 1];
                        for (j, slot) in part.iter_mut().enumerate() {
                            let i = t * per + j;
                            let d: u32 = bits[i * w..(i + 1) * w]
                                .iter()
                                .zip(qb)
                                .map(|(a, b)| (a ^ b).count_ones())
                                .sum();
                            // At most 64·w, far below u16::MAX for any embedding size.
                            let d = u16::try_from(d).unwrap_or(u16::MAX);
                            *slot = d;
                            h[usize::from(d).min(max)] += 1;
                        }
                        h
                    })
                })
                .collect();
            hs.into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        });
        let want = candidates.clamp(k, n);
        let (mut cut, mut below) = (0usize, 0usize);
        for d in 0..=max {
            let here: usize = hist
                .iter()
                .map(|h| h.get(d).copied().unwrap_or(0) as usize)
                .sum();
            if below + here >= want {
                cut = d;
                break;
            }
            below += here;
            cut = d;
        }
        // Everything under the cutoff, then rows at the cutoff until `want` are in.
        let mut at_cut = want.saturating_sub(below);
        let mut best = Top::new(k);
        for (i, d) in dist.iter().enumerate() {
            let d = usize::from(*d);
            if d > cut || (d == cut && at_cut == 0) {
                continue;
            }
            if d == cut {
                at_cut -= 1;
            }
            let v = &self.data[i * self.dims..(i + 1) * self.dims];
            #[allow(clippy::cast_precision_loss)]
            best.offer(dot(&q, v) as f32 * self.inv[i], i);
        }
        best.0
            .into_iter()
            .map(|(s, i)| (self.ids[i], s * qinv))
            .collect()
    }

    /// Exact scan over every row, split across threads.
    #[allow(clippy::many_single_char_names)]
    fn top_exact(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
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

    /// Random unit vectors are the pre-filter's worst case (no clusters); real embeddings recall
    /// better. Removal keeps the sign bits aligned with the rows.
    #[test]
    fn prefilter_recalls_the_exact_top_k() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            #[allow(clippy::cast_precision_loss)]
            let x = (seed >> 40) as f32 / (1u64 << 23) as f32;
            x - 1.0
        };
        let dims = 384;
        let mut ix = VecIndex::default();
        let mut keep = Vec::new();
        for id in 0..30_000i64 {
            let v = unit(&(0..dims).map(|_| next()).collect::<Vec<_>>());
            if id % 1000 == 0 {
                keep.push(v.clone());
            }
            ix.push(id, &quantize(&v));
        }
        ix.remove(5);
        let (mut hit, mut total) = (0, 0);
        for base in &keep {
            // A query near a stored vector, as a real recall is.
            let q = unit(&base.iter().map(|x| x + 0.03 * next()).collect::<Vec<_>>());
            let exact = ix.top_exact(&q, 10);
            let fast = ix.top_prefiltered(&q, 10, 1500);
            assert_eq!(fast[0].0, exact[0].0, "the near vector comes first");
            total += exact.len();
            hit += exact
                .iter()
                .filter(|e| fast.iter().any(|f| f.0 == e.0))
                .count();
        }
        assert!(hit * 100 >= total * 85, "recall {hit}/{total}");
        assert!(
            ix.top_prefiltered(&keep[0], 10, 1500)
                .iter()
                .all(|t| t.0 != 5)
        );
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
