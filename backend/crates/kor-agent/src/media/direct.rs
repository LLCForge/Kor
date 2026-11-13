//! `direct_generate`: image and video generation straight from OpenAI Images, Google Imagen and
//! Veo, Stability AI, Runway, Luma and a local ComfyUI (§20), one tool with a `provider` field.
//! Long jobs are polled until they finish (or the call is cancelled); outputs go to the gallery and
//! their list price to the usage ledger, like every other media tool.

#![allow(clippy::doc_markdown)] // product names (ComfyUI, OpenAI) read better as written

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use kor_core::ToolSpec;
use kor_providers::direct::{self, Direct, DirectKind, Job, Poll, Request};
use kor_tools::{Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};

use super::{
    MediaEnv, POLL_EVERY, VIDEO_DEADLINE, derived_from, ext_for, file_data_url, finish, save,
    shown, stem, str_arg, with_media_props,
};
use crate::gallery::Meta;
use kor_providers::media_catalog::Capability;

/// The configured direct providers of a session.
#[derive(Debug)]
pub struct DirectMedia {
    pub clients: Vec<Direct>,
    /// ComfyUI checkpoint for the built-in text-to-image workflow (`[media] comfyui_checkpoint`).
    pub checkpoint: String,
    pub env: MediaEnv,
}

impl DirectMedia {
    /// `None` without any provider.
    pub fn new(clients: Vec<Direct>, checkpoint: String, env: MediaEnv) -> Option<Arc<Self>> {
        (!clients.is_empty()).then(|| {
            Arc::new(Self {
                clients,
                checkpoint,
                env,
            })
        })
    }

    fn get(&self, kind: DirectKind) -> Option<&Direct> {
        self.clients.iter().find(|c| c.kind == kind)
    }
}

#[derive(Debug)]
pub struct DirectGenerate(pub Arc<DirectMedia>);

pub fn direct_tools(d: &Arc<DirectMedia>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(DirectGenerate(d.clone()))]
}

impl DirectGenerate {
    fn providers(&self) -> Vec<&'static str> {
        self.0.clients.iter().map(|c| c.kind.id()).collect()
    }

    /// The request from the tool input (paths resolved inside the project).
    fn request(&self, kind: DirectKind, input: &Value, ctx: &ToolCtx) -> Result<Request, String> {
        let video = str_arg(input, "kind") == Some("video");
        if video && !kind.videos() {
            return Err(format!("{} makes images only", kind.id()));
        }
        if !video && !kind.images() {
            return Err(format!("{} makes videos only (kind: video)", kind.id()));
        }
        let image_url = match str_arg(input, "image") {
            Some(p) => Some(file_data_url(&ctx.resolve(p)?)?),
            None => None,
        };
        let workflow = if kind == DirectKind::ComfyUi {
            Some(match str_arg(input, "workflow") {
                Some(p) => {
                    let path = ctx.resolve(p)?;
                    let text = std::fs::read_to_string(&path).map_err(|e| format!("{p}: {e}"))?;
                    serde_json::from_str(&text)
                        .map_err(|e| format!("{p}: not a workflow JSON ({e})"))?
                }
                None if !self.0.checkpoint.is_empty() => {
                    direct::default_workflow(&self.0.checkpoint)
                }
                None => {
                    return Err("give `workflow` (an API-format workflow JSON exported from ComfyUI) or set \
                         `[media] comfyui_checkpoint`"
                        .into());
                }
            })
        } else {
            None
        };
        let num = |k: &str| input.get(k).and_then(Value::as_u64);
        Ok(Request {
            model: str_arg(input, "model")
                .unwrap_or(kind.default_model(video))
                .to_owned(),
            prompt: str_arg(input, "prompt").unwrap_or_default().to_owned(),
            negative_prompt: str_arg(input, "negative_prompt").map(str::to_owned),
            aspect_ratio: str_arg(input, "aspect_ratio").map(str::to_owned),
            size: str_arg(input, "size").map(str::to_owned),
            duration_s: num("duration").and_then(|d| u32::try_from(d.clamp(1, 60)).ok()),
            n: num("n").map_or(1, |n| u8::try_from(n.clamp(1, 4)).unwrap_or(1)),
            seed: input.get("seed").and_then(Value::as_i64),
            image_url,
            workflow,
            video,
        })
    }
}

