# kor-sandbox

**What it does.** Wraps every command the agent runs so the OS confines it: bubblewrap (Linux), Seatbelt (macOS), AppContainer and job objects (Windows), Docker; writes only inside the project, network allowlist, CPU/memory limits; violations are reported.

**What it does not do.** Does not decide what may run (that is kor-agent's permission system); it only confines what was allowed.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-runtime`, `kor-tools`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
