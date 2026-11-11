# kor-graph

**What it does.** The live project graph built without an LLM (spec §7): tree-sitter parsing, resolver, petgraph + SQLite store, incremental updates by SHA-256, file watcher, communities, config/env and git co-change layers, queries (explain, path, impact, hotspots) and exports (JSON, Mermaid, Obsidian, HTML).

**What it does not do.** No model calls and no network.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-runtime`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
