//! macOS pieces of §27.10 that must run inside the desktop app's process (ADR 0052):
//! Core Spotlight index of sessions, Universal Clipboard detection and Handoff.
//!
//! Spotlight items and Handoff activities carry a `kor://` link. When the user picks one (a
//! Spotlight result, or Handoff on another Mac), macOS calls the app delegate's
//! `application:continueUserActivity:restorationHandler:`; [`install_continue_handler`] adds that
//! method to the delegate and re-opens the link through `NSWorkspace`, which reaches the app's
//! `kor://` handler like any other link. Outside macOS every function reports `Unsupported`.

use std::io;

/// One session in the Spotlight index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotItem {
    /// `kor://resume?id=…` (also the item's unique identifier).
    pub url: String,
    pub title: String,
    pub text: String,
}

/// Spotlight domain of Kor's items.
pub const DOMAIN: &str = "dev.kor.sessions";
/// Handoff activity type (declared in the app's `Info.plist` `NSUserActivityTypes`).
pub const ACTIVITY: &str = "dev.kor.session";
/// Pasteboard type macOS adds to content that came from another device (Universal Clipboard).
pub const REMOTE_CLIPBOARD_TYPE: &str = "com.apple.is-remote-clipboard";

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "macOS only")
}

/// Replace Kor's Spotlight items with `items`.
///
/// # Errors
/// Outside macOS, or when Core Spotlight is unavailable.
pub fn spotlight_index(items: &[SpotItem]) -> io::Result<()> {
    if crate::os() != crate::Os::Mac {
        return Err(unsupported());
    }
    imp::spotlight_index(items)
}

/// Remove every Kor item from Spotlight.
///
/// # Errors
/// Outside macOS, or when Core Spotlight is unavailable.
pub fn spotlight_clear() -> io::Result<()> {
    if crate::os() != crate::Os::Mac {
        return Err(unsupported());
    }
    imp::spotlight_clear()
}

/// The clipboard holds content pasted from an iPhone/iPad/other Mac (Universal Clipboard).
pub fn remote_clipboard() -> bool {
    crate::os() == crate::Os::Mac && imp::remote_clipboard()
}

/// Advertise the open session for Handoff (`url` = `kor://resume?id=…`; `web` = an https page
/// that continues it on a device without Kor, e.g. the remote side panel). Call on the main thread.
///
/// # Errors
/// Outside macOS.
pub fn handoff_begin(url: &str, title: &str, web: Option<&str>) -> io::Result<()> {
    if crate::os() != crate::Os::Mac {
        return Err(unsupported());
    }
    imp::handoff_begin(url, title, web)
}

/// Stop advertising (session closed). Call on the main thread.
pub fn handoff_end() {
    if crate::os() == crate::Os::Mac {
        imp::handoff_end();
    }
}

/// Add `application:continueUserActivity:restorationHandler:` to the app delegate so Spotlight
/// results and Handoff open their `kor://` link. Call once on the main thread after launch.
///
/// # Errors
/// Outside macOS, or when the app has no delegate yet.
pub fn install_continue_handler() -> io::Result<()> {
    if crate::os() != crate::Os::Mac {
        return Err(unsupported());
    }
    imp::install_continue_handler()
}

