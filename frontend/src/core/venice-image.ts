// Venice metinden görsel üretimi (POST /image/generate). Araçlara anahtar sızmaması için generate_image aracı bunu
// doğrudan çağırmaz; oturum, anahtarı kapatan bir yetenek (ToolContext.generateImage) olarak sağlar.
import { VENICE_BASE_URL } from './settings.ts'

const DEFAULT_MODEL = 'venice-sd35'
const TIMEOUT_MS = 120_000
const MIN_DIM = 256
const MAX_DIM = 1280

export interface VeniceImageOpts {
  prompt: string
  model?: string
  width?: number
  height?: number
  negativePrompt?: string
  format?: 'png' | 'jpeg' | 'webp'
}
export interface VeniceImage {
  data: Buffer
  format: string
}
type FetchLike = (url: string, init: RequestInit) => Promise<{ ok: boolean; status: number; json(): Promise<unknown>; text(): Promise<string> }>

const clampDim = (n: number): number => Math.max(MIN_DIM, Math.min(MAX_DIM, Math.round(n)))

export async function generateVeniceImage(apiKey: string, opts: VeniceImageOpts, fetchImpl: FetchLike = fetch): Promise<VeniceImage[]> {
  const format = opts.format ?? 'png'
  const body: Record<string, unknown> = { model: opts.model ?? DEFAULT_MODEL, prompt: opts.prompt, format }
  if (opts.width) body.width = clampDim(opts.width)
  if (opts.height) body.height = clampDim(opts.height)
  if (opts.negativePrompt) body.negative_prompt = opts.negativePrompt

  const res = await fetchImpl(`${VENICE_BASE_URL}/image/generate`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${apiKey}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(TIMEOUT_MS),
  })
  if (!res.ok) {
    const detail = await res.text().catch(() => '')
    throw new Error(`Görsel üretilemedi (HTTP ${res.status}): ${detail.slice(0, 300)}`)
  }
  const json = (await res.json()) as { images?: unknown }
  const list = Array.isArray(json.images) ? json.images : []
  return list.filter((x): x is string => typeof x === 'string' && x.length > 0).map((b64) => ({ data: Buffer.from(b64, 'base64'), format }))
}
