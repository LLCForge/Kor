# kor-components

**What it does.** Layer B (spec §24.2): on-demand helper programs and models (Python, Node, PDFium, Tesseract data, Chromium, llama.cpp, whisper, embedding models) pinned in `components.toml`, downloaded over HTTPS with resume and size caps, SHA-256 verified, unpacked into Kor's own folder.

**What it does not do.** No admin rights, no PATH changes, nothing in privacy mode, never an unpinned download.

**Called by:** `kor-cli`, `kor-desktop`, `kor-memory`, `kor-runtime`, `kor-tools`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
