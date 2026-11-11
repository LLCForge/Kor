// [g17] Sohbet üstü araç çubuğu: içerik türü filtresi, sabitlenen mesajlar, dışa aktarma seçimi,
//      toplu diyagram dışa aktarma (zip + ilerleme), sohbeti HTML'e aktarma, özet kartı, yan yana karşılaştırma.
import { BarChart3, Columns2, FileCode2, Images, X } from 'lucide-react'
import { useEffect, useMemo, useState, type RefObject } from 'react'
import { api, errorMessage } from './api.ts'
import type { ChatItem } from './chat-state.ts'
import { chatSummary, classifyMessage, diagramToPng, messagesToMarkdown, snapshotMessagesHtml } from './g17-export.ts'
import { focusMessage } from './G17MessageActions.tsx'
import { g17View, useG17View, type ContentFilter } from './g17-view.ts'
import { makeZip, type ZipEntry } from './g17-zip.ts'
import { Markdown } from './Markdown.tsx'
import { downloadBlob } from './rich-export.ts'

const stamp = () => new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
const FILTERS: ReadonlyArray<{ id: ContentFilter; label: string }> = [
  { id: 'all', label: 'Tümü' },
  { id: 'code', label: 'Kod' },
  { id: 'diagram', label: 'Diyagram' },
  { id: 'image', label: 'Görsel' },
]

type Bulk = { state: 'idle' | 'done'; count?: number } | { state: 'running'; done: number; total: number }

interface Props {
  items: readonly ChatItem[]
  containerRef: RefObject<HTMLDivElement | null>
}

