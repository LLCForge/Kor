// Model kataloğu: Venice model listesi + Artificial Analysis puanları.
// Saf modül (node importu yok): hem ana süreçte hem arayüz paketinde kullanılır.
import { z } from 'zod'

/** Artificial Analysis'ten doğrulanmış, sadeleştirilmiş model kaydı. */
export interface AaModel {
  id: string
  name: string
  slug: string | null
  creator: string | null
  intelligence: number | null
  coding: number | null
  math: number | null
  /** Saniyede çıktı token (medyan) */
  tokensPerSecond: number | null
  /** İlk token süresi, saniye (medyan) */
  ttftSeconds: number | null
  /** USD / 1M token, 3:1 girdi:çıktı karışımı */
  priceBlended: number | null
}

export interface AaScores {
  id: string
  name: string
  /** false: ad tam eşleşmedi, boyut/tarih etiketleri yok sayılarak eşleşti */
  exact: boolean
  intelligence: number | null
  coding: number | null
  math: number | null
  tokensPerSecond: number | null
  ttftSeconds: number | null
}

export interface ModelPrice {
  /** USD / 1M token */
  input: number
  output: number
  cacheInput: number | null
  cacheWrite: number | null
}

export interface CatalogModel {
  id: string
  name: string
  description: string
  uncensored: boolean
  supportsTools: boolean
  vision: boolean
  reasoning: boolean
  codeOptimized: boolean
  webSearch: boolean
  contextTokens: number | null
  maxCompletionTokens: number | null
  privacy: 'private' | 'anonymized' | null
  traits: string[]
  beta: boolean
  /** Kullanımdan kalkma tarihi (ISO) */
  deprecatedAt: string | null
  price: ModelPrice | null
  aa: AaScores | null
  /** [g9] Sağlayıcı; yoksa kimliğin önekinden çıkarılır (ön eksiz = Venice) */
  provider?: import('./providers.ts').ProviderId
}

const usdSchema = z.object({ usd: z.number().nonnegative() })
const veniceModelSchema = z.object({
  id: z.string().min(1).max(200),
  model_spec: z
    .object({
      name: z.string().max(300).optional(),
      description: z.string().max(5000).optional(),
      uncensored: z.boolean().optional(),
      traits: z.array(z.string().max(100)).max(50).optional(),
      privacy: z.string().max(50).optional(),
      availableContextTokens: z.number().nonnegative().optional(),
      maxCompletionTokens: z.number().nonnegative().optional(),
      betaModel: z.boolean().optional(),
      deprecation: z.object({ date: z.string().max(100).optional() }).nullish(),
      capabilities: z
        .object({
          supportsFunctionCalling: z.boolean().optional(),
          supportsVision: z.boolean().optional(),
          supportsReasoning: z.boolean().optional(),
          optimizedForCode: z.boolean().optional(),
          supportsWebSearch: z.boolean().optional(),
        })
        .optional(),
      pricing: z
        .object({
          input: usdSchema.optional(),
          output: usdSchema.optional(),
          cache_input: usdSchema.optional(),
          cache_write: usdSchema.optional(),
        })
        .optional(),
    })
    .optional(),
})

export type VeniceModel = z.infer<typeof veniceModelSchema>

/** Venice /models yanıtı: bozuk tek bir kayıt tüm listeyi düşürmez, atlanır. */
export function parseVeniceModels(json: unknown): VeniceModel[] {
  const parsed = z.object({ data: z.array(z.unknown()).max(2000) }).parse(json)
  return parsed.data.flatMap((raw) => {
    const m = veniceModelSchema.safeParse(raw)
    return m.success ? [m.data] : []
  })
}

/**
 * Venice kimliği → AA slug/kimliği. null: puan gösterme (ör. farklı ağırlıklı veya çoklu ajan sistemi).
 * Otomatik eşleşmenin yanıldığı durumlar fark edildikçe buraya eklenir.
 */
export const AA_OVERRIDES: Readonly<Record<string, string | null>> = {
  'venice-uncensored-role-play': null,
  'grok-4-20-multi-agent': null,
}

