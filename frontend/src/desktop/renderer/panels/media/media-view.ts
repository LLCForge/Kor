// Medya paneli ve parametre kartı için saf yardımcılar (§20, ADR 0060): etiketler, fiyat metni, plandan
// kart, kart yanıtı, galeri öğesinden hızlı eylem ön ayarı.
import type { Capability, CatalogModel, CatalogPrice, GalleryItem, JsonValue, MediaPlan, ParamCard, ParamField } from '../../../ipc-media.ts'

export const CAPABILITIES: readonly { id: Capability; label: string }[] = [
  { id: 'image', label: 'Görsel' },
  { id: 'edit', label: 'Düzenle' },
  { id: 'upscale', label: 'Büyüt' },
  { id: 'remove-bg', label: 'Arka planı kaldır' },
  { id: 'video', label: 'Video' },
  { id: 'image-to-video', label: 'Görselden video' },
  { id: 'tts', label: 'Seslendir' },
  { id: 'asr', label: 'Yazıya dök' },
  { id: 'audio', label: 'Ses efekti' },
  { id: 'music', label: 'Müzik' },
]

export function capabilityLabel(c: string | null | undefined): string {
  return CAPABILITIES.find((x) => x.id === c)?.label ?? (c || '—')
}

/** Girdi dosyası gerektiren yetenekler (referans zorunlu). */
export function needsReference(c: Capability): boolean {
  return c === 'edit' || c === 'upscale' || c === 'remove-bg' || c === 'image-to-video' || c === 'asr'
}

/** Prompt gerektirmeyen yetenekler. */
export function promptOptional(c: Capability): boolean {
  return c === 'upscale' || c === 'remove-bg' || c === 'asr'
}

export function valueText(v: JsonValue | null | undefined): string {
  if (v === null || v === undefined) return ''
  return typeof v === 'string' ? v : JSON.stringify(v)
}

/** `$0.03/görsel` ya da sağlayıcı fiyatı yoksa `?`. */
export function priceText(p: CatalogPrice | null): string {
  if (!p) return '?'
  const amount = p.currency === 'USD' ? `$${p.amount}` : `${p.amount} ${p.currency}`
  return `${amount}/${p.unit}`
}

export function modelRef(m: CatalogModel): string {
  return `${m.provider}:${m.id}`
}

/** Plan → kart: sorulacaklar, modelin yapamadığı değerler (en yakını varsayılan), maliyet, süre. */
export function cardFromPlan(p: MediaPlan, prompt: string): ParamCard {
  const fields: ParamField[] = p.plan.questions.map((q) => ({
    name: q.name,
    description: q.description,
    options: q.options,
    default: q.default,
    min: q.min,
    max: q.max,
  }))
  const notes: string[] = []
  for (const u of p.plan.unsupported) {
    const near = u.nearest === null ? '' : `; en yakını: ${valueText(u.nearest)}`
    notes.push(`${u.name} “${valueText(u.asked)}” bu modelde yok${near} (izin verilen: ${u.allowed})`)
    const param = p.model.params.find((x) => x.name === u.name)
    if (param) {
      fields.push({
        name: param.name,
        description: param.description ?? '',
        options: param.options ?? [],
        default: u.nearest ?? param.default ?? null,
        min: param.min ?? null,
        max: param.max ?? null,
      })
    }
  }
  return {
    title: `${modelRef(p.model)}: ${prompt}`.slice(0, 120),
    fields,
    notes,
    cost: p.estimateText,
    eta: p.eta === null ? null : `≈ ${p.eta} sn`,
    confirm: p.confirm,
    target: null,
    fixed: p.plan.input,
  }
}

/** Kartın başlangıç seçimleri: her alanın varsayılanı. */
export function initialValues(card: ParamCard): Record<string, JsonValue> {
  const out: Record<string, JsonValue> = {}
  for (const f of card.fields) if (f.default !== null && f.default !== undefined) out[f.name] = f.default
  return out
}

