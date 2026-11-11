//! Win32 side of [`super`]: the Jump List through `ICustomDestinationList`. Every COM object is an
//! owned `windows` interface (released on drop). The `PROPVARIANT` title owns a `CoTaskMemAlloc` string
//! (`SHStrDupW`): `windows`' `Drop for PROPVARIANT` frees it with `PropVariantClear`.
#![allow(unsafe_code)]

use std::io;
use std::path::Path;

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, SHStrDupW,
    ShellLink,
};
use windows::core::{HSTRING, Interface as _};

use super::{Item, List, windows_args};

fn err(e: &windows::core::Error) -> io::Error {
    io::Error::other(format!("jump list: {e}"))
}

/// COM for this thread, undone on drop when this call initialized it.
struct Com(bool);

impl Com {
    fn init() -> io::Result<Self> {
        // SAFETY: plain call; balanced by CoUninitialize in Drop when it succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self(false));
        }
        hr.ok().map_err(|e| err(&e))?;
        Ok(Self(true))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: matches the successful CoInitializeEx above on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn link(exe: &Path, item: &Item) -> windows::core::Result<IShellLinkW> {
    // SAFETY: COM calls on owned interfaces with HSTRING / local buffers that outlive each call.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetArguments(&HSTRING::from(windows_args(&item.args)))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        link.SetDescription(&HSTRING::from(item.title.as_str()))?;
        let store: IPropertyStore = link.cast()?;
        let title = SHStrDupW(&HSTRING::from(item.title.as_str()))?;
        let value = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_LPWSTR,
                    Anonymous: PROPVARIANT_0_0_0 { pwszVal: title },
                    ..Default::default()
                }),
            },
        };
        let key = PKEY_Title;
        store.SetValue(&raw const key, &raw const value)?;
        store.Commit()?;
        Ok(link)
    }
}

fn collection(exe: &Path, items: &[Item]) -> windows::core::Result<IObjectArray> {
    // SAFETY: COM calls on owned interfaces.
    unsafe {
        let col: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        for it in items {
            col.AddObject(&link(exe, it)?)?;
        }
        col.cast()
    }
}

pub(super) fn apply(exe: &Path, list: &List) -> io::Result<()> {
    let _com = Com::init()?;
    let run = || -> windows::core::Result<()> {
        // SAFETY: COM calls on owned interfaces; `slots` outlives BeginList.
        unsafe {
            let dl: ICustomDestinationList =
                CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
            let mut slots = 0u32;
            let _removed: IObjectArray = dl.BeginList(&raw mut slots)?;
            if !list.recent.is_empty() {
                dl.AppendCategory(
                    &HSTRING::from(list.recent_title.as_str()),
                    &collection(exe, &list.recent)?,
                )?;
            }
            if !list.tasks.is_empty() {
                dl.AddUserTasks(&collection(exe, &list.tasks)?)?;
            }
            dl.CommitList()
        }
    };
    run().map_err(|e| err(&e))
}