// Satıcı önekleri ve sunum etiketleri ad karşılaştırmasında anlamsızdır.
const DROP_TOKENS = new Set(['instruct', 'it', 'chat', 'latest', 'preview', 'fast', 'highspeed', 'ultrafast', 'openai', 'google', 'meta', 'mistralai', 'fp8'])
// Boyut (235b), etkin parametre (a22b) ve tarih (2507) etiketleri: yalnızca yaklaşık eşleşmede atılır.
const LOOSE_TOKEN = /^(\d+(\.\d+)?b|a\d+(\.\d+)?b|\d{4})$/

/** "Claude Sonnet 4.5" ve "Claude 4.5 Sonnet (Reasoning)" aynı anahtara iner: sıralı token listesi. */
export function nameTokens(name: string): string[] {
  const cleaned = name
    .toLowerCase()
    .replace(/\([^)]*\)|\[[^\]]*\]/g, ' ')
    .replace(/[^a-z0-9.]+/g, ' ')
    .replace(/(?<!\d)\.|\.(?!\d)/g, ' ')
  return cleaned
    .split(' ')
    .filter(Boolean)
    .flatMap((t) => {
      const m = /^([a-z]{2,})(\d[\d.]*)$/.exec(t) // qwen3 → qwen 3, gpt5 → gpt 5 (k2, v3 olduğu gibi kalır)
      return m?.[1] && m[2] ? [m[1], m[2]] : [t]
    })
    .filter((t) => !DROP_TOKENS.has(t))
    .sort()
}

const exactKey = (name: string) => nameTokens(name).join(' ')
const looseKey = (name: string) => nameTokens(name).filter((t) => !LOOSE_TOKEN.test(t)).join(' ')

