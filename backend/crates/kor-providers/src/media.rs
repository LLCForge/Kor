//! Venice image and video generation: `/image/generate`, `/video/{quote,queue,retrieve,complete}`
//! and `/models?type=image|video`. The key stays in the HTTP client; callers never see it.

use base64::Engine as _;
use kor_core::{ProviderError, ProviderErrorKind};
use kor_secrets::SecretString;
use reqwest::{Method, Response};
use serde_json::{Value, json};

use crate::ProviderManifest;
use crate::http::Http;

/// A generated image bigger than this is refused (a 4K PNG is well under it).
const MAX_IMAGE_RESPONSE: usize = 64 * 1024 * 1024;
/// A generated video bigger than this is refused rather than written half-way.
const MAX_VIDEO_BYTES: usize = 1024 * 1024 * 1024;

/// What a model generates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    fn query(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
        }
    }
}

/// A generation model and the options it accepts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaModel {
    pub id: String,
    /// Video: `text-to-video`, `image-to-video` or `video`. Empty for images.
    pub model_type: String,
    /// Price of one generation in USD (images: base or 1K resolution), when listed.
    pub price_usd: Option<f64>,
    pub durations: Vec<String>,
    pub resolutions: Vec<String>,
    pub aspect_ratios: Vec<String>,
    /// Image models sized by `width`/`height` (the others take `aspect_ratio`).
    pub sized_by_dimensions: bool,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// One entry of `/models?type=image|video`.
pub fn parse_model(m: &Value) -> Option<MediaModel> {
    let id = m.get("id")?.as_str()?.to_owned();
    let spec = m.get("model_spec");
    let c = spec.and_then(|s| s.get("constraints"));
    let get = |k: &str| c.and_then(|c| c.get(k));
    let pricing = spec.and_then(|s| s.get("pricing"));
    let price_usd = pricing
        .and_then(|p| {
            p.pointer("/generation/usd")
                .or_else(|| p.pointer("/resolutions/1K/usd"))
        })
        .and_then(Value::as_f64);
    Some(MediaModel {
        id,
        model_type: get("model_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        price_usd,
        durations: strings(get("durations")),
        resolutions: strings(get("resolutions")),
        aspect_ratios: {
            let a = strings(get("aspect_ratios"));
            if a.is_empty() {
                strings(get("aspectRatios"))
            } else {
                a
            }
        },
        sized_by_dimensions: get("widthHeightDivisor").is_some(),
    })
}

/// `POST /image/generate`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub aspect_ratio: Option<String>,
    /// `png`, `jpeg` or `webp`.
    pub format: String,
    /// 1–4 images.
    pub variants: u8,
}

impl ImageRequest {
    pub fn body(&self) -> Value {
        let mut b = json!({
            "model": self.model,
            "prompt": self.prompt,
            "format": self.format,
            "variants": self.variants.clamp(1, 4),
        });
        if let Some(n) = &self.negative_prompt {
            b["negative_prompt"] = json!(n);
        }
        if let Some(w) = self.width {
            b["width"] = json!(w);
        }
        if let Some(h) = self.height {
            b["height"] = json!(h);
        }
        if let Some(a) = &self.aspect_ratio {
            b["aspect_ratio"] = json!(a);
        }
        b
    }
}

/// `POST /video/queue` and `/video/quote`.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    /// One of the model's `durations`, e.g. `5s`.
    pub duration: String,
    pub aspect_ratio: Option<String>,
    pub resolution: Option<String>,
    /// Start frame for image-to-video: an `https://` URL or a `data:image/...` URL.
    pub image_url: Option<String>,
}

impl VideoRequest {
    fn quote_body(&self) -> Value {
        let mut b = json!({"model": self.model, "duration": self.duration});
        if let Some(a) = &self.aspect_ratio {
            b["aspect_ratio"] = json!(a);
        }
        if let Some(r) = &self.resolution {
            b["resolution"] = json!(r);
        }
        b
    }

    pub fn queue_body(&self) -> Value {
        let mut b = self.quote_body();
        b["prompt"] = json!(self.prompt);
        if let Some(i) = &self.image_url {
            b["image_url"] = json!(i);
        }
        b
    }
}

/// `POST /video/retrieve`.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoStatus {
    Processing {
        /// Typical total time for this model, when the server knows it.
        average_ms: Option<u64>,
        elapsed_ms: Option<u64>,
    },
    /// The finished MP4.
    Done(Vec<u8>),
    /// Finished; the file is at this URL (some models), valid for 24 hours.
    Url(String),
}

/// A Venice account's media endpoints.
#[derive(Debug, Clone)]
pub struct VeniceMedia {
    http: Http,
}

