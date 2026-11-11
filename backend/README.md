# Kor

Kor ("ember" in Turkish) is a provider-agnostic, local-first, open-source coding agent.
One CLI and one local daemon for standard, uncensored, aggregator, cloud and local model
providers, with zero telemetry and native support for the configuration files of
existing agents (`CLAUDE.md`, `AGENTS.md`, `.cursor/rules`, ...).

## Build

Requires Rust 1.88+.

```bash
cargo build --release
./target/release/kor doctor
```

## Commands available today

| Command | What it does |
|---|---|
| `kor` | Interactive agent (`/help` inside) |
| `kor "prompt"` | Run one task, asking before edits and commands |
| `kor -p "prompt"` | Print mode for scripts: refuses anything that needs approval |
| `kor resume [id]` / `kor sessions` | Continue or list saved sessions |
| `kor providers` / `kor models` | Providers with privacy tier and key status; cached model list |
| `kor chat -m provider/model "..."` | Single streamed reply, no tools |
| `kor doctor` | Checks OS, config, keychain, git and shells |
| `kor auth login <provider>` | Stores an API key in the OS keychain (hidden prompt or stdin) |
| `kor auth status [provider]` | Shows where each key comes from, masked |
| `kor auth logout <provider>` | Removes a stored key |
| `kor config explain` | Every effective setting and the layer that set it |
| `kor config paths` | Config file locations |

Every command takes `--lang tr|en|fr`. Without it Kor uses `KOR_LANG`, then
`ui.language` from config, then the OS locale, then English.

## Configuration

TOML, merged in this order (later wins): defaults, system, user, project
(`.kor/config.toml`, nearest ancestor), environment (`KOR_LANG`, `KOR_<SECTION>__<KEY>`),
command line.

| OS | User config |
|---|---|
| Linux | `$XDG_CONFIG_HOME/kor/config.toml` |
| macOS | `~/Library/Application Support/kor/config.toml` |
| Windows | `%APPDATA%\kor\config.toml` |

```toml
[ui]
language = "tr"          # auto | tr | en | fr

[agent]
reply_language = "auto"  # language the model answers in
model = "venice/deepseek-v4-flash"   # any provider/model listed by `kor models`

[privacy]
min_tier = "no_train"    # unknown | standard | no_train | no_log | local

[permissions]
mode = "default"         # default | plan | accept-edits | auto (auto needs an OS sandbox)
allow = ["Bash(cargo test *)", "Edit(src/**)"]
deny = ["Read(.env)", "Bash(git push *)"]
```

API keys never go in config files. Lookup order: `<PROVIDER>_API_KEY` environment
variable, then the OS keychain.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Contributions require a DCO sign-off; see [CONTRIBUTING.md](../CONTRIBUTING.md).
