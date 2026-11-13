//! `kor install-hooks post-install|pre-uninstall` (hidden; spec §24.4, ADR 0064): what the desktop
//! installers do through the bundled CLI so every package does the same thing. The NSIS installer
//! (per user) passes `--path` to put `kor` on the user PATH (`kor_platform::cli_path`, the same
//! reversible record the desktop's consent flow uses); the MSI sets the machine PATH itself and
//! passes `--system`. Both register the Chrome native messaging host for the extension ids in
//! `branding.toml`. `pre-uninstall` undoes exactly that and, in a user's context (no `--system`),
//! also removes the context-menu entries that user's Kor added (§25.4).

use clap::{Args, Subcommand};
use kor_i18n::args;
use kor_platform::native_host::{self, Scope};

use crate::Ctx;

#[derive(Debug, Subcommand)]
pub enum HookCmd {
    PostInstall(HookArgs),
    PreUninstall(HookArgs),
}

#[derive(Debug, Args)]
pub struct HookArgs {
    /// Machine-wide install (MSI, deb, rpm): native host for every user.
    #[arg(long)]
    pub system: bool,
    /// Add (or remove) this `kor` on the user PATH.
    #[arg(long)]
    pub path: bool,
}

pub fn run(ctx: &Ctx, cmd: &HookCmd) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let exe = std::env::current_exe()?;
    let brand = kor_platform::brand();
    let (install, a) = match cmd {
        HookCmd::PostInstall(a) => (true, a),
        HookCmd::PreUninstall(a) => (false, a),
    };
    let scope = if a.system { Scope::System } else { Scope::User };
    let mut ok = true;
    let mut fail = |what: &str, e: &dyn std::fmt::Display| {
        ok = false;
        eprintln!(
            "{}",
            l.t_args(
                "install-hooks-failed",
                &args!("what" => what.to_owned(), "error" => e.to_string())
            )
        );
    };
    if a.path {
        let r = if install {
            kor_platform::cli_path::install(&exe)
        } else {
            kor_platform::cli_path::remove(&exe)
        };
        if let Err(e) = r {
            fail("PATH", &e);
        }
    }
    let host = &brand.chrome.native_host;
    let r = if install {
        native_host::register(
            scope,
            host,
            &brand.product.description,
            &exe,
            &brand.chrome.extension_ids,
        )
        .map(drop)
    } else {
        native_host::unregister(scope, host)
    };
    if let Err(e) = r {
        fail("native messaging host", &e);
    }
    // §25.4: uninstalling leaves no "Open with Kor" entries behind. They belong to the user who
    // installed them (the app's consent flow / `kor integrate install`), so this runs in that user's
    // context: NSIS per user, the MSI's impersonated action. Nothing recorded → nothing to do.
    if !install
        && !a.system
        && let Err(e) = kor_platform::shell_integration::uninstall()
    {
        fail("context menu", &e);
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use clap::{Parser, Subcommand};

    use super::*;

    #[derive(Parser)]
    #[command(name = "k")]
    struct Cli {
        #[command(subcommand)]
        cmd: Top,
    }

    #[derive(Subcommand)]
    enum Top {
        InstallHooks {
            #[command(subcommand)]
            cmd: HookCmd,
        },
    }

    #[test]
    fn parses() {
        let Top::InstallHooks {
            cmd: HookCmd::PostInstall(a),
        } = Cli::try_parse_from(["k", "install-hooks", "post-install", "--path"])
            .unwrap()
            .cmd
        else {
            panic!()
        };
        assert!(a.path && !a.system);
        assert!(matches!(
            Cli::try_parse_from(["k", "install-hooks", "pre-uninstall", "--system"])
                .unwrap()
                .cmd,
            Top::InstallHooks {
                cmd: HookCmd::PreUninstall(HookArgs {
                    system: true,
                    path: false
                })
            }
        ));
        assert!(Cli::try_parse_from(["k", "install-hooks", "upgrade"]).is_err());
        let l = kor_i18n::Localizer::new(kor_i18n::Lang::Tr);
        assert_ne!(l.t("install-hooks-failed"), "install-hooks-failed");
    }
}
