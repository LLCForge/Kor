# kor-health

**What it does.** System profile (`system.json`), health alerts (disk, memory, temperature, runaway processes, caches) and cache cleanup suggestions (spec §19.2–19.3).

**What it does not do.** Does not delete anything without the user's approval.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-runtime`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
