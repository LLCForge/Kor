// [g17] Mesaj eylem çubuğu eklentileri: yanıtı dışa aktar (Markdown/PDF/HTML), kalıcı bağlantı, sabitleme,
//      dışa aktarma seçimi, yan yana karşılaştırma. Thread bunları eylem çubuğuna children olarak ekler.
import { Columns2, FileDown, Link2, ListChecks, Pin, PinOff } from 'lucide-react'
import { useRef, useState } from 'react'
import { api, errorMessage } from './api.ts'
import type { ChatItem } from './chat-state.ts'
import { messagesToMarkdown, snapshotMessagesHtml, type ExportTheme } from './g17-export.ts'
import { g17View, useG17View } from './g17-view.ts'
import { downloadBlob } from './rich-export.ts'

const anchorId = (id: string) => `g17-msg-${id}`

/** Mesaja yerel çapa ile gider ve kısa süre vurgular. */
export function focusMessage(id: string): void {
  const el = document.getElementById(anchorId(id))
  if (!el) return
  el.scrollIntoView({ behavior: 'smooth', block: 'center' })
  el.classList.add('g17-flash')
  window.setTimeout(() => el.classList.remove('g17-flash'), 1300)
}

interface Props {
  item: ChatItem
  items: readonly ChatItem[]
}

export function G17MessageActions({ item, items }: Props) {
  const view = useG17View()
  const detailsRef = useRef<HTMLDetailsElement>(null)
  const [status, setStatus] = useState<string | null>(null)
  if (item.kind !== 'user' && item.kind !== 'assistant') return null
  const pinned = view.pinned.has(item.id)
  const selected = view.selected.has(item.id)
  const comparing = view.compare.includes(item.id)

  const permalink = () => {
    void navigator.clipboard.writeText(`#${anchorId(item.id)}`).catch(() => undefined)
    focusMessage(item.id)
  }

  const close = () => {
    if (detailsRef.current) detailsRef.current.open = false
  }

  const exportMarkdown = () => {
    const md = messagesToMarkdown(items, new Set([item.id]))
    downloadBlob(new Blob([md], { type: 'text/markdown;charset=utf-8' }), `yanit-${item.id}.md`)
    close()
  }

  const node = () => document.getElementById(anchorId(item.id))

  const exportHtml = (theme: ExportTheme) => {
    const el = node()
    if (!el) return
    const html = snapshotMessagesHtml([el], { title: 'Kor yanıtı', theme })
    downloadBlob(new Blob([html], { type: 'text/html;charset=utf-8' }), `yanit-${item.id}.html`)
    close()
  }

  const exportPdf = async () => {
    const el = node()
    if (!el) return
    setStatus('PDF hazırlanıyor…')
    try {
      const html = snapshotMessagesHtml([el], { title: 'Kor yanıtı', theme: 'light' })
      const saved = await api.g17ExportPdf(html, `yanit-${item.id}.pdf`)
      setStatus(saved ? `Kaydedildi: ${saved}` : null)
      close()
    } catch (err) {
      setStatus(errorMessage(err))
    }
  }

  return (
    <>
      <button
        type="button"
        className={`action-btn${selected ? ' g17-on' : ''}`}
        onClick={() => g17View.toggleSelected(item.id)}
        aria-pressed={selected}
        aria-label="Dışa aktarma için seç"
        title="Dışa aktarma için seç"
      >
        <ListChecks size={16} aria-hidden="true" />
      </button>
      <button
        type="button"
        className={`action-btn${pinned ? ' g17-on' : ''}`}
        onClick={() => g17View.togglePinned(item.id)}
        aria-pressed={pinned}
        aria-label={pinned ? 'Sabitlemeyi kaldır' : 'Mesajı sabitle'}
        title={pinned ? 'Sabitlemeyi kaldır' : 'Mesajı sabitle'}
      >
        {pinned ? <PinOff size={16} aria-hidden="true" /> : <Pin size={16} aria-hidden="true" />}
      </button>
      <button type="button" className="action-btn" onClick={permalink} aria-label="Kalıcı bağlantıyı kopyala" title="Kalıcı bağlantıyı kopyala">
        <Link2 size={16} aria-hidden="true" />
      </button>
      {item.kind === 'assistant' && (
        <>
          <button
            type="button"
            className={`action-btn${comparing ? ' g17-on' : ''}`}
            onClick={() => g17View.toggleCompare(item.id)}
            aria-pressed={comparing}
            aria-label="Yan yana karşılaştırmaya ekle"
            title="Yan yana karşılaştır"
          >
            <Columns2 size={16} aria-hidden="true" />
          </button>
          <details ref={detailsRef} className="g17-menu">
            <summary className="action-btn" aria-label="Yanıtı dışa aktar" title="Yanıtı dışa aktar">
              <FileDown size={16} aria-hidden="true" />
            </summary>
            <div className="g17-menu-panel" role="group" aria-label="Yanıtı dışa aktar">
              <div className="g17-menu-actions">
                <button type="button" className="btn small" onClick={exportMarkdown}>Markdown indir</button>
                <button type="button" className="btn small" onClick={() => exportHtml('auto')}>HTML indir</button>
                <button type="button" className="btn small" onClick={() => void exportPdf()}>PDF indir</button>
              </div>
              {status && <p className="g17-menu-status" role="status">{status}</p>}
            </div>
          </details>
        </>
      )}
    </>
  )
}
