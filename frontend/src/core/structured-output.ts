// Yapılandırılmış çıktı: model yanıtını bir zod şemasına ZORLAR. Model destekliyorsa response_format json_schema
// gönderilir (Venice capabilities.supportsResponseSchema); desteklemiyorsa veya sağlayıcı reddederse şema isteme yazılır.
// Her yanıt: düşünce bloğu atılır → JSON ayrıştırılır → gerekirse veriyi değiştirmeyen onarım (repairJsonArguments) →
// şema doğrulaması. Geçersizse hata modele geri verilip yeniden denenir.
import type OpenAI from 'openai'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import { z } from 'zod'
import { getModelCapabilities } from './model-capabilities.ts'
import { repairJsonArguments } from './resilience-g10.ts'

export type StructuredParse<T> = { ok: true; value: T; repaired: boolean } | { ok: false; error: string }

/** Ham model çıktısını şemaya göre ayrıştırır; hiçbir durumda fırlatmaz. */
export function parseStructured<T>(raw: string, schema: z.ZodType<T>): StructuredParse<T> {
  const text = raw.replace(/<think>[\s\S]*?(<\/think>|$)/gi, ' ').trim()
  let value: unknown
  let repaired = false
  try {
    value = JSON.parse(text)
  } catch {
    const fixed = repairJsonArguments(text)
    if (!fixed.ok) return { ok: false, error: fixed.reason }
    value = JSON.parse(fixed.text)
    repaired = true
  }
  const result = schema.safeParse(value)
  return result.success ? { ok: true, value: result.data, repaired } : { ok: false, error: z.prettifyError(result.error) }
}

export class StructuredOutputError extends Error {
  override name = 'StructuredOutputError'
}

export interface StructuredRequest<T> {
  client: OpenAI
  model: string
  /** json_schema adı: ^[A-Za-z0-9_-]{1,64}$ */
  name: string
  instructions: string
  input: string
  /** Kök bir nesne olmalı (json_schema ve onarım nesne bekler). */
  schema: z.ZodType<T>
  maxAttempts?: number
  maxTokens?: number
  temperature?: number
  signal?: AbortSignal
}

export interface StructuredResult<T> {
  value: T
  attempts: number
  usage: { promptTokens: number; completionTokens: number }
}

const statusOf = (err: unknown): unknown => (typeof err === 'object' && err !== null ? (err as { status?: unknown }).status : undefined)

export async function completeStructured<T>(req: StructuredRequest<T>): Promise<StructuredResult<T>> {
  const jsonSchema = z.toJSONSchema(req.schema) as Record<string, unknown>
  let native = getModelCapabilities(req.model).responseSchema === true
  const messages: ChatCompletionMessageParam[] = [
    {
      role: 'system',
      content: `${req.instructions}\n\nOutput ONLY one JSON object matching this JSON Schema. No prose, no code fences.\n${JSON.stringify(jsonSchema)}`,
    },
    { role: 'user', content: req.input },
  ]
  const usage = { promptTokens: 0, completionTokens: 0 }
  const maxAttempts = req.maxAttempts ?? 3
  let lastError = ''
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    const body = {
      model: req.model,
      messages,
      stream: false as const,
      ...(req.temperature !== undefined ? { temperature: req.temperature } : {}),
      ...(req.maxTokens !== undefined ? { max_completion_tokens: req.maxTokens } : {}),
      // Venice'e özgü alan diğer sağlayıcılara taşıma katmanında gönderilmez (provider-pool-g9.ts).
      venice_parameters: { include_venice_system_prompt: false, disable_thinking: true, strip_thinking_response: true },
      ...(native ? { response_format: { type: 'json_schema' as const, json_schema: { name: req.name, schema: jsonSchema, strict: false } } } : {}),
    }
    let content: string
    try {
      const completion = await req.client.chat.completions.create(body, { ...(req.signal ? { signal: req.signal } : {}), maxRetries: 1 })
      usage.promptTokens += completion.usage?.prompt_tokens ?? 0
      usage.completionTokens += completion.usage?.completion_tokens ?? 0
      content = completion.choices[0]?.message.content ?? ''
    } catch (err) {
      // Sağlayıcı response_format'ı reddetti: istem moduna düşülür, deneme hakkı yenmez (native yalnızca bir kez kapanır).
      if (native && statusOf(err) === 400) {
        native = false
        attempt--
        continue
      }
      throw err
    }
    const parsed = parseStructured(content, req.schema)
    if (parsed.ok) return { value: parsed.value, attempts: attempt, usage }
    lastError = parsed.error
    messages.push(
      { role: 'assistant', content },
      { role: 'user', content: `That output was invalid: ${lastError}\nReply again with ONLY the corrected JSON object.` },
    )
  }
  throw new StructuredOutputError(`${maxAttempts} denemede şemaya uygun JSON üretilemedi: ${lastError}`)
}
