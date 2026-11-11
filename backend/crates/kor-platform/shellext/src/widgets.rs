//! Widgets board provider (`IWidgetProvider`, Windows App SDK) behind `kor-widgets.exe`.
//!
//! The Widgets host starts the exe through the package's COM `ExeServer` entry, creates the
//! provider and calls it as widgets are pinned, shown, resized and clicked. The provider pushes
//! [`crate::widget_card`] content with `WidgetManager::UpdateWidget` and refreshes active widgets
//! every [`REFRESH`]. Card buttons start `kor-desktop` (validated by `widget_card::Action`).
#![allow(unsafe_code)]
// `#[implement]` expansions trip these pedantic lints; the code is the windows crate's.
#![allow(clippy::inline_always, clippy::ref_as_ptr)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx, CoRegisterClassObject,
    CoRevokeClassObject, IClassFactory, IClassFactory_Impl, REGCLS_MULTIPLEUSE,
};
use windows_core::{BOOL, GUID, HSTRING, IUnknown, Interface, Ref, Result, implement};

use crate::menu::{self, Lang};
use crate::widget_card::{self, Action};
use crate::widgets_bindings::Microsoft::Windows::Widgets::Providers::{
    IWidgetProvider, IWidgetProvider_Impl, WidgetActionInvokedArgs, WidgetContext,
    WidgetContextChangedArgs, WidgetManager, WidgetUpdateRequestOptions,
};
use crate::widgets_bindings::Microsoft::Windows::Widgets::WidgetSize;

/// How often shown widgets are refreshed.
pub const REFRESH: Duration = Duration::from_secs(60);

/// Pinned widgets: id → (size name, shown on the board right now).
pub type Widgets = Arc<Mutex<HashMap<String, (String, bool)>>>;

fn size_name(s: WidgetSize) -> &'static str {
    match s {
        WidgetSize::Small => "small",
        WidgetSize::Large => "large",
        _ => "medium",
    }
}

fn lang() -> Lang {
    let text = menu::user_config()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    // SAFETY: no arguments.
    Lang::pick(menu::settings(&text).0.as_deref(), unsafe {
        GetUserDefaultUILanguage()
    })
}

/// Send the card for widget `id` at `size`.
///
/// # Errors
/// When the Widgets host rejects the update.
pub fn push(id: &str, size: &str) -> Result<()> {
    let sessions = widget_card::sessions_dir()
        .map(|d| widget_card::recent(&d, widget_card::rows_for(size)))
        .unwrap_or_default();
    let data = widget_card::data(lang(), &sessions, SystemTime::now());
    let opts = WidgetUpdateRequestOptions::CreateInstance(&HSTRING::from(id))?;
    opts.SetTemplate(&HSTRING::from(widget_card::TEMPLATE))?;
    opts.SetData(&HSTRING::from(data.to_string()))?;
    WidgetManager::GetDefault()?.UpdateWidget(&opts)
}

/// Refresh every widget that is currently shown.
pub fn refresh(widgets: &Widgets) {
    let shown: Vec<(String, String)> = widgets
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(_, (_, active))| *active)
        .map(|(id, (size, _))| (id.clone(), size.clone()))
        .collect();
    for (id, size) in shown {
        let _ = push(&id, &size);
    }
}

#[implement(IWidgetProvider)]
struct Provider {
    widgets: Widgets,
    live: Arc<AtomicUsize>,
    /// Signalled when the host releases a provider: the exe may exit when none is left.
    released: Sender<()>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
        let _ = self.released.send(());
    }
}

impl Provider {
    fn track(&self, ctx: &WidgetContext, active: Option<bool>) -> Result<()> {
        let id = ctx.Id()?.to_string();
        let size = size_name(ctx.Size()?).to_owned();
        let shown = match active {
            Some(a) => a,
            None => ctx.IsActive()?,
        };
        self.widgets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), (size.clone(), shown));
        push(&id, &size)
    }
}

impl IWidgetProvider_Impl for Provider_Impl {
    fn CreateWidget(&self, ctx: Ref<WidgetContext>) -> Result<()> {
        self.track(ctx.ok()?, None)
    }