/** Serbest değerli alanın metni → değer: sayı aralıkta olmalı; seçenekli alanda seçenek. `null` = geçersiz. */
export function parseFree(f: ParamField, text: string): JsonValue | null {
  const t = text.trim()
  if (t === '') return f.default ?? null
  const opt = f.options.find((o) => valueText(o).toLowerCase() === t.toLowerCase())
  if (opt !== undefined) return opt
  if (f.options.length > 0) return null
  const n = Number(t)
  if (Number.isFinite(n) && (f.min !== null || f.max !== null || typeof f.default === 'number')) {
    if (f.min !== null && n < f.min) return null
    if (f.max !== null && n > f.max) return null
    return n
  }
  if (t === 'true' || t === 'false') return t === 'true'
  return t
}

/** Kart eksiksiz mi: varsayılanı olmayan her alan seçilmiş olmalı. */
export function missingFields(card: ParamCard, values: Record<string, JsonValue>): string[] {
  return card.fields.filter((f) => values[f.name] === undefined).map((f) => f.name)
}

export type QuickAction = 'variation' | 'reseed' | 'upscale' | 'video' | 'remove-bg' | 'edit'

export const QUICK_ACTIONS: readonly { id: QuickAction; label: string }[] = [
  { id: 'variation', label: 'Varyasyon üret' },
  { id: 'reseed', label: 'Aynı seed, prompt değiştir' },
  { id: 'upscale', label: 'Upscale' },
  { id: 'video', label: 'Videoya çevir' },
  { id: 'remove-bg', label: 'Arka planı kaldır' },
  { id: 'edit', label: 'Düzenle' },
]

/** Üret sekmesine taşınan ön ayar. */
export interface Preset {
  capability: Capability
  model: string | null
  prompt: string
  params: Record<string, JsonValue>
  reference: string | null
  derivedFrom: string | null
  action: string | null
}

const SKIP = new Set(['prompt', 'text', 'model', 'name', 'ask', 'capability', 'action', 'save_as', 'derived_from', 'provider', 'input', 'image'])

/** Öğenin kendi model girdileri (dosyalar ve araç alanları hariç; `input` içindekiler de). */
function modelParams(item: GalleryItem, keepSeed: boolean): Record<string, JsonValue> {
  const out: Record<string, JsonValue> = {}
  const add = (o: Record<string, JsonValue>) => {
    for (const [k, v] of Object.entries(o)) {
      if (SKIP.has(k) || k.endsWith('_url') || k.endsWith('_urls') || (k === 'seed' && !keepSeed)) continue
      out[k] = v
    }
  }
  if (item.params && typeof item.params === 'object' && !Array.isArray(item.params)) {
    const p = item.params as Record<string, JsonValue>
    add(p)
    const inner = p['input']
    if (inner && typeof inner === 'object' && !Array.isArray(inner)) add(inner as Record<string, JsonValue>)
  }
  if (keepSeed && item.seed !== null) out['seed'] = item.seed
  return out
}

export function presetFor(item: GalleryItem, action: QuickAction | 'reference'): Preset {
  const own = (item.capability as Capability | null) ?? (item.kind === 'video' ? 'video' : 'image')
  const ownModel = item.model ? `${item.provider}:${item.model}` : null
  const base: Preset = { capability: own, model: null, prompt: item.prompt, params: {}, reference: null, derivedFrom: null, action }
  switch (action) {
    case 'reference':
      return { ...base, capability: 'edit', reference: item.id, action: null }
    case 'variation':
      return { ...base, model: ownModel, params: modelParams(item, false), derivedFrom: item.id }
    case 'reseed':
      return { ...base, model: ownModel, prompt: '', params: modelParams(item, true), derivedFrom: item.id }
    case 'upscale':
      return { ...base, capability: 'upscale', reference: item.id }
    case 'video':
      return { ...base, capability: 'image-to-video', reference: item.id }
    case 'remove-bg':
      return { ...base, capability: 'remove-bg', reference: item.id }
    case 'edit':
      return { ...base, capability: 'edit', prompt: '', reference: item.id }
  }
}

/** Zincir satırı: `kedi.png ← [upscale] taslak.png`. */
export function costText(item: GalleryItem): string {
  if (item.cost === null) return ''
  const approx = item.estimated ? '≈' : ''
  return item.currency === 'USD' || item.currency === null ? `${approx}$${item.cost.toFixed(3)}` : `${approx}${item.cost.toFixed(3)} ${item.currency}`
}
