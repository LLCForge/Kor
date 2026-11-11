//! Build Kor's two Windows 11 sparse packages (ADR 0052; used by `release.yml`).
//!
//! `cargo run -p kor-platform --example sparse_package -- <out-dir> <publisher> <version> <arch> <logo.png>`
//!
//! Writes `<out-dir>/Kor.ShellIntegration.msix`, `<out-dir>/Kor.Widgets.msix` (unsigned: CI signs
//! them like the installer) and `<out-dir>/kor-widgets.exe.manifest`, using `makeappx` from the
//! newest installed Windows SDK.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kor_platform::shell_integration::sparse::{
    self, Kind, Manifest, System, menu_manifest, widgets_exe_manifest, widgets_manifest,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, publisher, version, arch, logo] = args.as_slice() else {
        eprintln!("usage: sparse_package <out-dir> <publisher> <version> <x64|arm64> <logo.png>");
        return ExitCode::from(2);
    };
    match run(Path::new(out), publisher, version, arch, Path::new(logo)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sparse_package: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(out: &Path, publisher: &str, version: &str, arch: &str, logo: &Path) -> std::io::Result<()> {
    let m = Manifest {
        publisher,
        version,
        arch,
    };
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let makeappx = sparse::sdk_tool(&kits, "makeappx", "x64")
        .ok_or_else(|| std::io::Error::other("makeappx.exe not found (Windows SDK)"))?;
    std::fs::create_dir_all(out)?;
    for (kind, xml) in [
        (Kind::Menu, menu_manifest(&m)?),
        (Kind::Widgets, widgets_manifest(&m)?),
    ] {
        let stage = out.join(format!("stage-{}", kind.name()));
        let _ = std::fs::remove_dir_all(&stage);
        sparse::stage(&stage, &xml, logo, kind)?;
        sparse::pack(
            &mut System,
            &makeappx,
            &stage,
            &out.join(format!("{}.msix", kind.name())),
        )?;
        std::fs::remove_dir_all(&stage)?;
    }
    std::fs::write(
        out.join("kor-widgets.exe.manifest"),
        widgets_exe_manifest(publisher)?,
    )
}
