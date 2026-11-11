// [g18] "CSV yapıştır, grafik çıkar": mesaj kutusuna tablo biçimli metin (CSV/TSV/; ayraçlı, elektronik tablodan
//       kopyalanmış) yapıştırılınca bir öneri çipi belirir; tıklanınca veri yerelde grafiğe çevrilip pencerede gösterilir.
//       Pano okuma izni gerekmez: yalnızca kullanıcının kendi yapıştırma olayındaki metin kullanılır; ağ yok.
import { ChartColumn, X } from 'lucide-react'
import { useEffect, useId, useMemo, useState } from 'react'
import { ChartCard } from './G18ChartBlock.tsx'
import { isChartablePaste, parseChart } from './g18-chart-data.ts'
import { G7Modal } from './G7Modal.tsx'

export function G18PasteChart() {
  const [text, setText] = useState<string | null>(null)
  const [open, setOpen] = useState(false)
  const titleId = useId()

  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const target = e.target
      if (!(target instanceof HTMLTextAreaElement) || !target.classList.contains('composer-input')) return
      const pasted = e.clipboardData?.getData('text/plain') ?? ''
      setText(isChartablePaste(pasted) ? pasted : null)
    }
    document.addEventListener('paste', onPaste)
    return () => document.removeEventListener('paste', onPaste)
  }, [])

  const spec = useMemo(() => {
    if (!text) return null
    try {
      return parseChart(text)
    } catch (err) {
      console.warn('Yapıştırılan veri grafiğe çevrilemedi:', err)
      return null
    }
  }, [text])

  if (!text || !spec) return null
  const close = () => {
    setOpen(false)
    setText(null)
  }
  return (
    <>
      {!open && (
        <div className="g18-paste-chip" role="status">
          <ChartColumn size={15} aria-hidden="true" />
          <span>Yapıştırılan tablo grafiğe dönüştürülebilir.</span>
          <button type="button" className="btn small" onClick={() => setOpen(true)}>
            Grafiği göster
          </button>
          <button type="button" className="action-btn" onClick={() => setText(null)} aria-label="Öneriyi kapat" title="Kapat">
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}
      <G7Modal open={open} onClose={close} titleId={titleId} title="Yapıştırılan veriden grafik" className="g18-paste-modal">
        <ChartCard spec={spec} source={text} />
      </G7Modal>
    </>
  )
}
