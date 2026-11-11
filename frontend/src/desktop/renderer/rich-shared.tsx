// [g8] Zengin blok grubunun ortak arayüz parçaları: kart çerçevesi, güvenli JSON ayrıştırma + ham geri dönüş
//      ve bağımsız HTML dışa aktarma düğmesi (#120). Her blok bunları paylaşır (tekrar boilerplate yok).
import { Download } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { api, errorMessage } from './api.ts'
import { buildStandaloneHtml } from './rich-data.ts'
import { withinRichSizeLimit } from './rich-sanitize.ts'

/** Blok kodunu güvenle JSON olarak ayrıştırır (eval yok). Boyut aşılırsa ya da geçersizse hata döndürür. */
export function parseJsonBlock<T>(code: string): { ok: true; data: T } | { ok: false; error: string } {
  if (!withinRichSizeLimit(code)) return { ok: false, error: 'İçerik görüntülenemeyecek kadar büyük.' }
  try {
    return { ok: true, data: JSON.parse(code) as T }
  } catch {
    return { ok: false, error: 'Geçersiz JSON' }
  }
}

interface BlockFrameProps {
  icon: ReactNode
  label: string
  tools?: ReactNode
  children: ReactNode
  className?: string
}

/** Zengin blokların ortak kart çerçevesi: başlık (simge + etiket) + araç alanı + gövde. */
export function BlockFrame({ icon, label, tools, children, className }: BlockFrameProps) {
  return (
    <div className={`g8-block${className ? ` ${className}` : ''}`}>
      <div className="g8-head">
        <span className="g8-title">
          {icon} {label}
        </span>
        {tools && <div className="g8-tools">{tools}</div>}
      </div>
      <div className="g8-body">{children}</div>
    </div>
  )
}

/** Ayrıştırılamayan ya da akış sürerken tamamlanmamış blok için ham metni gösteren güvenli geri dönüş. */
export function RawFallback({ code, note }: { code: string; note?: string }) {
  return (
    <div className="g8-block g8-raw">
      {note && (
        <p className="g8-note" role="note">
          {note}
        </p>
      )}
      <pre>{code}</pre>
    </div>
  )
}

/** Artifact'i bağımsız HTML belgesi olarak dosyaya aktarır (#120). Kaydetme ana sürecin yol-hapisli penceresiyle yapılır. */
export function ExportHtmlButton({ title, getBody }: { title: string; getBody: () => string }) {
  const [state, setState] = useState<'idle' | 'busy' | 'done' | 'error'>('idle')
  const save = async () => {
    setState('busy')
    try {
      // g3SaveCode: yerel kaydetme penceresi + çalışma dizinine yol hapsi (renderer'dan ham yol erişimi yok).
      const saved = await api.g3SaveCode(buildStandaloneHtml(title, getBody()), 'html')
      setState(saved ? 'done' : 'idle')
      if (saved) setTimeout(() => setState('idle'), 2000)
    } catch (err) {
      console.warn('HTML dışa aktarılamadı:', errorMessage(err))
      setState('error')
      setTimeout(() => setState('idle'), 2500)
    }
  }
  return (
    <button
      type="button"
      className="action-btn"
      onClick={() => void save()}
      disabled={state === 'busy'}
      aria-label="Bağımsız HTML olarak kaydet"
      title={state === 'error' ? 'Kaydedilemedi' : state === 'done' ? 'Kaydedildi' : 'Bağımsız HTML olarak kaydet'}
    >
      <Download size={15} aria-hidden="true" />
    </button>
  )
}
