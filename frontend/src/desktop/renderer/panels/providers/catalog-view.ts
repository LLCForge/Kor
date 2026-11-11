// Katalog tarayıcısının saf mantığı (arama, sıralama, sağlayıcıya göre gruplama, biçimler); node testli.
import type { CatalogEntry } from '../../../ipc-providers.ts'
import { uiLocale, agoIntl } from '../../ui-locale.ts'

export type SortKey = 'intelligence' | 'coding' | 'priceIn' | 'context'

export const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  intelligence: 'Zekâ',
  coding: 'Kodlama',
  priceIn: 'Giriş fiyatı',
  context: 'Bağlam',
}

/** Sağlayıcı kimliği: katalog alanı, yoksa `sağlayıcı/model` önekinden; ikisi de yoksa `diğer`. */
export function providerOf(m: CatalogEntry): string {
  const p = m.provider?.trim()
  if (p) return p.toLowerCase()
  const i = m.model_id.indexOf('/')
  return i > 0 ? m.model_id.slice(0, i).toLowerCase() : 'diğer'
}

/** Bilinmeyen sağlayıcı logosu yerine harf monogramı (§23.2). */
export function monogram(provider: string): string {
  return (provider.match(/[\p{L}\p{N}]/u)?.[0] ?? '?').toLocaleUpperCase('tr')
}

export function matches(m: CatalogEntry, query: string): boolean {
  const words = query.toLocaleLowerCase('tr').split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const hay = `${m.model_id} ${m.provider ?? ''} ${m.license ?? ''}`.toLocaleLowerCase('tr')
  return words.every((w) => hay.includes(w))
}

function value(m: CatalogEntry, key: SortKey): number | null {
  switch (key) {
    case 'intelligence':
      return m.aa_intelligence_index
    case 'coding':
      return m.aa_coding_index
    case 'priceIn':
      return m.input_price_per_m
    case 'context':
      return m.context
  }
}

/** Eksik değerler her zaman sonda; fiyat artan, diğerleri azalan. */
export function compare(key: SortKey): (a: CatalogEntry, b: CatalogEntry) => number {
  const dir = key === 'priceIn' ? 1 : -1
  return (a, b) => {
    const x = value(a, key)
    const y = value(b, key)
    if (x === null || y === null) return x === y ? a.model_id.localeCompare(b.model_id) : x === null ? 1 : -1
    return x === y ? a.model_id.localeCompare(b.model_id) : (x - y) * dir
  }
}

export interface ProviderGroup {
  provider: string
  models: CatalogEntry[]
}

/** Arama + sıralama + gruplama; gruplar en iyi modellerinin sırasıyla dizilir. */
export function groupCatalog(models: readonly CatalogEntry[], query: string, key: SortKey): ProviderGroup[] {
  const sorted = models.filter((m) => matches(m, query)).sort(compare(key))
  const groups = new Map<string, CatalogEntry[]>()
  for (const m of sorted) {
    const p = providerOf(m)
    const g = groups.get(p)
    if (g) g.push(m)
    else groups.set(p, [m])
  }
  return [...groups.entries()].map(([provider, list]) => ({ provider, models: list }))
}

/** En yüksek skor (çubuk ölçeği için); veri yoksa 0. */
export function maxOf(models: readonly CatalogEntry[], pick: (m: CatalogEntry) => number | null): number {
  let max = 0
  for (const m of models) max = Math.max(max, pick(m) ?? 0)
  return max
}

export function formatBytes(n: number | null): string {
  if (n === null || !Number.isFinite(n) || n < 0) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toLocaleString(uiLocale(), { maximumFractionDigits: i === 0 ? 0 : 1 })} ${units[i]}`
}

/** Unix saniyesinden "5 dk önce" gibi kısa göreli zaman. */
export function ago(unixSecs: number, nowMs: number = Date.now()): string {
  const s = Math.max(0, Math.round(nowMs / 1000 - unixSecs))
  const intl = agoIntl(s)
  if (intl) return intl
  if (s < 60) return 'az önce'
  if (s < 3600) return `${Math.floor(s / 60)} dk önce`
  if (s < 86400) return `${Math.floor(s / 3600)} sa önce`
  return `${Math.floor(s / 86400)} gün önce`
}
