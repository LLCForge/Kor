//! `kor providers privacy` (spec §31.2, ADR 0057): every provider's privacy card — training use,
//! retention, region, ZDR, sub-processor, KVKK/GDPR note — with the policy page and the date it
//! was read, and whether it meets `privacy.require`. Unknown values print as "bilinmiyor".

use kor_i18n::Localizer;

use crate::Ctx;

#[derive(Debug, clap::Subcommand)]
pub enum ProvidersCmd {
    /// Privacy cards of every provider, as a table.
    Privacy {
        /// Print the cards as JSON.
        #[arg(long)]
        json: bool,
    },
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-providers"))
        .mut_subcommand("privacy", |s| {
            s.about(l.t("cmd-providers-privacy"))
                .mut_arg("json", |a| a.help(l.t("arg-providers-privacy-json")))
        })
}

pub fn run(ctx: &Ctx, cmd: &ProvidersCmd) -> bool {
    let l = &ctx.l;
    // `--privacy-mode` and `privacy.require` from the loaded config (the CLI applies the
    // network policy after loading it; apply again so this command works on its own too).
    if let Ok(c) = &ctx.config {
        kor_runtime::apply_network_policy(l, &c.config);
    }
    match cmd {
        ProvidersCmd::Privacy { json } => {
            let rows = kor_runtime::privacycard::rows(l);
            if *json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows).unwrap_or_default()
                );
            } else {
                print!("{}", kor_runtime::privacycard::table(l, &rows));
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[derive(Debug, clap::Parser)]
    struct T {
        #[command(subcommand)]
        c: Option<ProvidersCmd>,
    }

    #[test]
    fn args_parse() {
        let t = T::try_parse_from(["providers", "privacy", "--json"]).unwrap();
        assert!(matches!(t.c, Some(ProvidersCmd::Privacy { json: true })));
        assert!(T::try_parse_from(["providers"]).unwrap().c.is_none());
    }
}
