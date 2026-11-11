// [g15] Deterministik kayıt ve tekrar oynatma (#180). Kayıt, sağlayıcıya giden istekleri ve yanıtlarını maskeli saklar;
// tekrar oynatma aynı yanıtları sahte sağlayıcıdan (mock-provider.ts) çevrimdışı döndürür.
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { z } from 'zod'
import type { Session } from './agent.ts'
import { CHAT_PATH, errorResult, fallbackResult, mockReplySchema, providerRequest, type MockReply, type MockRequest, type MockRoute } from './mock-provider.ts'
import { isSensitivePath } from './permissions.ts'
import { maskSecrets } from './secrets.ts'
import { confineToWorkspace } from './workspace.ts'

const entrySchema = z
  .object({
    /**
     * Eşzamanlı oturumları (paralel alt ajanlar) ayıran kulvar: sohbette ilk kullanıcı mesajının özeti.
     * Yoksa aynı uç noktadaki ilk kullanılmamış kayıt eşleşir (elle yazılan tek oturumlu kayıtlar için).
     */
    lane: z.string().max(200).optional(),
    method: z.string().max(10).default('POST'),
    path: z.string().max(500).default(CHAT_PATH),
    /** İsteğin maskeli özeti; varsa tekrar oynatmada mesaj rolleri karşılaştırılır (sapma tespiti). */
    request: z.object({ model: z.string().max(200).optional(), messages: z.array(z.unknown()).max(100_000) }).optional(),
    reply: mockReplySchema.optional(),
    json: z.unknown().optional(),
  })
  .refine((e) => e.reply !== undefined || e.json !== undefined, 'Her kayıtta reply veya json bulunmalı')

export const recordingSchema = z.object({
  version: z.literal(1),
  /** Oturum kimlikleri gibi rastgelelik kaynaklarının türetildiği sabit seed */
  seed: z.number().int().min(0).max(2 ** 31),
  /** Kullanıcının sırayla verdiği girdiler (istemler ve onay yanıtları) */
  inputs: z.array(z.string().max(1_000_000)).max(10_000).default([]),
  entries: z.array(entrySchema).max(100_000),
})
export type Recording = z.infer<typeof recordingSchema>
export type RecordingEntry = z.infer<typeof entrySchema>

const shortHash = (text: string) => createHash('sha256').update(text).digest('hex').slice(0, 16)

const roleOf = (m: unknown): string =>
  typeof m === 'object' && m !== null && 'role' in m && typeof m.role === 'string' ? m.role : '?'

function laneOfMessages(messages: readonly unknown[]): string {
  const first: unknown = messages.find((m) => roleOf(m) === 'user')
  const content = typeof first === 'object' && first !== null && 'content' in first ? first.content : ''
  return `sohbet:${shortHash(JSON.stringify(content ?? ''))}`
}

export function laneOf(request: MockRequest): string {
  if (request.path !== CHAT_PATH) return `${request.method} ${request.path}`
  return laneOfMessages(Array.isArray(request.body.messages) ? request.body.messages : [])
}

