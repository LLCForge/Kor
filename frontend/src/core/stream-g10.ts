// [g10] Model akışının dayanıklılık katmanı. SDK (openai) bağlantı kurulurken 408/409/429/5xx ve bağlantı hatalarında
// Retry-After'a uyan, %25 jitter'lı üstel geri çekilmeyle zaten yeniden dener (maxRetries: 4); ama akış başladıktan
// sonraki kopmaları tekrar denemez ("streaming body cannot be retried"). Bu katman eksik kalanları ekler:
// akış ortasında kopmada kaldığı yerden devam, sağlayıcı devre kesicisi, bağlam taşmasında sıkıştırıp yeniden deneme,
// ilk token gecikmesinde yedek modele düşüş, bozuk araç argümanı JSON onarımı ve model reddi bildirimi.
import OpenAI from 'openai'
import { ChatCompletionStream } from 'openai/lib/ChatCompletionStream'
import type { ChatCompletionChunk } from 'openai/resources/chat/completions'
import type { CompletionUsage } from 'openai/resources/completions'
import { originalMessagesOf } from './tool-prune.ts'
import {
  backoffDelay,
  CircuitBreaker,
  isContextOverflowError,
  isTransientError,
  isValidJsonObject,
  repairJsonArguments,
  sleep,
  type G10Event,
  type RetryBudget,
} from './resilience-g10.ts'

type StreamFn = OpenAI['chat']['completions']['stream']
type StreamParams = Parameters<StreamFn>[0]
type StreamOptions = Parameters<StreamFn>[1]

/** Ajan oturumuna ait çağrılar için kancalar; başka çağrılar (özetleme vb.) yalnızca devre kesiciden geçer. */
export interface SessionStreamHooks {
  emit(event: G10Event): void
  notice(text: string): void
  budget: RetryBudget
  firstTokenTimeoutMs: number
  fallbackModel: string | null
  compact(signal: AbortSignal | undefined): Promise<boolean>
  invalidArgs(toolCallId: string, reason: string): void
}

export const MAX_RECONNECTS = 2
export const CONTINUE_PROMPT =
  '<system-reminder>Bağlantı koptuğu için önceki yanıtın yarıda kaldı. Yukarıdaki kısmi yanıtı tekrarlamadan, tam kaldığın yerden devam et.</system-reminder>'
const PROVIDER_BREAKER = { failureThreshold: 5, cooldownMs: 30_000 }

/**
 * SDK zaman aşımı başlıklar gelince biter; akışın geri kalanını bunlar korur. Uzun yanıt sınırlanmaz, sessizlik sınırlanır.
 * idleMs: parçalar arası; firstChunkMs: deneme başına ilk parçaya kadar (SDK yeniden denemeleri dahil);
 * noProgressMs: hiç parça gelmeden geçebilecek toplam süre (yeniden bağlanmalar dahil). Testler kısaltır.
 */
export const streamTimeouts = { idleMs: 120_000, firstChunkMs: 5 * 60_000, noProgressMs: 10 * 60_000 }

const providerBreakers = new Map<string, CircuitBreaker>()

/** Sağlayıcı (temel URL) başına devre kesici; art arda başarısız çağrılarda istekler bir süre hemen reddedilir. */
export function providerBreaker(key: string): CircuitBreaker {
  let b = providerBreakers.get(key)
  if (!b) {
    b = new CircuitBreaker(key, PROVIDER_BREAKER)
    providerBreakers.set(key, b)
  }
  return b
}

function providerKey(client: OpenAI): string {
  try {
    return new URL(client.baseURL).host
  } catch {
    return client.baseURL
  }
}

interface ToolBuffer {
  id: string
  name: string
  args: string
}

interface Meta {
  id: string
  created: number
  model: string
}

function addUsage(prev: CompletionUsage | null, next: CompletionUsage): CompletionUsage {
  if (!prev) return next
  return {
    ...next,
    prompt_tokens: prev.prompt_tokens + next.prompt_tokens,
    completion_tokens: prev.completion_tokens + next.completion_tokens,
    total_tokens: prev.total_tokens + next.total_tokens,
  }
}

