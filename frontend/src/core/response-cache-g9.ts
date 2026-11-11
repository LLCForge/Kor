// Yerel yanıt önbelleği (madde 52): varsayılan kapalı, yalnızca bu makinede. Anahtar, modelin, araç listesinin ve
// mesajların normalize edilmiş (boşluk/büyük-küçük harf) hâlidir: aynı veya yalnızca biçimce farklı istem tekrar
// sorulunca yanıt yerelden gelir. Gömme tabanlı "anlamsal" benzerlik yok: yerel gömme modeli ağır bir bağımlılık olurdu.
// Sır içeren, görsel/çok parçalı içerikli veya kullanıcının hariç tuttuğu metni içeren istemler hiç önbelleğe girmez.
// Önbellekten gelen araç çağrıları yeniden izin denetiminden geçer (runTurn normal akışı).
import { createHash, randomUUID } from 'node:crypto'
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import OpenAI from 'openai'
import { z } from 'zod'
import type { ChatStream, ChatStreamBody } from './provider-pool-g9.ts'
import { canStoreSecrets, revealSecret, sealSecret } from './connections.ts'
import { maskSecrets } from './secrets.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { DEFAULT_RESPONSE_CACHE, type ResponseCacheSettings } from './settings-g9.ts'

const RESPONSE_CACHE_FILE = path.join(USER_CONFIG_DIR, 'g9-response-cache.json')
const MAX_ENTRIES = 200
const SAVE_DELAY_MS = 1_000

const entrySchema = z.object({
  at: z.number(),
  model: z.string().max(200),
  content: z.string().max(200_000),
  finishReason: z.enum(['stop', 'tool_calls']),
  toolCalls: z.array(z.object({ name: z.string().max(200), arguments: z.string().max(200_000) })).max(64),
})
export type CachedCompletion = z.infer<typeof entrySchema>

let settings: ResponseCacheSettings = DEFAULT_RESPONSE_CACHE
let file = RESPONSE_CACHE_FILE
const entries = new Map<string, CachedCompletion>()
let loading: Promise<void> | null = null
let saveTimer: ReturnType<typeof setTimeout> | null = null

async function load(): Promise<void> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return
    throw err
  }
  // Kasa varsa dosya şifreli gelir; eski düz JSON dosyaları da okunmaya devam eder.
  const plain = canStoreSecrets() && !text.trimStart().startsWith('{') ? unseal(text) : text
  const parsed = z.record(z.string().max(64), entrySchema).safeParse(JSON.parse(plain))
  if (!parsed.success) {
    console.warn('Yanıt önbelleği dosyası bozuk; yok sayıldı.')
    return
  }
  for (const [key, entry] of Object.entries(parsed.data)) entries.set(key, entry)
}

/** Önbellek dosyası şifrelenir: yanıtlar kod/sır içerebilir. Kasa yoksa düz JSON (mode 600) yazılır. */
function seal(plain: string): string {
  if (!canStoreSecrets()) return plain
  try {
    return `korenc:${sealSecret(plain)}`
  } catch {
    return plain
  }
}

function unseal(text: string): string {
  if (!text.startsWith('korenc:') || !canStoreSecrets()) return text
  try {
    return revealSecret({ id: 'response-cache', secret: text.slice('korenc:'.length) }) ?? text
  } catch {
    return text
  }
}

async function save(): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true })
  const tmp = `${file}.${process.pid}.tmp`
  await writeFile(tmp, seal(JSON.stringify(Object.fromEntries(entries))), { encoding: 'utf8', mode: 0o600 })
  await rename(tmp, file)
}

function scheduleSave(): void {
  if (saveTimer) return
  saveTimer = setTimeout(() => {
    saveTimer = null
    save().catch((err: unknown) => console.error('Yanıt önbelleği yazılamadı:', err instanceof Error ? err.message : err))
  }, SAVE_DELAY_MS)
  saveTimer.unref?.()
}

export async function configureResponseCache(next: ResponseCacheSettings, opts: { file?: string } = {}): Promise<void> {
  settings = next
  if (opts.file && opts.file !== file) {
    file = opts.file
    entries.clear()
    loading = null
  }
  if (!next.enabled) return
  loading ??= load().catch((err: unknown) => console.error('Yanıt önbelleği okunamadı:', err instanceof Error ? err.message : err))
  await loading
}

export async function clearResponseCache(): Promise<void> {
  entries.clear()
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = null
  await rm(file, { force: true })
}

export function responseCacheStats(): { enabled: boolean; entries: number } {
  return { enabled: settings.enabled, entries: entries.size }
}

const normText = (text: string) => text.replace(/\s+/g, ' ').trim().toLocaleLowerCase('tr')

