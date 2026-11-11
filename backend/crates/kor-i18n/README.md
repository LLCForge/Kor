# kor-i18n

**What it does.** Turkish, English and French user-facing texts as Fluent files compiled into the binary (`locales/{en,tr,fr}/kor.ftl`), with English as the fallback.

**What it does not do.** No runtime downloads of translations; the desktop renderer has its own catalogues.

**Called by:** `kor-cli`, `kor-computer`, `kor-config`, `kor-desktop`, `kor-health`, `kor-proxy`, `kor-runtime`, `kor-web`.

Module map: the `//!` comment at the top of `src/lib.rs`. Where it sits in the whole: [ARCHITECTURE.md](../../../ARCHITECTURE.md).
