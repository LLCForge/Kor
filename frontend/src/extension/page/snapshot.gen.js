// ÜRETİLDİ: scripts/build-extension.ts, kaynak backend/crates/kor-tools/src/tools/browser/snapshot.js. Elle düzenlemeyin.
export function snapshot() {
  return (() => {
  const MAX = 400;
  const TEXT_CAP = 200000;
  const nodes = [];
  let n = 0;
  for (const old of document.querySelectorAll('[data-kor-ref]')) old.removeAttribute('data-kor-ref');
  const clean = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const labelFor = (el) => {
    if (el.id) {
      const l = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
      if (l) return clean(l.innerText);
    }
    const p = el.closest('label');
    return p ? clean(p.innerText) : '';
  };
  const byIds = (ids) => clean(ids.split(/\s+/).map((id) => (document.getElementById(id) || {}).innerText || '').join(' '));
  const nameOf = (el, role) => {
    const a = el.getAttribute('aria-label');
    if (a) return clean(a);
    const lb = el.getAttribute('aria-labelledby');
    if (lb) {
      const t = byIds(lb);
      if (t) return t;
    }
    if (/^(textbox|combobox|checkbox|radio|slider)$/.test(role)) {
      const l = labelFor(el);
      if (l) return l;
    }
    if (el.tagName === 'IMG') return clean(el.alt);
    if (el.tagName === 'INPUT' && /^(submit|button|reset)$/.test(el.type)) return clean(el.value);
    const t = clean(el.innerText || el.textContent);
    if (t) return t.slice(0, 120);
    return clean(el.placeholder || el.title || el.getAttribute('name') || '');
  };
  const roleOf = (el) => {
    const r = el.getAttribute('role');
    if (r) return r;
    const t = el.tagName.toLowerCase();
    if (t === 'a') return el.hasAttribute('href') ? 'link' : '';
    if (t === 'button' || t === 'summary') return 'button';
    if (t === 'input') {
      const ty = (el.type || 'text').toLowerCase();
      if (ty === 'hidden') return '';
      if (/^(button|submit|reset|image)$/.test(ty)) return 'button';
      if (ty === 'checkbox') return 'checkbox';
      if (ty === 'radio') return 'radio';
      if (ty === 'range') return 'slider';
      if (ty === 'file') return 'filebutton';
      return 'textbox';
    }
    if (t === 'textarea') return 'textbox';
    if (t === 'select') return 'combobox';
    if (/^h[1-6]$/.test(t)) return 'heading';
    if (t === 'img') return el.alt ? 'img' : '';
    if (el.isContentEditable) return 'textbox';
    if (el.hasAttribute('onclick') || el.getAttribute('tabindex') === '0') return 'button';
    return '';
  };
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (!r.width && !r.height) return null;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || cs.opacity === '0') return null;
    return r;
  };
  const vh = window.innerHeight;
  const vw = window.innerWidth;
  const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_ELEMENT);
  let el;
  while ((el = walker.nextNode())) {
    if (n >= MAX) break;
    const role = roleOf(el);
    if (!role) continue;
    if (el.closest('[aria-hidden="true"]')) continue;
    const r = visible(el);
    if (!r) continue;
    const ref = 'e' + ++n;
    el.setAttribute('data-kor-ref', ref);
    const node = { ref, role, name: nameOf(el, role) };
    if (role === 'textbox' || role === 'combobox') {
      const ty = (el.type || '').toLowerCase();
      if (ty === 'password') {
        node.value = el.value ? '****' : '';
        node.password = true;
      } else {
        node.value = clean(el.value || (el.isContentEditable ? el.innerText : '')).slice(0, 80);
      }
      if (el.placeholder) node.placeholder = clean(el.placeholder).slice(0, 60);
    }
    if (role === 'checkbox' || role === 'radio' || role === 'switch') {
      node.checked = el.checked === true || el.getAttribute('aria-checked') === 'true';
    }
    if (role === 'combobox' && el.tagName === 'SELECT') {
      node.options = Array.from(el.options).slice(0, 30).map((o) => clean(o.textContent) + (o.selected ? ' *' : ''));
    }
    if (role === 'link') {
      const h = el.getAttribute('href') || '';
      if (h && !h.startsWith('javascript:')) node.href = h.slice(0, 200);
    }
    if (role === 'heading') node.level = Number(el.tagName[1]) || Number(el.getAttribute('aria-level')) || 2;
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') node.disabled = true;
    if (r.bottom < 0 || r.top > vh || r.right < 0 || r.left > vw) node.offscreen = true;
    nodes.push(node);
  }
  const text = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').slice(0, TEXT_CAP);
  return {
    title: document.title,
    url: location.href,
    text,
    nodes,
    scroll: { y: Math.round(window.scrollY), height: Math.round(document.documentElement.scrollHeight), viewport: vh },
  };
})()
}
