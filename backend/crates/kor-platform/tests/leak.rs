//! Ratchet for Master Build Prompt §2 rule 2: OS branches (`cfg(windows)`, `cfg!(unix)`,
//! `target_os`…) belong in `kor-platform`. Every other file may keep at most the number recorded in
//! `os-branches.txt`; new files may have none. Move code into `kor-platform` and lower the numbers.
//! After lowering, regenerate with `KOR_UPDATE_OS_BASELINE=1 cargo test -p kor-platform --test leak`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const PATTERNS: &[&str] = &[
    "cfg(windows",
    "cfg!(windows",
    "cfg(unix",
    "cfg!(unix",
    "cfg(target_os",
    "cfg!(target_os",
    "cfg(not(windows",
    "cfg!(not(windows",
    "cfg(not(unix",
    "cfg!(not(unix",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "target" && name != "kor-platform" && name != "node_modules" {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn counts() -> BTreeMap<String, usize> {
    let root = repo();
    let mut files = Vec::new();
    for d in [
        "backend/crates",
        "backend/apps",
        "frontend/src-tauri/src",
        "frontend/src-tauri/build.rs",
    ] {
        let p = root.join(d);
        if p.is_file() {
            files.push(p);
        } else {
            rust_files(&p, &mut files);
        }
    }
    files
        .into_iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(&f).ok()?;
            let n = text
                .lines()
                .filter(|l| PATTERNS.iter().any(|p| l.contains(p)))
                .count();
            let rel = f
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            (n > 0).then_some((rel, n))
        })
        .collect()
}

#[test]
fn os_branches_do_not_spread_outside_kor_platform() {
    let baseline_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("os-branches.txt");
    let now = counts();
    if std::env::var_os("KOR_UPDATE_OS_BASELINE").is_some() {
        let text = now.iter().fold(String::new(), |mut s, (f, n)| {
            use std::fmt::Write as _;
            let _ = writeln!(s, "{n} {f}");
            s
        });
        std::fs::write(&baseline_path, text).unwrap();
        return;
    }
    let baseline: BTreeMap<String, usize> = std::fs::read_to_string(&baseline_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (n, f) = l.split_once(' ')?;
            Some((f.to_owned(), n.parse().ok()?))
        })
        .collect();
    let grown: Vec<String> = now
        .iter()
        .filter(|(f, n)| baseline.get(*f).is_none_or(|b| *n > b))
        .map(|(f, n)| {
            format!(
                "{f}: {n} (allowed {})",
                baseline.get(f).copied().unwrap_or(0)
            )
        })
        .collect();
    assert!(
        grown.is_empty(),
        "OS-specific code outside kor-platform grew; move it into kor-platform:\n{}",
        grown.join("\n")
    );
}
