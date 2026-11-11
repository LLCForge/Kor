// Çoklu sağlayıcı istemci havuzu (madde 47-49, 55, 56, 64; sağlayıcı izolasyonu).
// Her sağlayıcı OpenAI uyumlu olduğundan tek `openai` SDK'si yeterli; farklar yalnızca taşıma katmanında ele alınır:
//  - model kimliğindeki "<sağlayıcı>:" öneki tel üzerinde atılır (ham kimlik gider),
//  - Venice'e özgü alanlar (venice_parameters, prompt_cache_key) başka sağlayıcıya gönderilmez,
//  - araç desteklemediği bilinen modellere tools gönderilmez, Ollama max_tokens ister.
// Yönlendirme model kimliğiyle yapılır: oturum istemcisi (createClient) chat çağrılarını modelin sağlayıcısına,
// diğer her şeyi (katalog, web araması, faturalama) Venice'e iletir. Böylece özetleme, başlık, proje haritası ve
// alt ajanlar kendi modellerinin sağlayıcısını otomatik kullanır.
import { createHash } from 'node:crypto'
import OpenAI from 'openai'
import { Semaphore } from './agents.ts'
import { replayResponse, cachedCompletion, rememberCompletion } from './response-cache-g9.ts'
import { bridgeFetch, isVeniceModel, PROVIDERS, providerInfo, providerOf, type Provider, type ProviderId } from './providers.ts'
import { FORBIDDEN_HEADER, type ProviderEndpoint } from './settings-g9.ts'

export type FetchLike = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

/** Venice anahtarı olmadan createClient'a verilen yer tutucu; asla ağa gönderilmez (yerel 401 döner). */
export const NO_VENICE_KEY = 'g9-no-venice-key'
/** Sağlayıcı istemcilerinin SDK'deki anahtarı; gerçek anahtar her istekte havuzdan eklenir (SDK nesnesinde sır tutulmaz). */
const MANAGED_KEY = 'g9-managed-key'

export interface PoolConfig {
  /** Sağlayıcı başına anahtarlar: ilki etkin, sonrakiler 401/429'da sırayla denenen yedekler */
  keys: Partial<Record<ProviderId, readonly string[]>>
  endpoints: Partial<Record<ProviderId, ProviderEndpoint>>
  /** Yerel Ollama yalnızca kullanıcı açınca kullanılabilir */
  ollamaEnabled: boolean
  /** Araç çağırmayı desteklemediği bilinen modeller (tam kimlik) */
  noToolModels: ReadonlySet<string>
  /** Testler sahte fetch verir */
  fetchImpl: FetchLike
}

let config: PoolConfig = {
  keys: {},
  endpoints: {},
  ollamaEnabled: false,
  noToolModels: new Set(),
  fetchImpl: (input, init) => fetch(input, init),
}

const clients = new Map<ProviderId, OpenAI>()
const slots = new Map<ProviderId, Semaphore>()
/** Anahtar → bu zamana kadar (epoch ms) yedeğe geç; 401 kalıcı (yapılandırma değişene kadar) */
const cooled = new Map<string, number>()
/** Sağlayıcı "kalan istek 0" dediyse sıfırlanma anına kadar yeni istek bekletilir */
const blockedUntil = new Map<ProviderId, number>()
/** Kullanıcının sildiği veya değiştirdiği anahtarlar: eski oturum istemcileri de bunları bir daha göndermez. */
const revoked = new Set<string>()

export function configureProviders(patch: Partial<PoolConfig>): void {
  if (patch.keys) {
    const next = new Set(Object.values(patch.keys).flat())
    for (const k of Object.values(config.keys).flat()) if (!next.has(k)) revoked.add(k)
    for (const k of next) revoked.delete(k)
  }
  config = { ...config, ...patch }
  // İstemci temel URL'si ve kuyruk boyutu uç nokta ayarından gelir; anahtar değişince soğuma sıfırlanır.
  if (patch.endpoints) {
    clients.clear()
    slots.clear()
  }
  if (patch.keys) cooled.clear()
}

export function effectiveBaseURL(id: ProviderId): string {
  return (config.endpoints[id]?.baseURL ?? providerInfo(id).baseURL).replace(/\/+$/, '')
}

export function isProviderUsable(id: ProviderId): boolean {
  const p = providerInfo(id)
  if (p.keyless) return id !== 'ollama' || config.ollamaEnabled
  // Köprü sağlayıcısı (Venice web hesabı): kimlik oturumdan gelir, profil anahtarı yoktur.
  if (p.bridge) return (config.keys[id]?.length ?? 0) > 0
  return (config.keys[id]?.length ?? 0) > 0
}

