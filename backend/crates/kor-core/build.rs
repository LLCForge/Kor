//! Exposes the product version (`[workspace.package] version` in backend/Cargo.toml) as
//! `KOR_VERSION`, so every crate reports the product version, not its own component version.

use std::{env, fs, path::Path};

fn main() {
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
        .join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", root.display());
    let text = fs::read_to_string(&root).expect("backend/Cargo.toml is readable");
    let mut in_section = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == "[workspace.package]";
        } else if in_section && let Some(v) = line.strip_prefix("version") {
            let v = v
                .trim_start()
                .trim_start_matches('=')
                .trim()
                .trim_matches('"');
            println!("cargo:rustc-env=KOR_VERSION={v}");
            return;
        }
    }
    panic!("no version in [workspace.package] of {}", root.display());
}
