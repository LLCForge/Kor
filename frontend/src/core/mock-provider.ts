// [g15] Sahte sağlayıcı (#181). Taklit yerine gerçek OpenAI SDK'si kullanılır, yalnızca ağ katmanı (fetch) değiştirilir:
// chat.completions.stream (content.delta / chunk olayları, finalChatCompletion), create, get/post bu projedeki gibi birebir çalışır.
import OpenAI from 'openai'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import { z } from 'zod'
import { PROVIDERS } from './providers.ts'
import { VENICE_BASE_URL } from './settings.ts'

export const CHAT_PATH = '/chat/completions'

export const mockReplySchema = z.object({
  content: z.string().max(2_000_000).optional(),
  /** Venice akıl yürütme modellerinin delta.reasoning_content alanı */
  reasoning: z.string().max(2_000_000).optional(),
  toolCalls: z
    .array(
      z.object({
        /** Verilmezse sahte sağlayıcı sıralı ve belirlenebilir kimlik üretir */
        id: z.string().min(1).max(200).optional(),
        name: z.string().min(1).max(200),
        arguments: z.union([z.string().max(2_000_000), z.record(z.string(), z.unknown())]),
      }),
    )
    .max(128)
    .optional(),
  finishReason: z.string().max(50).optional(),
  usage: z.record(z.string(), z.unknown()).optional(),
})
export type MockReply = z.infer<typeof mockReplySchema>

export interface MockRequest {
  method: string
  /** Sağlayıcı baseURL'ine göre yol, sorgu dizesi hariç (ör. '/chat/completions', '/models') */
  path: string
  /** İsteğin JSON gövdesi; gövdesiz isteklerde boş nesne */
  body: Record<string, unknown>
}

export type MockResult = { reply: MockReply } | { json: unknown; status?: number }
export type MockRoute = (request: MockRequest) => MockResult | Promise<MockResult>
/** Yalnızca sohbet istekleri için senaryo; async olabilir (gecikme, eşzamanlılık ölçümü). */
export type ChatScript = (messages: ChatCompletionMessageParam[], request: MockRequest) => MockReply | Promise<MockReply>

const DEFAULT_USAGE = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 }

export function errorResult(status: number, message: string): MockResult {
  return { status, json: { error: { message } } }
}

/** Sohbet dışındaki isteklerin varsayılanı: model listesi boş (bağlam sınırı bilinmez, sıkıştırma tetiklenmez), gerisi 404. */
export function fallbackResult(request: MockRequest): MockResult {
  if (request.method === 'GET' && request.path === '/models') return { json: { object: 'list', data: [] } }
  return errorResult(404, `Sahte sağlayıcıda tanımsız uç nokta: ${request.method} ${request.path}`)
}

export function chatRoute(script: ChatScript): MockRoute {
  return async (request) => {
    if (request.method !== 'POST' || request.path !== CHAT_PATH) return fallbackResult(request)
    const messages = Array.isArray(request.body.messages) ? (request.body.messages as ChatCompletionMessageParam[]) : []
    return { reply: await script(messages, request) }
  }
}

const urlOf = (input: string | URL | Request): string => (input instanceof Request ? input.url : String(input))

function jsonBody(init: RequestInit | undefined): Record<string, unknown> {
  if (typeof init?.body !== 'string' || !init.body) return {}
  try {
    const parsed: unknown = JSON.parse(init.body)
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : {}
  } catch {
    // JSON olmayan gövde (ör. dosya yükleme) kaydedilmez/eşleştirilmez; istek yine de geçer.
    return {}
  }
}