/** Seed ve anahtardan RFC 9562 biçimli (v4) belirlenebilir kimlik. */
export function seededId(seed: number, key: string): string {
  const h = createHash('sha256').update(`${seed}:${key}`).digest('hex')
  const variant = '89ab'.charAt(Number.parseInt(h.charAt(16), 16) % 4)
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-${variant}${h.slice(17, 20)}-${h.slice(20, 32)}`
}

let idSeed: number | null = null
const seededSessions = new WeakSet<Session>()

/** Kayıt/tekrar oynatma açılışında çağrılır; null kapatır. */
export function setReplaySeed(seed: number | null): void {
  idSeed = seed
}

/**
 * runTurn'deki [g15] kancası: kayıt ve tekrar oynatmada oturum kimliği (prompt_cache_key, denetim günlüğü)
 * rastgele değil seed + kulvardan türetilir; paralel alt ajanların başlama sırası sonucu değiştirmez.
 */
export function g15BeforeModelCall(session: Session): void {
  if (idSeed === null || seededSessions.has(session)) return
  seededSessions.add(session)
  session.id = seededId(idSeed, laneOfMessages(session.messages))
}

/** Kayıttaki yanıtları sırayla döndüren route. Kulvar ve uç nokta eşleşir; rol dizisi farklıysa sapma hatası. */
export function replayRoute(recording: Recording): MockRoute {
  const used = new Set<number>()
  return (request) => {
    const lane = laneOf(request)
    const index = recording.entries.findIndex(
      (e, i) => !used.has(i) && e.method === request.method && e.path === request.path && (e.lane === undefined || e.lane === lane),
    )
    const entry = recording.entries[index]
    if (!entry) {
      if (request.path !== CHAT_PATH) return fallbackResult(request)
      return errorResult(400, `Tekrar oynatma sapması: kayıtta bu istek için yanıt kalmadı (kulvar ${lane}).`)
    }
    used.add(index)
    if (entry.request) {
      const expected = entry.request.messages.map(roleOf)
      const actual = (Array.isArray(request.body.messages) ? request.body.messages : []).map(roleOf)
      if (expected.join(' ') !== actual.join(' ')) {
        return errorResult(
          400,
          `Tekrar oynatma sapması: ${index + 1}. kayıt ${expected.length} mesajla istenmişti (${expected.join(' ')}), ` +
            `şimdi ${actual.length} mesaj var (${actual.join(' ')}). Araç sonuçları veya girdiler kayıttakinden farklı.`,
        )
      }
    }
    return entry.reply ? { reply: entry.reply } : { json: entry.json }
  }
}

/** Kayıt dosyası yolu: çalışma dizinine hapsedilir, hassas dosya ve .json dışı uzantı reddedilir. */
export function recordingPath(root: string, file: string): string {
  const abs = confineToWorkspace(root, file)
  if (isSensitivePath(abs)) throw new Error(`Hassas dosya kayıt için kullanılamaz: ${file}`)
  if (path.extname(abs).toLowerCase() !== '.json') throw new Error(`Kayıt dosyası .json uzantılı olmalı: ${file}`)
  return abs
}

export function parseRecording(raw: unknown, source: string): Recording {
  const parsed = recordingSchema.safeParse(raw)
  if (!parsed.success) throw new Error(`Kayıt dosyası geçersiz (${source}):\n${z.prettifyError(parsed.error)}`)
  return parsed.data
}

export function loadRecording(file: string): Recording {
  return parseRecording(JSON.parse(readFileSync(file, 'utf8')), file)
}

function maskDeep(value: unknown): unknown {
  if (typeof value === 'string') return maskSecrets(value)
  if (Array.isArray(value)) return value.map(maskDeep)
  if (typeof value === 'object' && value !== null) return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, maskDeep(v)]))
  return value
}

const chunkSchema = z.object({
  choices: z
    .array(
      z.object({
        delta: z
          .object({
            content: z.string().nullish(),
            reasoning_content: z.string().nullish(),
            tool_calls: z
              .array(
                z.object({
                  index: z.number().int().min(0).max(127),
                  id: z.string().nullish(),
                  function: z.object({ name: z.string().nullish(), arguments: z.string().nullish() }).nullish(),
                }),
              )
              .nullish(),
          })
          .nullish(),
        finish_reason: z.string().nullish(),
      }),
    )
    .default([]),
  usage: z.record(z.string(), z.unknown()).nullish(),
})

/** SSE parçalarını tek yanıtta birleştirir (SDK'nin finalChatCompletion birikimiyle aynı alanlar). */
export function replyFromChunks(chunks: readonly unknown[]): MockReply {
  let content = ''
  let reasoning = ''
  let finishReason: string | undefined
  let usage: Record<string, unknown> | undefined
  const calls: Array<{ id: string; name: string; arguments: string }> = []
  for (const raw of chunks) {
    const chunk = chunkSchema.safeParse(raw)
    if (!chunk.success) continue
    if (chunk.data.usage) usage = chunk.data.usage
    const choice = chunk.data.choices[0]
    if (!choice) continue
    content += choice.delta?.content ?? ''
    reasoning += choice.delta?.reasoning_content ?? ''
    if (choice.finish_reason) finishReason = choice.finish_reason
    for (const part of choice.delta?.tool_calls ?? []) {
      const call = (calls[part.index] ??= { id: '', name: '', arguments: '' })
      if (part.id) call.id = part.id
      call.name += part.function?.name ?? ''
      call.arguments += part.function?.arguments ?? ''
    }
  }
  return {
    ...(content ? { content } : {}),
    ...(reasoning ? { reasoning } : {}),
    ...(calls.length > 0 ? { toolCalls: calls.map((c) => ({ ...(c.id ? { id: c.id } : {}), name: c.name, arguments: c.arguments })) } : {}),
    ...(finishReason ? { finishReason } : {}),
    ...(usage ? { usage } : {}),
  }
}

/** create() yanıtını aynı biçime çevirir. */
function replyFromCompletion(raw: unknown): MockReply {
  const parsed = z
    .object({
      choices: z.array(
        z.object({
          finish_reason: z.string().nullish(),
          message: z.object({
            content: z.string().nullish(),
            reasoning_content: z.string().nullish(),
            tool_calls: z.array(z.object({ id: z.string(), function: z.object({ name: z.string(), arguments: z.string() }) })).nullish(),
          }),
        }),
      ),
      usage: z.record(z.string(), z.unknown()).nullish(),
    })
    .parse(raw)
  const choice = parsed.choices[0]
  return replyFromChunks([
    {
      choices: choice
        ? [
            {
              delta: {
                content: choice.message.content,
                reasoning_content: choice.message.reasoning_content,
                tool_calls: choice.message.tool_calls?.map((c, index) => ({ index, id: c.id, function: c.function })),
              },
              finish_reason: choice.finish_reason,
            },
          ]
        : [],
      usage: parsed.usage,
    },
  ])
}

/** SSE metnini olay olay ayrıştırır; çok satırlı data alanları birleştirilir. */
export function parseSse(text: string): unknown[] {
  const chunks: unknown[] = []
  for (const event of text.split(/\r?\n\r?\n/)) {
    const data = event
      .split(/\r?\n/)
      .filter((line) => line.startsWith('data:'))
      .map((line) => line.slice(5).replace(/^ /, ''))
      .join('\n')
    if (!data || data === '[DONE]') continue
    try {
      chunks.push(JSON.parse(data))
    } catch (err) {
      console.warn('Kayıt: ayrıştırılamayan SSE olayı atlandı:', err instanceof Error ? err.message : err)
    }
  }
  return chunks
}

/** Sistem istemi her istekte aynıdır ve yol/tarih içerir; kayıtta yalnızca uzunluğu tutulur. */
function storedRequest(body: Record<string, unknown>): { model?: string; messages: unknown[] } {
  const messages = (Array.isArray(body.messages) ? body.messages : []).map((m: unknown) =>
    roleOf(m) === 'system' && typeof m === 'object' && m !== null && 'content' in m
      ? { role: 'system', content: `<sistem istemi: ${String(m.content).length} karakter>` }
      : m,
  )
  return { ...(typeof body.model === 'string' ? { model: body.model } : {}), messages }
}

export interface Recorder {
  readonly recording: Recording
  /** Kullanıcı girdisini (maskeli) ekler */
  addInput(text: string): void
  /** Sağlayıcı isteklerini kaydeden fetch; diğer istekler dokunulmadan geçer. */
  wrap(base: typeof fetch): typeof fetch
  /** Akışı süren yanıtların kayda yazılmasını bekler */
  flush(): Promise<void>
}

/**
 * Her yeni kayıtta dosya baştan yazılır (eşzamanlı yazma olmaz, süreç kapanırsa son hal diskte kalır).
 * İstek/yanıt başlıkları (Authorization dahil) hiç saklanmaz; tüm metinler maskSecrets'ten geçer.
 */
export function createRecorder(file: string, seed: number): Recorder {
  const recording: Recording = { version: 1, seed, inputs: [], entries: [] }
  const pending = new Set<Promise<void>>()
  const save = () => writeFileSync(file, `${JSON.stringify(recording, null, 2)}\n`, 'utf8')
  const add = (entry: Record<string, unknown>) => {
    recording.entries.push(entrySchema.parse(maskDeep(entry)))
    save()
  }
  save()
  return {
    recording,
    addInput(text) {
      recording.inputs.push(maskSecrets(text))
      save()
    },
    wrap(base) {
      return async (input, init) => {
        const request = providerRequest(input, init)
        const response = await base(input, init)
        if (!request || !response.ok || !response.body) return response
        const meta = {
          lane: laneOf(request),
          method: request.method,
          path: request.path,
          ...(request.path === CHAT_PATH ? { request: storedRequest(request.body) } : {}),
        }
        const fail = (err: unknown) => console.error('Kayıt yazılamadı:', err instanceof Error ? err.message : err)
        if (request.path === CHAT_PATH && request.body.stream === true) {
          // Akış bölünür: biri SDK'ye canlı akar, diğeri bitince kayda yazılır.
          const [mine, theirs] = response.body.tee()
          const done = new Response(mine)
            .text()
            .then((text) => add({ ...meta, reply: replyFromChunks(parseSse(text)) }))
            .catch(fail)
            .finally(() => pending.delete(done))
          pending.add(done)
          return new Response(theirs, { status: response.status, statusText: response.statusText, headers: response.headers })
        }
        try {
          const json: unknown = await response.clone().json()
          add(request.path === CHAT_PATH ? { ...meta, reply: replyFromCompletion(json) } : { ...meta, json })
        } catch (err) {
          fail(err)
        }
        return response
      }
    },
    async flush() {
      await Promise.all(pending)
    },
  }
}
