// [g30] Otomatik hafıza onay bandı: tur sonunda çıkarılan aday gerçekler burada "kaydet / düzenle / atla" ile onaylanır.
// Hiçbir şey otomatik yazılmaz; kullanıcı onaylayınca global hafızaya geçer. Bir seferde bir öneri gösterilir (yığmasın).
import { BrainCircuit, Check, Pencil, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from './api.ts'

export function G30MemoryLayer() {
  const { data, mutate } = useSWR('g30:pending', () => api.g30.pendingList(), { revalidateOnFocus: false })
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [err, setErr] = useState<string | null>(null)

  useEffect(() => api.g30.onPending(() => void mutate()), [mutate])

  const items = data ?? []
  const first = items[0]
  if (!first) return null

  const resolve = (id: string, action: 'save' | 'skip', text?: string) => {
    setErr(null)
    setEditing(null)
    api.g30
      .pendingResolve(id, action, text)
      .then((next) => mutate(next, { revalidate: false }))
      .catch((e: unknown) => setErr(errorMessage(e)))
  }

  return (
    <div className="g30-band pop-in" role="status" aria-label="Hafıza önerisi">
      <BrainCircuit size={15} aria-hidden="true" />
      <div className="g30-band-body">
        <span className="g30-band-title">Hafızaya eklensin mi?{items.length > 1 ? ` · ${items.length} öneri` : ''}</span>
        {editing === first.id ? (
          <input
            className="text-input g30-band-input"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') resolve(first.id, 'save', draft.trim())
              if (e.key === 'Escape') setEditing(null)
            }}
            // eslint-disable-next-line jsx-a11y/no-autofocus -- düzenlemeye geçince odak doğal beklenti
            autoFocus
          />
        ) : (
          <span className="g30-band-text">{first.text}</span>
        )}
        {err && <span className="error-text g30-band-err">{err}</span>}
      </div>
      {editing === first.id ? (
        <button type="button" className="btn small" onClick={() => resolve(first.id, 'save', draft.trim())} disabled={!draft.trim()}>
          Kaydet
        </button>
      ) : (
        <>
          <button type="button" className="action-btn" title="Kaydet" aria-label="Kaydet" onClick={() => resolve(first.id, 'save')}>
            <Check size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" title="Düzenle" aria-label="Düzenle" onClick={() => { setEditing(first.id); setDraft(first.text) }}>
            <Pencil size={14} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" title="Atla" aria-label="Atla" onClick={() => resolve(first.id, 'skip')}>
            <X size={15} aria-hidden="true" />
          </button>
        </>
      )}
    </div>
  )
}