export function G17ThreadBar({ items, containerRef }: Props) {
  const view = useG17View()
  const [bulk, setBulk] = useState<Bulk>({ state: 'idle' })
  const [pdfStatus, setPdfStatus] = useState<string | null>(null)
  const [showSummary, setShowSummary] = useState(false)

  // Thread her sohbette yeniden bağlanır; görünüm durumunu sıfırla (önceki sohbetten taşmasın).
  useEffect(() => g17View.reset(), [])

  const textOf = useMemo(() => new Map(items.filter((i) => i.kind === 'user' || i.kind === 'assistant').map((i) => [i.id, i.text])), [items])

  // İçerik türü filtresi: eşleşen mesaj düğümlerini işaretle, kapsayıcıya sınıf ekle; CSS gerisini gizler.
  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    for (const el of Array.from(container.querySelectorAll('[data-g17-match]'))) el.removeAttribute('data-g17-match')
    if (view.filter === 'all') {
      container.classList.remove('g17-filtering')
      return
    }
    container.classList.add('g17-filtering')
    for (const [id, text] of textOf) {
      const el = document.getElementById(`g17-msg-${id}`)
      if (el && classifyMessage(text)[view.filter]) el.setAttribute('data-g17-match', 'true')
    }
    return () => container.classList.remove('g17-filtering')
  }, [view.filter, textOf, containerRef])

  const pinnedItems = items.filter((i) => view.pinned.has(i.id))
  const selectedItems = items.filter((i) => view.selected.has(i.id))
  const compareItems = view.compare.map((id) => items.find((i) => i.id === id)).filter((i): i is ChatItem => i !== undefined)

  const snippet = (text: string) => (text.trim().slice(0, 70) || '(boş)') + (text.trim().length > 70 ? '…' : '')

  const exportChatHtml = () => {
    const container = containerRef.current
    if (!container) return
    const nodes = Array.from(container.children).filter((c): c is HTMLElement => c instanceof HTMLElement && !c.classList.contains('g17-bar'))
    const html = snapshotMessagesHtml(nodes, { title: 'Kor sohbeti', theme: 'auto' })
    downloadBlob(new Blob([html], { type: 'text/html;charset=utf-8' }), `sohbet-${stamp()}.html`)
  }

  const bulkExport = async () => {
    const container = containerRef.current
    if (!container) return
    const svgs = Array.from(container.querySelectorAll('.g3-mermaid-host svg'))
    if (svgs.length === 0) {
      setBulk({ state: 'done', count: 0 })
      return
    }
    setBulk({ state: 'running', done: 0, total: svgs.length })
    const entries: ZipEntry[] = []
    for (let i = 0; i < svgs.length; i++) {
      try {
        const blob = await diagramToPng(new XMLSerializer().serializeToString(svgs[i] as SVGSVGElement), { scale: 2, transparent: false, theme: 'auto' })
        entries.push({ name: `diyagram-${i + 1}.png`, data: new Uint8Array(await blob.arrayBuffer()) })
      } catch {
        // Tek bir diyagram dönüştürülemezse atlanır; diğerleri yine paketlenir.
      }
      setBulk({ state: 'running', done: i + 1, total: svgs.length })
    }
    if (entries.length > 0) downloadBlob(makeZip(entries), `diyagramlar-${stamp()}.zip`)
    setBulk({ state: 'done', count: entries.length })
  }

  const exportSelectedMarkdown = () => {
    const md = messagesToMarkdown(items, view.selected)
    downloadBlob(new Blob([md], { type: 'text/markdown;charset=utf-8' }), `secili-mesajlar-${stamp()}.md`)
  }

  const exportSelectedPdf = async () => {
    const nodes = selectedItems.map((i) => document.getElementById(`g17-msg-${i.id}`)).filter((n): n is HTMLElement => n !== null)
    if (nodes.length === 0) return
    setPdfStatus('PDF hazırlanıyor…')
    try {
      const html = snapshotMessagesHtml(nodes, { title: 'Seçili mesajlar', theme: 'light' })
      const saved = await api.g17ExportPdf(html, `secili-mesajlar-${stamp()}.pdf`)
      setPdfStatus(saved ? `Kaydedildi: ${saved}` : null)
    } catch (err) {
      setPdfStatus(errorMessage(err))
    }
  }

  const summary = showSummary ? chatSummary(items) : null

  return (
    <div className="g17-bar" data-g17-omit>
      <div className="g17-bar-row">
        <div className="g17-filter" role="group" aria-label="İçerik türüne göre süz">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              className={`g17-filter-btn${view.filter === f.id ? ' active' : ''}`}
              aria-pressed={view.filter === f.id}
              onClick={() => g17View.setFilter(f.id)}
            >
              {f.label}
            </button>
          ))}
        </div>
        <div className="g17-bar-actions">
          <button type="button" className="btn small ghost" onClick={exportChatHtml} title="Sohbeti tek HTML dosyası olarak dışa aktar">
            <FileCode2 size={14} aria-hidden="true" /> HTML
          </button>
          <button type="button" className="btn small ghost" onClick={() => void bulkExport()} disabled={bulk.state === 'running'} title="Tüm diyagramları PNG olarak zip'le">
            <Images size={14} aria-hidden="true" />
            {bulk.state === 'running' ? `${bulk.done}/${bulk.total}` : 'Diyagramlar'}
          </button>
          <button type="button" className={`btn small ghost${showSummary ? ' active' : ''}`} onClick={() => setShowSummary((s) => !s)} aria-pressed={showSummary} title="Sohbet özeti">
            <BarChart3 size={14} aria-hidden="true" /> Özet
          </button>
        </div>
      </div>

      {bulk.state === 'done' && (
        <p className="g17-bar-note" role="status">
          {bulk.count === 0 ? 'Dışa aktarılacak diyagram bulunamadı.' : `${bulk.count} diyagram zip olarak indirildi.`}
        </p>
      )}

      {pinnedItems.length > 0 && (
        <div className="g17-pins" aria-label="Sabitlenen mesajlar">
          {pinnedItems.map((i) => (
            <button key={i.id} type="button" className="g17-pin-chip" onClick={() => focusMessage(i.id)} title="Mesaja git">
              <span className="g17-pin-role">{i.kind === 'user' ? 'Sen' : 'Asistan'}</span>
              <span className="truncate">{snippet(i.kind === 'user' || i.kind === 'assistant' ? i.text : '')}</span>
            </button>
          ))}
        </div>
      )}

      {selectedItems.length > 0 && (
        <div className="g17-selection" role="group" aria-label="Seçili mesajları dışa aktar">
          <span>{selectedItems.length} mesaj seçili</span>
          <button type="button" className="btn small" onClick={exportSelectedMarkdown}>Markdown</button>
          <button type="button" className="btn small" onClick={() => void exportSelectedPdf()}>PDF</button>
          <button type="button" className="btn small ghost" onClick={() => g17View.clearSelected()}>Temizle</button>
          {pdfStatus && <span className="g17-bar-note">{pdfStatus}</span>}
        </div>
      )}

      {summary && (
        <section className="g17-summary" aria-label="Sohbet özeti">
          <Stat label="Kullanıcı" value={summary.userMessages} />
          <Stat label="Asistan" value={summary.assistantMessages} />
          <Stat label="Araç çağrısı" value={summary.toolCalls} />
          <Stat label="Kod bloğu" value={summary.codeBlocks} />
          <Stat label="Diyagram" value={summary.diagrams} />
          <Stat label="Kelime" value={summary.words} />
        </section>
      )}

      {compareItems.length === 2 && (
        <div className="g17-compare" role="dialog" aria-label="Yan yana karşılaştırma" aria-modal="false">
          <div className="g17-compare-head">
            <span>Yan yana karşılaştırma</span>
            <button type="button" className="action-btn" onClick={() => g17View.clearCompare()} aria-label="Karşılaştırmayı kapat">
              <X size={16} aria-hidden="true" />
            </button>
          </div>
          <div className="g17-compare-cols">
            {compareItems.map((i, idx) => (
              <article key={i.id} className="g17-compare-col">
                <header>
                  <Columns2 size={14} aria-hidden="true" /> Yanıt {idx + 1}
                </header>
                <div className="prose">
                  <Markdown text={i.kind === 'assistant' ? i.text : ''} />
                </div>
              </article>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="g17-stat">
      <span className="g17-stat-value">{value}</span>
      <span className="g17-stat-label">{label}</span>
    </div>
  )
}
