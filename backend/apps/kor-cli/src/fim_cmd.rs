//! `kor fim <file> --line N [--col C] [--write]` (§4.2): fill-in-the-middle with Mistral
//! Codestral at a cursor position. Logic in `kor_runtime::fim`.

use clap::Args;
use kor_i18n::args;

use crate::Ctx;

#[derive(Debug, Args)]
pub struct FimArgs {
    /// File inside this project.
    pub file: String,
    /// 1-based line of the cursor.
    #[arg(long)]
    pub line: usize,
    /// 1-based column (default: end of the line).
    #[arg(long)]
    pub col: Option<usize>,
    /// Insert the completion into the file instead of printing it.
    #[arg(long)]
    pub write: bool,
}

pub fn run(ctx: &Ctx, a: &FimArgs) -> anyhow::Result<bool> {
    let l = &ctx.l;
    let root = std::env::current_dir()?;
    let cursor = match kor_runtime::fim::locate(l, &root, &a.file, a.line, a.col) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return Ok(false);
        }
    };
    let fill = match crate::providers::runtime()?.block_on(kor_runtime::fim::complete(l, &cursor)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return Ok(false);
        }
    };
    if a.write {
        kor_runtime::fim::insert(&cursor, &fill)?;
        println!(
            "{}",
            l.t_args("fim-written", &args!("file" => a.file.clone()))
        );
    } else {
        print!("{fill}");
    }
    Ok(true)
}
