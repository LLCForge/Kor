//! Win32 side of [`super::SystemRegistry`]: string values under `HKCU`, key trees, and the
//! association-changed notice for Explorer. Each `unsafe` block is one FFI call with pointers to
//! locals that outlive it; the only handle opened (`exists`) is closed before returning.
#![allow(unsafe_code)]

use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegDeleteTreeW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(i32::try_from(code).unwrap_or(i32::MAX))
}

/// Set `name` (`""`: default value) of `key`, creating the key.
pub(super) fn set(key: &str, name: &str, value: &str) -> io::Result<()> {
    let k = wide(key);
    let n = wide(name);
    let data = wide(value);
    let size = u32::try_from(data.len() * 2).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: NUL-terminated UTF-16 strings that outlive the call; `size` matches `data`.
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            n.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            size,
        )
    };
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(os_error(rc))
    }
}

/// A string value, `None` when missing.
pub(super) fn get(key: &str, name: &str) -> Option<String> {
    let k = wide(key);
    let n = wide(name);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes = 0u32;
    // SAFETY: valid strings; a null buffer asks for the size only.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            n.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &raw mut bytes,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
    let mut bytes = u32::try_from(buf.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: `buf` holds `bytes` bytes and outlives the call.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            n.as_ptr(),
            flags,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub(super) fn exists(key: &str) -> bool {
    let k = wide(key);
    let mut h: HKEY = null_mut();
    // SAFETY: `h` receives a handle that is closed right below.
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_READ, &raw mut h) };
    if rc == ERROR_SUCCESS {
        // SAFETY: `h` was opened above.
        unsafe { RegCloseKey(h) };
        true
    } else {
        false
    }
}

pub(super) fn delete_tree(key: &str) -> io::Result<()> {
    let k = wide(key);
    // SAFETY: valid NUL-terminated string.
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(os_error(rc))
    }
}

pub(super) fn notify() {
    // SAFETY: no item pointers are passed for SHCNE_ASSOCCHANGED.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED.cast_signed(),
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}
