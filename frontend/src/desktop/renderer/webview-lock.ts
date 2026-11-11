// WebView kilidi (spec §21, ADR 0053): masaüstü ve Chrome yan paneli hiçbir zaman "içinde tarayıcı var" hissi
// vermez. Tek yer: tarayıcı kısayolları, varsayılan sağ tık menüsü, dosya/link bırakınca gezinme, harici link ve
// window.open, view-source:/about:/file: şemaları, beforeunload iletişim kutusu. Rust tarafı (src-tauri
// webview_lock.rs) aynı listeyi gezinme düzeyinde uygular ve devtools'u kapatır.
//
// Bu modülün üst kısmı saf ve DOM'suzdur (node --test: webview-lock.test.ts); `installWebviewLock` olayları bağlar.
// Chrome eklentisinin panel sayfası aynı politikayı frontend/extension/webview-lock.js ile uygular.

/** Klavye olayının politikanın ihtiyaç duyduğu kısmı. */
export interface KeyLike {
  key: string
  code?: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
}

export interface KeyContext {
  /** Odak bir metin alanında/düzenleyicide (Backspace, Ctrl+A/Z gibi düzenleme tuşları serbest). */
  editable: boolean
  /** macOS: Cmd birincil değiştiricidir. */
  mac: boolean
  /** Geliştirici build'i (`KOR_DEVTOOLS=1` / `--devtools`): devtools kısayolları serbest. */
  devtools: boolean
}

/** Kor'un tarayıcı kısayollarının yerine koyduğu eylemler. */
export type Redirect = 'export' | 'save' | 'find' | 'zoomIn' | 'zoomOut' | 'zoomReset'

export type KeyDecision = { kind: 'allow' } | { kind: 'block'; reason: string } | { kind: 'redirect'; to: Redirect }

const ALLOW: KeyDecision = { kind: 'allow' }
const block = (reason: string): KeyDecision => ({ kind: 'block', reason })
const redirect = (to: Redirect): KeyDecision => ({ kind: 'redirect', to })

/**
 * §21.1 listesi. Engellenen tuşta yalnızca `preventDefault` çağrılır (yayılma durmaz): uygulamanın kendi
 * kısayolları (Ctrl+F konuşma araması, Ctrl+P/S dışa aktarma, Ctrl+K palet) aynı olayı yine alır.
 */
export function keyDecision(e: KeyLike, ctx: KeyContext): KeyDecision {
  const primary = ctx.mac ? e.metaKey : e.ctrlKey
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key
  // Geliştirici araçları: F12, Ctrl/Cmd+Shift+I/J/C, Cmd+Alt+I/J/C (macOS), Ctrl+U (kaynağı göster).
  // Terminal/düzenleyicide Ctrl+Shift+C kopyalamadır; orada serbest kalır.
  const devKey =
    key === 'F12' ||
    (primary && (e.shiftKey || (ctx.mac && e.altKey)) && ['i', 'j', 'c'].includes(key) && !(key === 'c' && ctx.editable)) ||
    (primary && !e.shiftKey && !e.altKey && key === 'u')
  if (devKey) return ctx.devtools ? ALLOW : block('devtools')
  // Yenile.
  if (key === 'F5' || (primary && key === 'r')) return block('reload')
  // Geri/ileri: Alt+←/→, Cmd+[ / ], fare yan tuşları (BrowserBack/Forward), düzenleyici dışında Backspace.
  if (e.altKey && !primary && (key === 'ArrowLeft' || key === 'ArrowRight')) return ctx.editable ? ALLOW : block('history')
  if (ctx.mac && e.metaKey && (key === '[' || key === ']')) return block('history')
  if (key === 'BrowserBack' || key === 'BrowserForward') return block('history')
  if (key === 'Backspace' && !ctx.editable) return block('history')
  // Yazdır → Kor'un dışa aktarma diyaloğu; Farklı kaydet → oturumu kaydet; Bul → Kor araması.
  if (primary && !e.altKey && key === 'p' && !e.shiftKey) return redirect('export')
  if (primary && !e.altKey && key === 's') return redirect('save')
  if (primary && !e.altKey && key === 'f' && !e.shiftKey) return redirect('find')
  if (primary && (key === 'g' || key === 'F3') && !ctx.editable) return block('find-next')
  if (key === 'F3') return block('find-next')
  // Sekme/pencere: Ctrl+T/N/W, Ctrl+Shift+T/N/W, Ctrl+Tab.
  if (primary && ['t', 'n', 'w'].includes(key)) return block('tabs')
  if (e.ctrlKey && key === 'Tab') return block('tabs')
  // Sayfa yakınlaştırma → Kor yazı boyutu.
  if (primary && (key === '+' || key === '=' || e.code === 'NumpadAdd')) return redirect('zoomIn')
  if (primary && (key === '-' || e.code === 'NumpadSubtract')) return redirect('zoomOut')
  if (primary && (key === '0' || e.code === 'Numpad0')) return redirect('zoomReset')
  // Tam ekran, gezinme geçmişini temizle, ana sayfa, adres çubuğu, yer imleri, indirmeler.
  if (key === 'F11' || key === 'F7' || key === 'BrowserHome' || key === 'BrowserRefresh' || key === 'BrowserSearch') return block('browser')
  if (primary && e.shiftKey && key === 'Delete') return block('browser')
  if (primary && ['l', 'd', 'h', 'j', 'o', 'e'].includes(key) && !ctx.editable && !e.shiftKey) return block('browser')
  if (e.altKey && !primary && (key === 'Home' || key === 'd')) return block('browser')
  return ALLOW
}

