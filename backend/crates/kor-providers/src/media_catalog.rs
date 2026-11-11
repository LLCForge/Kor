//! Media capability catalog (Master Build Prompt §20.1, §20.2, ADR 0060): one schema for the
//! generation models of every configured provider (fal.ai, Venice, Hugging Face), each with its
//! capabilities, typed parameters (options, ranges, defaults), the provider's list price and
//! where the data came from; plus the planning used before a generation: which parameters to ask
//! about, which given values the model does not support (with the nearest option and other
//! models that do), and the cost estimate ("≈" when units had to be guessed).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::fal::{self, Fal, FalModel};
use crate::huggingface::{Hf, HubModel, SearchQuery};
use crate::media::{MediaKind, MediaModel, VeniceMedia};

/// What a model does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Text to image.
    Image,
    /// Image to image (restyle, change).
    Edit,
    Upscale,
    RemoveBg,
    /// Text to video.
    Video,
    ImageToVideo,
    Tts,
    Asr,
    /// Sound effects and other generated audio.
    Audio,
    Music,
}

impl Capability {
    pub const ALL: [Self; 10] = [
        Self::Image,
        Self::Edit,
        Self::Upscale,
        Self::RemoveBg,
        Self::Video,
        Self::ImageToVideo,
        Self::Tts,
        Self::Asr,
        Self::Audio,
        Self::Music,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Edit => "edit",
            Self::Upscale => "upscale",
            Self::RemoveBg => "remove-bg",
            Self::Video => "video",
            Self::ImageToVideo => "image-to-video",
            Self::Tts => "tts",
            Self::Asr => "asr",
            Self::Audio => "audio",
            Self::Music => "music",
        }
    }

    /// The key, or a common alias (`text-to-image`, `speech-to-text`, ...).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|c| c.key() == s)
            .or(match s.as_str() {
                "text-to-image" | "image-generate" | "img" => Some(Self::Image),
                "image-to-image" | "image-edit" => Some(Self::Edit),
                "background-removal" | "removebg" | "rembg" => Some(Self::RemoveBg),
                "text-to-video" | "video-generate" => Some(Self::Video),
                "i2v" => Some(Self::ImageToVideo),
                "text-to-speech" | "speech" => Some(Self::Tts),
                "speech-to-text" | "transcribe" | "automatic-speech-recognition" => Some(Self::Asr),
                "text-to-audio" | "sfx" => Some(Self::Audio),
                _ => None,
            })
    }

    /// Output kind: `image`, `video`, `audio` or `text`.
    pub fn output(self) -> &'static str {
        match self {
            Self::Image | Self::Edit | Self::Upscale | Self::RemoveBg => "image",
            Self::Video | Self::ImageToVideo => "video",
            Self::Tts | Self::Audio | Self::Music => "audio",
            Self::Asr => "text",
        }
    }

    /// Needs an input file (image or audio) rather than (only) a prompt.
    pub fn needs_input(self) -> bool {
        matches!(
            self,
            Self::Edit | Self::Upscale | Self::RemoveBg | Self::ImageToVideo | Self::Asr
        )
    }

    fn fal_category(self) -> &'static str {
        match self {
            Self::Image => "text-to-image",
            Self::Edit | Self::Upscale | Self::RemoveBg => "image-to-image",
            Self::Video => "text-to-video",
            Self::ImageToVideo => "image-to-video",
            Self::Tts => "text-to-speech",
            Self::Asr => "speech-to-text",
            Self::Audio | Self::Music => "text-to-audio",
        }
    }

    /// Hub pipeline for the tasks Kor drives on Hugging Face (`kor_agent::media` routes).
    fn hf_pipeline(self) -> Option<&'static str> {
        Some(match self {
            Self::Image => "text-to-image",
            Self::Edit => "image-to-image",
            Self::Video => "text-to-video",
            Self::Tts => "text-to-speech",
            Self::Asr => "automatic-speech-recognition",
            _ => return None,
        })
    }
}

/// Parameter value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    String,
    Integer,
    Number,
    Boolean,
    /// One of `options`.
    Enum,
    /// A file: an `https`/`data:` URL, or a project path / gallery id Kor turns into one.
    File,
    Array,
    Object,
}

/// What a parameter means, whatever the provider calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Prompt,
    NegativePrompt,
    Size,
    AspectRatio,
    Resolution,
    Duration,
    Fps,
    Voice,
    Style,
    Seed,
    Audio,
    Count,
    Reference,
    Mask,
    Format,
    Other,
}

impl Role {
    fn of(name: &str) -> Self {
        match name {
            "prompt" | "text" | "input" | "lyrics" => Self::Prompt,
            "negative_prompt" => Self::NegativePrompt,
            "image_size" | "width" | "height" | "size" => Self::Size,
            "aspect_ratio" => Self::AspectRatio,
            "resolution" => Self::Resolution,
            "duration" | "seconds" | "seconds_total" | "duration_seconds" => Self::Duration,
            "fps" | "frames_per_second" => Self::Fps,
            "voice" | "voice_id" | "speaker" => Self::Voice,
            "style" | "style_preset" => Self::Style,
            "seed" => Self::Seed,
            "generate_audio" | "enable_audio" | "with_audio" => Self::Audio,
            "num_images" | "variants" | "num_outputs" => Self::Count,
            "mask_url" | "mask_image_url" => Self::Mask,
            "format" | "output_format" => Self::Format,
            n if n.ends_with("image_url")
                || n.ends_with("image_urls")
                || n == "image"
                || n == "audio_url"
                || n == "video_url"
                || n == "audio" =>
            {
                Self::Reference
            }
            _ => Self::Other,
        }
    }