#[async_trait]
impl Tool for DirectGenerate {
    fn name(&self) -> &'static str {
        "direct_generate"
    }
    fn rule_name(&self) -> &'static str {
        "direct_generate"
    }
    /// Paid network call (ComfyUI aside): asks unless allowed by a rule.
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn spec(&self) -> ToolSpec {
        let providers = self.providers();
        ToolSpec {
            name: self.name().to_owned(),
            description: format!(
                "Generate images or videos straight from a provider's own API and save them under kor-media/. \
                 Configured: {}. Videos: veo, runway (needs `image`, image-to-video), luma. ComfyUI runs locally \
                 (free) with a workflow JSON. Long jobs are polled until done. The output lists the saved files.",
                providers.join(", ")
            ),
            input_schema: with_media_props(json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string", "enum": providers},
                    "kind": {"type": "string", "enum": ["image", "video"]},
                    "prompt": {"type": "string"},
                    "model": {"type": "string", "description": "Default: gpt-image-1, imagen-4.0-generate-001, veo-3.0-fast-generate-001, core (Stability: core|ultra|sd3), gen4_image / gen4_turbo, photon-1 / ray-2"},
                    "negative_prompt": {"type": "string"},
                    "aspect_ratio": {"type": "string", "description": "e.g. 16:9 (Imagen, Veo, Stability, Luma)"},
                    "size": {"type": "string", "description": "OpenAI 1024x1024, Runway ratio 1280:720, Luma resolution 720p"},
                    "duration": {"type": "integer", "description": "Video seconds"},
                    "n": {"type": "integer", "minimum": 1, "maximum": 4},
                    "seed": {"type": "integer"},
                    "image": {"type": "string", "description": "Project path of a start / reference image"},
                    "workflow": {"type": "string", "description": "ComfyUI: project path of an API-format workflow JSON with {{prompt}}, {{negative}}, {{seed}} placeholders"},
                    "name": {"type": "string", "description": "File name stem (optional)"},
                },
                "required": ["provider", "prompt"],
            })),
        }
    }
    fn target(&self, _input: &Value) -> Option<String> {
        None
    }
    fn summary(&self, input: &Value) -> String {
        let p: String = str_arg(input, "prompt")
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect();
        format!("{}: {p}", str_arg(input, "provider").unwrap_or("?"))
    }
    async fn call(&self, input: Value, ctx: &ToolCtx) -> ToolOutput {
        let started = Instant::now();
        let Some(kind) = str_arg(&input, "provider").and_then(DirectKind::parse) else {
            return ToolOutput::err(format!(
                "`provider` must be one of: {}",
                self.providers().join(", ")
            ));
        };
        let Some(client) = self.0.get(kind) else {
            return ToolOutput::err(format!("{} is not configured (no key)", kind.id()));
        };
        let req = match self.request(kind, &input, ctx) {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(e),
        };
        let job = tokio::select! {
            r = client.submit(&req) => r,
            () = ctx.cancel.cancelled() => return ToolOutput::err("cancelled"),
        };
        let outputs = match job {
            Ok(Job::Done(o)) => o,
            Ok(Job::Pending(id)) => match wait(client, &id, ctx).await {
                Ok(o) => o,
                Err(e) => return ToolOutput::err(e),
            },
            Err(e) => return ToolOutput::err(format!("generation failed: {e}")),
        };
        let stem = stem(str_arg(&input, "name"), &req.prompt);
        let many = outputs.len() > 1;
        let mut out = String::new();
        let mut paths = Vec::new();
        for (i, o) in outputs.iter().enumerate() {
            let (bytes, ct) = match client.fetch(o).await {
                Ok(b) => b,
                Err(e) => return ToolOutput::err(format!("could not download the output: {e}")),
            };
            let url = match o {
                direct::Output::Url(u) => u.as_str(),
                direct::Output::Bytes { .. } => "",
            };
            let ext = ext_for(ct.as_deref(), url);
            match save(&ctx.root, &stem, many.then_some(i), &ext, &bytes) {
                Ok(p) => {
                    let _ = writeln!(out, "saved: {}", shown(&p));
                    paths.push(p);
                }
                Err(e) => return ToolOutput::err(format!("could not save the output: {e}")),
            }
        }
        let cost = direct::estimate(kind, &req).map(|c| (c, "USD".to_owned(), true));
        let price = cost.as_ref().map_or_else(String::new, |(c, _, _)| {
            format!(" (about ${c:.2}, list price)")
        });
        out.insert_str(
            0,
            &format!(
                "Generated {} file(s) with {}/{}{price}.\n",
                paths.len(),
                kind.id(),
                req.model
            ),
        );
        let meta = Meta {
            capability: Some(
                if req.video {
                    Capability::Video
                } else {
                    Capability::Image
                }
                .key()
                .into(),
            ),
            provider: kind.id().into(),
            model: req.model.clone(),
            prompt: req.prompt.clone(),
            params: input.clone(),
            seed: req.seed,
            cost,
            secs: started.elapsed().as_secs(),
            inputs: derived_from(&input).into_iter().collect(),
            action: str_arg(&input, "action").map(str::to_owned),
        };
        finish(&self.0.env, ctx, &paths, &meta, &input, &mut out);
        ToolOutput::ok(out)
    }
}

/// Poll until the job is done, failed, cancelled or past the deadline.
async fn wait(client: &Direct, id: &str, ctx: &ToolCtx) -> Result<Vec<direct::Output>, String> {
    let deadline = Instant::now() + VIDEO_DEADLINE;
    let every = if client.kind == DirectKind::ComfyUi {
        Duration::from_secs(1)
    } else {
        POLL_EVERY
    };
    loop {
        match client.poll(id).await {
            Ok(Poll::Done(o)) => return Ok(o),
            Ok(Poll::Failed(why)) => return Err(format!("{} failed: {why}", client.kind.id())),
            Ok(Poll::Running(_)) => {}
            Err(e) => return Err(format!("status check failed: {e}")),
        }
        if Instant::now() > deadline {
            return Err(format!(
                "{} did not finish within {} minutes (job {id})",
                client.kind.id(),
                VIDEO_DEADLINE.as_secs() / 60
            ));
        }
        tokio::select! {
            () = tokio::time::sleep(every) => {}
            () = ctx.cancel.cancelled() => return Err("cancelled".into()),
        }
    }
}
