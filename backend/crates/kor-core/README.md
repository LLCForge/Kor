# kor-core

**What it does.** Shared, dependency-light types every Kor crate exchanges: messages, tool calls, permission modes, usage; the product version; the network guard (`net::client_builder`) every HTTP client is built from; the IPC schema (`schema/ipc.json`) and theme constants.

**What it does not do.** No I/O besides the network guard, no provider wire formats, no OS branches.

**Called by:** `kor-agent`, `kor-cli`, `kor-compat`, `kor-components`, `kor-config`, `kor-desktop`, `kor-providers`, `kor-proxy`, `kor-runtime`, `kor-secrets`, `kor-tools`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
