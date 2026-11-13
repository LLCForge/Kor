//! Image and video generation tools backed by Venice: `media_models`, `generate_image`,
//! `generate_video`; and, backed by fal.ai and/or Hugging Face, `image_generate`, `image_edit`,
//! `video_generate`, `speech_to_text`, `text_to_speech` (each with a `provider` field),
//! `fal_run` and `fal_models` (see [`Studio`]); and `direct_generate` for OpenAI Images, Imagen,
//! Veo, Stability, Runway, Luma and a local `ComfyUI` ([`DirectMedia`]). Files are written to `<project>/kor-media/`; each tool output names them on
//! `saved: <absolute path>` lines, which the desktop app turns into previews.
//!
//! Generation costs money, so both generators are `Execute` tools and ask before running unless a
//! rule allows them. The API key lives inside [`VeniceMedia`]; the model never sees it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use kor_core::ToolSpec;
use kor_providers::fal::{Fal, MissingInput, describe, missing_inputs, output_files};
use kor_providers::huggingface::Hf;
use kor_providers::media::{
    ImageRequest, MediaKind, MediaModel, VeniceMedia, VideoRequest, VideoStatus,
};
use kor_providers::media_catalog::{self as catalog, Capability, CatalogModel};
use kor_tools::{ParamCard, ParamField, Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Map, Value, json};

use crate::gallery::{self, Meta};
use tokio::sync::OnceCell;

mod direct;
pub use direct::{DirectGenerate, DirectMedia, direct_tools};

/// Cheapest Venice image model ($0.01); sized by width/height.
pub const DEFAULT_IMAGE_MODEL: &str = "venice-sd35";
/// Cheapest text-to-video model at the time of writing (4 s ≈ $0.23).
pub const DEFAULT_TEXT_VIDEO_MODEL: &str = "seedance-1-5-pro-text-to-video-basic";
pub const DEFAULT_IMAGE_VIDEO_MODEL: &str = "seedance-1-5-pro-image-to-video-basic";
/// Output folder under the project root.
pub const MEDIA_DIR: &str = "kor-media";
const POLL_EVERY: Duration = Duration::from_secs(5);
const VIDEO_DEADLINE: Duration = Duration::from_secs(20 * 60);
/// A start frame larger than this is refused before upload.
const MAX_START_IMAGE: u64 = 10 * 1024 * 1024;

/// What the media tools follow (`[media]`, ADR 0060) and where media costs are recorded.
#[derive(Debug, Clone, Default)]
pub struct MediaEnv {
    /// The usage ledger (`kor cost`); media lines are marked as such.
    pub ledger: Option<PathBuf>,
    /// Per capability key (`image`, `video`...): saved parameter values, asked no more (§20.2).
    pub defaults: std::collections::BTreeMap<String, Map<String, Value>>,
    /// A generation estimated above this many dollars needs a yes (0: never).
    pub confirm_above: f64,
    /// Never ask about optional choices (`media.ask = false`); required ones still are.
    pub no_ask: bool,
}

impl MediaEnv {
    pub fn defaults_for(&self, cap: Option<Capability>) -> Map<String, Value> {
        cap.and_then(|c| self.defaults.get(c.key()))
            .cloned()
            .unwrap_or_default()
    }
}

/// Shared by the three tools of one session: the client and the model lists, fetched once.
#[derive(Debug)]
pub struct Media {
    venice: VeniceMedia,
    images: OnceCell<Vec<MediaModel>>,
    videos: OnceCell<Vec<MediaModel>>,
    env: MediaEnv,
}

impl Media {
    pub fn new(venice: VeniceMedia) -> Arc<Self> {
        Self::with_env(venice, MediaEnv::default())
    }

    pub fn with_env(venice: VeniceMedia, env: MediaEnv) -> Arc<Self> {
        Arc::new(Self {
            venice,
            images: OnceCell::new(),
            videos: OnceCell::new(),
            env,
        })
    }

    /// The Venice client (`kor media collect`).
    pub fn venice(&self) -> &VeniceMedia {
        &self.venice
    }

    async fn models(&self, kind: MediaKind) -> Result<&[MediaModel], String> {
        let cell = match kind {
            MediaKind::Image => &self.images,
            MediaKind::Video => &self.videos,
        };
        cell.get_or_try_init(|| async { self.venice.models(kind).await.map_err(|e| e.to_string()) })
            .await
            .map(Vec::as_slice)
    }

    async fn model(&self, kind: MediaKind, id: &str) -> Result<MediaModel, String> {
        self.models(kind)
            .await?
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| format!("unknown model `{id}`; call `media_models` for the list"))
    }
}

/// The three tools sharing one client.
pub fn tools(media: &Arc<Media>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(MediaModels(media.clone())),
        Arc::new(GenerateImage(media.clone())),
        Arc::new(GenerateVideo(media.clone())),
    ]
}

fn str_arg<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A short file stem: the given name or the first words of the prompt, ASCII letters and digits.
pub fn stem(name: Option<&str>, prompt: &str) -> String {
    let source = name.unwrap_or(prompt);
    let mut s = String::new();
    for c in source.chars() {
        let c = match c {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'İ' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
        if s.len() >= 40 {
            break;
        }
    }
    let s = s.trim_matches('-').to_owned();
    if s.is_empty() { "media".to_owned() } else { s }
}

/// `n` in lowercase letters (base 26). File names carry no digits on purpose: the secret redactor
/// treats long runs mixing upper case, lower case and digits as keys, and an absolute path such as
/// `/Users/Ada/proj/kor-media/x-1794249351` would be masked in the tool output.
fn letters(mut n: u64) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'a' + u8::try_from(n % 26).unwrap_or(0));
        n /= 26;
        if n == 0 {
            break;
        }
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn time_tag() -> String {
    letters(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
}

/// Write `bytes` to `<root>/kor-media/<stem>-<time>[-n].<ext>` without overwriting anything.
fn save(
    root: &Path,
    stem: &str,
    n: Option<usize>,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let dir = root.join(MEDIA_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {MEDIA_DIR}: {e}"))?;
    let base = match n {
        Some(i) => format!(
            "{stem}-{}-{}",
            time_tag(),
            letters(u64::try_from(i).unwrap_or(0) + 1)
        ),
        None => format!("{stem}-{}", time_tag()),
    };
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("{base}.{ext}")
        } else {
            format!("{base}-{}.{ext}", letters(attempt))
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                std::io::Write::write_all(&mut f, bytes).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("could not find a free file name".into())
}

/// Native separators: on Windows `\` breaks the path into short pieces the redactor leaves alone.
fn shown(p: &Path) -> String {
    kor_tools::display_path(p)
}

/// `preferred` when the model offers it; otherwise its first (or, for resolutions listed high to
/// low, its last and cheapest) option; `None` when the model lists none (the server decides).
fn default_choice<'a>(allowed: &'a [String], preferred: &str, last: bool) -> Option<&'a str> {
    allowed
        .iter()
        .find(|a| *a == preferred)
        .or_else(|| {
            if last {
                allowed.last()
            } else {
                allowed.first()
            }
        })
        .map(String::as_str)
}

fn check_choice(what: &str, value: Option<&str>, allowed: &[String]) -> Result<(), String> {
    match value {
        Some(v) if !allowed.is_empty() && !allowed.iter().any(|a| a == v) => Err(format!(
            "{what} `{v}` is not supported by this model; use one of: {}",
            allowed.join(", ")
        )),
        _ => Ok(()),
    }
}

/// `derived_from`: a gallery item this output comes from without being an input file.
fn derived_from(input: &Value) -> Option<PathBuf> {
    str_arg(input, "derived_from")
        .filter(|id| !id.starts_with('.') && !id.contains(['/', '\\']) && !id.contains(".."))
        .map(|id| Path::new(MEDIA_DIR).join(id))
}

/// A file input: a gallery id (`kor-media/<id>`) or a project path.
fn resolve_input(ctx: &ToolCtx, s: &str) -> Result<PathBuf, String> {
    gallery::resolve(&ctx.root, s).or_else(|_| ctx.resolve(s))
}

fn value_text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}

/// The parameter card for a planned call (§20.2), or `None` when there is nothing to ask or fix.
fn card_from_plan(
    title: String,
    model: &CatalogModel,
    plan: &catalog::Plan,
    env: &MediaEnv,
    root: &Path,
    target: Option<&str>,
) -> Option<ParamCard> {
    let mut fields: Vec<ParamField> = plan
        .questions
        .iter()
        .map(|q| ParamField {
            name: q.name.clone(),
            description: q.description.clone(),
            options: q.options.clone(),
            default: q.default.clone(),
            min: q.min,
            max: q.max,
        })
        .collect();
    let mut notes = Vec::new();
    for u in &plan.unsupported {
        let near = u
            .nearest
            .as_ref()
            .map(|n| format!("; nearest: {}", value_text(n)))
            .unwrap_or_default();
        notes.push(format!(
            "{} `{}` is not supported by {}{near} (allowed: {})",
            u.name,
            value_text(&u.asked),
            model.id,
            u.allowed
        ));
        if let Some(p) = model.param(&u.name) {
            fields.push(ParamField {
                name: p.name.clone(),
                description: p.description.clone(),
                options: p.options.clone(),
                default: u.nearest.clone().or_else(|| p.default.clone()),
                min: p.min,
                max: p.max,
            });
        }
    }
    let confirm = env.confirm_above > 0.0
        && plan
            .estimate
            .as_ref()
            .is_some_and(|e| e.currency == "USD" && e.amount > env.confirm_above);
    let card = ParamCard {
        title,
        fields,
        notes,
        cost: plan.estimate.as_ref().map(catalog::Estimate::display),
        eta: gallery::eta(root, &model.id).map(|s| format!("≈ {s} s")),
        confirm,
        target: target.map(str::to_owned),
        fixed: plan.input.clone(),
    };
    (!card.is_empty() || !card.fixed.is_empty()).then_some(card)
}

/// Record outputs in the gallery, charge the ledger, and copy the first one to `save_as`.
fn finish(
    env: &MediaEnv,
    ctx: &ToolCtx,
    paths: &[PathBuf],
    meta: &Meta,
    input: &Value,
    out: &mut String,
) {
    gallery::record(&ctx.root, paths, meta);
    gallery::charge(
        env.ledger.as_deref(),
        &format!("{}/{}", meta.provider, meta.model),
        meta.cost.as_ref(),
    );
    copy_to(ctx, paths, input, out);
}

/// `save_as`: copy the first output into the project (never over an existing file).
fn copy_to(ctx: &ToolCtx, paths: &[PathBuf], input: &Value, out: &mut String) {
    if let (Some(dest), Some(first)) = (str_arg(input, "save_as"), paths.first()) {
        let copied = ctx.resolve(dest).and_then(|to| {
            if to.exists() {
                return Err(format!("{dest} already exists; not overwritten"));
            }
            if let Some(d) = to.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::copy(first, &to)
                .map(|_| to)
                .map_err(|e| e.to_string())
        });
        match copied {
            Ok(to) => {
                let _ = writeln!(out, "copied: {}", shown(&to));
            }
            Err(e) => {
                let _ = writeln!(out, "not copied to {dest}: {e}");
            }
        }
    }
}