/** Sağ tık hedefinin anlamı (en özelden genele). */
export type MenuContext =
  | { kind: 'editable'; hasSelection: boolean; readOnly: boolean }
  | { kind: 'code'; hasSelection: boolean }
  | { kind: 'image'; src: string }
  | { kind: 'link'; href: string; hasSelection: boolean }
  | { kind: 'message'; role: 'assistant' | 'user'; hasSelection: boolean }
  | { kind: 'selection' }
  | { kind: 'none' }

export type MenuAction =
  | 'cut' | 'copy' | 'paste' | 'selectAll'
  | 'copyCode' | 'saveCode' | 'applyCode'
  | 'quote' | 'regenerate' | 'reference'
  | 'saveImage' | 'referenceImage' | 'copyImage'
  | 'openLink' | 'copyLink'

export interface MenuItem {
  action: MenuAction
  label: string
  shortcut?: string
  disabled?: boolean
}

/** §21.1: her bağlamda Kor'un kendi menüsünün öğeleri; `none` için menü açılmaz (boş liste). */
export function menuItems(ctx: MenuContext, mac: boolean): MenuItem[] {
  const mod = mac ? '⌘' : 'Ctrl+'
  switch (ctx.kind) {
    case 'editable':
      return [
        { action: 'cut', label: 'Kes', shortcut: `${mod}X`, disabled: !ctx.hasSelection || ctx.readOnly },
        { action: 'copy', label: 'Kopyala', shortcut: `${mod}C`, disabled: !ctx.hasSelection },
        { action: 'paste', label: 'Yapıştır', shortcut: `${mod}V`, disabled: ctx.readOnly },
        { action: 'selectAll', label: 'Tümünü seç', shortcut: `${mod}A` },
      ]
    case 'code':
      return [
        { action: ctx.hasSelection ? 'copy' : 'copyCode', label: 'Kopyala', shortcut: ctx.hasSelection ? `${mod}C` : undefined },
        { action: 'saveCode', label: 'Dosyaya kaydet' },
        { action: 'applyCode', label: "Diff'e uygula" },
      ]
    case 'image':
      return [
        { action: 'saveImage', label: 'Kaydet' },
        { action: 'referenceImage', label: 'Referans yap' },
        { action: 'copyImage', label: 'Kopyala' },
      ]
    case 'link':
      return [
        { action: 'openLink', label: 'Bağlantıyı tarayıcıda aç' },
        { action: 'copyLink', label: 'Bağlantıyı kopyala' },
        ...(ctx.hasSelection ? [{ action: 'copy' as const, label: 'Kopyala', shortcut: `${mod}C` }] : []),
      ]
    case 'message':
      return [
        ...(ctx.hasSelection ? [{ action: 'copy' as const, label: 'Kopyala', shortcut: `${mod}C` }] : []),
        { action: 'quote', label: 'Alıntıla' },
        ...(ctx.role === 'assistant' ? [{ action: 'regenerate' as const, label: 'Yeniden üret' }] : []),
        { action: 'reference', label: 'Referans yap' },
      ]
    case 'selection':
      return [{ action: 'copy', label: 'Kopyala', shortcut: `${mod}C` }]
    case 'none':
      return []
  }
}

