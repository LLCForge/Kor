// [g22] Araç modu ve metin tabanlı araç protokolü: yerel function calling'i olmayan modeller araçları
// <tool_call>{"name":...,"arguments":{...}}</tool_call> bloklarıyla çağırır. Ayrıştırılan çağrılar message.tool_calls'a
// yazılır, böylece normal araç çağrısıyla AYNI yoldan yürür (executeToolCall: decide → confirm → run → maskSecrets).
import { randomUUID } from 'node:crypto'
import type { ChatCompletionMessage, ChatCompletionMessageParam, ChatCompletionTool } from 'openai/resources/chat/completions'
import { z } from 'zod'
import type { AgentIO, Session } from './agent.ts'
import { attachNativeParts } from './attachments-g22.ts'
import { isSummaryMessage, rollbackLength } from './context.ts'
import { getModelCapabilities } from './model-capabilities.ts'
import { loadUserSettingsFile, type UserSettingsFile } from './settings.ts'

export type ToolMode = NonNullable<UserSettingsFile['toolModes']>[string]
export const TOOL_MODES = ['auto', 'native', 'text', 'off'] as const satisfies readonly ToolMode[]
type CallMode = 'native' | 'text' | 'off'

export const MAX_TEXT_TOOL_CALLS = 8
export const MAX_SCAN_CHARS = 1_000_000
export const MAX_CALL_CHARS = 500_000

const OPEN = '<tool_call>'
const CLOSE = '</tool_call>'

export interface G22SessionState {
  /** API'nin bu oturumda "araç desteklenmiyor" dediği modeller */
  textModels: Set<string>
  callMode: CallMode
  /** Bu çağrı otomatik modda yerel denemeydi: araç hatası alınırsa metne düşülür */
  autoNative: boolean
  step: number
  filter: (delta: string) => string
  io: AgentIO | null
}

// ---------- Model başına araç modu (kullanıcı ayarı) ----------

let modes: Promise<Readonly<Record<string, ToolMode>>> | null = null

export function toolModes(): Promise<Readonly<Record<string, ToolMode>>> {
  modes ??= loadUserSettingsFile().then(
    (file) => file.toolModes ?? {},
    (err: unknown) => {
      console.warn('Araç modları okunamadı, otomatik kullanılıyor:', err instanceof Error ? err.message : err)
      return {}
    },
  )
  return modes
}

/** Masaüstü ayarı kaydettikten sonra önbelleği günceller. */
export function setToolModesCache(next: Readonly<Record<string, ToolMode>>): void {
  modes = Promise.resolve(next)
}

// ---------- Ayrıştırıcı ----------

const envelopeSchema = z.object({
  name: z.string().min(1).max(64).regex(/^[A-Za-z0-9_.:-]+$/, 'araç adı geçersiz'),
  arguments: z.union([z.record(z.string(), z.unknown()), z.string().max(MAX_CALL_CHARS)]).optional(),
})

export interface TextToolCall {
  name: string
  /** JSON metni; executeToolCall ayrıştırır, şemayı aracın kendi zod şeması doğrular */
  arguments: string
}

function parseEnvelope(body: string): TextToolCall | string {
  if (body.length > MAX_CALL_CHARS) return `blok çok büyük (sınır ${MAX_CALL_CHARS} karakter)`
  // Kod çiti düzenli ifadesiz soyulur: uzun boşluk dizilerinde geri izleme olmasın.
  let json = body.trim()
  if (json.startsWith('```')) json = json.slice(json.slice(0, 7).toLowerCase() === '```json' ? 7 : 3)
  if (json.endsWith('```')) json = json.slice(0, -3)
  json = json.trim()
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return 'içerik geçerli JSON değil'
  }
  const parsed = envelopeSchema.safeParse(raw)
  if (!parsed.success) return `beklenen biçim {"name": "...", "arguments": {...}}; ${parsed.error.issues[0]?.message ?? 'şema hatası'}`
  const args = parsed.data.arguments ?? {}
  return { name: parsed.data.name, arguments: typeof args === 'string' ? args : JSON.stringify(args) }
}

