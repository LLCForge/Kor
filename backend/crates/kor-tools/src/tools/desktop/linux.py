# Kor desktop tools on Linux (python3 -I). Accessibility tree through AT-SPI2 (the Atspi GObject
# bindings, present on GNOME/KDE desktops); window focus/state/move through X11 EWMH via ctypes.
# Inputs come only from the environment (same names as the Windows and macOS scripts).
# A window handle is pid * 1000 + the frame's index among its app's children; KOR_TITLE (the title
# Kor saw in list_windows) guards against the index now pointing at another window.
import json
import os
import sys

E = lambda k: os.environ.get(k, "")


def fail(msg):
    sys.stderr.write(msg + "\n")
    sys.exit(1)


TYPES = {
    "push button": "Button", "button": "Button", "toggle button": "CheckBox", "link": "Hyperlink",
    "text": "Edit", "entry": "Edit", "password text": "Edit", "spin button": "Spinner",
    "combo box": "ComboBox", "check box": "CheckBox", "radio button": "RadioButton",
    "menu item": "MenuItem", "check menu item": "MenuItem", "radio menu item": "MenuItem", "menu": "MenuItem",
    "page tab": "TabItem", "list item": "ListItem", "tree item": "TreeItem", "table cell": "DataItem",
    "slider": "Slider", "document web": "Document", "document frame": "Document",
    "label": "Text", "static": "Text", "heading": "Text", "paragraph": "Text",
}
FRAMES = ("frame", "window", "dialog", "alert", "file chooser")


def atspi():
    try:
        import gi
        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi
    except (ImportError, ValueError):
        fail("Accessibility (AT-SPI) is not available on this desktop: python3 with the Atspi "
             "GObject bindings (python3-gi, gir1.2-atspi-2.0) was not found.")
    return Atspi


def ifaces(acc):
    try:
        return set(acc.get_interfaces())
    except Exception:
        return set()


def frames(At):
    desk = At.get_desktop(0)
    out = []
    for a in range(desk.get_child_count()):
        app = desk.get_child_at_index(a)
        if app is None:
            continue
        try:
            pid = app.get_process_id()
            n = app.get_child_count()
        except Exception:
            continue
        for i in range(min(n, 1000)):
            f = app.get_child_at_index(i)
            try:
                if f is None or f.get_role_name() not in FRAMES:
                    continue
                out.append((pid * 1000 + i, f, app))
            except Exception:
                continue
    return out


def window(At, hwnd):
    want = E("KOR_TITLE")
    all_frames = frames(At)
    if not all_frames:
        fail("No application exposes AT-SPI on this desktop (is the accessibility bus turned off?).")
    for h, f, app in all_frames:
        if hwnd and h != hwnd:
            continue
        if not hwnd and not f.get_state_set().contains(At.StateType.ACTIVE):
            continue
        if want and (f.get_name() or "") != want:
            fail("The window list changed; call list_windows again.")
        return h, f, app
    fail("No such window; call list_windows for a current hwnd." if hwnd else "No window is active.")


def extents(At, acc):
    if "Component" not in ifaces(acc):
        return (0, 0, 0, 0)
    r = At.Component.get_extents(acc, At.CoordType.SCREEN)
    return (r.x, r.y, r.width, r.height)


def value_of(At, acc, kinds):
    if "Text" in kinds:
        t = At.Text.get_text(acc, 0, min(At.Text.get_character_count(acc), 200))
        return t or ""
    if "Value" in kinds:
        return str(At.Value.get_current_value(acc))
    return ""


def snapshot(At):
    hwnd, frame, _ = window(At, int(E("KOR_UIA_HWND") or 0))
    limit = int(E("KOR_UIA_MAX") or 200)
    with_text = E("KOR_UIA_TEXT") == "1"
    items = []
    seen = [0]

    def walk(acc, path):
        if len(items) >= limit or seen[0] > 5000 or len(path) > 60:
            return
        try:
            n = acc.get_child_count()
        except Exception:
            return
        for i in range(n):
            if len(items) >= limit:
                return
            seen[0] += 1
            c = acc.get_child_at_index(i)
            if c is None:
                continue
            try:
                states = c.get_state_set()
                if not states.contains(At.StateType.SHOWING):
                    continue
                role = c.get_role_name()
                kind = TYPES.get(role)
                if kind and (kind != "Text" or with_text):
                    x, y, w, h = extents(At, c)
                    if w > 0 and h > 0:
                        password = role == "password text"
                        kinds = ifaces(c)
                        toggle = ""
                        if states.contains(At.StateType.CHECKABLE) or kind in ("CheckBox", "RadioButton"):
                            toggle = "On" if states.contains(At.StateType.CHECKED) else "Off"
                        value = "" if password or kind in ("Button", "Hyperlink") else value_of(At, c, kinds)
                        if kind == "Text" and value == (c.get_name() or ""):
                            value = ""
                        if len(value) > 120:
                            value = value[:120] + "…"
                        items.append({
                            "rid": ".".join(map(str, path + [i])), "type": kind, "name": c.get_name() or "",
                            "value": value, "toggle": toggle, "password": password,
                            "enabled": states.contains(At.StateType.ENABLED), "x": x, "y": y, "w": w, "h": h,
                        })
                walk(c, path + [i])
            except Exception:
                continue

    walk(frame, [])
    return {"hwnd": hwnd, "title": frame.get_name() or "", "items": items}


