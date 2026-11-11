// Arayüz dili (tr | en | fr). Kaynak metin Türkçedir ve kataloglarda anahtardır (gettext tarzı):
// en.json / fr.json, `scripts/i18n-extract.ts` ile kaynaktan üretilir. Ekrana yazılan metin düğümleri ve
// placeholder/title/aria-label/alt öznitelikleri katalogdan çevrilir; kod mantığı ve bileşenler değişmez.
// Kullanıcı ve model içeriği çevrilmez: `translate="no"` (HTML standardı) ve CONTENT seçicileri.
import en from './en.json' with { type: 'json' }
import fr from './fr.json' with { type: 'json' }

export type UiLang = 'tr' | 'en' | 'fr'

/** Çevrilmeyen bölgeler: kullanıcı/model içeriği, kod, düzenleyiciler, dosya ve oturum adları. */
const CONTENT = [
  '[translate="no"]',
  'script',
  'style',
  'textarea',
  'pre',
  'code',
  'kbd',
  '[contenteditable]',
  '.cm-editor',
  '.prose',
  '.user-bubble',
  '.tree-name',
  '.session-title',
  '.tool-summary',
  '.tool-diff-path',
  '.tool-diff-body',
  '.approval-subject',
].join(', ')

const ATTRS = ['placeholder', 'title', 'aria-label', 'alt'] as const

interface Pattern {
  re: RegExp
  /** Kaynak anahtar: yer tutucu adları (sırayla yakalama grupları) buradan okunur. */
  key: string
  out: string
}

