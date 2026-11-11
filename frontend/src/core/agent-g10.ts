// [g10] runTurn'deki "[g10] model çağrısı öncesi" kancası: araç korumasını ve akış dayanıklılığını kurar,
// tur günlüğünü yazar, bozuk argüman hatalarını ayrıntılandırır ve başarısız arka plan görevlerini ölü mektup kuyruğuna alır.
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import type { AgentIO, Session } from './agent.ts'
import { compactSession } from './compaction.ts'
import { RetryBudget, type G10Event } from './resilience-g10.ts'
import { maskSecrets } from './secrets.ts'
import { loadUserSettingsFile } from './settings.ts'
import type { ShellRegistry } from './shells.ts'
import { onProviderClient } from './provider-pool-g9.ts'
import { originalMessagesOf } from './tool-prune.ts'
import { installStreamResilience, type SessionStreamHooks } from './stream-g10.ts'
import { DEFAULT_TOOL_TIMEOUT_MS, guardTools, type GuardEnv } from './tool-guard-g10.ts'
import { tools as coreTools, type ToolContext } from './tools.ts'
import { addDeadLetter, completeGroups, fingerprint, writeJournal, type TurnJournal } from './turn-journal-g10.ts'

export type { G10Event }

/** Tur başına yeniden deneme bütçesi: araç ve akış yeniden denemeleri birlikte (sonsuz tekrar koruması). */
const TURN_RETRIES = 8
const TURN_RETRY_WAIT_MS = 60_000
const INVALID_JSON_PREFIX = 'Hata: araç argümanları geçerli JSON değil'

export interface G10Config {
  toolTimeoutMs: number
  /** 0: kapalı. Yalnızca fallbackModel ile birlikte etkilidir. */
  firstTokenTimeoutMs: number
  fallbackModel: string | null
}

export const DEFAULT_G10_CONFIG: G10Config = { toolTimeoutMs: DEFAULT_TOOL_TIMEOUT_MS, firstTokenTimeoutMs: 0, fallbackModel: null }

export interface G10SessionState {
  io: AgentIO | null
  budget: RetryBudget
  config: G10Config
  /** Turun ilk mesajı (nesne kimliği); sıkıştırma onu özetlerse günlük sistem mesajından sonrasını tutar */
  turnStart: ChatCompletionMessageParam | null
  prevFingerprint: string | null
  startedAt: number
  root: string
  applied: string[]
  invalidArgs: Map<string, string>
  hooks: SessionStreamHooks | null
}

function stateOf(session: Session): G10SessionState {
  session.g10 ??= {
    io: null,
    budget: new RetryBudget(TURN_RETRIES, TURN_RETRY_WAIT_MS),
    config: DEFAULT_G10_CONFIG,
    turnStart: null,
    prevFingerprint: null,
    startedAt: Date.now(),
    root: '',
    applied: [],
    invalidArgs: new Map(),
    hooks: null,
  }
  return session.g10
}

/** Maliyet doğurabilecek yedek model yalnızca kullanıcı ayar dosyasından okunur (proje dosyası açamaz). */
export async function loadG10Config(): Promise<G10Config> {
  try {
    const g = (await loadUserSettingsFile()).g10 ?? {}
    return {
      toolTimeoutMs: (g.toolTimeoutSec ?? DEFAULT_TOOL_TIMEOUT_MS / 1000) * 1000,
      firstTokenTimeoutMs: (g.firstTokenTimeoutSec ?? 0) * 1000,
      fallbackModel: g.fallbackModel ?? null,
    }
  } catch (err) {
    console.warn('g10 ayarları okunamadı, varsayılanlar kullanılıyor:', err instanceof Error ? err.message : err)
    return DEFAULT_G10_CONFIG
  }
}

const isTopLevel = (session: Session) => session.agents.length > 0

// ---------- Akış kancaları ----------

const hooksByMessages = new WeakMap<object, SessionStreamHooks>()
const streamLookup = (messages: unknown): SessionStreamHooks | null =>
  // İstek gövdesi budanmış/metin protokolü kopyası olabilir: kancalar asıl oturum dizisine bağlıdır.
  typeof messages === 'object' && messages !== null ? (hooksByMessages.get(originalMessagesOf(messages as ChatCompletionMessageParam[])) ?? null) : null
let providerClientsHooked = false