    /// Asked about before generating when the model offers a choice (§20.2).
    pub fn is_choice(self) -> bool {
        matches!(
            self,
            Self::Size
                | Self::AspectRatio
                | Self::Resolution
                | Self::Duration
                | Self::Voice
                | Self::Style
                | Self::Audio
        )
    }
}

/// One input of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Param {
    pub name: String,
    pub kind: ParamKind,
    pub role: Role,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A list price as the provider publishes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub amount: f64,
    pub currency: String,
    /// `image`, `second`, `megapixel`, `request`, ...
    pub unit: String,
}

/// One generation model, whatever the provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    /// `fal`, `venice` or `huggingface`.
    pub provider: String,
    pub id: String,
    pub name: String,
    pub capabilities: Vec<Capability>,
    pub params: Vec<Param>,
    pub price: Option<Price>,
    /// Where the entry came from (API endpoint or document).
    pub source: String,
    /// Takes a reference image/audio (image-to-image, image-to-video, transcription).
    pub accepts_reference: bool,
}

impl CatalogModel {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    pub fn by_role(&self, role: Role) -> Option<&Param> {
        self.params.iter().find(|p| p.role == role)
    }

    /// `provider/id`, the reference used by `--model` and `media.defaults`.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.provider, self.id)
    }

    fn finish(mut self) -> Self {
        self.accepts_reference = self.params.iter().any(|p| p.role == Role::Reference);
        self
    }
}

// ---------------------------------------------------------------------------------------------
// Converters
// ---------------------------------------------------------------------------------------------

/// Typed parameters of a JSON schema (fal's `OpenAPI` input), required first.
pub fn params_from_schema(schema: &Value) -> Vec<Param> {
    let required: BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut out: Vec<Param> = schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, p)| {
            // `anyOf`/`oneOf` branches: the first typed one decides; enums come from any branch.
            let branches: Vec<&Value> = std::iter::once(p)
                .chain(
                    ["anyOf", "oneOf"]
                        .iter()
                        .filter_map(|k| p.get(*k).and_then(Value::as_array))
                        .flatten(),
                )
                .collect();
            let options: Vec<Value> = branches
                .iter()
                .filter_map(|b| b.get("enum").and_then(Value::as_array))
                .flatten()
                .cloned()
                .collect();
            let ty = branches
                .iter()
                .filter_map(|b| b.get("type").and_then(Value::as_str))
                .find(|t| *t != "null");
            let num = |k: &str| {
                branches
                    .iter()
                    .find_map(|b| b.get(k).and_then(Value::as_f64))
            };
            let role = Role::of(name);
            let kind = if role == Role::Reference || role == Role::Mask || name.ends_with("_url") {
                ParamKind::File
            } else if !options.is_empty() {
                ParamKind::Enum
            } else {
                match ty {
                    Some("integer") => ParamKind::Integer,
                    Some("number") => ParamKind::Number,
                    Some("boolean") => ParamKind::Boolean,
                    Some("array") => ParamKind::Array,
                    Some("object") => ParamKind::Object,
                    _ => ParamKind::String,
                }
            };
            Param {
                name: name.clone(),
                kind,
                role,
                required: required.contains(name.as_str()),
                options,
                min: num("minimum"),
                max: num("maximum"),
                default: p.get("default").cloned(),
                description: p
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect(),
            }
        })
        .collect();
    out.sort_by_key(|p| !p.required);
    out
}

fn has(hay: &str, words: &[&str]) -> bool {
    words.iter().any(|w| hay.contains(w))
}

/// Capabilities of a fal model from its listing category (and, for `image-to-image` and
/// `text-to-audio`, its id and name).
pub fn fal_capabilities(category: &str, id: &str, name: &str) -> Vec<Capability> {
    let hay = format!("{id} {name}").to_ascii_lowercase();
    match category {
        "text-to-image" => vec![Capability::Image],
        "image-to-image" if has(&hay, &["birefnet", "background", "rembg", "remove-bg"]) => {
            vec![Capability::RemoveBg]
        }
        "image-to-image" if has(&hay, &["upscal", "esrgan", "aura-sr", "clarity", "seedvr"]) => {
            vec![Capability::Upscale]
        }
        "image-to-image" => vec![Capability::Edit],
        "text-to-video" => vec![Capability::Video],
        "image-to-video" => vec![Capability::ImageToVideo],
        "text-to-speech" => vec![Capability::Tts],
        "speech-to-text" => vec![Capability::Asr],
        "text-to-audio" if has(&hay, &["music", "song", "lyria", "minimax-music"]) => {
            vec![Capability::Music]
        }
        "text-to-audio" => vec![Capability::Audio],
        _ => Vec::new(),
    }
}

