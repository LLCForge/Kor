// Yerel model yönetimi (madde 47-ek): Ollama'da yüklü modelleri listeleme, indirme (pull), bellekten boşaltma ve
// bellek kullanımı. Yalnızca bu makinedeki (localhost) Ollama ile konuşur ve yalnızca kullanıcı başlatınca çalışır.
import { z } from 'zod'
import { effectiveBaseURL, providerFetch } from './provider-pool-g9.ts'
import { isLoopbackHost } from './providers.ts'

/** Ollama model adı: "llama3.1:8b", "library/qwen3:latest" gibi; yol ve kabuk karakterleri yok. */
const OLLAMA_MODEL_NAME = /^[A-Za-z0-9][A-Za-z0-9._\-/:]{0,199}$/

export interface OllamaModel {
  name: string
  sizeBytes: number
  family: string | null
  parameterSize: string | null
  quantization: string | null
}

export interface OllamaRunning {
  name: string
  sizeBytes: number
  vramBytes: number
  expiresAt: string | null
}

export interface PullProgress {
  status: string
  completed: number | null
  total: number | null
}

const tagsSchema = z.object({
  models: z
    .array(
      z.object({
        name: z.string().max(200),
        size: z.number().nonnegative().optional(),
        details: z
          .object({ family: z.string().max(100).nullish(), parameter_size: z.string().max(50).nullish(), quantization_level: z.string().max(50).nullish() })
          .optional(),
      }),
    )
    .max(1_000),
})

const psSchema = z.object({
  models: z
    .array(z.object({ name: z.string().max(200), size: z.number().nonnegative().optional(), size_vram: z.number().nonnegative().optional(), expires_at: z.string().max(100).nullish() }))
    .max(1_000),
})

const progressSchema = z.object({
  status: z.string().max(500).optional(),
  completed: z.number().nonnegative().optional(),
  total: z.number().nonnegative().optional(),
  error: z.string().max(2_000).optional(),
})

/** Ollama yönetim kökü (/v1 olmadan); uzak sunucuda yönetim reddedilir. */
export function ollamaRoot(): string {
  const url = new URL(effectiveBaseURL('ollama'))
  if (!isLoopbackHost(url.hostname)) throw new Error('Yerel model yönetimi yalnızca bu makinedeki (localhost) Ollama için kullanılabilir.')
  return url.origin
}

export function assertOllamaName(name: string): string {
  if (!OLLAMA_MODEL_NAME.test(name)) throw new Error(`Geçersiz model adı: ${name}`)
  return name
}

async function call(method: 'GET' | 'POST', pathname: string, body?: object, signal?: AbortSignal): Promise<Response> {
  const res = await providerFetch('ollama')(`${ollamaRoot()}${pathname}`, {
    method,
    ...(body ? { body: JSON.stringify(body), headers: { 'content-type': 'application/json' } } : {}),
    signal: signal ?? AbortSignal.timeout(15_000),
  })
  if (!res.ok) {
    const text = (await res.text()).slice(0, 300)
    throw new Error(`Ollama isteği başarısız (HTTP ${res.status})${text ? `: ${text}` : ''}`)
  }
  return res
}

export async function ollamaList(signal?: AbortSignal): Promise<{ models: OllamaModel[]; running: OllamaRunning[] }> {
  const [tags, ps] = await Promise.all([call('GET', '/api/tags', undefined, signal), call('GET', '/api/ps', undefined, signal)])
  const models = tagsSchema.parse(await tags.json()).models.map((m) => ({
    name: m.name,
    sizeBytes: m.size ?? 0,
    family: m.details?.family ?? null,
    parameterSize: m.details?.parameter_size ?? null,
    quantization: m.details?.quantization_level ?? null,
  }))
  const running = psSchema.parse(await ps.json()).models.map((m) => ({ name: m.name, sizeBytes: m.size ?? 0, vramBytes: m.size_vram ?? 0, expiresAt: m.expires_at ?? null }))
  return { models, running }
}

/** keep_alive: 0 modeli hemen bellekten boşaltır (Ollama API). */
export async function ollamaUnload(name: string): Promise<void> {
  const res = await call('POST', '/api/generate', { model: assertOllamaName(name), keep_alive: 0 })
  await res.body?.cancel()
}

/** NDJSON ilerleme akışını okur; Ollama hata satırı gönderirse fırlatır. */
export async function ollamaPull(name: string, onProgress: (p: PullProgress) => void, signal: AbortSignal): Promise<void> {
  const res = await call('POST', '/api/pull', { model: assertOllamaName(name), stream: true }, signal)
  if (!res.body) return
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader()
  let buffer = ''
  for (;;) {
    const { done, value } = await reader.read()
    if (value) buffer += value
    const lines = buffer.split('\n')
    buffer = done ? '' : (lines.pop() ?? '')
    for (const line of lines) {
      if (!line.trim()) continue
      const p = progressSchema.parse(JSON.parse(line))
      if (p.error) throw new Error(`Ollama: ${p.error}`)
      onProgress({ status: p.status ?? '', completed: p.completed ?? null, total: p.total ?? null })
    }
    if (done) return
  }
}
