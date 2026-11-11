// Ayarlar > Şablonlar (§15.11): `kor templates list|show|new` ve `kor run template:<ad>` karşılıkları.
// Çalıştır, doğrulanmış istemi mesaj kutusuna koyar (cdx:draft); göndermeyi kullanıcı yapar.
import { FilePlus2, LayoutTemplate, Play } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { TaskTemplate, TemplateInput } from '../../ipc-templates.ts'
import { api, errorMessage } from '../api.ts'
import { buildTemplateInputs } from './template-inputs.ts'
import '../panels/graph/panel.css'
import '../panels/security/panel.css'

const SOURCES: Record<TaskTemplate['source'], string> = { project: 'proje', personal: 'kişisel', plugin: 'eklenti', builtin: 'yerleşik' }

function InputField({ input, value, onChange }: { input: TemplateInput; value: string | boolean; onChange: (v: string | boolean) => void }) {
  const id = useId()
  const label = `${input.name}${input.required ? ' *' : ''}`
  if (input.kind.type === 'bool') {
    return (
      <label className="ks-check">
        <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />
        <span translate="no">{input.name}</span>
        {input.description && <span className="muted" translate="no">{input.description}</span>}
      </label>
    )
  }
  return (
    <div className="ks-field">
      <label htmlFor={id} translate="no">{label}</label>
      {input.kind.type === 'enum' ? (
        <select id={id} className="g7-select" value={String(value)} onChange={(e) => onChange(e.target.value)}>
          {!input.required && <option value="">—</option>}
          {input.kind.values.map((v) => (
            <option key={v} value={v} translate="no">{v}</option>
          ))}
        </select>
      ) : (
        <input id={id} className="text-input" type={input.kind.type === 'number' ? 'number' : 'text'} value={String(value)} onChange={(e) => onChange(e.target.value)} placeholder={input.default ?? ''} spellCheck={false} />
      )}
      {input.description && <span className="muted" translate="no">{input.description}</span>}
    </div>
  )
}

function TemplateDetail({ name }: { name: string }) {
  const { data, error } = useSWR(['g5:template', name], () => api.templates.show(name))
  const [values, setValues] = useState<Record<string, string | boolean>>({})
  const [busy, setBusy] = useState(false)
  const [runError, setRunError] = useState<string | null>(null)
  const [sent, setSent] = useState(false)

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted" role="status" data-kor-cmd="templates.show">Şablon okunuyor…</p>

  const run = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setRunError(null)
    setSent(false)
    try {
      const r = await api.templates.run(data.name, buildTemplateInputs(data.inputs, values))
      window.dispatchEvent(new CustomEvent('cdx:draft', { detail: r.prompt }))
      setSent(true)
    } catch (err) {
      setRunError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="ks-preview" onSubmit={(e) => void run(e)} data-kor-cmd="templates.show">
      <p translate="no">{data.description}</p>
      {data.path && <p className="muted ks-note" translate="no">{data.path}</p>}
      {data.inputs.map((i) => (
        <InputField key={i.name} input={i} value={values[i.name] ?? (i.kind.type === 'bool' ? i.default === 'true' : '')} onChange={(v) => setValues((s) => ({ ...s, [i.name]: v }))} />
      ))}
      {data.steps.length > 0 && (
        <details>
          <summary>{`Adımlar (${data.steps.length})`}</summary>
          <ol translate="no">{data.steps.map((s, i) => <li key={i}>{s}</li>)}</ol>
        </details>
      )}
      {data.acceptance.length > 0 && (
        <details>
          <summary>{`Kabul ölçütleri (${data.acceptance.length})`}</summary>
          <ul translate="no">{data.acceptance.map((s, i) => <li key={i}>{s}</li>)}</ul>
        </details>
      )}
      {runError && <p className="error-text" role="alert">{runError}</p>}
      {sent && <p className="muted" role="status">İstem mesaj kutusuna kondu; gözden geçirip gönderin.</p>}
      <button type="submit" className="btn small primary" disabled={busy} data-kor-cmd="templates.run">
        <Play size={14} aria-hidden="true" />
        Mesaj kutusuna koy
      </button>
    </form>
  )
}

function NewTemplate({ onDone }: { onDone: () => void }) {
  const id = useId()
  const [name, setName] = useState('')
  const [personal, setPersonal] = useState(false)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const r = await api.templates.new(name.trim(), personal)
      setResult(r.path)
      setName('')
      onDone()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="ks-form" onSubmit={(e) => void submit(e)}>
      <div className="ks-inline">
        <label htmlFor={`${id}-n`} className="sr-only">Şablon adı</label>
        <input id={`${id}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="yeni-sablon" maxLength={64} pattern="[A-Za-z0-9_\-]+" spellCheck={false} translate="no" />
        <button type="submit" className="btn small" disabled={busy || !name.trim()} data-kor-cmd="templates.new">
          <FilePlus2 size={14} aria-hidden="true" />
          Oluştur
        </button>
      </div>
      <label className="ks-check">
        <input type="checkbox" checked={personal} onChange={(e) => setPersonal(e.target.checked)} />
        Kişisel (~/.kor/templates); işaretsizse projede (.kor/templates)
      </label>
      {result && <p className="muted" role="status">Oluşturuldu: <code translate="no">{result}</code></p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </form>
  )
}

export function TemplatesSection() {
  const id = useId()
  const { data, error, mutate } = useSWR('templates', () => api.templates.list())
  const [selected, setSelected] = useState<string | null>(null)
  return (
    <div className="ks ks-settings">
      <section className="kg-card" aria-labelledby={`${id}-t`}>
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <LayoutTemplate size={16} aria-hidden="true" />
          Görev şablonları
        </h3>
        {error ? (
          <p className="error-text" role="alert">{errorMessage(error)}</p>
        ) : !data ? (
          <p className="muted" role="status" data-kor-cmd="templates.list">Şablonlar okunuyor…</p>
        ) : (
          <ul className="ks-list" aria-label="Şablonlar" data-kor-cmd="templates.list">
            {data.templates.map((t) => (
              <li key={t.name}>
                <button type="button" className="kg-link" aria-expanded={selected === t.name} onClick={() => setSelected(selected === t.name ? null : t.name)}>
                  <span translate="no">{t.name}</span>
                  <span className="kg-link-meta">{SOURCES[t.source]}</span>
                </button>
                {selected === t.name && <TemplateDetail name={t.name} />}
              </li>
            ))}
          </ul>
        )}
        {data && data.warnings.length > 0 && <p className="ks-warn" translate="no">{data.warnings.join('\n')}</p>}
        <details className="ks-more">
          <summary>Yeni şablon</summary>
          <NewTemplate onDone={() => void mutate()} />
        </details>
      </section>
    </div>
  )
}