export function usableProviders(): ProviderId[] {
  return PROVIDERS.filter((p) => isProviderUsable(p.id)).map((p) => p.id)
}

/**
 * Oturum açmak için anahtar: Venice anahtarı varsa o, yoksa başka kullanılabilir sağlayıcı varsa yer tutucu
 * (Venice'e giden istekler yerelde reddedilir). Hiçbiri yoksa Türkçe hata.
 */
export function chatKey(veniceKey: string | null): string {
  if (veniceKey) return veniceKey
  if (usableProviders().some((id) => id !== 'venice')) return NO_VENICE_KEY
  throw new Error("API anahtarı gerekli: Ayarlar > Sağlayıcılar bölümünden bir anahtar ekleyin veya yerel Ollama'yı etkinleştirin.")
}

function refusal(status: number, message: string): Response {
  return new Response(JSON.stringify({ error: { message, type: 'g9_local_refusal' } }), { status, headers: { 'content-type': 'application/json' } })
}

function missingKeyMessage(p: Provider): string {
  if (p.bridge) return `${p.label} oturumu yok. Ayarlar > Bağdaştırıcılar bölümünden e-posta ve parolayla giriş yapın.`
  return p.venice
    ? 'Venice API anahtarı tanımlı değil. Bu işlem (Venice modeli, web araması veya Venice kataloğu) Venice anahtarı gerektirir; Ayarlar > Sağlayıcılar bölümünden ekleyin veya başka sağlayıcının modelini seçin.'
    : `${p.label} API anahtarı tanımlı değil. Ayarlar > Sağlayıcılar bölümünden ekleyin.`
}

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return '(geçersiz adres)'
  }
}

function sameOrigin(url: string, base: string): boolean {
  try {
    return new URL(url).origin === new URL(base).origin
  } catch {
    return false
  }
}

function bearerOf(headers: Headers): string | null {
  return headers.get('authorization')?.replace(/^Bearer\s+/i, '') || null
}

function keysFor(id: ProviderId, fromHeader: string | null): string[] {
  const configured = config.keys[id] ?? []
  const own = id === 'venice' && fromHeader && fromHeader !== NO_VENICE_KEY && fromHeader !== MANAGED_KEY ? [fromHeader] : []
  const unique = [...new Set([...own, ...configured])].filter((k) => !revoked.has(k))
  const now = Date.now()
  // Soğumadaki anahtarlar sona: hepsi soğumadaysa yine de denenir (sağlayıcı kendi hatasını döner).
  return [...unique.filter((k) => (cooled.get(k) ?? 0) <= now), ...unique.filter((k) => (cooled.get(k) ?? 0) > now)]
}

/** Ham sıfırlama başlığı → bekleme (ms). Venice epoch ms, OpenAI/Groq "6m0s" gibi süre, bazıları saniye verir. */
export function resetDelayMs(raw: string | null, now = Date.now()): number | null {
  if (!raw) return null
  const n = Number(raw)
  if (raw.trim() !== '' && Number.isFinite(n)) {
    if (n > 1e12) return n - now
    if (n > 1e9) return n * 1000 - now
    return n * 1000
  }
  const units: Record<string, number> = { ms: 1, s: 1_000, m: 60_000, h: 3_600_000 }
  let total = 0
  let matched = false
  for (const m of raw.matchAll(/(\d+(?:\.\d+)?)(ms|s|m|h)/g)) {
    matched = true
    total += Number(m[1]) * (units[m[2] ?? 's'] ?? 1_000)
  }
  return matched ? total : null
}

const MAX_RATE_WAIT_MS = 60_000

function noteRateHeaders(id: ProviderId, headers: Headers): void {
  if (headers.get('x-ratelimit-remaining-requests') !== '0') return
  const delay = resetDelayMs(headers.get('x-ratelimit-reset-requests')) ?? 1_000
  blockedUntil.set(id, Date.now() + Math.min(Math.max(delay, 0), MAX_RATE_WAIT_MS))
}

function coolDown(key: string, res: Response): void {
  if (res.status === 401) {
    cooled.set(key, Infinity)
    return
  }
  const delay = resetDelayMs(res.headers.get('retry-after')) ?? (res.status === 402 ? 5 * 60_000 : 60_000)
  cooled.set(key, Date.now() + Math.min(Math.max(delay, 1_000), 10 * 60_000))
  pruneCooled()
}

