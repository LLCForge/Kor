# kor-secrets

**What it does.** API key lookup (environment, then OS keychain), the encrypted vault, `SecretString` that zeroizes and never prints, and redaction of secrets in output and logs.

**What it does not do.** Never writes keys into config files or logs.

**Called by:** `kor-agent`, `kor-cli`, `kor-desktop`, `kor-providers`, `kor-proxy`, `kor-runtime`, `kor-tools`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
