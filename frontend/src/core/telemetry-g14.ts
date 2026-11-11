// [g14] Gözlemlenebilirlik çekirdeği: tek olay veriyolu ve ölçüm kaydı.
// agent.ts'de g14 için yalnızca "model çağrısı öncesi" işareti var; model çağrısı sonu ve araç olayları,
// turun io nesnesinin yöntemleri bir kez (yerinde) sarılarak gözlenir. Sarmalayıcılar önce kaydı alır,
// sonra özgün yöntemi aynen çağırır; kayıt hatası ajanı asla durdurmaz.
import { randomBytes } from 'node:crypto'
import path from 'node:path'
import type { AgentIO, Session, SubagentInfo } from './agent.ts'
import { estimateMessageTokens, estimateTokens, type ContextInfo } from './context.ts'
import {
  costOf,
  scaleSplit,
  type LedgerEntry,
  type ModelCallRecord,
  type PriceLookup,
  type SpanRecord,
  type SpanStatus,
  type TimelineStep,
  type ToolCallRecord,
  type TokenSplit,
  type TurnRecord,
} from './metrics-g14.ts'
import { maskSecrets } from './secrets.ts'
import type { Tool, ToolContext } from './tools.ts'

export type TelemetryEventKind = 'turn' | 'model' | 'tool' | 'subagent' | 'alert' | 'compact'

/** Olay akışı satırı (kronolojik, filtrelenebilir). */
export interface TelemetryEvent {
  id: number
  at: number
  sessionId: string
  traceId: string
  kind: TelemetryEventKind
  phase: 'start' | 'end' | 'info'
  label: string
  status?: SpanStatus
  durationMs?: number
}

/** Ham istek/yanıt denetçisi kaydı; metinler maskeli ve kısaltılmış JSON. */
export interface RawExchange {
  id: string
  at: number
  sessionId: string
  model: string
  status: SpanStatus
  durationMs: number
  request: string
  response: string
}

export type TelemetryNotice =
  | { type: 'turn'; turn: TurnRecord; spans: SpanRecord[] }
  | { type: 'model'; call: ModelCallRecord; ledger: LedgerEntry }
  | { type: 'tool'; call: ToolCallRecord }
  | { type: 'event'; event: TelemetryEvent }

interface TelemetryConfig {
  slowToolMs: number
  price: PriceLookup
  tagsOf: (sessionId: string) => readonly string[]
}

const LIMITS = { turns: 300, models: 500, tools: 3000, events: 1500, raw: 20 } as const
const MAX_SUMMARY_CHARS = 200
const RAW_MESSAGES = 12
const RAW_CONTENT_CHARS = 2000

const config: TelemetryConfig = { slowToolMs: 30_000, price: () => null, tagsOf: () => [] }

/** Bellekteki son kayıtlar (halka tamponlar); panolar ve Prometheus buradan okur. */
export const store = {
  turns: [] as TurnRecord[],
  models: [] as ModelCallRecord[],
  tools: [] as ToolCallRecord[],
  events: [] as TelemetryEvent[],
  raw: [] as RawExchange[],
}

/** Prometheus sayaçları: halka tamponlar taşsa da tekdüze artar. */
const counters = new Map<string, { name: string; labels: Record<string, string>; value: number }>()
const listeners = new Set<(notice: TelemetryNotice) => void>()
let eventSeq = 0

export function configureTelemetry(next: Partial<TelemetryConfig>): void {
  Object.assign(config, next)
}

export function telemetryConfig(): Readonly<TelemetryConfig> {
  return config
}

