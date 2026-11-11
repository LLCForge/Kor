# kor-platform

**What it does.** Every operating-system difference behind one API: folders, processes and file locks, PATH (`cli_path`), shell completions, OS integration (context menus, jump lists, toasts, taskbar), AppContainer, WSL, the embedded `branding.toml` (`brand()`), Layer C system installs (`sysdeps`) and Chrome native-host registration (`native_host`).

**What it does not do.** No product logic. Other crates must not branch on the OS themselves; `tests/leak.rs` enforces it.

**Called by:** `kor-agent`, `kor-cli`, `kor-components`, `kor-computer`, `kor-desktop`, `kor-health`, `kor-runtime`, `kor-sandbox`, `kor-tools`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