/** null: önbelleğe uygun değil (kapalı, görsel/çok parçalı içerik, sır veya hariç tutulan metin). */
export function responseCacheKey(body: ChatStreamBody): string | null {
  if (!settings.enabled) return null
  const exclude = settings.exclude.map((x) => x.toLocaleLowerCase('tr'))
  const parts: unknown[] = []
  for (const m of body.messages) {
    if (m.content !== null && m.content !== undefined && typeof m.content !== 'string') return null
    const text = m.content ?? ''
    if (maskSecrets(text) !== text) return null
    const lower = text.toLocaleLowerCase('tr')
    if (exclude.some((x) => lower.includes(x))) return null
    const calls = m.role === 'assistant' ? (m.tool_calls ?? []).map((c) => (c.type === 'function' ? [c.function.name, c.function.arguments] : [c.type])) : []
    parts.push([m.role, m.role === 'user' || m.role === 'system' ? normText(text) : text, calls])
  }
  const tools = (body.tools ?? []).map((t) => (t.type === 'function' ? t.function.name : t.type))
  const effort = 'reasoning_effort' in body ? body.reasoning_effort : null
  const material = JSON.stringify([body.model, parts, tools, body.temperature ?? null, body.top_p ?? null, body.max_completion_tokens ?? null, effort ?? null])
  return createHash('sha256').update(material).digest('hex')
}

export function cachedCompletion(body: ChatStreamBody): CachedCompletion | null {
  const key = responseCacheKey(body)
  const hit = key ? entries.get(key) : undefined
  if (!key || !hit) return null
  entries.delete(key)
  if (Date.now() - hit.at > settings.ttlHours * 3_600_000) {
    scheduleSave()
    return null
  }
  entries.set(key, hit) // en son kullanılan sona: taşınca en eskisi atılır
  return hit
}

/** Akış tamamlanınca (yalnızca stop/tool_calls ile ve sır içermiyorsa) yanıt saklanır. */
export function rememberCompletion(body: ChatStreamBody, stream: ChatStream): void {
  const key = responseCacheKey(body)
  if (!key) return
  stream.finalChatCompletion().then(
    (completion) => {
      const choice = completion.choices[0]
      if (!choice || (choice.finish_reason !== 'stop' && choice.finish_reason !== 'tool_calls')) return
      const content = choice.message.content ?? ''
      const toolCalls = (choice.message.tool_calls ?? []).flatMap((c) => (c.type === 'function' ? [{ name: c.function.name, arguments: c.function.arguments }] : []))
      if (!content.trim() && toolCalls.length === 0) return
      if (maskSecrets(content) !== content || toolCalls.some((c) => maskSecrets(c.arguments) !== c.arguments)) return
      entries.set(key, { at: Date.now(), model: completion.model, content, finishReason: choice.finish_reason, toolCalls })
      while (entries.size > MAX_ENTRIES) {
        const oldest = entries.keys().next().value
        if (oldest === undefined) break
        entries.delete(oldest)
      }
      scheduleSave()
    },
    // Hata veya iptal asıl tüketiciye (runTurn) zaten ulaşır; burada yalnızca önbelleğe yazılmaz.
    () => undefined,
  )
}

/** Önbellekteki yanıtı SDK'nın normal akış ayrıştırıcısına veren ağsız istemci (kullanım bilgisi yok: maliyet sıfır). */
export function replayResponse(hit: CachedCompletion): OpenAI {
  const id = `g9cache-${randomUUID()}`
  const created = Math.floor(Date.now() / 1000)
  const toolCalls = hit.toolCalls.map((c, index) => ({
    index,
    // Kimlik yenilenir: aynı yanıt aynı sohbette ikinci kez gelirse araç sonucu eşleşmesi karışmasın.
    id: `call_${randomUUID().replaceAll('-', '')}`,
    type: 'function' as const,
    function: { name: c.name, arguments: c.arguments },
  }))
  const chunk = (delta: object, finish: string | null) => ({ id, object: 'chat.completion.chunk', created, model: hit.model, choices: [{ index: 0, delta, finish_reason: finish }] })
  const sse = [
    chunk({ role: 'assistant', content: hit.content, ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}) }, null),
    chunk({}, hit.finishReason),
  ]
    .map((c) => `data: ${JSON.stringify(c)}\n\n`)
    .join('')
    .concat('data: [DONE]\n\n')
  return new OpenAI({
    apiKey: 'g9-response-cache',
    baseURL: 'http://127.0.0.1/g9-response-cache',
    maxRetries: 0,
    fetch: async () => new Response(sse, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
  })
}
