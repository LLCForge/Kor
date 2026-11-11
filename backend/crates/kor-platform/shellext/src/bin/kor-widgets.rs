//! `kor-widgets.exe`: Kor's Windows 11 Widgets board provider (§27.10, ADR 0052).
//!
//! Started by COM through the sparse package's `ExeServer` entry with
//! `-RegisterProcessAsComServer`; registers the provider class and runs until the Widgets host
//! releases it. Started any other way it exits at once.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    if !std::env::args().any(|a| a.eq_ignore_ascii_case("-RegisterProcessAsComServer")) {
        return std::process::ExitCode::SUCCESS;
    }
    match kor_shellext::widgets::serve() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("kor-widgets: the Widgets board exists only on Windows 11");
}
