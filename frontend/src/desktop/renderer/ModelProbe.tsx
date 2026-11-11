import { Play, Square } from 'lucide-react'
import { useEffect, useId, useMemo, useState } from 'react'
import { recommendForCoding, sortCatalog, type CatalogModel } from '../../core/model-catalog.ts'
import { PROBE_MAX_TOKENS, type ProbeResult } from '../../core/model-probe.ts'
import { api, errorMessage } from './api.ts'
import { useCatalog } from './g7-bus.ts'
import { formatMs, formatUsd } from './g7-format.ts'
import { G7Modal } from './G7Modal.tsx'
import { Markdown } from './Markdown.tsx'
import { uiLocale } from './ui-locale.ts'

type Slot =
  | { status: 'idle' }
  | { status: 'running'; startedAt: number }
  | { status: 'done'; result: ProbeResult }
  | { status: 'error'; message: string }
  | { status: 'aborted' }

interface ModelProbeProps {
  open: boolean
  onClose: () => void
  currentModel: string
  /** Karşılaştırma penceresinden gelen ön seçim */
  initialModels: readonly string[] | undefined
}

/** İki modelin varsayılanı: ön seçim > mevcut model + en iyi kodlayıcı (veya listedeki bir sonraki). */
function defaultPair(models: readonly CatalogModel[], current: string, initial: readonly string[] | undefined): [string, string] {
  const ids = models.map((m) => m.id)
  const first = initial?.[0] ?? current
  const best = recommendForCoding(models).best.find((m) => m.id !== first)?.id
  const second = initial?.[1] ?? best ?? ids.find((id) => id !== first) ?? first
  return [first, second]
}

export function ModelProbe({ open, onClose, currentModel, initialModels }: ModelProbeProps) {
  const titleId = useId()
  const { data, error } = useCatalog(open)
  const models = useMemo(() => sortCatalog(data?.models ?? [], 'name'), [data])
  const [picked, setPicked] = useState<[string, string] | null>(null)
  const [prompt, setPrompt] = useState('')
  const [runId, setRunId] = useState<string | null>(null)
  const [slots, setSlots] = useState<[Slot, Slot]>([{ status: 'idle' }, { status: 'idle' }])
  const [now, setNow] = useState(() => Date.now())
  const pair = picked ?? (models.length > 0 ? defaultPair(models, currentModel, initialModels) : [currentModel, currentModel])
  const running = slots.some((s) => s.status === 'running')

  // Sayaç yalnızca istek sürerken çalışır.
  useEffect(() => {
    if (!running) return
    const timer = setInterval(() => setNow(Date.now()), 200)
    return () => clearInterval(timer)
  }, [running])

  const setSlot = (i: 0 | 1, slot: Slot) => setSlots((prev) => (i === 0 ? [slot, prev[1]] : [prev[0], slot]))

  const start = () => {
    const id = crypto.randomUUID()
    const text = prompt.trim()
    if (!text) return
    setRunId(id)
    const startedAt = Date.now()
    setSlots([
      { status: 'running', startedAt },
      { status: 'running', startedAt },
    ])
    pair.forEach((model, index) => {
      const i = index === 0 ? 0 : 1
      api
        .g7RunProbe(id, model, text)
        .then((result) => setSlot(i, { status: 'done', result }))
        .catch((err: unknown) => {
          console.error('Model testi başarısız:', err)
          setSlots((prev) => {
            const next: [Slot, Slot] = [prev[0], prev[1]]
            // İptal edilen istek "aborted" olarak işaretlenmişse hata mesajıyla ezilmez.
            if (next[i].status !== 'aborted') next[i] = { status: 'error', message: errorMessage(err) }
            return next
          })
        })
    })
  }

  const cancel = () => {
    if (!runId) return
    setSlots((prev) => [prev[0].status === 'running' ? { status: 'aborted' } : prev[0], prev[1].status === 'running' ? { status: 'aborted' } : prev[1]])
    api.g7AbortProbe(runId).catch((err: unknown) => console.error('Test iptal edilemedi:', err))
  }

  const close = () => {
    if (running) cancel()
    onClose()
  }

  const choose = (i: 0 | 1, id: string) => setPicked(i === 0 ? [id, pair[1]] : [pair[0], id])

  return (
    <G7Modal open={open} onClose={close} titleId={titleId} title="Model testi" className="g7-full">
      <div className="g7-full-body">
        <p className="dialog-desc text-pretty">
          Aynı istem iki modele araçsız ve tek turda gönderilir; yanıt, süre, token ve maliyet yan yana gösterilir. Her test ücretlidir ve yanıt en fazla{' '}
          {PROBE_MAX_TOKENS.toLocaleString(uiLocale())} token ile sınırlıdır. Sohbet geçmişi gönderilmez.
        </p>
        {error && <p className="error-box" role="alert">{errorMessage(error)}</p>}

        <div className="g7-probe-pickers">
          {([0, 1] as const).map((i) => (
            <div key={i} className="g7-field">
              <label htmlFor={`${titleId}-m${i}`} className="g7-label">{i === 0 ? 'Model A' : 'Model B'}</label>
              <select id={`${titleId}-m${i}`} className="g7-select" value={pair[i]} onChange={(e) => choose(i, e.target.value)} disabled={running}>
                {!models.some((m) => m.id === pair[i]) && <option value={pair[i]}>{pair[i]}</option>}
                {models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                    {m.uncensored ? ' (sansürsüz)' : ''}
                  </option>
                ))}
              </select>
            </div>
          ))}
        </div>

        <div className="g7-field">
          <label htmlFor={`${titleId}-prompt`} className="g7-label">İstem</label>
          <textarea
            id={`${titleId}-prompt`}
            className="dialog-textarea g7-probe-input"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="Ör. TypeScript'te bir LRU önbellek yaz ve karmaşıklığını açıkla."
            maxLength={20_000}
            disabled={running}
          />
        </div>

        <div className="dialog-actions">
          <span className="g7-note">{prompt.length.toLocaleString(uiLocale())} / 20.000 karakter</span>
          {running ? (
            <button type="button" className="btn" onClick={cancel}>
              <Square size={14} aria-hidden="true" /> İptal
            </button>
          ) : (
            <button type="button" className="btn primary" onClick={start} disabled={!prompt.trim() || pair[0] === ''}>
              <Play size={14} aria-hidden="true" /> Çalıştır
            </button>
          )}
        </div>

        <div className="g7-probe-results" aria-live="polite">
          {([0, 1] as const).map((i) => (
            <ProbeColumn key={i} label={models.find((m) => m.id === pair[i])?.name ?? pair[i]} slot={slots[i]} now={now} />
          ))}
        </div>
      </div>
    </G7Modal>
  )
}

