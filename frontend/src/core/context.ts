import type OpenAI from 'openai'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import type { CompletionUsage } from 'openai/resources/completions'
import { z } from 'zod'
import type { AgentIO, Session } from './agent.ts'
import { fallbackContextLimit } from './context-limits.ts'
import { splitModelId } from './providers.ts'
import { DEFAULT_COMPACT_MODEL } from './settings.ts'
import { pruneOldToolOutputs } from './tool-prune.ts'

/**
 * Otomatik sıkıştırmanın tetiklendiği en yüksek token sayısı (geniş pencerede oranı sınırlar). Çok düşük tutulmamalı:
 * 48k'da birkaç dosya okuması sıkıştırma tetikliyor, her özet önbelleği bozup işi unutturuyordu.
 */
export const MAX_AUTO_COMPACT_TOKENS = 128_000

let compactCeiling = MAX_AUTO_COMPACT_TOKENS
let autoCompactRatio = 0.7

/** Eşik tavanı ve oranı ayarlardan değiştirilebilir (Ayarlar > Bağlam). null: varsayılana dön. */
export function configureCompactTrigger(options: { maxTokens?: number | null; ratio?: number | null }): void {
  if (options.maxTokens !== undefined) compactCeiling = options.maxTokens && options.maxTokens > 0 ? options.maxTokens : MAX_AUTO_COMPACT_TOKENS
  if (options.ratio !== undefined) autoCompactRatio = options.ratio === null ? 0.7 : Math.min(0.95, Math.max(0.3, options.ratio))
}

export function compactTrigger(limit: number): number {
  const byRatio = Math.min(Math.floor(limit * autoCompactRatio), compactCeiling)
  // Taban yalnızca pencere tabana yetecek kadar büyükse uygulanır; 8k'lık pencerede 12k eşik anlamsız olurdu.
  return limit > 17_000 ? Math.max(Math.min(12_000, compactCeiling), byRatio) : byRatio
}

/** Bağlam sınırının bu oranı aşılınca eski turlar özetlenir (üst sınır için compactTrigger). */
export const AUTO_COMPACT_RATIO = 0.7

export function autoCompactRatioNow(): number {
  return autoCompactRatio
}

/** Türkçe metin ve kod için kaba tahmin; bilerek düşük tutuldu ki sınır taşmadan önce sıkıştırılsın. */
const CHARS_PER_TOKEN = 3.5
/** Mesaj başına rol/ayraç ek yükü */
const MESSAGE_OVERHEAD_TOKENS = 4

export interface ModelCatalogEntry {
  contextTokens: number
  supportsTools: boolean
  maxCompletionTokens?: number | null
}

const catalog = new Map<string, ModelCatalogEntry>()
// Katalog başka sağlayıcılarca da dolar; Venice listesinin gelip gelmediği ayrı izlenir.
let veniceCatalogLoaded = false
let catalogAttemptAt = 0
let catalogLoading: Promise<void> | null = null
const CATALOG_RETRY_MS = 5 * 60_000

const catalogSchema = z.object({
  data: z.array(
    z.object({
      id: z.string().max(200),
      model_spec: z
        .object({
          availableContextTokens: z.number().int().positive().optional(),
          maxCompletionTokens: z.number().nonnegative().optional(),
          capabilities: z.object({ supportsFunctionCalling: z.boolean().optional() }).optional(),
        })
        .optional(),
    }),
  ),
})

/** Venice /models yanıtındaki bağlam sınırlarını önbelleğe alır. */
export function ingestModelCatalog(raw: unknown): void {
  for (const m of catalogSchema.parse(raw).data) {
    const contextTokens = m.model_spec?.availableContextTokens
    if (!contextTokens) continue
    catalog.set(m.id, { contextTokens, supportsTools: m.model_spec?.capabilities?.supportsFunctionCalling === true, maxCompletionTokens: m.model_spec?.maxCompletionTokens || null })
    veniceCatalogLoaded = true
  }
}

export function setModelCatalogEntry(id: string, entry: ModelCatalogEntry): void {
  catalog.set(id, entry)
}

