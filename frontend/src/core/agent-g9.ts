// runTurn'deki [g9] kancaları: görev başına maliyet tavanı (53), canlı akış maliyet sayacı ve kesme (54),
// parametre ön ayarları (59) ve düşük maliyet modu (62). Fiyatlar model-registry-g9.ts'ten okunur.
import type { AgentIO, Session, TokenUsage } from './agent.ts'
import { currentContextTokens } from './context.ts'
import { originalMessagesOf } from './tool-prune.ts' // [perf]
import { cheapestCapable, priceOf, registeredModels } from './model-registry-g9.ts'
import type { TaskModels } from './model-catalog.ts'
import { isProviderUsable, onChatStream } from './provider-pool-g9.ts'
import type { ParamPreset } from './settings-g9.ts'

export interface TurnCostInfo {
  /** Bu görevin (tur + alt ajanları) harcaması, USD; akış sürerken tahmin içerir */
  turnUsd: number
  capUsd: number | null
  /** Akış sürüyor: değer çıktı uzunluğundan tahmin ediliyor */
  live: boolean
  /** Fiyatı bilinmeyen model kullanıldı: tutar eksik olabilir */
  unpriced: boolean
}

export interface G9CoreConfig {
  costCapUsd: number | null
  lowCostMode: boolean
  presets: Readonly<Record<string, ParamPreset>>
  /** Model kimliği → ön ayar adı */
  modelPresets: Readonly<Record<string, string>>
}

let core: G9CoreConfig = { costCapUsd: null, lowCostMode: false, presets: {}, modelPresets: {} }

export function configureG9Core(patch: Partial<G9CoreConfig>): void {
  core = { ...core, ...patch }
}

/** Akış sırasında çıktı token tahmini (Türkçe/kod için kaba): 4 karakter = 1 token. */
const CHARS_PER_TOKEN = 4
const EMIT_INTERVAL_MS = 250
/** Düşük maliyet modunda özetleme modelinin yeterli sayılması için en az bağlam */
const COMPACTION_MIN_CONTEXT = 32_000

export function usageCostUsd(usage: ReadonlyMap<string, TokenUsage>): { usd: number; unpriced: boolean } {
  let usd = 0
  let unpriced = false
  for (const [model, u] of usage) {
    const price = priceOf(model)
    if (!price) {
      if (u.promptTokens + u.completionTokens > 0) unpriced = true
      continue
    }
    usd += (u.promptTokens * price.input + u.completionTokens * price.output) / 1e6
  }
  return { usd, unpriced }
}

interface TurnCost {
  baseUsd: number
  /** Akıştaki çağrıların tahmini (çağrı bitince gerçek kullanım usage haritasına düşer) */
  live: Map<object, number>
  lastEmit: number
  stopped: boolean
}

/** Üst oturum ve alt ajanları aynı usage haritasını paylaşır: görev maliyeti o haritaya bağlanır. */
const turns = new WeakMap<Map<string, TokenUsage>, TurnCost>()
/** Akış kancası gövdedeki mesaj dizisinden oturumu bulur (dizinin kimliği oturum boyunca sabittir). */
const watched = new WeakMap<object, { session: Session; io: AgentIO }>()
const lowCostFilled = new WeakMap<Session, TaskModels>()

function turnInfo(session: Session, t: TurnCost, live: boolean): TurnCostInfo {
  const { usd, unpriced } = usageCostUsd(session.usage)
  const pending = [...t.live.values()].reduce((a, b) => a + b, 0)
  return { turnUsd: Math.max(0, usd - t.baseUsd) + pending, capUsd: core.costCapUsd, live, unpriced }
}

const fmt = (usd: number) => `$${usd.toFixed(4)}`

/** Açıkça seçilmemiş yan iş modellerini (başlık, özetleme) en ucuz yeterli modele yönlendirir; kapatılınca geri alır. */
function applyLowCost(session: Session): void {
  const previous = lowCostFilled.get(session) ?? {}
  const current: TaskModels = { ...(session.taskModels ?? {}) }
  for (const [kind, model] of Object.entries(previous) as [keyof TaskModels, string][]) {
    if (current[kind] === model) delete current[kind]
  }
  const filled: TaskModels = {}
  if (core.lowCostMode) {
    const models = registeredModels()
    const title = current.title ? null : cheapestCapable(models, {}, isProviderUsable)
    const compaction = current.compaction ? null : cheapestCapable(models, { minContext: COMPACTION_MIN_CONTEXT }, isProviderUsable)
    if (title) filled.title = title.id
    if (compaction) filled.compaction = compaction.id
  }
  if (Object.keys(previous).length === 0 && Object.keys(filled).length === 0) return
  session.taskModels = { ...current, ...filled }
  lowCostFilled.set(session, filled)
}