/// The `kor://` link a continued activity carries: Spotlight's identifier or Handoff's `url`.
pub fn continued_link(activity_type: &str, spotlight_id: Option<&str>, user_url: Option<&str>) -> Option<String> {
    let link = if activity_type == "com.apple.corespotlightitem" {
        spotlight_id
    } else if activity_type == ACTIVITY {
        user_url
    } else {
        None
    }?;
    (link.starts_with("kor://") && link.len() <= 4096).then(|| link.to_owned())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    //! Each `unsafe` block is one Objective-C message to a class or object obtained just before
    //! (null-checked); objects created with `alloc`/`init` are released when no longer needed.
    use std::ffi::{CStr, c_char};
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use block2::Block;
    use objc2::ffi;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::sel;
    use objc2_foundation::NSString;

    use super::{ACTIVITY, DOMAIN, REMOTE_CLIPBOARD_TYPE, SpotItem};

    #[link(name = "CoreSpotlight", kind = "framework")]
    unsafe extern "C" {}

    type Done = Block<dyn Fn(*mut AnyObject)>;

    fn class(name: &CStr) -> io::Result<&'static AnyClass> {
        AnyClass::get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, format!("{name:?} missing")))
    }

    fn ns(s: &str) -> Retained<NSString> {
        NSString::from_str(s)
    }

    fn array(items: &[*mut AnyObject]) -> io::Result<*mut AnyObject> {
        let cls = class(c"NSMutableArray")?;
        // SAFETY: `+array` returns an autoreleased empty array.
        let arr: *mut AnyObject = unsafe { msg_send![cls, array] };
        for &i in items {
            // SAFETY: `arr` is a live NSMutableArray, `i` a live object.
            let () = unsafe { msg_send![arr, addObject: i] };
        }
        Ok(arr)
    }

    fn release(o: *mut AnyObject) {
        if !o.is_null() {
            // SAFETY: `o` was returned +1 by alloc/init.
            let () = unsafe { msg_send![o, release] };
        }
    }

    pub fn spotlight_index(items: &[SpotItem]) -> io::Result<()> {
        let attr_cls = class(c"CSSearchableItemAttributeSet")?;
        let item_cls = class(c"CSSearchableItem")?;
        let index_cls = class(c"CSSearchableIndex")?;
        let mut made = Vec::with_capacity(items.len());
        for it in items {
            // SAFETY: alloc/init pairs on Core Spotlight classes; arguments are live NSStrings.
            unsafe {
                let set: *mut AnyObject = msg_send![attr_cls, alloc];
                let set: *mut AnyObject = msg_send![set, initWithItemContentType: &*ns("public.text")];
                if set.is_null() {
                    continue;
                }
                let () = msg_send![set, setTitle: &*ns(&it.title)];
                let () = msg_send![set, setContentDescription: &*ns(&it.text)];
                let () = msg_send![set, setTextContent: &*ns(&it.text)];
                let item: *mut AnyObject = msg_send![item_cls, alloc];
                let item: *mut AnyObject = msg_send![
                    item,
                    initWithUniqueIdentifier: &*ns(&it.url),
                    domainIdentifier: &*ns(DOMAIN),
                    attributeSet: set
                ];
                release(set);
                if !item.is_null() {
                    made.push(item);
                }
            }
        }
        spotlight_clear()?;
        let arr = array(&made)?;
        // SAFETY: the default index accepts an NSArray of CSSearchableItem; no completion block.
        unsafe {
            let index: *mut AnyObject = msg_send![index_cls, defaultSearchableIndex];
            let () = msg_send![index, indexSearchableItems: arr, completionHandler: None::<&Done>];
        }
        for i in made {
            release(i);
        }
        Ok(())
    }

    pub fn spotlight_clear() -> io::Result<()> {
        let index_cls = class(c"CSSearchableIndex")?;
        let domains = array(&[Retained::as_ptr(&ns(DOMAIN)).cast_mut().cast()])?;
        // SAFETY: as above, with an NSArray of NSString domains.
        unsafe {
            let index: *mut AnyObject = msg_send![index_cls, defaultSearchableIndex];
            let () = msg_send![
                index,
                deleteSearchableItemsWithDomainIdentifiers: domains,
                completionHandler: None::<&Done>
            ];
        }
        Ok(())
    }

    pub fn remote_clipboard() -> bool {
        let Ok(cls) = class(c"NSPasteboard") else {
            return false;
        };
        // SAFETY: `generalPasteboard` and `types` are plain getters; `types` may be nil.
        unsafe {
            let pb: *mut AnyObject = msg_send![cls, generalPasteboard];
            if pb.is_null() {
                return false;
            }
            let types: *mut AnyObject = msg_send![pb, types];
            if types.is_null() {
                return false;
            }
            let has: Bool = msg_send![types, containsObject: &*ns(REMOTE_CLIPBOARD_TYPE)];
            has.as_bool()
        }
    }

    /// The current Handoff activity (+1 retained), 0 when none.
    static CURRENT: AtomicUsize = AtomicUsize::new(0);

    pub fn handoff_begin(url: &str, title: &str, web: Option<&str>) -> io::Result<()> {
        let cls = class(c"NSUserActivity")?;
        let dict_cls = class(c"NSDictionary")?;
        let url_cls = class(c"NSURL")?;
        // SAFETY: alloc/init of NSUserActivity; setters take live objects.
        let act = unsafe {
            let act: *mut AnyObject = msg_send![cls, alloc];
            let act: *mut AnyObject = msg_send![act, initWithActivityType: &*ns(ACTIVITY)];
            if act.is_null() {
                return Err(io::Error::other("NSUserActivity"));
            }
            let () = msg_send![act, setTitle: &*ns(title)];
            let info: *mut AnyObject =
                msg_send![dict_cls, dictionaryWithObject: &*ns(url), forKey: &*ns("url")];
            let () = msg_send![act, setUserInfo: info];
            if let Some(w) = web {
                let u: *mut AnyObject = msg_send![url_cls, URLWithString: &*ns(w)];
                if !u.is_null() {
                    let () = msg_send![act, setWebpageURL: u];
                }
            }
            let () = msg_send![act, setEligibleForHandoff: Bool::YES];
            let () = msg_send![act, becomeCurrent];
            act
        };
        end(CURRENT.swap(act as usize, Ordering::AcqRel));
        Ok(())
    }

    fn end(prev: usize) {
        let prev = prev as *mut AnyObject;
        if !prev.is_null() {
            // SAFETY: `prev` is the +1 activity stored by `handoff_begin`.
            let () = unsafe { msg_send![prev, invalidate] };
            release(prev);
        }
    }

    pub fn handoff_end() {
        end(CURRENT.swap(0, Ordering::AcqRel));
    }

    fn string(o: *mut AnyObject) -> Option<String> {
        if o.is_null() {
            return None;
        }
        // SAFETY: `o` is an NSString; UTF8String lives as long as it (copied at once).
        unsafe {
            let p: *const c_char = msg_send![o, UTF8String];
            (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }

    extern "C-unwind" fn continue_activity(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        activity: *mut AnyObject,
        _restore: *mut AnyObject,
    ) -> Bool {
        if activity.is_null() {
            return Bool::NO;
        }
        // SAFETY: `activity` is the NSUserActivity macOS passes; getters may return nil.
        let link = unsafe {
            let kind: *mut AnyObject = msg_send![activity, activityType];
            let info: *mut AnyObject = msg_send![activity, userInfo];
            let get = |k: &str| -> Option<String> {
                if info.is_null() {
                    return None;
                }
                let v: *mut AnyObject = msg_send![info, objectForKey: &*ns(k)];
                string(v)
            };
            super::continued_link(
                &string(kind).unwrap_or_default(),
                get("kCSSearchableItemActivityIdentifier").as_deref(),
                get("url").as_deref(),
            )
        };
        let Some(link) = link else {
            return Bool::NO;
        };
        let (Ok(url_cls), Ok(ws_cls)) = (class(c"NSURL"), class(c"NSWorkspace")) else {
            return Bool::NO;
        };
        // SAFETY: NSURL from a checked kor:// string; NSWorkspace delivers it to our handler.
        unsafe {
            let u: *mut AnyObject = msg_send![url_cls, URLWithString: &*ns(&link)];
            if u.is_null() {
                return Bool::NO;
            }
            let ws: *mut AnyObject = msg_send![ws_cls, sharedWorkspace];
            msg_send![ws, openURL: u]
        }
    }

    pub fn install_continue_handler() -> io::Result<()> {
        let app_cls = class(c"NSApplication")?;
        // SAFETY: `sharedApplication`/`delegate` are getters; the delegate's class gets one
        // method with the documented signature (BOOL, id, id, block).
        unsafe {
            let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
            let delegate: *mut AnyObject = if app.is_null() { app } else { msg_send![app, delegate] };
            if delegate.is_null() {
                return Err(io::Error::other("no app delegate"));
            }
            let cls = (*delegate).class();
            let imp: Imp = std::mem::transmute(
                continue_activity
                    as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject) -> Bool,
            );
            // Returns NO when the delegate already has the method: nothing to do then.
            let _ = ffi::class_addMethod(
                std::ptr::from_ref(cls).cast_mut(),
                sel!(application:continueUserActivity:restorationHandler:),
                imp,
                c"c@:@@@?".as_ptr(),
            );
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::io;

    use super::SpotItem;

    pub fn spotlight_index(_: &[SpotItem]) -> io::Result<()> {
        Err(super::unsupported())
    }
    pub fn spotlight_clear() -> io::Result<()> {
        Err(super::unsupported())
    }
    pub fn remote_clipboard() -> bool {
        false
    }
    pub fn handoff_begin(_: &str, _: &str, _: Option<&str>) -> io::Result<()> {
        Err(super::unsupported())
    }
    pub fn handoff_end() {}
    pub fn install_continue_handler() -> io::Result<()> {
        Err(super::unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continued_activities_only_open_kor_links() {
        assert_eq!(
            continued_link("com.apple.corespotlightitem", Some("kor://resume?id=a1"), None).as_deref(),
            Some("kor://resume?id=a1")
        );
        assert_eq!(continued_link(ACTIVITY, None, Some("kor://resume?id=b")).as_deref(), Some("kor://resume?id=b"));
        assert!(continued_link(ACTIVITY, None, Some("https://evil")).is_none());
        assert!(continued_link("other", Some("kor://x"), Some("kor://y")).is_none());
        if crate::os() != crate::Os::Mac {
            assert!(!remote_clipboard());
            assert_eq!(spotlight_clear().unwrap_err().kind(), io::ErrorKind::Unsupported);
            handoff_end();
        }
    }
}
