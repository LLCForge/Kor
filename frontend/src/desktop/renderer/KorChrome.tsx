// Yerel uygulama hissi (spec §21, ADR 0053): WebView kilidini bağlar, Kor'un kendi bağlam menüsünü, "çalışan görev
// var" kapatma diyaloğunu, yazı boyutunu (Ctrl + / - / 0) ve bildirimleri (kor-ui Toaster) çizer. App'e bir kez konur.
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { webviewApi, windowEvents } from '../ipc-webview.ts'
import { api, errorMessage } from './api.ts'
import { insertIntoDraft } from './g21/quote.ts'
import { Button, Modal, Toaster, toast } from './kor-ui/index.ts'
import { installWebviewLock, isMac, menuItems, nextScale, type MenuAction, type MenuContext } from './webview-lock.ts'

const SCALE_KEY = 'kor.uiScale'

function readScale(): number {
  try {
    const v = Number(window.localStorage.getItem(SCALE_KEY))
    return Number.isFinite(v) && v >= 0.5 && v <= 2 ? v : 1
  } catch {
    return 1
  }
}

function applyScale(scale: number): void {
  document.documentElement.style.setProperty('zoom', String(scale))
  try {
    window.localStorage.setItem(SCALE_KEY, String(scale))
  } catch {
    // Depolama kapalıysa ölçek bu oturumla sınırlı kalır.
  }
}

/** Composer taslağının sonuna ekler (gönderilmez); App `g21:draft-insert` olayını dinler. */
const insertIntoComposer = insertIntoDraft

const quoteLines = (text: string) =>
  text
    .trim()
    .split('\n')
    .map((l) => `> ${l}`)
    .join('\n')

interface OpenMenu {
  x: number
  y: number
  ctx: MenuContext
  target: Element
}

async function writeClipboard(text: string): Promise<void> {
  await navigator.clipboard.writeText(text)
}

function messageText(target: Element): string {
  const sel = window.getSelection()?.toString() ?? ''
  if (sel) return sel
  const msg = target.closest('.message')
  return (msg?.querySelector('.prose, .user-text') ?? msg)?.textContent ?? ''
}

function codeOf(target: Element): { text: string; lang: string } {
  const block = target.closest('.code-block') ?? target.closest('pre')
  const pre = block?.querySelector('pre') ?? block
  const lang = block?.querySelector('.g3-code-lang')?.textContent ?? ''
  return { text: (pre?.textContent ?? '').replace(/\n$/, ''), lang: lang === 'kod' ? '' : lang }
}

/** Menü eylemini uygular; başarısızlık kullanıcıya bildirim olarak döner (sessiz hata yok). */
async function run(action: MenuAction, ctx: MenuContext, target: Element): Promise<void> {
  const field = target.closest('textarea, input, [contenteditable="true"], .cm-editor') as HTMLElement | null
  switch (action) {
    case 'cut':
    case 'copy':
    case 'selectAll':
      field?.focus()
      if (!document.execCommand(action)) toast('Pano işlemi yapılamadı', 'warning')
      return
    case 'paste': {
      field?.focus()
      const text = await navigator.clipboard.readText()
      // insertText geri al geçmişini korur.
      if (!document.execCommand('insertText', false, text)) toast('Yapıştırılamadı', 'warning')
      return
    }
    case 'copyCode':
      await writeClipboard(codeOf(target).text)
      return
    case 'saveCode': {
      const btn = target.closest('.code-block')?.querySelector<HTMLButtonElement>('button[aria-label="Dosyaya kaydet"]')
      if (btn) btn.click()
      else {
        const { text, lang } = codeOf(target)
        const saved = await api.g3SaveCode(text, lang)
        if (saved) toast(`Kaydedildi: ${saved}`, 'success')
      }
      return
    }
    case 'applyCode': {
      const { text, lang } = codeOf(target)
      // Uygulama ajan üzerinden yapılır: dosya başına diff önizlemesi ve onay (ADR 0030) atlanmaz.
      insertIntoComposer(`Bu kodu ilgili dosyaya uygula (diff önizlemesiyle):\n\`\`\`${lang}\n${text}\n\`\`\``)
      return
    }
    case 'quote':
      insertIntoComposer(`${quoteLines(messageText(target))}\n\n`)
      return
    case 'reference':
      insertIntoComposer(`Referans (önceki mesaj):\n${quoteLines(messageText(target))}\n\n`)
      return
    case 'regenerate': {
      const btn = target.closest('.message')?.querySelector<HTMLButtonElement>('button[aria-label="Yanıtı yeniden üret"]')
      if (btn && !btn.disabled) btn.click()
      else toast('Bu yanıt şu an yeniden üretilemez', 'warning')
      return
    }
    case 'saveImage': {
      if (ctx.kind !== 'image') return
      const saved = await webviewApi.saveImage(ctx.src)
      if (saved) toast(`Kaydedildi: ${saved}`, 'success')
      return
    }
    case 'referenceImage':
      if (ctx.kind !== 'image') return
      insertIntoComposer(`Referans görsel: ${decodeURIComponent(ctx.src.replace(/^https?:\/\/asset\.localhost\//, ''))}\n\n`)
      return
    case 'copyImage': {
      if (ctx.kind !== 'image') return
      const blob = await (await fetch(ctx.src)).blob()
      // Pano yalnızca PNG görseli kabul eder; diğer türler PNG'ye çizilerek kopyalanır.
      const png = blob.type === 'image/png' ? blob : await toPng(blob)
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })])
      return
    }
    case 'openLink':
      if (ctx.kind === 'link') await api.openExternal(ctx.href)
      return
    case 'copyLink':
      if (ctx.kind === 'link') await writeClipboard(ctx.href)
      return
  }
}

