//! Win32 side of [`super`]: the per-user `Path` in `HKCU\Environment`. Read without expanding
//! `%VARS%` and written back as `REG_EXPAND_SZ`, so entries like `%USERPROFILE%\.cargo\bin` survive.
//! Each `unsafe` block is one FFI call with pointers to locals that outlive it; no handles are
//! held (`RegGetValueW` / `RegSetKeyValueW` open and close the key themselves).
#![allow(unsafe_code)]

use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(i32::try_from(code).unwrap_or(i32::MAX))
}

/// The raw user `Path` (empty when unset).
pub(super) fn user_path() -> io::Result<String> {
    let key = wide("Environment");
    let name = wide("Path");
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut bytes = 0u32;
    // SAFETY: valid NUL-terminated strings; a null buffer asks for the size only.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &raw mut bytes,
        )
    };
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(String::new());
    }
    if rc != ERROR_SUCCESS {
        return Err(os_error(rc));
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut bytes = u32::try_from(buf.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: `buf` holds `bytes` bytes and outlives the call.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(os_error(rc));
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..len]))
}

/// Write the user `Path` and tell running programs (Explorer, new terminals) that it changed.
pub(super) fn set_user_path(value: &str) -> io::Result<()> {
    let key = wide("Environment");
    let name = wide("Path");
    let data = wide(value);
    let size = u32::try_from(data.len() * 2).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: `data` is a NUL-terminated UTF-16 string of `size` bytes and outlives the call.
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_EXPAND_SZ,
            data.as_ptr().cast(),
            size,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(os_error(rc));
    }
    let area = wide("Environment");
    let mut result = 0usize;
    // SAFETY: `area` outlives the call; a hung window is skipped after 5 s.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &raw mut result,
        );
    }
    Ok(())
}