/// A fal model with its schema (from `expand=openapi-3.0`) and price (`/models/pricing`).
pub fn from_fal(
    m: &FalModel,
    schema: Option<&Value>,
    price: Option<&fal::Price>,
) -> Option<CatalogModel> {
    let capabilities = fal_capabilities(&m.category, &m.endpoint_id, &m.name);
    if capabilities.is_empty() {
        return None;
    }
    Some(
        CatalogModel {
            provider: "fal".into(),
            id: m.endpoint_id.clone(),
            name: if m.name.is_empty() {
                m.endpoint_id.clone()
            } else {
                m.name.clone()
            },
            capabilities,
            params: schema.map(params_from_schema).unwrap_or_default(),
            price: price.map(|p| Price {
                amount: p.unit_price,
                currency: p.currency.clone(),
                unit: p.unit.clone(),
            }),
            source: format!("{}/models?category={}", fal::PLATFORM_BASE, m.category),
            accepts_reference: false,
        }
        .finish(),
    )
}

/// A fal model known only by its id (a tool call names it), with the capability it is used for.
pub fn fal_model(
    id: &str,
    cap: Capability,
    schema: Option<&Value>,
    price: Option<&fal::Price>,
) -> CatalogModel {
    let m = FalModel {
        endpoint_id: id.to_owned(),
        name: id.to_owned(),
        category: cap.fal_category().to_owned(),
        description: String::new(),
    };
    let mut c = from_fal(&m, schema, price).unwrap_or_else(|| CatalogModel {
        provider: "fal".into(),
        id: id.to_owned(),
        name: id.to_owned(),
        capabilities: Vec::new(),
        params: schema.map(params_from_schema).unwrap_or_default(),
        price: None,
        source: format!("{}/models?endpoint_id={id}", fal::PLATFORM_BASE),
        accepts_reference: false,
    });
    c.capabilities = vec![cap];
    c.source = format!("{}/models?endpoint_id={id}", fal::PLATFORM_BASE);
    c.finish()
}

fn param(name: &str, kind: ParamKind, required: bool) -> Param {
    Param {
        name: name.into(),
        kind,
        role: Role::of(name),
        required,
        options: Vec::new(),
        min: None,
        max: None,
        default: None,
        description: String::new(),
    }
}

fn enum_param(name: &str, options: &[String], default: Option<&str>) -> Param {
    Param {
        options: options.iter().map(|o| json!(o)).collect(),
        default: default
            .filter(|d| options.iter().any(|o| o == d))
            .or_else(|| options.first().map(String::as_str))
            .map(|d| json!(d)),
        ..param(name, ParamKind::Enum, false)
    }
}

fn range(name: &str, kind: ParamKind, min: f64, max: f64, default: Option<Value>) -> Param {
    Param {
        min: Some(min),
        max: Some(max),
        default,
        ..param(name, kind, false)
    }
}

/// A Venice model from `/models?type=image|video` (the inputs `kor_agent::media` sends).
pub fn from_venice(m: &MediaModel, kind: MediaKind) -> CatalogModel {
    let mut params = vec![param("prompt", ParamKind::String, true)];
    let capabilities = match kind {
        MediaKind::Image => {
            params.push(param("negative_prompt", ParamKind::String, false));
            if m.sized_by_dimensions {
                // The bounds `generate_image` enforces (Venice: 256-1280).
                params.push(range(
                    "width",
                    ParamKind::Integer,
                    256.0,
                    1280.0,
                    Some(json!(1024)),
                ));
                params.push(range(
                    "height",
                    ParamKind::Integer,
                    256.0,
                    1280.0,
                    Some(json!(1024)),
                ));
            } else if !m.aspect_ratios.is_empty() {
                params.push(enum_param("aspect_ratio", &m.aspect_ratios, Some("1:1")));
            }
            params.push(enum_param(
                "format",
                &["png".into(), "jpeg".into(), "webp".into()],
                Some("png"),
            ));
            params.push(range(
                "variants",
                ParamKind::Integer,
                1.0,
                4.0,
                Some(json!(1)),
            ));
            vec![Capability::Image]
        }
        MediaKind::Video => {
            if !m.durations.is_empty() {
                params.push(enum_param("duration", &m.durations, None));
            }
            if !m.resolutions.is_empty() {
                // Listed high to low; the cheapest (last) is what `generate_video` picks.
                params.push(enum_param("resolution", &m.resolutions, Some("720p")));
            }
            if !m.aspect_ratios.is_empty() {
                params.push(enum_param("aspect_ratio", &m.aspect_ratios, Some("16:9")));
            }
            if m.model_type == "image-to-video" {
                let mut p = param("image", ParamKind::File, true);
                p.role = Role::Reference;
                params.push(p);
                vec![Capability::ImageToVideo]
            } else {
                vec![Capability::Video]
            }
        }
    };
    CatalogModel {
        provider: "venice".into(),
        id: m.id.clone(),
        name: m.id.clone(),
        capabilities,
        params,
        price: m.price_usd.map(|p| Price {
            amount: p,
            currency: "USD".into(),
            unit: if kind == MediaKind::Image {
                "image".into()
            } else {
                "video".into()
            },
        }),
        source: format!(
            "api.venice.ai/api/v1/models?type={}",
            match kind {
                MediaKind::Image => "image",
                MediaKind::Video => "video",
            }
        ),
        accepts_reference: false,
    }
    .finish()
}