/**
 * Her model çağrısından önce. true dönerse tur durur (görev maliyet tavanı aşıldı).
 * Üst düzey oturumun ilk adımı görevin başlangıcıdır; alt ajanlar aynı görev hesabına yazar.
 */
export function g9BeforeModelCall(session: Session, step: number, io: AgentIO): boolean {
  applyLowCost(session)
  let t = turns.get(session.usage)
  if (!t || (step === 0 && session.agents.length > 0)) {
    t = { baseUsd: usageCostUsd(session.usage).usd, live: new Map(), lastEmit: 0, stopped: false }
    turns.set(session.usage, t)
  }
  watched.set(session.messages, { session, io })
  const info = turnInfo(session, t, false)
  io.g9Cost?.(info)
  if (core.costCapUsd !== null && info.turnUsd >= core.costCapUsd) {
    t.stopped = true
    io.write(`\n[Görev maliyet tavanı aşıldı (${fmt(info.turnUsd)} / ${fmt(core.costCapUsd)}); tur durduruldu. Tavanı Ayarlar > Sağlayıcılar bölümünden değiştirebilirsin.]\n`)
    return true
  }
  return false
}

/** İstek gövdesine eklenen ön ayar alanları (yalnızca tanımlı olanlar). */
export function g9RequestFields(session: Pick<Session, 'model'>): { temperature?: number; top_p?: number; max_completion_tokens?: number } {
  const name = Object.hasOwn(core.modelPresets, session.model) ? core.modelPresets[session.model] : undefined
  const preset = name && Object.hasOwn(core.presets, name) ? core.presets[name] : undefined
  if (!preset) return {}
  return {
    ...(preset.temperature !== undefined ? { temperature: preset.temperature } : {}),
    ...(preset.top_p !== undefined ? { top_p: preset.top_p } : {}),
    ...(preset.maxTokens !== undefined ? { max_completion_tokens: preset.maxTokens } : {}),
  }
}

// Canlı sayaç: yalnızca runTurn'ün kendi çağrıları (gövdedeki mesaj dizisi izlenen bir oturumunki) sayılır.
onChatStream((body, stream, fromCache) => {
  const w = watched.get(originalMessagesOf(body.messages))
  const t = w ? turns.get(w.session.usage) : undefined
  if (!w || !t || fromCache) return
  const price = priceOf(w.session.model)
  if (!price) return
  const key = {}
  const promptUsd = (currentContextTokens(w.session) * price.input) / 1e6
  let chars = 0
  const update = (delta: string) => {
    chars += delta.length
    t.live.set(key, promptUsd + (Math.ceil(chars / CHARS_PER_TOKEN) * price.output) / 1e6)
    const info = turnInfo(w.session, t, true)
    if (core.costCapUsd !== null && info.turnUsd > core.costCapUsd && !t.stopped) {
      t.stopped = true
      w.io.write(`\n[Görev maliyet tavanı aşıldı (tahmini ${fmt(info.turnUsd)} / ${fmt(core.costCapUsd)}); akış kesildi.]\n`)
      stream.abort()
      return
    }
    const now = Date.now()
    if (now - t.lastEmit >= EMIT_INTERVAL_MS) {
      t.lastEmit = now
      w.io.g9Cost?.(info)
    }
  }
  stream.on('content.delta', ({ delta }) => update(delta))
  stream.on('chunk', (chunk) => {
    const d = chunk.choices[0]?.delta as { reasoning_content?: unknown; tool_calls?: Array<{ function?: { arguments?: string } }> } | undefined
    if (typeof d?.reasoning_content === 'string') update(d.reasoning_content)
    for (const call of d?.tool_calls ?? []) if (call.function?.arguments) update(call.function.arguments)
  })
  // runTurn gerçek kullanımı await'ten hemen sonra ekler; makro görevde okununca tahmin gerçeğiyle yer değiştirir.
  const settle = () =>
    setTimeout(() => {
      t.live.delete(key)
      w.io.g9Cost?.(turnInfo(w.session, t, false))
    }, 0)
  stream.finalChatCompletion().then(settle, settle)
})