/** Soğuma kaydı sınırsız büyümesin: süresi geçmiş girdiler atılır (401 = Infinity, kalır). */
function pruneCooled(): void {
  if (cooled.size < 512) return
  const now = Date.now()
  for (const [key, until] of cooled) if (until <= now) cooled.delete(key)
}

/** Bir anahtar 401 aldıysa (kullanıcı anahtarı düzeltebilir): soğumayı çöz, sonraki istek onu yine dener. */
export function reviveKeys(keys: readonly string[]): void {
  for (const key of keys) cooled.delete(key)
}

/** Soğuma haritasının boyutu (testler ve tanılama için). */
export function cooledSize(): number {
  return cooled.size
}

/**
 * Sağlayıcı hatasını sınıflandırır: anahtar değiştirmek hangi durumda işe yarar?
 * auth (401/403), quota (402 — kredi bitti, yedek anahtar da bitmiş olabilir), rate (429), context (413 — sıkıştır),
 * server (5xx), other.
 */
export type FailureKind = 'auth' | 'quota' | 'rate' | 'context' | 'server' | 'other'

export function classifyFailure(status: number): FailureKind {
  if (status === 401 || status === 403) return 'auth'
  if (status === 402) return 'quota'
  if (status === 429) return 'rate'
  if (status === 413) return 'context'
  if (status >= 500) return 'server'
  return 'other'
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error('İptal edildi')
}

function sleep(ms: number, signal: AbortSignal | null | undefined): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(abortReason(signal))
    const onAbort = () => {
      clearTimeout(timer)
      if (signal) reject(abortReason(signal))
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    signal?.addEventListener('abort', onAbort, { once: true })
  })
}

function semaphoreFor(id: ProviderId): Semaphore | null {
  const max = config.endpoints[id]?.maxConcurrent
  if (!max) return null
  let s = slots.get(id)
  if (!s) {
    s = new Semaphore(max)
    slots.set(id, s)
  }
  return s
}

/** Yanıt gövdesi tamamen okunduğunda, hata verdiğinde veya iptal edildiğinde done bir kez çağrılır (akış süresince yuva tutulur). */
function onBodyDone(res: Response, done: () => void): Response {
  if (!res.body) {
    done()
    return res
  }
  let finished = false
  const finish = () => {
    if (finished) return
    finished = true
    done()
  }
  const reader = res.body.getReader()
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const chunk = await reader.read()
        if (chunk.done) {
          finish()
          controller.close()
        } else {
          controller.enqueue(chunk.value)
        }
      } catch (err) {
        finish()
        controller.error(err)
      }
    },
    cancel(reason) {
      finish()
      return reader.cancel(reason)
    },
  })
  return new Response(body, { status: res.status, statusText: res.statusText, headers: res.headers })
}

// Anthropic ailesi önbelleği otomatik değil, açık kırılma noktası ister. Venice ve OpenRouter içerik parçasındaki
// cache_control'u iletir (docs.venice.ai chat/completions; openrouter.ai/docs prompt caching).
const CACHE_BREAKPOINT_PROVIDERS: ReadonlySet<ProviderId> = new Set(['venice', 'openrouter'])
const ANTHROPIC_MODEL = /claude|anthropic/i

function withCacheControl(message: unknown): unknown {
  if (typeof message !== 'object' || message === null) return message
  const m = message as { content?: unknown }
  const mark = { type: 'ephemeral' }
  if (typeof m.content === 'string') {
    return m.content ? { ...m, content: [{ type: 'text', text: m.content, cache_control: mark }] } : message
  }
  if (!Array.isArray(m.content)) return message
  const parts = [...(m.content as unknown[])]
  for (let i = parts.length - 1; i >= 0; i--) {
    const p = parts[i]
    if (typeof p === 'object' && p !== null && (p as { type?: unknown }).type === 'text') {
      parts[i] = { ...p, cache_control: mark }
      return { ...m, content: parts }
    }
  }
  return message
}

/**
 * İki kırılma noktası (Anthropic sınırı 4): sistem mesajı (araç tanımları + sistem istemi önbelleğe girer) ve son
 * mesaj (konuşma her turda kaldığı yerden okunur). Yalnızca giden gövde değişir; oturum geçmişi string kalır.
 */
export function addCacheBreakpoints(messages: readonly unknown[]): unknown[] {
  const out = [...messages]
  const sys = out.findIndex((m) => typeof m === 'object' && m !== null && (m as { role?: unknown }).role === 'system')
  if (sys >= 0) out[sys] = withCacheControl(out[sys])
  const last = out.length - 1
  if (last > sys && last >= 0) out[last] = withCacheControl(out[last])
  return out
}

