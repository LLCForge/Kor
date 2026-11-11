// Mesaj eylem çubuğu düğmeleri: kullanıcı mesajını düzenleyip yeniden gönderme, son yanıtı yeniden üretme.
// Thread.tsx bunları tek satırla bağlar; eylemler App köprüsünden (g2Bridge) gelir.
import { History, Pencil, RotateCcw } from 'lucide-react'
import { useState, type FormEvent, type KeyboardEvent } from 'react'
import { startEditing, stopEditing } from '../edit-store.ts'
import { isHumanUserText } from '../../../core/message-text.ts'
import { api, errorMessage } from '../api.ts'
import type { ChatItem } from '../chat-state.ts'
import { g2Bridge } from './use-g2-app.ts'

interface ActionProps {
  item: ChatItem
  items: readonly ChatItem[]
  running: boolean
}

export function G2UserActions({ item, items, running }: ActionProps) {
  if (item.kind !== 'user' || !g2Bridge.current || !isHumanUserText(item.text)) return null
  const activeId = g2Bridge.current.activeId
  // [cdx] Bu mesajdan önceye geri sar: g4 kontrol noktası metinle, olmazsa sırayla (n. kullanıcı mesajı ↔ n. nokta) eşlenir.
  const rewind = () =>
    void api.g4
      .listCheckpoints(activeId)
      .then((cps) => {
        const wanted = item.text.trim()
        const nth = items.filter((i) => i.kind === 'user').findIndex((i) => i.id === item.id)
        const cp = cps.find((c) => c.text.trim() === wanted) ?? cps[nth] ?? null
        if (!cp) throw new Error('Bu mesaj için kontrol noktası yok (sohbet eski ya da dosya değişikliği olmamış).')
        if (!cp.available) throw new Error('Bu kontrol noktası artık kullanılamıyor (bellek sınırı).')
        window.dispatchEvent(new CustomEvent('cdx:rewind', { detail: cp }))
      })
      .catch((err: unknown) => window.dispatchEvent(new CustomEvent('cdx:toast', { detail: errorMessage(err) })))
  return (
    <>
      <button
        type="button"
        className="action-btn"
        onClick={() => startEditing(item.id)}
        disabled={running}
        aria-label="Mesajı düzenle ve yeniden gönder"
        title={running ? 'Ajan çalışırken düzenlenemez' : 'Düzenle ve yeniden gönder'}
      >
        <Pencil size={16} aria-hidden="true" />
      </button>
      <button type="button" className="action-btn" onClick={rewind} disabled={running} aria-label="Bu mesajdan önceye geri sar" title={running ? 'Ajan çalışırken geri sarılamaz' : 'Bu mesajdan önceye geri sar (dosyalar + isteğe bağlı sohbet)'}>
        <History size={16} aria-hidden="true" />
      </button>
    </>
  )
}

/** cdesktop satır içi düzenleme kutusu: balonun yerine turuncu çerçeveli metin alanı + İptal / Yeniden dene. */
export function InlineEditBox({ item, items }: { item: Extract<ChatItem, { kind: 'user' }>; items: readonly ChatItem[] }) {
  const [text, setText] = useState(item.text)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!text.trim()) return
    setBusy(true)
    setError(null)
    try {
      await g2Bridge.current?.editMessage(item.id, items, text)
      stopEditing()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Escape') { e.preventDefault(); stopEditing() }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); e.currentTarget.form?.requestSubmit() }
  }
  return (
    <form className="cdx-inline-edit" onSubmit={(e) => void submit(e)}>
      <textarea className="cdx-inline-edit-input" value={text} onChange={(e) => setText(e.target.value)} onKeyDown={onKeyDown} autoFocus rows={Math.min(12, Math.max(2, text.split('\n').length))} aria-label="Mesajı düzenle" />
      {error && <p className="error-text" role="alert">{error}</p>}
      <div className="cdx-inline-edit-actions">
        <span className="dialog-meta">Bu mesajdan sonraki konuşma kaldırılır ve yeniden çalıştırılır. Ctrl+Enter gönderir.</span>
        <button type="button" className="btn small cdx-review-cancel" onClick={stopEditing} disabled={busy}>İptal</button>
        <button type="submit" className="btn small cdx-btn-accent" disabled={busy || !text.trim()}>{busy ? 'Gönderiliyor…' : 'Yeniden dene'}</button>
      </div>
    </form>
  )
}

export function G2AssistantActions({ item, items, running }: ActionProps) {
  const [busy, setBusy] = useState(false)
  if (running || item.kind !== 'assistant' || !g2Bridge.current) return null
  // Yalnızca son kullanıcı mesajından sonraki son yanıtta gösterilir.
  const lastUser = items.findLastIndex((i) => i.kind === 'user' && isHumanUserText(i.text))
  if (lastUser < 0 || items.findLast((i) => i.kind === 'assistant') !== item || items.indexOf(item) < lastUser) return null

  const regenerate = async () => {
    setBusy(true)
    try {
      await g2Bridge.current?.regenerate(items)
    } finally {
      setBusy(false)
    }
  }

  return (
    <button type="button" className="action-btn" onClick={() => void regenerate()} disabled={busy} aria-label="Yanıtı yeniden üret" title="Yeniden üret">
      <RotateCcw size={16} aria-hidden="true" />
    </button>
  )
}