export function onTelemetry(listener: (notice: TelemetryNotice) => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function counterSamples(): ReadonlyArray<{ name: string; labels: Record<string, string>; value: number }> {
  return [...counters.values()]
}

/** Testler için: tüm kayıtları ve sayaçları sıfırlar. */
export function resetTelemetry(): void {
  for (const list of Object.values(store)) list.length = 0
  counters.clear()
  liveTrackers.clear()
  eventSeq = 0
}

function push<T>(list: T[], item: T, limit: number): void {
  list.push(item)
  if (list.length > limit) list.splice(0, list.length - limit)
}

function inc(name: string, labels: Record<string, string>, by = 1): void {
  const key = `${name}${JSON.stringify(labels)}`
  const c = counters.get(key)
  if (c) c.value += by
  else counters.set(key, { name, labels, value: by })
}

function notify(notice: TelemetryNotice): void {
  for (const l of listeners) {
    try {
      l(notice)
    } catch (err) {
      console.error('Gözlem dinleyicisi hata verdi:', err instanceof Error ? err.message : err)
    }
  }
}

function event(e: Omit<TelemetryEvent, 'id' | 'at'>): void {
  const full: TelemetryEvent = { id: ++eventSeq, at: Date.now(), ...e }
  push(store.events, full, LIMITS.events)
  notify({ type: 'event', event: full })
}

/** Uyarılar (bütçe, yavaş araç) da olay akışında görünür. */
export function recordAlert(sessionId: string, label: string): void {
  event({ sessionId, traceId: '', kind: 'alert', phase: 'info', label })
}

/** Sıkıştırma olayı: sayaç + iz. Başarısızlık ayrı etiketlenir (operatör hangi modelin patladığını görsün). */
export function recordCompaction(sessionId: string, model: string, before: number, after: number, ok: boolean): void {
  inc('compaction_total', { model, ok: ok ? 'true' : 'false' })
  inc('compaction_tokens_saved', { model }, Math.max(0, before - after))
  if (ok) event({ sessionId, traceId: '', kind: 'compact', phase: 'info', label: `compact ${before}→${after}` })
  else recordAlert(sessionId, 'compact başarısız')
}

const hex = (bytes: number) => randomBytes(bytes).toString('hex')
const short = (text: string, max: number) => (text.length > max ? `${text.slice(0, max)}… [${text.length} karakter]` : text)

// ---------- İzleyici (io başına) ----------

interface Link {
  traceId: string
  parentSpanId: string
  agentType: string
  callId: string
  background: boolean
  /** Kayıtlar ve etiketler üst düzey sohbete atfedilir */
  rootSessionId: string
}

interface OpenModel {
  spanId: string
  model: string
  start: number
  firstTokenAt: number | null
  thinkingStart: number | null
  thinkingEnd: number | null
  contentStart: number | null
  promptBase: number
  cachedBase: number
  completionBase: number
  split: TokenSplit
  request: string
  reasoning: string
}

interface OpenTool {
  spanId: string
  name: string
  summary: string
  start: number
}

interface OpenTurn {
  traceId: string
  spanId: string
  parentSpanId: string | null
  subagent: string | null
  project: string
  model: string
  start: number
  steps: TimelineStep[]
  spans: SpanRecord[]
  tokens: TurnRecord['tokens']
  costUsd: number | null
  modelCalls: number
  toolCalls: number
}

interface Tracker {
  session: Session
  link: Link | undefined
  turn: OpenTurn | null
  model: OpenModel | null
  tools: Map<string, OpenTool>
  signal: AbortSignal | undefined
  onAbort: (() => void) | null
}

const trackers = new WeakMap<AgentIO, Tracker>()
const childLinks = new WeakMap<AgentIO, Link>()
/** Açık turu olan izleyiciler; tur kapanınca çıkarılır (sızıntı olmasın). */
const liveTrackers = new Set<Tracker>()
const toolDefsCache = new WeakMap<readonly Tool[], number>()

const rootOf = (t: Tracker) => t.link?.rootSessionId ?? t.session.id

function safe(fn: () => void): void {
  try {
    fn()
  } catch (err) {
    console.error('Gözlem kaydı başarısız:', err instanceof Error ? err.message : err)
  }
}

function providerOf(session: Session): string {
  // Testlerdeki sahte istemcilerde baseURL yoktur.
  const base: unknown = session.client.baseURL
  if (typeof base !== 'string') return 'bilinmiyor'
  try {
    return new URL(base).hostname
  } catch {
    return 'bilinmiyor'
  }
}

function toolDefsTokens(tools: readonly Tool[]): number {
  let cached = toolDefsCache.get(tools)
  if (cached === undefined) {
    cached = estimateTokens(JSON.stringify(tools.map((t) => [t.name, t.description, t.parameters])))
    toolDefsCache.set(tools, cached)
  }
  return cached
}

function estimateSplit(session: Session): TokenSplit {
  const split: TokenSplit = { system: 0, toolDefs: toolDefsTokens(session.tools), toolResults: 0, history: 0 }
  session.messages.forEach((m, i) => {
    const tokens = estimateMessageTokens(m)
    if (i === 0 && m.role === 'system') split.system += tokens
    else if (m.role === 'tool') split.toolResults += tokens
    else split.history += tokens
  })
  return split
}

function rawRequest(session: Session): string {
  const tail = session.messages.slice(-RAW_MESSAGES).map((m) => ({
    role: m.role,
    content: typeof m.content === 'string' ? short(m.content, RAW_CONTENT_CHARS) : Array.isArray(m.content) ? '[çok parçalı içerik]' : null,
    ...(m.role === 'assistant' && m.tool_calls
      ? { tool_calls: m.tool_calls.map((c) => (c.type === 'function' ? { name: c.function.name, arguments: short(c.function.arguments, 1000) } : { type: c.type })) }
      : {}),
    ...(m.role === 'tool' ? { tool_call_id: m.tool_call_id } : {}),
  }))
  const base: unknown = session.client.baseURL
  return maskSecrets(
    JSON.stringify(
      {
        not: 'Oturum durumundan yeniden oluşturuldu; diğer modüllerin eklediği istek alanları gösterilmeyebilir.',
        method: 'POST',
        url: `${typeof base === 'string' ? base : ''}/chat/completions`,
        headers: { Authorization: 'Bearer [maskelendi]', 'Content-Type': 'application/json' },
        body: {
          model: session.model,
          stream: true,
          messages_omitted: Math.max(0, session.messages.length - RAW_MESSAGES),
          messages: tail,
          tools: session.tools.map((t) => t.name),
        },
      },
      null,
      2,
    ),
  )
}

function trackerFor(session: Session, io: AgentIO): Tracker {
  let t = trackers.get(io)
  // CLI aynı io'yu yeni oturumla kullanabilir; izleyici oturuma bağlıdır.
  if (!t || t.session !== session) {
    if (t?.turn) finishTurn(t, 'error', 'Oturum değişti')
    t = { session, link: childLinks.get(io), turn: null, model: null, tools: new Map(), signal: undefined, onAbort: null }
    const first = !trackers.has(io)
    trackers.set(io, t)
    if (first) wrapIo(io)
  }
  return t
}

function wrapIo(io: AgentIO): void {
  const { toolStart, toolEnd, write, reasoning, contextUpdate, subagentStarted, subagentFinished } = io
  const current = () => trackers.get(io)
  io.toolStart = (callId, name, summary) => {
    safe(() => onToolStart(current(), callId, name, summary))
    toolStart.call(io, callId, name, summary)
  }
  io.toolEnd = (callId, result) => {
    safe(() => onToolEnd(current(), callId, result))
    toolEnd.call(io, callId, result)
  }
  io.write = (text) => {
    safe(() => onContent(current()))
    write.call(io, text)
  }
  io.reasoning = (delta) => {
    safe(() => onReasoning(current(), delta))
    reasoning?.call(io, delta)
  }
  io.contextUpdate = (info) => {
    safe(() => endModel(current(), 'ok', undefined, info))
    contextUpdate?.call(io, info)
  }
  if (subagentStarted) {
    io.subagentStarted = (callId: string, info: SubagentInfo) => {
      const child = subagentStarted.call(io, callId, info)
      safe(() => {
        const t = current()
        if (child === io || !t?.turn) return
        childLinks.set(child, {
          traceId: t.turn.traceId,
          parentSpanId: t.tools.get(callId)?.spanId ?? t.turn.spanId,
          agentType: info.type,
          callId,
          background: info.background,
          rootSessionId: rootOf(t),
        })
      })
      return child
    }
  }
  if (subagentFinished) {
    io.subagentFinished = (callId, result, ok) => {
      safe(() => finishChildren(callId, ok ? 'ok' : 'error', ok ? undefined : short(maskSecrets(result), MAX_SUMMARY_CHARS)))
      subagentFinished.call(io, callId, result, ok)
    }
  }
}

/** [g14] agent.ts "model çağrısı öncesi" kancası: her adımda bir model çağrısı izi açar; adım 0 yeni tur demektir. */
export function g14BeforeModelCall(session: Session, io: AgentIO, step: number, ctx: ToolContext): void {
  safe(() => {
    // Durdurulan tur kapandıktan sonra döngü bir adım daha dönebilir; yeni iz açılmaz.
    if (ctx.signal.aborted) return
    const t = trackerFor(session, io)
    if (step === 0 || !t.turn) beginTurn(t, ctx)
    beginModel(t)
  })
}

function beginTurn(t: Tracker, ctx: ToolContext): void {
  if (t.turn) finishTurn(t, 'error', 'Tur tamamlanmadan yenisi başladı')
  const project = path.basename(ctx.root) || ctx.root
  t.turn = {
    traceId: t.link?.traceId ?? hex(16),
    spanId: hex(8),
    parentSpanId: t.link?.parentSpanId ?? null,
    subagent: t.link?.agentType ?? null,
    project,
    model: t.session.model,
    start: Date.now(),
    steps: [],
    spans: [],
    tokens: { system: 0, toolDefs: 0, toolResults: 0, history: 0, completion: 0, cached: 0 },
    costUsd: null,
    modelCalls: 0,
    toolCalls: 0,
  }
  t.signal = ctx.signal
  // Kullanıcı durdurunca tur hemen "iptal" kapanır; sonradan gelen araç sonuçları yok sayılır.
  const onAbort = () => safe(() => finishTurn(t, 'cancelled', 'Kullanıcı durdurdu'))
  ctx.signal.addEventListener('abort', onAbort, { once: true })
  t.onAbort = onAbort
  liveTrackers.add(t)
  event({ sessionId: rootOf(t), traceId: t.turn.traceId, kind: t.link ? 'subagent' : 'turn', phase: 'start', label: t.link ? `Alt ajan: ${t.link.agentType}` : `Tur başladı (${project})` })
}

function beginModel(t: Tracker): void {
  if (!t.turn) return
  if (t.model) endModel(t, 'error', 'Yanıt alınamadı')
  const s = t.session
  t.model = {
    spanId: hex(8),
    model: s.model,
    start: Date.now(),
    firstTokenAt: null,
    thinkingStart: null,
    thinkingEnd: null,
    contentStart: null,
    promptBase: s.contextState?.promptTokens ?? 0,
    cachedBase: s.contextState?.cachedTokens ?? 0,
    completionBase: s.usage.get(s.model)?.completionTokens ?? 0,
    split: estimateSplit(s),
    request: rawRequest(s),
    reasoning: '',
  }
  event({ sessionId: rootOf(t), traceId: t.turn.traceId, kind: 'model', phase: 'start', label: `Model çağrısı: ${s.model}` })
}

function onContent(t: Tracker | undefined): void {
  const m = t?.model
  if (!m || m.contentStart !== null) return
  const now = Date.now()
  m.contentStart = now
  m.firstTokenAt ??= now
  if (m.thinkingStart !== null) m.thinkingEnd ??= now
}

function onReasoning(t: Tracker | undefined, delta: string): void {
  const m = t?.model
  if (!m) return
  const now = Date.now()
  m.firstTokenAt ??= now
  m.thinkingStart ??= now
  if (m.contentStart === null) m.thinkingEnd = now
  if (m.reasoning.length < RAW_CONTENT_CHARS) m.reasoning += delta
}

function endModel(t: Tracker | undefined, status: SpanStatus, message?: string, info?: ContextInfo): void {
  const m = t?.model
  const turn = t?.turn
  if (!t || !m || !turn) return
  t.model = null
  const s = t.session
  const sessionId = rootOf(t)
  const end = Date.now()
  const promptTokens = info ? Math.max(0, info.promptTokens - m.promptBase) : 0
  const cachedTokens = info ? Math.max(0, info.cachedTokens - m.cachedBase) : 0
  // Alt ajanlar kullanım haritasını paylaşır; eşzamanlı aynı modelli çağrılarda bu fark yaklaşık kalır.
  const completionTokens = status === 'ok' ? Math.max(0, (s.usage.get(m.model)?.completionTokens ?? 0) - m.completionBase) : 0
  const split = scaleSplit(m.split, promptTokens)
  const record: ModelCallRecord = {
    id: m.spanId,
    traceId: turn.traceId,
    sessionId,
    model: m.model,
    provider: providerOf(s),
    start: m.start,
    firstTokenAt: m.firstTokenAt,
    end,
    status,
    ...(message ? { message } : {}),
    promptTokens,
    completionTokens,
    cachedTokens,
    split,
  }
  push(store.models, record, LIMITS.models)
  inc('kor_model_requests_total', { model: m.model, status })
  inc('kor_tokens_total', { model: m.model, type: 'prompt' }, promptTokens)
  inc('kor_tokens_total', { model: m.model, type: 'completion' }, completionTokens)
  inc('kor_tokens_total', { model: m.model, type: 'cached' }, cachedTokens)

  turn.model = m.model
  turn.modelCalls++
  turn.tokens.system += split.system
  turn.tokens.toolDefs += split.toolDefs
  turn.tokens.toolResults += split.toolResults
  turn.tokens.history += split.history
  turn.tokens.completion += completionTokens
  turn.tokens.cached += cachedTokens
  const cost = costOf(m.model, promptTokens, completionTokens, config.price)
  if (cost !== null) turn.costUsd = (turn.costUsd ?? 0) + cost

  const firstToken = m.firstTokenAt ?? end
  turn.steps.push({ kind: 'wait', label: 'Model bekleniyor', start: m.start, end: firstToken, status })
  if (m.thinkingStart !== null) turn.steps.push({ kind: 'thinking', label: 'Düşünme', start: m.thinkingStart, end: m.thinkingEnd ?? end, status })
  if (m.contentStart !== null) turn.steps.push({ kind: 'response', label: 'Yanıt', start: m.contentStart, end, status })
  turn.spans.push({
    traceId: turn.traceId,
    spanId: m.spanId,
    parentSpanId: turn.spanId,
    kind: 'model',
    name: `chat ${m.model}`,
    start: m.start,
    end,
    status,
    ...(message ? { message } : {}),
    attrs: {
      'gen_ai.operation.name': 'chat',
      'gen_ai.provider.name': record.provider,
      'gen_ai.request.model': m.model,
      'gen_ai.usage.input_tokens': promptTokens,
      'gen_ai.usage.output_tokens': completionTokens,
      'gen_ai.usage.cached_input_tokens': cachedTokens,
      'kor.session.id': sessionId,
    },
  })
  event({ sessionId, traceId: turn.traceId, kind: 'model', phase: 'end', label: `Model yanıtı: ${m.model}`, status, durationMs: end - m.start })

  const traceId = turn.traceId
  const project = turn.project
  // Yanıt mesajı agent.ts'te bu çağrıdan hemen sonra eklenir; eşzamanlı akış bitince okunur.
  queueMicrotask(() =>
    safe(() => {
      const last = s.messages.at(-1)
      const reply = last?.role === 'assistant' && status === 'ok' ? last : null
      const calls = reply?.tool_calls ?? []
      const tools = calls.flatMap((c) => (c.type === 'function' ? [c.function.name] : []))
      const ledger: LedgerEntry = {
        at: end,
        sessionId,
        traceId,
        project,
        model: m.model,
        promptTokens,
        completionTokens,
        cachedTokens,
        costUsd: cost,
        tools,
        tags: [...config.tagsOf(sessionId)],
        ok: status === 'ok',
      }
      push(store.raw, {
        id: m.spanId,
        at: m.start,
        sessionId,
        model: m.model,
        status,
        durationMs: end - m.start,
        request: m.request,
        response: maskSecrets(
          JSON.stringify(
            status === 'ok'
              ? {
                  usage: { prompt_tokens: promptTokens, completion_tokens: completionTokens, cached_tokens: cachedTokens },
                  message: reply
                    ? {
                        role: 'assistant',
                        content: typeof reply.content === 'string' ? short(reply.content, RAW_CONTENT_CHARS * 2) : null,
                        tool_calls: calls.map((c) => (c.type === 'function' ? { name: c.function.name, arguments: short(c.function.arguments, 1000) } : { type: c.type })),
                      }
                    : null,
                  ...(m.reasoning ? { reasoning: short(m.reasoning, RAW_CONTENT_CHARS) } : {}),
                }
              : { error: message ?? 'Yanıt alınamadı' },
            null,
            2,
          ),
        ),
      }, LIMITS.raw)
      notify({ type: 'model', call: record, ledger })
      // Araç istemeyen yanıt, bekleyen bildirim yoksa turun sonudur.
      if (reply && calls.length === 0 && t.turn?.traceId === traceId && !t.model && t.tools.size === 0 && s.inbox.length === 0) finishTurn(t, 'ok')
    }),
  )
}

function onToolStart(t: Tracker | undefined, callId: string, name: string, summary: string): void {
  if (!t?.turn) return
  t.turn.toolCalls++
  t.tools.set(callId, { spanId: hex(8), name, summary: short(maskSecrets(summary), MAX_SUMMARY_CHARS), start: Date.now() })
  event({ sessionId: rootOf(t), traceId: t.turn.traceId, kind: 'tool', phase: 'start', label: `${name}: ${t.tools.get(callId)?.summary ?? ''}` })
}

/** agent.ts araç sonucunu bu öneklerle döner (executeToolCall). */
export function toolStatusOf(result: string, aborted: boolean): SpanStatus {
  if (aborted) return 'cancelled'
  if (result.startsWith('Engellendi:') || result.startsWith('Kullanıcı bu işlemi reddetti')) return 'denied'
  if (result.startsWith('Hata:')) return 'error'
  return 'ok'
}

function onToolEnd(t: Tracker | undefined, callId: string, result: string): void {
  const tool = t?.tools.get(callId)
  if (!t?.turn || !tool) return
  t.tools.delete(callId)
  const status = toolStatusOf(result, t.signal?.aborted === true)
  closeTool(t, callId, tool, status, result)
  // Ön plan alt ajanı hata/sınırla biterse kendi turu açık kalabilir; arka plandakiler subagentFinished ile kapanır.
  finishChildren(callId, status === 'ok' ? 'ok' : 'error', status === 'ok' ? undefined : 'Alt ajan turu tamamlanmadı', false)
}

function closeTool(t: Tracker, callId: string, tool: OpenTool, status: SpanStatus, result: string): void {
  const turn = t.turn
  if (!turn) return
  const end = Date.now()
  const slow = end - tool.start >= config.slowToolMs
  const record: ToolCallRecord = { id: callId, traceId: turn.traceId, sessionId: rootOf(t), name: tool.name, summary: tool.summary, start: tool.start, end, status, slow }
  push(store.tools, record, LIMITS.tools)
  inc('kor_tool_calls_total', { tool: tool.name, status })
  turn.steps.push({ kind: 'tool', label: tool.name, start: tool.start, end, status })
  const message = status === 'ok' ? undefined : short(maskSecrets(result), MAX_SUMMARY_CHARS)
  turn.spans.push({
    traceId: turn.traceId,
    spanId: tool.spanId,
    parentSpanId: turn.spanId,
    kind: 'tool',
    name: `execute_tool ${tool.name}`,
    start: tool.start,
    end,
    status,
    ...(message ? { message } : {}),
    attrs: { 'gen_ai.operation.name': 'execute_tool', 'gen_ai.tool.name': tool.name, 'gen_ai.tool.call.id': callId, 'kor.tool.summary': tool.summary, 'kor.tool.slow': slow },
  })
  event({ sessionId: rootOf(t), traceId: turn.traceId, kind: 'tool', phase: 'end', label: tool.name, status, durationMs: end - tool.start })
  notify({ type: 'tool', call: record })
}

function finishChildren(callId: string, status: SpanStatus, message?: string, background?: boolean): void {
  for (const child of [...liveTrackers]) {
    if (child.link?.callId === callId && (background === undefined || child.link.background === background)) finishTurn(child, status, message)
  }
}

function finishTurn(t: Tracker, status: SpanStatus, message?: string): void {
  const turn = t.turn
  if (!turn) return
  if (t.model) endModel(t, status === 'cancelled' ? 'cancelled' : 'error', message ?? 'Yanıt alınamadı')
  for (const [callId, tool] of t.tools) closeTool(t, callId, tool, 'cancelled', message ?? 'Tur kapandı')
  t.tools.clear()
  if (t.onAbort) t.signal?.removeEventListener('abort', t.onAbort)
  t.onAbort = null
  t.turn = null
  liveTrackers.delete(t)

  const end = Date.now()
  const record: TurnRecord = {
    traceId: turn.traceId,
    spanId: turn.spanId,
    parentSpanId: turn.parentSpanId,
    sessionId: rootOf(t),
    subagent: turn.subagent,
    project: turn.project,
    model: turn.model,
    start: turn.start,
    end,
    status,
    ...(message ? { message } : {}),
    steps: turn.steps.sort((a, b) => a.start - b.start),
    tokens: turn.tokens,
    costUsd: turn.costUsd,
    modelCalls: turn.modelCalls,
    toolCalls: turn.toolCalls,
  }
  push(store.turns, record, LIMITS.turns)
  inc('kor_turns_total', { status })
  const spans: SpanRecord[] = [
    {
      traceId: turn.traceId,
      spanId: turn.spanId,
      parentSpanId: turn.parentSpanId,
      kind: 'turn',
      name: turn.subagent ? `invoke_agent ${turn.subagent}` : 'invoke_agent kor',
      start: turn.start,
      end,
      status,
      ...(message ? { message } : {}),
      attrs: {
        'gen_ai.operation.name': 'invoke_agent',
        'gen_ai.agent.name': turn.subagent ?? 'kor',
        'kor.session.id': rootOf(t),
        'kor.project': turn.project,
        'kor.model_calls': turn.modelCalls,
        'kor.tool_calls': turn.toolCalls,
      },
    },
    ...turn.spans,
  ]
  event({ sessionId: rootOf(t), traceId: turn.traceId, kind: turn.subagent ? 'subagent' : 'turn', phase: 'end', label: turn.subagent ? `Alt ajan bitti: ${turn.subagent}` : 'Tur bitti', status, durationMs: end - turn.start })
  notify({ type: 'turn', turn: record, spans })
}

export interface LiveTurn {
  sessionId: string
  traceId: string
  start: number
  subagent: string | null
}

export function liveTurns(): LiveTurn[] {
  return [...liveTrackers].flatMap((t) => (t.turn ? [{ sessionId: rootOf(t), traceId: t.turn.traceId, start: t.turn.start, subagent: t.turn.subagent }] : []))
}

/**
 * Hata ile biten turları kapatır: runTurn hata fırlatınca io'ya olay gelmez. isRunning, üst düzey oturum için
 * çalışma durumunu verir (alt ajan oturumları için undefined). İptal edilen sinyaller de burada kapanır.
 */
export function sweepTelemetry(isRunning: (session: Session) => boolean | undefined): void {
  for (const t of [...liveTrackers]) {
    if (t.signal?.aborted) finishTurn(t, 'cancelled', 'Durduruldu')
    else if (isRunning(t.session) === false) {
      if (t.model) finishTurn(t, 'error', 'Sağlayıcı çağrısı başarısız (ayrıntı sohbette)')
      else finishTurn(t, 'ok')
    }
  }
}
