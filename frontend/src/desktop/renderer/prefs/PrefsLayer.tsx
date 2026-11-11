// ADR 0050 katmanı: okunabilirlik ayarlarını köke uygular (§28.9), ajanın sorusunu kompakt seçim kartıyla
// sorar (§28.13) ve otomatik özetlemeden önce neyin özetleneceğini gösterir (§28.1). App.tsx bir kez bağlar.
import { Archive, HelpCircle, Scissors } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import useSWR from 'swr'
import type { CompactPreviewEvent, QuestionEvent, UiPrefs } from '../../ipc-prefs.ts'
import type { ParamCardEvent } from '../../ipc-media.ts'
import { ParamCardView } from '../panels/media/ParamCard.tsx'
import { api, errorMessage } from '../api.ts'
import { prefsApi } from './api.ts'
import { cssVars, diffMode, rootClasses, setAutoSound, setCurrentUi } from './readability.ts'
import './prefs.css'

const contextEvents = (api as unknown as { context: { onCompactPreview(cb: (payload: unknown) => void): () => void } }).context


const DIFF_KEYS = ['g4.diffMode', 'g8-diff-mode']

function applyUi(u: UiPrefs): void {
  setCurrentUi(u)
  const el = document.documentElement
  for (const c of [...el.classList]) if (c === 'kor-ui' || c.startsWith('kor-ui-')) el.classList.remove(c)
  for (const c of rootClasses(u)) el.classList.add(c)
  for (const [k, v] of Object.entries(cssVars(u))) el.style.setProperty(k, v)
  // Diff görünümünün varsayılanı yapılandırmadan; panelde değiştirilen seçim oturum boyunca kalır.
  try {
    for (const k of DIFF_KEYS) window.localStorage.setItem(k, diffMode(u.diffView))
  } catch (err) {
    console.warn('Diff görünümü kaydedilemedi:', err)
  }
}

type Pending = (QuestionEvent | CompactPreviewEvent | ParamCardEvent) & { sessionId: string }

