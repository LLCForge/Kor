//! `kor comment <files…> --lang tr [--missing] [--write]` (Master Build Prompt §12.2): translate
//! code comments into the target language and, with `--missing`, write one-line doc comments for
//! functions without one. Without `--write` it only prints the diff (logic: `kor_runtime::comment`).

use std::path::PathBuf;

use clap::Args;
use kor_i18n::{Localizer, args};

use crate::Ctx;

#[derive(Debug, Args)]
pub struct CommentArgs {
    /// Source files (Rust, Python, TypeScript/JavaScript, Go, Java, C#, C/C++, PHP, Ruby).
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Target language (`tr`, `en`, `fr`, `de`… or a language name).
    #[arg(long)]
    pub lang: String,
    /// Also write a one-line comment above functions that have none.
    #[arg(long)]
    pub missing: bool,
    /// Only add missing comments; leave existing ones as they are.
    #[arg(long, requires = "missing")]
    pub only_missing: bool,
    /// Write the changes into the files (default: show the diff only).
    #[arg(long)]
    pub write: bool,
    #[arg(short, long)]
    pub model: Option<String>,
}

pub fn localize(c: clap::Command, l: &Localizer) -> clap::Command {
    c.about(l.t("cmd-comment"))
        .mut_arg("files", |a| a.help(l.t("arg-comment-files")))
        .mut_arg("lang", |a| a.help(l.t("arg-comment-lang")))
        .mut_arg("missing", |a| a.help(l.t("arg-comment-missing")))
        .mut_arg("only_missing", |a| a.help(l.t("arg-comment-only-missing")))
        .mut_arg("write", |a| a.help(l.t("arg-comment-write")))
}

pub fn run(ctx: &Ctx, a: &CommentArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let mut ok = true;
    let mut jobs = Vec::new();
    for f in &a.files {
        let lang = f.extension().and_then(|e| e.to_str()).and_then(kor_graph::lang::Lang::from_ext);
        match (lang, std::fs::read_to_string(f)) {
            (Some(lang), Ok(src)) => jobs.push((f, lang, src)),
            (None, _) => {
                eprintln!("{}", l.t_args("comment-unsupported", &args!("file" => f.display().to_string())));
                ok = false;
            }
            (_, Err(e)) => {
                eprintln!("{}", l.t_args("error-generic", &args!("error" => format!("{}: {e}", f.display()))));
                ok = false;
            }
        }
    }
    if jobs.is_empty() {
        return Ok(false);
    }
    let Some((provider, model)) = crate::pr_cmd::model(ctx, a.model.as_deref()) else {
        return Ok(false);
    };
    let rt = crate::providers::runtime()?;
    for (file, lang, src) in jobs {
        let out = rt.block_on(kor_runtime::comment::run(&provider, &model, lang, &src, &a.lang, !a.only_missing, a.missing));
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{}", l.t_args("error-generic", &args!("error" => e)));
                ok = false;
                continue;
            }
        };
        let name = file.display().to_string();
        if out.text == src {
            println!("{}", l.t_args("comment-unchanged", &args!("file" => name)));
            continue;
        }
        if a.write {
            if let Err(e) = std::fs::write(file, &out.text) {
                eprintln!("{}", l.t_args("error-generic", &args!("error" => format!("{name}: {e}"))));
                ok = false;
                continue;
            }
        } else {
            print!("{}", kor_agent::port::unified_diff(&name, &src, &out.text));
        }
        println!(
            "{}",
            l.t_args(
                if a.write { "comment-written" } else { "comment-preview" },
                &args!("file" => name, "translated" => out.translated, "added" => out.added, "kept" => out.kept)
            )
        );
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    #[derive(clap::Parser)]
    struct T {
        #[command(flatten)]
        a: super::CommentArgs,
    }

    #[test]
    fn needs_files_and_a_language() {
        assert!(T::try_parse_from(["kor", "--lang", "tr"]).is_err());
        assert!(T::try_parse_from(["kor", "a.rs"]).is_err());
        assert!(T::try_parse_from(["kor", "a.rs", "--lang", "tr", "--only-missing"]).is_err(), "--only-missing needs --missing");
        let t = T::try_parse_from(["kor", "a.rs", "b.py", "--lang", "tr", "--missing", "--write"]).unwrap();
        assert_eq!(t.a.files.len(), 2);
        assert!(t.a.missing && t.a.write && !t.a.only_missing);
    }
}
