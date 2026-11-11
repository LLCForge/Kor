// [temel] Güvenli önizleme bileşeni (GÜVENLİK SINIRI): HTML/SVG içeriğini izole, opak origin'li, ağsız bir
//         iframe'de gösterir. Zengin blok grupları (grafik, HTML önizleme, SVG ...) bunun üzerine inşa eder.
//
//   - Belge kor-sandbox: şemasından, ana sürecin verdiği CSP başlığıyla yüklenir (sandbox-document.ts). srcdoc
//     kullanılmaz: srcdoc ana belgenin CSP'sini devralır ve satır içi stil/betikleri engellerdi.
//   - sandbox="allow-scripts" (yalnızca belgenin alıcı betiği için); allow-same-origin ASLA. İçerik betikleri statik
//     modda CSP ile engellenir, kart başına kullanıcı "Betikleri çalıştır" deyince betik modlu belge açılır.
//   - Ağ yok (connect-src 'none'); alt çerçevenin başka adrese gitmesini ana süreç engeller.
//   - SVG içerik DOMPurify ile temizlenir; içerik yalnızca belge "hazır" deyince ve yalnızca bu iframe'e gönderilir.
//   - Gelen mesajlar yalnızca bu iframe'in penceresinden (event.source) kabul edilir; yükseklik sınırlıdır.
import { ShieldAlert } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { HOST_SOURCE, PREVIEW_SOURCE, sandboxUrl } from '../sandbox-scheme.ts'
import { cssColor, sanitizeSvg, withinRichSizeLimit } from './rich-sanitize.ts'

interface SandboxedPreviewProps {
  /** Gömülecek işaretleme (gövde parçası). kind='svg' ise temizlenir. */
  content: string
  kind?: 'html' | 'svg'
  /** Erişilebilirlik için iframe başlığı (zorunlu). */
  title: string
  /** Kart başına betik iznini sunan düğmeyi gösterir (varsayılan kapalı). */
  allowScriptsToggle?: boolean
  /** Yükseklik bildirilene kadar kullanılan yükseklik (px). */
  height?: number
}

const DEFAULT_HEIGHT = 240
const MIN_HEIGHT = 40
const MAX_HEIGHT = 2400

/** Tema değişkeninin hesaplanan değerini okur (önizleme gövdesine aktarmak için; cssColor ile süzülür). */
function themeColor(name: string): string | undefined {
  try {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || undefined
  } catch {
    return undefined
  }
}

/** İçerik değişince iframe yeniden kurulsun diye kısa, kararlı bir anahtar (güvenlik amacı yok). */
function contentKey(text: string): string {
  let h = 5381
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0
  return `${text.length}:${h >>> 0}`
}

export function SandboxedPreview({ content, kind = 'html', title, allowScriptsToggle = false, height }: SandboxedPreviewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const [allowScripts, setAllowScripts] = useState(false)
  const [autoHeight, setAutoHeight] = useState<number | null>(null)

  const prepared = useMemo((): { body: string } | { error: string } => {
    if (!withinRichSizeLimit(content)) return { error: 'İçerik görüntülenemeyecek kadar büyük.' }
    if (kind !== 'svg') return { body: content }
    const clean = sanitizeSvg(content)
    return clean.ok ? { body: clean.html } : { error: clean.reason }
  }, [content, kind])

  const body = 'body' in prepared ? prepared.body : null
  const frameKey = `${allowScripts ? 'scripts' : 'static'}:${body === null ? '' : contentKey(body)}`

  useEffect(() => {
    setAutoHeight(null)
    if (body === null) return
    const onMessage = (e: MessageEvent) => {
      const frame = iframeRef.current?.contentWindow
      // Opak origin: origin dizesi 'null'dur. Yalnızca bu iframe'in penceresinden gelen mesaja güvenilir.
      if (!frame || e.source !== frame) return
      const data = e.data as { source?: unknown; ready?: unknown; height?: unknown }
      if (data?.source !== PREVIEW_SOURCE) return
      if (data.ready === true) {
        frame.postMessage(
          { source: HOST_SOURCE, html: body, scripts: allowScripts, bg: cssColor(themeColor('--surface')), fg: cssColor(themeColor('--text')) },
          '*',
        )
      } else if (typeof data.height === 'number' && Number.isFinite(data.height)) {
        setAutoHeight(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, Math.ceil(data.height))))
      }
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [body, allowScripts, frameKey])

  if (body === null) {
    return (
      <div className="g-rich-preview g-rich-error" role="alert">
        <ShieldAlert size={15} aria-hidden="true" /> {'error' in prepared ? prepared.error : ''}
      </div>
    )
  }

  const frameHeight = autoHeight ?? height ?? DEFAULT_HEIGHT
  return (
    <div className="g-rich-preview">
      {allowScriptsToggle && !allowScripts && (
        <button type="button" className="g-rich-run" onClick={() => setAllowScripts(true)} title="Bu önizlemede betiklerin çalışmasına izin ver">
          <ShieldAlert size={14} aria-hidden="true" /> Betikleri çalıştır
        </button>
      )}
      {/* key: mod veya içerik değişince iframe yeniden kurulur; alıcı yalnızca ilk içeriği kabul eder. */}
      <iframe
        key={frameKey}
        ref={iframeRef}
        className="g-rich-frame"
        title={title}
        sandbox="allow-scripts"
        src={sandboxUrl(allowScripts ? 'scripts' : 'static')}
        referrerPolicy="no-referrer"
        style={{ height: frameHeight }}
      />
    </div>
  )
}