const isNonReasoningVariant = (aaName: string) => /\(\s*non[-\s]?reasoning/i.test(aaName)

/** Aynı anahtara inen AA varyantlarından (Reasoning / Non-reasoning / high...) Venice modeline en uygun olanı. */
function pickVariant(candidates: readonly AaModel[], reasoning: boolean): AaModel | undefined {
  return [...candidates].sort((a, b) => {
    const fit = (m: AaModel) => (isNonReasoningVariant(m.name) === !reasoning ? 1 : 0)
    return fit(b) - fit(a) || (b.intelligence ?? -1) - (a.intelligence ?? -1)
  })[0]
}

function toScores(m: AaModel, exact: boolean): AaScores {
  return {
    id: m.id,
    name: m.name,
    exact,
    intelligence: m.intelligence,
    coding: m.coding,
    math: m.math,
    tokensPerSecond: m.tokensPerSecond,
    ttftSeconds: m.ttftSeconds,
  }
}

export interface AaIndex {
  byExact: Map<string, AaModel[]>
  byLoose: Map<string, AaModel[]>
  byRef: Map<string, AaModel>
}

export function indexAaModels(models: readonly AaModel[]): AaIndex {
  const index: AaIndex = { byExact: new Map(), byLoose: new Map(), byRef: new Map() }
  const push = (map: Map<string, AaModel[]>, key: string, m: AaModel) => {
    if (!key) return
    const list = map.get(key)
    if (list) list.push(m)
    else map.set(key, [m])
  }
  for (const m of models) {
    push(index.byExact, exactKey(m.name), m)
    push(index.byLoose, looseKey(m.name), m)
    index.byRef.set(m.id, m)
    if (m.slug) index.byRef.set(m.slug, m)
  }
  return index
}

/** Venice modelini AA modeliyle eşleştirir; önce elle düzeltme tablosu, sonra tam ad, sonra yaklaşık ad. */
export function matchAaModel(
  venice: { id: string; name: string; reasoning: boolean },
  index: AaIndex,
  overrides: Readonly<Record<string, string | null>> = AA_OVERRIDES,
): AaScores | null {
  if (Object.hasOwn(overrides, venice.id)) {
    const ref = overrides[venice.id]
    const m = ref ? index.byRef.get(ref) : undefined
    return m ? toScores(m, true) : null
  }
  const exact = pickVariant(index.byExact.get(exactKey(venice.name)) ?? [], venice.reasoning)
  if (exact) return toScores(exact, true)
  const loose = pickVariant(index.byLoose.get(looseKey(venice.name)) ?? [], venice.reasoning)
  return loose ? toScores(loose, false) : null
}

export function buildCatalog(venice: readonly VeniceModel[], aa: readonly AaModel[] | null): CatalogModel[] {
  const index = aa ? indexAaModels(aa) : null
  return venice.map((m): CatalogModel => {
    const spec = m.model_spec
    const caps = spec?.capabilities
    const traits = spec?.traits ?? []
    const p = spec?.pricing
    const name = spec?.name ?? m.id
    const reasoning = caps?.supportsReasoning === true
    return {
      id: m.id,
      name,
      description: spec?.description ?? '',
      uncensored: spec?.uncensored === true || traits.includes('most_uncensored'),
      supportsTools: caps?.supportsFunctionCalling === true,
      vision: caps?.supportsVision === true,
      reasoning,
      codeOptimized: caps?.optimizedForCode === true,
      webSearch: caps?.supportsWebSearch === true,
      contextTokens: spec?.availableContextTokens ?? null,
      maxCompletionTokens: spec?.maxCompletionTokens ?? null,
      privacy: spec?.privacy === 'private' || spec?.privacy === 'anonymized' ? spec.privacy : null,
      traits,
      beta: spec?.betaModel === true,
      deprecatedAt: spec?.deprecation?.date ?? null,
      price:
        p?.input && p.output
          ? { input: p.input.usd, output: p.output.usd, cacheInput: p.cache_input?.usd ?? null, cacheWrite: p.cache_write?.usd ?? null }
          : null,
      aa: index ? matchAaModel({ id: m.id, name, reasoning }, index) : null,
    }
  })
}

/** AA'nın da kullandığı 3:1 girdi:çıktı karışık fiyat (USD / 1M token). */
export function blendedPrice(price: ModelPrice | null): number | null {
  return price ? (3 * price.input + price.output) / 4 : null
}

export function estimateCostUsd(price: ModelPrice | null, promptTokens: number, completionTokens: number): number | null {
  return price ? (promptTokens * price.input + completionTokens * price.output) / 1e6 : null
}

// ---------- Filtre ve sıralama ----------

export const CATALOG_TABS = [
  { id: 'all', label: 'Tümü' },
  { id: 'uncensored', label: 'Sansürsüz' },
  { id: 'coding', label: 'Kodlama' },
  { id: 'reasoning', label: 'Akıl yürütme' },
  { id: 'vision', label: 'Görsel' },
  { id: 'cheap', label: 'Ucuz' },
] as const
export type CatalogTab = (typeof CATALOG_TABS)[number]['id']

/** "Ucuz" sekmesi: karışık fiyatı bu eşiğin altında olanlar (USD / 1M token). */
export const CHEAP_BLENDED_USD = 1
/** "Kodlama" sekmesinde AA kodlama sıralamasında ilk kaç model gösterilir. */
const CODING_TAB_TOP = 15

/** AA kodlama puanına göre sıra (1 = en iyi). Aynı AA modeline eşleşen varyantlar aynı sırayı alır. */
export function codingRanks(models: readonly CatalogModel[]): Map<string, number> {
  const scores = new Map<string, number>()
  for (const m of models) if (m.aa?.coding != null) scores.set(m.aa.id, m.aa.coding)
  const order = [...scores.entries()].sort((a, b) => b[1] - a[1]).map(([id]) => id)
  const ranks = new Map<string, number>()
  for (const m of models) {
    const i = m.aa ? order.indexOf(m.aa.id) : -1
    if (i >= 0) ranks.set(m.id, i + 1)
  }
  return ranks
}

export function matchesTab(m: CatalogModel, tab: CatalogTab, ranks: ReadonlyMap<string, number>): boolean {
  switch (tab) {
    case 'all':
      return true
    case 'uncensored':
      return m.uncensored
    case 'coding':
      return m.codeOptimized || m.traits.includes('default_code') || (ranks.get(m.id) ?? Infinity) <= CODING_TAB_TOP
    case 'reasoning':
      return m.reasoning
    case 'vision':
      return m.vision
    case 'cheap':
      return (blendedPrice(m.price) ?? Infinity) <= CHEAP_BLENDED_USD
  }
}

/** Takma ad → model kimliği. Takma ad değilse girdi aynen döner. */
export function resolveModelAlias(input: string, aliases: Readonly<Record<string, string>>): string {
  const key = input.trim().toLowerCase()
  return Object.hasOwn(aliases, key) ? (aliases[key] ?? input.trim()) : input.trim()
}

export function aliasesFor(id: string, aliases: Readonly<Record<string, string>>): string[] {
  return Object.entries(aliases)
    .filter(([, target]) => target === id)
    .map(([alias]) => alias)
    .sort()
}

export function filterCatalog(
  models: readonly CatalogModel[],
  opts: { tab: CatalogTab; query: string; aliases: Readonly<Record<string, string>> },
): CatalogModel[] {
  const ranks = codingRanks(models)
  const q = opts.query.trim().toLocaleLowerCase('tr')
  return models.filter((m) => {
    if (!matchesTab(m, opts.tab, ranks)) return false
    if (!q) return true
    const haystack = `${m.name} ${m.id} ${aliasesFor(m.id, opts.aliases).join(' ')}`.toLocaleLowerCase('tr')
    return haystack.includes(q)
  })
}

export const SORT_KEYS = [
  { id: 'recommended', label: 'Önerilen' },
  { id: 'coding', label: 'Kodlama puanı' },
  { id: 'intelligence', label: 'Zeka' },
  { id: 'math', label: 'Matematik' },
  { id: 'price', label: 'Fiyat' },
  { id: 'inputPrice', label: 'Girdi fiyatı' },
  { id: 'outputPrice', label: 'Çıktı fiyatı' },
  { id: 'cachePrice', label: 'Önbellek fiyatı' },
  { id: 'context', label: 'Bağlam' },
  { id: 'speed', label: 'Hız' },
  { id: 'name', label: 'Ad' },
] as const
export type SortKey = (typeof SORT_KEYS)[number]['id']
export type SortDir = 'asc' | 'desc'

function metric(m: CatalogModel, key: SortKey): number | string | null {
  switch (key) {
    case 'recommended':
      return null
    case 'coding':
      return m.aa?.coding ?? null
    case 'intelligence':
      return m.aa?.intelligence ?? null
    case 'math':
      return m.aa?.math ?? null
    case 'price':
      return blendedPrice(m.price)
    case 'inputPrice':
      return m.price?.input ?? null
    case 'outputPrice':
      return m.price?.output ?? null
    case 'cachePrice':
      return m.price?.cacheInput ?? null
    case 'context':
      return m.contextTokens
    case 'speed':
      return m.aa?.tokensPerSecond ?? null
    case 'name':
      return m.name.toLocaleLowerCase('tr')
  }
}

/** Fiyat ve ad küçükten büyüğe, puanlar büyükten küçüğe doğaldır. */
export function defaultDir(key: SortKey): SortDir {
  return key === 'name' || key === 'price' || key.endsWith('Price') ? 'asc' : 'desc'
}

/**
 * Sansürsüz modeller her zaman en üstte kalır (kullanıcı isteği); seçilen sıralama her grubun içinde uygulanır.
 * Değeri olmayan (ör. AA puanı yok) modeller grubun sonuna düşer.
 */
export function sortCatalog(models: readonly CatalogModel[], key: SortKey, dir: SortDir = defaultDir(key)): CatalogModel[] {
  const sign = dir === 'asc' ? 1 : -1
  const recommended = (a: CatalogModel, b: CatalogModel) => Number(b.supportsTools) - Number(a.supportsTools) || a.name.localeCompare(b.name)
  return [...models].sort((a, b) => {
    const group = Number(b.uncensored) - Number(a.uncensored)
    if (group !== 0) return group
    const va = metric(a, key)
    const vb = metric(b, key)
    if (va === null && vb === null) return recommended(a, b)
    if (va === null) return 1
    if (vb === null) return -1
    const cmp = typeof va === 'string' || typeof vb === 'string' ? String(va).localeCompare(String(vb), 'tr') : va - vb
    return cmp * sign || recommended(a, b)
  })
}

export interface CodingRecommendations {
  /** AA kodlama puanına göre ilk 3 (araç destekli) */
  best: CatalogModel[]
  /** Puan / fiyat oranı en iyi olan (puanı medyanın üstünde) */
  value: CatalogModel | null
  /** En iyi sansürsüz kodlayıcı */
  uncensored: CatalogModel | null
  /** AA puanı yoksa Venice işaretlerinden (default_code, optimizedForCode) çıkarılan öneriler */
  fromVenice: boolean
}

export function recommendForCoding(models: readonly CatalogModel[]): CodingRecommendations {
  const agentReady = models.filter((m) => m.supportsTools)
  const scored = agentReady.filter((m) => m.aa?.coding != null)
  const byCoding = [...scored].sort((a, b) => (b.aa?.coding ?? 0) - (a.aa?.coding ?? 0))
  if (byCoding.length === 0) {
    // AA verisi yok: Venice'ın "default_code" ve "most_intelligent" işaretleri + koda optimize modeller.
    const flagged = [
      ...agentReady.filter((m) => m.traits.includes('default_code')),
      ...agentReady.filter((m) => m.traits.includes('most_intelligent')),
      ...agentReady.filter((m) => m.codeOptimized),
    ]
    const best = [...new Map(flagged.map((m) => [m.id, m])).values()].slice(0, 3)
    const cheapest = agentReady
      .filter((m) => m.codeOptimized && m.price)
      .sort((a, b) => (blendedPrice(a.price) ?? Infinity) - (blendedPrice(b.price) ?? Infinity))[0]
    return { best, value: cheapest ?? null, uncensored: agentReady.find((m) => m.uncensored && m.codeOptimized) ?? null, fromVenice: true }
  }
  const scores = byCoding.map((m) => m.aa?.coding ?? 0).sort((a, b) => a - b)
  const median = scores[Math.floor(scores.length / 2)] ?? 0
  const value =
    byCoding
      .filter((m) => (m.aa?.coding ?? 0) >= median && (blendedPrice(m.price) ?? 0) > 0)
      .sort((a, b) => (b.aa?.coding ?? 0) / (blendedPrice(b.price) ?? 1) - (a.aa?.coding ?? 0) / (blendedPrice(a.price) ?? 1))[0] ?? null
  return {
    best: byCoding.slice(0, 3),
    value,
    uncensored: byCoding.find((m) => m.uncensored) ?? null,
    fromVenice: false,
  }
}

// ---------- Görev tipine göre model ----------

export const TASK_KINDS = ['explore', 'plan', 'review', 'title', 'compaction'] as const
export type TaskKind = (typeof TASK_KINDS)[number]
export type TaskModels = Partial<Record<TaskKind, string>>

/** Yerleşik alt ajan → görev tipi. Özel ajanlar ana modeli kullanır (kendi model alanı yoksa). */
const TASK_FOR_AGENT: Readonly<Record<string, TaskKind>> = { Explore: 'explore', Plan: 'plan', Review: 'review' }

export function taskForAgent(agentName: string): TaskKind | null {
  return Object.hasOwn(TASK_FOR_AGENT, agentName) ? (TASK_FOR_AGENT[agentName] ?? null) : null
}

/** Ayar değerleri takma ad olabilir ("explore": "cheap"); kimliğe çevrilir. */
export function resolveTaskModels(taskModels: TaskModels, aliases: Readonly<Record<string, string>>): TaskModels {
  const out: TaskModels = {}
  for (const kind of TASK_KINDS) {
    const value = taskModels[kind]
    if (value) out[kind] = resolveModelAlias(value, aliases)
  }
  return out
}

/** Alt ajan modeli: tanımdaki model > görev modeli > ana model. */
export function subagentModel(def: { name: string; model: string | null }, taskModels: TaskModels | undefined, parentModel: string): string {
  const task = taskForAgent(def.name)
  return def.model ?? (task ? taskModels?.[task] : undefined) ?? parentModel
}

export const MODEL_ALIAS_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/