/// A Hub model for a pipeline Kor runs; parameters are the task's standard inputs (Hugging Face
/// Inference task specification) since the Hub publishes no per-model schema or price.
pub fn from_hf(m: &HubModel, cap: Capability) -> CatalogModel {
    let mut params = Vec::new();
    match cap {
        Capability::Asr => {
            let mut p = param("audio", ParamKind::File, true);
            p.role = Role::Reference;
            params.push(p);
        }
        Capability::Tts => params.push(param("text", ParamKind::String, true)),
        _ => {
            params.push(param("prompt", ParamKind::String, true));
            if cap == Capability::Edit {
                let mut p = param("image", ParamKind::File, true);
                p.role = Role::Reference;
                params.push(p);
            }
            params.push(param("negative_prompt", ParamKind::String, false));
            params.push(param("num_inference_steps", ParamKind::Integer, false));
            params.push(param("guidance_scale", ParamKind::Number, false));
            params.push(param("seed", ParamKind::Integer, false));
            if cap == Capability::Image {
                params.push(param("width", ParamKind::Integer, false));
                params.push(param("height", ParamKind::Integer, false));
            }
            if cap == Capability::Video {
                params.push(param("num_frames", ParamKind::Integer, false));
            }
        }
    }
    CatalogModel {
        provider: "huggingface".into(),
        id: m.id.clone(),
        name: m.id.clone(),
        capabilities: vec![cap],
        params,
        price: None,
        source: format!(
            "huggingface.co/api/models?pipeline_tag={}",
            cap.hf_pipeline().unwrap_or_default()
        ),
        accepts_reference: false,
    }
    .finish()
}

// ---------------------------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------------------------

/// The configured providers; a missing one contributes nothing (no key: no option, §20.1).
#[derive(Debug, Default, Clone, Copy)]
pub struct Sources<'a> {
    pub fal: Option<&'a Fal>,
    pub venice: Option<&'a VeniceMedia>,
    pub hf: Option<&'a Hf>,
}

/// Models per fal category and Hub pipeline.
const FAL_LIMIT: u32 = 40;
const HF_LIMIT: u32 = 10;

/// Catalog entries for `caps` from every source, plus one line per source that failed.
pub async fn fetch(src: &Sources<'_>, caps: &[Capability]) -> (Vec<CatalogModel>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let wanted = |m: &CatalogModel| m.capabilities.iter().any(|c| caps.contains(c));
    if let Some(f) = src.fal {
        let categories: BTreeSet<&str> = caps.iter().map(|c| c.fal_category()).collect();
        let mut found: Vec<(FalModel, Option<Value>)> = Vec::new();
        for cat in categories {
            match f.catalog(cat, FAL_LIMIT).await {
                Ok(list) => found.extend(list),
                Err(e) => errors.push(format!("fal {cat}: {}", e.message)),
            }
        }
        let ids: Vec<String> = found.iter().map(|(m, _)| m.endpoint_id.clone()).collect();
        let prices = match f.prices(&ids).await {
            Ok(p) => p,
            Err(e) => {
                errors.push(format!("fal pricing: {}", e.message));
                std::collections::HashMap::new()
            }
        };
        out.extend(
            found
                .iter()
                .filter_map(|(m, s)| from_fal(m, s.as_ref(), prices.get(&m.endpoint_id)))
                .filter(wanted),
        );
    }
    if let Some(v) = src.venice {
        for kind in [MediaKind::Image, MediaKind::Video] {
            let need = match kind {
                MediaKind::Image => caps.contains(&Capability::Image),
                MediaKind::Video => {
                    caps.contains(&Capability::Video) || caps.contains(&Capability::ImageToVideo)
                }
            };
            if !need {
                continue;
            }
            match v.models(kind).await {
                Ok(list) => out.extend(
                    list.iter()
                        .filter(|m| m.model_type != "video")
                        .map(|m| from_venice(m, kind))
                        .filter(wanted),
                ),
                Err(e) => errors.push(format!("venice: {}", e.message)),
            }
        }
    }
    if let Some(h) = src.hf {
        for cap in caps {
            let Some(pipeline) = cap.hf_pipeline() else {
                continue;
            };
            let q = SearchQuery {
                pipeline_tag: Some(pipeline.into()),
                limit: HF_LIMIT,
                ..SearchQuery::default()
            };
            match h.search(&q).await {
                Ok(list) => out.extend(list.iter().map(|m| from_hf(m, *cap))),
                Err(e) => errors.push(format!("huggingface {pipeline}: {}", e.message)),
            }
        }
    }
    // One entry per model; a model listed under several categories keeps all its capabilities.
    let mut models: Vec<CatalogModel> = Vec::new();
    for m in out {
        match models.iter_mut().find(|x| x.reference() == m.reference()) {
            Some(x) => {
                for c in m.capabilities {
                    if !x.capabilities.contains(&c) {
                        x.capabilities.push(c);
                    }
                }
            }
            None => models.push(m),
        }
    }
    (models, errors)
}

// ---------------------------------------------------------------------------------------------
// Planning a generation (§20.2)
// ---------------------------------------------------------------------------------------------

/// A parameter to ask about, with the options this model allows and the default marked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub name: String,
    pub role: Role,
    pub kind: ParamKind,
    pub description: String,
    pub options: Vec<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub default: Option<Value>,
}

