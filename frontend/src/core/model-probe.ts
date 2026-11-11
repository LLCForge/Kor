// Model karşılaştırma testi: aynı istem araçsız, tek turda bir modele gönderilir; süre, token ve maliyet ölçülür.
import type OpenAI from 'openai'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import { estimateCostUsd, type ModelPrice } from './model-catalog.ts'

/** Test yanıtı bu sınırda kesilir: karşılaştırma için yeterli, maliyeti sınırlar. */
export const PROBE_MAX_TOKENS = 2048

export interface ProbeResult {
  model: string
  text: string
  durationMs: number
  /** İlk metin parçasının geldiği an; metin hiç gelmediyse null */
  firstTokenMs: number | null
  promptTokens: number | null
  completionTokens: number | null
  costUsd: number | null
  finishReason: string | null
}

export async function runModelProbe(
  client: OpenAI,
  model: string,
  prompt: string,
  signal: AbortSignal,
  price: ModelPrice | null,
  now: () => number = () => performance.now(),
): Promise<ProbeResult> {
  const messages: ChatCompletionMessageParam[] = [{ role: 'user', content: prompt }]
  const body = {
    model,
    messages,
    max_completion_tokens: PROBE_MAX_TOKENS,
    stream_options: { include_usage: true },
    // Venice'ın kendi sistem istemi karşılaştırmayı bozmasın.
    venice_parameters: { include_venice_system_prompt: false },
  }
  const started = now()
  let firstTokenMs: number | null = null
  const stream = client.chat.completions.stream(body, { signal })
  stream.on('content.delta', () => {
    firstTokenMs ??= Math.round(now() - started)
  })
  const completion = await stream.finalChatCompletion()
  const usage = completion.usage
  const choice = completion.choices[0]
  return {
    model,
    text: choice?.message.content ?? '',
    durationMs: Math.round(now() - started),
    firstTokenMs,
    promptTokens: usage?.prompt_tokens ?? null,
    completionTokens: usage?.completion_tokens ?? null,
    costUsd: usage ? estimateCostUsd(price, usage.prompt_tokens, usage.completion_tokens) : null,
    finishReason: choice?.finish_reason ?? null,
  }
}
