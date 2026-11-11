// §28.5 onay kuyruğu: bütün oturumlardan, daemon işlerinden ve uzak görevlerden bekleyen onaylar tek ekranda.
// Klavye `kor approvals` ile aynı: Enter/→ onayla, ← reddet, ↓/J sonraki, ↑/K önceki, D ayrıntı, A benzerleri onayla, Esc kapat.
import { Check, ChevronDown, ChevronUp, ListChecks, ShieldAlert, X } from 'lucide-react'
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../api.ts'
import { shellui } from './bridge.ts'
import { RISK_LABEL, SOURCE_LABEL, elapsed, move, queueAction, similarIds } from './state.ts'

interface Props {
  /** Kart kapatılınca (Esc) — açılır pencere ya da panel */
  onClose?: () => void
  /** Açılınca klavye odağı kuyruğa geçer */
  autoFocus?: boolean
}

export function ApprovalQueue({ onClose, autoFocus }: Props) {
  const { data, error, isLoading, mutate } = useSWR('shellui:queue', () => shellui.queue())
  useEffect(() => shellui.onChanged(() => void mutate()), [mutate])
  const [index, setIndex] = useState(0)
  const [details, setDetails] = useState(false)
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const box = useRef<HTMLDivElement>(null)
  const items = data?.items ?? []
  const current = items[Math.min(index, Math.max(0, items.length - 1))] ?? null
  const similar = current ? similarIds(items, current) : []

  useEffect(() => {
    if (autoFocus) box.current?.focus()
  }, [autoFocus])

  const answer = async (ids: string[], approve: boolean) => {
    setBusy(true)
    setProblem(null)
    try {
      if (ids.length === 1 && ids[0]) {
        if (approve) await shellui.approve(ids[0])
        else await shellui.deny(ids[0])
      } else {
        const r = await shellui.answerMany(ids, approve)
        if (!r.ok) setProblem(r.failed.map((f) => `${f.id}: ${f.error}`).join('\n'))
      }
      setDetails(false)
      await mutate()
    } catch (err) {
      setProblem(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const action = queueAction(e.key)
    if (!action || busy) return
    e.preventDefault()
    if (action === 'close') return onClose?.()
    if (!current) return
    switch (action) {
      case 'approve':
        return void answer([current.id], true)
      case 'deny':
        return void answer([current.id], false)
      case 'approveSimilar':
        return void answer(similar, true)
      case 'details':
        return setDetails((d) => !d)
      case 'next':
      case 'prev':
        setDetails(false)
        return setIndex((i) => move(i, items.length, action))
    }
  }

  if (isLoading) {
    return <p className="ksh-muted" role="status">Onaylar yükleniyor…</p>
  }
  if (error) {
    return (
      <p className="error-text" role="alert">
        Onay kuyruğu alınamadı: {errorMessage(error)}
      </p>
    )
  }
  return (
    <div
      ref={box}
      className="ksh-queue"
      tabIndex={0}
      onKeyDown={onKey}
      aria-label="Onay kuyruğu"
      data-kor-cmd="shellui.queue"
      data-kor-event="kor:shellui/changed"
    >
      {!current ? (
        <div className="ksh-empty">
          <ListChecks size={20} aria-hidden="true" />
          <p>Bekleyen onay yok.</p>
          <p className="ksh-muted">Sohbetler, daemon işleri ve uzak görevler onay isteyince burada sırayla görünür.</p>
        </div>
      ) : (
        <>
          <div className="ksh-queue-head">
            <span className="ksh-muted">
              {Math.min(index, items.length - 1) + 1}/{items.length}
            </span>
            <span className={`ksh-risk ${current.risk}`}>
              {current.risk === 'high' && <ShieldAlert size={12} aria-hidden="true" />}
              {RISK_LABEL[current.risk]}
            </span>
            <span className="ksh-chip">{SOURCE_LABEL[current.source]}</span>
            <span className="ksh-muted">{elapsed(current.since, Date.now())}</span>
          </div>
          <article className="ksh-card" aria-live="polite">
            {current.title && (
              <h3 className="ksh-card-title" translate="no">
                {current.title}
              </h3>
            )}
            <p className="ksh-card-tool">
              <strong translate="no">{current.tool}</strong>
            </p>
            <p className="ksh-card-summary" translate="no">
              {current.summary}
            </p>
            {current.destructive && (
              <p className="ksh-danger" role="note">
                Yıkıcı işlem: <span translate="no">{current.destructive}</span>
              </p>
            )}
            {details && current.files.length > 0 && (
              <ul className="ksh-files" translate="no">
                {current.files.map((f) => (
                  <li key={f}>{f}</li>
                ))}
              </ul>
            )}
            {details && current.files.length === 0 && <p className="ksh-muted">Dosya değişikliği yok.</p>}
          </article>
          {similar.length > 1 && (
            <button type="button" className="btn small" disabled={busy} onClick={() => void answer(similar, true)} data-kor-cmd="shellui.answerMany">
              <Check size={14} aria-hidden="true" /> {similar.length} benzer isteğin hepsini onayla
            </button>
          )}
          <div className="ksh-actions">
            <button type="button" className="btn small" disabled={busy} onClick={() => void answer([current.id], false)} data-kor-cmd="shellui.deny">
              <X size={14} aria-hidden="true" /> Reddet
            </button>
            <button type="button" className="btn small primary" disabled={busy} onClick={() => void answer([current.id], true)} data-kor-cmd="shellui.approve">
              <Check size={14} aria-hidden="true" /> Onayla
            </button>
            <button
              type="button"
              className="action-btn"
              aria-label="Önceki onay"
              title="Önceki (↑ / K)"
              disabled={items.length < 2}
              onClick={() => setIndex((i) => move(i, items.length, 'prev'))}
            >
              <ChevronUp size={14} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="action-btn"
              aria-label="Sonraki onay (ertele)"
              title="Sonraki, ertele (↓ / J)"
              disabled={items.length < 2}
              onClick={() => setIndex((i) => move(i, items.length, 'next'))}
            >
              <ChevronDown size={14} aria-hidden="true" />
            </button>
            <button type="button" className="btn small" onClick={() => setDetails((d) => !d)} aria-pressed={details}>
              Ayrıntı
            </button>
          </div>
          <p className="ksh-muted ksh-keys">Enter/→ onayla · ← reddet · ↓/J ertele · ↑/K önceki · D ayrıntı · A benzerleri onayla · Esc kapat</p>
        </>
      )}
      {problem && (
        <p className="error-text" role="alert">
          {problem}
        </p>
      )}
    </div>
  )
}
