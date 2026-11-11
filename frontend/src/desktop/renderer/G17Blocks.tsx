// [g17] Blok düzeyi dışa aktarma: diyagram (PNG/SVG/pano/kaynak), kod bloğu indirme, tablo TSV kopyalama.
//      MermaidBlock, CodeBlock ve Markdown tablosuna tek satırla bağlanır.
import type { Element } from 'hast'
import { Download, FileDown } from 'lucide-react'
import { useRef, useState, type RefObject } from 'react'
import { suggestFileName } from '../../core/code-langs.ts'
import { CopyButton } from './CopyButton.tsx'
import { copyImageToClipboard, decorateSvgBackground, diagramToPng, resolveTheme, tableToTsv, type ExportTheme } from './g17-export.ts'
import { downloadBlob } from './rich-export.ts'

const stamp = () => new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')

interface DiagramExportProps {
  hostRef: RefObject<HTMLDivElement | null>
  /** Diyagram kaynağı (ör. mermaid metni) — "yerleştirme kodu" olarak kopyalanır. */
  source: string
  sourceLabel?: string
}

/** Render edilmiş bir diyagram/grafik için dışa aktarma menüsü. */
export function G17DiagramExport({ hostRef, source, sourceLabel = 'Mermaid' }: DiagramExportProps) {
  const detailsRef = useRef<HTMLDetailsElement>(null)
  const [scale, setScale] = useState(2)
  const [transparent, setTransparent] = useState(false)
  const [theme, setTheme] = useState<ExportTheme>('auto')
  const [title, setTitle] = useState('')
  const [timestamp, setTimestamp] = useState(false)
  const [status, setStatus] = useState<{ kind: 'note' | 'error'; text: string } | null>(null)

  const close = () => {
    if (detailsRef.current) detailsRef.current.open = false
  }
  const svgString = (): string => {
    const svg = hostRef.current?.querySelector('svg')
    if (!svg) throw new Error('Diyagram henüz hazır değil')
    return new XMLSerializer().serializeToString(svg)
  }

  const run = async (fn: () => Promise<void> | void, done: string) => {
    setStatus(null)
    try {
      await fn()
      setStatus({ kind: 'note', text: done })
      close()
    } catch (err) {
      setStatus({ kind: 'error', text: err instanceof Error ? err.message : 'İşlem başarısız' })
    }
  }

  const exportPng = () =>
    run(async () => {
      const blob = await diagramToPng(svgString(), { scale, transparent, theme, title: title.trim() || undefined, timestamp })
      downloadBlob(blob, `diyagram-${stamp()}.png`)
    }, 'PNG indirildi')

  const exportSvg = () =>
    run(() => {
      const svg = decorateSvgBackground(svgString(), transparent, resolveTheme(theme))
      downloadBlob(new Blob([svg], { type: 'image/svg+xml' }), `diyagram-${stamp()}.svg`)
    }, 'SVG indirildi')

  const copyImage = () =>
    run(async () => {
      const blob = await diagramToPng(svgString(), { scale, transparent, theme, title: title.trim() || undefined, timestamp })
      await copyImageToClipboard(blob)
    }, 'Panoya kopyalandı')

  const copySource = (text: string, label: string) => run(() => navigator.clipboard.writeText(text), `${label} kopyalandı`)

  return (
    <details ref={detailsRef} className="g17-menu">
      <summary className="code-copy" aria-label="Diyagramı dışa aktar" title="Dışa aktar">
        <Download size={15} aria-hidden="true" />
      </summary>
      <div className="g17-menu-panel" role="group" aria-label="Diyagram dışa aktarma seçenekleri">
        <label className="g17-field">
          <span>Ölçek</span>
          <select value={scale} onChange={(e) => setScale(Number(e.target.value))}>
            <option value={1}>1x</option>
            <option value={2}>2x</option>
            <option value={3}>3x</option>
            <option value={4}>4x</option>
          </select>
        </label>
        <label className="g17-field">
          <span>Tema</span>
          <select value={theme} onChange={(e) => setTheme(e.target.value as ExportTheme)}>
            <option value="auto">Otomatik</option>
            <option value="light">Açık</option>
            <option value="dark">Koyu</option>
          </select>
        </label>
        <label className="g17-check">
          <input type="checkbox" checked={transparent} onChange={(e) => setTransparent(e.target.checked)} />
          <span>Saydam arka plan</span>
        </label>
        <label className="g17-field">
          <span>Başlık</span>
          <input type="text" value={title} maxLength={200} placeholder="İsteğe bağlı" onChange={(e) => setTitle(e.target.value)} />
        </label>
        <label className="g17-check">
          <input type="checkbox" checked={timestamp} onChange={(e) => setTimestamp(e.target.checked)} />
          <span>Zaman damgası ekle</span>
        </label>
        <div className="g17-menu-actions">
          <button type="button" className="btn small" onClick={() => void exportPng()}>PNG indir</button>
          <button type="button" className="btn small" onClick={() => void exportSvg()}>SVG indir</button>
          <button type="button" className="btn small" onClick={() => void copyImage()}>Panoya kopyala</button>
          <button type="button" className="btn small ghost" onClick={() => void copySource(source, sourceLabel)}>{sourceLabel} kaynağı</button>
          <button type="button" className="btn small ghost" onClick={() => void run(() => navigator.clipboard.writeText(svgString()), 'SVG kaynağı kopyalandı')}>SVG kaynağı</button>
        </div>
        {status && (
          <p className={`g17-menu-status${status.kind === 'error' ? ' error-text' : ''}`} role={status.kind === 'error' ? 'alert' : 'status'}>
            {status.text}
          </p>
        )}
      </div>
    </details>
  )
}

/** Kod bloğunu, diline uygun uzantıyla tarayıcı indirmesi olarak kaydeder. */
export function G17CodeDownload({ text, language }: { text: string; language: string }) {
  const download = () => {
    const blob = new Blob([text], { type: 'text/plain;charset=utf-8' })
    downloadBlob(blob, suggestFileName(language))
  }
  return (
    <button type="button" className="code-copy" onClick={download} aria-label="Dosya olarak indir" title="Dosya olarak indir">
      <FileDown size={15} aria-hidden="true" />
    </button>
  )
}

/** Tabloyu TSV olarak panoya kopyalar (elektronik tabloya yapıştırmaya uygun). */
export function G17TableTsv({ node }: { node: Element }) {
  return (
    <CopyButton
      className="action-btn g17-table-tsv"
      label="Tabloyu TSV olarak kopyala (hücreler sekmeyle)"
      getText={() => tableToTsv(node)}
      icon={<FileDown size={14} aria-hidden="true" />}
    />
  )
}