/** Venice API'sine özel alanlar yalnızca gerçek Venice uç noktasına gider (web köprüsü de kendi çevirisini yapar). */
function isVeniceApi(id: ProviderId): boolean {
  return providerInfo(id).venice
}

/**
 * Anthropic'in OpenAI uyum katmanının katı kuralları (docs.anthropic.com/en/api/openai-sdk): `temperature` ile `top_p`
 * birlikte 400; Claude 4.7+ 1'in altındaki `temperature` ve 0.99 dışı `top_p` için 400; `n` yalnızca 1; `response_format`
 * yalnızca `json_schema` + `strict`. Araç şemalarında kök her zaman `type: "object"` olmalı (yoksa
 * `tools.N.function.parameters.type: Field required`).
 */
export function normalizeAnthropic(body: Record<string, unknown>): void {
  const temperature = typeof body.temperature === 'number' ? body.temperature : undefined
  delete body.top_p
  // Yeni modeller 1'in altını reddediyor; 1 zaten varsayılan olduğu için göndermemek en güvenlisi.
  if (temperature !== undefined && temperature < 1) delete body.temperature
  if (typeof body.n === 'number' && body.n !== 1) delete body.n
  const format = body.response_format as { type?: unknown } | undefined
  if (format && format.type !== 'json_schema') delete body.response_format
  // `adaptive` uyum katmanında 400 veriyor: yalnızca `enabled` gönderilir.
  const thinking = body.thinking as { type?: unknown; budget_tokens?: unknown } | undefined
  if (thinking && thinking.type === 'adaptive') body.thinking = { type: 'enabled' }
  // budget_tokens max_tokens'tan küçük olmalı; değilse güvenli bir bütçeye çekilir.
  const maxTokens = typeof body.max_tokens === 'number' ? body.max_tokens : typeof body.max_completion_tokens === 'number' ? body.max_completion_tokens : undefined
  const budget = (body.thinking as { budget_tokens?: unknown } | undefined)?.budget_tokens
  if (maxTokens !== undefined && typeof budget === 'number' && budget >= maxTokens) {
    body.thinking = { type: 'enabled', budget_tokens: Math.max(1_024, Math.floor(maxTokens / 2)) }
  }
  // Çoklu system/developer mesajı tek system bloğunda birleştirilir (katman hepsini zaten başa taşır).
  if (Array.isArray(body.messages)) body.messages = mergeSystemMessages(body.messages)
  // Zorunlu araç seçimi: en yeni modeller `required`/isimli seçimi reddediyor ve thinking ile çakışır.
  if (body.tool_choice !== undefined && body.tool_choice !== 'auto') body.tool_choice = 'auto'
  if (Array.isArray(body.tools)) {
    body.tools = body.tools.map((tool) => {
      if (!tool || typeof tool !== 'object') return tool
      const t = tool as { type?: unknown; function?: { parameters?: unknown } }
      if (t.type !== 'function' || !t.function) return tool
      return { ...t, function: { ...t.function, parameters: objectRootSchema(t.function.parameters) } }
    })
  }
}

/** system/developer mesajlarını tek `system` mesajında birleştirir; sıralama korunur. */
function mergeSystemMessages(messages: unknown[]): unknown[] {
  const texts: string[] = []
  const rest: unknown[] = []
  for (const m of messages) {
    if (!m || typeof m !== 'object') continue
    const msg = m as { role?: unknown; content?: unknown }
    if (msg.role !== 'system' && msg.role !== 'developer') {
      rest.push(m)
      continue
    }
    if (typeof msg.content === 'string') texts.push(msg.content)
    else if (Array.isArray(msg.content)) {
      for (const p of msg.content) {
        if (p && typeof p === 'object' && (p as { type?: unknown }).type === 'text') texts.push(String((p as { text?: unknown }).text ?? ''))
      }
    }
  }
  if (texts.length === 0) return messages
  return [{ role: 'system', content: texts.join('\n') }, ...rest]
}