/// A value the user asked for that this model cannot do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsupported {
    pub name: String,
    pub asked: Value,
    /// The closest value this model accepts.
    pub nearest: Option<Value>,
    /// What it accepts, for the message.
    pub allowed: String,
}

/// Cost before generating, in the price's currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub amount: f64,
    pub currency: String,
    /// The units were guessed (no duration given, megapixels rounded, ...): show "≈".
    pub estimated: bool,
}

impl Estimate {
    /// `$0.04`, `≈$0.40`, `≈0.10 EUR`.
    pub fn display(&self) -> String {
        let approx = if self.estimated { "≈" } else { "" };
        if self.currency == "USD" {
            format!("{approx}${:.2}", self.amount)
        } else {
            format!("{approx}{:.2} {}", self.amount, self.currency)
        }
    }
}

/// What to do before generating.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Inputs so far: saved defaults, then what was given (values coerced to their types).
    pub input: Map<String, Value>,
    pub questions: Vec<Question>,
    pub unsupported: Vec<Unsupported>,
    pub estimate: Option<Estimate>,
}

fn text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}

/// A value in the parameter's type: `"5"` stays a string for a string enum and becomes `5` for an
/// integer; an enum value is matched to the option with the same text.
pub fn coerce(p: &Param, v: &Value) -> Value {
    let t = text(v);
    if let Some(o) = p
        .options
        .iter()
        .find(|o| text(o).eq_ignore_ascii_case(t.trim()))
    {
        return o.clone();
    }
    match p.kind {
        ParamKind::Integer => t
            .trim()
            .parse::<i64>()
            .map_or_else(|_| v.clone(), |n| json!(n)),
        ParamKind::Number => t
            .trim()
            .parse::<f64>()
            .map_or_else(|_| v.clone(), |n| json!(n)),
        ParamKind::Boolean => match t.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" | "evet" | "var" | "oui" => json!(true),
            "false" | "no" | "off" | "0" | "hayır" | "yok" | "non" => json!(false),
            _ => v.clone(),
        },
        _ => v.clone(),
    }
}

/// A comparable size of a value: `4k` 2160, `1080p` 1080, `10s` 10, `16:9` 1.78, `5` 5.
fn magnitude(s: &str) -> Option<f64> {
    let s = s.trim().to_ascii_lowercase();
    if let Some((a, b)) = s.split_once(':') {
        let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        return (b > 0.0).then(|| a / b);
    }
    let digits: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let n: f64 = digits.parse().ok()?;
    let rest = s[digits.len()..].trim();
    Some(match rest {
        "k" if (n - 4.0).abs() < f64::EPSILON => 2160.0,
        "k" if (n - 2.0).abs() < f64::EPSILON => 1440.0,
        "k" => n * 540.0,
        _ => n,
    })
}

/// Does `p` accept `v`? `Err` carries the nearest accepted value, when there is one.
pub fn check(p: &Param, v: &Value) -> Result<(), Option<Value>> {
    if !p.options.is_empty() {
        let t = text(v);
        if p.options
            .iter()
            .any(|o| text(o).eq_ignore_ascii_case(t.trim()))
        {
            return Ok(());
        }
        let Some(m) = magnitude(&t) else {
            return Err(None);
        };
        return Err(p
            .options
            .iter()
            .filter_map(|o| magnitude(&text(o)).map(|x| (o, (x - m).abs())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(o, _)| o.clone()));
    }
    let n = v.as_f64().or_else(|| magnitude(&text(v)));
    match (n, p.min, p.max) {
        (Some(n), Some(lo), _) if n < lo => Err(Some(num_like(p, lo))),
        (Some(n), _, Some(hi)) if n > hi => Err(Some(num_like(p, hi))),
        _ => Ok(()),
    }
}

fn num_like(p: &Param, x: f64) -> Value {
    if p.kind == ParamKind::Integer {
        #[allow(clippy::cast_possible_truncation)] // a schema bound
        let n = x as i64;
        json!(n)
    } else {
        json!(x)
    }
}

fn allowed(p: &Param) -> String {
    if !p.options.is_empty() {
        return p.options.iter().map(text).collect::<Vec<_>>().join(", ");
    }
    match (p.min, p.max) {
        (Some(a), Some(b)) => format!("{a}–{b}"),
        (Some(a), None) => format!("≥ {a}"),
        (None, Some(b)) => format!("≤ {b}"),
        (None, None) => String::new(),
    }
}

/// Values named in the prompt itself: `4k`/`1080p` (resolution), `10 s`/`10 saniye` (duration),
/// `16:9` (aspect ratio). Only for roles the model has; given parameters win.
pub fn prompt_hints(prompt: &str) -> Vec<(Role, String)> {
    let mut out = Vec::new();
    let words: Vec<String> = prompt
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')'))
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches('.').to_lowercase())
        .collect();
    for (i, w) in words.iter().enumerate() {
        let num_prefix = w.chars().next().is_some_and(|c| c.is_ascii_digit());
        if !num_prefix {
            continue;
        }
        if matches!(w.as_str(), "2k" | "4k" | "8k")
            || (w.ends_with('p') && w[..w.len() - 1].parse::<u32>().is_ok_and(|n| n >= 240))
        {
            out.push((Role::Resolution, w.clone()));
        } else if w.contains(':') && magnitude(w).is_some() {
            out.push((Role::AspectRatio, w.clone()));
        } else {
            let unit_glued = ["s", "sec", "secs", "sn"]
                .iter()
                .find_map(|u| w.strip_suffix(u).filter(|n| n.parse::<u32>().is_ok()));
            let unit_next = words.get(i + 1).is_some_and(|n| {
                matches!(
                    n.as_str(),
                    "s" | "sec"
                        | "secs"
                        | "second"
                        | "seconds"
                        | "saniye"
                        | "saniyelik"
                        | "sn"
                        | "seconde"
                        | "secondes"
                )
            });
            if let Some(n) = unit_glued {
                out.push((Role::Duration, n.to_owned()));
            } else if unit_next && w.parse::<u32>().is_ok() {
                out.push((Role::Duration, w.clone()));
            }
        }
    }
    out
}