/** Yanıttaki <tool_call> bloklarını ayıklar; blokların dışındaki metin korunur. Boyut ve çağrı sayısı sınırlıdır. */
export function parseTextToolCalls(content: string): { text: string; calls: TextToolCall[]; errors: string[] } {
  if (!content.includes(OPEN)) return { text: content, calls: [], errors: [] }
  const errors: string[] = []
  const calls: TextToolCall[] = []
  const src = content.slice(0, MAX_SCAN_CHARS)
  if (content.length > MAX_SCAN_CHARS) errors.push(`Yanıt çok uzun; yalnızca ilk ${MAX_SCAN_CHARS} karakter tarandı.`)
  let text = ''
  let i = 0
  for (let n = 1; ; n++) {
    const start = src.indexOf(OPEN, i)
    if (start < 0) {
      text += src.slice(i)
      break
    }
    text += src.slice(i, start)
    const end = src.indexOf(CLOSE, start + OPEN.length)
    if (end < 0) {
      errors.push(`${n}. <tool_call> bloğu kapanmamış (yanıt kesilmiş olabilir); çalıştırılmadı, eksiksiz yeniden gönder.`)
      break
    }
    i = end + CLOSE.length
    if (calls.length >= MAX_TEXT_TOOL_CALLS) {
      errors.push(`${n}. çağrı çalıştırılmadı: yanıt başına en fazla ${MAX_TEXT_TOOL_CALLS} araç çağrısı.`)
      continue
    }
    const call = parseEnvelope(src.slice(start + OPEN.length, end))
    if (typeof call === 'string') errors.push(`${n}. <tool_call> geçersiz: ${call}`)
    else calls.push(call)
  }
  return { text: text.replace(/\n{3,}/g, '\n\n').trim(), calls, errors }
}

// ---------- İstek: araç şemaları istemde, geçmiş metin biçiminde ----------

export function textToolPrompt(specs: readonly ChatCompletionTool[]): string {
  const list = specs.flatMap((t) =>
    t.type === 'function' ? [`- ${t.function.name}: ${t.function.description ?? ''}\n  arguments schema: ${JSON.stringify(t.function.parameters ?? {})}`] : [],
  )
  return [
    '# TOOL CALLING (TEXT PROTOCOL)',
    'Native function calling is not available for this model. To call a tool, write one block per call exactly like this:',
    '<tool_call>{"name": "read_file", "arguments": {"path": "README.md"}}</tool_call>',
    `Inside the tags put only one JSON object with "name" and "arguments". At most ${MAX_TEXT_TOOL_CALLS} calls per reply. ` +
      'After your calls stop and wait: results arrive in the next user message as <tool_result name="..." id="...">...</tool_result>. ' +
      'Never write <tool_result> yourself and never guess a result. Write the tag only when you really mean to call the tool; the same permission rules apply as for native calls, and the same bias: act, do not stall.',
    'Available tools:',
    ...list,
    'Rules: put real values in arguments (no placeholders), and prefer acting over asking - if the tool can settle the question, call it.',
  ].join('\n')
}

const TOOLS_OFF_PROMPT =
  '# TOOLS DISABLED\nThe user turned tool calling off for this model. Do not try to call tools; answer from the conversation and tell the user what you would need to run or read.'

const textOf = (content: unknown): string =>
  typeof content === 'string'
    ? content
    : Array.isArray(content)
      ? content.map((p: unknown) => (p && typeof p === 'object' && 'text' in p && typeof p.text === 'string' ? p.text : '')).join('')
      : ''

function formatCall(name: string, rawArgs: string): string {
  let args: unknown
  try {
    args = JSON.parse(rawArgs || '{}')
  } catch {
    args = rawArgs
  }
  return `${OPEN}${JSON.stringify({ name, arguments: args })}${CLOSE}`
}