async fn read_limited(
    http: &Http,
    mut resp: Response,
    max: usize,
) -> Result<Vec<u8>, ProviderError> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| http.transport(&e))? {
        if out.len() + chunk.len() > max {
            return Err(http.err(
                ProviderErrorKind::Protocol,
                format!("response larger than {} MB", max / (1024 * 1024)),
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

impl VeniceMedia {
    /// `manifest` is the Venice provider's (base URL and auth header).
    pub fn new(manifest: &ProviderManifest, key: SecretString) -> Result<Self, ProviderError> {
        Ok(Self {
            http: Http::new(manifest, Some(key))?,
        })
    }

    /// Raw `GET /models?type=text` (pricing, context, capabilities for the desktop catalog).
    pub async fn text_models(&self) -> Result<Value, ProviderError> {
        self.http.get_json("models?type=text").await
    }

    /// `GET /api_keys/rate_limits`: balances, tier and per-model limits of this key.
    pub async fn rate_limits(&self) -> Result<Value, ProviderError> {
        self.http.get_json("api_keys/rate_limits").await
    }

    pub async fn models(&self, kind: MediaKind) -> Result<Vec<MediaModel>, ProviderError> {
        let v = self
            .http
            .get_json(&format!("models?type={}", kind.query()))
            .await?;
        Ok(v.get("data")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(parse_model).collect())
            .unwrap_or_default())
    }

    /// Generated images, decoded.
    pub async fn image(&self, req: &ImageRequest) -> Result<Vec<Vec<u8>>, ProviderError> {
        let resp = self
            .http
            .post_plain_json("image/generate", &req.body())
            .await?;
        let body = read_limited(&self.http, resp, MAX_IMAGE_RESPONSE).await?;
        let v: Value = serde_json::from_slice(&body)
            .map_err(|e| self.http.err(ProviderErrorKind::Protocol, e.to_string()))?;
        let images: Vec<Vec<u8>> = v
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
            .collect();
        if images.is_empty() {
            return Err(self
                .http
                .err(ProviderErrorKind::Protocol, "no image in the response"));
        }
        Ok(images)
    }

    /// Price in USD before anything is generated.
    pub async fn video_quote(&self, req: &VideoRequest) -> Result<f64, ProviderError> {
        let resp = self
            .http
            .post_plain_json("video/quote", &req.quote_body())
            .await?;
        let v = self.http.read_json(resp).await?;
        v.get("quote").and_then(Value::as_f64).ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "no quote in the response")
        })
    }

    /// Starts generation; returns the queue id and, for some models, the final download URL.
    pub async fn video_queue(
        &self,
        req: &VideoRequest,
    ) -> Result<(String, Option<String>), ProviderError> {
        let resp = self
            .http
            .post_plain_json("video/queue", &req.queue_body())
            .await?;
        let v = self.http.read_json(resp).await?;
        let id = v.get("queue_id").and_then(Value::as_str).ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "no queue_id in the response")
        })?;
        Ok((
            id.to_owned(),
            v.get("download_url")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ))
    }

    pub async fn video_retrieve(&self, queue_id: &str) -> Result<VideoStatus, ProviderError> {
        let resp = self
            .http
            .post_plain_json("video/retrieve", &json!({"queue_id": queue_id}))
            .await?;
        let is_json = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("json"));
        if !is_json {
            return Ok(VideoStatus::Done(
                read_limited(&self.http, resp, MAX_VIDEO_BYTES).await?,
            ));
        }
        let v = self.http.read_json(resp).await?;
        Ok(parse_status(&v))
    }

    /// Deletes the finished media from Venice's servers.
    pub async fn video_complete(&self, queue_id: &str) -> Result<(), ProviderError> {
        self.http
            .post_plain_json("video/complete", &json!({"queue_id": queue_id}))
            .await
            .map(drop)
    }

    /// Downloads a finished video from its `download_url`. The URL is fetched without the API key:
    /// it points at a storage host, and the key must not travel anywhere but the API.
    pub async fn download(&self, url: &str) -> Result<Vec<u8>, ProviderError> {
        if !url.starts_with("https://") {
            return Err(self.http.err(
                ProviderErrorKind::InvalidRequest,
                "download URL is not https",
            ));
        }
        let client = kor_core::net::client_builder()
            .user_agent(format!("kor/{}", kor_core::VERSION))
            .build()
            .map_err(|e| self.http.transport(&e))?;
        let resp = client
            .request(Method::GET, url)
            .send()
            .await
            .map_err(|e| self.http.transport(&e))?;
        if !resp.status().is_success() {
            return Err(self.http.err(
                ProviderErrorKind::Unavailable,
                format!("download failed: HTTP {}", resp.status().as_u16()),
            ));
        }
        read_limited(&self.http, resp, MAX_VIDEO_BYTES).await
    }

    /// Embedding vectors for `texts`, in input order (`POST /embeddings`, OpenAI-compatible).
    pub async fn embed(
        &self,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ProviderError> {
        let body = json!({"model": model, "input": texts, "encoding_format": "float"});
        let resp = self.http.post_plain_json("embeddings", &body).await?;
        let raw = read_limited(&self.http, resp, MAX_IMAGE_RESPONSE).await?;
        let v: Value = serde_json::from_slice(&raw)
            .map_err(|e| self.http.err(ProviderErrorKind::Protocol, e.to_string()))?;
        parse_embeddings(&v, texts.len()).ok_or_else(|| {
            self.http
                .err(ProviderErrorKind::Protocol, "malformed embeddings response")
        })
    }
}