/// Optional inputs every media tool takes (gallery history and project output, §20.3).
fn with_media_props(mut schema: Value) -> Value {
    let extra = json!({
        "save_as": {"type": "string", "description": "Also copy the first output to this project path, e.g. public/logo.png"},
        "action": {"type": "string", "description": "Quick action on a gallery item, kept in its history: variation, reseed, upscale, video, remove-bg, edit"},
        "capability": {"type": "string", "description": "Catalog capability (image, edit, upscale, remove-bg, video, image-to-video, tts, asr, audio, music), kept in the gallery"},
        "derived_from": {"type": "string", "description": "Gallery id this output derives from without being sent as input (a variation, the same seed with a new prompt)"},
    });
    if let (Some(props), Some(extra)) = (
        schema.get_mut("properties").and_then(Value::as_object_mut),
        extra.as_object(),
    ) {
        for (k, v) in extra {
            props.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    schema
}

/// Venice tools: plan the call against the model's constraints (top-level inputs).
async fn venice_card(
    media: &Media,
    kind: MediaKind,
    input: &Value,
    ctx: &ToolCtx,
) -> Option<ParamCard> {
    let default = match kind {
        MediaKind::Image => DEFAULT_IMAGE_MODEL,
        MediaKind::Video if input.get("image").is_some() => DEFAULT_IMAGE_VIDEO_MODEL,
        MediaKind::Video => DEFAULT_TEXT_VIDEO_MODEL,
    };
    let id = str_arg(input, "model").unwrap_or(default);
    let m = media.model(kind, id).await.ok()?;
    let cm = catalog::from_venice(&m, kind);
    let cap = cm.capabilities.first().copied();
    let prompt = str_arg(input, "prompt").unwrap_or_default();
    let given: Map<String, Value> = input
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| {
            cm.param(k).is_some_and(|p| {
                !matches!(p.role, catalog::Role::Prompt | catalog::Role::Reference)
            })
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let ask = input.get("ask").and_then(Value::as_bool) != Some(false) && !media.env.no_ask;
    let mut plan = catalog::plan(&cm, prompt, &given, &media.env.defaults_for(cap), ask);
    if kind == MediaKind::Video {
        // Venice quotes a video before generating: an exact price, not an estimate.
        let s = |k: &str| plan.input.get(k).and_then(Value::as_str).map(str::to_owned);
        let req = VideoRequest {
            model: id.to_owned(),
            prompt: prompt.to_owned(),
            duration: s("duration")
                .or_else(|| m.durations.first().cloned())
                .unwrap_or_else(|| "5s".into()),
            aspect_ratio: s("aspect_ratio")
                .or_else(|| default_choice(&m.aspect_ratios, "16:9", false).map(str::to_owned)),
            resolution: s("resolution")
                .or_else(|| default_choice(&m.resolutions, "720p", true).map(str::to_owned)),
            image_url: None,
        };
        if let Ok(q) = media.venice.video_quote(&req).await {
            plan.estimate = Some(catalog::Estimate {
                amount: q,
                currency: "USD".into(),
                estimated: false,
            });
        }
    }
    let title: String = format!("{id}: {prompt}").chars().take(100).collect();
    card_from_plan(title, &cm, &plan, &media.env, &ctx.root, None)
}

// ---------------------------------------------------------------------------------------------
// media_models
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct MediaModels(Arc<Media>);

#[async_trait]
impl Tool for MediaModels {
    fn name(&self) -> &'static str {
        "media_models"
    }
    fn rule_name(&self) -> &'static str {
        "media_models"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_owned(),
            description: "List the image or video generation models (Venice) with price and the sizes, aspect ratios, \
                durations and resolutions each accepts. Free; use it to choose a model for generate_image or generate_video."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"kind": {"type": "string", "enum": ["image", "video"]}},
                "required": ["kind"],
            }),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        str_arg(input, "kind").map(str::to_owned)
    }
    async fn call(&self, input: Value, _ctx: &ToolCtx) -> ToolOutput {
        let kind = match str_arg(&input, "kind") {
            Some("video") => MediaKind::Video,
            Some("image") => MediaKind::Image,
            _ => return ToolOutput::err("`kind` must be \"image\" or \"video\""),
        };
        let models = match self.0.models(kind).await {
            Ok(m) => m,
            Err(e) => return ToolOutput::err(e),
        };
        let mut out = String::new();
        for m in models {
            let price = m.price_usd.map_or_else(String::new, |p| format!(" ${p}"));
            let opts = match kind {
                MediaKind::Image if m.sized_by_dimensions => "width/height".to_owned(),
                MediaKind::Image => format!("aspect_ratio {}", m.aspect_ratios.join(" ")),
                MediaKind::Video => format!(
                    "{} | durations {} | resolutions {} | aspect {}",
                    m.model_type,
                    m.durations.join(" "),
                    m.resolutions.join(" "),
                    m.aspect_ratios.join(" ")
                ),
            };
            let _ = writeln!(out, "- {}{price}: {opts}", m.id);
        }
        if kind == MediaKind::Video {
            out.push_str("Video prices depend on duration and resolution; generate_video reports the quote.\n");
        }
        ToolOutput::ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// generate_image
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct GenerateImage(Arc<Media>);

#[async_trait]
impl Tool for GenerateImage {
    fn name(&self) -> &'static str {
        "generate_image"
    }
    fn rule_name(&self) -> &'static str {
        "generate_image"
    }
    /// Paid network call: asks unless allowed by a rule.
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_owned(),
            description: format!(
                "Generate images from a text prompt (Venice, paid per image) and save them in the project under \
                 {MEDIA_DIR}/. Default model {DEFAULT_IMAGE_MODEL} (sized with width/height, 256-1280); most other \
                 models take aspect_ratio instead (see media_models). Write the prompt in English for best results. \
                 The output lists the saved files."
            ),
            input_schema: with_media_props(json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "What to draw"},
                    "model": {"type": "string"},
                    "negative_prompt": {"type": "string"},
                    "width": {"type": "integer", "minimum": 256, "maximum": 1280},
                    "height": {"type": "integer", "minimum": 256, "maximum": 1280},
                    "aspect_ratio": {"type": "string", "description": "e.g. 16:9, for models that use it"},
                    "format": {"type": "string", "enum": ["png", "jpeg", "webp"]},
                    "variants": {"type": "integer", "minimum": 1, "maximum": 4},
                    "name": {"type": "string", "description": "File name stem (optional)"},
                },
                "required": ["prompt"],
            })),
        }
    }
    fn target(&self, _input: &Value) -> Option<String> {
        None
    }
    async fn param_card(&self, input: &Value, ctx: &ToolCtx) -> Option<ParamCard> {
        venice_card(&self.0, MediaKind::Image, input, ctx).await
    }
    fn summary(&self, input: &Value) -> String {
        let model = str_arg(input, "model").unwrap_or(DEFAULT_IMAGE_MODEL);
        let prompt: String = str_arg(input, "prompt")
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect();
        format!("{model}: {prompt}")
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let started = std::time::Instant::now();
        let Some(prompt) = str_arg(&input, "prompt") else {
            return ToolOutput::err("`prompt` is required");
        };
        let model_id = str_arg(&input, "model").unwrap_or(DEFAULT_IMAGE_MODEL);
        let model = match self.0.model(MediaKind::Image, model_id).await {
            Ok(m) => m,
            Err(e) => return ToolOutput::err(e),
        };
        let dim = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
        };
        let (width, height) = (dim("width"), dim("height"));
        let aspect = str_arg(&input, "aspect_ratio");
        if (width.is_some() || height.is_some()) && !model.sized_by_dimensions {
            return ToolOutput::err(format!(
                "`{model_id}` is sized by aspect_ratio, not width/height; use one of: {}",
                model.aspect_ratios.join(", ")
            ));
        }
        if let Err(e) = check_choice("aspect_ratio", aspect, &model.aspect_ratios) {
            return ToolOutput::err(e);
        }
        let format = match str_arg(&input, "format") {
            Some(f @ ("png" | "jpeg" | "webp")) => f.to_owned(),
            Some(f) => return ToolOutput::err(format!("unsupported format `{f}`")),
            None => "png".to_owned(),
        };
        let variants = input
            .get("variants")
            .and_then(Value::as_u64)
            .map_or(1, |v| u8::try_from(v.clamp(1, 4)).unwrap_or(1));
        let req = ImageRequest {
            model: model_id.to_owned(),
            prompt: prompt.to_owned(),
            negative_prompt: str_arg(&input, "negative_prompt").map(str::to_owned),
            width,
            height,
            aspect_ratio: aspect.map(str::to_owned),
            format: format.clone(),
            variants,
        };
        let images = tokio::select! {
            r = self.0.venice.image(&req) => r,
            () = ctx.cancel.cancelled() => return ToolOutput::err("cancelled"),
        };
        let images = match images {
            Ok(i) => i,
            Err(e) => return ToolOutput::err(format!("image generation failed: {e}")),
        };
        let stem = stem(str_arg(&input, "name"), prompt);
        let many = images.len() > 1;
        let mut out = String::new();
        let cost = model.price_usd.map_or_else(String::new, |p| {
            #[allow(clippy::cast_precision_loss)] // at most four images
            let total = p * images.len() as f64;
            format!(" (about ${total:.2})")
        });
        let _ = writeln!(
            out,
            "Generated {} image(s) with {model_id}{cost}.",
            images.len()
        );
        let mut paths = Vec::new();
        for (i, bytes) in images.iter().enumerate() {
            match save(&ctx.root, &stem, many.then_some(i), &format, bytes) {
                Ok(p) => {
                    let _ = writeln!(out, "saved: {}", shown(&p));
                    paths.push(p);
                }
                Err(e) => return ToolOutput::err(format!("could not save the image: {e}")),
            }
        }
        #[allow(clippy::cast_precision_loss)] // at most four images
        let cost = model
            .price_usd
            .map(|p| (p * images.len() as f64, "USD".to_owned(), false));
        let meta = Meta {
            capability: Some(Capability::Image.key().into()),
            provider: "venice".into(),
            model: model_id.to_owned(),
            prompt: prompt.to_owned(),
            params: input.clone(),
            seed: input.get("seed").and_then(Value::as_i64),
            cost,
            secs: started.elapsed().as_secs(),
            inputs: derived_from(&input).into_iter().collect(),
            action: str_arg(&input, "action").map(str::to_owned),
        };
        finish(&self.0.env, ctx, &paths, &meta, &input, &mut out);
        ToolOutput::ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// generate_video
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct GenerateVideo(Arc<Media>);