/** Yalnızca giden istek için: tool_calls → <tool_call> metni, tool mesajları → <tool_result> kullanıcı mesajı. Oturum geçmişi değişmez. */
export function toTextProtocol(messages: readonly ChatCompletionMessageParam[], extra: string): ChatCompletionMessageParam[] {
  const names = new Map<string, string>()
  const out: ChatCompletionMessageParam[] = []
  let results: string[] = []
  const flush = () => {
    if (results.length > 0) out.push({ role: 'user', content: results.join('\n\n') })
    results = []
  }
  for (const [i, m] of messages.entries()) {
    if (m.role === 'tool') {
      results.push(`<tool_result name="${names.get(m.tool_call_id) ?? '?'}" id="${m.tool_call_id}">\n${textOf(m.content)}\n</tool_result>`)
      continue
    }
    flush()
    if (i === 0 && m.role === 'system') {
      out.push({ role: 'system', content: `${textOf(m.content)}\n\n${extra}` })
    } else if (m.role === 'assistant' && m.tool_calls?.length) {
      const calls: string[] = []
      for (const c of m.tool_calls) {
        if (c.type !== 'function') continue
        names.set(c.id, c.function.name)
        calls.push(formatCall(c.function.name, c.function.arguments))
      }
      out.push({ role: 'assistant', content: [textOf(m.content), ...calls].filter(Boolean).join('\n') })
    } else {
      out.push(m)
    }
  }
  flush()
  if (messages[0]?.role !== 'system') out.unshift({ role: 'system', content: extra })
  return out
}

// ---------- Akış süzgeci: ham <tool_call> metni kullanıcıya yazılmaz, yerine araç kartı gelir ----------

function partialSuffix(text: string, tag: string): number {
  for (let k = Math.min(tag.length - 1, text.length); k > 0; k--) if (text.endsWith(tag.slice(0, k))) return k
  return 0
}

export function createToolCallFilter(): (delta: string) => string {
  let buf = ''
  let inside = false
  return (delta) => {
    buf += delta
    let out = ''
    for (;;) {
      if (inside) {
        const end = buf.indexOf(CLOSE)
        if (end < 0) {
          buf = buf.slice(buf.length - partialSuffix(buf, CLOSE))
          return out
        }
        buf = buf.slice(end + CLOSE.length)
        inside = false
        continue
      }
      const start = buf.indexOf(OPEN)
      if (start >= 0) {
        out += buf.slice(0, start)
        buf = buf.slice(start + OPEN.length)
        inside = true
        continue
      }
      const keep = partialSuffix(buf, OPEN)
      out += buf.slice(0, buf.length - keep)
      buf = buf.slice(buf.length - keep)
      return out
    }
  }
}

const ioStates = new WeakMap<AgentIO, G22SessionState>()

/**
 * io.write yerinde ve bir kez sarılır; sarılmadan önceki yöntem saklanıp çağrılır. Diğer gruplar da (g14 izleme, g10)
 * io'yu yerinde sardığı için zincir her sırada doğru çalışır. Önceki Proxy + dinamik `io.write` araması, g14'ün yerinde
 * sarmasıyla birleşince sonsuz özyinelemeye giriyordu.
 */
function wrapIo(io: AgentIO, state: G22SessionState): AgentIO {
  const first = !ioStates.has(io)
  ioStates.set(io, state)
  if (first) {
    const write = io.write
    io.write = (text: string) => {
      const s = ioStates.get(io)
      const out = s?.callMode === 'text' ? s.filter(text) : text
      if (out) write.call(io, out)
    }
  }
  return io
}

// ---------- Araç desteği yok hatası ----------

