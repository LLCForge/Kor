//! AppKit side of [`super`]: the Dock menu. tao's app delegate has no `applicationDockMenu:`, so it is
//! added to the delegate's class once at run time, together with `korDockItem:` (the items' action,
//! reached through the responder chain). Each item carries its `kor://` link as `representedObject`;
//! opening it through `NSWorkspace` hands it back to the running app (`application:openURLs:`).
//! Everything runs on the main thread (`MainThreadMarker`); the menu lives in a thread local there.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::io;
use std::sync::Once;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{MainThreadMarker, MainThreadOnly as _, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem, NSWorkspace};
use objc2_foundation::{NSString, NSURL};

use super::List;

thread_local! {
    static MENU: RefCell<Option<Retained<NSMenu>>> = const { RefCell::new(None) };
}

static INSTALL: Once = Once::new();

extern "C-unwind" fn dock_menu(
    _this: *mut AnyObject,
    _sel: Sel,
    _app: *mut AnyObject,
) -> *mut AnyObject {
    MENU.with(|m| {
        m.borrow().as_ref().map_or(std::ptr::null_mut(), |menu| {
            Retained::as_ptr(menu).cast_mut().cast()
        })
    })
}

extern "C-unwind" fn dock_item(_this: *mut AnyObject, _sel: Sel, sender: *mut AnyObject) {
    // SAFETY: AppKit sends this action from one of our NSMenuItems (the only items with this selector).
    let Some(item) = (unsafe { sender.cast::<NSMenuItem>().as_ref() }) else {
        return;
    };
    let Some(url) = item.representedObject() else {
        return;
    };
    if let Some(url) = url.downcast_ref::<NSURL>() {
        NSWorkspace::sharedWorkspace().openURL(url);
    }
}

fn install(mtm: MainThreadMarker) -> io::Result<()> {
    let app = NSApplication::sharedApplication(mtm);
    let delegate = app
        .delegate()
        .ok_or_else(|| io::Error::other("no app delegate"))?;
    // `Retained<ProtocolObject<dyn NSApplicationDelegate>>` → the object behind it.
    let object: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
    let class: &AnyClass = object.class();
    let class = std::ptr::from_ref(class).cast_mut();
    INSTALL.call_once(|| {
        // SAFETY: the function pointers match the declared type encodings
        // (`@@:@`: id (id, SEL, id); `v@:@`: void (id, SEL, id)); adding to a registered class is allowed.
        unsafe {
            let menu: Imp = std::mem::transmute::<
                extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject,
                Imp,
            >(dock_menu);
            let item: Imp = std::mem::transmute::<
                extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                Imp,
            >(dock_item);
            objc2::ffi::class_addMethod(class, sel!(applicationDockMenu:), menu, c"@@:@".as_ptr());
            objc2::ffi::class_addMethod(class, sel!(korDockItem:), item, c"v@:@".as_ptr());
        }
    });
    Ok(())
}

pub(super) fn apply(list: &List) -> io::Result<()> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| io::Error::other("Dock menu: not on the main thread"))?;
    install(mtm)?;
    let menu = NSMenu::new(mtm);
    let groups = [
        (Some(list.recent_title.as_str()), &list.recent),
        (None, &list.tasks),
    ];
    for (title, items) in groups {
        if items.is_empty() {
            continue;
        }
        if menu.numberOfItems() > 0 {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        if let Some(t) = title {
            let header = NSMenuItem::new(mtm);
            header.setTitle(&NSString::from_str(t));
            header.setEnabled(false);
            menu.addItem(&header);
        }
        for it in items {
            let Some(url) = it
                .args
                .first()
                .and_then(|u| NSURL::URLWithString(&NSString::from_str(u)))
            else {
                continue;
            };
            // SAFETY: `korDockItem:` was added to the delegate class in `install`.
            let entry = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&it.title),
                    Some(sel!(korDockItem:)),
                    &NSString::from_str(""),
                )
            };
            // SAFETY: an NSURL is a valid represented object.
            unsafe { entry.setRepresentedObject(Some(&url)) };
            menu.addItem(&entry);
        }
    }
    MENU.with(|m| *m.borrow_mut() = Some(menu));
    Ok(())
}
