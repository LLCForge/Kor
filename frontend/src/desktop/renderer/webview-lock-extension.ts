// Chrome yan paneli için WebView kilidi (spec §21.2, ADR 0053): masaüstüyle aynı politika (webview-lock.ts),
// build-ui.ts bunu frontend/extension/webview-lock.js olarak paketler. Panel React kullanmadığından Kor bağlam
// menüsü burada sade DOM ile çizilir. Chrome'un kendi kısayolları (Ctrl+Shift+I, Ctrl+T, F11…) tarayıcı
// sayfaya iletmeden işlediği için engellenemez; ADR 0053 bunları listeler.
import { installWebviewLock, menuItems, nextScale, isMac, type MenuAction, type MenuContext } from './webview-lock.ts'

declare const chrome: { tabs?: { create(p: { url: string }): void } } | undefined

const SCALE_KEY = 'kor.uiScale'
let scale = 1
try {
  scale = Number(localStorage.getItem(SCALE_KEY)) || 1
} catch {
  // depolama yoksa varsayılan ölçek
}
document.documentElement.style.setProperty('zoom', String(scale))

let open: HTMLElement | null = null
const close = () => {
  open?.remove()
  open = null
}

function run(action: MenuAction, ctx: MenuContext, target: Element): void {
  const field = target.closest('textarea, input') as HTMLTextAreaElement | HTMLInputElement | null
  field?.focus()
  if (action === 'paste') {
    void navigator.clipboard.readText().then((t) => document.execCommand('insertText', false, t))
    return
  }
  if (action === 'openLink' && ctx.kind === 'link') {
    chrome?.tabs?.create({ url: ctx.href })
    return
  }
  if (action === 'copyLink' && ctx.kind === 'link') {
    void navigator.clipboard.writeText(ctx.href)
    return
  }
  if (action === 'cut' || action === 'copy' || action === 'selectAll') document.execCommand(action)
}

function showMenu(x: number, y: number, ctx: MenuContext, target: Element): void {
  close()
  // Panelde kod/mesaj/görsel eylemleri yok: yalnızca pano ve bağlantı öğeleri.
  const items = menuItems(ctx, isMac()).filter((i) => ['cut', 'copy', 'paste', 'selectAll', 'openLink', 'copyLink'].includes(i.action))
  if (items.length === 0) return
  const menu = document.createElement('div')
  menu.className = 'kor-context-menu'
  menu.setAttribute('role', 'menu')
  for (const it of items) {
    const b = document.createElement('button')
    b.type = 'button'
    b.setAttribute('role', 'menuitem')
    b.disabled = it.disabled === true
    b.textContent = it.label
    b.addEventListener('pointerdown', (e) => e.preventDefault())
    b.addEventListener('click', () => {
      close()
      run(it.action, ctx, target)
    })
    menu.append(b)
  }
  document.body.append(menu)
  const r = menu.getBoundingClientRect()
  menu.style.left = `${Math.max(4, Math.min(x, innerWidth - r.width - 4))}px`
  menu.style.top = `${Math.max(4, Math.min(y, innerHeight - r.height - 4))}px`
  open = menu
}

addEventListener('pointerdown', (e) => {
  if (open && !open.contains(e.target as Node)) close()
}, true)
addEventListener('keydown', (e) => {
  if (e.key === 'Escape') close()
}, true)
addEventListener('blur', close)

installWebviewLock({
  devtools: false,
  onMenu: showMenu,
  onRedirect: (to) => {
    if (to === 'zoomIn' || to === 'zoomOut' || to === 'zoomReset') {
      scale = nextScale(scale, to)
      document.documentElement.style.setProperty('zoom', String(scale))
      try {
        localStorage.setItem(SCALE_KEY, String(scale))
      } catch {
        // oturumla sınırlı
      }
    }
  },
  openExternal: (url) => chrome?.tabs?.create({ url }),
})