export type NavDecision = 'allow' | 'external' | 'block'

const APP_ORIGINS = ['tauri://localhost', 'http://tauri.localhost', 'https://tauri.localhost']

/**
 * Bir bağlantı/gezinme hedefi için karar: uygulamanın kendi sayfası ve sayfa içi çapalar `allow`, http(s)/mailto
 * sistem tarayıcısında `external`, view-source:/about:/file:/javascript:/data: ve bilinmeyen şemalar `block`.
 */
export function navDecision(href: string, base: string): NavDecision {
  const raw = href.trim()
  if (raw === '' || raw.startsWith('#')) return 'allow'
  let url: URL
  try {
    url = new URL(raw, base)
  } catch {
    return 'block'
  }
  // `tauri://` gibi özel olmayan şemalarda URL.origin "null" döner; protokol + host karşılaştırılır.
  const origin = `${url.protocol}//${url.host}`
  const b = new URL(base)
  if (APP_ORIGINS.includes(origin) || origin === `${b.protocol}//${b.host}`) return 'allow'
  if (url.protocol === 'http:' || url.protocol === 'https:' || url.protocol === 'mailto:') return 'external'
  return 'block'
}

/** Kor yazı boyutu adımları (Ctrl + / - / 0 yerine, §21.1). */
export const UI_SCALES = [0.8, 0.9, 1, 1.1, 1.25, 1.5] as const

export function nextScale(current: number, to: 'zoomIn' | 'zoomOut' | 'zoomReset'): number {
  if (to === 'zoomReset') return 1
  const i = UI_SCALES.findIndex((s) => s >= current - 1e-9)
  const at = i === -1 ? UI_SCALES.length - 1 : i
  const next = to === 'zoomIn' ? Math.min(at + 1, UI_SCALES.length - 1) : Math.max(at - 1, 0)
  return UI_SCALES[next]!
}

/** Dosya bırakmanın izinli olduğu bölge (composer, ek alanı): `data-kor-drop` taşıyan ata. */
export function dropAllowed(path: readonly { dataset?: Record<string, string | undefined> }[]): boolean {
  return path.some((el) => el.dataset?.korDrop !== undefined)
}

// ---------------------------------------------------------------------------------------------------------------
// DOM bağlama

export interface LockOptions {
  devtools: boolean
  /** Kor menüsünü açar; boş bağlamda çağrılmaz. */
  onMenu: (x: number, y: number, ctx: MenuContext, target: Element) => void
  /** Ctrl+P/S/F ve yakınlaştırma yönlendirmeleri. */
  onRedirect: (to: Redirect) => void
  /** Harici bağlantıyı sistem tarayıcısında açar. */
  openExternal: (url: string) => void
}

export const isMac = (): boolean => /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)

export function isEditable(el: Element | null): el is HTMLElement {
  if (!el) return false
  if (el instanceof HTMLTextAreaElement) return true
  if (el instanceof HTMLInputElement) return !['button', 'checkbox', 'radio', 'range', 'color', 'file', 'submit', 'reset', 'image'].includes(el.type)
  return el instanceof HTMLElement && (el.isContentEditable || el.closest('.cm-editor') !== null)
}

function selectionText(): string {
  return window.getSelection()?.toString() ?? ''
}

/** Sağ tık hedefinden bağlam: düzenleyici > kod > görsel > link > mesaj > seçim. */
export function describeTarget(target: Element): MenuContext {
  const hasSelection = selectionText().length > 0
  const editable = target.closest('textarea, input, [contenteditable="true"], .cm-editor')
  if (editable && isEditable(editable)) {
    const ro = (editable as HTMLInputElement).readOnly === true || (editable as HTMLInputElement).disabled === true
    const el = editable as HTMLInputElement
    const fieldSel = typeof el.selectionStart === 'number' && el.selectionStart !== el.selectionEnd
    return { kind: 'editable', hasSelection: fieldSel || hasSelection, readOnly: ro }
  }
  if (target.closest('.code-block, pre')) return { kind: 'code', hasSelection }
  const img = target.closest('img')
  if (img instanceof HTMLImageElement && img.closest('.messages, .media-strip, .prose, .g2-attachments')) return { kind: 'image', src: img.currentSrc || img.src }
  const link = target.closest('a[href]')
  if (link instanceof HTMLAnchorElement) return { kind: 'link', href: link.href, hasSelection }
  const msg = target.closest('.message.assistant, .message.user')
  if (msg) return { kind: 'message', role: msg.classList.contains('user') ? 'user' : 'assistant', hasSelection }
  if (hasSelection) return { kind: 'selection' }
  return { kind: 'none' }
}

