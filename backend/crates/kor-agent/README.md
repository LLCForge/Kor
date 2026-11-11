# kor-agent

**What it does.** The agent loop: model call, tool calls with permission checks, hooks and per-file review before writes, results; sessions and checkpoints, subagents and multi-agent orchestration, MCP client and server, the hash-chained audit log, cost routing.

**What it does not do.** No CLI or UI code; session assembly (config, keys, providers) is kor-runtime's job.

**Called by:** `kor-cli`, `kor-desktop`, `kor-runtime`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
