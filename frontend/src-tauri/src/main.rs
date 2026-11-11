#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if kor_desktop_lib::terminal_only() {
        return;
    }
    kor_desktop_lib::run();
}