/** Araç şemasının kökünü `type: "object"` yapar ve birleşim dallarının alanlarını köke taşır (tel üzeri güvence). */
function objectRootSchema(input: unknown): Record<string, unknown> {
  const base = input && typeof input === 'object' && !Array.isArray(input) ? (input as Record<string, unknown>) : {}
  const properties: Record<string, unknown> = { ...(base.properties as Record<string, unknown> | undefined) }
  // Anthropic araç şeması kökte anyOf/oneOf/allOf kabul etmez; dalların alanlarını köke birleştirip bu anahtarları düşür.
  for (const key of ['anyOf', 'oneOf', 'allOf'] as const) {
    const branches = base[key]
    if (!Array.isArray(branches)) continue
    for (const branch of branches) {
      if (!branch || typeof branch !== 'object') continue
      const props = (branch as Record<string, unknown>).properties
      if (props && typeof props === 'object') Object.assign(properties, props as Record<string, unknown>)
    }
  }
  const { anyOf: _a, oneOf: _o, allOf: _l, ...rest } = base
  return { ...rest, type: 'object', properties }
}

/** Gövdeyi sağlayıcıya göre düzeltir. Venice'te yalnızca (varsa) "venice:" öneki atılır. */
export function normalizeBody(id: ProviderId, body: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = { ...body }
  // Dahili alan: yalnızca Ollama num_ctx'e çevirir. Tel üzerine ASLA çıkmamalı — Venice gibi katı doğrulayıcılar
  // tanımadığı üst düzey alanda 400 "Invalid request parameters" döner. Bu yüzden erken dönüşlerden ÖNCE silinir.
  const contextLimit = typeof out.__contextLimit === 'number' ? out.__contextLimit : null
  delete out.__contextLimit
  const full = typeof body.model === 'string' ? body.model : null
  if (full?.startsWith(`${id}:`)) out.model = full.slice(id.length + 1)
  if (CACHE_BREAKPOINT_PROVIDERS.has(id) && full && ANTHROPIC_MODEL.test(full) && Array.isArray(out.messages)) {
    out.messages = addCacheBreakpoints(out.messages)
  }
  // Araç çağırmayan sağlayıcı/modeller: tools alanı yok sayılırsa model şaşırmasın diye hiç gönderilmez.
  if (!providerInfo(id).venice && (providerInfo(id).noTools || (full && config.noToolModels.has(full)))) {
    delete out.tools
    delete out.tool_choice
    delete out.parallel_tool_calls
  }
  if (isVeniceApi(id)) return out
  // Köprü sağlayıcıları web alanlarını kendisi okur (venice_parameters → webEnabled vb.); düşürülmemeli.
  if (!providerInfo(id).bridge) {
    delete out.venice_parameters
    delete out.prompt_cache_key
  }
  if (id === 'anthropic') normalizeAnthropic(out)
  else if (id === 'openai') normalizeOpenAI(out)
  else if (id === 'groq') normalizeGroq(out)
  // Gemini modeli farklı sağlayıcılardan da seçilebilir: kimliğe göre uygulanır (system rolü desteklenmez).
  if (full !== null && /gemini/i.test(full)) normalizeGemini(out)
  if (full && config.noToolModels.has(full)) {
    delete out.tools
    delete out.tool_choice
    delete out.parallel_tool_calls
  }
  // ArliAI'de tool_choice varsayılanı "none": gönderilmezse model araçları hiç çağırmaz.
  if (id === 'arliai' && out.tools !== undefined && out.tool_choice === undefined) out.tool_choice = 'auto'
  if (id === 'ollama') {
    if (out.max_completion_tokens !== undefined && out.max_tokens === undefined) {
      out.max_tokens = out.max_completion_tokens
      delete out.max_completion_tokens
    }
    // Yerel sunucunun bağlam penceresi: verilmezse Ollama varsayılanı (çoğu modelde 4k) kullanılır ve istek kesilir.
    if (out.num_ctx === undefined && contextLimit) out.num_ctx = contextLimit
  }
  return out
}

/** OpenAI reasoning ailesi (`o3`, `o4`, `gpt-5` akıl yürütme): temperature desteklenmez, `max_completion_tokens` istenir. */
function isOpenAIReasoning(model: string | null): boolean {
  return model !== null && /(^|\/)(o[1-9]|gpt-5)/i.test(model)
}

function normalizeOpenAI(body: Record<string, unknown>): void {
  const model = typeof body.model === 'string' ? body.model : null
  if (model && isOpenAIReasoning(model)) {
    delete body.temperature
    delete body.top_p
    delete body.presence_penalty
    delete body.frequency_penalty
    if (body.max_tokens !== undefined && body.max_completion_tokens === undefined) {
      body.max_completion_tokens = body.max_tokens
      delete body.max_tokens
    }
  }
  if (typeof body.n === 'number' && body.n !== 1) delete body.n
}

/** Groq: `n>1` ve `logprobs` yalnızca bazı modellerde; ikisi de düşürülür. */
function normalizeGroq(body: Record<string, unknown>): void {
  if (typeof body.n === 'number' && body.n !== 1) delete body.n
  delete body.logprobs
  delete body.top_logprobs
}

