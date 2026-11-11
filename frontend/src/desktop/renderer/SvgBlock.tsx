// [g8] ```svg blokları (75, 83): model üretimi SVG'yi SandboxedPreview (kind="svg" → DOMPurify ile temizlenir)
//      içinde güvenli gösterir. Kaynak/önizleme geçişi ve PNG indirme sunar. PNG, TEMİZLENMİŞ SVG'den üretilir
//      (dış referans yok → canvas kirlenmez); iframe'e dokunulmaz, opak origin sorunu doğmaz.
import { ArrowDown, Code, FileCode } from 'lucide-react'
import { useMemo, useState } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { SandboxedPreview } from './SandboxedPreview.tsx'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { downloadBlob, svgToPng } from './rich-export.ts'
import { sanitizeSvg } from './rich-sanitize.ts'

export function SvgBlock({ code }: RichBlockProps) {
  const [showSource, setShowSource] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const clean = useMemo(() => sanitizeSvg(code), [code])

  const savePng = async (): Promise<void> => {
    if (!clean.ok) return
    setBusy(true)
    setError(null)
    try {
      const blob = await svgToPng(clean.html)
      downloadBlob(blob, 'svg-onizleme.png')
    } catch (err) {
      console.warn('SVG PNG dışa aktarımı başarısız:', err)
      setError('PNG oluşturulamadı.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="code-block g8-rich">
      <div className="code-head">
        <span className="g8-lang">
          <FileCode size={13} aria-hidden="true" /> svg
        </span>
        <div className="g8-tools">
          <button
            type="button"
            className={`code-copy${showSource ? ' g8-on' : ''}`}
            onClick={() => setShowSource((s) => !s)}
            aria-pressed={showSource}
            aria-label="Kaynak kodu göster"
            title="Kaynak kodu göster"
          >
            <Code size={15} aria-hidden="true" />
          </button>
          <button type="button" className="code-copy" onClick={() => void savePng()} disabled={busy || !clean.ok} aria-label="PNG olarak indir" title="PNG olarak indir">
            <ArrowDown size={15} aria-hidden="true" />
          </button>
          <CopyButton className="code-copy" label="SVG kodunu kopyala" getText={() => code} />
        </div>
      </div>
      {showSource ? <pre className="g8-source">{code}</pre> : <SandboxedPreview content={code} kind="svg" title="SVG önizleme" />}
      {error && (
        <p className="g8-note error-text" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}

registerRichBlock('svg', SvgBlock)
