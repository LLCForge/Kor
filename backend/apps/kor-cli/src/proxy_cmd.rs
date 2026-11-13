//! `kor proxy serve|status|key` (§4.3.3): the embedded CLIProxyAPI-compatible gateway. All
//! logic is in `kor_proxy`; this module parses flags and prints.

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ProxyAuthCmd {
    /// Sign in with a subscription (Claude Code, Codex, Copilot, …) and save an auth file.
    Login {
        /// Provider name: claude, codex, codex-device, antigravity, kimi, kimi-ai, devin, copilot.
        provider: String,
    },
    /// List saved subscription auth files.
    List,
    /// Refresh tokens in one auth file (basename under the auth dir).
    Refresh { file: String },
    /// Remove one saved auth file.
    Remove { file: String },
}

#[derive(Debug, Subcommand)]
pub enum ProxyCmd {
    /// Serve OpenAI, Anthropic and Gemini compatible endpoints until Ctrl+C.
    Serve {
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        host: Option<String>,
        /// A `proxy.yaml` or an existing CLIProxyAPI `config.yaml`.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        no_auth: bool,
        /// Keep serving in the background of this session (the interactive screen's
        /// `/proxy serve --background`) instead of waiting for Ctrl+C.
        #[arg(long)]
        background: bool,
    },
    /// Usage of a running proxy.
    Status {
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        host: Option<String>,
    },
    /// The proxy access key, masked; `--rotate` replaces it, `--show` prints it in full.
    Key {
        #[arg(long, conflicts_with = "show")]
        rotate: bool,
        #[arg(long)]
        show: bool,
    },
    /// Subscription OAuth accounts used by the proxy credential pool.
    #[command(subcommand)]
    Auth(ProxyAuthCmd),
}

// BoringSSL (wreq, kor-proxy's TLS client) is not built for static musl or Windows ARM64.
#[cfg(kor_proxy)]
mod gateway;
#[cfg(kor_proxy)]
pub use gateway::run;

/// This build has no gateway: say so instead of failing to start.
#[cfg(not(kor_proxy))]
pub fn run(ctx: &crate::Ctx, _cmd: ProxyCmd) -> anyhow::Result<bool> {
    Err(anyhow::anyhow!(ctx.l.t("proxy-not-in-build")))
}