// §20.2: a media tool's parameter card from the agent; the answer goes back like a question's.
function AgentParamCard({ p, onDone }: { p: ParamCardEvent; onDone: () => void }) {
  const [error, setError] = useState<string | null>(null)
  const answer = async (value: { values: Record<string, unknown> } | { cancel: true }) => {
    try {
      await prefsApi.answer(p.id, value)
      onDone()
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  return (
    <div className="kor-ask-card pop-in km-ask-layer-card" data-kor-event="kor:event">
      <ParamCardView card={p.card} onSubmit={(values) => void answer({ values })} onCancel={() => void answer({ cancel: true })} cmd="prefs.answer" />
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}

function QuestionCard({ q, onDone }: { q: QuestionEvent; onDone: () => void }) {
  const titleId = useId()
  const [choice, setChoice] = useState(q.default)
  const [remember, setRemember] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const first = useRef<HTMLInputElement>(null)
  useEffect(() => first.current?.focus(), [])
  const send = async (index: number) => {
    try {
      await prefsApi.answer(q.id, { choice: index, remember })
      onDone()
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  return (
    <form
      className="kor-ask-card pop-in"
      role="dialog"
      aria-labelledby={titleId}
      data-kor-event="kor:event"
      onSubmit={(e) => {
        e.preventDefault()
        void send(choice)
      }}
    >
      <div className="kor-ask-head">
        <HelpCircle size={16} aria-hidden="true" />
        <span id={titleId} className="kor-ask-title" translate="no">{q.question}</span>
      </div>
      <div className="kor-ask-options" role="radiogroup" aria-labelledby={titleId}>
        {q.options.map((o, i) => (
          <label key={o} className={`kor-ask-option${i === choice ? ' selected' : ''}`}>
            <input ref={i === q.default ? first : undefined} type="radio" name={q.id} checked={i === choice} onChange={() => setChoice(i)} />
            <span className="kor-ask-num">{i + 1}</span>
            <span translate="no">{o}</span>
            {i === q.default && <span className="kor-ask-default">önerilen</span>}
          </label>
        ))}
      </div>
      <label className="kor-ask-remember">
        <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
        Her zaman böyle yap (kural olarak kaydet)
      </label>
      {error && <p className="error-text" role="alert">{error}</p>}
      <div className="kor-ask-actions">
        <button type="button" className="btn" onClick={() => void send(q.default)} data-kor-cmd="prefs.answer">Önerileni seç</button>
        <button type="submit" className="btn primary" data-kor-cmd="prefs.answer">Seç (↵)</button>
      </div>
    </form>
  )
}

function CompactCard({ c, onDone }: { c: CompactPreviewEvent; onDone: () => void }) {
  const titleId = useId()
  const [keep, setKeep] = useState<Set<number>>(new Set())
  const [error, setError] = useState<string | null>(null)
  const total = c.items.reduce((s, i) => s + i.tokens, 0)
  const largest = [...c.items].sort((a, b) => b.tokens - a.tokens).slice(0, 30).sort((a, b) => a.index - b.index)
  const answer = async (value: { keep: number[] } | { cancel: true }) => {
    try {
      if (c.confirm) await prefsApi.answer(c.id, value)
      onDone()
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  const toggle = (i: number) => setKeep((prev) => {
    const next = new Set(prev)
    if (next.has(i)) next.delete(i)
    else next.add(i)
    return next
  })
  return (
    <div className="kor-ask-card pop-in" role="dialog" aria-labelledby={titleId} data-kor-event="kor:context/compactPreview">
      <div className="kor-ask-head">
        <Scissors size={16} aria-hidden="true" />
        <span id={titleId} className="kor-ask-title">
          Bağlam doluyor: {c.items.length} parça (≈ {total.toLocaleString(uiLocale())} token) özetlenecek
        </span>
      </div>
      <p className="g7-note">{c.confirm ? 'Olduğu gibi kalmasını istediklerini işaretle; gerisi özetlenir.' : 'Bilgi: özetleme onay beklemeden yapılıyor (context.compact_confirm kapalı).'}</p>
      <ul className="kor-compact-list">
        {largest.map((i) => (
          <li key={i.index}>
            <label>
              {c.confirm && <input type="checkbox" checked={keep.has(i.index)} onChange={() => toggle(i.index)} />}
              <span className={`kor-compact-kind ${i.kind}`}>{i.kind === 'tool' ? 'araç' : i.kind === 'user' ? 'sen' : 'ajan'}</span>
              <span className="kor-compact-label" translate="no">{i.label || '—'}</span>
              <span className="kor-compact-tokens">≈ {i.tokens.toLocaleString(uiLocale())}</span>
            </label>
          </li>
        ))}
      </ul>
      {error && <p className="error-text" role="alert">{error}</p>}
      <div className="kor-ask-actions">
        {c.confirm ? (
          <>
            <button type="button" className="btn" onClick={() => void answer({ cancel: true })} data-kor-cmd="prefs.answer">Şimdi özetleme</button>
            <button type="button" className="btn primary" onClick={() => void answer({ keep: [...keep] })} data-kor-cmd="prefs.answer">
              <Archive size={14} aria-hidden="true" /> Özetle{keep.size > 0 ? ` (${keep.size} korunur)` : ''}
            </button>
          </>
        ) : (
          <button type="button" className="btn" onClick={onDone}>Tamam</button>
        )}
      </div>
    </div>
  )
}

export function PrefsLayer({ sessionId }: { sessionId: string | null }) {
  const { data: ui } = useSWR('prefs:ui', () => prefsApi.ui(), { revalidateOnFocus: false })
  const { data: auto } = useSWR('prefs:auto', () => prefsApi.auto(), { revalidateOnFocus: false })
  const [pending, setPending] = useState<Pending[]>([])

  useEffect(() => {
    if (ui) applyUi(ui)
  }, [ui])
  useEffect(() => {
    setAutoSound(auto?.find((r) => r.key === 'sound')?.value ?? true)
  }, [auto])

  useEffect(
    () =>
      api.onEvent((event) => {
        const body = event as unknown as Pending
        if (body.type === 'question' || body.type === 'paramCard') setPending((prev) => [...prev.filter((p) => p.id !== body.id), body])
        // Tur bitince ya da durunca bekleyen kartlar kalkar (yanıt verecek kimse kalmadı).
        const t = (event as { type: string }).type
        if (t === 'done' || t === 'error') setPending((prev) => prev.filter((p) => p.sessionId !== event.sessionId))
      }),
    [],
  )
  // §28.1 `kor:context/compactPreview`: otomatik özetlemeden önce.
  useEffect(
    () =>
      contextEvents.onCompactPreview((payload) => {
        const c = { ...(payload as Omit<CompactPreviewEvent, 'type'>), type: 'compactPreview' as const }
        setPending((prev) => [...prev.filter((p) => p.id !== c.id), c])
      }),
    [],
  )

  const shown = pending.find((p) => p.sessionId === sessionId)
  if (!shown) return null
  const close = () => setPending((prev) => prev.filter((p) => p.id !== shown.id))
  return (
    <div className="kor-ask-layer">
      {shown.type === 'question' ? (
        <QuestionCard key={shown.id} q={shown} onDone={close} />
      ) : shown.type === 'paramCard' ? (
        <AgentParamCard key={shown.id} p={shown} onDone={close} />
      ) : (
        <CompactCard key={shown.id} c={shown} onDone={close} />
      )}
    </div>
  )
}

import { uiLocale } from '../ui-locale.ts'