/// Plan a generation with `model`: start from `defaults` (`media.defaults` for the capability),
/// add `given`, check every given value, ask (when `ask`) about required inputs still missing and
/// the model's choices (size, aspect ratio, resolution, duration, voice, style, audio).
pub fn plan(
    model: &CatalogModel,
    prompt: &str,
    given: &Map<String, Value>,
    defaults: &Map<String, Value>,
    ask: bool,
) -> Plan {
    let mut input = Map::new();
    let mut unsupported = Vec::new();
    for (k, v) in defaults {
        if let Some(p) = model.param(k)
            && check(p, &coerce(p, v)).is_ok()
        {
            input.insert(k.clone(), coerce(p, v));
        }
    }
    let mut given = given.clone();
    for (role, v) in prompt_hints(prompt) {
        if let Some(p) = model.by_role(role)
            && !given.contains_key(&p.name)
        {
            given.insert(p.name.clone(), json!(v));
        }
    }
    for (k, v) in &given {
        let Some(p) = model.param(k) else {
            // Unknown to the catalog (an HF model's extra input): passed as given.
            input.insert(k.clone(), v.clone());
            continue;
        };
        let v = coerce(p, v);
        match check(p, &v) {
            Ok(()) => {
                input.insert(k.clone(), v);
            }
            Err(nearest) => unsupported.push(Unsupported {
                name: k.clone(),
                asked: v,
                nearest,
                allowed: allowed(p),
            }),
        }
    }
    let questions = model
        .params
        .iter()
        .filter(|p| !input.contains_key(&p.name))
        .filter(|p| !matches!(p.role, Role::Prompt | Role::Reference | Role::Mask))
        .filter(|p| unsupported.iter().all(|u| u.name != p.name))
        .filter(|p| {
            (p.required && p.default.is_none())
                || (ask
                    && p.role.is_choice()
                    && (!p.options.is_empty() || p.kind == ParamKind::Boolean))
        })
        .map(|p| Question {
            name: p.name.clone(),
            role: p.role,
            kind: p.kind,
            description: p.description.clone(),
            options: if p.kind == ParamKind::Boolean && p.options.is_empty() {
                vec![json!(true), json!(false)]
            } else {
                p.options.clone()
            },
            min: p.min,
            max: p.max,
            default: p.default.clone(),
        })
        .collect();
    let estimate = estimate(model, prompt, &input);
    Plan {
        input,
        questions,
        unsupported,
        estimate,
    }
}

/// Cost of one generation from the list price and the inputs.
pub fn estimate(
    model: &CatalogModel,
    prompt: &str,
    input: &Map<String, Value>,
) -> Option<Estimate> {
    let price = model.price.as_ref()?;
    let unit = price.unit.to_ascii_lowercase();
    let value = |role: Role| {
        model.by_role(role).and_then(|p| {
            input
                .get(&p.name)
                .or(p.default.as_ref())
                .map(|v| (v.clone(), input.contains_key(&p.name)))
        })
    };
    let (units, estimated): (f64, bool) = if unit.starts_with("image") {
        let n = value(Role::Count)
            .and_then(|(v, _)| v.as_f64())
            .unwrap_or(1.0);
        (n.max(1.0), false)
    } else if unit.starts_with("second") || unit.starts_with("minute") {
        let (secs, known) = value(Role::Duration)
            .and_then(|(v, k)| magnitude(&text(&v)).map(|n| (n, k)))
            .unwrap_or((5.0, false));
        let per = if unit.starts_with("minute") {
            secs / 60.0
        } else {
            secs
        };
        // An input file's length (transcription) is not known before the run.
        (per, !known || model.capabilities.contains(&Capability::Asr))
    } else if unit.starts_with("megapixel") {
        let w = input.get("width").and_then(Value::as_f64);
        let h = input.get("height").and_then(Value::as_f64);
        let mp = match (w, h) {
            (Some(w), Some(h)) => w * h / 1e6,
            _ => 1.0,
        };
        (mp.max(1.0).ceil(), true)
    } else if unit.contains("character") {
        #[allow(clippy::cast_precision_loss)] // prompt length
        let chars = prompt.chars().count() as f64;
        let per = if unit.contains("1k") || unit.contains("1000") {
            chars / 1000.0
        } else {
            chars
        };
        (per, false)
    } else if matches!(
        unit.as_str(),
        "video"
            | "videos"
            | "request"
            | "requests"
            | "generation"
            | "generations"
            | "call"
            | "calls"
            | "unit"
            | "units"
    ) {
        (1.0, false)
    } else {
        (1.0, true)
    };
    Some(Estimate {
        amount: price.amount * units,
        currency: price.currency.clone(),
        estimated,
    })
}