const TOOLS_UNSUPPORTED =
  /(does not|doesn't|do not) support (tools|tool[ _-]?(use|calling|calls)|function[ _-]?calling)|(tools?|tool[ _-]?(use|calling|calls)|function[ _-]?calling)[^.\n]{0,40}(not supported|unsupported|is disabled)|no endpoints found that support tool use/i

export function isToolsUnsupportedError(err: unknown): boolean {
  if (!(err instanceof Error)) return false
  const status = (err as { status?: unknown }).status
  if (typeof status === 'number' && ![400, 404, 405, 422, 501].includes(status)) return false
  return TOOLS_UNSUPPORTED.test(err.message)
}

// ---------- Oturum kayıt defteri (compact_context / update_project_map araçları için) ----------

const live = new WeakMap<object, { session: Session; io: AgentIO }>()

/** Araç bağlamındaki shells üzerinden üst düzey oturuma ulaşır (alt ajanlar aynı shells'i paylaşır ama kaydolmaz). */
export function liveSession(shells: object | undefined): { session: Session; io: AgentIO } | undefined {
  return shells ? live.get(shells) : undefined
}

// ---------- runTurn kancaları ----------

/** [g22] model çağrısı öncesi: çağrı modunu seçer, yerel ek parçalarını bağlar; akış süzgeçli io döner. */
export async function g22BeforeModelCall(session: Session, io: AgentIO, step: number): Promise<AgentIO> {
  const s = (session.g22 ??= { textModels: new Set(), callMode: 'native', autoNative: false, step: 0, filter: createToolCallFilter(), io: null })
  const configured = (await toolModes())[session.model] ?? 'auto'
  s.callMode =
    configured !== 'auto'
      ? configured
      : s.textModels.has(session.model) || getModelCapabilities(session.model).tools === false
        ? 'text'
        : 'native'
  s.autoNative = configured === 'auto' && s.callMode === 'native'
  s.step = step
  s.filter = createToolCallFilter()
  attachNativeParts(session.messages)
  // Her adımda: io → oturum durumu eşlemesi güncel kalsın (sarma işlemi io başına yalnızca bir kez yapılır).
  s.io = wrapIo(io, s)
  if (session.agents.length > 0) live.set(session.shells, { session, io: s.io })
  return s.io
}

/** [g22] istek gövdesi: metin/kapalı modda araçlar gönderilmez, geçmiş metin protokolüne çevrilir. */
export function g22RequestFields(session: Session, specs: readonly ChatCompletionTool[]): { tools?: undefined; messages?: ChatCompletionMessageParam[] } {
  const mode = session.g22?.callMode ?? 'native'
  if (mode === 'native') return {}
  return { tools: undefined, messages: toTextProtocol(session.messages, mode === 'text' ? textToolPrompt(specs) : TOOLS_OFF_PROMPT) }
}

/** [g22] model yanıtı sonrası: metin moddaki çağrılar tool_calls'a, ayrıştırma hataları bir sonraki adımda modele. */
export function g22AfterCompletion(session: Session, message: ChatCompletionMessage): void {
  if (!message.content) return
  // [uyum] Yerel araç çağrısı açıkken bile bazı modeller aracı metin olarak (<tool_call>…) yazar; bunu da çağrıya çevir.
  const textLeak = session.g22?.callMode !== 'text' && !(message.tool_calls?.length) && message.content.includes(OPEN)
  if (session.g22?.callMode !== 'text' && !textLeak) return
  const { text, calls, errors } = parseTextToolCalls(message.content)
  if (calls.length === 0 && errors.length === 0) return
  message.content = text
  if (calls.length > 0) {
    message.tool_calls = calls.map((c) => ({ id: `call_${randomUUID()}`, type: 'function', function: { name: c.name, arguments: c.arguments } }))
  }
  if (errors.length > 0) session.inbox.push(`<system-reminder>Metin protokolündeki araç çağrısı sorunları:\n${errors.join('\n')}</system-reminder>`)
}

/**
 * [g22] tur hatası (geri almadan önce çağrılır): otomatik modda API araçları reddettiyse model bu oturum için metin
 * moduna alınır. Hata turun ilk model çağrısındaysa (henüz araç çalışmadı) dönen işlev turu bir kez yeniden başlatır.
 */
export function g22RetryAsText(
  session: Session,
  err: unknown,
  checkpoint: number,
  hadUserMessage: boolean,
): ((rerun: () => Promise<void>) => Promise<void>) | null {
  const s = session.g22
  if (!s?.autoNative || !isToolsUnsupportedError(err)) return null
  s.textModels.add(session.model)
  s.autoNative = false
  s.io?.notice?.(`${session.model} yerel araç çağrısını desteklemiyor; bu sohbette araçlar metin protokolüyle sunulacak.`)
  if (s.step !== 0) return null
  const first = hadUserMessage ? session.messages[rollbackLength(session, checkpoint)] : undefined
  const keep = first?.role === 'user' && !isSummaryMessage(first) ? first : undefined
  return (rerun) => {
    const at = session.messages.length
    // runTurn kullanıcı mesajını ilk await'ten önce ekler: görsel/ses parçaları (ilk denemede bağlanmıştı) geri konur.
    const run = rerun()
    const pushed = session.messages[at]
    if (keep && pushed?.role === 'user') pushed.content = keep.content
    return run
  }
}