fn image_data_url(path: &Path) -> Result<String, String> {
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => return Err("the start image must be .png, .jpg or .webp".into()),
    };
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_START_IMAGE {
        return Err(format!(
            "the start image is larger than {} MB",
            MAX_START_IMAGE / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[async_trait]
impl Tool for GenerateVideo {
    fn name(&self) -> &'static str {
        "generate_video"
    }
    fn rule_name(&self) -> &'static str {
        "generate_video"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_owned(),
            description: format!(
                "Generate a short video (Venice, paid; usually $0.2-$2 and 1-5 minutes) from a text prompt, or from a \
                 start image in the project (image-to-video). Saves an MP4 under {MEDIA_DIR}/. Defaults: \
                 {DEFAULT_TEXT_VIDEO_MODEL}, or {DEFAULT_IMAGE_VIDEO_MODEL} when `image` is given, with the model's \
                 shortest duration. Each model accepts specific durations/resolutions/aspect ratios (see media_models). \
                 Write the prompt in English for best results."
            ),
            input_schema: with_media_props(json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "What happens in the video"},
                    "model": {"type": "string"},
                    "duration": {"type": "string", "description": "e.g. 4s, 5s; must be one the model accepts"},
                    "resolution": {"type": "string", "description": "e.g. 720p"},
                    "aspect_ratio": {"type": "string", "description": "e.g. 16:9"},
                    "image": {"type": "string", "description": "Project path or gallery id of a start image (png/jpg/webp) for image-to-video"},
                    "name": {"type": "string", "description": "File name stem (optional)"},
                },
                "required": ["prompt"],
            })),
        }
    }
    fn target(&self, _input: &Value) -> Option<String> {
        None
    }
    async fn param_card(&self, input: &Value, ctx: &ToolCtx) -> Option<ParamCard> {
        venice_card(&self.0, MediaKind::Video, input, ctx).await
    }
    fn summary(&self, input: &Value) -> String {
        let model = str_arg(input, "model").unwrap_or(if input.get("image").is_some() {
            DEFAULT_IMAGE_VIDEO_MODEL
        } else {
            DEFAULT_TEXT_VIDEO_MODEL
        });
        let duration = str_arg(input, "duration").unwrap_or("shortest");
        let prompt: String = str_arg(input, "prompt")
            .unwrap_or_default()
            .chars()
            .take(70)
            .collect();
        format!("{model}, {duration}: {prompt}")
    }
    #[allow(clippy::too_many_lines)] // validate, quote, queue, poll, download, save: one flow in order
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(prompt) = str_arg(&input, "prompt") else {
            return ToolOutput::err("`prompt` is required");
        };
        let image = str_arg(&input, "image");
        let model_id = str_arg(&input, "model").unwrap_or(if image.is_some() {
            DEFAULT_IMAGE_VIDEO_MODEL
        } else {
            DEFAULT_TEXT_VIDEO_MODEL
        });
        let model = match self.0.model(MediaKind::Video, model_id).await {
            Ok(m) => m,
            Err(e) => return ToolOutput::err(e),
        };
        match (image.is_some(), model.model_type.as_str()) {
            (true, "text-to-video") => {
                return ToolOutput::err(format!(
                    "`{model_id}` is text-to-video; pick an image-to-video model for a start image"
                ));
            }
            (false, "image-to-video") => {
                return ToolOutput::err(format!(
                    "`{model_id}` needs a start `image`; use a text-to-video model otherwise"
                ));
            }
            (_, "video") => {
                return ToolOutput::err(format!(
                    "`{model_id}` edits an existing video; it is not supported here"
                ));
            }
            _ => {}
        }
        let duration = match str_arg(&input, "duration") {
            Some(d) => d.to_owned(),
            None => match model.durations.first() {
                Some(d) => d.clone(),
                None => "5s".to_owned(),
            },
        };
        // Some models require these; when the caller leaves them out, pick 16:9 and 720p (or the
        // model's first ratio and lowest resolution) so the request is valid and the quote matches it.
        let aspect = str_arg(&input, "aspect_ratio")
            .or_else(|| default_choice(&model.aspect_ratios, "16:9", false));
        let resolution = str_arg(&input, "resolution")
            .or_else(|| default_choice(&model.resolutions, "720p", true));
        for (what, value, allowed) in [
            ("duration", Some(duration.as_str()), &model.durations),
            ("resolution", resolution, &model.resolutions),
            ("aspect_ratio", aspect, &model.aspect_ratios),
        ] {
            if let Err(e) = check_choice(what, value, allowed) {
                return ToolOutput::err(e);
            }
        }
        let image_path = match image.map(|p| resolve_input(ctx, p)) {
            Some(Ok(p)) => Some(p),
            Some(Err(e)) => return ToolOutput::err(format!("start image: {e}")),
            None => None,
        };
        let image_url = match image_path.as_deref().map(image_data_url) {
            Some(Ok(url)) => Some(url),
            Some(Err(e)) => return ToolOutput::err(format!("start image: {e}")),
            None => None,
        };
        let req = VideoRequest {
            model: model_id.to_owned(),
            prompt: prompt.to_owned(),
            duration: duration.clone(),
            aspect_ratio: aspect.map(str::to_owned),
            resolution: resolution.map(str::to_owned),
            image_url,
        };
        let venice = &self.0.venice;
        let quote = venice.video_quote(&req).await.ok();
        let (queue_id, direct_url) = match venice.video_queue(&req).await {
            Ok(q) => q,
            Err(e) => return ToolOutput::err(format!("video generation failed: {e}")),
        };
        let started = std::time::Instant::now();
        let stem_name = stem(str_arg(&input, "name"), prompt);
        let cap = if image_path.is_some() {
            Capability::ImageToVideo
        } else {
            Capability::Video
        };
        let meta = Meta {
            capability: Some(cap.key().into()),
            provider: "venice".into(),
            model: model_id.to_owned(),
            prompt: prompt.to_owned(),
            params: input.clone(),
            seed: None,
            cost: quote.map(|q| (q, "USD".to_owned(), false)),
            secs: 0,
            inputs: image_path.into_iter().chain(derived_from(&input)).collect(),
            action: str_arg(&input, "action").map(str::to_owned),
        };
        let job = gallery::job_start(
            &ctx.root,
            gallery::Job {
                provider: "venice".into(),
                model: model_id.to_owned(),
                request_id: queue_id.clone(),
                path: direct_url.clone().unwrap_or_default(),
                prompt: prompt.to_owned(),
                stem: stem_name.clone(),
                input: input.clone(),
                capability: meta.capability.clone(),
                inputs: meta.inputs.clone(),
                action: meta.action.clone(),
                cost: meta.cost.clone(),
                ..gallery::Job::default()
            },
        );
        let bytes = match venice_wait(venice, &queue_id, direct_url.as_deref(), &ctx.cancel).await {
            Ok(b) => b,
            Err((e, resumable)) => {
                if !resumable {
                    gallery::job_done(&ctx.root, &job);
                }
                return ToolOutput::err(e);
            }
        };
        let path = match save(&ctx.root, &stem_name, None, "mp4", &bytes) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(format!("could not save the video: {e}")),
        };
        gallery::job_done(&ctx.root, &job);
        // Remove the finished file from Venice's servers; ours is saved.
        let _ = venice.video_complete(&queue_id).await;
        let cost = quote.map_or_else(String::new, |q| format!(", quoted ${q:.2}"));
        let mut out = format!(
            "Generated a {duration} video with {model_id} in {} s{cost}.\nsaved: {}\n",
            started.elapsed().as_secs(),
            shown(&path)
        );
        let meta = Meta {
            secs: started.elapsed().as_secs(),
            ..meta
        };
        finish(
            &self.0.env,
            ctx,
            std::slice::from_ref(&path),
            &meta,
            &input,
            &mut out,
        );
        ToolOutput::ok(out)
    }
}

