//! `kor-lint [--baseline FILE] [--update] PATH...` (§32.2, ADR 0062). Run from `backend/` so the
//! paths in the baseline stay stable. Exit 1 when a file gains findings over the baseline.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut baseline: Option<PathBuf> = None;
    let mut update = false;
    let mut roots = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--baseline" => baseline = args.next().map(PathBuf::from),
            "--update" => update = true,
            "-h" | "--help" => {
                println!("kor-lint [--baseline FILE] [--update] PATH...");
                return ExitCode::SUCCESS;
            }
            _ => roots.push(PathBuf::from(a)),
        }
    }
    if roots.is_empty() {
        eprintln!("kor-lint: no path given");
        return ExitCode::from(2);
    }
    let (findings, errors) = kor_lint::scan_paths(&roots);
    for e in &errors {
        eprintln!("kor-lint: skipped (does not parse): {e}");
    }
    let Some(path) = baseline else {
        for f in &findings {
            println!("{f}");
        }
        println!("{} finding(s)", findings.len());
        return if findings.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    };
    if update {
        let text = kor_lint::render_baseline(&kor_lint::tally(&findings));
        return match std::fs::write(&path, text) {
            Ok(()) => {
                println!(
                    "kor-lint: baseline written ({} finding(s)) to {}",
                    findings.len(),
                    path.display()
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("kor-lint: {}: {e}", path.display());
                ExitCode::from(2)
            }
        };
    }
    let base = match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| kor_lint::parse_baseline(&t))
    {
        Ok(b) => b,
        Err(e) => {
            eprintln!("kor-lint: baseline {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let (worse, better) = kor_lint::compare(&base, &findings);
    for (rule, file, now, allowed) in &better {
        println!(
            "kor-lint: {file}: {rule} down to {now} (baseline {allowed}); run with --update to lock it in"
        );
    }
    if worse.is_empty() {
        println!(
            "kor-lint: ok ({} known finding(s), none new)",
            findings.len()
        );
        return ExitCode::SUCCESS;
    }
    for (rule, file, now, allowed) in &worse {
        eprintln!("kor-lint: {file}: {rule} {now} > baseline {allowed}");
        for f in findings
            .iter()
            .filter(|f| f.file == *file && f.rule.id() == rule)
        {
            eprintln!("  {f}");
        }
    }
    eprintln!("kor-lint: new findings; fix them or add `// kor-lint: allow(<rule>) <why>`");
    ExitCode::FAILURE
}