/// `data[].embedding`, placed by `index`; `None` unless every input got a vector.
pub fn parse_embeddings(v: &Value, count: usize) -> Option<Vec<Vec<f32>>> {
    let mut out: Vec<Option<Vec<f32>>> = vec![None; count];
    for (pos, item) in v.get("data")?.as_array()?.iter().enumerate() {
        let i = item
            .get("index")
            .and_then(Value::as_u64)
            .map_or(pos, |i| usize::try_from(i).unwrap_or(usize::MAX));
        #[allow(clippy::cast_possible_truncation)]
        let vec: Vec<f32> = item
            .get("embedding")?
            .as_array()?
            .iter()
            .map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Option<_>>()?;
        *out.get_mut(i)? = Some(vec);
    }
    out.into_iter().collect()
}

/// JSON answer of `/video/retrieve`.
pub fn parse_status(v: &Value) -> VideoStatus {
    if v.get("status").and_then(Value::as_str) == Some("COMPLETED")
        && let Some(url) = v.get("download_url").and_then(Value::as_str)
    {
        return VideoStatus::Url(url.to_owned());
    }
    VideoStatus::Processing {
        average_ms: v.get("average_execution_time").and_then(Value::as_u64),
        elapsed_ms: v.get("execution_duration").and_then(Value::as_u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_image_and_video_models() {
        let img = parse_model(&json!({"id": "venice-sd35", "model_spec": {
            "constraints": {"widthHeightDivisor": 16}, "pricing": {"generation": {"usd": 0.01}}}}))
        .unwrap();
        assert!(img.sized_by_dimensions);
        assert_eq!(img.price_usd, Some(0.01));
        let img = parse_model(&json!({"id": "krea-3", "model_spec": {
            "constraints": {"aspectRatios": ["1:1", "16:9"]}, "pricing": {"resolutions": {"1K": {"usd": 0.07}}}}}))
        .unwrap();
        assert!(!img.sized_by_dimensions);
        assert_eq!(img.aspect_ratios, ["1:1", "16:9"]);
        assert_eq!(img.price_usd, Some(0.07));
        let vid = parse_model(&json!({"id": "wan", "model_spec": {"constraints": {
            "model_type": "text-to-video", "durations": ["5s", "10s"], "resolutions": ["720p"], "aspect_ratios": ["16:9"]}}}))
        .unwrap();
        assert_eq!(vid.model_type, "text-to-video");
        assert_eq!(vid.durations, ["5s", "10s"]);
        assert_eq!(vid.price_usd, None);
    }

    #[test]
    fn request_bodies_carry_only_set_fields() {
        let r = ImageRequest {
            model: "m".into(),
            prompt: "p".into(),
            negative_prompt: None,
            width: Some(512),
            height: None,
            aspect_ratio: None,
            format: "png".into(),
            variants: 9,
        };
        let b = r.body();
        assert_eq!(b["width"], 512);
        assert!(b.get("height").is_none() && b.get("negative_prompt").is_none());
        assert_eq!(b["variants"], 4, "clamped to the API's limit");

        let v = VideoRequest {
            model: "m".into(),
            prompt: "p".into(),
            duration: "5s".into(),
            aspect_ratio: Some("16:9".into()),
            resolution: None,
            image_url: Some("data:image/png;base64,AA".into()),
        };
        let q = v.queue_body();
        assert_eq!(q["duration"], "5s");
        assert_eq!(q["image_url"], "data:image/png;base64,AA");
        assert!(v.quote_body().get("prompt").is_none());
    }

    #[test]
    fn retrieve_status() {
        assert!(matches!(
            parse_status(
                &json!({"status": "PROCESSING", "average_execution_time": 90000, "execution_duration": 1000})
            ),
            VideoStatus::Processing {
                average_ms: Some(90000),
                elapsed_ms: Some(1000)
            }
        ));
        assert_eq!(
            parse_status(&json!({"status": "COMPLETED", "download_url": "https://x/v.mp4"})),
            VideoStatus::Url("https://x/v.mp4".into())
        );
    }

    #[test]
    fn embeddings_follow_the_index_and_need_every_input() {
        let v = json!({"data": [
            {"index": 1, "embedding": [0.5, 1.0]},
            {"index": 0, "embedding": [0.25, 0.0]}
        ]});
        assert_eq!(
            parse_embeddings(&v, 2).unwrap(),
            vec![vec![0.25, 0.0], vec![0.5, 1.0]]
        );
        assert!(parse_embeddings(&v, 3).is_none());
        assert!(
            parse_embeddings(&json!({"data": [{"index": 5, "embedding": [1.0]}]}), 1).is_none()
        );
    }
}
