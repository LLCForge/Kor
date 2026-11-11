# kor-lint

**What it does.** A source scanner for the performance rules clippy cannot see (spec §32.2, ADR 0062):
blocking calls inside `async` code, I/O or JSON parsing on the UI render path, unbounded channels, uncached
regexes. `kor-lint [--baseline FILE] [--update] PATH...` exits 1 when a file gains findings over the baseline.

**What it does not do.** No fixes and no type information: it is a pattern scanner run in CI next to clippy.

**Called by:** CI and `just lint`-style tooling; nothing inside the workspace links it.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
