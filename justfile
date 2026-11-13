# SPDX-License-Identifier: Apache-2.0 OR MIT
# Kor's single entry point (spec §22.2): `git clone && just setup && just dev` on Windows, macOS and Linux.
# Every recipe is one program per line (no shell syntax), so the same file runs under PowerShell, sh and bash.
# Toolchains: rust-toolchain.toml (Rust), Node 22 (desktop renderer), Python 3.11+ (release helpers).

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

python := if os_family() == "windows" { "python" } else { "python3" }
exe := if os_family() == "windows" { ".exe" } else { "" }
# The host's Rust target, for the desktop app's `kor` sidecar (binaries/kor-<target>).
target := if os() == "windows" { arch() + "-pc-windows-msvc" } else if os() == "macos" { arch() + "-apple-darwin" } else { arch() + "-unknown-linux-gnu" }

# List the recipes.
default:
    @just --list --unsorted

# Install the pinned Rust toolchain and the renderer's npm packages.
setup:
    rustup show active-toolchain
    rustup component add rustfmt clippy
    npm --prefix frontend ci

# Run the desktop app (Tauri dev) with the UI rebuilt on change.
dev:
    npm --prefix frontend run dev

# Build the CLI, the renderer and the desktop app (debug).
build:
    cargo build --manifest-path backend/Cargo.toml --workspace --locked
    npm --prefix frontend run build:ui
    cargo build --manifest-path frontend/src-tauri/Cargo.toml --locked

# Run the `kor` CLI from source: `just kor doctor`.
kor *args:
    cargo run --manifest-path backend/Cargo.toml --locked -p kor-cli -- {{args}}

# Every test: Rust (CLI, crates, desktop), renderer, release and repo tooling.
test:
    cargo test --manifest-path backend/Cargo.toml --workspace --locked
    cargo test --manifest-path frontend/src-tauri/Cargo.toml --locked
    node --test frontend/src/desktop/renderer/*.test.ts frontend/src/desktop/renderer/i18n/*.test.ts frontend/src/desktop/renderer/panels/*/*.test.ts frontend/src/desktop/renderer/review/*.test.ts frontend/src/desktop/renderer/perfmon/*.test.ts frontend/scripts/*.test.ts frontend/src/desktop/renderer/kor-ui/*.test.ts
    {{python}} scripts/desktop_release.py selftest
    {{python}} scripts/repro_check.py selftest
    {{python}} scripts/pgo.py selftest

# Lints as CI runs them: versions, clippy (warnings are errors), strict TypeScript, design rules.
lint:
    {{python}} scripts/version.py check
    cargo clippy --manifest-path backend/Cargo.toml --workspace --all-targets --locked -- -D warnings
    cargo clippy --manifest-path frontend/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
    cargo fmt --manifest-path backend/Cargo.toml --all --check
    npm --prefix frontend run typecheck
    npm --prefix frontend run check:design

# Format the Rust code.
fmt:
    cargo fmt --manifest-path backend/Cargo.toml --all
    cargo fmt --manifest-path frontend/src-tauri/Cargo.toml --all

# Installers for this OS (ADR 0032/0064): webview = bootstrapper | offline | fixed (with runtime=<dir>).
release bundles="" webview="bootstrapper" runtime="":
    {{python}} scripts/version.py check
    cargo build --manifest-path backend/Cargo.toml --locked --release -p kor-cli --target {{target}}
    {{python}} -c "import shutil, pathlib; d = pathlib.Path('frontend/src-tauri/binaries'); d.mkdir(exist_ok=True); shutil.copy('backend/target/{{target}}/release/kor{{exe}}', d / 'kor-{{target}}{{exe}}'); pathlib.Path('frontend/src-tauri/integrations').mkdir(exist_ok=True)"
    {{python}} scripts/desktop_release.py webview {{webview}} frontend/src-tauri/target/tauri.webview.json {{runtime}}
    npm --prefix frontend run build -- --target {{target}} {{ if bundles != "" { "--bundles " + bundles } else { "" } }} --config src-tauri/tauri.release.conf.json --config src-tauri/target/tauri.webview.json

# Windows installers as a debug build (fast; what CI checks on every PR): msi + nsis.
release-debug-windows:
    cargo build --manifest-path backend/Cargo.toml --locked -p kor-cli
    {{python}} -c "import shutil, pathlib; d = pathlib.Path('frontend/src-tauri/binaries'); d.mkdir(exist_ok=True); shutil.copy('backend/target/debug/kor.exe', d / 'kor-{{target}}.exe'); pathlib.Path('frontend/src-tauri/integrations').mkdir(exist_ok=True)"
    npm --prefix frontend run build -- --debug --bundles msi,nsis --config src-tauri/tauri.release.conf.json

# Show or install system prerequisites (§24.3) with the CLI from source: `just prereq list`.
prereq *args:
    cargo run --manifest-path backend/Cargo.toml --locked -p kor-cli -- prereq {{args}}
