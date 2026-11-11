// Web siteleri paneli: form alanlarından köprü nesnesi kurma ve sonucu metne çevirme (saf, node testli).

export type FieldType = 'text' | 'textarea' | 'number' | 'checkbox' | 'select'

export interface Field {
  name: string
  /** Türkçe etiket */
  label: string
  type?: FieldType
  options?: readonly string[]
  placeholder?: string
  required?: boolean
  /** Virgül/satır ile ayrılmış listeye çevrilir */
  list?: boolean
}

export type FormState = Record<string, string | boolean>

/** Boş alanlar atlanır; sayılar sayıya, listeler diziye, onay kutuları booleana çevrilir. */
export function buildArgs(fields: readonly Field[], state: FormState): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const f of fields) {
    const v = state[f.name]
    if (f.type === 'checkbox') {
      if (v === true) out[f.name] = true
      continue
    }
    const s = typeof v === 'string' ? v.trim() : ''
    if (!s) continue
    if (f.type === 'number') {
      const n = Number(s.replace(',', '.'))
      if (Number.isFinite(n)) out[f.name] = n
    } else if (f.list) {
      out[f.name] = splitList(s)
    } else {
      out[f.name] = s
    }
  }
  return out
}

/** Zorunlu ama boş alanların etiketleri. */
export function missing(fields: readonly Field[], state: FormState): string[] {
  return fields.filter((f) => f.required && f.type !== 'checkbox' && !(typeof state[f.name] === 'string' && (state[f.name] as string).trim())).map((f) => f.label)
}

export function splitList(s: string): string[] {
  return s.split(/[,\n]/).map((x) => x.trim()).filter(Boolean)
}

/** `{ markdown }` ya da `{ text }` varsa o; dizi/nesne ise okunur JSON; yoksa düz metin. */
export function resultText(r: unknown): string {
  if (r == null) return ''
  if (typeof r === 'string') return r
  if (typeof r === 'object' && !Array.isArray(r)) {
    const o = r as Record<string, unknown>
    for (const k of ['markdown', 'text', 'xml']) if (typeof o[k] === 'string') return o[k] as string
  }
  return JSON.stringify(r, null, 2)
}

/** Sonuçtaki kaydedilmiş dosya yolları (`files`, `saved`, `dir`, `path`). */
export function resultFiles(r: unknown): string[] {
  if (!r || typeof r !== 'object') return []
  if (Array.isArray(r)) return r.every((x) => typeof x === 'string' && /[\\/]/.test(x)) ? (r as string[]) : []
  const o = r as Record<string, unknown>
  const out: string[] = []
  if (Array.isArray(o.files)) out.push(...o.files.filter((x): x is string => typeof x === 'string'))
  for (const k of ['saved', 'dir', 'path']) if (typeof o[k] === 'string') out.push(o[k] as string)
  return out
}