/** Gemini OpenAI-uyum katmanı: `system` rolü desteklenmez, ilk mesaja metin olarak katlanır. */
function normalizeGemini(body: Record<string, unknown>): void {
  delete body.logprobs
  delete body.top_logprobs
  if (!Array.isArray(body.messages)) return
  const systems: string[] = []
  const rest: unknown[] = []
  for (const m of body.messages) {
    const msg = m as { role?: unknown; content?: unknown }
    if (msg.role === 'system' || msg.role === 'developer') {
      if (typeof msg.content === 'string') systems.push(msg.content)
      else if (Array.isArray(msg.content)) {
        for (const p of msg.content) {
          if (p && typeof p === 'object' && (p as { type?: unknown }).type === 'text') systems.push(String((p as { text?: unknown }).text ?? ''))
        }
      }
      continue
    }
    rest.push(m)
  }
  if (systems.length === 0) return
  const head = rest[0] as { role?: unknown; content?: unknown } | undefined
  if (head?.role === 'user') {
    const text = typeof head.content === 'string' ? head.content : ''
    rest[0] = { ...head, content: `${systems.join('\n')}\n\n${text}` }
  } else {
    rest.unshift({ role: 'user', content: systems.join('\n') })
  }
  body.messages = rest
}

function rewriteBody(id: ProviderId, text: string): string {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return text // JSON olmayan gövde olduğu gibi gider.
  }
  return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? JSON.stringify(normalizeBody(id, parsed as Record<string, unknown>)) : text
}

async function withTimeout(id: ProviderId, signal: AbortSignal | null | undefined, run: (s: AbortSignal | undefined) => Promise<Response>): Promise<Response> {
  const ms = config.endpoints[id]?.timeoutMs
  if (!ms) return run(signal ?? undefined)
  const timer = new AbortController()
  const handle = setTimeout(() => timer.abort(new Error(`${providerInfo(id).label} ${ms} ms içinde yanıt vermedi`)), ms)
  try {
    return await run(signal ? AbortSignal.any([signal, timer.signal]) : timer.signal)
  } finally {
    clearTimeout(handle)
  }
}

interface SendInit extends RequestInit {
  headers: Headers
  method: string
}

const ANTHROPIC_VERSION = '2023-06-01'

async function sendWithKeys(id: ProviderId, url: string, init: SendInit, keys: readonly (string | null)[], doFetch: FetchLike): Promise<Response> {
  const wait = (blockedUntil.get(id) ?? 0) - Date.now()
  if (wait > 0) await sleep(Math.min(wait, MAX_RATE_WAIT_MS), init.signal)
  const release = await semaphoreFor(id)?.acquire(init.signal ?? undefined)
  try {
    for (let i = 0; ; i++) {
      const key = keys[i] ?? null
      if (key) {
        init.headers.set('authorization', `Bearer ${key}`)
        // Anthropic her istekte anthropic-version ister; x-api-key yalnızca kendi uç noktaları (ör. /models) için.
        if (id === 'anthropic') {
          init.headers.set('anthropic-version', ANTHROPIC_VERSION)
          if (init.method !== 'POST') init.headers.set('x-api-key', key)
        }
      } else {
        init.headers.delete('authorization')
      }
      const res = await withTimeout(id, init.signal, (signal) => doFetch(url, { ...init, signal: signal ?? null }))
      noteRateHeaders(id, res.headers)
      // 401/429/402: sonraki anahtar denenir. 402 (kota) tek anahtarlık bir sorun olabilir; yedek varsa şans verilir.
      if (key && i < keys.length - 1 && (res.status === 401 || res.status === 429 || res.status === 402)) {
        coolDown(key, res)
        // Atılan yanıtın gövdesi okunmaz; bağlantı serbest kalsın diye iptal edilir.
        await res.body?.cancel()
        continue
      }
      return release ? onBodyDone(res, release) : res
    }
  } catch (err) {
    release?.()
    throw err
  }
}

interface Flight {
  promise: Promise<Response>
  refs: number
  controller: AbortController
}
const flights = new Map<string, Flight>()

/**
 * İstek tekilleştirme (madde 55): aynı sağlayıcıya aynı anda giden birebir aynı istek tek çağrıya iner; her bekleyen
 * yanıtın kendi kopyasını (tee) okur. Alttaki istek ancak tüm bekleyenler vazgeçince iptal edilir.
 */