/// Poll a queued Venice video until its file is ready. `Err((message, resumable))`: after a
/// cancel or the deadline Venice may still finish, so the job stays for `kor media collect`.
async fn venice_wait(
    venice: &VeniceMedia,
    queue_id: &str,
    direct_url: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<u8>, (String, bool)> {
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > VIDEO_DEADLINE {
            return Err((
                format!(
                    "the video was not ready after {} minutes (queue id {queue_id}); it may still finish on Venice (`kor media collect`)",
                    VIDEO_DEADLINE.as_secs() / 60
                ),
                true,
            ));
        }
        tokio::select! {
            () = tokio::time::sleep(POLL_EVERY) => {}
            () = cancel.cancelled() => {
                return Err((
                    format!("cancelled while generating (queue id {queue_id}); Venice may still finish and bill the video"),
                    true,
                ));
            }
        }
        match venice.video_retrieve(queue_id).await {
            Ok(VideoStatus::Processing { .. }) => {}
            Ok(VideoStatus::Done(b)) => return Ok(b),
            Ok(VideoStatus::Url(u)) => {
                return venice
                    .download(&u)
                    .await
                    .map_err(|e| (format!("download failed: {e}"), false));
            }
            Err(e) => {
                // A link-only model may report "not found" on retrieve once its URL is ready.
                if let Some(u) = direct_url
                    && let Ok(b) = venice.download(u).await
                {
                    return Ok(b);
                }
                return Err((format!("video generation failed: {e}"), false));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// fal.ai and Hugging Face: image_generate, image_edit, video_generate, speech_to_text,
// text_to_speech (each with a `provider` field), fal_run and fal_models.
// ---------------------------------------------------------------------------------------------

const FAL_POLL: Duration = Duration::from_secs(2);
const FAL_DEADLINE: Duration = Duration::from_secs(30 * 60);
/// An image larger than this is saved but not attached for the model to look at.
const MAX_VISION_IMAGE: usize = 5 * 1024 * 1024;
const MAX_INPUT_FILE: u64 = 20 * 1024 * 1024;
/// Default speech-to-text models (the docs' examples / fal's `speech-to-text` listing).
pub const HF_ASR_MODEL: &str = "openai/whisper-large-v3";
pub const HF_IMAGE_MODEL: &str = "stabilityai/stable-diffusion-3-medium-diffusers";
pub const FAL_ASR_MODEL: &str = "fal-ai/wizper";

/// Backends of the provider-neutral media tools: fal (own key) and/or Hugging Face (token).
#[derive(Debug, Default)]
pub struct Studio {
    pub fal: Option<Fal>,
    pub hf: Option<Hf>,
    schemas: std::sync::Mutex<std::collections::HashMap<String, Value>>,
    prices: std::sync::Mutex<std::collections::HashMap<String, Option<kor_providers::fal::Price>>>,
    env: MediaEnv,
}

impl Studio {
    /// `None` when neither backend is configured.
    pub fn new(fal: Option<Fal>, hf: Option<Hf>) -> Option<Arc<Self>> {
        Self::with_env(fal, hf, MediaEnv::default())
    }

    pub fn with_env(fal: Option<Fal>, hf: Option<Hf>, env: MediaEnv) -> Option<Arc<Self>> {
        (fal.is_some() || hf.is_some()).then(|| {
            Arc::new(Self {
                fal,
                hf,
                schemas: std::sync::Mutex::default(),
                prices: std::sync::Mutex::default(),
                env,
            })
        })
    }

    pub fn env(&self) -> &MediaEnv {
        &self.env
    }

    /// fal's list price of `model` (cached; `None` without fal's own key).
    async fn price(&self, model: &str) -> Option<kor_providers::fal::Price> {
        if let Some(p) = self.prices.lock().ok().and_then(|m| m.get(model).cloned()) {
            return p;
        }
        let p = self.fal.as_ref()?.price(model).await.ok().flatten();
        if let Ok(mut m) = self.prices.lock() {
            m.insert(model.to_owned(), p.clone());
        }
        p
    }

    /// The catalog view of a fal model (its schema and price), for planning and cost.
    async fn fal_catalog_model(&self, model: &str, cap: Capability) -> Option<CatalogModel> {
        let schema = self.schema(model).await;
        let price = self.price(model).await;
        if schema.is_none() && price.is_none() {
            return None;
        }
        Some(catalog::fal_model(
            model,
            cap,
            schema.as_ref(),
            price.as_ref(),
        ))
    }

    /// Cost of running `model` on `body` from fal's list price.
    async fn estimate(
        &self,
        model: &str,
        cap: Option<Capability>,
        prompt: &str,
        body: &Value,
    ) -> Option<(f64, String, bool)> {
        let cm = self
            .fal_catalog_model(model, cap.unwrap_or(Capability::Image))
            .await?;
        let e = catalog::estimate(&cm, prompt, body.as_object()?)?;
        Some((e.amount, e.currency, e.estimated))
    }

    /// A model's input schema from fal's keyless model API (cached), if fal is configured.
    async fn schema(&self, model: &str) -> Option<Value> {
        if let Some(s) = self.schemas.lock().ok().and_then(|m| m.get(model).cloned()) {
            return Some(s);
        }
        let s = self.fal.as_ref()?.input_schema(model).await.ok()?;
        if let Ok(mut m) = self.schemas.lock() {
            m.insert(model.to_owned(), s.clone());
        }
        Some(s)
    }

    async fn price_note(&self, model: &str) -> String {
        match self.fal.as_ref() {
            Some(f) => match f.price(model).await {
                Ok(Some(p)) => format!(
                    " List price: {} {} per {}.",
                    p.unit_price, p.currency, p.unit
                ),
                _ => String::new(),
            },
            None => String::new(),
        }
    }
}

/// Which tool a [`StudioTool`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    ImageGenerate,
    ImageEdit,
    VideoGenerate,
    SpeechToText,
    TextToSpeech,
    FalRun,
    FalModels,
}

#[derive(Debug)]
pub struct StudioTool {
    studio: Arc<Studio>,
    which: Which,
}

/// The tools a studio offers (`fal_run` / `fal_models` only with a fal key).
pub fn studio_tools(s: &Arc<Studio>) -> Vec<Arc<dyn Tool>> {
    let mut w = vec![
        Which::ImageGenerate,
        Which::ImageEdit,
        Which::VideoGenerate,
        Which::SpeechToText,
        Which::TextToSpeech,
    ];
    if s.fal.is_some() {
        w.extend([Which::FalRun, Which::FalModels]);
    }
    w.into_iter()
        .map(|which| {
            Arc::new(StudioTool {
                studio: s.clone(),
                which,
            }) as Arc<dyn Tool>
        })
        .collect()
}

/// How one call reaches a model.
enum Route<'a> {
    /// fal's own queue (or fal via the HF router) with this endpoint id.
    Queue(Box<std::borrow::Cow<'a, Fal>>, String),
    /// HF Inference provider (or a dedicated endpoint) with this repo id.
    HfInference(&'a Hf, String),
    /// Another provider (Replicate, Together, ...) through the HF router.
    Router(&'a Hf, kor_providers::huggingface::ProviderRoute),
}

fn ext_for(content_type: Option<&str>, url: &str) -> String {
    let by_type = content_type.and_then(|ct| {
        Some(
            match ct.split(';').next()?.trim() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/webp" => "webp",
                "image/gif" => "gif",
                "video/mp4" => "mp4",
                "video/webm" => "webm",
                "audio/mpeg" => "mp3",
                "audio/wav" | "audio/x-wav" => "wav",
                "audio/ogg" => "ogg",
                "audio/flac" => "flac",
                "model/gltf-binary" => "glb",
                _ => return None,
            }
            .to_owned(),
        )
    });
    by_type
        .or_else(|| {
            let last = url.split(['?', '#']).next()?.rsplit('/').next()?;
            let (_, ext) = last.rsplit_once('.')?;
            (ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
                .then(|| ext.to_ascii_lowercase())
        })
        .unwrap_or_else(|| "bin".to_owned())
}

/// Inputs up to this size travel inline as `data:` URLs; larger ones go to fal's storage.
const INLINE_INPUT: u64 = 2 * 1024 * 1024;

/// A project file as a URL for a fal model input: inline when small (or when there is no storage,
/// e.g. fal through the HF router), otherwise uploaded to fal's storage.
async fn fal_input_url(fal: &Fal, path: &Path) -> Result<String, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len <= INLINE_INPUT || !fal.can_upload() {
        return file_data_url(path);
    }
    let mime = input_mime(path)?;
    if len > kor_providers::fal::MAX_UPLOAD_BYTES {
        return Err(format!(
            "the file is larger than {} MB",
            kor_providers::fal::MAX_UPLOAD_BYTES / (1024 * 1024)
        ));
    }
    let bytes = tokio::fs::read(path).await.map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map_or_else(|| "input".to_owned(), |n| n.to_string_lossy().into_owned());
    kor_tools::report_progress(format!(
        "uploading {name} ({} MB) to fal storage",
        len / (1024 * 1024)
    ));
    fal.upload(bytes, mime, &name).await.map_err(|e| e.message)
}

fn input_mime(path: &Path) -> Result<&'static str, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    Ok(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        _ => return Err(format!("unsupported file type `.{ext}`")),
    })
}

/// A project file as a `data:` URL for a model input (fal accepts data URIs for small files).
fn file_data_url(path: &Path) -> Result<String, String> {
    let mime = input_mime(path)?;
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_INPUT_FILE {
        return Err(format!(
            "the file is larger than {} MB",
            MAX_INPUT_FILE / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Text asking the user for what the model needs, or `None` when nothing is missing.
pub fn ask_text(tool: &str, missing: &[MissingInput], price: &str) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    let mut s = format!(
        "Not generated yet. Ask the user in ONE compact question (offer only these options, mark the defaults), \
         then call {tool} again with the answers in `input` (or with `ask: false` to accept the defaults):\n"
    );
    for m in missing {
        let opts = m
            .options
            .iter()
            .map(|o| o.as_str().map_or_else(|| o.to_string(), str::to_owned))
            .collect::<Vec<_>>()
            .join(" | ");
        let default = m
            .default
            .as_ref()
            .map(|d| {
                format!(
                    " (default {})",
                    d.as_str().map_or_else(|| d.to_string(), str::to_owned)
                )
            })
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "- {}: {}{default}{}",
            m.name,
            if opts.is_empty() { "free value" } else { &opts },
            if m.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", m.description)
            }
        );
    }
    s.push_str(price.trim_start());
    Some(s)
}

/// What a fal run left behind.
#[derive(Debug)]
pub struct Saved {
    pub paths: Vec<PathBuf>,
    pub result: Value,
    pub secs: u64,
    /// The first image, for vision models.
    pub first_image: Option<kor_core::ImageSource>,
}

impl Saved {
    /// Lines for the model / terminal: summary, text output, `saved:` lines.
    pub fn report(&self, model: &str) -> String {
        let mut out = format!(
            "{model} finished in {} s with {} file(s).\n",
            self.secs,
            self.paths.len()
        );
        if let Some(t) = self.result.get("text").and_then(Value::as_str) {
            let _ = writeln!(out, "text: {t}");
        }
        for p in &self.paths {
            let _ = writeln!(out, "saved: {}", shown(p));
        }
        if self.paths.is_empty() {
            let s: String = self.result.to_string().chars().take(4000).collect();
            let _ = writeln!(out, "result: {s}");
        }
        out
    }
}

/// Forwards a line to [`kor_tools::report_progress`] when it differs from the previous one (a
/// poll every 2 s mostly repeats the same status).
fn progress_reporter() -> impl Fn(String) + Send + Sync {
    let last = std::sync::Mutex::new(String::new());
    move |line: String| {
        let mut l = last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *l != line {
            kor_tools::report_progress(line.clone());
            *l = line;
        }
    }
}

/// Download an output file, reporting progress in 10% steps (or every 5 MB without a length).
async fn download_reporting(url: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let on = progress_reporter();
    let mb = |b: u64| b / (1024 * 1024);
    kor_providers::fal::download_with_progress(
        url,
        kor_providers::fal::MAX_OUTPUT_BYTES,
        &|done, total| {
            on(match total {
                Some(t) if t > 0 => format!(
                    "downloading output: {}% of {} MB",
                    done * 100 / t / 10 * 10,
                    mb(t)
                ),
                _ => format!("downloading output: {} MB", mb(done) / 5 * 5),
            });
        },
    )
    .await
    .map_err(|e| format!("download failed: {}", e.message))
}

/// Where a fal run is saved and what the gallery keeps about it.
#[derive(Debug, Clone)]
pub struct FalSave<'a> {
    pub root: &'a Path,
    pub stem: &'a str,
    /// Gallery data; the model, seed and time are filled in.
    pub meta: Meta,
    /// Usage ledger for the media cost line.
    pub ledger: Option<&'a Path>,
}

/// Run `model` on fal's queue (reporting each status) and save every output file under
/// `<root>/kor-media/` with its gallery sidecar. While it runs the request is kept in
/// `kor-media/.jobs/`, so a process that ends early leaves it for [`collect`]. Shared by the
/// tools, `kor media` and the desktop.
pub async fn fal_run_save(
    fal: &Fal,
    model: &str,
    input: &Value,
    save_to: &FalSave<'_>,
    cancel: &tokio_util::sync::CancellationToken,
    on_status: &(dyn Fn(&kor_providers::fal::QueueStatus) + Send + Sync),
) -> Result<Saved, String> {
    let started = std::time::Instant::now();
    let sub = fal
        .submit(model, input)
        .await
        .map_err(|e| format!("{model} failed: {}", e.message))?;
    let m = &save_to.meta;
    let job = gallery::job_start(
        save_to.root,
        gallery::Job {
            provider: m.provider.clone(),
            model: model.to_owned(),
            request_id: sub.request_id.clone(),
            path: sub.path.clone(),
            prompt: m.prompt.clone(),
            stem: save_to.stem.to_owned(),
            input: input.clone(),
            capability: m.capability.clone(),
            inputs: m.inputs.clone(),
            action: m.action.clone(),
            cost: m.cost.clone(),
            ..gallery::Job::default()
        },
    );
    let result = fal
        .wait(&sub, FAL_POLL, FAL_DEADLINE, cancel, on_status)
        .await;
    // A cancel or the deadline cancels the request on fal's side too: nothing left to collect.
    gallery::job_done(save_to.root, &job);
    let result = result.map_err(|e| format!("{model} failed: {}", e.message))?;
    save_outputs(model, input, result, save_to, started).await
}

/// Download and save every output file of a finished fal request, then record it.
async fn save_outputs(
    model: &str,
    input: &Value,
    result: Value,
    save_to: &FalSave<'_>,
    started: std::time::Instant,
) -> Result<Saved, String> {
    let (root, stem_name) = (save_to.root, save_to.stem);
    let files = output_files(&result);
    let mut saved = Saved {
        paths: Vec::new(),
        result,
        secs: 0,
        first_image: None,
    };
    for (i, f) in files.iter().enumerate() {
        let (bytes, ct) = download_reporting(&f.url).await?;
        let ct = f.content_type.clone().or(ct);
        let ext = ext_for(ct.as_deref(), &f.url);
        let p = save(
            root,
            stem_name,
            (files.len() > 1).then_some(i),
            &ext,
            &bytes,
        )
        .map_err(|e| format!("could not save the output: {e}"))?;
        saved.paths.push(p);
        if saved.first_image.is_none()
            && bytes.len() <= MAX_VISION_IMAGE
            && let Some(mt) = ct.filter(|c| c.starts_with("image/"))
        {
            saved.first_image = Some(kor_core::ImageSource::Base64 {
                media_type: mt,
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            });
        }
    }
    saved.secs = started.elapsed().as_secs();
    let meta = Meta {
        model: model.to_owned(),
        seed: gallery::seed_of(&saved.result, input),
        secs: saved.secs,
        ..save_to.meta.clone()
    };
    gallery::record(root, &saved.paths, &meta);
    gallery::charge(
        save_to.ledger,
        &format!("{}/{model}", meta.provider),
        meta.cost.as_ref(),
    );
    Ok(saved)
}

/// Clients [`collect`] may need.
#[derive(Debug, Clone, Copy, Default)]
pub struct Backends<'a> {
    /// fal's own client, or fal through Hugging Face for a job with provider `huggingface`.
    pub fal: Option<&'a Fal>,
    pub venice: Option<&'a VeniceMedia>,
}

/// Finish a request an earlier process queued (`kor media collect`, the desktop's jobs list):
/// wait for it, save its outputs into the gallery, charge the ledger.
///
/// # Errors
/// The provider is not configured, the request failed, or the files could not be saved.
pub async fn collect(
    job: &gallery::Job,
    b: Backends<'_>,
    root: &Path,
    ledger: Option<&Path>,
    cancel: &tokio_util::sync::CancellationToken,
    on_status: &(dyn Fn(&str) + Send + Sync),
) -> Result<Saved, String> {
    let started = std::time::Instant::now();
    let meta = Meta {
        capability: job.capability.clone(),
        provider: job.provider.clone(),
        model: job.model.clone(),
        prompt: job.prompt.clone(),
        params: job.input.clone(),
        seed: None,
        cost: job.cost.clone(),
        secs: 0,
        inputs: job.inputs.clone(),
        action: job.action.clone(),
    };
    if job.provider == "venice" {
        let v = b.venice.ok_or("Venice is not configured")?;
        on_status(&format!(
            "waiting for {} (queue id {})",
            job.model, job.request_id
        ));
        let direct = Some(job.path.as_str()).filter(|p| !p.is_empty());
        let bytes = match venice_wait(v, &job.request_id, direct, cancel).await {
            Ok(b) => b,
            Err((e, resumable)) => {
                if !resumable {
                    gallery::job_done(root, &job.id);
                }
                return Err(e);
            }
        };
        let path = save(root, &job.stem, None, "mp4", &bytes)?;
        gallery::job_done(root, &job.id);
        let _ = v.video_complete(&job.request_id).await;
        let secs = started.elapsed().as_secs();
        let meta = Meta { secs, ..meta };
        gallery::record(root, std::slice::from_ref(&path), &meta);
        gallery::charge(ledger, &format!("venice/{}", job.model), meta.cost.as_ref());
        return Ok(Saved {
            paths: vec![path],
            result: json!({}),
            secs,
            first_image: None,
        });
    }
    let f = b.fal.ok_or("fal is not configured")?;
    let sub = kor_providers::fal::Submitted {
        request_id: job.request_id.clone(),
        path: job.path.clone(),
        queue_position: None,
    };
    let on = |s: &kor_providers::fal::QueueStatus| on_status(&kor_providers::fal::status_line(s));
    let result = f.wait(&sub, FAL_POLL, FAL_DEADLINE, cancel, &on).await;
    gallery::job_done(root, &job.id);
    let result = result.map_err(|e| format!("{} failed: {}", job.model, e.message))?;
    let save_to = FalSave {
        root,
        stem: &job.stem,
        meta,
        ledger,
    };
    save_outputs(&job.model, &job.input, result, &save_to, started).await
}

