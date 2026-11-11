import { createHash } from 'node:crypto'
import { originalMessagesOf } from './tool-prune.ts' // [perf] budanmış istek gövdesi → asıl dizi
import type OpenAI from 'openai'
import type { ChatCompletion } from 'openai/resources/chat/completions'
import type { AgentIO, Session } from './agent.ts'
import { autoVerify } from './auto-verify.ts'
import { maskSecrets, registerSecret } from './secrets.ts'
import type { Tool, ToolContext } from './tools.ts'

export const MAX_LENGTH_CONTINUATIONS = 3
/** Aynı çağrı aynı sonucu verdikten sonraki tekrar engellenir; bu kadar tekrar turu durdurur. */
export const LOOP_BLOCK_AT = 3
export const LOOP_STOP_AT = 15
/** Aynı çağrı arada bu kadardan fazla FARKLI çağrı olmuşsa döngü sayılmaz (ilerleme var: başka dosyaya bakıp geri dönmek normaldir). */
export const LOOP_GAP = 2
// Sonucu zamanla değişmesi beklenen yoklama araçları döngü sayılmaz.
const LOOP_EXEMPT = new Set(['bash_output'])

/** Üst oturum ve alt ajanlarının paylaştığı bağlantı (anahtar: ortak ShellRegistry). */
export interface SessionLink {
  notify(text: string): void
  client(): OpenAI
  /** Turda dosya değiştiren bir araç başarıyla çalıştı mı (autoVerify için) */
  filesChanged: boolean
}

interface CallRecord {
  streak: number
  blocked: number
  lastHash: string
  /** Son görüldüğü çağrı sırası */
  lastSeq: number
}

interface TurnState {
  io: AgentIO | null
  continuations: number
  calls: Map<string, CallRecord>
  stopReason: string | null
  /** Bu turdaki araç çağrısı sırası (döngü aralığı için) */
  seq?: number
}

const links = new WeakMap<object, SessionLink>()
const turnStates = new WeakMap<Session, TurnState>()
const originalTools = new WeakMap<Tool, Tool>()
const originalClients = new WeakMap<object, OpenAI>()
const wrappedFor = new WeakMap<Session, { tools: readonly Tool[]; client: OpenAI }>()

export function linkFor(owner: object | undefined): SessionLink | undefined {
  return owner ? links.get(owner) : undefined
}

