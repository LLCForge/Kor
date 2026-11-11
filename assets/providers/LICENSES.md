# Provider and model logos (`assets/providers/`)

Spec §23.2, ADR 0053. Every logo is 24×24 (`viewBox="0 0 24 24"`) in two variants:

- `<id>.svg`: single colour, `fill="currentColor"` (takes the surrounding text colour; used by default in the UI and,
  as a glyph, in the TUI).
- `<id>-color.svg`: the brand's own colours. Brands whose mark is monochrome by design (OpenAI, Anthropic, xAI,
  Grok, Groq, Ollama, LM Studio, Moonshot, Z.ai, GitHub, Black Forest Labs/Flux) have no separate colour file; the
  single-colour file is used for both variants.

The desktop imports them through the generated `frontend/src/desktop/renderer/provider-logos.gen.ts`
(`node frontend/scripts/provider-logos.ts`). A provider without a file here is drawn as a letter monogram in an
ember circle (`ProviderLogo`), never with a guessed or redrawn logo.

## Source and licence of the files

The SVG files come from [`@lobehub/icons-static-svg`](https://github.com/lobehub/lobe-icons) 1.102.0
(MIT License, © LobeHub), copied unchanged. The MIT licence covers the SVG drawings as distributed by LobeHub.

Exceptions, taken from the vendor's own published file (not openly licensed; trademark, nominative use only; the
download URL of each is in `assets/LICENSES.toml`): `abliteration`, `arliai`, `localai`, `mancer`, `uncensoredllm`.
They were sanitised (scripts, styles, titles and unused ids removed; referenced ids namespaced) without changing
geometry or colours; the single-colour file keeps only the mark's own shapes with `fill="currentColor"`.

Providers with no official SVG mark found on the vendor's domain are drawn with the neutral letter badge:
Abacus.AI, Oracle (OCI), Refuseless, Jan (its site logo is a raster image inside an SVG), KoboldCpp, llama.cpp and
text-generation-webui.

## Trademarks

The names and logos are trademarks of their respective owners (Anthropic, OpenAI, Google, DeepSeek, Alibaba Cloud
(Qwen), xAI, Meta, NVIDIA, Black Forest Labs, Moonshot AI (Kimi), Zhipu AI (ChatGLM), AionLabs, Kuaishou (Kling),
OpenRouter, Groq, Mistral AI, Ollama, Together AI, Venice.ai, Microsoft (Azure), Amazon Web Services (Bedrock,
SageMaker), Cerebras, Cohere, fal, Abliteration.ai, Arli AI, LocalAI, Mancer, UncensoredLLM.ai, Fireworks AI, Hugging Face, LM Studio, vLLM, Perplexity, SambaNova, Z.ai, GitHub).
Kor is not affiliated with or endorsed by them. The logos are shown only to identify the provider or model family
the user selected (nominative use), following the owners' brand guidelines:

- shown at the size of the adjacent text (16–24 px), never larger than the model name;
- never recoloured except through the single-colour variant, never stretched, rotated, outlined or combined into a
  new mark;
- clear space around the mark (the 24×24 box) is kept; the monogram fallback is used instead of approximating a
  logo that is not in this folder.

If an owner asks for a logo to be removed or changed, delete its files here and regenerate; the UI falls back to the
monogram automatically.
