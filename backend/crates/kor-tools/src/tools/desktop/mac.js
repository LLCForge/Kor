// Kor desktop tools on macOS (osascript -l JavaScript). Accessibility tree through System Events
// (AXUIElement underneath; needs the Accessibility permission). Inputs come only from the
// environment: KOR_UIA_ACTION | KOR_DESK_ACTION, KOR_UIA_HWND, KOR_UIA_MAX, KOR_UIA_TEXT,
// KOR_UIA_RID, KOR_UIA_TYPE, KOR_UIA_NAME, KOR_UIA_VERB, KOR_UIA_VALUE, KOR_HWND, KOR_TITLE,
// KOR_STATE, KOR_X, KOR_Y, KOR_W, KOR_H.
// A window handle is pid * 1000 + the window's index in its process; KOR_TITLE (the title Kor saw in
// list_windows) guards against the index now pointing at another window.
ObjC.import('Foundation');
ObjC.import('AppKit');

const ENV = ObjC.deepUnwrap($.NSProcessInfo.processInfo.environment);
const E = (k) => (ENV[k] === undefined || ENV[k] === null ? '' : String(ENV[k]));
const se = Application('System Events');

const TYPES = {
  AXButton: 'Button', AXDisclosureTriangle: 'Button', AXMenuButton: 'SplitButton', AXLink: 'Hyperlink',
  AXTextField: 'Edit', AXTextArea: 'Edit', AXComboBox: 'ComboBox', AXPopUpButton: 'ComboBox',
  AXCheckBox: 'CheckBox', AXRadioButton: 'RadioButton', AXMenuItem: 'MenuItem', AXMenuBarItem: 'MenuItem',
  AXRow: 'ListItem', AXCell: 'DataItem', AXSlider: 'Slider', AXIncrementor: 'Spinner', AXWebArea: 'Document',
  AXStaticText: 'Text',
};

function str(v) {
  return v === null || v === undefined ? '' : String(v);
}

function windowOf(hwnd) {
  let proc;
  let idx = 0;
  if (hwnd) {
    const pid = Math.floor(hwnd / 1000);
    idx = hwnd % 1000;
    const found = se.processes.whose({ unixId: pid })();
    if (found.length === 0) throw new Error('No such window; call list_windows for a current hwnd.');
    proc = found[0];
  } else {
    const front = se.processes.whose({ frontmost: true })();
    if (front.length === 0) throw new Error('No app is in front.');
    proc = front[0];
  }
  const wins = proc.windows();
  if (idx >= wins.length) throw new Error('No such window; call list_windows for a current hwnd.');
  const win = wins[idx];
  const want = E('KOR_TITLE');
  if (want && str(win.name()) !== want) {
    throw new Error('The window list changed; call list_windows again.');
  }
  return { proc, win, hwnd: proc.unixId() * 1000 + idx };
}

// One Apple event per property for all children at once; null where a child lacks it.
function bulk(spec, n, prop) {
  try {
    const v = spec[prop]();
    if (Array.isArray(v) && v.length === n) return v;
  } catch (e) {}
  return new Array(n).fill(null);
}

function label(name, title, desc) {
  return str(name) || str(title) || str(desc);
}

function toggleOf(type, value) {
  if (type !== 'CheckBox' && type !== 'RadioButton') return '';
  return Number(value) === 1 ? 'On' : 'Off';
}

function snapshot() {
  const { win, hwnd } = windowOf(Number(E('KOR_UIA_HWND')) || 0);
  const max = Number(E('KOR_UIA_MAX')) || 200;
  const withText = E('KOR_UIA_TEXT') === '1';
  const items = [];
  let visited = 0;
  const walk = (el, path) => {
    if (items.length >= max || visited > 4000 || path.length > 40) return;
    const spec = el.uiElements;
    let n = 0;
    try { n = spec.length; } catch (e) { return; }
    const r = bulk(spec, n, 'role');
    const names = bulk(spec, n, 'name');
    const titles = bulk(spec, n, 'title');
    const descs = bulk(spec, n, 'description');
    const subroles = bulk(spec, n, 'subrole');
    const enabled = bulk(spec, n, 'enabled');
    const pos = bulk(spec, n, 'position');
    const size = bulk(spec, n, 'size');
    for (let i = 0; i < n && items.length < max; i++) {
      visited++;
      const type = TYPES[r[i]];
      const p = pos[i] || [0, 0];
      const s = size[i] || [0, 0];
      if (type && (type !== 'Text' || withText) && s[0] > 0 && s[1] > 0) {
        const password = subroles[i] === 'AXSecureTextField';
        let value = '';
        let raw = null;
        if (!password && (type === 'Edit' || type === 'ComboBox' || type === 'Slider' || type === 'CheckBox' || type === 'RadioButton' || type === 'Text')) {
          try { raw = spec[i].value(); } catch (e) {}
        }
        const toggle = toggleOf(type, raw);
        if (!toggle && raw !== null) value = str(raw);
        if (value.length > 120) value = value.substring(0, 120) + '…';
        items.push({
          rid: path.concat([i]).join('.'), type, name: label(names[i], titles[i], descs[i]), value, toggle,
          password, enabled: enabled[i] !== false, x: Math.round(p[0]), y: Math.round(p[1]), w: Math.round(s[0]), h: Math.round(s[1]),
        });
      }
      if (type !== 'Text') walk(spec[i], path.concat([i]));
    }
  };
  walk(win, []);
  return { hwnd, title: str(win.name()), items };
}