function streamHooks(session: Session, s: G10SessionState): SessionStreamHooks {
  return {
    emit: (event) => s.io?.g10Event?.(event),
    notice: (text) => s.io?.notice?.(text),
    get budget() {
      return s.budget
    },
    get firstTokenTimeoutMs() {
      return s.config.firstTokenTimeoutMs
    },
    get fallbackModel() {
      return s.config.fallbackModel
    },
    compact: async (signal) => {
      try {
        const result = await compactSession(session, { ...(s.io ? { io: s.io } : {}), ...(signal ? { signal } : {}) })
        return result.compacted
      } catch (err) {
        console.error('Bağlam taşmasında sıkıştırma başarısız:', err instanceof Error ? err.message : err)
        return false
      }
    },
    invalidArgs: (id, reason) => {
      s.invalidArgs.set(id, reason)
    },
  }
}

// ---------- Araç koruması ortamı ----------

/** Üst oturum ve alt ajanları aynı ShellRegistry'yi paylaşır; araç bağlamından oturum durumuna bu yolla ulaşılır. */
const envByShells = new WeakMap<ShellRegistry, { state: G10SessionState; journal: () => Promise<void> }>()
const fallbackEnv: GuardEnv = {
  emit: () => undefined,
  budget: new RetryBudget(TURN_RETRIES, TURN_RETRY_WAIT_MS),
  toolTimeoutMs: DEFAULT_TOOL_TIMEOUT_MS,
  noteApplied: () => undefined,
}

function envFor(ctx: ToolContext): GuardEnv {
  const entry = ctx.shells ? envByShells.get(ctx.shells) : undefined
  if (!entry) return fallbackEnv
  const { state, journal } = entry
  return {
    emit: (event) => state.io?.g10Event?.(event),
    budget: state.budget,
    toolTimeoutMs: state.config.toolTimeoutMs,
    noteApplied: (summary) => {
      state.applied.push(summary)
      void journal()
    },
  }
}

// ---------- Tur günlüğü ----------

function turnMessages(session: Session, s: G10SessionState): { messages: ChatCompletionMessageParam[]; fromStart: boolean } {
  const index = s.turnStart ? session.messages.indexOf(s.turnStart) : -1
  const fromStart = index <= 0
  return { messages: completeGroups(session.messages.slice(fromStart ? 1 : index)), fromStart }
}

const journalWrites = new WeakMap<Session, { running: Promise<void>; pending: TurnJournal | null }>()

/**
 * Anlık görüntü eşzamanlı alınır, yazma arka planda yapılır: model çağrısı disk yazmasını beklemez.
 * Yazma sürerken gelen görüntüler birleşir: yalnızca en yenisi serileştirilir (ara adımlar boşuna yazılmaz).
 */
function persistJournal(session: Session, s: G10SessionState): Promise<void> {
  const { messages, fromStart } = turnMessages(session, s)
  if (messages.length === 0) return Promise.resolve()
  const snapshot = {
    version: 1 as const,
    sessionId: session.id,
    root: s.root,
    startedAt: s.startedAt,
    updatedAt: Date.now(),
    prevFingerprint: s.prevFingerprint,
    fromStart,
    messages,
    applied: s.applied.slice(-200),
  }
  const active = journalWrites.get(session)
  if (active) {
    active.pending = snapshot
    return active.running
  }
  const entry: { running: Promise<void>; pending: TurnJournal | null } = { running: Promise.resolve(), pending: snapshot }
  journalWrites.set(session, entry)
  entry.running = (async () => {
    await Promise.resolve() // ilk yazma bu adımı bloklamasın
    while (entry.pending) {
      const next = entry.pending
      entry.pending = null
      await writeJournal(next).catch((err: unknown) => console.error('Tur günlüğü yazılamadı:', err instanceof Error ? err.message : err))
    }
    journalWrites.delete(session)
  })()
  return entry.running
}

/** Testler ve kurtarma işleyicisi için: sıradaki günlük yazmaları bitene kadar bekler. */
export function journalFlushed(session: Session): Promise<void> {
  return journalWrites.get(session)?.running ?? Promise.resolve()
}

function markTurnStart(session: Session, s: G10SessionState): void {
  // Turun başı: sondaki kullanıcı rolü bloğu (kullanıcı mesajı, bildirimler, kuyruktaki mesajlar).
  let i = session.messages.length
  while (i > 1 && session.messages[i - 1]?.role === 'user') i--
  s.turnStart = session.messages[i] ?? null
  s.prevFingerprint = i - 1 <= 0 ? null : fingerprint(session.messages[i - 1])
  s.startedAt = Date.now()
}

