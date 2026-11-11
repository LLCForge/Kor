# kor-compat

**What it does.** Reads other AI coding tools' files (Claude Code, Codex, Cursor, Windsurf, Cline, Copilot, Gemini, Aider …) and Kor's own, in place, into one model; trust rules for untrusted projects.

**What it does not do.** Never writes to another tool's files; exports only under `.kor/export/`.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-runtime`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