/// Other models with `cap` whose parameter of the same role accepts `value`, cheapest first
/// (unpriced last), for "switch to a model that can" (§20.2, with the §5 cost comparison).
pub fn alternatives<'a>(
    models: &'a [CatalogModel],
    cap: Capability,
    exclude: &CatalogModel,
    role: Role,
    value: &Value,
    prompt: &str,
) -> Vec<(&'a CatalogModel, Option<Estimate>)> {
    let mut out: Vec<(&CatalogModel, Option<Estimate>)> = models
        .iter()
        .filter(|m| m.capabilities.contains(&cap) && m.reference() != exclude.reference())
        .filter_map(|m| {
            let p = m.by_role(role)?;
            let v = coerce(p, value);
            check(p, &v).ok()?;
            let mut input = Map::new();
            input.insert(p.name.clone(), v);
            Some((m, estimate(m, prompt, &input)))
        })
        .collect();
    out.sort_by(|a, b| match (&a.1, &b.1) {
        (Some(x), Some(y)) => x.amount.total_cmp(&y.amount),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.id.cmp(&b.0.id),
    });
    out.truncate(5);
    out
}

/// Role of the parameter `name` in `model` (for [`alternatives`]).
pub fn role_of(model: &CatalogModel, name: &str) -> Role {
    model.param(name).map_or(Role::Other, |p| p.role)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn video_model() -> CatalogModel {
        from_venice(
            &MediaModel {
                id: "wan".into(),
                model_type: "text-to-video".into(),
                price_usd: None,
                durations: vec!["5s".into(), "8s".into()],
                resolutions: vec!["1080p".into(), "720p".into()],
                aspect_ratios: vec!["16:9".into(), "9:16".into(), "1:1".into()],
                sized_by_dimensions: false,
            },
            MediaKind::Video,
        )
    }

    fn fal_schema() -> Value {
        json!({
            "required": ["prompt"],
            "properties": {
                "prompt": {"type": "string"},
                "image_size": {"anyOf": [{"$ref": "#/x"}, {"type": "string", "enum": ["square_hd", "landscape_16_9"]}], "default": "landscape_16_9"},
                "num_images": {"type": "integer", "minimum": 1, "maximum": 4, "default": 1},
                "seed": {"type": "integer"},
                "image_url": {"type": "string"},
                "generate_audio": {"type": "boolean", "default": true},
                "duration": {"type": "string", "enum": ["5", "10"], "default": "5"}
            }
        })
    }

    #[test]
    fn schema_params_are_typed_with_roles() {
        let p = params_from_schema(&fal_schema());
        assert_eq!(p[0].name, "prompt", "required first");
        let size = p.iter().find(|p| p.name == "image_size").unwrap();
        assert_eq!((size.kind, size.role), (ParamKind::Enum, Role::Size));
        assert_eq!(size.options.len(), 2);
        let n = p.iter().find(|p| p.name == "num_images").unwrap();
        assert_eq!(
            (n.kind, n.min, n.max),
            (ParamKind::Integer, Some(1.0), Some(4.0))
        );
        let img = p.iter().find(|p| p.name == "image_url").unwrap();
        assert_eq!((img.kind, img.role), (ParamKind::File, Role::Reference));
    }

    #[test]
    fn capabilities_from_fal_listing() {
        assert_eq!(
            fal_capabilities("image-to-image", "fal-ai/birefnet/v2", ""),
            [Capability::RemoveBg]
        );
        assert_eq!(
            fal_capabilities("image-to-image", "fal-ai/esrgan", ""),
            [Capability::Upscale]
        );
        assert_eq!(
            fal_capabilities("image-to-image", "fal-ai/flux/dev/image-to-image", ""),
            [Capability::Edit]
        );
        assert_eq!(
            fal_capabilities("text-to-audio", "fal-ai/lyria2", "Lyria 2"),
            [Capability::Music]
        );
        assert_eq!(
            fal_capabilities("text-to-audio", "fal-ai/mmaudio", ""),
            [Capability::Audio]
        );
        assert!(fal_capabilities("video-to-video", "x", "").is_empty());
        assert_eq!(Capability::parse("text-to-image"), Some(Capability::Image));
        assert_eq!(Capability::parse("remove-bg"), Some(Capability::RemoveBg));
    }

    #[test]
    fn plan_asks_choices_and_flags_unsupported_with_nearest() {
        let m = video_model();
        // "4k, 10 seconds" in the prompt: this model has neither.
        let p = plan(
            &m,
            "a cat video in 4k, 10 seconds",
            &Map::new(),
            &Map::new(),
            true,
        );
        let nearest = |n: &str| {
            p.unsupported
                .iter()
                .find(|u| u.name == n)
                .and_then(|u| u.nearest.clone())
        };
        assert_eq!(p.unsupported.len(), 2);
        assert_eq!(nearest("resolution"), Some(json!("1080p")));
        assert_eq!(nearest("duration"), Some(json!("8s")));
        assert_eq!(p.questions.len(), 1, "only the aspect ratio is still open");
        assert_eq!(p.questions[0].default, Some(json!("16:9")));

        let mut defaults = Map::new();
        defaults.insert("aspect_ratio".into(), json!("9:16"));
        defaults.insert("duration".into(), json!("5s"));
        defaults.insert("resolution".into(), json!("720p"));
        let p = plan(&m, "a cat", &Map::new(), &defaults, true);
        assert!(p.questions.is_empty(), "saved defaults are not asked again");
        assert_eq!(p.input["aspect_ratio"], "9:16");
        let p = plan(&m, "a cat", &Map::new(), &Map::new(), false);
        assert!(p.questions.is_empty(), "--no-ask asks nothing optional");
    }

    #[test]
    fn coercion_and_ranges() {
        let params = params_from_schema(&fal_schema());
        let n = params.iter().find(|p| p.name == "num_images").unwrap();
        assert_eq!(coerce(n, &json!("3")), json!(3));
        assert_eq!(check(n, &json!(9)), Err(Some(json!(4))));
        let d = params.iter().find(|p| p.name == "duration").unwrap();
        assert_eq!(
            coerce(d, &json!(10)),
            json!("10"),
            "string enum keeps its type"
        );
        let a = params.iter().find(|p| p.name == "generate_audio").unwrap();
        assert_eq!(coerce(a, &json!("evet")), json!(true));
        assert_eq!(
            prompt_hints("make it 1080p, 16:9 and 6s long"),
            vec![
                (Role::Resolution, "1080p".into()),
                (Role::AspectRatio, "16:9".into()),
                (Role::Duration, "6".into())
            ]
        );
    }

    #[test]
    fn estimates_mark_guesses() {
        let mut m = from_fal(
            &FalModel {
                endpoint_id: "fal-ai/x".into(),
                name: "X".into(),
                category: "text-to-image".into(),
                description: String::new(),
            },
            Some(&fal_schema()),
            Some(&fal::Price {
                unit_price: 0.03,
                unit: "image".into(),
                currency: "USD".into(),
            }),
        )
        .unwrap();
        let mut input = Map::new();
        input.insert("num_images".into(), json!(2));
        let e = estimate(&m, "p", &input).unwrap();
        assert!((e.amount - 0.06).abs() < 1e-9 && !e.estimated);
        assert_eq!(e.display(), "$0.06");
        m.price = Some(Price {
            amount: 0.1,
            currency: "USD".into(),
            unit: "seconds".into(),
        });
        let e = estimate(&m, "p", &Map::new()).unwrap();
        assert!(e.estimated, "the model's default duration is a guess");
        assert!(e.display().starts_with('≈'));
    }

    #[test]
    fn alternatives_are_cheapest_first() {
        let base = video_model();
        let mut a = video_model();
        a.id = "kling".into();
        a.params
            .iter_mut()
            .find(|p| p.name == "resolution")
            .unwrap()
            .options
            .push(json!("4k"));
        a.price = Some(Price {
            amount: 1.0,
            currency: "USD".into(),
            unit: "video".into(),
        });
        let mut b = a.clone();
        b.id = "veo".into();
        b.price = Some(Price {
            amount: 0.5,
            currency: "USD".into(),
            unit: "video".into(),
        });
        let all = vec![base.clone(), a, b];
        let alt = alternatives(
            &all,
            Capability::Video,
            &base,
            Role::Resolution,
            &json!("4k"),
            "",
        );
        let ids: Vec<&str> = alt.iter().map(|(m, _)| m.id.as_str()).collect();
        assert_eq!(ids, ["veo", "kling"]);
    }

    #[tokio::test]
    async fn fetches_fal_catalog_with_schemas_and_prices() {
        let server = MockServer::start().await;
        let manifest =
            crate::ProviderManifest::parse(include_str!("../../../providers/fal.toml"), "fal.toml")
                .unwrap();
        let fal = Fal::with_bases(
            &manifest,
            kor_secrets::SecretString::from("k".to_owned()),
            &server.uri(),
            Some(&server.uri()),
            "",
        )
        .unwrap();
        let openapi = json!({"paths": {"/fal-ai/flux/schnell": {"post": {"requestBody": {"content": {"application/json": {"schema": fal_schema()}}}}}}});
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("category", "text-to-image"))
            .and(query_param("expand", "openapi-3.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"endpoint_id": "fal-ai/flux/schnell", "metadata": {"display_name": "FLUX schnell", "category": "text-to-image"}, "openapi": openapi}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models/pricing"))
            .and(query_param("endpoint_id", "fal-ai/flux/schnell"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"prices": [
                {"endpoint_id": "fal-ai/flux/schnell", "unit_price": 0.003, "unit": "megapixels", "currency": "USD"}
            ]})))
            .mount(&server)
            .await;
        let (models, errors) = fetch(
            &Sources {
                fal: Some(&fal),
                ..Sources::default()
            },
            &[Capability::Image],
        )
        .await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.reference(), "fal:fal-ai/flux/schnell");
        assert_eq!(m.price.as_ref().unwrap().unit, "megapixels");
        assert!(m.accepts_reference);
        assert!(m.by_role(Role::Size).is_some());
    }
}
