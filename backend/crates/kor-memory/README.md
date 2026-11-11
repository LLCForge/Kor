# kor-memory

**What it does.** Project memory as plain Markdown per project and the global knowledge base (SQLite with full-text and local-embedding search), `kor recall`, `kor knowledge`.

**What it does not do.** Never sends text or embeddings off the machine; only appends or replaces its own marked sections.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-runtime`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