function ProbeColumn({ label, slot, now }: { label: string; slot: Slot; now: number }) {
  return (
    <article className="g7-probe-col" aria-label={label} aria-busy={slot.status === 'running'}>
      <header className="g7-probe-head">
        <strong className="truncate">{label}</strong>
        <span className="g7-note">
          {slot.status === 'running' && `Yanıt bekleniyor... ${formatMs(now - slot.startedAt)}`}
          {slot.status === 'aborted' && 'İptal edildi'}
          {slot.status === 'error' && 'Hata'}
          {slot.status === 'done' && 'Tamamlandı'}
        </span>
      </header>
      {slot.status === 'done' && (
        <dl className="g7-probe-stats">
          <div>
            <dt>Süre</dt>
            <dd>{formatMs(slot.result.durationMs)}</dd>
          </div>
          <div>
            <dt>İlk token</dt>
            <dd>{formatMs(slot.result.firstTokenMs)}</dd>
          </div>
          <div>
            <dt>Token (girdi / çıktı)</dt>
            <dd>
              {slot.result.promptTokens?.toLocaleString(uiLocale()) ?? '—'} / {slot.result.completionTokens?.toLocaleString(uiLocale()) ?? '—'}
            </dd>
          </div>
          <div>
            <dt>Maliyet</dt>
            <dd>{formatUsd(slot.result.costUsd, 4)}</dd>
          </div>
        </dl>
      )}
      <div className="g7-probe-body">
        {slot.status === 'error' && <p className="error-text">{slot.message}</p>}
        {slot.status === 'done' &&
          (slot.result.text.trim() ? (
            <div className="prose">
              <Markdown text={slot.result.text} />
            </div>
          ) : (
            <p className="muted">Model metin döndürmedi{slot.result.finishReason ? ` (bitiş nedeni: ${slot.result.finishReason})` : ''}.</p>
          ))}
        {slot.status === 'done' && slot.result.finishReason === 'length' && <p className="g7-note">Yanıt token sınırında kesildi.</p>}
        {slot.status === 'idle' && <p className="muted">Sonuç burada görünecek.</p>}
      </div>
    </article>
  )
}
