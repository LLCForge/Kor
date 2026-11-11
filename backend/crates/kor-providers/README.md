# kor-providers

**What it does.** The `Provider` trait, protocol adapters (OpenAI-compatible, Anthropic, Gemini, Bedrock …), declarative provider manifests (`backend/providers/*.toml`), failover and the model catalogue.

**What it does not do.** No agent logic; adding an OpenAI-compatible provider needs only a manifest.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-proxy`, `kor-runtime`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