function invoke() {
  const { win } = windowOf(Number(E('KOR_UIA_HWND')) || 0);
  let el = win;
  for (const i of E('KOR_UIA_RID').split('.').map(Number)) el = el.uiElements[i];
  let role;
  let name;
  try {
    role = el.role();
    name = label(el.name(), el.title(), el.description());
  } catch (e) {
    throw new Error('The element is gone; call uia_snapshot again.');
  }
  if (TYPES[role] !== E('KOR_UIA_TYPE') || name !== E('KOR_UIA_NAME')) {
    throw new Error('The element is gone; call uia_snapshot again.');
  }
  if (el.enabled() === false) throw new Error('The element is disabled: ' + name);
  const press = () => {
    const acts = el.actions.name();
    if (acts.indexOf('AXPress') < 0) return false;
    el.actions.byName('AXPress').perform();
    return true;
  };
  switch (E('KOR_UIA_VERB')) {
    case 'set_value':
      if (el.subrole() === 'AXSecureTextField') throw new Error('Refusing to fill a password field; the user types passwords.');
      el.value = E('KOR_UIA_VALUE');
      return 'set';
    case 'focus':
      el.focused = true;
      return 'focused';
    case 'toggle':
      if (TYPES[role] !== 'CheckBox' && TYPES[role] !== 'RadioButton') throw new Error('This element cannot be toggled.');
      if (!press()) throw new Error('This element cannot be toggled.');
      return 'toggled';
    case 'select':
      el.selected = true;
      return 'selected';
    case 'expand':
    case 'collapse': {
      const open = E('KOR_UIA_VERB') === 'expand';
      if (role === 'AXDisclosureTriangle') {
        if (Number(el.value()) !== (open ? 1 : 0)) press();
      } else {
        el.attributes.byName('AXDisclosing').value = open;
      }
      return open ? 'expanded' : 'collapsed';
    }
    default: {
      if (press()) return 'invoked';
      const p = el.position();
      const s = el.size();
      se.click({ at: [Math.round(p[0] + s[0] / 2), Math.round(p[1] + s[1] / 2)] });
      return 'clicked';
    }
  }
}

function listWindows() {
  const out = [];
  for (const proc of se.processes.whose({ backgroundOnly: false })()) {
    let wins;
    try { wins = proc.windows(); } catch (e) { continue; }
    if (wins.length === 0) continue;
    const pid = proc.unixId();
    const front = proc.frontmost();
    const pname = str(proc.name());
    wins.forEach((w, i) => {
      if (i >= 1000) return;
      let min = false;
      try { min = w.attributes.byName('AXMinimized').value() === true; } catch (e) {}
      const p = w.position() || [0, 0];
      const s = w.size() || [0, 0];
      out.push({
        hwnd: pid * 1000 + i, title: str(w.name()), process: pname, pid, x: p[0], y: p[1], w: s[0], h: s[1],
        state: min ? 'minimized' : 'normal', focused: front && i === 0,
      });
    });
  }
  return out;
}

function windowAction(action) {
  const { proc, win } = windowOf(Number(E('KOR_HWND')) || 0);
  const title = str(win.name());
  const minimized = (v) => { win.attributes.byName('AXMinimized').value = v; };
  if (action === 'focus') {
    try { minimized(false); } catch (e) {}
    proc.frontmost = true;
    win.actions.byName('AXRaise').perform();
    return 'focused: ' + title;
  }
  if (action === 'move') {
    win.position = [Number(E('KOR_X')), Number(E('KOR_Y'))];
    win.size = [Number(E('KOR_W')), Number(E('KOR_H'))];
    return 'moved: ' + title;
  }
  const state = E('KOR_STATE');
  if (state === 'minimize') minimized(true);
  else if (state === 'restore') minimized(false);
  else if (state === 'maximize') {
    // Fill the visible frame of the main screen (Cocoa's origin is bottom-left).
    const full = $.NSScreen.mainScreen.frame;
    const vis = $.NSScreen.mainScreen.visibleFrame;
    win.position = [vis.origin.x, full.size.height - vis.origin.y - vis.size.height];
    win.size = [vis.size.width, vis.size.height];
  } else if (state === 'close') {
    const close = win.buttons.whose({ subrole: 'AXCloseButton' })();
    if (close.length === 0) throw new Error('This window has no close button.');
    close[0].click();
  } else throw new Error('state must be minimize, maximize, restore or close');
  return state + ': ' + title;
}

function run() {
  const action = E('KOR_UIA_ACTION') || E('KOR_DESK_ACTION');
  switch (action) {
    case 'snapshot': return JSON.stringify(snapshot());
    case 'invoke': return invoke();
    case 'list_windows': return JSON.stringify(listWindows());
    case 'focus':
    case 'state':
    case 'move': return windowAction(action);
    default: throw new Error('unknown action ' + action);
  }
}
