// Bağlam dökümünün parça satırları (§28.1, ADR 0050): her mesaj, dosya ve araç çıktısı kaç token tutuyor;
// "bağlamdan çıkar" içeriğini işaretle değiştirir, "sabitle" bir sonraki özetlemede olduğu gibi korur.
// CLI/TUI: /context items · /context drop <n> · /context pin <n>.
import { Pin, X } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { ContextApi, ContextItems as ContextItemsView } from '../../ipc.gen.ts'
import { api, errorMessage } from '../api.ts'
import './prefs.css'

// §16.3: the `context.*` types come from the generated contract (kor-core/schema/ipc.json).
const contextApi = (api as unknown as { context: Pick<ContextApi, 'items' | 'drop' | 'pin'> }).context

export function ContextItems({ sessionId }: { sessionId: string | null }) {
  const data = useSWR(sessionId ? ['context:items', sessionId] : null, () => contextApi.items(sessionId ?? ''), { revalidateOnFocus: false })
  const [error, setError] = useState<string | null>(null)
  if (!sessionId) return null
  const act = async (p: Promise<ContextItemsView>) => {
    setError(null)
    try {
      await data.mutate(await p, { revalidate: false })
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const rows = [...(data.data?.items ?? [])].sort((a, b) => b.tokens - a.tokens).slice(0, 40)
  return (
    <section className="g14-section" aria-label="Bağlamdaki parçalar" data-kor-cmd="context.items">
      <h3 className="g14-h">Bağlamdaki parçalar</h3>
      {data.error && <p className="error-text" role="alert">{errorMessage(data.error)}</p>}
      {data.data && rows.length === 0 && <p className="muted g14-empty">Bu sohbette henüz bir şey yok.</p>}
      <ul className="kor-compact-list">
        {rows.map((i) => {
          const pinned = data.data?.pinned.includes(i.index) ?? false
          return (
            <li key={i.index}>
              <label>
                <span className={`kor-compact-kind ${i.kind}`}>{i.kind === 'tool' ? 'araç' : i.kind === 'user' ? 'sen' : 'ajan'}</span>
                <span className="kor-compact-label" translate="no">{i.label || '—'}</span>
                <span className="kor-compact-tokens">≈ {i.tokens.toLocaleString(uiLocale())}</span>
                <button type="button" className={`action-btn${pinned ? ' active' : ''}`} aria-pressed={pinned} aria-label={pinned ? 'Sabitlemeyi kaldır' : 'Sabitle (özette korunur)'} title={pinned ? 'Sabitlemeyi kaldır' : 'Sabitle (özette korunur)'} data-kor-cmd="context.pin" onClick={() => void act(contextApi.pin(sessionId, i.index, !pinned))}>
                  <Pin size={14} aria-hidden="true" />
                </button>
                <button type="button" className="action-btn" aria-label="Bağlamdan çıkar" title="Bağlamdan çıkar" data-kor-cmd="context.drop" onClick={() => void act(contextApi.drop(sessionId, i.index))}>
                  <X size={14} aria-hidden="true" />
                </button>
              </label>
            </li>
          )
        })}
      </ul>
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

import { uiLocale } from '../ui-locale.ts'