interface Compiled {
  exact: Map<string, string>
  patterns: Pattern[]
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** `{0} dosya değişti` → /^(.+?) dosya değişti$/; yalnızca sayısal yer tutucular kalıptır. */
export function compile(catalog: Readonly<Record<string, string>>): Compiled {
  const exact = new Map<string, string>()
  const patterns: Array<Pattern & { weight: number }> = []
  for (const [key, value] of Object.entries(catalog)) {
    if (!value) continue
    exact.set(key, value)
    if (!/\{\d+\}/.test(key)) continue
    const fixed = key.replace(/\{\d+\}/g, '')
    // Sabit kısmı harfsiz kalıp ("{0}", "{0} · {1}") her metni yakalar; çevrilecek bir şey yoktur.
    if (!/\p{L}{2,}/u.test(fixed)) continue
    const source = key
      .split(/(\{\d+\})/)
      .map((part) => (/^\{\d+\}$/.test(part) ? '(.+?)' : escapeRe(part)))
      .join('')
    patterns.push({ re: new RegExp(`^${source}$`, 'su'), key, out: value, weight: fixed.length })
  }
  // Daha uzun sabit metinli kalıp önce: "{0} dosya değişti", "{0} değişti"den önce denenir.
  patterns.sort((a, b) => b.weight - a.weight)
  return { exact, patterns: patterns.map(({ re, key, out }) => ({ re, key, out })) }
}

/** Kalıp yer tutucularının sırası anahtardaki sıradır ({0}, {1}…); çeviri onları istediği sırada kullanır. */
function fill(p: Pattern, m: RegExpExecArray): string {
  let i = 0
  const slots = new Map<string, string>()
  for (const name of p.key.match(/\{\d+\}/g) ?? []) slots.set(name, m[++i] ?? '')
  return p.out.replace(/\{\d+\}/g, (name) => slots.get(name) ?? name)
}

const compiled: Partial<Record<UiLang, Compiled>> = {}

function catalogFor(lang: UiLang): Compiled | null {
  if (lang === 'tr') return null
  const c = (compiled[lang] ??= compile((lang === 'en' ? en : fr) as Record<string, string>))
  return c
}

/** Tek bir metni çevirir; baştaki/sondaki boşluk korunur, eşleşme yoksa kaynak döner. */
export function translateText(text: string, lang: UiLang): string {
  const c = catalogFor(lang)
  if (!c) return text
  const lead = /^\s*/.exec(text)?.[0] ?? ''
  const trail = /\s*$/.exec(text)?.[0] ?? ''
  const core = text.slice(lead.length, text.length - trail.length).replace(/\s+/g, ' ')
  if (!core || !/\p{L}/u.test(core)) return text
  const hit = c.exact.get(core)
  if (hit) return lead + hit + trail
  for (const p of c.patterns) {
    const m = p.re.exec(core)
    if (m) return lead + fill(p, m) + trail
  }
  return text
}

// ---------------------------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------------------------

interface Rec {
  source: string
  shown: string
}

let current: UiLang = 'tr'
const texts = new WeakMap<Text, Rec>()
const attrs = new WeakMap<Element, Map<string, Rec>>()
let observer: MutationObserver | null = null
let titleSource: string | null = null

function excluded(el: Element | null): boolean {
  return el !== null && el.closest(CONTENT) !== null
}

function visitText(node: Text): void {
  if (excluded(node.parentElement)) return
  const now = node.nodeValue ?? ''
  const rec = texts.get(node)
  // Bizim yazdığımız değer: kaynak aynıdır. Başka bir değer: React yeni kaynak yazdı.
  const source = rec && now === rec.shown ? rec.source : now
  const shown = translateText(source, current)
  if (shown !== now) node.nodeValue = shown
  texts.set(node, { source, shown })
}

function visitAttr(el: Element, name: string): void {
  const now = el.getAttribute(name)
  // Form alanının içeriği kullanıcınındır ama placeholder/title arayüz metnidir: alanın kendisine değil
  // çevresine bakılır.
  const host = el.matches('textarea, input, [contenteditable]') ? el.parentElement : el
  if (now === null || excluded(host)) return
  let map = attrs.get(el)
  const rec = map?.get(name)
  const source = rec && now === rec.shown ? rec.source : now
  const shown = translateText(source, current)
  if (shown !== now) el.setAttribute(name, shown)
  if (!map) {
    map = new Map()
    attrs.set(el, map)
  }
  map.set(name, { source, shown })
}

function visitTree(root: Node): void {
  if (root.nodeType === Node.TEXT_NODE) {
    visitText(root as Text)
    return
  }
  if (!(root instanceof Element)) return
  for (const a of ATTRS) if (root.hasAttribute(a)) visitAttr(root, a)
  if (excluded(root)) return
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => {
      if (!(n instanceof Element) || n === root || !n.matches(CONTENT)) return NodeFilter.FILTER_ACCEPT
      // İçerik bölgesine girilmez; yalnızca form alanının kendi öznitelikleri (placeholder) çevrilir.
      for (const a of ATTRS) if (n.hasAttribute(a)) visitAttr(n, a)
      return NodeFilter.FILTER_REJECT
    },
  })
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) visitText(n as Text)
    else for (const a of ATTRS) if ((n as Element).hasAttribute(a)) visitAttr(n as Element, a)
  }
}

function syncTitle(): void {
  if (titleSource === null || document.title !== translateText(titleSource, current)) titleSource = document.title
  document.title = translateText(titleSource, current)
}

export function getUiLanguage(): UiLang {
  return current
}

const listeners = new Set<(lang: UiLang) => void>()

/** Dil değişince çağrılır (ör. ayar ekranındaki seçicinin durumu). */
export function onUiLanguage(listener: (lang: UiLang) => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** Dili uygular: tüm belge yeniden çevrilir; sonraki değişiklikler gözlemciyle anında çevrilir. */
export function setUiLanguage(lang: UiLang): void {
  current = lang
  document.documentElement.lang = lang
  visitTree(document.body)
  syncTitle()
  if (!observer) {
    observer = new MutationObserver((records) => {
      for (const r of records) {
        if (r.type === 'characterData') visitText(r.target as Text)
        else if (r.type === 'attributes' && r.attributeName) visitAttr(r.target as Element, r.attributeName)
        else for (const n of r.addedNodes) visitTree(n)
      }
    })
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: [...ATTRS],
    })
  }
  for (const l of listeners) l(lang)
}
