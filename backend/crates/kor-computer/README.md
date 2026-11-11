# kor-computer

**What it does.** The policy the computer-use tools share: per-app tiers (blocked, read, click, ask, full), defaults, stop when the user moves the mouse.

**What it does not do.** The tools themselves live in kor-tools; this crate does not click anything.

**Called by:** `kor-cli`, `kor-desktop`, `kor-health`, `kor-runtime`, `kor-tools`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