export function modelCatalogEntry(id: string): ModelCatalogEntry | undefined {
  return catalog.get(id)
}

export function hasModelCatalog(): boolean {
  return catalog.size > 0
}

export function contextLimitOf(model: string): number | null {
  const known = catalog.get(model)?.contextTokens
  if (known) return known
  // Sağlayıcı listeleri çoğu zaman bağlam boyu vermez; sınır bilinmeyince otomatik sıkıştırma hiç tetiklenmiyordu.
  // Bu yüzden bilinmeyen modelde yedek tabloya düşülür (null dönmez): erken sıkıştırma, taşmadan iyidir.
  const { provider, model: raw } = splitModelId(model)
  return fallbackContextLimit(provider.id, raw)
}

/** Modelin çıktı (completion) sınırı: korunan bütçe bu sınıra göre kalibre edilir. Bilinmiyorsa null. */
export function completionLimitOf(model: string): number | null {
  return catalog.get(model)?.maxCompletionTokens ?? null
}

/** Model listesini bir kez yükler; başarısız olursa 5 dakika boyunca yeniden denemez (her adımda ağ isteği atılmasın). */
export async function ensureModelCatalog(client: OpenAI): Promise<void> {
  if (veniceCatalogLoaded || Date.now() - catalogAttemptAt < CATALOG_RETRY_MS) return
  // Testlerdeki sahte istemcilerde genel get yöntemi yoktur.
  if (typeof (client as { get?: unknown }).get !== 'function') return
  catalogLoading ??= (async () => {
    try {
      ingestModelCatalog(await client.get<unknown>('/models', { query: { type: 'text' }, timeout: 15_000, maxRetries: 1 }))
    } catch (err) {
      console.warn('Model listesi (bağlam sınırları) alınamadı:', err instanceof Error ? err.message : err)
    } finally {
      catalogAttemptAt = Date.now()
      catalogLoading = null
    }
  })()
  await catalogLoading
}

let compactModel = DEFAULT_COMPACT_MODEL

/** Ayarlardaki özetleme modeli; masaüstü ve CLI açılışta çağırır. */
export function configureCompaction(options: { model: string }): void {
  compactModel = options.model
}

export function configuredCompactModel(): string {
  return compactModel
}

/** Oturum başına bağlam ve verimlilik durumu (Session.contextState). */
export interface ContextState {
  /** Son model çağrısının gerçek prompt_tokens değeri */
  lastPromptTokens: number
  /** lastPromptTokens ölçüldüğünde istekteki mesaj sayısı; sonrakiler tahminle eklenir */
  lastMessageCount: number
  promptTokens: number
  cachedTokens: number
  calls: number
  compactions: number
  /** Sıkıştırmaların toplam küçültmesi: sonraki her çağrıda bu kadar token gönderilmez */
  activeReduction: number
  savedTokens: number
  compactionCostTokens: number
  /** Bağlam bu değerin altındaysa otomatik sıkıştırma yeniden denenmez (boşuna tekrar etmesin) */
  minNextCompact: number
  /** Bu turda sıkıştırmanın sildiği net mesaj sayısı; runTurn geri alması bunu hesaba katar */
  turnShift: number
  compacting: boolean
  /** Önbelleği bozmamak için ertelenen sistem istemi yenilemesi; sonraki sıkıştırmada uygulanır */
  refreshSystemPrompt: (() => Promise<string>) | null
  /** Artımlı sayaç: son ölçümden sonra eklenen mesajların tahmini token toplamı */
  tailTokens: number | null
  /** tailTokens hangi uzunlukta hesaplandı */
  tailCount: number
}

export function contextStateOf(session: Session): ContextState {
  session.contextState ??= {
    lastPromptTokens: 0,
    lastMessageCount: 0,
    promptTokens: 0,
    cachedTokens: 0,
    calls: 0,
    compactions: 0,
    activeReduction: 0,
    savedTokens: 0,
    compactionCostTokens: 0,
    minNextCompact: 0,
    turnShift: 0,
    compacting: false,
    refreshSystemPrompt: null,
    tailTokens: null,
    tailCount: 0,
  }
  return session.contextState
}