/** Olay yakalayıcıları bağlar; dönen fonksiyon hepsini kaldırır. */
export function installWebviewLock(opts: LockOptions): () => void {
  const mac = isMac()
  const onKey = (e: KeyboardEvent) => {
    const d = keyDecision(e, { editable: isEditable(document.activeElement), mac, devtools: opts.devtools })
    if (d.kind === 'allow') return
    e.preventDefault()
    if (d.kind === 'redirect') {
      // find/export/save: uygulamanın kendi dinleyicileri (Thread araması, palet) olayı yine alır.
      if (d.to.startsWith('zoom')) opts.onRedirect(d.to)
      else window.dispatchEvent(new CustomEvent('kor:shortcut', { detail: d.to }))
    }
  }
  const onMenu = (e: MouseEvent) => {
    e.preventDefault()
    const t = e.target instanceof Element ? e.target : null
    if (!t) return
    const ctx = describeTarget(t)
    if (ctx.kind !== 'none') opts.onMenu(e.clientX, e.clientY, ctx, t)
  }
  const onClick = (e: MouseEvent) => {
    const a = e.target instanceof Element ? e.target.closest('a[href]') : null
    if (!(a instanceof HTMLAnchorElement)) return
    const d = navDecision(a.getAttribute('href') ?? '', location.href)
    if (d === 'allow') return
    e.preventDefault()
    if (d === 'external') opts.openExternal(a.href)
  }
  // Fare yan tuşları (geri/ileri) gezinmez.
  const onMouseUp = (e: MouseEvent) => {
    if (e.button === 3 || e.button === 4) e.preventDefault()
  }
  // Dosya/link bırakma: izinli bölge dışında WebView o dosyaya gitmez (dropEffect none).
  const onDrag = (e: DragEvent) => {
    const path = e.composedPath() as unknown as { dataset?: Record<string, string | undefined> }[]
    if (dropAllowed(path)) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'none'
  }
  // Kor kendi "çalışan görev var" diyaloğunu gösterir; tarayıcının beforeunload kutusu hiç açılmaz.
  const onBeforeUnload = (e: BeforeUnloadEvent) => {
    e.stopImmediatePropagation()
  }
  // Görsellerin ve UI kromunun varsayılan sürüklenmesi kapalı (içerik metni sürüklenebilir).
  const onDragStart = (e: DragEvent) => {
    const t = e.target
    if (t instanceof HTMLImageElement || (t instanceof HTMLAnchorElement && !t.closest('[draggable="true"]'))) e.preventDefault()
  }
  const nativeOpen = window.open
  window.open = ((url?: string | URL) => {
    const href = url === undefined ? '' : String(url)
    if (navDecision(href, location.href) === 'external') opts.openExternal(new URL(href, location.href).href)
    return null
  }) as typeof window.open
  window.onbeforeunload = null

  window.addEventListener('keydown', onKey, true)
  window.addEventListener('contextmenu', onMenu, true)
  window.addEventListener('click', onClick, true)
  window.addEventListener('auxclick', onClick, true)
  window.addEventListener('mouseup', onMouseUp, true)
  window.addEventListener('dragover', onDrag, true)
  window.addEventListener('drop', onDrag, true)
  window.addEventListener('dragstart', onDragStart, true)
  window.addEventListener('beforeunload', onBeforeUnload, true)
  return () => {
    window.open = nativeOpen
    window.removeEventListener('keydown', onKey, true)
    window.removeEventListener('contextmenu', onMenu, true)
    window.removeEventListener('click', onClick, true)
    window.removeEventListener('auxclick', onClick, true)
    window.removeEventListener('mouseup', onMouseUp, true)
    window.removeEventListener('dragover', onDrag, true)
    window.removeEventListener('drop', onDrag, true)
    window.removeEventListener('dragstart', onDragStart, true)
    window.removeEventListener('beforeunload', onBeforeUnload, true)
  }
}
