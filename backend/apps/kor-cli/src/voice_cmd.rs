//! `kor voice [--seconds N]` (spec §10, ADR 0048): microphone → local whisper.cpp → task text.
//! No audio leaves the machine. whisper.cpp and its model are verified components, downloaded only
//! after the user agrees. `kor voice` prints the text (`kor "$(kor voice)"`); `/voice` in the
//! interactive session puts it into the prompt.

use std::io::Write as _;

use kor_i18n::{Localizer, args};
use kor_providers::CancellationToken;
use kor_runtime::visual::voice;

use crate::Ctx;

#[derive(Debug, clap::Args)]
pub struct VoiceArgs {
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=i64::from(voice::MAX_SECONDS)))]
    pub seconds: u32,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long, short)]
    pub yes: bool,
}

/// Localized help, for `localized_command` in main.rs.
pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-voice"))
        .mut_arg("seconds", |a| a.help(l.t("arg-voice-seconds")))
        .mut_arg("language", |a| a.help(l.t("arg-voice-language")))
        .mut_arg("yes", |a| a.help(l.t("arg-voice-yes")))
}

fn mb(bytes: u64) -> String {
    // Display only; f64 is exact far beyond any download size.
    #[allow(clippy::cast_precision_loss)]
    let v = bytes as f64 / 1_048_576.0;
    format!("{v:.1} MB")
}

fn fail(l: &Localizer, e: impl Into<String>) -> Option<String> {
    eprintln!("{}", l.t_args("error-generic", &args!("error" => e.into())));
    None
}

/// Make sure whisper.cpp and the model are there (asking before any download), record and
/// transcribe. `None` after printing why not.
pub fn listen(ctx: &Ctx, a: &VoiceArgs) -> Option<String> {
    let l = &ctx.l;
    let rt = match crate::providers::runtime() {
        Ok(rt) => rt,
        Err(e) => return fail(l, e.to_string()),
    };
    let st = voice::status();
    if !st.ready() {
        if st.missing.iter().any(|(_, _, size)| size.is_none()) {
            return fail(l, l.t("voice-no-build"));
        }
        let privacy = ctx.config.as_ref().is_ok_and(|c| c.config.privacy.mode);
        kor_components::set_privacy(privacy);
        let list: Vec<String> = st
            .missing
            .iter()
            .map(|(n, v, s)| format!("{n} {v} ({})", mb(s.unwrap_or(0))))
            .collect();
        if !crate::shell_cmd::confirm(
            l,
            &l.t_args("voice-missing", &args!("list" => list.join(", "))),
            a.yes,
        ) {
            return None;
        }
        let show = |p: kor_components::Progress| {
            let pct = p.total.filter(|t| *t > 0).map_or(0, |t| p.done * 100 / t);
            eprint!("\r{} {pct:>3}% ({})", p.name, mb(p.done));
            let _ = std::io::stderr().flush();
        };
        for (name, _, _) in &st.missing {
            let r = rt.block_on(kor_components::ensure(name, &show));
            eprintln!();
            if let Err(e) = r {
                return fail(l, e);
            }
        }
    }
    eprintln!(
        "{}",
        l.t_args("voice-recording", &args!("seconds" => a.seconds))
    );
    match rt.block_on(voice::listen(
        a.seconds,
        a.language.as_deref(),
        &CancellationToken::new(),
    )) {
        Ok(text) => Some(text),
        Err(e) => fail(l, e),
    }
}

pub fn run(ctx: &Ctx, a: &VoiceArgs) -> bool {
    match listen(ctx, a) {
        Some(text) => {
            println!("{text}");
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        cmd: Sub,
    }

    #[derive(clap::Subcommand)]
    enum Sub {
        Voice(VoiceArgs),
    }

    #[test]
    fn seconds_are_bounded() {
        let Sub::Voice(a) = Cli::try_parse_from(["kor", "voice"]).unwrap().cmd;
        assert_eq!((a.seconds, a.yes), (10, false));
        let Sub::Voice(a) =
            Cli::try_parse_from(["kor", "voice", "--seconds", "30", "--language", "tr", "-y"])
                .unwrap()
                .cmd;
        assert_eq!(
            (a.seconds, a.language.as_deref(), a.yes),
            (30, Some("tr"), true)
        );
        assert!(Cli::try_parse_from(["kor", "voice", "--seconds", "0"]).is_err());
        assert!(Cli::try_parse_from(["kor", "voice", "--seconds", "121"]).is_err());
    }
}