export function stableStringify(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(',')}]`
  if (value && typeof value === 'object') {
    const entries = Object.entries(value).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${stableStringify(v)}`).join(',')}}`
  }
  return JSON.stringify(value) ?? 'undefined'
}

const hash = (text: string) => createHash('sha256').update(text).digest('hex')

/**
 * Döngü tespiti (madde 144): aynı tur içinde aynı araç + aynı argüman + aynı sonuç ikinci kez gelince,
 * üçüncü çağrı çalıştırılmaz ve modele uyarı döner; on beşinci ARDIŞIK tekrar turu durdurur. Araya başka çağrılar
 * girmişse (başka dosyaya bakıp geri dönmek) tekrar sayılmaz. Sonuç farklıysa
 * (ör. düzeltmeden sonra yeniden test) ilerleme var sayılır ve sayaç sıfırlanır.
 */
export function guardTool(tool: Tool, turn: () => TurnState, link: SessionLink): Tool {
  const wrapped: Tool = {
    ...tool,
    run: async (raw: unknown, ctx: ToolContext) => {
      if (LOOP_EXEMPT.has(tool.name)) return tool.run(raw, ctx)
      const state = turn()
      const key = `${tool.name}:${stableStringify(raw)}`
      state.seq = (state.seq ?? 0) + 1
      const record = state.calls.get(key)
      // Araya LOOP_GAP'ten fazla farklı çağrı girdiyse bu bir tekrar değil, dönüş: sayaç sıfırlanır.
      if (record && state.seq - record.lastSeq > LOOP_GAP + 1) {
        record.streak = 0
        record.blocked = 0
      }
      if (record && record.streak >= LOOP_BLOCK_AT - 1) {
        record.lastSeq = state.seq
        record.blocked++
        const count = record.streak + record.blocked
        if (count >= LOOP_STOP_AT) {
          state.stopReason = `Döngü tespit edildi: ${tool.name} aynı argümanlarla ${count} kez çağrıldı; tur durduruldu.`
          return `${state.stopReason} Kullanıcıya ne denediğini ve neden ilerleyemediğini açıkla.`
        }
        return (
          `Döngü uyarısı: ${tool.name} bu turda aynı argümanlarla ${count}. kez çağrıldı ve önceki ${record.streak} çağrı aynı sonucu verdi; ` +
          'bu çağrı ÇALIŞTIRILMADI. Aynı çağrıyı tekrarlama: elindeki sonucu kullan, farklı bir yaklaşım dene veya kullanıcıya sor.'
        )
      }
      let result: string
      try {
        result = await tool.run(raw, ctx)
      } catch (err) {
        remember(state, key, `hata:${err instanceof Error ? err.message : String(err)}`)
        throw err
      }
      remember(state, key, result)
      if (tool.kind === 'edit' && tool.name !== 'remember') link.filesChanged = true
      return result
    },
  }
  originalTools.set(wrapped, tool)
  return wrapped
}

function remember(state: TurnState, key: string, result: string): void {
  const h = hash(result)
  const record = state.calls.get(key)
  const seq = state.seq ?? 0
  if (record && record.lastHash === h) {
    record.streak++
    record.lastSeq = seq
  } else state.calls.set(key, { streak: 1, blocked: 0, lastHash: h, lastSeq: seq })
}

/** Uzunluk sınırında kesilen argüman JSON'u kapanmamış olur; böyle bir çağrı asla çalıştırılmaz. */
function isCompleteJson(text: string): boolean {
  try {
    JSON.parse(text || '{}')
    return true
  } catch {
    return false
  }
}

/**
 * finish_reason: length (madde 145): yarım araç çağrıları atılır ve "Devam et" bildirimi gelen kutusuna
 * konur; runTurn bunu bir sonraki adımda kullanıcı mesajı olarak ekler (tur başına en fazla 3 kez).
 * Normal bitişte, dosya değişmişse autoVerify tetiklenir (madde 149).
 */
export function afterCompletion(completion: ChatCompletion, session: Session, state: TurnState, onTurnEnd: () => void): void {
  const choice = completion.choices[0]
  if (!choice) return
  const calls = choice.message.tool_calls ?? []
  if (choice.finish_reason === 'length') {
    const complete = calls.filter((c) => c.type !== 'function' || isCompleteJson(c.function.arguments))
    const dropped = calls.length - complete.length
    if (dropped > 0) choice.message.tool_calls = complete
    state.continuations++
    if (state.continuations > MAX_LENGTH_CONTINUATIONS) {
      state.io?.write(`\n[Yanıt ${MAX_LENGTH_CONTINUATIONS} kez uzunluk sınırına takıldı; otomatik devam durduruldu]\n`)
      return
    }
    const note = dropped > 0 ? ` Yarım kalan ${dropped} araç çağrısı çalıştırılmadı; gerekiyorsa eksiksiz olarak yeniden gönder.` : ''
    session.inbox.push(
      `<system-reminder>Önceki yanıtın uzunluk sınırına takılıp kesildi (otomatik devam ${state.continuations}/${MAX_LENGTH_CONTINUATIONS}).${note} Devam et: tekrar etmeden tam kaldığın yerden sürdür.</system-reminder>`,
    )
    return
  }
  if (calls.length === 0 && session.inbox.length === 0) onTurnEnd()
}

function unwrapClient(client: OpenAI): OpenAI {
  return originalClients.get(client) ?? client
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

type StreamArgs = Parameters<OpenAI['chat']['completions']['stream']>

function guardClient(base: OpenAI, session: Session, state: () => TurnState, onTurnEnd: () => void): OpenAI {
  const stream = (...args: StreamArgs) => {
    const [body] = args
    const s = base.chat.completions.stream(...args)
    // Yalnızca ajan döngüsünün kendi çağrısı; başka grupların (ör. özetleme) çağrıları etkilenmez.
    if (originalMessagesOf(body.messages) !== session.messages) return s
    const final = s.finalChatCompletion.bind(s)
    s.finalChatCompletion = async () => {
      const completion = await final()
      afterCompletion(completion, session, state(), onTurnEnd)
      return completion
    }
    return s
  }
  const completions = forward(base.chat.completions, (p) => (p === 'stream' ? stream : undefined))
  const chat = forward(base.chat, (p) => (p === 'completions' ? completions : undefined))
  const proxy = forward(base, (p) => (p === 'chat' ? chat : undefined))
  originalClients.set(proxy, base)
  return proxy
}

/**
 * runTurn'deki [g5] işaretinden her model çağrısından önce çağrılır. true dönerse tur durdurulur.
 * Oturum bağlantısını kurar, araçları döngü korumasıyla ve istemciyi uzunluk/tur sonu kancasıyla sarar.
 */
export function g5BeforeModelCall(
  session: Session,
  step: number,
  io: AgentIO,
  ctx: ToolContext,
  notify: (session: Session, text: string) => void,
): boolean {
  registerSecret((session.client as { apiKey?: unknown }).apiKey)
  if (!links.has(session.shells)) {
    links.set(session.shells, { notify: (text) => notify(session, maskSecrets(text)), client: () => session.client, filesChanged: false })
  }
  const link = links.get(session.shells)
  if (!link) return false

  let state = turnStates.get(session)
  if (!state || step === 0) {
    state = { io, continuations: 0, calls: new Map(), stopReason: null }
    turnStates.set(session, state)
  }
  state.io = io
  if (state.stopReason) {
    io.write(`\n[${state.stopReason}]\n`)
    return true
  }

  const getState = (): TurnState => turnStates.get(session) ?? { io: null, continuations: 0, calls: new Map(), stopReason: null }
  const wrapped = wrappedFor.get(session)
  if (wrapped?.tools !== session.tools) {
    // Alt ajan, üst oturumun sarılmış araçlarını alır: önce çözülür ki sayaçlar karışmasın.
    session.tools = session.tools.map((t) => guardTool(originalTools.get(t) ?? t, getState, link))
  }
  if (wrapped?.client !== session.client) {
    const onTurnEnd = () => {
      // Yalnızca üst düzey oturum (alt ajanlar agent aracını alamaz) ve dosya değiştiyse.
      if (session.agents.length === 0 || !link.filesChanged) return
      link.filesChanged = false
      void autoVerify({ root: ctx.root, permissions: session.permissions, owner: session.shells, notify: link.notify, signal: session.lifetime.signal }).catch(
        (err: unknown) => console.error('autoVerify hatası:', err instanceof Error ? err.message : err),
      )
    }
    session.client = guardClient(unwrapClient(session.client), session, getState, onTurnEnd)
  }
  wrappedFor.set(session, { tools: session.tools, client: session.client })
  return false
}
