// Uygulama genelinde `title` öznitelikleri için temalı ipucu (§23.1): tarayıcının yerel ipucu yerine
// 400 ms gecikmeli, temalı `.kor-tooltip`. Öğeyi değiştirmeden çalışır: `title` üzerine gelince
// `data-kor-title`'a taşınır (yerel ipucu çıkmaz), ayrılınca geri konur. Erişilebilir ad korunur:
// görünür metni ya da `aria-label`'ı olmayan öğeye `title`, `aria-label` olarak da yazılır.

export const DELAY_MS = 400

/** İpucunun konumu: öğenin altında ortalı; sığmazsa üstte; pencere kenarlarından içeride kalır. */
export function placeTip(
  target: { left: number; top: number; width: number; height: number },
  tip: { width: number; height: number },
  view: { width: number; height: number },
  gap = 6,
): { left: number; top: number } {
  const margin = 8
  let left = target.left + target.width / 2 - tip.width / 2
  left = Math.max(margin, Math.min(left, view.width - tip.width - margin))
  let top = target.top + target.height + gap
  if (top + tip.height > view.height - margin) top = target.top - tip.height - gap
  return { left: Math.round(left), top: Math.round(Math.max(margin, top)) }
}

/** `title` taşıyan en yakın öğe (ipucu kendi bileşenini kullanan `[data-kor-tooltip]` hariç). */
function titled(el: EventTarget | null): HTMLElement | null {
  let n = el instanceof Element ? el : null
  while (n && n !== document.body) {
    if (n instanceof HTMLElement && (n.hasAttribute('title') || n.hasAttribute('data-kor-title'))) {
      return n.closest('[data-kor-tooltip]') ? null : n
    }
    n = n.parentElement
  }
  return null
}

/**
 * Yalnızca simgesi olan denetimin iki adı da olmalı: ekran okuyucu için `aria-label`, fareyle ipucu
 * için `title`. Biri eksikse diğerinden doldurulur; ipucu açıkken `title` `data-kor-title`'dadır.
 */
export function pairLabel(aria: string | null, title: string | null, held: string | null): { aria?: string; title?: string } {
  const a = aria?.trim() || null
  const t = title?.trim() || held?.trim() || null
  if (a && !t) return { title: a }
  if (t && !a) return { aria: t }
  return {}
}

const CONTROL = 'button, a[href], [role=button], [role=tab], [role=menuitem], [role=switch], [role=checkbox], [role=link]'

function nameIconControls(root: ParentNode) {
  const all = root instanceof Element && root.matches(CONTROL) ? [root, ...root.querySelectorAll(CONTROL)] : [...root.querySelectorAll(CONTROL)]
  for (const el of all) {
    if (!el.querySelector('svg') || (el as HTMLElement).innerText?.trim()) continue
    const fix = pairLabel(el.getAttribute('aria-label'), el.getAttribute('title'), el.getAttribute('data-kor-title'))
    if (fix.aria) el.setAttribute('aria-label', fix.aria)
    if (fix.title && !el.hasAttribute('data-kor-title')) el.setAttribute('title', fix.title)
  }
}

/** Katmanı kurar; geri dönen fonksiyon kaldırır. */
export function installTitleTooltips(): () => void {
  const tip = document.createElement('div')
  tip.className = 'kor-tooltip kor-tooltip-floating'
  tip.setAttribute('role', 'tooltip')
  tip.hidden = true
  document.body.appendChild(tip)
  let current: HTMLElement | null = null
  let timer: number | undefined

  const hide = () => {
    window.clearTimeout(timer)
    tip.hidden = true
    if (current?.hasAttribute('data-kor-title')) {
      current.setAttribute('title', current.getAttribute('data-kor-title') ?? '')
      current.removeAttribute('data-kor-title')
      current.removeAttribute('aria-describedby')
    }
    current = null
  }

  const show = (el: HTMLElement) => {
    if (current === el) return
    hide()
    const text = el.getAttribute('title') ?? ''
    if (!text.trim()) return
    current = el
    el.setAttribute('data-kor-title', text)
    el.removeAttribute('title')
    if (!el.getAttribute('aria-label') && !el.textContent?.trim()) el.setAttribute('aria-label', text)
    timer = window.setTimeout(() => {
      if (current !== el || !el.isConnected) return
      tip.textContent = text
      tip.id ||= `kor-tip-${Math.random().toString(36).slice(2)}`
      el.setAttribute('aria-describedby', tip.id)
      tip.hidden = false
      const r = el.getBoundingClientRect()
      const t = tip.getBoundingClientRect()
      const p = placeTip(r, t, { width: window.innerWidth, height: window.innerHeight })
      tip.style.left = `${p.left}px`
      tip.style.top = `${p.top}px`
    }, DELAY_MS)
  }

  const over = (e: Event) => {
    const el = titled(e.target)
    if (el) show(el)
    else if (current && !(e.target instanceof Node && current.contains(e.target))) hide()
  }
  const out = (e: Event) => {
    const to = (e as PointerEvent).relatedTarget
    if (current && !(to instanceof Node && current.contains(to))) hide()
  }
  const key = (e: KeyboardEvent) => {
    if (e.key === 'Escape') hide()
  }
  // Only a scroll that moves the hovered element hides the tip (other panels may scroll on their own).
  const scroll = (e: Event) => {
    const s = e.target
    if (current && (s === document || (s instanceof Node && s.contains(current)))) hide()
  }
  nameIconControls(document)
  const names = new MutationObserver((list) => {
    for (const m of list) {
      if (m.type === 'attributes') nameIconControls(m.target as Element)
      else for (const n of m.addedNodes) if (n instanceof Element) nameIconControls(n)
    }
  })
  names.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['aria-label', 'title'] })
  document.addEventListener('pointerover', over, true)
  document.addEventListener('pointerout', out, true)
  document.addEventListener('focusin', over, true)
  document.addEventListener('focusout', hide, true)
  document.addEventListener('pointerdown', hide, true)
  document.addEventListener('keydown', key, true)
  window.addEventListener('scroll', scroll, true)
  return () => {
    hide()
    names.disconnect()
    document.removeEventListener('pointerover', over, true)
    document.removeEventListener('pointerout', out, true)
    document.removeEventListener('focusin', over, true)
    document.removeEventListener('focusout', hide, true)
    document.removeEventListener('pointerdown', hide, true)
    document.removeEventListener('keydown', key, true)
    window.removeEventListener('scroll', scroll, true)
    tip.remove()
  }
}
