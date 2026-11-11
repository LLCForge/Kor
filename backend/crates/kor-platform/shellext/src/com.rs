//! The `IExplorerCommand` COM server Explorer loads for the Windows 11 context menu.
//!
//! One root command "Kor" with a submenu ([`crate::menu::Verb`]); each child hides itself when it
//! does not apply to the selection. Every `unsafe` block is one COM/Win32 call on interfaces the
//! shell handed us or on locals that outlive it.
#![allow(unsafe_code)]
// `#[implement]` expansions trip these pedantic lints; the code is the windows crate's.
#![allow(clippy::inline_always, clippy::ref_as_ptr)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER, HMODULE,
    S_FALSE, S_OK,
};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECF_HASSUBCOMMANDS, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand,
    IEnumExplorerCommand_Impl, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SHStrDupW,
    SIGDN_FILESYSPATH,
};
use windows_core::{
    BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement,
};

use crate::menu::{self, Lang, Selection, Verb};

fn clsid() -> GUID {
    GUID::try_from(crate::SHELL_CLSID).unwrap_or_default()
}

/// Folder holding this DLL.
fn module_dir() -> Option<PathBuf> {
    let mut module = HMODULE::default();
    // SAFETY: the address of a function in this DLL identifies it; no reference count is taken.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR((module_dir as *const ()).cast::<u16>()),
            &raw mut module,
        )
        .ok()?;
    }
    let mut buf = vec![0u16; 32_768];
    // SAFETY: `buf` outlives the call; the length is its capacity.
    let n = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    if n == 0 {
        return None;
    }
    PathBuf::from(String::from_utf16_lossy(&buf[..n]))
        .parent()
        .map(std::path::Path::to_path_buf)
}

/// `ui.language` and `shell.context_menu.extended` from Kor's config, read on each use so a
/// settings change applies without restarting Explorer.
fn settings() -> (Lang, bool) {
    let text = menu::user_config()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let (lang, extended) = menu::settings(&text);
    // SAFETY: no arguments.
    let os = unsafe { GetUserDefaultUILanguage() };
    (Lang::pick(lang.as_deref(), os), extended)
}

fn dup(s: &str) -> Result<PWSTR> {
    // SAFETY: the shell frees the returned CoTaskMem string.
    unsafe { SHStrDupW(&HSTRING::from(s)) }
}

/// File-system paths of the selection (at most [`crate::MAX_PATHS`]).
fn paths(items: &Ref<IShellItemArray>) -> Result<Vec<PathBuf>> {
    let Some(array) = items.as_ref() else {
        return Ok(Vec::new());
    };
    // SAFETY: `array` is a live interface from the shell.
    let count = unsafe { array.GetCount()? };
    let mut out = Vec::new();
    for i in 0..count.min(u32::try_from(crate::MAX_PATHS).unwrap_or(u32::MAX)) {
        // SAFETY: `i < count`; the returned string is freed right after copying.
        unsafe {
            let item = array.GetItemAt(i)?;
            // Virtual items (Control Panel, libraries' roots) have no path: skip them.
            let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
                continue;
            };
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const c_void));
            out.push(PathBuf::from(
                path.map_err(|_| windows_core::Error::from(E_FAIL))?,
            ));
        }
    }
    Ok(out)
}

fn selection(paths: &[PathBuf]) -> Selection {
    Selection {
        count: paths.len(),
        files: paths.iter().filter(|p| !p.is_dir()).count(),
    }
}

/// The root "Kor" entry (`verb == None`) or one of its children.
#[implement(IExplorerCommand)]
struct Command {
    verb: Option<Verb>,
}

impl IExplorerCommand_Impl for Command_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match self.verb {
            None => dup("Kor"),
            Some(v) => dup(v.title(settings().0)),
        }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        let dir = module_dir().ok_or_else(|| windows_core::Error::from(E_FAIL))?;
        let exe = menu::desktop_candidates(&dir)
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| windows_core::Error::from(E_FAIL))?;
        dup(&format!("{},0", exe.display()))
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        let mut g = clsid();
        // Children: the CLSID with the last byte replaced by the verb's index.
        if let Some(v) = self.verb {
            g.data4[7] = v.index();
        }
        Ok(g)
    }

    fn GetState(&self, items: Ref<IShellItemArray>, _slow: BOOL) -> Result<u32> {
        let sel = selection(&paths(&items)?);
        let (_, extended) = settings();
        let shown = match self.verb {
            None => !menu::entries(sel, extended).is_empty(),
            Some(v) => v.visible(sel, extended),
        };
        Ok(u32::try_from(if shown { ECS_ENABLED.0 } else { ECS_HIDDEN.0 }).unwrap_or(0))
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _ctx: Ref<IBindCtx>) -> Result<()> {
        let Some(verb) = self.verb else {
            return Ok(());
        };
        let paths = paths(&items)?;
        if paths.is_empty() {
            return Ok(());
        }
        let dir = module_dir().ok_or_else(|| windows_core::Error::from(E_FAIL))?;
        crate::launch_desktop(&dir, &verb.args(&paths)).map_err(|_| E_FAIL.into())
    }

    fn GetFlags(&self) -> Result<u32> {
        let f = if self.verb.is_none() {
            ECF_HASSUBCOMMANDS
        } else {
            ECF_DEFAULT
        };
        Ok(u32::try_from(f.0).unwrap_or(0))
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        if self.verb.is_some() {
            return Err(E_NOTIMPL.into());
        }
        Ok(Enum::new(0).into())
    }
}

