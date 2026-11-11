// Tek bir işlem kartı: alanlar, çalıştır düğmesi (data-kor-cmd çağıranın verdiği yöntem), sonuç ve hata.
// `danger`: geri alınamaz işler için düğme önce "Emin misiniz?" sorar.
import { Play, TriangleAlert } from 'lucide-react'
import { useId, useState } from 'react'
import { errorMessage } from '../../api.ts'
import { buildArgs, type Field, type FormState, missing, resultFiles, resultText } from './forms.ts'

export interface ActionProps {
  title: string
  /** Kısa açıklama (Türkçe) */
  note?: string
  fields: readonly Field[]
  run: (args: Record<string, unknown>) => Promise<unknown>
  'data-kor-cmd': string
  button?: string
  danger?: boolean
  initial?: FormState
  onDone?: (r: unknown) => void
}

export function FieldInput({ f, value, onChange, id }: { f: Field; value: string | boolean | undefined; onChange: (v: string | boolean) => void; id: string }) {
  if (f.type === 'checkbox') {
    return (
      <label className="ks-check" htmlFor={id}>
        <input id={id} type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />
        {f.label}
      </label>
    )
  }
  return (
    <label className="ks-field" htmlFor={id}>
      {f.label}
      {f.type === 'textarea' ? (
        <textarea id={id} className="ks-textarea" value={typeof value === 'string' ? value : ''} placeholder={f.placeholder} onChange={(e) => onChange(e.target.value)} />
      ) : f.type === 'select' ? (
        <select id={id} value={typeof value === 'string' ? value : ''} onChange={(e) => onChange(e.target.value)}>
          <option value="">—</option>
          {(f.options ?? []).map((o) => (
            <option key={o} value={o} translate="no">{o}</option>
          ))}
        </select>
      ) : (
        <input id={id} type="text" inputMode={f.type === 'number' ? 'decimal' : undefined} value={typeof value === 'string' ? value : ''} placeholder={f.placeholder} onChange={(e) => onChange(e.target.value)} />
      )}
    </label>
  )
}

export function Result({ value }: { value: unknown }) {
  const files = resultFiles(value)
  const text = resultText(value)
  return (
    <div className="kw-result">
      {files.length > 0 && (
        <ul className="kw-files">
          {files.map((f) => (
            <li key={f} translate="no">{f}</li>
          ))}
        </ul>
      )}
      {text && <pre className="ks-code kw-out" translate="no">{text}</pre>}
    </div>
  )
}

export function Action(props: ActionProps) {
  const id = useId()
  const [state, setState] = useState<FormState>(props.initial ?? {})
  const [busy, setBusy] = useState(false)
  const [armed, setArmed] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<unknown>(undefined)

  const go = async () => {
    const gaps = missing(props.fields, state)
    if (gaps.length) {
      setError(`Eksik: ${gaps.join(', ')}`)
      return
    }
    if (props.danger && !armed) {
      setArmed(true)
      return
    }
    setArmed(false)
    setBusy(true)
    setError(null)
    try {
      const r = await props.run(buildArgs(props.fields, state))
      setResult(r)
      props.onDone?.(r)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <details className="ks-more kw-action">
      <summary>{props.title}</summary>
      <form
        className="ks-form"
        onSubmit={(e) => {
          e.preventDefault()
          void go()
        }}
      >
        {props.note && <p className="ks-note muted">{props.note}</p>}
        {props.fields.map((f) => (
          <FieldInput key={f.name} id={`${id}-${f.name}`} f={f} value={state[f.name]} onChange={(v) => setState((s) => ({ ...s, [f.name]: v }))} />
        ))}
        <div className="ks-actions">
          <button type="submit" className={`btn small ${props.danger && armed ? 'danger' : 'primary'}`} disabled={busy} data-kor-cmd={props['data-kor-cmd']}>
            {props.danger && armed ? <TriangleAlert size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
            {busy ? 'Çalışıyor…' : props.danger && armed ? 'Eminim, uygula' : (props.button ?? 'Çalıştır')}
          </button>
          {armed && (
            <button type="button" className="btn small" onClick={() => setArmed(false)}>
              Vazgeç
            </button>
          )}
        </div>
        {busy && <p className="muted" role="status">Çalışıyor…</p>}
        {error && <p className="error-text" role="alert">{error}</p>}
        {result !== undefined && !busy && <Result value={result} />}
      </form>
    </details>
  )
}