export function estimateTokens(text: string): number {
  return Math.ceil(text.length / CHARS_PER_TOKEN)
}

/** Mesajın modele giden metni: içerik parçaları ve araç çağrısı argümanları dahil. */
export function messageText(m: ChatCompletionMessageParam): string {
  const content =
    typeof m.content === 'string'
      ? m.content
      : Array.isArray(m.content)
        ? m.content.map((p) => ('text' in p && typeof p.text === 'string' ? p.text : '[ek]')).join('\n')
        : ''
  if (m.role !== 'assistant' || !m.tool_calls) return content
  const calls = m.tool_calls.map((c) => (c.type === 'function' ? `${c.function.name}(${c.function.arguments})` : '')).join('\n')
  return `${content}\n${calls}`
}

export function estimateMessageTokens(m: ChatCompletionMessageParam): number {
  return estimateTokens(messageText(m)) + MESSAGE_OVERHEAD_TOKENS
}

export function estimateMessagesTokens(messages: readonly ChatCompletionMessageParam[]): number {
  return messages.reduce((sum, m) => sum + estimateMessageTokens(m), 0)
}

/** İstekte gerçekten giden geçmiş: eski araç çıktıları kırpılmış hâliyle (agent.ts ile aynı budama). */
function sentMessagesTokens(session: Session): number {
  return estimateMessagesTokens(pruneOldToolOutputs(session.messages))
}

export function toolsTokens(session: Session): number {
  return estimateTokens(JSON.stringify(session.tools.map((t) => [t.name, t.description, t.parameters])))
}

/**
 * Bir sonraki istekte gidecek bağlamın boyutu. Son gerçek ölçüm (prompt_tokens) taban alınır; sonradan eklenen
 * mesajlar her çağrıda baştan tahmin edilmesin diye artımlı sayaç tutulur (append-only: kesme/sıkıştırma tabanı sıfırlar).
 */
export function currentContextTokens(session: Session): number {
  const s = contextStateOf(session)
  if (s.lastPromptTokens > 0 && s.lastMessageCount <= session.messages.length) {
    const tail = session.messages.length - s.lastMessageCount
    if (s.tailTokens !== null && s.tailCount === tail) return s.lastPromptTokens + s.tailTokens
    const measured = estimateMessagesTokens(session.messages.slice(s.lastMessageCount))
    s.tailTokens = measured
    s.tailCount = tail
    return s.lastPromptTokens + measured
  }
  return sentMessagesTokens(session) + toolsTokens(session)
}

/** Sıkıştırmadan sonra ölçüm tabanı tahmine çekilir; sonraki gerçek yanıt bunu düzeltir. */
export function resetContextBaseline(session: Session): number {
  const s = contextStateOf(session)
  const estimate = sentMessagesTokens(session) + toolsTokens(session)
  s.lastPromptTokens = estimate
  s.lastMessageCount = session.messages.length
  s.tailTokens = 0
  s.tailCount = 0
  s.minNextCompact = 0
  return estimate
}

/** Model değişti: bağlam durumu yeni modele göre sıfırlanır (eski sınırın sayacı taşınmaz). */
export function resetContextForModel(session: Session): void {
  const s = contextStateOf(session)
  s.lastPromptTokens = 0
  s.lastMessageCount = 0
  s.tailTokens = null
  s.tailCount = 0
  s.minNextCompact = 0
  s.activeReduction = 0
  s.compactions = 0
}

export interface ContextInfo {
  /** Bir sonraki istekte gidecek tahmini bağlam */
  usedTokens: number
  /** Modelin bağlam sınırı; model listesi alınamadıysa null */
  limitTokens: number | null
  /** Otomatik sıkıştırmanın tetiklendiği gerçek token sayısı (yüzde değil, tavan uygulanmış) */
  compactAtTokens: number | null
  autoCompactRatio: number
  promptTokens: number
  cachedTokens: number
  calls: number
  compactions: number
  /** Sıkıştırma sayesinde gönderilmeyen toplam token (brüt) */
  savedTokens: number
  /** Özetleme çağrılarının harcadığı token */
  compactionCostTokens: number
  /** Bağlam bütçesinin dağılımı (tahmin): yeni sohbet taslağında yok */
  breakdown?: ContextBreakdown
}