/// Children of the root, in menu order; each hides itself when it does not apply.
#[implement(IEnumExplorerCommand)]
struct Enum {
    pos: AtomicUsize,
}

impl Enum {
    fn new(pos: usize) -> Self {
        Self {
            pos: AtomicUsize::new(pos),
        }
    }
}

impl IEnumExplorerCommand_Impl for Enum_Impl {
    fn Next(&self, celt: u32, out: *mut Option<IExplorerCommand>, fetched: *mut u32) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        let mut n = 0u32;
        while n < celt {
            let i = self.pos.fetch_add(1, Ordering::SeqCst);
            let Some(&verb) = Verb::ALL.get(i) else {
                self.pos.store(Verb::ALL.len(), Ordering::SeqCst);
                break;
            };
            let cmd: IExplorerCommand = Command { verb: Some(verb) }.into();
            // SAFETY: the caller provides room for `celt` entries.
            unsafe { out.add(n as usize).write(Some(cmd)) };
            n += 1;
        }
        if !fetched.is_null() {
            // SAFETY: checked non-null; caller-owned.
            unsafe { fetched.write(n) };
        }
        if n == celt { S_OK } else { S_FALSE }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        self.pos.fetch_add(celt as usize, Ordering::SeqCst);
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.pos.store(0, Ordering::SeqCst);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        Ok(Enum::new(self.pos.load(Ordering::SeqCst)).into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: checked non-null.
        unsafe { object.write(std::ptr::null_mut()) };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let cmd: IExplorerCommand = Command { verb: None }.into();
        // SAFETY: `iid` and `object` were checked; `query` AddRefs on success.
        unsafe { cmd.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM entry point: the class factory for [`crate::SHELL_CLSID`].
///
/// # Safety
/// Called by COM with valid pointers (each is still checked for null).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid_in: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid_in.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }
    // SAFETY: checked non-null above.
    unsafe {
        object.write(std::ptr::null_mut());
        if *clsid_in != clsid() {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, object)
    }
}

/// COM asks whether the DLL may be unloaded.
// shortcut: never unloads (S_FALSE) instead of counting live objects; dllhost exits on its own.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_factory_builds_a_root_command_with_all_children() {
        let mut ptr = std::ptr::null_mut();
        let iid = IClassFactory::IID;
        let clsid_v = clsid();
        // SAFETY: valid pointers to locals.
        let hr = unsafe { DllGetClassObject(&raw const clsid_v, &raw const iid, &raw mut ptr) };
        assert_eq!(hr, S_OK);
        // SAFETY: `ptr` is an AddRef'd IClassFactory.
        let factory = unsafe { IClassFactory::from_raw(ptr) };
        // SAFETY: COM call on a live interface.
        let root: IExplorerCommand = unsafe { factory.CreateInstance(None) }.unwrap();
        // SAFETY: as above.
        unsafe {
            assert_eq!(
                root.GetFlags().unwrap(),
                u32::try_from(ECF_HASSUBCOMMANDS.0).unwrap()
            );
            assert_eq!(root.GetCanonicalName().unwrap(), clsid());
            let title = root.GetTitle(None).unwrap();
            assert_eq!(title.to_string().unwrap(), "Kor");
            CoTaskMemFree(Some(title.0 as *const c_void));
            assert_eq!(
                root.GetState(None, false).unwrap(),
                u32::try_from(ECS_HIDDEN.0).unwrap()
            );
            let e = root.EnumSubCommands().unwrap();
            let mut got: [Option<IExplorerCommand>; 7] = Default::default();
            let mut n = 0u32;
            assert_eq!(e.Next(&mut got, Some(&raw mut n)), S_FALSE);
            assert_eq!(n as usize, Verb::ALL.len());
            let names: Vec<u8> = got[..6]
                .iter()
                .map(|c| c.as_ref().unwrap().GetCanonicalName().unwrap().data4[7])
                .collect();
            assert_eq!(names, [1, 2, 3, 4, 5, 6]);
            e.Reset().unwrap();
            let c = e.Clone().unwrap();
            let mut one: [Option<IExplorerCommand>; 1] = [None];
            assert_eq!(c.Next(&mut one, None), S_OK);
            // Nothing selected: the child is hidden and invoking it does nothing.
            let child = one[0].take().unwrap();
            assert_eq!(
                child.GetState(None, false).unwrap(),
                u32::try_from(ECS_HIDDEN.0).unwrap()
            );
            child.Invoke(None, None).unwrap();
        }
        let other = GUID::from_u128(1);
        let mut p = std::ptr::null_mut();
        // SAFETY: valid pointers to locals.
        assert_eq!(
            unsafe { DllGetClassObject(&raw const other, &raw const iid, &raw mut p) },
            CLASS_E_CLASSNOTAVAILABLE
        );
        assert_eq!(DllCanUnloadNow(), S_FALSE);
        assert!(module_dir().is_some());
    }
}