function chunkOf(meta: Meta, choice: ChatCompletionChunk.Choice | null, usage?: CompletionUsage): ChatCompletionChunk {
  return {
    id: meta.id,
    object: 'chat.completion.chunk',
    created: meta.created,
    model: meta.model,
    choices: choice ? [choice] : [],
    ...(usage ? { usage } : {}),
  }
}

const isMidStreamApiError = (err: unknown) => err instanceof OpenAI.APIError && err.status === undefined

class EmptyStreamError extends Error {
  constructor() {
    super('Model yanıt göndermeden akışı kapattı.')
    this.name = 'EmptyStreamError'
  }
}

class StreamStallError extends Error {
  constructor(ms: number) {
    super(`Model ${Math.round(ms / 1000)} sn boyunca veri göndermedi; bağlantı kesildi.`)
    this.name = 'StreamStallError'
  }
}

/** Sinyali dinlemeyen bağdaştırıcıda da takılmasın: her parça beklemesi iptalle yarışır. */
async function* untilAborted<T>(source: AsyncIterable<T>, signal: AbortSignal): AsyncGenerator<T> {
  const it = source[Symbol.asyncIterator]()
  const aborted = new Promise<never>((_, reject) => signal.addEventListener('abort', () => reject(signal.reason as Error), { once: true }))
  aborted.catch(() => undefined)
  for (;;) {
    signal.throwIfAborted()
    const next = await Promise.race([it.next(), aborted])
    if (next.done) return
    yield next.value
  }
}

interface PumpInput {
  original: StreamFn
  params: StreamParams
  options: StreamOptions
  hooks: SessionStreamHooks | null
  breaker: CircuitBreaker
  cancel: AbortSignal
}