/** g5, uzunluk sınırında kesilen çağrıyı atar; diğer bozuk argümanlar için ajanın genel hatasına neden eklenir. */
function annotateInvalidArgs(session: Session, s: G10SessionState): void {
  if (s.invalidArgs.size === 0) return
  for (const m of session.messages.slice(-50)) {
    if (m.role !== 'tool' || typeof m.content !== 'string' || !m.content.startsWith(INVALID_JSON_PREFIX)) continue
    const reason = s.invalidArgs.get(m.tool_call_id)
    if (reason) m.content += ` Ayrıntı: ${reason}`
  }
  s.invalidArgs.clear()
}

// ---------- Ölü mektup kuyruğu ----------

const wrappedIo = new WeakSet<AgentIO>()
const seenShells = new WeakMap<ShellRegistry, Set<string>>()

function recordDeadLetter(session: Session, s: G10SessionState, kind: 'subagent' | 'shell', title: string, error: string): void {
  const safeTitle = maskSecrets(title)
  addDeadLetter({ sessionId: session.id, kind, title: safeTitle, error: maskSecrets(error) })
    .then(() => s.io?.g10Event?.({ kind: 'deadLetter', title: safeTitle }))
    .catch((err: unknown) => console.error('Ölü mektup kaydedilemedi:', err instanceof Error ? err.message : err))
}

/**
 * Arka plan alt ajanı başarısızlığı yalnızca AgentIO.subagentFinished ile bildirilir; çekirdekte başka kanca olmadığından
 * tur io'sunun bu iki yöntemi bir kez sarılır (davranış aynen korunur, yalnızca kayıt eklenir).
 */
function captureSubagentFailures(session: Session, s: G10SessionState, io: AgentIO): void {
  if (wrappedIo.has(io)) return
  wrappedIo.add(io)
  const started = io.subagentStarted?.bind(io)
  const finished = io.subagentFinished?.bind(io)
  if (!finished) return
  const titles = new Map<string, string>()
  if (started) {
    io.subagentStarted = (callId, info) => {
      if (info.background) titles.set(callId, `${info.type}: ${info.description}`)
      return started(callId, info)
    }
  }
  io.subagentFinished = (callId, result, ok) => {
    if (!ok) recordDeadLetter(session, s, 'subagent', titles.get(callId) ?? `alt ajan ${callId.slice(0, 8)}`, result)
    titles.delete(callId)
    finished(callId, result, ok)
  }
}

function captureShellFailures(session: Session, s: G10SessionState): void {
  let seen = seenShells.get(session.shells)
  if (!seen) {
    seen = new Set()
    seenShells.set(session.shells, seen)
  }
  for (const shell of session.shells.list()) {
    if (shell.status !== 'exited' || shell.exitCode === 0 || seen.has(shell.id)) continue
    seen.add(shell.id)
    recordDeadLetter(session, s, 'shell', shell.command, shell.exitCode === null ? 'Süreç bir sinyalle sonlandı' : `Çıkış kodu ${shell.exitCode}`)
  }
}

// ---------- Kanca ----------

export async function g10BeforeModelCall(session: Session, io: AgentIO, ctx: ToolContext, step: number): Promise<void> {
  const s = stateOf(session)
  s.io = io
  s.root = ctx.root
  const top = isTopLevel(session)
  if (step === 0) {
    s.budget = new RetryBudget(TURN_RETRIES, TURN_RETRY_WAIT_MS)
    s.config = await loadG10Config()
    s.invalidArgs.clear()
    markTurnStart(session, s)
    if (top) captureSubagentFailures(session, s, io)
  }

  if (top) envByShells.set(session.shells, { state: s, journal: () => persistJournal(session, s) })
  guardTools(session.tools, coreTools, envFor)

  s.hooks ??= streamHooks(session, s)
  hooksByMessages.set(session.messages, s.hooks)
  installStreamResilience(session.client, streamLookup)
  // [g9 ↔ g10] Venice dışı sağlayıcı istemcileri de aynı dayanıklılığı (yeniden bağlanma, sağlayıcı başına devre kesici) alır.
  if (!providerClientsHooked) {
    providerClientsHooked = true
    onProviderClient((client) => installStreamResilience(client, streamLookup))
  }

  annotateInvalidArgs(session, s)
  if (top) {
    captureShellFailures(session, s)
    s.applied = []
    void persistJournal(session, s)
  }
}
