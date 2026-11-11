# kor-tools

**What it does.** The built-in tools the model calls: files (read, edit with verification, write), search, shell (through kor-sandbox), web and browser, computer use, PDF and OCR, secret scan. Every path is confined to the project root; failures are tool results, never panics.

**What it does not do.** No permission decisions (kor-agent) and no network outside the network guard.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-memory`, `kor-runtime`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
