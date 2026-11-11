# kor-cli

**What it does.** The `kor` binary: clap commands (one `*_cmd.rs` file per command group), the REPL and full-screen TUI, `kor doctor`, `kor prereq`, and the hidden `install-hooks` step the desktop installers run.

**What it does not do.** Thin: core logic belongs in kor-runtime and the crates below it.

**Called by:** nothing inside the workspace (it is a binary).

Module map: the `//!` comment at the top of `src/lib.rs` (`src/main.rs` for the binary). Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
