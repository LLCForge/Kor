// §20.2 parametre kartı: her açık seçim tek kartta, tıklanabilir çiplerle; varsayılanlar işaretli, Enter
// varsayılanlarla geçer. Maliyet eşiği aşılırsa düğme onay ister. Hem ajanın sorusu (PrefsLayer,
// `paramCard` olayı) hem medya panelinin Üret sekmesi kullanır.
import { Coins, Clock, SlidersHorizontal } from 'lucide-react'
import { useEffect, useId, useRef, useState, type FormEvent } from 'react'
import type { JsonValue, ParamCard as Card, ParamField } from '../../../ipc-media.ts'
import { initialValues, missingFields, parseFree, valueText } from './media-view.ts'
import './panel.css'

function Field({ f, value, onChange }: { f: ParamField; value: JsonValue | undefined; onChange: (v: JsonValue | undefined) => void }) {
  const labelId = useId()
  const [text, setText] = useState(value === undefined ? '' : valueText(value))
  const [bad, setBad] = useState(false)
  if (f.options.length > 0) {
    return (
      <div className="km-field" role="group" aria-labelledby={labelId}>
        <span id={labelId} className="km-field-name" translate="no">{f.name}</span>
        <div className="km-chips">
          {f.options.map((o) => {
            const on = value !== undefined && valueText(value) === valueText(o)
            const isDefault = f.default !== null && valueText(f.default) === valueText(o)
            return (
              <button
                key={valueText(o)}
                type="button"
                className={`km-chip${on ? ' on' : ''}`}
                aria-pressed={on}
                onClick={() => onChange(o)}
                title={isDefault ? 'Varsayılan' : undefined}
              >
                <span translate="no">{valueText(o)}</span>
                {isDefault && <span className="km-default" aria-hidden="true">•</span>}
              </button>
            )
          })}
        </div>
        {f.description && <span className="km-hint" translate="no">{f.description}</span>}
      </div>
    )
  }
  const range = f.min !== null || f.max !== null ? ` (${f.min ?? ''}–${f.max ?? ''})` : ''
  return (
    <label className="km-field">
      <span className="km-field-name" translate="no">{f.name}{range}</span>
      <input
        className={bad ? 'km-bad' : undefined}
        value={text}
        placeholder={f.default === null ? '' : valueText(f.default)}
        aria-invalid={bad}
        onChange={(e) => {
          setText(e.target.value)
          const v = parseFree(f, e.target.value)
          setBad(v === null && e.target.value.trim() !== '')
          onChange(v === null ? undefined : v)
        }}
      />
      {f.description && <span className="km-hint" translate="no">{f.description}</span>}
    </label>
  )
}

export function ParamCardView({
  card,
  busy = false,
  onSubmit,
  onCancel,
  submitLabel = 'Üret',
  cmd,
}: {
  card: Card
  busy?: boolean
  onSubmit: (values: Record<string, JsonValue>) => void
  onCancel?: () => void
  submitLabel?: string
  /** §16.3 `data-kor-cmd` of the backend call the buttons make. */
  cmd?: string
}) {
  const titleId = useId()
  const [values, setValues] = useState<Record<string, JsonValue>>(() => initialValues(card))
  const [agreed, setAgreed] = useState(false)
  const first = useRef<HTMLButtonElement>(null)
  useEffect(() => first.current?.focus(), [])
  const missing = missingFields(card, values)
  const blocked = busy || missing.length > 0 || (card.confirm && !agreed)
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!blocked) onSubmit(values)
  }
  return (
    <form className="km-card" aria-labelledby={titleId} onSubmit={submit}>
      <div className="km-card-head">
        <SlidersHorizontal size={16} aria-hidden="true" />
        <span id={titleId} className="km-card-title" translate="no">{card.title}</span>
      </div>
      {card.notes.length > 0 && (
        <ul className="km-notes" role="status">
          {card.notes.map((n) => (
            <li key={n} translate="no">{n}</li>
          ))}
        </ul>
      )}
      {card.fields.map((f) => (
        <Field
          key={f.name}
          f={f}
          value={values[f.name]}
          onChange={(v) =>
            setValues((prev) => {
              const next = { ...prev }
              if (v === undefined) delete next[f.name]
              else next[f.name] = v
              return next
            })
          }
        />
      ))}
      <div className="km-meta">
        {card.cost && (
          <span className="km-meta-item">
            <Coins size={14} aria-hidden="true" /> Tahmini maliyet: <span translate="no">{card.cost}</span>
          </span>
        )}
        {card.eta && (
          <span className="km-meta-item">
            <Clock size={14} aria-hidden="true" /> Süre: <span translate="no">{card.eta}</span>
          </span>
        )}
      </div>
      {card.confirm && (
        <label className="km-confirm">
          <input type="checkbox" checked={agreed} onChange={(e) => setAgreed(e.target.checked)} />
          Bu maliyet eşiğin (media.confirm_above) üstünde; onaylıyorum
        </label>
      )}
      {missing.length > 0 && <p className="km-hint" role="alert">Seçilmesi gereken: <span translate="no">{missing.join(', ')}</span></p>}
      <div className="km-actions">
        {onCancel && (
          <button type="button" className="btn" onClick={onCancel} disabled={busy} data-kor-cmd={cmd}>
            Vazgeç
          </button>
        )}
        <button ref={first} type="submit" className="btn primary" disabled={blocked} data-kor-cmd={cmd}>
          {submitLabel} (↵)
        </button>
      </div>
    </form>
  )
}