def action_index(At, acc, names):
    if "Action" not in ifaces(acc):
        return -1
    for i in range(At.Action.get_n_actions(acc)):
        if (At.Action.get_action_name(acc, i) or "").lower() in names:
            return i
    return -1


def invoke(At):
    _, frame, _ = window(At, int(E("KOR_UIA_HWND") or 0))
    el = frame
    try:
        for i in [int(p) for p in E("KOR_UIA_RID").split(".")]:
            el = el.get_child_at_index(i)
        role = el.get_role_name()
    except Exception:
        fail("The element is gone; call uia_snapshot again.")
    if TYPES.get(role) != E("KOR_UIA_TYPE") or (el.get_name() or "") != E("KOR_UIA_NAME"):
        fail("The element is gone; call uia_snapshot again.")
    states = el.get_state_set()
    if not states.contains(At.StateType.ENABLED):
        fail("The element is disabled: " + (el.get_name() or ""))
    kinds = ifaces(el)
    verb = E("KOR_UIA_VERB")

    def do(names, what):
        i = action_index(At, el, names)
        if i < 0:
            fail("This element cannot be " + what + ".")
        At.Action.do_action(el, i)

    if verb == "set_value":
        if role == "password text":
            fail("Refusing to fill a password field; the user types passwords.")
        if "EditableText" not in kinds:
            fail("This element has no editable value.")
        At.EditableText.set_text_contents(el, E("KOR_UIA_VALUE"))
        return "set"
    if verb == "focus":
        if "Component" not in kinds or not At.Component.grab_focus(el):
            fail("This element cannot take focus.")
        return "focused"
    if verb == "toggle":
        do(("toggle", "click", "press", "activate"), "toggled")
        return "toggled"
    if verb == "select":
        parent = el.get_parent()
        if parent is None or "Selection" not in ifaces(parent):
            fail("This element cannot be selected.")
        At.Selection.select_child(parent, el.get_index_in_parent())
        return "selected"
    if verb in ("expand", "collapse"):
        want = verb == "expand"
        if states.contains(At.StateType.EXPANDED) != want:
            do(("expand or contract", "expand", "collapse", "activate", "click", "press"), verb + "ed")
        return "expanded" if want else "collapsed"
    i = action_index(At, el, ("click", "press", "activate", "jump", "toggle", "open"))
    if i >= 0:
        At.Action.do_action(el, i)
        return "invoked"
    fail("This element has no action; click it with left_click at its position instead.")


def list_windows(At):
    out = []
    for hwnd, f, app in frames(At):
        try:
            states = f.get_state_set()
            x, y, w, h = extents(At, f)
            out.append({
                "hwnd": hwnd, "title": f.get_name() or "", "process": app.get_name() or "",
                "pid": hwnd // 1000, "x": x, "y": y, "w": w, "h": h,
                "state": "minimized" if states.contains(At.StateType.ICONIFIED) else "normal",
                "focused": states.contains(At.StateType.ACTIVE),
            })
        except Exception:
            continue
    return out


# ---- X11 (EWMH) window control ----
def x11():
    if E("XDG_SESSION_TYPE") == "wayland" or not E("DISPLAY"):
        fail("Not available on this desktop: Wayland does not let one app focus, move or resize "
             "another app's windows. Use uia_invoke or keyboard shortcuts instead.")
    import ctypes
    import ctypes.util
    try:
        x = ctypes.cdll.LoadLibrary(ctypes.util.find_library("X11") or "libX11.so.6")
    except OSError:
        fail("Not available on this desktop: libX11 was not found.")
    c = ctypes
    x.XOpenDisplay.restype = c.c_void_p
    x.XOpenDisplay.argtypes = [c.c_char_p]
    x.XDefaultRootWindow.restype = c.c_ulong
    x.XDefaultRootWindow.argtypes = [c.c_void_p]
    x.XDefaultScreen.argtypes = [c.c_void_p]
    x.XInternAtom.restype = c.c_ulong
    x.XInternAtom.argtypes = [c.c_void_p, c.c_char_p, c.c_int]
    x.XGetWindowProperty.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_long, c.c_long, c.c_int, c.c_ulong,
                                     c.POINTER(c.c_ulong), c.POINTER(c.c_int), c.POINTER(c.c_ulong),
                                     c.POINTER(c.c_ulong), c.POINTER(c.c_void_p)]
    x.XFree.argtypes = [c.c_void_p]
    x.XSendEvent.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_long, c.c_void_p]
    x.XIconifyWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_int]
    x.XMoveResizeWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_int, c.c_uint, c.c_uint]
    x.XFlush.argtypes = [c.c_void_p]
    d = x.XOpenDisplay(None)
    if not d:
        fail("Not available on this desktop: cannot open the X display.")
    return x, d


