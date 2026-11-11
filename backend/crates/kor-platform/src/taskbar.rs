//! Taskbar overlay badge (§27.9, ADR 0051). Windows draws a small coloured dot over the app's
//! taskbar button (`ITaskbarList3::SetOverlayIcon`); macOS and Linux show a number badge through
//! the window system instead (the desktop app sets it), so this is a no-op there.
//!
//! The progress bar itself is Tauri's `set_progress_bar` on every platform.

/// RGBA pixels of a `size`×`size` anti-aliased filled circle in `color`.
pub fn dot_rgba(size: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    #[allow(clippy::cast_precision_loss)] // icon sizes are tiny
    let (r, c) = (size as f32 / 2.0, size as f32 / 2.0);
    for y in 0..size {
        for x in 0..size {
            #[allow(clippy::cast_precision_loss)]
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let cover = (r - d + 0.5).clamp(0.0, 1.0);
            if cover > 0.0 {
                let i = ((y * size + x) * 4) as usize;
                out[i..i + 3].copy_from_slice(&color[..3]);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    out[i + 3] = (f32::from(color[3]) * cover).round() as u8;
                }
            }
        }
    }
    out
}

/// Set (`Some(color)`) or clear (`None`) the overlay dot of the window `hwnd` (a Win32 `HWND`
/// as an integer; ignored elsewhere). Call on the UI thread.
pub fn set_overlay(hwnd: isize, color: Option<[u8; 4]>, description: &str) -> Result<(), String> {
    imp(hwnd, color, description)
}

#[cfg(windows)]
#[allow(unsafe_code)] // COM + GDI calls on handles this function owns
fn imp(hwnd: isize, color: Option<[u8; 4]>, description: &str) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, DestroyIcon, HICON, ICONINFO,
    };
    use windows::core::HSTRING;

    const SIZE: u32 = 16;
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: plain COM/GDI calls; every handle created here is released before returning.
    unsafe {
        // Already initialised on the UI thread (S_FALSE) is fine; a different mode is reported
        // by CoCreateInstance below.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        list.HrInit().map_err(|e| e.to_string())?;
        let Some(color) = color else {
            return list
                .SetOverlayIcon(hwnd, HICON::default(), &HSTRING::new())
                .map_err(|e| e.to_string());
        };
        // GDI wants BGRA, premultiplied for the alpha channel.
        let mut bgra = dot_rgba(SIZE, color);
        for px in bgra.chunks_exact_mut(4) {
            let a = u16::from(px[3]);
            let (r, g, b) = (u16::from(px[0]), u16::from(px[1]), u16::from(px[2]));
            #[allow(clippy::cast_possible_truncation)]
            {
                px[0] = (b * a / 255) as u8;
                px[1] = (g * a / 255) as u8;
                px[2] = (r * a / 255) as u8;
            }
        }
        #[allow(clippy::cast_possible_wrap)]
        let side = SIZE as i32;
        let color_bm = CreateBitmap(side, side, 1, 32, Some(bgra.as_ptr().cast()));
        let mask = vec![0u8; (SIZE * SIZE / 8) as usize];
        let mask_bm = CreateBitmap(side, side, 1, 1, Some(mask.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bm,
            hbmColor: color_bm,
        };
        let icon = CreateIconIndirect(&raw const info);
        let _ = DeleteObject(color_bm.into());
        let _ = DeleteObject(mask_bm.into());
        let icon = icon.map_err(|e| e.to_string())?;
        let r = list
            .SetOverlayIcon(hwnd, icon, &HSTRING::from(description))
            .map_err(|e| e.to_string());
        let _ = DestroyIcon(icon);
        r
    }
}

#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn imp(_hwnd: isize, _color: Option<[u8; 4]>, _description: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_is_round_and_coloured() {
        let px = dot_rgba(16, [255, 160, 0, 255]);
        assert_eq!(px.len(), 16 * 16 * 4);
        let at = |x: usize, y: usize| &px[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(at(8, 8), &[255, 160, 0, 255]);
        assert_eq!(at(0, 0)[3], 0, "corner is transparent");
        assert!(at(0, 8)[3] > 0 && at(0, 8)[3] < 255, "edge is anti-aliased");
    }

    #[test]
    fn clearing_without_a_window_does_not_panic() {
        // A null HWND fails on Windows and is a no-op elsewhere; neither panics.
        let _ = set_overlay(0, None, "");
    }
}
