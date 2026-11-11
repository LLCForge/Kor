// Model fiyat/yetenek kaydı ve "en ucuz yeterli model" seçimi (madde 51, 61, 62).
// Saf modül (node importu yok): ana süreç, CLI ve arayüz paketi birlikte kullanır.
import { blendedPrice, type CatalogModel, type ModelPrice } from './model-catalog.ts'
import { providerOf, type ProviderId } from './providers.ts'
import type { PriceEntry } from './settings-g9.ts'

export interface CapabilityNeeds {
  tools?: boolean
  vision?: boolean
  /** En az bu kadar bağlam (token); bağlamı bilinmeyen model elenir */
  minContext?: number
}

export function modelProvider(m: { id: string; provider?: ProviderId | undefined }): ProviderId {
  return m.provider ?? providerOf(m.id)
}

/** Fiyat tablosu satırı katalogdaki fiyatı ve bağlam boyutunu ezer (yapılandırılabilir fiyat tablosu). */
export function applyPriceTable(models: readonly CatalogModel[], table: Readonly<Record<string, PriceEntry>>): CatalogModel[] {
  return models.map((m) => {
    const row = Object.hasOwn(table, m.id) ? table[m.id] : undefined
    if (!row) return m
    return {
      ...m,
      price: { input: row.input, output: row.output, cacheInput: row.cacheInput ?? m.price?.cacheInput ?? null, cacheWrite: m.price?.cacheWrite ?? null },
      contextTokens: row.contextTokens ?? m.contextTokens,
    }
  })
}

export function meetsNeeds(m: CatalogModel, needs: CapabilityNeeds): boolean {
  if (needs.tools && !m.supportsTools) return false
  if (needs.vision && !m.vision) return false
  if (needs.minContext && (m.contextTokens ?? 0) < needs.minContext) return false
  return true
}

/** Yeteneklere yeten, fiyatı bilinen ve sağlayıcısı kullanılabilir modellerin en ucuzu; eşitlikte bağlamı büyük olan. */
export function cheapestCapable(
  models: readonly CatalogModel[],
  needs: CapabilityNeeds,
  usable: (provider: ProviderId) => boolean = () => true,
): CatalogModel | null {
  let best: CatalogModel | null = null
  let bestPrice = Infinity
  for (const m of models) {
    const price = blendedPrice(m.price)
    if (price === null || m.deprecatedAt || !meetsNeeds(m, needs) || !usable(modelProvider(m))) continue
    const better = price < bestPrice || (price === bestPrice && best !== null && (m.contextTokens ?? 0) > (best.contextTokens ?? 0))
    if (better) {
      best = m
      bestPrice = price
    }
  }
  return best
}

// ---------- Süreç içi kayıt (ana süreç ve CLI) ----------

const registry = new Map<string, CatalogModel>()
let table: Readonly<Record<string, PriceEntry>> = {}
let fallback: ((id: string) => { input: number; output: number } | undefined) | null = null

/** Katalog yenilendikçe çağrılır; aynı kimlik yeni kayıtla değişir. */
export function registerModels(models: readonly CatalogModel[]): void {
  for (const m of models) registry.set(m.id, m)
}

export function registeredModels(): CatalogModel[] {
  return applyPriceTable([...registry.values()], table)
}

export function setPriceTable(next: Readonly<Record<string, PriceEntry>>): void {
  table = next
}

export function priceTable(): Readonly<Record<string, PriceEntry>> {
  return table
}

/** Katalog yüklenmeden önce (ör. masaüstünün eski model listesi) fiyat kaynağı. */
export function setPriceFallback(fn: ((id: string) => { input: number; output: number } | undefined) | null): void {
  fallback = fn
}

/** Öncelik: kullanıcı fiyat tablosu > sağlayıcı kataloğu > yedek kaynak. Bilinmiyorsa null. */
export function priceOf(id: string): ModelPrice | null {
  const row = Object.hasOwn(table, id) ? table[id] : undefined
  if (row) return { input: row.input, output: row.output, cacheInput: row.cacheInput ?? null, cacheWrite: null }
  const known = registry.get(id)?.price
  if (known) return known
  const p = fallback?.(id)
  return p ? { input: p.input, output: p.output, cacheInput: null, cacheWrite: null } : null
}
