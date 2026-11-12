//! `kor export --all [--out <file.zip>]` (Master Build Prompt §22.2): every piece of the user's
//! data in one archive (logic: `kor_runtime::export`).

use std::path::PathBuf;

use clap::Args;
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Everything: sessions, memory, knowledge base, audit log, settings (secrets stay in the keychain).
    #[arg(long, required = true)]
    pub all: bool,
    /// Archive to write (default `kor-export-<date>.zip` in the current folder).
    #[arg(long, short)]
    pub out: Option<PathBuf>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-export"))
        .mut_arg("all", |a| a.help(l.t("arg-export-all")))
        .mut_arg("out", |a| a.help(l.t("arg-export-out")))
}

#[allow(clippy::cast_precision_loss)] // sizes and ratios for display
#[allow(clippy::unnecessary_wraps)] // same signature as the other command runners
pub fn run(ctx: &Ctx, a: &ExportArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let out = a
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(kor_runtime::export::default_name()));
    match kor_runtime::export::export_all(&out, env!("CARGO_PKG_VERSION")) {
        Ok(s) => {
            println!(
                "{}",
                l.t_args(
                    "export-done",
                    &args!("path" => s.path.display().to_string(), "files" => s.files, "mb" => format!("{:.1}", s.bytes as f64 / 1_048_576.0))
                )
            );
            Ok(true)
        }
        Err(e) => {
            eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    #[derive(clap::Parser)]
    struct T {
        #[command(flatten)]
        a: super::ExportArgs,
    }

    #[test]
    fn needs_all_and_takes_an_output_path() {
        assert!(T::try_parse_from(["kor"]).is_err());
        let t = T::try_parse_from(["kor", "--all", "--out", "x.zip"]).unwrap();
        assert!(t.a.all);
        assert_eq!(t.a.out.unwrap(), std::path::PathBuf::from("x.zip"));
    }
}