/// A generation chosen from the catalog (`kor media generate`, the desktop media panel).
#[derive(Debug, Clone, Default)]
pub struct CatalogRun {
    pub capability: Option<Capability>,
    pub prompt: String,
    /// Planned inputs (`media_catalog::plan` plus the answers).
    pub params: Map<String, Value>,
    /// Gallery id or project path of the reference (edit, upscale, image-to-video, transcription).
    pub reference: Option<String>,
    pub action: Option<String>,
    pub name: Option<String>,
    pub save_as: Option<String>,
}

/// The tool that runs `run` on `model`, and its input.
///
/// # Errors
/// A reference for a model that takes none, or a capability the provider route cannot run.
pub fn catalog_call(
    model: &CatalogModel,
    run: &CatalogRun,
) -> Result<(&'static str, Value), String> {
    let cap = run
        .capability
        .or_else(|| model.capabilities.first().copied())
        .ok_or("the model has no capability")?;
    let params = Value::Object(run.params.clone());
    let (tool, mut input) = match model.provider.as_str() {
        "venice" => {
            let mut i = params;
            i["prompt"] = json!(run.prompt);
            i["model"] = json!(model.id);
            if let Some(r) = &run.reference {
                i["image"] = json!(r);
            }
            let tool = if cap == Capability::Image {
                "generate_image"
            } else {
                "generate_video"
            };
            (tool, i)
        }
        "fal" => {
            let mut body = params;
            if !run.prompt.is_empty() {
                let key = model
                    .by_role(catalog::Role::Prompt)
                    .map_or("prompt", |p| p.name.as_str());
                body[key] = json!(run.prompt);
            }
            if let Some(r) = &run.reference {
                let p = model
                    .by_role(catalog::Role::Reference)
                    .ok_or_else(|| format!("{} takes no reference file", model.id))?;
                body[p.name.as_str()] =
                    if p.name.ends_with("_urls") || p.kind == catalog::ParamKind::Array {
                        json!([r])
                    } else {
                        json!(r)
                    };
            }
            ("fal_run", json!({ "model": model.id, "input": body }))
        }
        "huggingface" => {
            let tool = match cap {
                Capability::Image => "image_generate",
                Capability::Edit => "image_edit",
                Capability::Video => "video_generate",
                Capability::Tts => "text_to_speech",
                Capability::Asr => "speech_to_text",
                other => {
                    return Err(format!(
                        "{} is not available through Hugging Face",
                        other.key()
                    ));
                }
            };
            let mut i = json!({ "provider": "huggingface", "model": model.id, "input": params });
            if !run.prompt.is_empty() {
                i["prompt"] = json!(run.prompt);
            }
            if let Some(r) = &run.reference {
                i[if cap == Capability::Asr {
                    "audio"
                } else {
                    "image"
                }] = json!(r);
            }
            (tool, i)
        }
        other => return Err(format!("unknown provider `{other}`")),
    };
    input["ask"] = json!(false);
    input["capability"] = json!(cap.key());
    for (k, v) in [
        ("name", &run.name),
        ("action", &run.action),
        ("save_as", &run.save_as),
    ] {
        if let Some(v) = v {
            input[k] = json!(v);
        }
    }
    Ok((tool, input))
}

/// Run a catalog choice with the session's media tools (already planned and asked: the tool
/// itself asks nothing more).
pub async fn run_catalog(
    tools: &[Arc<dyn Tool>],
    model: &CatalogModel,
    run: &CatalogRun,
    ctx: &ToolCtx,
) -> ToolOutput {
    let (name, input) = match catalog_call(model, run) {
        Ok(x) => x,
        Err(e) => return ToolOutput::err(e),
    };
    match tools.iter().find(|t| t.name() == name) {
        Some(t) => t.call(input, ctx).await,
        None => ToolOutput::err(format!(
            "{name} is not available: {} is not configured",
            model.provider
        )),
    }
}

/// `media_gallery`: the project's generated media and where each came from (§20.3).
#[derive(Debug)]
pub struct MediaGallery;

/// The gallery tool (one per session, whichever media providers are configured).
pub fn gallery_tool() -> Arc<dyn Tool> {
    Arc::new(MediaGallery)
}

#[async_trait]
impl Tool for MediaGallery {
    fn name(&self) -> &'static str {
        "media_gallery"
    }
    fn rule_name(&self) -> &'static str {
        "media_gallery"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_owned(),
            description: format!(
                "List the images, videos and audio generated in this project ({MEDIA_DIR}/, newest first) with \
                 prompt, model, seed, cost and the item each was derived from; with `id`, one item and its whole \
                 derivation chain. Pass an id as `image`/`audio` of a media tool to use it as a reference \
                 (image-to-image, upscale, image-to-video), with `action` naming the step."
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "A gallery id (file name in kor-media/)"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                },
            }),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        str_arg(input, "id").map(str::to_owned)
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let line = |i: &gallery::Item| {
            let cost = i.cost.map_or_else(String::new, |c| {
                format!(" {}${c:.3}", if i.estimated { "≈" } else { "" })
            });
            let from = i
                .parent
                .as_deref()
                .map_or_else(String::new, |p| format!(" ← {p}"));
            let action = i
                .action
                .as_deref()
                .map_or_else(String::new, |a| format!(" [{a}]"));
            let seed = i.seed.map_or_else(String::new, |s| format!(" seed {s}"));
            let prompt: String = i.prompt.chars().take(80).collect();
            format!(
                "- {} ({}, {} {}{seed}{cost}){action}{from}: {prompt}",
                i.id, i.kind, i.provider, i.model
            )
        };
        if let Some(id) = str_arg(&input, "id") {
            let c = gallery::chain(&ctx.root, id);
            if c.is_empty() {
                return ToolOutput::err(format!("no gallery item `{id}`"));
            }
            let mut out = format!("{id} and what it was derived from (newest first):\n");
            for i in &c {
                let _ = writeln!(out, "{}", line(i));
            }
            return ToolOutput::ok(out);
        }
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(20);
        let items = gallery::list(&ctx.root);
        if items.is_empty() {
            return ToolOutput::ok(format!("The gallery ({MEDIA_DIR}/) is empty."));
        }
        let mut out = String::new();
        for i in items.iter().take(limit) {
            let _ = writeln!(out, "{}", line(i));
        }
        ToolOutput::ok(out)
    }
}

/// One HF task for `kor hf infer` and the desktop: `text-to-image` (input = prompt, saved under
/// `<root>/kor-media/`), `speech-to-text` / `image-classification` / `object-detection` (input =
/// a file path), `feature-extraction` (input = text). Returns printable text.
pub async fn hf_infer(
    hf: &Hf,
    model: &str,
    task: &str,
    input: &str,
    root: &Path,
) -> Result<String, String> {
    use kor_providers::huggingface::Task;
    let file = || -> Result<(Vec<u8>, String), String> {
        let p = Path::new(input);
        let url = file_data_url(p)?;
        let (mime, b64) = url
            .trim_start_matches("data:")
            .split_once(";base64,")
            .ok_or("bad file")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| e.to_string())?;
        Ok((bytes, mime.to_owned()))
    };
    let e = |e: kor_core::ProviderError| e.message;
    match task {
        "text-to-image" => {
            let (bytes, ct) = hf.text_to_image(model, input, json!({})).await.map_err(e)?;
            let p = save(
                root,
                &stem(None, input),
                None,
                &ext_for(ct.as_deref(), ""),
                &bytes,
            )?;
            gallery::record(
                root,
                std::slice::from_ref(&p),
                &Meta {
                    capability: Some(Capability::Image.key().into()),
                    provider: "huggingface".into(),
                    model: model.to_owned(),
                    prompt: input.to_owned(),
                    params: json!({}),
                    ..Meta::default()
                },
            );
            Ok(format!("saved: {}", shown(&p)))
        }
        "speech-to-text" => {
            let (b, mime) = file()?;
            hf.speech_to_text(model, b, &mime).await.map_err(e)
        }
        "image-classification" | "object-detection" => {
            let (b, mime) = file()?;
            let t = if task == "object-detection" {
                Task::ObjectDetection
            } else {
                Task::ImageClassification
            };
            let v = hf.vision(model, t, b, &mime).await.map_err(e)?;
            Ok(serde_json::to_string_pretty(&v).unwrap_or_default())
        }
        "feature-extraction" => {
            let v = hf.embed(model, &[input.to_owned()]).await.map_err(e)?;
            let dim = v.first().map_or(0, Vec::len);
            let head: Vec<String> = v
                .first()
                .into_iter()
                .flatten()
                .take(8)
                .map(|x| format!("{x:.4}"))
                .collect();
            Ok(format!("{dim} dimensions: [{}, ...]", head.join(", ")))
        }
        other => Err(format!(
            "unknown task `{other}`; use text-to-image, speech-to-text, feature-extraction, image-classification or object-detection"
        )),
    }
}

