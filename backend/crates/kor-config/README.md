# kor-config

**What it does.** Layered TOML configuration (defaults, system, user, project, environment, command line) with the layer that set each key, for `kor config explain`.

**What it does not do.** No secrets (they live in kor-secrets) and no writes to other tools' config files.

**Called by:** `kor-cli`, `kor-computer`, `kor-desktop`, `kor-health`, `kor-proxy`, `kor-runtime`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