/** İç akış(lar)ı tüketip tek, kesintisiz bir NDJSON parça akışı üretir. */
async function pump(controller: ReadableStreamDefaultController<Uint8Array>, input: PumpInput): Promise<void> {
  const { original, params, options, hooks, breaker } = input
  const encoder = new TextEncoder()
  const enqueue = (chunk: ChatCompletionChunk) => controller.enqueue(encoder.encode(`${JSON.stringify(chunk)}\n`))
  const userSignal = options?.signal ?? undefined
  let body: StreamParams = params
  let partial = ''
  let reconnects = 0
  let compacted = false
  let degraded = false
  let meta: Meta | null = null
  let usage: CompletionUsage | null = null
  let lastProgress = Date.now()

  for (;;) {
    breaker.assertPass()
    const local = new AbortController()
    const signal = AbortSignal.any([input.cancel, local.signal, ...(userSignal ? [userSignal] : [])])
    const fallback = hooks?.fallbackModel ?? null
    const firstTokenMs = !degraded && hooks && fallback && fallback !== body.model ? hooks.firstTokenTimeoutMs : 0
    let firstTokenTimedOut = false
    const timer =
      firstTokenMs > 0
        ? setTimeout(() => {
            firstTokenTimedOut = true
            local.abort()
          }, firstTokenMs)
        : undefined
    let stalledAfter = 0
    let idle: NodeJS.Timeout | undefined
    const armIdle = (ms: number) => {
      clearTimeout(idle)
      idle = setTimeout(() => {
        stalledAfter = ms
        local.abort()
      }, ms)
    }
    armIdle(streamTimeouts.firstChunkMs)
    // Sağlayıcı aynı yanıtta birden çok kümülatif usage parçası gönderebilir: deneme içinde son değer geçerlidir.
    let attemptUsage: CompletionUsage | null = null
    let received = false
    const tools = new Map<number, ToolBuffer>()
    let finish: ChatCompletionChunk.Choice['finish_reason'] = null
    let refusal = ''
    try {
      const inner = original(body, { ...options, signal })
      // Hatalar yineleme üzerinden ele alınır; dinleyicisiz 'error' olayı işlenmemiş ret üretmesin.
      inner.on('error', () => undefined)
      for await (const chunk of untilAborted(inner, signal)) {
        armIdle(streamTimeouts.idleMs)
        lastProgress = Date.now()
        if (!received) {
          received = true
          clearTimeout(timer)
        }
        meta ??= { id: chunk.id, created: chunk.created, model: chunk.model }
        if (chunk.usage) attemptUsage = chunk.usage
        const choice = chunk.choices[0]
        if (!choice) continue
        const { tool_calls: toolDeltas, ...delta } = choice.delta
        // Araç çağrıları parça parça değil, bitince (gerekirse onarılmış) tek parça olarak iletilir.
        for (const tc of toolDeltas ?? []) {
          const buf = tools.get(tc.index) ?? { id: '', name: '', args: '' }
          if (tc.id) buf.id = tc.id
          if (tc.function?.name) buf.name += tc.function.name
          if (tc.function?.arguments) buf.args += tc.function.arguments
          tools.set(tc.index, buf)
        }
        if (delta.content) partial += delta.content
        if (delta.refusal) refusal += delta.refusal
        if (choice.finish_reason) finish = choice.finish_reason
        if (Object.keys(delta).length > 0) enqueue(chunkOf(meta, { ...choice, delta, finish_reason: null }))
      }
      clearTimeout(timer)
      clearTimeout(idle)
      // İptal SDK yineleyicisini sessizce bitirebilir; yarım yanıt başarı sayılmasın.
      signal.throwIfAborted()
      // Hiç parça gelmeden kapanan akış SDK'da hatadır; sessizce boş yanıta dönüşmesin.
      if (!received) throw new EmptyStreamError()
      breaker.success()
      if (attemptUsage) usage = addUsage(usage, attemptUsage)
      const head = meta ?? { id: 'g10', created: Math.floor(Date.now() / 1000), model: body.model }
      for (const [index, t] of [...tools.entries()].sort(([a], [b]) => a - b)) {
        let args = t.args
        // length: g5 yarım çağrıyı zaten atar; kesik JSON asla tamamlanmaz.
        if (!isValidJsonObject(args) && finish !== 'length') {
          const repaired = repairJsonArguments(args)
          if (repaired.ok) {
            args = repaired.text
            hooks?.emit({ kind: 'jsonRepaired', tool: t.name, fixes: repaired.fixes })
          } else {
            hooks?.invalidArgs(t.id, repaired.reason)
          }
        }
        enqueue(chunkOf(head, { index: 0, delta: { tool_calls: [{ index, id: t.id, type: 'function', function: { name: t.name, arguments: args } }] }, finish_reason: null }))
      }
      enqueue(chunkOf(head, { index: 0, delta: {}, finish_reason: finish ?? (tools.size > 0 ? 'tool_calls' : 'stop') }))
      if (usage) enqueue(chunkOf(head, null, usage))
      if (hooks && (finish === 'content_filter' || refusal.trim())) {
        const reason = refusal.trim() || 'içerik filtresi (finish_reason: content_filter)'
        hooks.emit({ kind: 'refusal', reason })
        // Bildirim satırı arayüzde ilk satırla sınırlı; alternatifler g10 bandında ve sonraki satırlarda.
        hooks.notice(
          `Model isteği reddetti: ${reason.slice(0, 110)}\n` +
            'Seçenekler: isteği bağlamıyla daha açık yeniden ifade et, görevi küçük adımlara böl veya model seçiciden başka (ör. sansürsüz) bir model dene.',
        )
      }
      controller.close()
      return
    } catch (caught) {
      clearTimeout(timer)
      clearTimeout(idle)
      if (attemptUsage) usage = addUsage(usage, attemptUsage)
      const err = stalledAfter > 0 ? new StreamStallError(stalledAfter) : caught
      if (userSignal?.aborted || input.cancel.aborted) {
        breaker.release()
        throw err
      }
      if (firstTokenTimedOut && !received && hooks && fallback) {
        breaker.release()
        degraded = true
        hooks.emit({ kind: 'degraded', from: body.model, to: fallback, seconds: Math.round(firstTokenMs / 1000) })
        body = { ...body, model: fallback }
        continue
      }
      // Budanmış/metin protokolü kopyasında bu yol eski diziyi yeniden gönderirdi: runTurn gövdeyi yeniden kurar.
      if (hooks && !compacted && isContextOverflowError(err) && originalMessagesOf(params.messages) === params.messages) {
        breaker.success()
        compacted = true
        const ok = await hooks.compact(userSignal)
        hooks.emit({ kind: 'contextTrimmed', ok })
        if (!ok) throw err
        // Sıkıştırma oturum dizisini yerinde kısaltır; params.messages aynı dizidir.
        body = { ...body, messages: params.messages }
        continue
      }
      const transient = isTransientError(err) || err instanceof EmptyStreamError || err instanceof StreamStallError || (received && isMidStreamApiError(err))
      if (!transient) {
        // Sağlayıcı yanıt verdi (ör. 400): sağlıklıdır. Durum kodsuz yerel hata devreyi etkilemez.
        if (err instanceof OpenAI.APIError && err.status !== undefined) breaker.success()
        else breaker.release()
        throw err
      }
      if (breaker.failure()) hooks?.emit({ kind: 'circuitOpen', target: breaker.name, retryInSec: Math.ceil(breaker.retryInMs() / 1000) })
      if (!hooks) throw err
      reconnects++
      const delay = 250 + backoffDelay(reconnects, 250, 4_000)
      if (reconnects > MAX_RECONNECTS || Date.now() - lastProgress >= streamTimeouts.noProgressMs || !hooks.budget.take(delay)) throw err
      hooks.emit({ kind: 'reconnect', attempt: reconnects, continued: partial.length > 0 })
      await sleep(delay, userSignal)
      // Kısmi metin arayüze zaten aktı: aynı yanıtın devamı istenir, baştan değil.
      if (partial) {
        body = {
          ...body,
          messages: [...params.messages, { role: 'assistant', content: partial }, { role: 'user', content: CONTINUE_PROMPT }],
        }
      }
    }
  }
}