/** URL bilinen bir sağlayıcıya aitse istek bilgisini, değilse null döner. */
export function providerRequest(input: string | URL | Request, init: RequestInit | undefined): MockRequest | null {
  const url = new URL(urlOf(input))
  const provider = PROVIDERS.find((p) => `${url.origin}${url.pathname}`.startsWith(`${p.baseURL}/`))
  if (!provider) return null
  return {
    method: (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase(),
    path: url.pathname.slice(new URL(provider.baseURL).pathname.length),
    body: jsonBody(init),
  }
}

interface WireToolCall {
  index: number
  id: string
  type: 'function'
  function: { name: string; arguments: string }
}

function completionParts(reply: MockReply, nextId: () => string) {
  const toolCalls: WireToolCall[] = (reply.toolCalls ?? []).map((c, index) => ({
    index,
    id: c.id ?? nextId(),
    type: 'function',
    function: { name: c.name, arguments: typeof c.arguments === 'string' ? c.arguments : JSON.stringify(c.arguments) },
  }))
  const finishReason = reply.finishReason ?? (toolCalls.length > 0 ? 'tool_calls' : 'stop')
  return { toolCalls, finishReason, usage: reply.usage ?? DEFAULT_USAGE }
}

/** Yanıtı SDK'nin ayrıştırdığı SSE akışına çevirir (rol, düşünme, içerik, araç çağrıları, bitiş, kullanım). */
function sseBody(reply: MockReply, model: string, id: string, nextId: () => string): string {
  const { toolCalls, finishReason, usage } = completionParts(reply, nextId)
  const head = { id, object: 'chat.completion.chunk', created: 0, model }
  const chunk = (delta: Record<string, unknown>, finish: string | null = null) => ({
    ...head,
    choices: [{ index: 0, delta, finish_reason: finish, logprobs: null }],
  })
  const chunks: unknown[] = [chunk({ role: 'assistant' })]
  if (reply.reasoning) chunks.push(chunk({ reasoning_content: reply.reasoning }))
  if (reply.content) chunks.push(chunk({ content: reply.content }))
  for (const call of toolCalls) chunks.push(chunk({ tool_calls: [call] }))
  chunks.push(chunk({}, finishReason), { ...head, choices: [], usage })
  return `${chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join('')}data: [DONE]\n\n`
}

function jsonCompletion(reply: MockReply, model: string, id: string, nextId: () => string): unknown {
  const { toolCalls, finishReason, usage } = completionParts(reply, nextId)
  const message = {
    role: 'assistant',
    content: reply.content ?? null,
    refusal: null,
    ...(reply.reasoning ? { reasoning_content: reply.reasoning } : {}),
    ...(toolCalls.length > 0 ? { tool_calls: toolCalls.map(({ index: _index, ...call }) => call) } : {}),
  }
  return { id, object: 'chat.completion', created: 0, model, choices: [{ index: 0, message, finish_reason: finishReason, logprobs: null }], usage }
}

/**
 * Sağlayıcı isteklerini route'a yönlendiren fetch. Sağlayıcı dışı adresler reddedilir: sahte sağlayıcı
 * ve tekrar oynatma hiçbir koşulda ağa çıkmaz. Kimlikler sıralıdır (belirlenebilir).
 */
export function mockFetch(route: MockRoute): typeof fetch {
  let calls = 0
  let completions = 0
  const nextId = () => `call_mock_${++calls}`
  return async (input, init) => {
    const request = providerRequest(input, init)
    if (!request) throw new TypeError(`Çevrimdışı mod: ağ erişimi kapalı (${new URL(urlOf(input)).origin})`)
    const result = await route(request)
    // Gerçek fetch gibi: istek beklerken iptal edildiyse AbortError ile reddedilir.
    init?.signal?.throwIfAborted()
    if (!('reply' in result)) {
      return new Response(JSON.stringify(result.json ?? null), { status: result.status ?? 200, headers: { 'content-type': 'application/json' } })
    }
    const model = typeof request.body.model === 'string' ? request.body.model : 'mock'
    const id = `chatcmpl-mock-${++completions}`
    if (request.body.stream === true) {
      return new Response(sseBody(result.reply, model, id, nextId), { headers: { 'content-type': 'text/event-stream' } })
    }
    return new Response(JSON.stringify(jsonCompletion(result.reply, model, id, nextId)), { headers: { 'content-type': 'application/json' } })
  }
}

/** Ağ kullanmayan gerçek SDK istemcisi. Yeniden deneme kapalı: hata senaryoları test edilebilsin. */
export function createMockClient(route: MockRoute): OpenAI {
  return new OpenAI({ apiKey: 'mock-key', baseURL: VENICE_BASE_URL, maxRetries: 0, fetch: mockFetch(route) })
}