    fn DeleteWidget(&self, id: &HSTRING, _state: &HSTRING) -> Result<()> {
        self.widgets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.to_string());
        Ok(())
    }

    fn OnActionInvoked(&self, args: Ref<WidgetActionInvokedArgs>) -> Result<()> {
        let args = args.ok()?;
        let Some(action) = Action::parse(&args.Verb()?.to_string(), &args.Data()?.to_string())
        else {
            return Ok(());
        };
        if let Some(dir) = crate::exe_dir() {
            let _ = crate::launch_desktop(&dir, &action.args());
        }
        Ok(())
    }

    fn OnWidgetContextChanged(&self, args: Ref<WidgetContextChangedArgs>) -> Result<()> {
        self.track(&args.ok()?.WidgetContext()?, None)
    }

    fn Activate(&self, ctx: Ref<WidgetContext>) -> Result<()> {
        self.track(ctx.ok()?, Some(true))
    }

    fn Deactivate(&self, id: &HSTRING) -> Result<()> {
        if let Some(w) = self
            .widgets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&id.to_string())
        {
            w.1 = false;
        }
        Ok(())
    }
}

/// Run the COM server: register the provider class, refresh shown widgets every [`REFRESH`] and
/// return once the host has released every provider (after a short grace period: the host often
/// creates a new one right away when the board reopens).
///
/// # Errors
/// When COM cannot be initialised or the class cannot be registered.
pub fn serve() -> Result<()> {
    let clsid = GUID::try_from(crate::WIDGET_CLSID)?;
    // SAFETY: first COM call of this thread; the MTA lets the host call from any thread.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let (released, done) = std::sync::mpsc::channel();
    let widgets = Widgets::default();
    let live = Arc::new(AtomicUsize::new(0));
    let factory: IClassFactory = ProviderFactory {
        widgets: widgets.clone(),
        live: live.clone(),
        released,
    }
    .into();
    // SAFETY: `factory` stays alive until it is revoked below.
    let cookie = unsafe {
        CoRegisterClassObject(
            &raw const clsid,
            &factory,
            CLSCTX_LOCAL_SERVER,
            REGCLS_MULTIPLEUSE,
        )?
    };
    loop {
        match done.recv_timeout(REFRESH) {
            Err(RecvTimeoutError::Timeout) => refresh(&widgets),
            Err(RecvTimeoutError::Disconnected) => break,
            Ok(()) => {
                std::thread::sleep(Duration::from_secs(5));
                if live.load(Ordering::SeqCst) == 0 {
                    break;
                }
            }
        }
    }
    // SAFETY: `cookie` came from the registration above.
    unsafe { CoRevokeClassObject(cookie) }
}

/// Class factory the exe registers for [`crate::WIDGET_CLSID`].
#[implement(IClassFactory)]
struct ProviderFactory {
    widgets: Widgets,
    /// Providers alive now.
    live: Arc<AtomicUsize>,
    released: Sender<()>,
}

impl std::fmt::Debug for ProviderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderFactory")
    }
}

impl IClassFactory_Impl for ProviderFactory_Impl {
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
        self.live.fetch_add(1, Ordering::SeqCst);
        let provider: IWidgetProvider = Provider {
            widgets: self.widgets.clone(),
            live: self.live.clone(),
            released: self.released.clone(),
        }
        .into();
        // SAFETY: `iid`/`object` checked; `query` AddRefs on success.
        unsafe { provider.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_creates_a_provider_that_signals_release() {
        let (tx, rx) = std::sync::mpsc::channel();
        let widgets = Widgets::default();
        let live = Arc::new(AtomicUsize::new(0));
        let f: IClassFactory = ProviderFactory {
            widgets: widgets.clone(),
            live: live.clone(),
            released: tx,
        }
        .into();
        // SAFETY: COM call on a live interface.
        let p: IWidgetProvider = unsafe { f.CreateInstance(None) }.unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 1);
        widgets
            .lock()
            .unwrap()
            .insert("w1".into(), ("small".into(), true));
        p.Deactivate(&HSTRING::from("w1")).unwrap();
        assert!(!widgets.lock().unwrap()["w1"].1);
        p.DeleteWidget(&HSTRING::from("w1"), &HSTRING::new())
            .unwrap();
        assert!(widgets.lock().unwrap().is_empty());
        assert!(rx.try_recv().is_err());
        drop(p);
        assert!(rx.try_recv().is_ok());
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(size_name(WidgetSize::Large), "large");
    }
}