export interface ContextBreakdown {
  /** Sistem istemi (talimatlar, ortam, araç politikası) — bellek hariç */
  systemTokens: number
  /** Kurum/kullanıcı belleği, proje bilgisi ve oturum notları */
  memoryTokens: number
  /** Araç tanımları (JSON şema) */
  toolsTokens: number
  /** Konuşma geçmişi (kullanıcı, asistan, araç sonuçları) */
  historyTokens: number
}

// Sistem mesajındaki bellek bölümleri (prompt.ts ve agent-g11.ts başlıkları): bir sonraki aynı/üst düzey başlığa kadar.
const MEMORY_HEADINGS = ['# ORGANIZATION MEMORY', '# MEMORY', '## PROJECT KNOWLEDGE', '## SESSION MEMORY']

function memoryChars(system: string): number {
  let total = 0
  for (const heading of MEMORY_HEADINGS) {
    const start = system.indexOf(`\n${heading}`)
    if (start < 0) continue
    const level = heading.startsWith('## ') ? /\n#{1,2} / : /\n# /
    const rest = system.slice(start + 1 + heading.length)
    const end = rest.search(level)
    total += heading.length + (end < 0 ? rest.length : end)
  }
  return total
}

export function contextBreakdown(session: Session): ContextBreakdown {
  const first = session.messages[0]
  const system = first?.role === 'system' ? messageText(first) : ''
  const memoryTokens = Math.ceil(Math.min(system.length, memoryChars(system)) / CHARS_PER_TOKEN)
  return {
    systemTokens: Math.max(0, estimateTokens(system) - memoryTokens),
    memoryTokens,
    toolsTokens: toolsTokens(session),
    historyTokens: Math.max(0, sentMessagesTokens(session) - (first?.role === 'system' ? estimateMessageTokens(first) : 0)),
  }
}

export function contextInfo(session: Session): ContextInfo {
  const s = contextStateOf(session)
  return {
    breakdown: contextBreakdown(session),
    usedTokens: currentContextTokens(session),
    limitTokens: contextLimitOf(session.model),
    /** Gerçek eşik (token): yüzde değil, tavan uygulanmış hâli. */
    compactAtTokens: contextLimitOf(session.model) ? compactTrigger(contextLimitOf(session.model) ?? 0) : null,
    autoCompactRatio: autoCompactRatioNow(),
    promptTokens: s.promptTokens,
    cachedTokens: s.cachedTokens,
    calls: s.calls,
    compactions: s.compactions,
    savedTokens: s.savedTokens,
    compactionCostTokens: s.compactionCostTokens,
  }
}

/** Henüz oturumu olmayan yeni sohbet taslağı için: yalnızca seçili modelin sınırı. */
export function emptyContextInfo(model: string): ContextInfo {
  return {
    usedTokens: 0,
    limitTokens: contextLimitOf(model),
    compactAtTokens: compactTrigger(contextLimitOf(model) ?? 0),
    autoCompactRatio: autoCompactRatioNow(),
    promptTokens: 0,
    cachedTokens: 0,
    calls: 0,
    compactions: 0,
    savedTokens: 0,
    compactionCostTokens: 0,
  }
}

/** Model çağrısı sonrası: gerçek bağlam boyutu ve önbellek isabeti kaydedilir, arayüz güncellenir. */
export function afterModelCall(session: Session, usage: CompletionUsage | undefined, io: AgentIO): void {
  const s = contextStateOf(session)
  s.calls++
  if (usage) {
    s.lastPromptTokens = usage.prompt_tokens
    s.lastMessageCount = session.messages.length
    s.promptTokens += usage.prompt_tokens
    s.cachedTokens += usage.prompt_tokens_details?.cached_tokens ?? 0
  }
  s.savedTokens += s.activeReduction
  io.contextUpdate?.(contextInfo(session))
}

