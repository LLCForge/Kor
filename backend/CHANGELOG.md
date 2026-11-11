# Changelog

All notable changes are documented here. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Changed
- Desktop control (`computer.enabled`), the right-click menu and the available OS integrations are on by default:
  the desktop installs them once on first launch and never re-installs one the user removed (`[integrations]`).
- Default model `venice/deepseek-v4-1-flash-uncensored`; Artificial Analysis scores come only from the central
  model catalog (no per-user key).
- Desktop: animations with a "System / Always on / Off" setting, responsive layout from 800×600 up, views spread
  over the side-panel header, the sidebar's grouped Tools and Settings; every Kor command in the `/` menu.

### Fixed
- Workspace trust did not match the verbatim (`\\?\C:\…`) form of the same folder; the per-file review was skipped
  for tools wrapped by the project graph; the desktop overflowed tokio's worker stack in debug builds; the
  desktop could not find the `kor` CLI outside an installed bundle.

### Added
- Fork-friendliness and community (§22.2–22.3, ADR 0064): root `justfile` (setup/dev/build/test/lint/fmt/e2e/
  release/oss-check/rebrand), `ARCHITECTURE.md`, a crate `README.md` in every crate, `.devcontainer/`, `flake.nix`,
  `branding.toml` with a reversible `just rebrand <Name>`, community files at the repository root, issue forms,
  labels, `just oss-check` in CI.
- Installers (§24.1, §24.4): WebView2 bootstrapper embedded (offline / fixed runtime on request), VC++ runtime
  bundled, the installers put `kor` on PATH and register the Chrome native host (`kor install-hooks`), portable
  Windows `.zip`, AUR `PKGBUILD`, readable `install.sh` with checksum and minisign checks.
- `kor prereq list|install` and the desktop "System prerequisites" view (§24.3): Docker/Podman, WSL2, Xcode
  tools, WebView2 and macOS permissions through winget/choco/brew/apt/dnf/pacman/zypper/apk/nix, with the exact
  command shown and the user's consent; `kor doctor` lists them.
- Clean-machine install tests (§24.5): every package installed on fresh Windows, macOS 14/15, Ubuntu, Fedora,
  Arch and Alpine runners, smoke-tested online and with the network cut.
- Release flow with cargo-dist (Windows, macOS, Linux glibc/musl, x64 and arm64), Sigstore provenance, and
  minisign-signed raw binaries for self-update. CI moved to the repository root; `kor --version` and `kor doctor`
  run on every platform without swallowed errors; desktop crate, musl and Intel macOS jobs added.
- `kor update` (`check`, `channel stable|beta|nightly`, `rollback`, `--from <file>`): signature-verified,
  atomic replacement, daily background check in interactive sessions, nothing in privacy mode.
- `kor undo`: per-session journal of every file the agent changes; one command reverts the session.
- `--privacy-mode` / `privacy.mode`: local providers only; web, MCP, browser, SSH and catalog refresh off.
- `kor config set`, `computer.enabled` (desktop control; on by default since 0.8, per-app tiers still apply), `kor-platform` crate with a
  ratchet test against OS-specific code elsewhere.
- CLI ↔ TUI ↔ desktop parity manifest (`parity.toml`) checked in CI; every CLI command also runs as `/command`
  in the interactive screen; new `kor agents|memory|connections|cost|routines` and `kor mcp add|remove|enable|disable`.
- Plugins: installed plugins' commands, agents and skills load into the agent (hooks do not).

