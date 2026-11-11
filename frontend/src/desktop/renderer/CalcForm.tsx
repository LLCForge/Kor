// [g8] ```calc hesap makinesi (#101) + slider parametre denemesi (#103) ve ```form form önizleme (#102).
//      GÜVENLİK: formül güvenli ayrıştırıcıyla değerlendirilir (safe-expr; eval/new Function YOK). Form gönderimi
//      yalnızca yerel özet çıkarır; dış gönderim/ağ yoktur.
import { Calculator, ClipboardList } from 'lucide-react'
import { useMemo, useState } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { evaluateExpr, expressionVariables } from './safe-expr.ts'
import { BlockFrame, parseJsonBlock, RawFallback } from './rich-shared.tsx'

interface CalcVar {
  name: string
  label?: string
  value?: number
  min?: number
  max?: number
  step?: number
  slider?: boolean
}
interface CalcSpec {
  title?: string
  formula: string
  unit?: string
  variables?: CalcVar[]
}

function CalcView({ spec }: { spec: CalcSpec }) {
  // Değişkenler tanımlıysa onları, değilse formülden çıkarılanları (varsayılan 0) kullan.
  const vars = useMemo<CalcVar[]>(() => {
    if (spec.variables?.length) return spec.variables.filter((v) => typeof v?.name === 'string')
    return expressionVariables(spec.formula).map((name) => ({ name }))
  }, [spec])
  const [values, setValues] = useState<Record<string, number>>(() => Object.fromEntries(vars.map((v) => [v.name, Number(v.value) || 0])))

  const result = useMemo(() => {
    try {
      return { ok: true as const, value: evaluateExpr(spec.formula, values) }
    } catch (err) {
      return { ok: false as const, error: err instanceof Error ? err.message : 'Hesaplanamadı' }
    }
  }, [spec.formula, values])

  return (
    <BlockFrame
      icon={<Calculator size={14} aria-hidden="true" />}
      label={spec.title ?? 'Hesap makinesi'}
      className="g8-calc"
      tools={result.ok ? <CopyButton className="action-btn" label="Sonucu kopyala" getText={() => String(result.value)} /> : undefined}
    >
      <code className="g8-formula">{spec.formula}</code>
      <div className="g8-fields">
        {vars.map((v) => {
          const val = values[v.name] ?? 0
          const set = (n: number) => setValues((prev) => ({ ...prev, [v.name]: n }))
          return (
            <label key={v.name} className="g8-field">
              <span className="g8-field-label">{v.label ?? v.name}</span>
              <span className="g8-field-input">
                {v.slider ? (
                  <>
                    <input
                      type="range"
                      min={v.min ?? 0}
                      max={v.max ?? 100}
                      step={v.step ?? 1}
                      value={val}
                      onChange={(e) => set(Number(e.target.value))}
                      aria-label={v.label ?? v.name}
                    />
                    <output className="g8-field-out">{val}</output>
                  </>
                ) : (
                  <input
                    type="number"
                    className="g8-input"
                    value={Number.isFinite(val) ? val : 0}
                    min={v.min}
                    max={v.max}
                    step={v.step}
                    onChange={(e) => set(Number(e.target.value))}
                    aria-label={v.label ?? v.name}
                  />
                )}
              </span>
            </label>
          )
        })}
      </div>
      <div className={`g8-result${result.ok ? '' : ' g8-result-error'}`} role="status" aria-live="polite">
        {result.ok ? (
          <>
            = <strong>{formatNumber(result.value)}</strong>
            {spec.unit ? ` ${spec.unit}` : ''}
          </>
        ) : (
          result.error
        )}
      </div>
    </BlockFrame>
  )
}

/** Sonucu okunabilir biçimde yuvarlar (uzun kayan nokta kuyruklarını kısaltır). */
function formatNumber(n: number): string {
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 1e6) / 1e6)
}

export function CalcBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<CalcSpec>(code)
  if (!parsed.ok || typeof parsed.data?.formula !== 'string') return <RawFallback code={code} note={parsed.ok ? 'formula alanı gerekli' : parsed.error} />
  return <CalcView spec={parsed.data} />
}

// --- Form önizleme (#102) -----------------------------------------------------

interface FormField {
  name: string
  label?: string
  type?: 'text' | 'number' | 'email' | 'textarea' | 'select' | 'checkbox'
  options?: string[]
  placeholder?: string
  required?: boolean
}
interface FormSpec {
  title?: string
  fields: FormField[]
}

function FormView({ spec }: { spec: FormSpec }) {
  const fields = spec.fields.filter((f) => typeof f?.name === 'string')
  const [values, setValues] = useState<Record<string, string | boolean>>({})
  const [summary, setSummary] = useState<string | null>(null)
  const set = (name: string, value: string | boolean) => setValues((prev) => ({ ...prev, [name]: value }))

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    // Yerel özet: değerler yalnızca metne dönüştürülür; hiçbir yere gönderilmez.
    const lines = fields.map((f) => {
      const raw = values[f.name]
      const text = typeof raw === 'boolean' ? (raw ? 'evet' : 'hayır') : (raw ?? '')
      return `${f.label ?? f.name}: ${text}`
    })
    setSummary(lines.join('\n'))
  }

  return (
    <BlockFrame icon={<ClipboardList size={14} aria-hidden="true" />} label={spec.title ?? 'Form'} className="g8-form">
      <form onSubmit={submit} className="g8-fields">
        {fields.map((f) => (
          <label key={f.name} className="g8-field">
            <span className="g8-field-label">
              {f.label ?? f.name}
              {f.required ? ' *' : ''}
            </span>
            <span className="g8-field-input">{renderField(f, values[f.name], set)}</span>
          </label>
        ))}
        <button type="submit" className="action-btn g8-submit">
          Özet çıkar
        </button>
      </form>
      {summary !== null && (
        <div className="g8-summary">
          <div className="g8-summary-head">
            <span>Yerel özet (gönderilmedi)</span>
            <CopyButton className="action-btn" label="Özeti kopyala" getText={() => summary} />
          </div>
          <pre>{summary}</pre>
        </div>
      )}
    </BlockFrame>
  )
}

function renderField(f: FormField, value: string | boolean | undefined, set: (name: string, value: string | boolean) => void) {
  const label = f.label ?? f.name
  if (f.type === 'checkbox') {
    return <input type="checkbox" checked={Boolean(value)} onChange={(e) => set(f.name, e.target.checked)} aria-label={label} />
  }
  if (f.type === 'textarea') {
    return <textarea className="g8-input" value={String(value ?? '')} placeholder={f.placeholder} onChange={(e) => set(f.name, e.target.value)} aria-label={label} />
  }
  if (f.type === 'select') {
    return (
      <select className="g8-input" value={String(value ?? '')} onChange={(e) => set(f.name, e.target.value)} aria-label={label}>
        <option value="">—</option>
        {(f.options ?? []).map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    )
  }
  return (
    <input
      type={f.type === 'number' ? 'number' : f.type === 'email' ? 'email' : 'text'}
      className="g8-input"
      value={String(value ?? '')}
      placeholder={f.placeholder}
      onChange={(e) => set(f.name, e.target.value)}
      aria-label={label}
    />
  )
}

export function FormBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<FormSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.fields)) return <RawFallback code={code} note={parsed.ok ? 'fields dizisi gerekli' : parsed.error} />
  return <FormView spec={parsed.data} />
}

registerRichBlock('calc', CalcBlock)
registerRichBlock('form', FormBlock)