impl StudioTool {
    fn provider<'a>(&self, input: &'a Value) -> Result<&'a str, String> {
        let p = str_arg(input, "provider").unwrap_or(if self.studio.fal.is_some() {
            "fal"
        } else {
            "huggingface"
        });
        match p {
            "fal" if self.studio.fal.is_none() => {
                Err("fal is not configured (set FAL_KEY or run `kor auth login fal`)".into())
            }
            "huggingface" if self.studio.hf.is_none() => Err(
                "Hugging Face is not configured (set HF_TOKEN or run `kor auth login huggingface`)"
                    .into(),
            ),
            "fal" | "huggingface" => Ok(p),
            other => Err(format!(
                "unknown provider `{other}`; use fal or huggingface"
            )),
        }
    }

    /// fal endpoint id, or for Hugging Face the repo's hf-inference / fal-ai route.
    async fn route(
        &self,
        provider: &str,
        model: &str,
        hf_inference: bool,
    ) -> Result<Route<'_>, String> {
        if provider == "fal" {
            let fal = self.studio.fal.as_ref().ok_or("fal is not configured")?;
            return Ok(Route::Queue(
                Box::new(std::borrow::Cow::Borrowed(fal)),
                model.to_owned(),
            ));
        }
        let hf = self
            .studio
            .hf
            .as_ref()
            .ok_or("Hugging Face is not configured")?;
        let routes = match hf.provider_mapping(model).await {
            Ok(r) => r,
            Err(_) if hf_inference => return Ok(Route::HfInference(hf, model.to_owned())),
            Err(e) => return Err(e.to_string()),
        };
        if hf_inference
            && routes
                .iter()
                .any(|r| r.provider == "hf-inference" && r.live)
        {
            return Ok(Route::HfInference(hf, model.to_owned()));
        }
        if let Some(r) = routes.iter().find(|r| r.provider == "fal-ai" && r.live) {
            let fal = hf.fal().map_err(|e| e.to_string())?;
            return Ok(Route::Queue(
                Box::new(std::borrow::Cow::Owned(fal)),
                r.provider_id.clone(),
            ));
        }
        let task = self.hub_task();
        if let Some(r) = routes.iter().find(|r| {
            r.live
                && r.task == task
                && kor_providers::huggingface::router_supports(&r.provider, task)
        }) {
            return Ok(Route::Router(hf, r.clone()));
        }
        let served: Vec<&str> = routes.iter().map(|r| r.provider.as_str()).collect();
        Err(format!(
            "`{model}` has no {task} route Kor can drive on Hugging Face (providers: {}); \
             pick another model (`kor hf search --pipeline <task>`)",
            if served.is_empty() {
                "none".to_owned()
            } else {
                served.join(", ")
            }
        ))
    }

    /// The Hub task name (`inferenceProviderMapping[].task`) this tool runs.
    fn hub_task(&self) -> &'static str {
        match self.which {
            Which::ImageEdit => "image-to-image",
            Which::VideoGenerate => "text-to-video",
            Which::SpeechToText => "automatic-speech-recognition",
            Which::TextToSpeech => "text-to-speech",
            _ => "text-to-image",
        }
    }

    /// Run on another provider through the HF router, save its output, attach the first image.
    #[allow(clippy::too_many_arguments)] // same inputs as `hf_inference`
    async fn run_router(
        &self,
        hf: &Hf,
        route: &kor_providers::huggingface::ProviderRoute,
        prompt: &str,
        file: Option<&Path>,
        input: &Value,
        ctx: &ToolCtx,
        stem_name: &str,
    ) -> ToolOutput {
        use kor_providers::huggingface::Routed;
        let file_path = file;
        let file = match file.map(file_data_url).transpose() {
            Ok(f) => f,
            Err(e) => return ToolOutput::err(e),
        };
        let started = std::time::Instant::now();
        let on = progress_reporter();
        let model = format!(
            "{} via Hugging Face ({})",
            route.provider_id, route.provider
        );
        let out = hf
            .run_routed(
                route,
                prompt,
                file.as_deref(),
                &Self::extra(input),
                &ctx.cancel,
                &|s| on(s.to_owned()),
            )
            .await;
        let (bytes, ct, url) = match out {
            Ok(Routed::Text(t)) => return ToolOutput::ok(format!("Transcript ({model}):\n{t}")),
            Ok(Routed::Bytes(b, ct)) => (b, Some(ct), String::new()),
            Ok(Routed::Urls(urls)) => {
                let Some(u) = urls.into_iter().next() else {
                    return ToolOutput::err(format!("{model}: no output"));
                };
                match download_reporting(&u).await {
                    Ok((b, ct)) => (b, ct, u),
                    Err(e) => return ToolOutput::err(e),
                }
            }
            Err(e) => return ToolOutput::err(format!("{model} failed: {}", e.message)),
        };
        let path = match save(
            &ctx.root,
            stem_name,
            None,
            &ext_for(ct.as_deref(), &url),
            &bytes,
        ) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(format!("could not save the output: {e}")),
        };
        let mut text = format!(
            "{model} finished in {} s with 1 file(s).\nsaved: {}\n",
            started.elapsed().as_secs(),
            shown(&path)
        );
        let repo = str_arg(input, "model").unwrap_or(&route.provider_id);
        let mut meta = self.meta(input, repo, "huggingface", prompt, file_path);
        meta.secs = started.elapsed().as_secs();
        finish(
            &self.studio.env,
            ctx,
            std::slice::from_ref(&path),
            &meta,
            input,
            &mut text,
        );
        let mut o = ToolOutput::ok(text);
        if bytes.len() <= MAX_VISION_IMAGE
            && let Some(mt) = ct.filter(|c| c.starts_with("image/"))
        {
            o.image = Some(kor_core::ImageSource::Base64 {
                media_type: mt,
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            });
        }
        o
    }

    /// Run on fal's queue, save every output file, attach the first image.
    #[allow(clippy::too_many_arguments)] // the call, its tool input and the gallery data
    async fn run_queue(
        &self,
        fal: &Fal,
        model: &str,
        input: &Value,
        ctx: &ToolCtx,
        stem_name: &str,
        tool_input: &Value,
        mut meta: Meta,
    ) -> ToolOutput {
        let on = progress_reporter();
        let on_status =
            |s: &kor_providers::fal::QueueStatus| on(kor_providers::fal::status_line(s));
        let cap = meta.capability.as_deref().and_then(Capability::parse);
        meta.cost = self.studio.estimate(model, cap, &meta.prompt, input).await;
        meta.params = input.clone();
        let save = FalSave {
            root: &ctx.root,
            stem: stem_name,
            meta,
            ledger: self.studio.env.ledger.as_deref(),
        };
        match fal_run_save(fal, model, input, &save, &ctx.cancel, &on_status).await {
            Ok(s) => {
                let mut text = s.report(model);
                copy_to(ctx, &s.paths, tool_input, &mut text);
                let mut o = ToolOutput::ok(text);
                o.image = s.first_image;
                o
            }
            Err(e) => ToolOutput::err(e),
        }
    }

    /// Gallery data of a call (cost and time are filled in later).
    fn meta(
        &self,
        input: &Value,
        model: &str,
        provider: &str,
        prompt: &str,
        file: Option<&Path>,
    ) -> Meta {
        Meta {
            capability: self
                .capability(input, model, file.is_some())
                .map(|c| c.key().to_owned()),
            provider: provider.to_owned(),
            model: model.to_owned(),
            prompt: prompt.to_owned(),
            params: Self::extra(input),
            seed: input.pointer("/input/seed").and_then(Value::as_i64),
            cost: None,
            secs: 0,
            inputs: file
                .map(Path::to_path_buf)
                .into_iter()
                .chain(derived_from(input))
                .collect(),
            action: str_arg(input, "action").map(str::to_owned),
        }
    }

    /// The catalog capability this call serves.
    fn capability(&self, input: &Value, model: &str, has_file: bool) -> Option<Capability> {
        if let Some(c) = str_arg(input, "capability").and_then(Capability::parse) {
            return Some(c);
        }
        Some(match self.which {
            Which::ImageGenerate => Capability::Image,
            Which::ImageEdit if model == kor_providers::fal::BACKGROUND_REMOVAL_MODEL => {
                Capability::RemoveBg
            }
            Which::ImageEdit => Capability::Edit,
            Which::VideoGenerate if has_file => Capability::ImageToVideo,
            Which::VideoGenerate => Capability::Video,
            Which::SpeechToText => Capability::Asr,
            Which::TextToSpeech => Capability::Tts,
            Which::FalRun | Which::FalModels => return None,
        })
    }

    /// §20.2 for fal models: their schema says what can be chosen; Hub models publish none.
    async fn card(&self, input: &Value, ctx: &ToolCtx) -> Option<ParamCard> {
        if matches!(self.which, Which::FalModels | Which::SpeechToText) {
            return None;
        }
        if self.which != Which::FalRun && self.provider(input).ok()? != "fal" {
            return None;
        }
        let model = str_arg(input, "model").or(match self.which {
            Which::ImageGenerate => Some(kor_providers::fal::DEFAULT_IMAGE_MODEL),
            Which::ImageEdit => Some(kor_providers::fal::DEFAULT_EDIT_MODEL),
            _ => None,
        })?;
        let cap = self.capability(input, model, input.get("image").is_some());
        let cm = self
            .studio
            .fal_catalog_model(model, cap.unwrap_or(Capability::Image))
            .await?;
        let prompt = str_arg(input, "prompt")
            .or_else(|| input.pointer("/input/prompt").and_then(Value::as_str))
            .unwrap_or_default();
        let given = input
            .get("input")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let ask = self.which != Which::FalRun
            && input.get("ask").and_then(Value::as_bool) != Some(false)
            && !self.studio.env.no_ask;
        let plan = catalog::plan(&cm, prompt, &given, &self.studio.env.defaults_for(cap), ask);
        let title: String = format!("{model}: {prompt}").chars().take(100).collect();
        card_from_plan(
            title,
            &cm,
            &plan,
            &self.studio.env,
            &ctx.root,
            Some("input"),
        )
    }

    /// Optional choices worth asking about for this tool (§20.2), when the model offers them.
    fn optional_choices(&self) -> &'static [&'static str] {
        match self.which {
            Which::VideoGenerate => &["duration", "resolution", "aspect_ratio", "generate_audio"],
            // Images are cheap and quick: only required inputs are asked about.
            _ => &[],
        }
    }

    async fn questions(&self, model: &str, input: &Value) -> Option<String> {
        let schema = self.studio.schema(model).await?;
        let body = input.get("input").cloned().unwrap_or_else(|| json!({}));
        let mut missing = missing_inputs(&schema, &body);
        if input.get("ask").and_then(Value::as_bool) != Some(false) {
            missing.extend(
                describe(&schema, self.optional_choices())
                    .into_iter()
                    .filter(|m| !m.options.is_empty() || m.name == "generate_audio")
                    .filter(|m| input.get("input").and_then(|i| i.get(&m.name)).is_none()),
            );
        }
        missing.retain(|m| input.get("input").and_then(|i| i.get(&m.name)).is_none());
        let price = self.studio.price_note(model).await;
        ask_text(self.name(), &missing, &price)
    }

    /// Models of a fal category, for the user to choose from when none was named.
    async fn choose_model(&self, category: &str) -> String {
        let mut s = format!(
            "No model given. Ask the user which {category} model to use (cost and quality differ), then call again with `model`."
        );
        if let Some(fal) = &self.studio.fal
            && let Ok(models) = fal.models(None, Some(category), 8).await
        {
            s.push_str(" Options on fal:\n");
            for m in models {
                let _ = writeln!(s, "- {} ({})", m.endpoint_id, m.name);
            }
        } else {
            s.push_str(" For Hugging Face, name a repo id (see `kor hf search --pipeline ");
            s.push_str(category);
            s.push_str("`).");
        }
        s
    }

    fn extra(input: &Value) -> Value {
        input
            .get("input")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}))
    }

    #[allow(clippy::too_many_lines)] // validate, pick a route, build the input, run: one flow
    async fn generate(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let provider = match self.provider(&input) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let prompt = str_arg(&input, "prompt").or_else(|| str_arg(&input, "text"));
        let needs_prompt = matches!(
            self.which,
            Which::ImageGenerate | Which::ImageEdit | Which::VideoGenerate | Which::TextToSpeech
        );
        if needs_prompt && prompt.is_none() {
            return ToolOutput::err("`prompt` is required");
        }
        let prompt = prompt.unwrap_or_default();
        let file = match str_arg(
            &input,
            if self.which == Which::SpeechToText {
                "audio"
            } else {
                "image"
            },
        ) {
            Some(p) => match resolve_input(ctx, p) {
                Ok(p) => Some(p),
                Err(e) => return ToolOutput::err(e),
            },
            None => None,
        };
        let (default_model, category) = match (self.which, provider) {
            (Which::ImageGenerate, "fal") => (
                Some(kor_providers::fal::DEFAULT_IMAGE_MODEL),
                "text-to-image",
            ),
            (Which::ImageGenerate, _) => (Some(HF_IMAGE_MODEL), "text-to-image"),
            (Which::ImageEdit, "fal") => (
                Some(kor_providers::fal::DEFAULT_EDIT_MODEL),
                "image-to-image",
            ),
            (Which::ImageEdit, _) => (None, "image-to-image"),
            (Which::VideoGenerate, _) if file.is_some() => (None, "image-to-video"),
            (Which::VideoGenerate, _) => (None, "text-to-video"),
            (Which::SpeechToText, "fal") => (Some(FAL_ASR_MODEL), "speech-to-text"),
            (Which::SpeechToText, _) => (Some(HF_ASR_MODEL), "speech-to-text"),
            _ => (None, "text-to-speech"),
        };
        let Some(model) = str_arg(&input, "model").or(default_model) else {
            return ToolOutput::ok(self.choose_model(category).await);
        };
        if self.which == Which::ImageEdit && file.is_none() {
            return ToolOutput::err("`image` (a project path) is required");
        }
        if self.which == Which::SpeechToText && file.is_none() {
            return ToolOutput::err("`audio` (a project path) is required");
        }
        let hf_inference = matches!(self.which, Which::ImageGenerate | Which::SpeechToText);
        let route = match self.route(provider, model, hf_inference).await {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(e),
        };
        let stem_name = stem(
            str_arg(&input, "name"),
            if prompt.is_empty() {
                "transcript"
            } else {
                prompt
            },
        );
        match route {
            Route::HfInference(hf, repo) => {
                self.hf_inference(hf, &repo, prompt, file.as_deref(), &input, ctx, &stem_name)
                    .await
            }
            Route::Router(hf, r) => {
                self.run_router(hf, &r, prompt, file.as_deref(), &input, ctx, &stem_name)
                    .await
            }
            Route::Queue(fal, endpoint) => {
                let schema = self.studio.schema(&endpoint).await;
                let has = |k: &str| {
                    schema
                        .as_ref()
                        .and_then(|s| s.pointer(&format!("/properties/{k}")))
                        .is_some()
                };
                let mut body = Self::extra(&input);
                if let Some(o) = body.as_object_mut() {
                    if !prompt.is_empty() {
                        let key = if self.which == Which::TextToSpeech
                            && (has("text") || schema.is_none())
                            && !has("prompt")
                        {
                            "text"
                        } else {
                            "prompt"
                        };
                        o.entry(key).or_insert_with(|| json!(prompt));
                    }
                    if let Some(f) = &file {
                        let url = match fal_input_url(&fal, f).await {
                            Ok(u) => u,
                            Err(e) => return ToolOutput::err(format!("{}: {e}", shown(f))),
                        };
                        let (key, many) = match self.which {
                            Which::SpeechToText => ("audio_url", false),
                            _ if has("image_urls") => ("image_urls", true),
                            _ => ("image_url", false),
                        };
                        o.entry(key)
                            .or_insert_with(|| if many { json!([url]) } else { json!(url) });
                    }
                }
                let check = json!({ "input": body, "ask": input.get("ask") });
                if let Some(q) = self.questions(&endpoint, &check).await {
                    return ToolOutput::ok(q);
                }
                let meta = self.meta(&input, &endpoint, provider, prompt, file.as_deref());
                self.run_queue(&fal, &endpoint, &body, ctx, &stem_name, &input, meta)
                    .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn hf_inference(
        &self,
        hf: &Hf,
        repo: &str,
        prompt: &str,
        file: Option<&Path>,
        input: &Value,
        ctx: &ToolCtx,
        stem_name: &str,
    ) -> ToolOutput {
        if self.which == Which::SpeechToText {
            {
                let Some(f) = file else {
                    return ToolOutput::err("`audio` is required");
                };
                let url = match file_data_url(f) {
                    Ok(u) => u,
                    Err(e) => return ToolOutput::err(e),
                };
                let (mime, b64) = url
                    .trim_start_matches("data:")
                    .split_once(";base64,")
                    .unwrap_or_default();
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default();
                return match hf.speech_to_text(repo, bytes, mime).await {
                    Ok(t) => ToolOutput::ok(format!("Transcript ({repo}):\n{t}")),
                    Err(e) => ToolOutput::err(e.message),
                };
            }
        }
        {
            {
                let r = tokio::select! {
                    r = hf.text_to_image(repo, prompt, Self::extra(input)) => r,
                    () = ctx.cancel.cancelled() => return ToolOutput::err("cancelled"),
                };
                let (bytes, ct) = match r {
                    Ok(x) => x,
                    Err(e) => return ToolOutput::err(format!("{repo} failed: {}", e.message)),
                };
                let ext = ext_for(ct.as_deref(), "");
                let path = match save(&ctx.root, stem_name, None, &ext, &bytes) {
                    Ok(p) => p,
                    Err(e) => return ToolOutput::err(format!("could not save the image: {e}")),
                };
                let mut text = format!(
                    "Generated an image with {repo} (Hugging Face).\nsaved: {}\n",
                    shown(&path)
                );
                let meta = self.meta(input, repo, "huggingface", prompt, None);
                finish(
                    &self.studio.env,
                    ctx,
                    std::slice::from_ref(&path),
                    &meta,
                    input,
                    &mut text,
                );
                let mut o = ToolOutput::ok(text);
                if bytes.len() <= MAX_VISION_IMAGE {
                    o.image = Some(kor_core::ImageSource::Base64 {
                        media_type: ct.unwrap_or_else(|| "image/jpeg".into()),
                        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                    });
                }
                o
            }
        }
    }

    async fn fal_run(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(fal) = &self.studio.fal else {
            return ToolOutput::err("fal is not configured");
        };
        let Some(model) = str_arg(&input, "model") else {
            return ToolOutput::err("`model` is required (see fal_models)");
        };
        let mut body = Self::extra(&input);
        // File inputs (`*_url`, `*_urls`) may name a project path or a gallery id (§20.3).
        let mut files: Vec<(String, Option<usize>, PathBuf)> = Vec::new();
        for (k, v) in body.as_object().into_iter().flatten() {
            if !(k.ends_with("_url") || k.ends_with("_urls")) {
                continue;
            }
            let items: Vec<(Option<usize>, &str)> = match v {
                Value::String(s) => vec![(None, s.as_str())],
                Value::Array(a) => a
                    .iter()
                    .enumerate()
                    .filter_map(|(i, x)| x.as_str().map(|s| (Some(i), s)))
                    .collect(),
                _ => Vec::new(),
            };
            for (i, s) in items {
                if ["https://", "http://", "data:"]
                    .iter()
                    .any(|p| s.starts_with(p))
                {
                    continue;
                }
                match resolve_input(ctx, s) {
                    Ok(p) => files.push((k.clone(), i, p)),
                    Err(e) => return ToolOutput::err(format!("{k}: {e}")),
                }
            }
        }
        for (k, i, p) in &files {
            let url = match fal_input_url(fal, p).await {
                Ok(u) => u,
                Err(e) => return ToolOutput::err(format!("{}: {e}", shown(p))),
            };
            match i {
                Some(i) => body[k.as_str()][*i] = json!(url),
                None => body[k.as_str()] = json!(url),
            }
        }
        let check = json!({ "input": body, "ask": false });
        if let Some(q) = self.questions(model, &check).await {
            return ToolOutput::ok(q);
        }
        let prompt = body
            .get("prompt")
            .or_else(|| body.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut meta = self.meta(&input, model, "fal", prompt, None);
        meta.inputs = files.into_iter().map(|(_, _, p)| p).collect();
        meta.inputs.extend(derived_from(&input));
        self.run_queue(
            fal,
            model,
            &body,
            ctx,
            &stem(
                str_arg(&input, "name"),
                if prompt.is_empty() { model } else { prompt },
            ),
            &input,
            meta,
        )
        .await
    }

    async fn fal_models(&self, input: Value) -> ToolOutput {
        let Some(fal) = &self.studio.fal else {
            return ToolOutput::err("fal is not configured");
        };
        if let Some(model) = str_arg(&input, "model") {
            let schema = match fal.input_schema(model).await {
                Ok(s) => s,
                Err(e) => return ToolOutput::err(e.message),
            };
            let mut s = format!(
                "{model} inputs (required: {}):\n",
                schema
                    .get("required")
                    .map_or_else(|| "[]".into(), Value::to_string)
            );
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (k, p) in props {
                    let ty = p.get("type").and_then(Value::as_str).unwrap_or("any");
                    let en = p
                        .get("enum")
                        .map(|e| format!(" one of {e}"))
                        .unwrap_or_default();
                    let de = p
                        .get("default")
                        .map(|d| format!(" default {d}"))
                        .unwrap_or_default();
                    let desc: String = p
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .chars()
                        .take(120)
                        .collect();
                    let _ = writeln!(s, "- {k} ({ty}){en}{de}: {desc}");
                }
            }
            s.push_str(&self.studio.price_note(model).await);
            return ToolOutput::ok(s);
        }
        let category = str_arg(&input, "category");
        match fal.models(str_arg(&input, "query"), category, 25).await {
            Ok(models) if models.is_empty() => ToolOutput::ok("No models found."),
            Ok(models) => {
                let mut s = String::new();
                for m in models {
                    let d: String = m.description.chars().take(100).collect();
                    let _ = writeln!(s, "- {} [{}] {}: {d}", m.endpoint_id, m.category, m.name);
                }
                ToolOutput::ok(s)
            }
            Err(e) => ToolOutput::err(e.message),
        }
    }
}

#[async_trait]
impl Tool for StudioTool {
    fn name(&self) -> &'static str {
        match self.which {
            Which::ImageGenerate => "image_generate",
            Which::ImageEdit => "image_edit",
            Which::VideoGenerate => "video_generate",
            Which::SpeechToText => "speech_to_text",
            Which::TextToSpeech => "text_to_speech",
            Which::FalRun => "fal_run",
            Which::FalModels => "fal_models",
        }
    }
    fn rule_name(&self) -> &'static str {
        self.name()
    }
    async fn param_card(&self, input: &Value, ctx: &ToolCtx) -> Option<ParamCard> {
        self.card(input, ctx).await
    }
    /// Paid uploads; `fal_models` is a free lookup.
    fn kind(&self) -> ToolKind {
        if self.which == Which::FalModels {
            ToolKind::Read
        } else {
            ToolKind::Execute
        }
    }
    fn spec(&self) -> ToolSpec {
        let providers = match (self.studio.fal.is_some(), self.studio.hf.is_some()) {
            (true, true) => vec!["fal", "huggingface"],
            (true, false) => vec!["fal"],
            _ => vec!["huggingface"],
        };
        let common = json!({
            "provider": {"type": "string", "enum": providers, "description": "Default: the first listed"},
            "model": {"type": "string", "description": "fal endpoint id (e.g. fal-ai/flux/schnell) or, for huggingface, a Hub repo id"},
            "input": {"type": "object", "description": "Extra model inputs by name (see fal_models with `model`)"},
            "ask": {"type": "boolean", "description": "false: do not stop to ask about optional choices; use defaults"},
            "name": {"type": "string", "description": "File name stem (optional)"},
        });
        let mut props = common.as_object().cloned().unwrap_or_default();
        let (description, required): (String, Vec<&str>) = match self.which {
            Which::ImageGenerate => {
                props.insert("prompt".into(), json!({"type": "string"}));
                (
                    format!(
                        "Generate images (paid) with fal or Hugging Face and save them under {MEDIA_DIR}/; the first image is shown to you. Defaults: fal {}, huggingface {HF_IMAGE_MODEL}. If the answer asks for choices, ask the user, then call again.",
                        kor_providers::fal::DEFAULT_IMAGE_MODEL
                    ),
                    vec!["prompt"],
                )
            }
            Which::ImageEdit => {
                props.insert(
                    "prompt".into(),
                    json!({"type": "string", "description": "The change to make"}),
                );
                props.insert("image".into(), json!({"type": "string", "description": "Project path of the image to edit (png/jpg/webp)"}));
                (
                    format!(
                        "Edit an image from the project (paid): restyle, change, remove background ({}), upscale... Saves the result under {MEDIA_DIR}/. Default fal model {}.",
                        kor_providers::fal::BACKGROUND_REMOVAL_MODEL,
                        kor_providers::fal::DEFAULT_EDIT_MODEL
                    ),
                    vec!["prompt", "image"],
                )
            }
            Which::VideoGenerate => {
                props.insert("prompt".into(), json!({"type": "string"}));
                props.insert("image".into(), json!({"type": "string", "description": "Project path of a start image (image-to-video)"}));
                (
                    format!(
                        "Generate a video (paid, minutes) from text or a start image with fal or Hugging Face; saves an MP4 under {MEDIA_DIR}/. Without `model` it lists models to ask the user about; it also asks about duration/resolution/aspect ratio/audio unless given in `input` or `ask` is false."
                    ),
                    vec!["prompt"],
                )
            }
            Which::SpeechToText => {
                props.insert("audio".into(), json!({"type": "string", "description": "Project path of an audio file (mp3/wav/flac/ogg/m4a)"}));
                (
                    format!(
                        "Transcribe speech (paid) with fal ({FAL_ASR_MODEL}) or Hugging Face ({HF_ASR_MODEL})."
                    ),
                    vec!["audio"],
                )
            }
            Which::TextToSpeech => {
                props.insert(
                    "prompt".into(),
                    json!({"type": "string", "description": "Text to speak"}),
                );
                (
                    format!(
                        "Speak text aloud (paid) with a fal or Hugging Face TTS model; saves audio under {MEDIA_DIR}/. Without `model` it lists models to ask the user about."
                    ),
                    vec!["prompt"],
                )
            }
            Which::FalRun => {
                props.remove("provider");
                props.remove("ask");
                (
                    format!(
                        "Run ANY fal.ai model (paid) by endpoint id with its JSON `input` (find ids and inputs with fal_models). Output files are saved under {MEDIA_DIR}/; missing required inputs are reported so you can ask the user."
                    ),
                    vec!["model", "input"],
                )
            }
            Which::FalModels => {
                props.clear();
                props.insert("query".into(), json!({"type": "string", "description": "Free text, e.g. upscale, background removal, music"}));
                props.insert(
                    "category".into(),
                    json!({"type": "string", "enum": kor_providers::fal::CATEGORIES}),
                );
                props.insert("model".into(), json!({"type": "string", "description": "An endpoint id: show its inputs and price"}));
                ("Find fal.ai models (free): search by text or category, or show one model's input schema and list price.".into(), vec![])
            }
        };
        ToolSpec {
            name: self.name().to_owned(),
            description,
            input_schema: with_media_props(
                json!({"type": "object", "properties": props, "required": required}),
            ),
        }
    }
    fn target(&self, input: &Value) -> Option<String> {
        if self.which == Which::FalModels {
            return str_arg(input, "model")
                .or_else(|| str_arg(input, "query"))
                .map(str::to_owned);
        }
        None
    }
    fn summary(&self, input: &Value) -> String {
        let model = str_arg(input, "model").unwrap_or("default model");
        let prompt: String = str_arg(input, "prompt")
            .unwrap_or_default()
            .chars()
            .take(70)
            .collect();
        format!(
            "{} {model}: {prompt}",
            str_arg(input, "provider").unwrap_or_default()
        )
        .trim()
        .to_owned()
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        match self.which {
            Which::FalRun => self.fal_run(input, ctx).await,
            Which::FalModels => self.fal_models(input).await,
            _ => self.generate(input, ctx).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studio_helpers() {
        assert_eq!(ext_for(Some("image/png"), "https://x/a"), "png");
        assert_eq!(ext_for(None, "https://x/v.MP4?sig=1"), "mp4");
        assert_eq!(ext_for(None, "https://x/noext"), "bin");
        let miss = vec![MissingInput {
            name: "duration".into(),
            description: "Seconds".into(),
            options: vec![json!("5"), json!("10")],
            default: Some(json!("5")),
        }];
        let t = ask_text("video_generate", &miss, " List price: 0.1 USD per second.").unwrap();
        assert!(t.contains("- duration: 5 | 10 (default 5) — Seconds"));
        assert!(t.contains("List price"));
        assert!(ask_text("x", &[], "").is_none());
    }

    #[tokio::test]
    async fn fal_tool_saves_outputs_and_asks_for_missing_inputs() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let base = server.uri();
        let png = vec![0x89, b'P', b'N', b'G'];
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("endpoint_id", "fal-ai/video"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{"endpoint_id": "fal-ai/video",
                "openapi": {"paths": {"/fal-ai/video": {"post": {"requestBody": {"content": {"application/json": {"schema": {
                    "required": ["prompt"], "properties": {"prompt": {}, "duration": {"enum": ["5", "10"], "default": "5"}}}}}}}}}}}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/fal-ai/img"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "r", "response_url": format!("{base}/fal-ai/img/requests/r")})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fal-ai/img/requests/r/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "COMPLETED"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fal-ai/img/requests/r"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"images": [{
                "url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png)),
                "content_type": "image/png"}]})))
            .mount(&server)
            .await;
        let manifest = kor_providers::ProviderManifest::parse(
            include_str!("../../../providers/fal.toml"),
            "fal.toml",
        )
        .unwrap();
        let fal = Fal::with_bases(
            &manifest,
            kor_secrets::SecretString::from("k".to_owned()),
            &base,
            Some(&base),
            "",
        )
        .unwrap();
        let studio = Studio::new(Some(fal), None).unwrap();
        let tools = studio_tools(&studio);
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let video = tools.iter().find(|t| t.name() == "video_generate").unwrap();
        let out = video
            .call(json!({"prompt": "a cat", "model": "fal-ai/video"}), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("duration: 5 | 10 (default 5)"),
            "{}",
            out.content
        );
        let image = tools.iter().find(|t| t.name() == "image_generate").unwrap();
        let out = image
            .call(json!({"prompt": "a cat", "model": "fal-ai/img"}), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("saved: "));
        assert!(out.image.is_some(), "the image goes back to vision models");
    }

    /// A model served only by Together goes through the HF router; poll lines reach the
    /// agent's progress sink; a large input is uploaded to fal storage instead of inlined.
    #[tokio::test]
    async fn router_route_progress_and_large_upload() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path("/api/models/Qwen/Qwen-Image"))
            .and(query_param("expand[]", "inferenceProviderMapping"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"inferenceProviderMapping": {
                "together": {"status": "live", "providerId": "Qwen/Qwen-Image", "task": "text-to-image"}}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/together/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{
                "b64_json": base64::engine::general_purpose::STANDARD.encode([0xFF, 0xD8, 0xFF])}]})))
            .mount(&server)
            .await;
        let manifest = kor_providers::ProviderManifest::parse(
            include_str!("../../../providers/huggingface.toml"),
            "huggingface.toml",
        )
        .unwrap();
        let hf = Hf::with_bases(
            &manifest,
            Some(kor_secrets::SecretString::from("hf_t".to_owned())),
            &base,
            &base,
            false,
        )
        .unwrap()
        .with_router(&base)
        .unwrap();
        let studio = Studio::new(None, Some(hf)).unwrap();
        let tools = studio_tools(&studio);
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(dir.path()).unwrap();
        let image = tools.iter().find(|t| t.name() == "image_generate").unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let sink: kor_tools::ProgressSink = Arc::new(move |l| s2.lock().unwrap().push(l));
        let out = kor_tools::with_progress(
            sink,
            image.call(
                json!({"prompt": "a cat", "model": "Qwen/Qwen-Image", "provider": "huggingface"}),
                &ctx,
            ),
        )
        .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("via Hugging Face (together)"),
            "{}",
            out.content
        );
        assert!(out.content.contains("saved: "));
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["running on together".to_owned()]
        );

        // fal storage: a 3 MB input goes to the upload flow (the PUT itself is covered in
        // kor-providers; outside its tests a plain-http upload URL is refused), a small one stays
        // inline.
        Mock::given(method("POST"))
            .and(path("/storage/upload/initiate"))
            .and(wiremock::matchers::body_json(
                json!({"content_type": "image/png", "file_name": "big.png"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "upload_url": format!("{base}/put/x"), "file_url": "https://v3b.fal.media/files/x.png"})))
            .expect(1)
            .mount(&server)
            .await;
        let fal_manifest = kor_providers::ProviderManifest::parse(
            include_str!("../../../providers/fal.toml"),
            "fal.toml",
        )
        .unwrap();
        let key = kor_secrets::SecretString::from("k".to_owned());
        let fal = Fal::with_bases(&fal_manifest, key.clone(), &base, Some(&base), "")
            .unwrap()
            .with_storage(&fal_manifest, key, &base)
            .unwrap();
        let big = dir.path().join("big.png");
        std::fs::write(&big, vec![0u8; 3 * 1024 * 1024]).unwrap();
        assert_eq!(
            fal_input_url(&fal, &big).await.unwrap_err(),
            "malformed upload answer"
        );
        let small = dir.path().join("small.png");
        std::fs::write(&small, [1u8, 2, 3]).unwrap();
        assert!(
            fal_input_url(&fal, &small)
                .await
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn stems_are_short_ascii() {
        assert_eq!(
            stem(None, "Güneşli bir İstanbul sabahı, Galata"),
            "gunesli-bir-istanbul-sabahi-galata"
        );
        assert_eq!(stem(Some("Logo v2!"), "x"), "logo-v2");
        assert_eq!(stem(None, "!!!"), "media");
        assert!(stem(None, &"a b ".repeat(50)).len() <= 41);
    }

    #[test]
    fn saving_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let a = save(dir.path(), "x", None, "png", b"1").unwrap();
        let b = save(dir.path(), "x", None, "png", b"2").unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(dir.path().join(MEDIA_DIR)));
        assert_eq!(std::fs::read(&a).unwrap(), b"1");
        assert_eq!(std::fs::read(&b).unwrap(), b"2");
    }

    /// The output path must survive the secret redactor, or the model and the desktop preview get
    /// `[REDACTED]` instead of the file.
    #[test]
    fn saved_paths_are_not_mistaken_for_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let p = save(dir.path(), "campfire-logo", Some(2), "png", b"x").unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.chars().any(|c| c.is_ascii_digit()), "{name}");
        // The name Kor chose, as the project-relative path the model sees (the test's own temp
        // folder is random on macOS and may look like a token itself).
        let line = format!("saved: {MEDIA_DIR}/{name}");
        assert_eq!(kor_secrets::Redactor::default().redact(&line), line);
        assert_eq!(letters(0), "a");
        assert_eq!(letters(27), "bb");
    }

    #[test]
    fn choices_are_checked_against_the_model() {
        let allowed = vec!["5s".to_owned(), "10s".to_owned()];
        assert!(check_choice("duration", Some("5s"), &allowed).is_ok());
        assert!(
            check_choice("duration", Some("7s"), &allowed)
                .unwrap_err()
                .contains("5s, 10s")
        );
        assert!(
            check_choice("duration", Some("7s"), &[]).is_ok(),
            "no list means no check"
        );
        assert!(check_choice("duration", None, &allowed).is_ok());
    }

    /// Venice rejects `seedance-1-5-pro-text-to-video-basic` without `aspect_ratio` ("Required").
    #[test]
    fn required_options_get_sensible_defaults() {
        let ratios: Vec<String> = ["21:9", "16:9", "1:1"].map(String::from).to_vec();
        let res: Vec<String> = ["1080p", "720p", "480p"].map(String::from).to_vec();
        assert_eq!(default_choice(&ratios, "16:9", false), Some("16:9"));
        assert_eq!(
            default_choice(&["1:1".to_owned(), "9:16".to_owned()], "16:9", false),
            Some("1:1")
        );
        assert_eq!(default_choice(&res, "720p", true), Some("720p"));
        assert_eq!(
            default_choice(&["1080p".to_owned(), "540p".to_owned()], "720p", true),
            Some("540p")
        );
        assert_eq!(
            default_choice(&[], "16:9", false),
            None,
            "image-to-video takes the ratio from the image"
        );
    }

    #[test]
    fn start_image_must_be_a_supported_picture() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("a.png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
        assert!(
            image_data_url(&png)
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        let txt = dir.path().join("a.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(image_data_url(&txt).is_err());
    }
}