async function toPng(blob: Blob): Promise<Blob> {
  const bmp = await createImageBitmap(blob)
  const canvas = new OffscreenCanvas(bmp.width, bmp.height)
  canvas.getContext('2d')?.drawImage(bmp, 0, 0)
  return canvas.convertToBlob({ type: 'image/png' })
}

function ContextMenu({ menu, onClose }: { menu: OpenMenu; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: menu.x, top: menu.y })
  const items = menuItems(menu.ctx, isMac())
  // Pencereye sığdır: sağ/alt kenarı taşarsa sola/yukarı açılır.
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const r = el.getBoundingClientRect()
    setPos({ left: Math.max(4, Math.min(menu.x, window.innerWidth - r.width - 4)), top: Math.max(4, Math.min(menu.y, window.innerHeight - r.height - 4)) })
    el.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }, [menu])
  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== 'Escape') return
      if (e.type === 'pointerdown' && ref.current?.contains(e.target as Node)) return
      onClose()
    }
    window.addEventListener('pointerdown', close, true)
    window.addEventListener('keydown', close, true)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    return () => {
      window.removeEventListener('pointerdown', close, true)
      window.removeEventListener('keydown', close, true)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
    }
  }, [onClose])
  const onKey = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const buttons = [...(ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])]
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
    buttons[(i + (e.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length]?.focus()
  }
  return (
    <div ref={ref} className="kor-context-menu" role="menu" aria-label="Bağlam menüsü" style={pos} onKeyDown={onKey}>
      {items.map((it) => (
        <button
          key={it.action}
          type="button"
          role="menuitem"
          disabled={it.disabled}
          onPointerDown={(e) => e.preventDefault() /* seçim ve odak kaybolmasın */}
          onClick={() => {
            onClose()
            void run(it.action, menu.ctx, menu.target).catch((err: unknown) => toast(errorMessage(err), 'error'))
          }}
        >
          <span>{it.label}</span>
          {it.shortcut && <kbd>{it.shortcut}</kbd>}
        </button>
      ))}
    </div>
  )
}

export function KorChrome() {
  const [menu, setMenu] = useState<OpenMenu | null>(null)
  const [closing, setClosing] = useState<number | null>(null)

  useEffect(() => {
    applyScale(readScale())
    const install = (devtools: boolean) =>
      installWebviewLock({
        devtools,
        onMenu: (x, y, ctx, target) => setMenu({ x, y, ctx, target }),
        onRedirect: (to) => {
          if (to === 'zoomIn' || to === 'zoomOut' || to === 'zoomReset') applyScale(nextScale(readScale(), to))
        },
        openExternal: (url) => void api.openExternal(url).catch((err: unknown) => toast(errorMessage(err), 'error')),
      })
    let uninstall = install(false)
    // Temizlikten sonra gelen yanıt kilidi yeniden kurmaz (StrictMode çift etkisi iki kilit bırakıyordu: Ctrl+= iki adım).
    let disposed = false
    // Geliştirici build'inde devtools kısayolları açılır (karar Rust'ta: debug + KOR_DEVTOOLS=1 / --devtools).
    void webviewApi
      .devtools()
      .then((on) => {
        if (!on || disposed) return
        uninstall()
        uninstall = install(true)
      })
      .catch(() => {
        // Köprü yanıt vermezse devtools kapalı kalır: güvenli taraf.
      })
    const offClose = windowEvents.onCloseRequested((p) => {
      const n = typeof p === 'object' && p !== null && typeof (p as { running?: unknown }).running === 'number' ? (p as { running: number }).running : 1
      setClosing(n)
    })
    return () => {
      disposed = true
      uninstall()
      offClose()
    }
  }, [])

  return (
    <>
      {menu && <ContextMenu menu={menu} onClose={() => setMenu(null)} />}
      {closing !== null && (
        <Modal
          title="Çalışan görev var"
          description={closing === 1 ? 'Bir sohbet hâlâ çalışıyor. Pencere kapatılırsa ajan durur.' : `${closing} sohbet hâlâ çalışıyor. Pencere kapatılırsa ajanlar durur.`}
          onClose={() => setClosing(null)}
          footer={
            <>
              <Button onClick={() => setClosing(null)} autoFocus>
                Vazgeç
              </Button>
              <Button
                variant="danger"
                data-kor-cmd="webview.closeNow"
                onClick={() => void webviewApi.closeNow().catch((err: unknown) => toast(errorMessage(err), 'error'))}
              >
                Yine de kapat
              </Button>
            </>
          }
        />
      )}
      <Toaster />
    </>
  )
}