def xprop(x, d, w, name, fmt_bytes):
    import ctypes as c
    t, f, n, after, data = c.c_ulong(), c.c_int(), c.c_ulong(), c.c_ulong(), c.c_void_p()
    atom = x.XInternAtom(d, name.encode(), 0)
    if x.XGetWindowProperty(d, w, atom, 0, 65536, 0, 0, c.byref(t), c.byref(f), c.byref(n), c.byref(after), c.byref(data)) != 0 or not data:
        return None
    try:
        if fmt_bytes:
            return c.string_at(data, n.value).decode("utf-8", "replace")
        return list(c.cast(data, c.POINTER(c.c_ulong))[: n.value])
    finally:
        x.XFree(data)


def xwindow(x, d, pid, title):
    root = x.XDefaultRootWindow(d)
    same_pid = []
    for w in xprop(x, d, root, "_NET_CLIENT_LIST", False) or []:
        if (xprop(x, d, w, "_NET_WM_PID", False) or [0])[0] != pid:
            continue
        same_pid.append(w)
        if (xprop(x, d, w, "_NET_WM_NAME", True) or xprop(x, d, w, "WM_NAME", True) or "") == title:
            return w
    if len(same_pid) == 1:
        return same_pid[0]
    fail("No X11 window matches; the app may run under Wayland. Call list_windows again.")


def client_message(x, d, w, name, data):
    import ctypes as c

    class Msg(c.Structure):
        _fields_ = [("type", c.c_int), ("serial", c.c_ulong), ("send_event", c.c_int), ("display", c.c_void_p),
                    ("window", c.c_ulong), ("message_type", c.c_ulong), ("format", c.c_int), ("l", c.c_long * 5)]

    class Event(c.Union):
        _fields_ = [("xclient", Msg), ("pad", c.c_long * 24)]

    ev = Event()
    ev.xclient.type = 33  # ClientMessage
    ev.xclient.send_event = 1
    ev.xclient.window = w
    ev.xclient.message_type = x.XInternAtom(d, name.encode(), 0)
    ev.xclient.format = 32
    for i, v in enumerate(data):
        ev.xclient.l[i] = v
    x.XSendEvent(d, x.XDefaultRootWindow(d), 0, (1 << 20) | (1 << 19), c.byref(ev))


def window_action(At, action):
    _, frame, app = window(At, int(E("KOR_HWND") or 0))
    title = frame.get_name() or ""
    x, d = x11()
    w = xwindow(x, d, app.get_process_id(), title)
    atom = lambda n: x.XInternAtom(d, n.encode(), 0)
    maxed = [atom("_NET_WM_STATE_MAXIMIZED_VERT"), atom("_NET_WM_STATE_MAXIMIZED_HORZ")]
    if action == "focus":
        client_message(x, d, w, "_NET_ACTIVE_WINDOW", [2, 0, 0])
        out = "focused: " + title
    elif action == "move":
        client_message(x, d, w, "_NET_WM_STATE", [0] + maxed + [1])
        x.XMoveResizeWindow(d, w, int(E("KOR_X")), int(E("KOR_Y")), int(E("KOR_W")), int(E("KOR_H")))
        out = "moved: " + title
    else:
        state = E("KOR_STATE")
        if state == "minimize":
            x.XIconifyWindow(d, w, x.XDefaultScreen(d))
        elif state == "maximize":
            client_message(x, d, w, "_NET_WM_STATE", [1] + maxed + [1])
        elif state == "restore":
            client_message(x, d, w, "_NET_WM_STATE", [0] + maxed + [1])
            client_message(x, d, w, "_NET_ACTIVE_WINDOW", [2, 0, 0])
        elif state == "close":
            client_message(x, d, w, "_NET_CLOSE_WINDOW", [0, 2])
        else:
            fail("state must be minimize, maximize, restore or close")
        out = state + ": " + title
    x.XFlush(d)
    return out


def emit(text):
    sys.stdout.buffer.write(text.encode("utf-8") + b"\n")


def main():
    action = E("KOR_UIA_ACTION") or E("KOR_DESK_ACTION")
    At = atspi()
    if action == "snapshot":
        emit(json.dumps(snapshot(At)))
    elif action == "invoke":
        emit(invoke(At))
    elif action == "list_windows":
        emit(json.dumps(list_windows(At)))
    elif action in ("focus", "state", "move"):
        emit(window_action(At, action))
    else:
        fail("unknown action " + action)


try:
    main()
except SystemExit:
    raise
except Exception as e:  # noqa: BLE001 - one readable line for the tool result
    fail(str(e) or type(e).__name__)
