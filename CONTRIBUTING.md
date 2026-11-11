# Contributing to Kor

Thank you for helping. Kor is 100% free and open source (Apache-2.0 OR MIT); every part of it, including the
team server and the desktop app, lives in this repository under the same license. This guide gets your first
pull request merged.

## Your first pull request

1. Pick an issue labelled `good first issue` or `help wanted`, or open one first for anything larger than a bug
   fix. Changes to the architecture (a new crate, a new dependency between crates, a new storage format, a new
   network destination) need an issue first to agree on the design.
2. Set up once and run the app:
   ```bash
   just setup        # toolchains pinned by rust-toolchain.toml, npm packages, git hooks: none
   just dev          # desktop app with the CLI next to it
   ```
   No `just`? `cargo install just`, `winget install Casey.Just`, `brew install just` or `apt install just`.
3. Before pushing:
   ```bash
   just fmt          # rustfmt
   just lint         # clippy -D warnings, TypeScript strict, design checks
   just test         # Rust and renderer tests
   ```
4. Open the pull request with the template; CI runs the same recipes on Linux, macOS and Windows.

## Rules that keep Kor forkable

- **Three languages, always.** User-facing text goes into `backend/crates/kor-i18n/locales/{en,tr,fr}/kor.ftl`
  (CLI, TUI) or, for the desktop, as a Turkish source string with its English and French in the renderer
  catalogues. The `every_locale_has_the_same_keys` test fails otherwise. French in `.ftl` uses a narrow no-break
  space (U+202F) before `: ; ? !`.
- **Three platforms, always.** OS differences live only in `kor-platform` (and `kor-sandbox`); a ratchet test
  (`kor-platform/tests/leak.rs`) fails when another crate branches on the OS.
- **No brand strings in code.** The product name, ids, URLs and colours come from `branding.toml` and
  `assets/theme.json`.
- **Code style.** `clippy::pedantic` clean; English identifiers and comments; comments say why, not what; no
  magic numbers; `unsafe` only in `kor-platform` with a `// SAFETY:` comment and an ADR.
- **Privacy.** No telemetry, no network call to a destination the user did not configure, no secrets in logs
  (pass anything that may hold a key through `kor_secrets::Redactor`). Privacy mode must keep working offline.
- **Dependencies.** OSI-approved licenses only; `cargo deny` rejects the rest.

## Commits

- [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/): `feat(graph): incremental update by
  SHA-256`, `fix(cli): …`, `docs: …`, `refactor: …`, `test: …`, `ci: …`, `chore: …`; `!` or a
  `BREAKING CHANGE:` footer for breaking changes. `kor changelog` builds the changelog from them.
- Sign off every commit (`git commit -s`) to certify the
  [Developer Certificate of Origin](https://developercertificate.org/).

## Versions

Releases follow SemVer. A breaking change to a public contract (the `kor-core` traits, the IPC schema, the
provider manifest schema, the on-disk formats, the CLI flags) is announced one major version ahead: the old form
is marked `deprecated` and keeps working for that whole major version.
`scripts/version.py` is the only thing that edits version lines (ADR 0012); `just release` runs it.

By contributing you agree your work is dual-licensed under Apache-2.0 and MIT.
