# kor-runtime

**What it does.** Session assembly shared by the CLI and the desktop app: trust, key handling, provider registry, privacy and offline gates, updates, analysis commands (audit, quality, deps), forges, triggers, offline bundles. Functions return localized messages instead of printing.

**What it does not do.** No terminal or window code: callers decide how to show results.

**Called by:** `kor-cli`, `kor-desktop`, `kor-proxy`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