function resilientStream(input: Omit<PumpInput, 'cancel'>): ChatCompletionStream {
  const cancel = new AbortController()
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      pump(controller, { ...input, cancel: cancel.signal }).catch((err: unknown) => {
        try {
          controller.error(err)
        } catch {
          // Akış zaten kapandı (tüketici iptal etti).
        }
      })
    },
    cancel() {
      cancel.abort()
    },
  })
  return ChatCompletionStream.fromReadableStream(readable)
}

const installed = new WeakSet<object>()

/**
 * İstemcinin chat.completions.stream yöntemini yerinde sarar (istemci başına bir kez).
 * session.client'ı değiştirmemek bilinçli: g5 istemciyi her değişimde yeniden sarar, katmanlar çoğalırdı.
 * Sahte (test) istemcileri OpenAI örneği olmadığından atlanır.
 */
export function installStreamResilience(client: OpenAI, lookup: (messages: unknown) => SessionStreamHooks | null): void {
  if (!(client instanceof OpenAI)) return
  const completions = client.chat.completions
  const current: unknown = Object.getOwnPropertyDescriptor(completions, 'stream')?.value
  if (typeof current === 'function' && installed.has(current)) return
  const proto: unknown = Object.getPrototypeOf(completions)
  const base = (proto as { stream?: unknown } | null)?.stream
  if (typeof base !== 'function') return
  const original = base.bind(completions) as StreamFn
  const breaker = providerBreaker(providerKey(client))
  const wrapped = ((params: StreamParams, options?: StreamOptions) =>
    resilientStream({ original, params, options, hooks: lookup(params.messages), breaker })) as StreamFn
  installed.add(wrapped)
  Object.defineProperty(completions, 'stream', { value: wrapped, configurable: true, writable: true })
}