async function shared(key: string, signal: AbortSignal | null | undefined, run: (s: AbortSignal) => Promise<Response>): Promise<Response> {
  signal?.throwIfAborted()
  let flight = flights.get(key)
  if (!flight) {
    const controller = new AbortController()
    const created: Flight = { promise: run(controller.signal), refs: 0, controller }
    const drop = () => {
      if (flights.get(key) === created) flights.delete(key)
    }
    // Başarısız yanıt veya hata paylaşılmaya devam etmez: sonraki istek yeniden dener.
    created.promise.then((r) => (r.ok ? undefined : drop()), drop)
    flights.set(key, created)
    flight = created
  }
  const f = flight
  f.refs++
  let left = false
  const leave = () => {
    if (left) return
    left = true
    signal?.removeEventListener('abort', leave)
    if (--f.refs > 0) return
    if (flights.get(key) === f) flights.delete(key)
    f.controller.abort()
  }
  signal?.addEventListener('abort', leave, { once: true })
  try {
    const res = await f.promise
    signal?.throwIfAborted()
    return onBodyDone(res.clone(), leave)
  } catch (err) {
    leave()
    throw err
  }
}

const sha = (text: string) => createHash('sha256').update(text).digest('hex')

/**
 * Sağlayıcıya özgü fetch: host izni (yalnızca sağlayıcının kendi kökeni), anahtar ve yedek anahtar, özel başlıklar,
 * zaman aşımı, eşzamanlılık kuyruğu, hız penceresi ve tekilleştirme. inner verilirse gerçek istek onunla yapılır.
 */
export function providerFetch(id: ProviderId, inner?: FetchLike, override?: { keys: readonly string[] }): FetchLike {
  const provider = providerInfo(id)
  return async (input, init = {}) => {
    const base = effectiveBaseURL(id)
    let url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
    // Venice istemcisi sabit temel URL ile kurulur; kullanıcı uç nokta ayarı varsa önek değiştirilir.
    if (base !== provider.baseURL && url.startsWith(provider.baseURL)) url = base + url.slice(provider.baseURL.length)
    // Köprü sağlayıcıları (ör. Venice web hesabı) kendi taşımalarını kullanır: host izni ve anahtar
    // yerleşimi onlara aittir; buradan yalnızca gövde düzeltilir.
    if (provider.bridge) {
      const method = (init.method ?? 'GET').toUpperCase()
      const rewritten = typeof init.body === 'string' && method === 'POST' ? rewriteBody(id, init.body) : init.body
      return bridgeFetch(id)(url, { ...init, method, ...(rewritten !== undefined ? { body: rewritten } : {}) })
    }
    if (!sameOrigin(url, base)) {
      return refusal(403, `Sağlayıcı izolasyonu: ${hostOf(url)} adresine ${provider.label} istemcisiyle istek gönderilemez.`)
    }
    const headers = new Headers(init.headers)
    const keys: readonly (string | null)[] = provider.keyless ? [null] : (override?.keys ?? keysFor(id, bearerOf(headers)))
    if (keys.length === 0) return refusal(401, missingKeyMessage(provider))
    for (const [name, value] of Object.entries(config.endpoints[id]?.headers ?? {})) {
      if (!FORBIDDEN_HEADER.test(name)) headers.set(name, value)
    }
    const method = (init.method ?? 'GET').toUpperCase()
    const body = typeof init.body === 'string' && method === 'POST' ? rewriteBody(id, init.body) : init.body
    const doFetch = inner ?? config.fetchImpl
    const run = (signal: AbortSignal | null | undefined) =>
      sendWithKeys(id, url, { ...init, method, headers, ...(body !== undefined ? { body } : {}), signal: signal ?? null }, keys, doFetch)
    // Belirli bir anahtarın testi paylaşılmaz.
    if (override) return run(init.signal)
    return shared(sha(`${id} ${method} ${url} ${typeof body === 'string' ? body : ''}`), init.signal, run)
  }
}

/** Venice dışı sağlayıcının önbelleğe alınmış istemcisi. */
const clientInitHooks = new Set<(client: OpenAI) => void>()

/**
 * [g10 ↔ g9] Her sağlayıcı istemcisi oluşturulunca (var olanlar için hemen) çağrılır. g10 akış dayanıklılığını
 * (yeniden bağlanma, devre kesici) yalnızca oturum istemcisine, yani yönlendiricinin arkasındaki Venice istemcisine
 * kurabiliyordu; diğer sağlayıcıların akışları korumasız kalıyordu.
 */
