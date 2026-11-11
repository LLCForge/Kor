//! Win32 side of [`super`]: default values of `HKCU` keys. Each `unsafe` block is one FFI call
//! with pointers to locals that outlive it; no handles are held.
#![allow(unsafe_code)]

use std::io;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(i32::try_from(code).unwrap_or(i32::MAX))
}

pub(super) fn set_default(key: &str, value: &str) -> io::Result<()> {
    let k = wide(key);
    let data = wide(value);
    let size = u32::try_from(data.len() * 2).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: NUL-terminated strings that outlive the call; a null value name is the default value.
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            null(),
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

pub(super) fn get_default(key: &str) -> io::Result<Option<String>> {
    let k = wide(key);
    let mut buf = vec![0u16; 1024];
    let mut bytes = u32::try_from(buf.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: `buf` holds `bytes` bytes and outlives the call.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            null(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if rc != ERROR_SUCCESS {
        return Err(os_error(rc));
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(Some(String::from_utf16_lossy(&buf[..len])))
}

pub(super) fn delete(key: &str) -> io::Result<bool> {
    let k = wide(key);
    // SAFETY: NUL-terminated string that outlives the call.
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    match rc {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        _ => Err(os_error(rc)),
    }
}
