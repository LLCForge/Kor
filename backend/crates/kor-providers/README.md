# kor-providers

**What it does.** The `Provider` trait, protocol adapters (OpenAI-compatible, Anthropic, Gemini, Bedrock …), declarative provider manifests (`backend/providers/*.toml`), failover and the model catalogue.

**What it does not do.** No agent logic; adding an OpenAI-compatible provider needs only a manifest.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-proxy`, `kor-runtime`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).

## Direct media providers (§20)

`direct.rs` talks to each provider's own API with one submit → poll → outputs shape; the `direct_generate` agent tool (kor-agent) uses it, saves outputs in the gallery and charges list prices to the usage ledger as estimates (gpt-image-1 is token-priced and Luma has no fixed list price; neither is charged).

| Provider | Key | Endpoints | Jobs |
| --- | --- | --- | --- |
| OpenAI Images | `OPENAI_API_KEY` (chat key) | `POST /images/generations` | sync |
| Google Imagen | `GEMINI_API_KEY` (chat key) | `POST /models/{model}:predict` | sync |
| Google Veo | `GEMINI_API_KEY` | `:predictLongRunning`, `GET /{operation}`, file download with the key | polled |
| Stability AI | `STABILITY_API_KEY` | `POST /v2beta/stable-image/generate/{core,ultra,sd3}` (multipart) | sync |
| Runway | `RUNWAYML_API_SECRET` | `POST /v1/text_to_image`, `/v1/image_to_video`, `GET /v1/tasks/{id}` | polled |
| Luma | `LUMAAI_API_KEY` | `POST /dream-machine/v1/generations[/image]`, `GET …/{id}` | polled |
| ComfyUI (local) | none; `[media] comfyui_url`, `comfyui_checkpoint` | `POST /prompt`, `GET /history/{id}`, `GET /view` | polled over HTTP; the only one in privacy mode |

Keys also come from `kor auth login stability|runway|luma`. ComfyUI workflows are API-format JSON (ComfyUI "Export (API)"); `{{prompt}}`, `{{negative}}` and `{{seed}}` in string values are filled in. Progress is read from `/history` rather than the WebSocket feed.
