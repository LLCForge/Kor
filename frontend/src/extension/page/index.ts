// Sayfa betiği (ADR 0058): `page.js` olarak sekmeye (izole dünya) bir kez enjekte edilir ve `globalThis.__korPage`
// altında işlevler sunar. Arka plan `scripting.executeScript({ func: (n, a) => __korPage[n](...a) })` ile çağırır.
// Sayfa içeriği yalnızca veridir; burada sayfadaki hiçbir metin talimat olarak yorumlanmaz. Parola, kart, CVV ve
// tek kullanımlık kod alanlarına hiçbir zaman yazılmaz; form hiçbir zaman Kor tarafından gönderilmez (fill + submit
// yalnızca araç onayıyla ve kullanıcının izniyle Enter'dır, form doldurucu hiç göndermez).
import { sensitive, type FieldDesc } from '../fields.ts'
import { snapshot } from './snapshot.gen.js'

type Result = { ok: true; [k: string]: unknown } | { ok: false; error: string }

const fail = (error: string): Result => ({ ok: false, error })

function byRef(ref: string, attr = 'data-kor-ref'): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[${attr}="${CSS.escape(ref)}"]`)
}

function labelOf(el: Element): string {
  const parts: string[] = []
  const aria = el.getAttribute('aria-label')
  if (aria) parts.push(aria)
  const labelledby = el.getAttribute('aria-labelledby')
  if (labelledby) parts.push(labelledby.split(/\s+/).map((id) => document.getElementById(id)?.innerText ?? '').join(' '))
  if (el.id) {
    const l = document.querySelector<HTMLLabelElement>(`label[for="${CSS.escape(el.id)}"]`)
    if (l) parts.push(l.innerText)
  }
  const wrap = el.closest('label')
  if (wrap) parts.push(wrap.innerText)
  return parts.join(' ').replace(/\s+/g, ' ').trim().slice(0, 200)
}

function describe(el: HTMLElement): FieldDesc {
  const input = el as HTMLInputElement
  return {
    type: el.tagName === 'TEXTAREA' ? 'textarea' : el.tagName === 'SELECT' ? 'select' : (input.type || 'text').toLowerCase(),
    autocomplete: el.getAttribute('autocomplete') ?? '',
    label: labelOf(el),
    name: el.getAttribute('name') ?? '',
    id: el.id,
    placeholder: el.getAttribute('placeholder') ?? '',
  }
}

function setValue(el: HTMLElement, value: string): void {
  if (el.isContentEditable) {
    el.focus()
    document.execCommand('selectAll')
    document.execCommand('insertText', false, value)
    return
  }
  const proto = Object.getPrototypeOf(el) as object
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  el.focus()
  // React/Vue değeri prototipteki setter'dan izler; doğrudan atama onların durumunu güncellemez.
  if (setter) setter.call(el, value)
  else (el as HTMLInputElement).value = value
  el.dispatchEvent(new Event('input', { bubbles: true }))
  el.dispatchEvent(new Event('change', { bubbles: true }))
}

function mouse(el: HTMLElement, types: string[], button = 0): void {
  const r = el.getBoundingClientRect()
  const init = { bubbles: true, cancelable: true, composed: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, button, view: window }
  for (const t of types) {
    const E = t.startsWith('pointer') ? PointerEvent : MouseEvent
    el.dispatchEvent(new E(t, init))
  }
}

function key(target: Element, combo: string): void {
  const parts = combo.split('+')
  const k = parts.pop() ?? ''
  const mods = new Set(parts.map((p) => p.toLowerCase()))
  const init = {
    key: k.length === 1 ? k : k,
    bubbles: true,
    cancelable: true,
    ctrlKey: mods.has('control') || mods.has('ctrl'),
    shiftKey: mods.has('shift'),
    altKey: mods.has('alt'),
    metaKey: mods.has('meta') || mods.has('cmd'),
  }
  target.dispatchEvent(new KeyboardEvent('keydown', init))
  if (k.length === 1) target.dispatchEvent(new KeyboardEvent('keypress', init))
  target.dispatchEvent(new KeyboardEvent('keyup', init))
}

function focusables(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('a[href],button,input,select,textarea,[tabindex]:not([tabindex="-1"]),[contenteditable="true"]')].filter(
    (e) => !(e as HTMLInputElement).disabled && e.getClientRects().length > 0,
  )
}

function act(action: string, a: Record<string, unknown>): Result | Promise<Result> {
  const ref = typeof a.ref === 'string' ? a.ref : ''
  const el = ref ? byRef(ref) : null
  if (ref && !el) return fail(`Sayfada ${ref} yok; referanslar gezinmeden sonra değişir, browser_read ile yeniden okuyun`)
  switch (action) {
    case 'click': {
      if (!el) return fail('ref gerekli')
      el.scrollIntoView({ block: 'center', inline: 'center' })
      const button = a.button === 'right' ? 2 : a.button === 'middle' ? 1 : 0
      if (button === 2) mouse(el, ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'contextmenu'], 2)
      else {
        mouse(el, ['pointerover', 'mouseover', 'pointerdown', 'mousedown', 'pointerup', 'mouseup'], button)
        if (button === 0) el.click()
        if (a.double === true) {
          el.click()
          mouse(el, ['dblclick'])
        }
      }
      return { ok: true, url: location.href }
    }
    case 'fill': {
      if (!el) return fail('ref gerekli')
      if (sensitive(describe(el))) return fail('Güvenlik: parola, kart, CVV veya tek kullanımlık kod alanlarına Kor yazmaz; kullanıcı kendisi girer.')
      const text = typeof a.text === 'string' ? a.text : ''
      el.scrollIntoView({ block: 'center' })
      const current = (el as HTMLInputElement).value ?? el.innerText
      setValue(el, a.clear === false ? current + text : text)
      if (a.submit === true) {
        key(el, 'Enter')
        const form = (el as HTMLInputElement).form
        if (form) form.requestSubmit()
      }
      return { ok: true }
    }
    case 'press': {
      const combo = typeof a.key === 'string' ? a.key : ''
      const target = document.activeElement ?? document.body
      key(target, combo)
      if (combo === 'Tab' || combo === 'Shift+Tab') {
        const list = focusables()
        const i = list.indexOf(target as HTMLElement)
        const next = list[(i + (combo === 'Tab' ? 1 : -1) + list.length) % list.length]
        next?.focus()
      } else if (combo === 'Enter' && target instanceof HTMLInputElement && target.form) {
        target.form.requestSubmit()
      }
      return { ok: true }
    }
    case 'scroll': {
      if (el) {
        el.scrollIntoView({ block: 'center' })
        return { ok: true, message: `${ref} görünür alana kaydırıldı` }
      }
      const amount = typeof a.amount === 'number' ? a.amount : Math.round(innerHeight * 0.9)
      const dir = typeof a.direction === 'string' ? a.direction : 'down'
      if (dir === 'down') scrollBy(0, amount)
      else if (dir === 'up') scrollBy(0, -amount)
      else if (dir === 'top') scrollTo(0, 0)
      else if (dir === 'bottom') scrollTo(0, document.documentElement.scrollHeight)
      else return fail('direction: down, up, top veya bottom')
      return { ok: true, message: `scrolled ${dir}; position ${Math.round(scrollY)}/${document.documentElement.scrollHeight}` }
    }
    case 'select': {
      if (!(el instanceof HTMLSelectElement)) return fail('Bir <select> değil')
      const want = typeof a.value === 'string' ? a.value : ''
      const o = [...el.options].find((x) => x.value === want) ?? [...el.options].find((x) => x.textContent?.trim() === want.trim())
      if (!o) return fail(`Seçenek yok: ${want}`)
      setValue(el, o.value)
      return { ok: true, message: `selected ${o.textContent?.trim() ?? o.value}` }
    }
    case 'hover': {
      if (!el) return fail('ref gerekli')
      el.scrollIntoView({ block: 'center' })
      mouse(el, ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'pointermove', 'mousemove'])
      return { ok: true }
    }
    case 'hasText': {
      const t = typeof a.text === 'string' ? a.text : ''
      return { ok: true, found: (document.body?.innerText ?? '').includes(t) }
    }
    case 'upload': {
      if (!(el instanceof HTMLInputElement) || el.type !== 'file') return fail(`${ref} bir dosya girişi değil`)
      const files = Array.isArray(a.files) ? (a.files as Array<{ name: string; mime: string; data: string }>) : []
      const dt = new DataTransfer()
      for (const f of files) {
        const bin = atob(f.data)
        const bytes = new Uint8Array(bin.length)
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
        dt.items.add(new File([bytes], f.name, { type: f.mime }))
      }
      el.files = dt.files
      el.dispatchEvent(new Event('input', { bubbles: true }))
      el.dispatchEvent(new Event('change', { bubbles: true }))
      return { ok: true }
    }
    default:
      return fail(`Bilinmeyen işlem: ${action}`)
  }
}

// §27.12 form doldurucu: alanlar `data-kor-form-ref` ile işaretlenir.
function readForm(): Result {
  let n = 0
  const fields = []
  for (const el of document.querySelectorAll<HTMLElement>('input, textarea, select, [contenteditable="true"]')) {
    if (!el.getClientRects().length) continue
    const d = describe(el)
    if (['submit', 'button', 'reset', 'image', 'checkbox', 'radio', 'range', 'color'].includes(d.type)) continue
    if (n >= 150) break
    const ref = 'f' + ++n
    el.setAttribute('data-kor-form-ref', ref)
    const options = el instanceof HTMLSelectElement ? [...el.options].slice(0, 50).map((o) => o.textContent?.trim() ?? o.value) : []
    fields.push({ ref, label: d.label, name: d.name, id: d.id, placeholder: d.placeholder, autocomplete: d.autocomplete, kind: d.type, options, sensitive: sensitive(d) })
  }
  return { ok: true, fields, title: document.title, url: location.href }
}

function fillForm(fills: Array<{ ref: string; value: string }>): Result {
  const filled: string[] = []
  const refused: string[] = []
  const missing: string[] = []
  for (const f of fills) {
    const el = byRef(f.ref, 'data-kor-form-ref')
    if (!el) {
      missing.push(f.ref)
      continue
    }
    if (sensitive(describe(el))) {
      refused.push(f.ref)
      continue
    }
    if (el instanceof HTMLSelectElement) {
      const o = [...el.options].find((x) => x.value === f.value || x.textContent?.trim() === f.value.trim())
      if (!o) {
        missing.push(f.ref)
        continue
      }
      setValue(el, o.value)
    } else setValue(el, f.value)
    // Doldurulan alan vurgulanır; kullanıcı inceleyip gönder'e kendisi basar.
    el.style.outline = '2px solid Highlight' // sistem vurgu rengi; sayfanın kendi temasıyla çakışmaz
    el.style.outlineOffset = '1px'
    filled.push(f.ref)
  }
  return { ok: true, filled, refused, missing }
}

function formValues(refs: string[]): Result {
  const values: Record<string, string> = {}
  for (const r of refs) {
    const el = byRef(r, 'data-kor-form-ref')
    if (el) values[r] = el.isContentEditable ? el.innerText : (el as HTMLInputElement).value
  }
  return { ok: true, values }
}

function pageText(): Result {
  return { ok: true, url: location.href, title: document.title, text: (document.body?.innerText ?? '').slice(0, 60000), selection: String(getSelection() ?? '').slice(0, 20000) }
}

// §29.2 hosting paneli: etikete göre doldurma (kor_web panel haritaları). Gönderen kullanıcıdır.
function panelFill(fields: Array<{ labels: string[]; value: string }>): Result {
  const inputs = [...document.querySelectorAll<HTMLInputElement>('input, textarea, select')].filter((e) => e.getClientRects().length && !sensitive(describe(e)) && !['submit', 'button'].includes((e.type || '').toLowerCase()))
  const label = (e: HTMLElement) => [describe(e).label, e.getAttribute('placeholder'), e.getAttribute('name'), e.id].filter(Boolean).join(' ').toLowerCase()
  const filled: string[] = []
  const missing: string[] = []
  const used = new Set<HTMLElement>()
  for (const f of fields) {
    const el = inputs.find((e) => !used.has(e) && f.labels.some((l) => label(e).includes(l)))
    if (!el) {
      missing.push(f.labels[0] ?? '')
      continue
    }
    used.add(el)
    setValue(el, f.value)
    filled.push(f.labels[0] ?? '')
  }
  return { ok: true, filled, missing }
}

// §27.13 "Bileşen seç": imleç altındaki eleman vurgulanır, tıklanan elemanın yapısı yakalanır.
const STYLE_PROPS = [
  'display', 'position', 'flex-direction', 'flex-wrap', 'justify-content', 'align-items', 'gap', 'grid-template-columns', 'grid-template-rows',
  'width', 'height', 'max-width', 'margin', 'padding', 'border', 'border-radius', 'background-color', 'color', 'font-family', 'font-size',
  'font-weight', 'line-height', 'letter-spacing', 'text-align', 'text-transform', 'box-shadow', 'opacity', 'cursor', 'overflow',
]
const DEFAULTS = new Set(['none', 'normal', '0px', 'auto', 'static', 'visible', 'rgba(0, 0, 0, 0)', 'start', 'nowrap', 'stretch', '0px none rgb(0, 0, 0)'])

function capture(target: HTMLElement): Result {
  const nodes = [target, ...target.querySelectorAll<HTMLElement>('*')].slice(0, 300)
  nodes.forEach((n, i) => n.setAttribute('data-k', String(i)))
  const styles: string[] = []
  const a11y: string[] = []
  for (const n of nodes) {
    const cs = getComputedStyle(n)
    const props = STYLE_PROPS.map((p) => [p, cs.getPropertyValue(p)] as const).filter(([, v]) => v && !DEFAULTS.has(v))
    if (props.length) styles.push(`[data-k="${n.dataset.k ?? ''}"] { ${props.map(([p, v]) => `${p}: ${v}`).join('; ')} }`)
    const role = n.getAttribute('role') ?? ({ BUTTON: 'button', A: 'link', INPUT: 'textbox', H1: 'heading', H2: 'heading', H3: 'heading', NAV: 'navigation', IMG: 'img', UL: 'list', LI: 'listitem' } as Record<string, string>)[n.tagName]
    if (role) a11y.push(`${role}: ${(n.getAttribute('aria-label') ?? n.getAttribute('alt') ?? n.innerText ?? '').replace(/\s+/g, ' ').trim().slice(0, 80)}`)
  }
  const states: string[] = []
  for (const sheet of [...document.styleSheets]) {
    let rules: CSSRuleList
    try {
      rules = sheet.cssRules
    } catch {
      continue // başka kökenden stil sayfası okunamaz
    }
    for (const r of [...rules]) {
      if (!(r instanceof CSSStyleRule) || !/:(hover|focus|focus-visible|active)/.test(r.selectorText)) continue
      const base = r.selectorText.replace(/:(hover|focus-visible|focus|active)/g, '')
      try {
        if (nodes.some((n) => n.matches(base))) states.push(r.cssText.replace(/url\([^)]*\)/g, 'none'))
      } catch {
        // geçersiz seçici
      }
      if (states.join('').length > 8000) break
    }
  }
  const clone = target.cloneNode(true) as HTMLElement
  const removed: string[] = []
  for (const x of clone.querySelectorAll('script, style, iframe, noscript')) x.remove()
  for (const x of clone.querySelectorAll('img, picture, video, canvas, svg, object, embed')) {
    removed.push(`${x.tagName.toLowerCase()}: ${(x.getAttribute('alt') ?? x.getAttribute('src') ?? '').slice(0, 80)}`)
    const ph = document.createElement('span')
    ph.setAttribute('data-kor-placeholder', x.tagName === 'svg' ? 'icon' : 'image')
    ph.setAttribute('data-k', x.getAttribute('data-k') ?? '')
    x.replaceWith(ph)
  }
  for (const x of [clone, ...clone.querySelectorAll('*')]) {
    for (const at of [...x.attributes]) if (/^on/i.test(at.name) || at.name === 'srcset' || at.name === 'src') x.removeAttribute(at.name)
    const st = x.getAttribute('style')
    if (st && /url\(/i.test(st)) x.setAttribute('style', st.replace(/url\([^)]*\)/gi, 'none'))
  }
  nodes.forEach((n) => n.removeAttribute('data-k'))
  const rect = target.getBoundingClientRect()
  return {
    ok: true,
    url: location.href,
    title: document.title,
    html: clone.outerHTML.slice(0, 60000),
    styles: styles.join('\n').slice(0, 30000),
    a11y: a11y.join('\n').slice(0, 6000),
    states: states.join('\n').slice(0, 8000),
    removed,
    rect: { x: rect.left, y: rect.top, w: rect.width, h: rect.height },
    dpr: devicePixelRatio,
  }
}

function pick(): Promise<Result> {
  return new Promise((resolve) => {
    const box = document.createElement('div')
    box.style.cssText = `position:fixed;z-index:2147483647;pointer-events:none;outline:2px solid Highlight;background:color-mix(in srgb, Highlight 12%, transparent);border-radius:4px;transition:all 60ms`
    document.documentElement.append(box)
    let current: HTMLElement | null = null
    const move = (e: MouseEvent) => {
      const el = document.elementFromPoint(e.clientX, e.clientY)
      if (!(el instanceof HTMLElement) || el === box) return
      current = el
      const r = el.getBoundingClientRect()
      Object.assign(box.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` })
    }
    const done = (r: Result) => {
      removeEventListener('mousemove', move, true)
      removeEventListener('click', click, true)
      removeEventListener('keydown', esc, true)
      box.remove()
      resolve(r)
    }
    const click = (e: MouseEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (current) {
        box.remove()
        done(capture(current))
      }
    }
    const esc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') done(fail('Seçim iptal edildi'))
    }
    addEventListener('mousemove', move, true)
    addEventListener('click', click, true)
    addEventListener('keydown', esc, true)
  })
}

const api = { snapshot: () => ({ ok: true, snapshot: snapshot() }) as Result, act, readForm, fillForm, formValues, pageText, panelFill, pick }

;(globalThis as unknown as { __korPage?: typeof api }).__korPage ??= api