### Fixed
- CLI plugins installed into `~/.claude/plugins` (another tool's folder); now `~/.kor/plugins`.
- Slash command output was invisible in the full-screen TUI.
- Editing an external MCP server after disabling it lost its extra fields.

### Added (earlier)
- Cargo workspace (edition 2024, MSRV 1.88); CI on Linux, macOS and Windows (x86_64 and
  arm64) with `cargo-deny` and `cargo-audit` gates.
- ADR-0001: overall architecture.
- `kor-core`: messages, content parts, tool specs, model references, privacy tiers,
  usage and cost, stream events, normalized provider errors.
- `kor-config`: layered TOML configuration with per-key provenance.
- `kor-secrets`: OS keychain store, env-first key resolution, masking, redaction.
- `kor-i18n`: Fluent-based Turkish, English and French with English fallback and a key
  integrity test.
- `kor` CLI: `doctor`, `auth login|status|logout`, `config explain|paths`, `--lang`.
- `kor-providers`: `Provider` trait, streaming SSE/NDJSON decoder with cancellation,
  normalized and redacted provider errors.
- Adapters: `openai_compat`, `openai` (Responses API, `store: false`), `anthropic`, `gemini`, `ollama`.
- Provider manifests for anthropic, openai, google, local (Ollama) and venice; user overrides
  in `<config dir>/kor/providers/`.
- Model catalog cache (24 hours, `--refresh`, `--offline`).
- `kor providers`, `kor models`, `kor chat`.
- `StreamEvent::ThinkingSignature`.
- `kor-tools`: `read_file`, `write_file`, `edit`, `list`, `glob`, `grep`, `bash` with project-root
  confinement, atomic writes, output limits, timeouts and API-key scrubbing for commands.
- `kor-sandbox`: bubblewrap (Linux) and Seatbelt (macOS) command wrapping.
- `kor-agent`: agent loop, permission rules and modes
  (`default`, `plan`, `accept-edits`, `auto`), dangerous-command guard, system prompt
  with `AGENTS.md` / `CLAUDE.md` / `KOR.md`, JSONL sessions.
- `kor` interactive REPL, `kor "prompt"`, `kor -p`, `kor resume`, `kor sessions`, `--model`, `--mode`.
- Config: `agent.model`, `agent.max_turns`, `agent.max_tokens`, `[permissions]`.
- `kor-compat`: reads `CLAUDE.md` with `@imports`, `.claude/settings*.json`,
  rules, commands, skills, and agents, plus Codex (`AGENTS.md` hierarchy, `config.toml`, prompts,
  `[model_providers]`) and Cursor (`.cursorrules`, `.cursor/rules/*.mdc`, commands, `.cursorignore`)
  in place; golden-file tests.
- Workspace trust: `kor trust`, `kor trust --revoke`.
- `kor compat status [--all]`.
- Hooks: `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`,
  `SessionStart`, `SessionEnd`, `Notification`.
- Custom slash commands from `CLAUDE.md` projects, Cursor, Codex, and Kor; skills via the `skill` tool or `/name`.
- Glob-scoped rules attached to the first matching file the agent reads or edits.
- `.korignore`, `.cursorignore`, `.aiderignore` hide paths from all tools.

- Model-agnostic tool calling: text tool protocol (Hermes/Qwen, Llama, Mistral, fenced JSON),
  automatic fallback when a provider rejects native tools, `agent.tool_mode`.
- Foreign tool names and argument names are routed to Kor tools; Anthropic text-editor commands.
- `apply_patch` tool (Codex V4A patch format).
- Tool results can carry images; screenshots reach the model.
- `WebSearch` with Tavily, Brave, Exa, Perplexity, SearXNG and keyless DuckDuckGo backends.
- Desktop control on Windows, macOS and Linux via `enigo`/`xcap`; `right_click`, `double_click`.
- `RunElevated`: one command with administrator/root rights through the OS consent prompt.
- Brand kit in the CLI (gradient banner, ember spinner with TR/EN/FR verbs) and desktop renderer.

### Security
- Project-level `.kor/config.toml` cannot grant permissions (`allow`, widening `mode`); only restrict.
- Wildcard `Bash(...)` allow rules never match compound commands; deny/ask rules check every segment.

### Changed
- Environment variables are `KOR_*`; the `CLAUDE_CODE_*` names are still read as fallbacks.
- Team memory sync has no default endpoint (was `api.anthropic.com`); set `KOR_TEAM_MEMORY_URL`.
- Plugins install into `~/.kor/plugins`; `.kor-plugin/` and `.claude-plugin/` metadata are both read.
- The MCP server identifies as `kor`.

- Claude Code `defaultMode: bypassPermissions` maps to Kor `auto` instead of bypassing prompts.

### Removed
- Claude Code's "undercover" mode (instructions to hide AI involvement in public repositories).
- The Anthropic-only `WebSearch` server-tool payload.
- `kor` without arguments starts the interactive agent instead of printing a notice.
- `kor auth` reads provider ids and env var names from manifests instead of a static list.