export function onProviderClient(hook: (client: OpenAI) => void): () => void {
  clientInitHooks.add(hook)
  for (const client of clients.values()) hook(client)
  return () => clientInitHooks.delete(hook)
}

export function providerClient(id: ProviderId): OpenAI {
  let client = clients.get(id)
  if (!client) {
    client = new OpenAI({ apiKey: MANAGED_KEY, baseURL: effectiveBaseURL(id), maxRetries: 4, timeout: 5 * 60_000, fetch: providerFetch(id) })
    clients.set(id, client)
    for (const hook of clientInitHooks) hook(client)
  }
  return client
}

type StreamArgs = Parameters<OpenAI['chat']['completions']['stream']>
type CreateArgs = Parameters<OpenAI['chat']['completions']['create']>
export type ChatStream = ReturnType<OpenAI['chat']['completions']['stream']>
export type ChatStreamBody = StreamArgs[0]
export type ChatStreamHook = (body: ChatStreamBody, stream: ChatStream, fromCache: boolean) => void

const streamHooks = new Set<ChatStreamHook>()

/** Her akışlı chat çağrısında (önbellekten gelenler dahil) çağrılır: canlı maliyet sayacı ve tavan buradan izler. */
export function onChatStream(hook: ChatStreamHook): () => void {
  streamHooks.add(hook)
  return () => streamHooks.delete(hook)
}

/** Nesnenin yöntemleri hedefe bağlanır: SDK'nın özel (#) alanları Proxy üzerinden okunamaz. */
function forward<T extends object>(target: T, override: (prop: string | symbol) => unknown): T {
  return new Proxy(target, {
    get(t, prop) {
      const replaced = override(prop)
      if (replaced !== undefined) return replaced
      const value: unknown = Reflect.get(t, prop, t)
      return typeof value === 'function' ? value.bind(t) : value
    },
  })
}

/**
 * Venice istemcisini sarar: chat.completions çağrıları gövdedeki modelin sağlayıcısına yönlenir; diğer her şey
 * (model listesi, web araması) Venice'te kalır. Yanıt önbelleği açıksa birebir aynı istek yerelden yanıtlanır.
 */
export function routeByModel(base: OpenAI): OpenAI {
  const pick = (model: string): OpenAI => (isVeniceModel(model) ? base : providerClient(providerOf(model)))
  const stream = (...args: StreamArgs): ChatStream => {
    const [body] = args
    const hit = cachedCompletion(body)
    const s = hit ? replayResponse(hit).chat.completions.stream(...args) : pick(body.model).chat.completions.stream(...args)
    if (!hit) rememberCompletion(body, s)
    for (const hook of streamHooks) hook(body, s, hit !== null)
    return s
  }
  const create = (...args: CreateArgs) => pick(args[0].model).chat.completions.create(...args)
  const completions = forward(base.chat.completions, (p) => (p === 'stream' ? stream : p === 'create' ? create : undefined))
  const chat = forward(base.chat, (p) => (p === 'completions' ? completions : undefined))
  return forward(base, (p) => (p === 'chat' ? chat : undefined))
}

export interface ProbeResult {
  provider: ProviderId
  ok: boolean
  status: number | null
  latencyMs: number | null
  message: string
}

function probeMessage(status: number): string {
  if (status === 401 || status === 403) return `Kimlik doğrulanamadı (HTTP ${status})`
  if (status === 429) return 'Hız sınırına takıldı (HTTP 429)'
  if (status >= 500) return `Sağlayıcı hatası (HTTP ${status})`
  return `Beklenmeyen yanıt (HTTP ${status})`
}

/** Sağlık probu (madde 50): model listesi uç noktasına tek istek; gecikme ve erişilebilirlik. */
export async function probeProvider(id: ProviderId, opts: { keys?: readonly string[]; timeoutMs?: number } = {}): Promise<ProbeResult> {
  const provider = providerInfo(id)
  const started = performance.now()
  try {
    const doFetch = providerFetch(id, undefined, opts.keys ? { keys: opts.keys } : undefined)
    const res = await doFetch(`${effectiveBaseURL(id)}${provider.modelsPath}`, { method: 'GET', signal: AbortSignal.timeout(opts.timeoutMs ?? 8_000) })
    const latencyMs = Math.round(performance.now() - started)
    await res.body?.cancel()
    return { provider: id, ok: res.ok, status: res.status, latencyMs, message: res.ok ? 'Erişilebilir' : probeMessage(res.status) }
  } catch (err) {
    return { provider: id, ok: false, status: null, latencyMs: null, message: `Erişilemiyor: ${err instanceof Error ? err.message : String(err)}` }
  }
}