/**
 * Önbellek dostu sistem istemi yenilemesi: henüz model çağrısı yapılmamış oturumda hemen uygulanır;
 * aksi halde önbelleğe alınmış öneki bozmamak için sonraki sıkıştırmaya ertelenir.
 */
export async function scheduleSystemPromptRefresh(session: Session, build: () => Promise<string>): Promise<void> {
  const s = contextStateOf(session)
  if (s.calls === 0) {
    session.messages[0] = { role: 'system', content: await build() }
    s.refreshSystemPrompt = null
    return
  }
  s.refreshSystemPrompt = build
}

/** runTurn hata geri alması: bu turda sıkıştırma mesajları öne kaydırdıysa tur başlangıcı buna göre düzeltilir. */
export function rollbackLength(session: Session, checkpoint: number): number {
  const shift = session.contextState?.turnShift ?? 0
  if (shift === 0) return checkpoint
  // Turun başı da özetlendiyse özet kalır (dosyalar gerçekten değişti), yalnızca sonrası geri alınır.
  const floor = isSummaryMessage(session.messages[1]) ? 2 : 1
  return Math.min(session.messages.length, Math.max(floor, checkpoint - shift))
}

// ---------- Özet mesajı biçimi ----------

export const SUMMARY_HEADER = '[Önceki konuşmanın özeti]'
const TITLE_PREFIX = 'Sohbet başlığı: '

/** Özet gövdesini ayraç satırlarından ayıran sınır: gövde kullanıcı tarafından düzenlenebildiği için kaçış gerekir. */
const BODY_FENCE = '--- ÖZET ---'

export function formatSummaryMessage(title: string, summary: string): string {
  return [
    SUMMARY_HEADER,
    `${TITLE_PREFIX}${title.replace(/\s+/g, ' ').slice(0, 200)}`,
    'Bu mesaj, bağlam sınırı nedeniyle sıkıştırılan önceki konuşmanın otomatik özetidir. Kullanıcı bunu yazmadı; kaldığın yerden devam et ve özetteki açık işleri unutma.',
    BODY_FENCE,
    // Gövdenin ilk satırı "Sohbet başlığı: " ile başlasa da kaçmaz; sınır satırı araya girer.
    summary.trim(),
  ].join('\n')
}

export function isSummaryMessage(m: ChatCompletionMessageParam | undefined): boolean {
  return m?.role === 'user' && typeof m.content === 'string' && m.content.startsWith(SUMMARY_HEADER)
}

/** Özet mesajındaki asıl özet metni (başlık satırları hariç). Eski biçim (sınır satırsız) de okunur. */
export function summaryBody(content: string): string {
  const lines = content.split('\n')
  const fence = lines.indexOf(BODY_FENCE)
  if (fence >= 0) return lines.slice(fence + 1).join('\n').trim()
  return lines.slice(lines[1]?.startsWith(TITLE_PREFIX) ? 4 : 1).join('\n').trim()
}

/** Özeti düzenler; başlık ve açıklama satırları korunur (kullanıcı yalnızca gövdeyi düzeltir). */
export function rewriteSummaryMessage(content: string, body: string): string {
  const lines = content.split('\n')
  const fence = lines.indexOf(BODY_FENCE)
  const head = fence >= 0 ? lines.slice(0, fence + 1) : [SUMMARY_HEADER, ...(lines[1]?.startsWith(TITLE_PREFIX) ? [lines[1] as string] : []), lines[2] ?? '', BODY_FENCE]
  return [...head, body.trim()].join('\n')
}

/** Sıkıştırmadan önceki sohbet başlığı; geçmiş listesinde başlık değişmesin. */
export function summaryTitle(content: string): string | null {
  const line = content.split('\n')[1]
  return line?.startsWith(TITLE_PREFIX) ? line.slice(TITLE_PREFIX.length).trim() || null : null
}
