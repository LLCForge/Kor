import { randomUUID } from 'node:crypto'
import OpenAI from 'openai'
import type {
  ChatCompletionMessageParam,
  ChatCompletionMessageToolCall,
  ChatCompletionTool,
} from 'openai/resources/chat/completions'
import { MAX_CONCURRENT_SUBAGENTS, Semaphore, type AgentDefinition } from './agents.ts'
import { writeAudit, type AuditEntry } from './audit.ts'
import { decide, type PermissionConfig } from './permissions.ts'
import { VENICE_BASE_URL } from './settings.ts'
import { ShellRegistry } from './shells.ts'
import { getDynamicTools } from './dynamic-tools.ts'
import { runPostToolHooks, runPreToolHooks } from './hooks.ts'
import { tools, type SubagentRequest, type Tool, type ToolContext, type ToolPreview } from './tools.ts'
import { isSessionEnded, sessionEndedSync } from './assistant-surface.ts' // [g23]
import { DESKTOP_DEFERRED, deferDesktopTools, deferredByName, isDeferredToolName } from './tools-g23.ts' // [g23]
import { resolveCompatCall, sanitizeToolName } from './tool-compat.ts' // [uyum]
import { noteSubdirToolArgs, takeSubdirHints } from './subdir-hints-g24.ts' // [g24]
import { adversaryVerdict, tightenDecision } from './permission-judge-g26.ts' // [g26]
import { applyPlanAgent } from './prompt-g26.ts' // [g26]

// [g1] importlar

// [g2] importlar
import { g2BeforeModelCall, g2RequestFields, g2RestoreQueue, type ConsumedQueueItem, type G2SessionState, type QueuedMessageInfo } from './user-queue.ts'

// [g3] importlar
import type { InteractBridge } from './interactive.ts'
import { createThinkFilter, noteModelChange, reasoningDelta, stripThink } from './reasoning.ts'

// [g4] importlar

// [g5] importlar
import { g5BeforeModelCall } from './agent-g5.ts'
import { maskSecrets } from './secrets.ts'

// [g6] importlar
import { autoCompact, compactSession, type CompactionEvent } from './compaction.ts'
import { aliasMessages, pruneOldToolOutputs } from './tool-prune.ts' // [perf]
import { afterModelCall, contextLimitOf, rollbackLength, type ContextInfo, type ContextState } from './context.ts'
// Proje haritası ajanları sohbet alt ajanlarıyla aynı eşzamanlılık sınırını paylaşır.
export { subagentSlots }

// [g9] importlar
import { g9BeforeModelCall, g9RequestFields, type TurnCostInfo } from './agent-g9.ts'
import { providerFetch, routeByModel } from './provider-pool-g9.ts'

// [g10] importlar
import { g10BeforeModelCall, type G10Event, type G10SessionState } from './agent-g10.ts'
import { completeGroups } from './turn-journal-g10.ts'

// [g11] importlar
import { g11BeforeModelCall } from './agent-g11.ts'

// [g12] importlar
import { g12BeforeModelCall } from './agent-g12.ts'
import type { BulkDeleteConfirm } from './bulk-delete-g12.ts'

// [g13] importlar
import { g13BeforeModelCall, type G13SessionState } from './prompt-g13.ts'

// [g14] importlar
import { g14BeforeModelCall } from './telemetry-g14.ts'

// [g15] importlar
import { g15BeforeModelCall } from './replay.ts'

// [g16] importlar

// [g18] importlar

// [g19] importlar

// [g20] importlar

// [g21] importlar

// [g22] importlar
import { g22AfterCompletion, g22BeforeModelCall, g22RequestFields, g22RetryAsText, type G22SessionState } from './text-tools-g22.ts'

// [g7] importlar
import { subagentModel, type TaskModels } from './model-catalog.ts'
import type { SessionBridge } from './tools-g7.ts'
import { manualPermissions } from './trust.ts'
import { stripSystemBlock } from './system-blocks.ts'

export interface RateLimitWindow {
  limit: number
  remaining: number
  /** Epoch ms */
  resetAt: number
}

export interface LiveRateLimits {
  requests: RateLimitWindow | null
  tokens: RateLimitWindow | null
  balanceUsd: number | null
  observedAt: number
}

function readWindow(headers: Headers, kind: 'requests' | 'tokens'): RateLimitWindow | null {
  const limit = Number(headers.get(`x-ratelimit-limit-${kind}`))
  const remaining = Number(headers.get(`x-ratelimit-remaining-${kind}`))
  const resetAt = Number(headers.get(`x-ratelimit-reset-${kind}`))
  return limit > 0 && Number.isFinite(remaining) && resetAt > 0 ? { limit, remaining, resetAt } : null
}

/** Venice her yanıtta kalan istek/token ve bakiye bilgisini başlıklarda döner. */
export function parseRateLimitHeaders(headers: Headers): LiveRateLimits | null {
  const requests = readWindow(headers, 'requests')
  const tokens = readWindow(headers, 'tokens')
  const balance = headers.get('x-venice-balance-usd')
  if (!requests && !tokens && balance === null) return null
  return { requests, tokens, balanceUsd: balance === null ? null : Number(balance), observedAt: Date.now() }
}

/** SDK, 408/409/429/5xx hatalarında Retry-After başlığına uyan üstel geri çekilmeyle yeniden dener. */
export function createClient(apiKey: string, onRateLimits?: (limits: LiveRateLimits) => void): OpenAI {
  // [g9] chat çağrıları modelin sağlayıcısına yönlenir; Venice isteği de host izni + yedek anahtar korumasından geçer.
  return routeByModel(new OpenAI({
    apiKey,
    baseURL: VENICE_BASE_URL,
    maxRetries: 4,
    // Yalnızca başlıklara kadar geçerli; akışın kendisini stream-g10 zaman aşımları korur.
    timeout: 5 * 60_000,
    fetch: providerFetch('venice', async (input, init) => {
      const response = await fetch(input, init)
      const limits = onRateLimits ? parseRateLimitHeaders(response.headers) : null
      if (limits) onRateLimits?.(limits)
      return response
    }),
  }))
}

export interface TokenUsage {
  promptTokens: number
  completionTokens: number
}

export type Approval = 'yes' | 'no' | 'always'

export interface SubagentInfo {
  type: string
  description: string
  background: boolean
  model: string
}

/** Ajanın dış dünyaya (arayüz/CLI) bildirdiği olaylar. callId her araç çağrısı için benzersizdir. */
export interface AgentIO {
  write(text: string): void
  toolStart(callId: string, name: string, summary: string): void
  toolEnd(callId: string, result: string): void
  /** warning: tehlikeli komut gibi durumlarda kullanıcıya gösterilecek uyarı; preview: dosya araçlarında yapılacak değişiklik (diff) */
  confirm(callId: string, name: string, summary: string, warning?: string, preview?: ToolPreview): Promise<Approval>
  /** Alt ajan başlarken; dönen io alt ajanın olaylarını taşır. Yoksa alt ajan olayları üst io'ya aynen akar. */
  subagentStarted?(callId: string, info: SubagentInfo): AgentIO
  /** Arka plan alt ajanı bittiğinde */
  subagentFinished?(callId: string, result: string, ok: boolean): void
  /** Arka plan işinden gelen bildirim konuşmaya eklendiğinde */
  notice?(text: string): void

  // [g1] AgentIO olayları

  // [g2] AgentIO olayları
  /** Kuyruktaki kullanıcı mesajları konuşmaya eklendiğinde (arayüz "Sırada" etiketini kaldırır) */
  g2QueueConsumed?(items: QueuedMessageInfo[]): void

  // [g3] AgentIO olayları
  /** Akıl yürütme modelinin düşünme metni (parça parça) */
  reasoning?(delta: string): void
  /** Oturumun modeli değişti; yeni model ilk kez çağrılmadan hemen önce */
  modelChanged?(from: string, to: string): void
  /** ask_user / exit_plan_mode için kullanıcıyı bekleyen köprü */
  interact?: InteractBridge | undefined

  // [g4] AgentIO olayları

  // [g5] AgentIO olayları

  // [g6] AgentIO olayları
  /** Her model çağrısından sonra: bağlam doluluğu ve önbellek isabeti */
  contextUpdate?(info: ContextInfo): void
  /** Eski turlar özetlendiğinde (otomatik veya elle) */
  compacted?(event: CompactionEvent): void
  /** Sıkıştırma sürüyor / bitti (ayraçta "özetleniyor…" durumu) */
  compactionState?(running: boolean): void
  /** Sıkıştırma başarısız: arayüzde görünür uyarı (sessiz console.warn yerine) */
  compactionFailed?(message: string): void

  // [g9] AgentIO olayları
  /** Görev (tur + alt ajanlar) maliyeti; akış sürerken tahmini, tavan aşılınca tur durur */
  g9Cost?(info: TurnCostInfo): void

  // [g10] AgentIO olayları
  /** Güvenilirlik olayları: devre kesici, takılan araç, akış yeniden bağlanma, yedek model, JSON onarımı, ret, çakışma */
  g10Event?(event: G10Event): void

  // [g11] AgentIO olayları

  // [g12] AgentIO olayları
  /** Toplu silme için dosya listeli ikinci onay; yoksa confirm uyarı metniyle kullanılır */
  g12ConfirmBulkDelete?: BulkDeleteConfirm

  // [g13] AgentIO olayları

  // [g14] AgentIO olayları

  // [g15] AgentIO olayları

  // [g16] AgentIO olayları

  // [g18] AgentIO olayları

  // [g19] AgentIO olayları

  // [g20] AgentIO olayları

  // [g21] AgentIO olayları

  // [g22] AgentIO olayları

  // [g7] AgentIO olayları
}

export interface Session {
  id: string
  client: OpenAI
  model: string
  maxTurns: number
  permissions: PermissionConfig
  messages: ChatCompletionMessageParam[]
  /** Oturum boyunca harcanan token; alt ajanlar da aynı haritaya yazar */
  usage: Map<string, TokenUsage>
  /** 'Bu oturumda hep izin ver' denen araçlar */
  alwaysAllowed: Set<string>
  tools: readonly Tool[]
  agents: readonly AgentDefinition[]
  /** Arka plan işlerinden gelen, bir sonraki model çağrısından önce eklenecek bildirimler */
  inbox: string[]
  /** inbox'a bildirim düştüğünde (ör. ajan boştaysa yeni tur başlatmak için) */
  onInbox: (() => void) | null
  /** Oturum kapanınca iptal edilir: arka plan alt ajanları buna bağlıdır */
  lifetime: AbortController
  shells: ShellRegistry
  /** Venice görsel üretimi yeteneği (anahtarı kapatır); generate_image aracına ToolContext ile geçer. */
  generateImage?: ToolContext['generateImage']
  /** Venice gömme yeteneği; file_search (RAG) aracına ToolContext ile geçer. */
  embed?: ToolContext['embed']
  /** İzole ajan tarayıcısı yeteneği (masaüstü); browser_* araçlarına ToolContext ile geçer. */
  browser?: ToolContext['browser']
  /** Tam bilgisayar kontrolü yeteneği (masaüstü); computer_* araçlarına ToolContext ile geçer. */
  computer?: ToolContext['computer']
  /** Pencere yönetimi / pano / OCR / UIA yetenekleri (masaüstü); ilgili araçlara ToolContext ile geçer. */
  windows?: ToolContext['windows']
  clipboard?: ToolContext['clipboard']
  ocr?: ToolContext['ocr']
  uia?: ToolContext['uia']
  screenRegion?: ToolContext['screenRegion']
  drag?: ToolContext['drag']
  system?: ToolContext['system']
  pixelColor?: ToolContext['pixelColor']
  windowGeometry?: ToolContext['windowGeometry']
  recentFiles?: ToolContext['recentFiles']
  clipboardImage?: ToolContext['clipboardImage']
  windowCapture?: ToolContext['windowCapture']
  displays?: ToolContext['displays']
  chromeExtension?: ToolContext['chromeExtension']
  mcpResources?: ToolContext['mcpResources']

  // [g1] Session alanları

  // [g2] Session alanları
  /** Mesaj kuyruğu, bekleyen görseller, akıl yürütme düzeyi (bkz. user-queue.ts) */
  g2?: G2SessionState

  // [g3] Session alanları
  /** Son model çağrısında kullanılan model; değişiklik notu için */
  g3LastModel?: string

  // [g4] Session alanları

  // [g5] Session alanları

  // [g6] Session alanları
  /** Bağlam, önbellek ve sıkıştırma durumu; ilk kullanımda oluşturulur (context.ts) */
  contextState?: ContextState

  // [g9] Session alanları

  // [g10] Session alanları
  /** Yeniden deneme bütçesi, tur günlüğü durumu ve ayarlar (agent-g10.ts) */
  g10?: G10SessionState

  // [g11] Session alanları

  // [g11] Session alanları
  /** true: araç listesi izin listesidir; ertelenmiş araç yüklenemez (tarif, kısıtlı alt ajan). */
  toolsLocked?: boolean
  /** [cdx] Modelin use_skill ile yüklediği skiller: sıkıştırma sonrası yeniden verilir, her turda hatırlatılır */
  activeSkills?: Set<string>
  /** [cdx] Sıkıştırma skill gövdelerini özete düşürdü; bir sonraki model çağrısından önce yeniden ver */
  skillsReinject?: boolean

  // [g12] Session alanları

  // [g11] Session alanları (diğer)

  // [g12] Session alanları

  // [g13] Session alanları
  /** Oturum katmanı ve sistem mesajı sonundaki proje/sürüm/oturum bloğunun önbelleği (prompt-g13.ts) */
  g13?: G13SessionState

  // [g14] Session alanları

  // [g15] Session alanları

  // [g16] Session alanları

  // [g18] Session alanları

  // [g19] Session alanları

  // [g20] Session alanları

  // [g21] Session alanları

  // [g22] Session alanları
  /** Araç modu (yerel/metin/kapalı), metin protokolü süzgeci; ilk model çağrısında oluşturulur (text-tools-g22.ts) */
  g22?: G22SessionState

  // [g7] Session alanları
  /** Görev tipine göre model (settings.taskModels; takma adlar çözülmüş). compaction ve title diğer gruplarca okunur. */
  taskModels?: TaskModels
  /** Masaüstü: yan oturum / oturumlar arası mesaj yeteneği. Alt ajanlara aktarılmaz. */
  sessionBridge?: SessionBridge
}

export interface SessionOptions {
  client: OpenAI
  model: string
  maxTurns: number
  permissions: PermissionConfig
  systemPrompt: string
  agents: readonly AgentDefinition[]
  id?: string
  /** Kayıttan geri yüklemede sistem mesajı hariç önceki mesajlar */
  history?: ChatCompletionMessageParam[]
  tools?: readonly Tool[]
  generateImage?: ToolContext['generateImage']
  embed?: ToolContext['embed']
  browser?: ToolContext['browser']
  computer?: ToolContext['computer']
  windows?: ToolContext['windows']
  clipboard?: ToolContext['clipboard']
  ocr?: ToolContext['ocr']
  uia?: ToolContext['uia']
  screenRegion?: ToolContext['screenRegion']
  drag?: ToolContext['drag']
  system?: ToolContext['system']
  pixelColor?: ToolContext['pixelColor']
  windowGeometry?: ToolContext['windowGeometry']
  recentFiles?: ToolContext['recentFiles']
  clipboardImage?: ToolContext['clipboardImage']
  windowCapture?: ToolContext['windowCapture']
  displays?: ToolContext['displays']
  chromeExtension?: ToolContext['chromeExtension']
  mcpResources?: ToolContext['mcpResources']
}

export function createSession(o: SessionOptions): Session {
  const session: Session = {
    id: o.id ?? randomUUID(),
    client: o.client,
    model: o.model,
    maxTurns: o.maxTurns,
    permissions: o.permissions,
    messages: [{ role: 'system', content: o.systemPrompt }, ...(o.history ?? [])],
    usage: new Map(),
    alwaysAllowed: new Set(),
    // Üst düzey oturumlar çekirdek + etkin MCP araçlarını alır; alt ajanlar zaten üstten süzülmüş listeyi alır (o.tools).
    // [perf] Masaüstü kontrol aileleri ertelenir (tool_search ya da doğrudan çağrı yükler): istek başına ~35 şema daha az.
    tools: o.tools ?? [...deferDesktopTools(tools), ...getDynamicTools()],
    generateImage: o.generateImage,
    embed: o.embed,
    browser: o.browser,
    computer: o.computer,
    windows: o.windows,
    clipboard: o.clipboard,
    ocr: o.ocr,
    uia: o.uia,
    screenRegion: o.screenRegion,
    drag: o.drag,
    system: o.system,
    pixelColor: o.pixelColor,
    windowGeometry: o.windowGeometry,
    recentFiles: o.recentFiles,
    clipboardImage: o.clipboardImage,
    windowCapture: o.windowCapture,
    displays: o.displays,
    chromeExtension: o.chromeExtension,
    mcpResources: o.mcpResources,
    agents: o.agents,
    inbox: [],
    onInbox: null,
    lifetime: new AbortController(),
    shells: new ShellRegistry((shell, tail) => {
      if (shell.status === 'killed') return
      notify(
        session,
        `<task-notification>\nArka plan komutu bitti (shell_id: ${shell.id}, çıkış kodu ${shell.exitCode ?? '?'}): ${shell.command}\nSon çıktı:\n${maskSecrets(tail.trim()) || '(çıktı yok)'}\n</task-notification>`,
      )
    }),
  }
  return session
}

/** Arka plan işinden ajana bildirim gönderir. */
export function notify(session: Session, text: string): void {
  session.inbox.push(text)
  session.onInbox?.()
}

/** Oturumu kapatır: arka plan alt ajanları iptal edilir, arka plan komutları sonlandırılır. */
export function disposeSession(session: Session): void {
  session.onInbox = null
  session.lifetime.abort()
  session.shells.killAll()
}

// Testte model, reddedilen "git reset --hard"ı "git.exe reset --hard" gibi varyasyonlarla tekrar denedi.
const NO_WORKAROUND =
  'Bu adımı atla ve devam et ya da kullanıcıya sor. Ama önce şunu dene: aynı işi başka bir araç, başka bir yazım veya doğrudan dosya aracıyla yap (ör. kabuk cd yerine grep/list_files, bloklanan komut yerine node/curl). Hâlâ blokluysa kullanıcıya tek satırda söyle.'

const MAX_SUBAGENT_REPORT_CHARS = 20_000
const subagentSlots = new Semaphore(MAX_CONCURRENT_SUBAGENTS)

/** Araç tanımları çağrı başına yeniden kurulmasın: (oturum araçları + ajan listesi) değişmedikçe aynı dizi döner. */
let specsCache: { key: unknown[]; specs: ChatCompletionTool[] } | null = null

function toolSpecsFor(session: Session): ChatCompletionTool[] {
  const key: unknown[] = [session.tools, session.agents]
  if (specsCache && specsCache.key[0] === key[0] && specsCache.key[1] === key[1]) return specsCache.specs
  const specs = session.tools.map((t) => {
    let description = t.description
    if (t.name === 'agent') {
      const list = session.agents.map((a) => `- ${a.name}: ${a.description}`).join('\n')
      description += `\n\nKullanılabilir ajan tipleri:\n${list}`
    }
    return { type: 'function' as const, function: { name: sanitizeToolName(t.name), description, parameters: t.parameters } }
  })
  specsCache = { key, specs }
  return specs
}

function systemPromptOf(session: Session): string {
  const first = session.messages[0]
  return first?.role === 'system' && typeof first.content === 'string' ? first.content : ''
}

// Alt ajana taşınmayan üst düzey bölümler: kullanıcı kişiselleştirmesi ve çapraz oturum belleği. Tek amaçlı, rapor
// yazıp kapanan bir alt ajanın işine yaramaz; araştırma ajanı 15+ tur yapınca her turda tekrar gönderilmesi baskın
// maliyet olur. Proje haritası ve git anlık görüntüsü KORUNUR (kod odaklı alt ajanlara görev bağlamı sağlar).
const SUBAGENT_OMIT_HEADERS = ['# MEMORY', '# ORGANIZATION MEMORY', '# REMOTE CONNECTIONS'] as const

/** Verilen üst düzey "# Başlık" bölümünü (bir sonraki "# " başlığına ya da metin sonuna kadar) çıkarır. */
function stripTopSection(prompt: string, header: string): string {
  const start = prompt.indexOf(`\n\n${header}\n`)
  if (start < 0) return prompt
  const next = prompt.indexOf('\n\n# ', start + 2)
  return prompt.slice(0, start) + (next < 0 ? '' : prompt.slice(next))
}

/** Üst oturumun istemini alt ajana göre inceltir: ajan zekâsı/bellek bloğu (g11) ve kişiselleştirme bölümleri çıkarılır. */
export function stripSubagentSections(prompt: string): string {
  let base = stripSystemBlock(prompt, 'g11') // POLİTİKA + güven bloğu + proje bilgisi + oturum belleği
  for (const header of SUBAGENT_OMIT_HEADERS) base = stripTopSection(base, header)
  return base
}

function subagentSystemPrompt(parent: Session, def: AgentDefinition): string {
  return `${stripSubagentSections(systemPromptOf(parent))}\n\n# SUB-AGENT ROLE: ${def.name}\n${def.prompt}`
}

async function runSubagent(
  parent: Session,
  request: SubagentRequest,
  parentIo: AgentIO,
  ctx: ToolContext,
  callId: string,
): Promise<string> {
  const def = parent.agents.find((d) => d.name === request.subagent_type)
  if (!def) {
    throw new Error(`Bilinmeyen ajan tipi "${request.subagent_type}". Kullanılabilir: ${parent.agents.map((a) => a.name).join(', ')}`)
  }
  // Derinlik 1: alt ajanlar agent aracını hiçbir koşulda alamaz.
  const allowed = parent.tools.filter((t) => t.name !== 'agent' && t.name !== 'run_team' && (def.tools === 'all' || def.tools.includes(t.name)))
  const background = request.run_in_background === true
  const model = subagentModel(def, parent.taskModels, parent.model)

  const child = createSession({
    client: parent.client,
    model,
    maxTurns: parent.maxTurns <= 0 ? 60 : Math.min(parent.maxTurns, 40), // ana süreç sınırsız olsa da alt ajanlara makul tavan
    permissions: def.manualApproval ? manualPermissions(parent.permissions) : parent.permissions,
    systemPrompt: subagentSystemPrompt(parent, def),
    agents: [],
    tools: allowed,
    ...(parent.generateImage ? { generateImage: parent.generateImage } : {}),
    ...(parent.embed ? { embed: parent.embed } : {}),
    ...(parent.browser ? { browser: parent.browser } : {}),
    ...(parent.computer ? { computer: parent.computer } : {}),
    ...(parent.windows ? { windows: parent.windows } : {}),
    ...(parent.clipboard ? { clipboard: parent.clipboard } : {}),
    ...(parent.ocr ? { ocr: parent.ocr } : {}),
    ...(parent.uia ? { uia: parent.uia } : {}),
    ...(parent.screenRegion ? { screenRegion: parent.screenRegion } : {}),
    ...(parent.drag ? { drag: parent.drag } : {}),
    ...(parent.system ? { system: parent.system } : {}),
    ...(parent.pixelColor ? { pixelColor: parent.pixelColor } : {}),
    ...(parent.windowGeometry ? { windowGeometry: parent.windowGeometry } : {}),
    ...(parent.recentFiles ? { recentFiles: parent.recentFiles } : {}),
    ...(parent.clipboardImage ? { clipboardImage: parent.clipboardImage } : {}),
    ...(parent.windowCapture ? { windowCapture: parent.windowCapture } : {}),
    ...(parent.displays ? { displays: parent.displays } : {}),
    ...(parent.chromeExtension ? { chromeExtension: parent.chromeExtension } : {}),
    ...(parent.mcpResources ? { mcpResources: parent.mcpResources } : {}),
  })
  // Maliyet, oturum izinleri ve arka plan komutları üst oturumla ortak.
  child.usage = parent.usage
  child.toolsLocked = def.tools !== 'all' || parent.toolsLocked === true
  child.alwaysAllowed = def.manualApproval ? new Set() : parent.alwaysAllowed
  child.shells = parent.shells

  const io = parentIo.subagentStarted?.(callId, { type: def.name, description: request.description, background, model }) ?? parentIo
  // Ön plan alt ajanı turla birlikte durur; arka plandaki oturum kapanana kadar yaşar.
  const signal = background ? parent.lifetime.signal : ctx.signal

  const run = async (): Promise<string> => {
    const release = await subagentSlots.acquire(signal)
    try {
      await runTurn(child, request.prompt, io, { root: ctx.root, signal, ...(ctx.onFileChange ? { onFileChange: ctx.onFileChange } : {}) })
      const last = child.messages.findLast((m) => m.role === 'assistant' && typeof m.content === 'string' && m.content.trim())
      const report = typeof last?.content === 'string' ? last.content.trim() : '(alt ajan rapor yazmadı)'
      return report.length > MAX_SUBAGENT_REPORT_CHARS ? `${report.slice(0, MAX_SUBAGENT_REPORT_CHARS)}\n[rapor kesildi]` : report
    } finally {
      release()
    }
  }

  if (!background) return `[${def.name} alt ajanı raporu]\n${await run()}`

  void run().then(
    (report) => {
      parentIo.subagentFinished?.(callId, report, true)
      notify(parent, `<task-notification>\nArka plan alt ajanı tamamlandı (id: ${callId}, ${def.name}: ${request.description})\n${report}\n</task-notification>`)
    },
    (err: unknown) => {
      if (parent.lifetime.signal.aborted) return // Oturum kapandı: bildirim gereksiz.
      const message = err instanceof Error ? err.message : String(err)
      parentIo.subagentFinished?.(callId, message, false)
      notify(parent, `<task-notification>\nArka plan alt ajanı başarısız oldu (id: ${callId}, ${def.name}: ${request.description}): ${message}\n</task-notification>`)
    },
  )
  return `Arka plan alt ajanı başlatıldı (id: ${callId}). Bitince sonucu bildirim olarak alacaksın; beklerken başka işlere devam edebilirsin.`
}

async function executeToolCall(
  session: Session,
  name: string,
  rawArgs: string,
  io: AgentIO,
  ctx: ToolContext,
): Promise<string> {
  const callId = randomUUID()
  const audit = (entry: Omit<AuditEntry, 'sessionId' | 'root' | 'model' | 'tool'>) =>
    writeAudit({ sessionId: session.id, root: ctx.root, model: session.model, tool: name, ...entry })

  let tool = session.tools.find((t) => t.name === name || sanitizeToolName(t.name) === name)
  // Ertelenmiş bir aracı model adıyla doğrudan çağırdıysa (adını istemden biliyor) tool_search'e zorlamadan yüklenir.
  if (!tool && isDeferredToolName(name) && ctx.unlockTools) {
    ctx.unlockTools([name])
    tool = session.tools.find((t) => t.name === name || sanitizeToolName(t.name) === name)
  }

  let args: unknown
  try {
    args = JSON.parse(rawArgs || '{}')
  } catch {
    await audit({ args: rawArgs, decision: 'invalid', reason: 'geçersiz JSON' })
    // Model bu mesajı görüp çağrıyı düzeltilmiş argümanlarla tekrarlayabilir.
    return 'Hata: araç argümanları geçerli JSON değil. Argümanları geçerli JSON olarak yeniden gönder.'
  }
  // [uyum] Claude Code adları (Read/Edit/Bash/Task/TodoWrite…), harf/çizgi farkları ve öneksiz MCP adları çevrilir:
  // skiller ve ajanlar hangi ajan için yazılmış olursa olsun her modelde çalışsın.
  if (!tool) {
    const known = new Set(session.tools.map((t) => t.name))
    const compat = resolveCompatCall(name, args && typeof args === 'object' && !Array.isArray(args) ? (args as Record<string, unknown>) : {}, known)
    if (!compat && ctx.unlockTools) {
      // Ertelenmiş aracın takma adı olabilir (ör. "Screenshot" → computer_screenshot): çeviriyi ertelenmiş kayıtla da dene.
      const deferredNames = new Set([...known, ...DESKTOP_DEFERRED])
      const c2 = resolveCompatCall(name, (args ?? {}) as Record<string, unknown>, deferredNames)
      if (c2 && !known.has(c2.name)) {
        ctx.unlockTools([c2.name])
        tool = session.tools.find((t) => t.name === c2.name)
        if (tool) {
          args = c2.args
          io.notice?.(`Araç adı çevrildi: ${name} → ${c2.name}`)
        }
      }
    } else if (compat) {
      tool = session.tools.find((t) => t.name === compat.name)
      args = compat.args
      if (tool) io.notice?.(`Araç adı çevrildi: ${name} → ${compat.name}`)
    }
  }
  if (!tool) {
    await audit({ args: rawArgs, decision: 'invalid', reason: 'bilinmeyen araç' })
    const hint = isDeferredToolName(name) ? ' Önce tool_search ile yükle.' : ''
    const near = session.tools.map((t) => t.name).filter((n) => n.includes(name.toLowerCase().slice(0, 4))).slice(0, 5)
    return `Hata: bilinmeyen veya bu ajana kapalı araç '${name}'.${hint}${near.length ? ` Benzer adlar: ${near.join(', ')}.` : ''} Araç adları küçük harf ve snake_case'dir (ör. read_file, edit_file, bash, grep, list_files, agent).`
  }

  io.toolStart(callId, name, tool.summarize(args))
  const finish = (raw: string) => {
    // [g5] Gizli bilgi maskeleme: araç sonucu modele ve arayüze gitmeden önce tek noktada (CWE-200).
    const result = maskSecrets(raw)
    io.toolEnd(callId, result)
    return result
  }

  let decision = decide(session.permissions, name, tool.kind, tool.subject(args), session.alwaysAllowed)
  // [g26] Yargıç reddi gevşetmez. Bypass kullanıcının seçimidir.
  if (decision.action !== 'deny' && session.permissions.mode !== 'bypass' && tool.kind === 'exec') {
    const next = tightenDecision(decision.action, adversaryVerdict(String(tool.subject(args))))
    if (next === 'deny') decision = { action: 'deny', reason: 'Komut politikası: çözülen gövde tehlikeli.' }
    else if (next === 'ask' && decision.action === 'allow') decision = { action: 'ask', warning: 'Komut politikası bu çağrıyı onaya bıraktı.' }
  }
  // Bilgisayar kontrolü araçları: bilinçli 'bypass' dışında her eylem ayrı onay ister; allow kuralı/oturum izni
  // bunları toplu açamaz. warning bulunması nedeniyle 'hep izin ver' de kalıcılaşmaz (bkz. aşağısı).
  if (tool.sensitive && decision.action === 'allow' && session.permissions.mode !== 'bypass') {
    decision = { action: 'ask', warning: 'Bilgisayar kontrolü: bu eylem gerçek fare/klavye/ekranı kullanır' }
  }
  let auditDecision: AuditEntry['decision'] = 'allow'
  if (decision.action === 'deny') {
    await audit({ args, decision: 'deny', reason: decision.reason })
    return finish(`Engellendi: ${decision.reason} ${NO_WORKAROUND}`)
  }
  if (decision.action === 'ask') {
    // Dosya araçları: onay kartında gerçek diff gösterilsin diye değişiklik önceden hesaplanır (yan etkisiz).
    const preview = tool.preview ? await tool.preview(args, { root: ctx.root, signal: ctx.signal, shells: session.shells }) : null
    const answer = await io.confirm(callId, name, tool.summarize(args), decision.warning, preview ?? undefined)
    if (answer === 'no') {
      await audit({ args, decision: 'rejected', reason: decision.warning })
      return finish(`Kullanıcı bu işlemi reddetti. ${NO_WORKAROUND}`)
    }
    // Tehlikeli komutlar için 'hep izin ver' kalıcı yapılmaz; her seferinde sorulur.
    if (answer === 'always' && !decision.warning) session.alwaysAllowed.add(name)
    auditDecision = 'approved'
  }

  // Ön kancalar: izin verildikten sonra, araç çalışmadan önce. Engellerse araç çalışmaz, gerekçe modele döner.
  let pre: Awaited<ReturnType<typeof runPreToolHooks>>
  try {
    pre = await runPreToolHooks(name, args, ctx.root)
  } catch (err) {
    // Kanca sonuç veremediyse araç çalışmaz (kapalı başarısızlık); kart da açık kalmaz.
    const reason = err instanceof Error ? err.message : String(err)
    await audit({ args, decision: 'deny', reason: `kanca hatası: ${reason}` })
    return finish(`Kanca hatası: ${reason}. Araç çalıştırılmadı. ${NO_WORKAROUND}`)
  }
  if (pre.blocked) {
    await audit({ args, decision: 'deny', reason: `kanca: ${pre.reason}` })
    return finish(`Kanca engelledi: ${pre.reason} ${NO_WORKAROUND}`)
  }

  const callCtx: ToolContext = {
    root: ctx.root,
    signal: ctx.signal,
    shells: session.shells,
    ...(ctx.onFileChange
      ? { onFileChange: (p: string, before: string | null, after: string) => ctx.onFileChange?.(p, before, after, callId) }
      : {}),
    // Alt ajan başlatma yalnızca agent aracına sahip (üst düzey) oturumlarda vardır.
    ...(session.agents.length > 0 ? { spawnSubagent: (req: SubagentRequest) => runSubagent(session, req, io, ctx, callId) } : {}),
    ...(session.sessionBridge ? { sessionBridge: session.sessionBridge } : {}),
    // Worktree tur düzeyinde (main.ts turCtx'e koyar); oturumda değil, tur bağlamından geçer.
    ...(ctx.worktree ? { worktree: ctx.worktree } : {}),
    // [g3] etkileşimli araç köprüsü
    ...(io.interact ? { interact: io.interact, callId } : {}),
    ...(session.generateImage ? { generateImage: session.generateImage } : {}),
    ...(session.embed ? { embed: session.embed } : {}),
    ...(session.browser ? { browser: session.browser } : {}),
    ...(session.computer ? { computer: session.computer } : {}),
    ...(session.windows ? { windows: session.windows } : {}),
    ...(session.clipboard ? { clipboard: session.clipboard } : {}),
    ...(session.ocr ? { ocr: session.ocr } : {}),
    ...(session.uia ? { uia: session.uia } : {}),
    ...(session.screenRegion ? { screenRegion: session.screenRegion } : {}),
    ...(session.drag ? { drag: session.drag } : {}),
    ...(session.system ? { system: session.system } : {}),
    ...(session.pixelColor ? { pixelColor: session.pixelColor } : {}),
    ...(session.windowGeometry ? { windowGeometry: session.windowGeometry } : {}),
    ...(session.recentFiles ? { recentFiles: session.recentFiles } : {}),
    ...(session.clipboardImage ? { clipboardImage: session.clipboardImage } : {}),
    ...(session.windowCapture ? { windowCapture: session.windowCapture } : {}),
    ...(session.displays ? { displays: session.displays } : {}),
    ...(session.chromeExtension ? { chromeExtension: session.chromeExtension } : {}),
    ...(session.mcpResources ? { mcpResources: session.mcpResources } : {}),
    // [g23] yerel asistan: depo, konum, ağ ve ertelenmiş araç kilidi tur bağlamından gelir
    sessionId: ctx.sessionId ?? session.id,
    permissionMode: session.permissions.mode, // [g12] toplu silme onayı güncel izin modunu görsün (bypass = sorma)
    ...(ctx.assistantHome ? { assistantHome: ctx.assistantHome } : {}),
    ...(ctx.locate ? { locate: ctx.locate } : {}),
    ...(ctx.net ? { net: ctx.net } : {}),
    ...(ctx.unlockTools ? { unlockTools: ctx.unlockTools } : {}),
    ...(ctx.sessionForkSource ? { sessionForkSource: ctx.sessionForkSource } : {}),
    ...(ctx.sessionsDir ? { sessionsDir: ctx.sessionsDir } : {}),
  }

  const started = performance.now()
  try {
    // Şema hataları da modele döner; model eksik/yanlış alanı düzeltebilir.
    const result = await tool.run(args, callCtx)
    // [cdx] use_skill ile yüklenen skill oturum boyunca "etkin" sayılır (g11 her turda hatırlatır, sıkıştırma sonrası yeniden verir).
    if (name === 'use_skill' && args && typeof args === 'object' && typeof (args as { name?: unknown }).name === 'string' && !result.startsWith('"')) {
      ;(session.activeSkills ??= new Set()).add((args as { name: string }).name)
    }
    // Son kancalar: çıktıları araç sonucuna bağlam olarak eklenir (güvenilmez; işaretli ve maskeli).
    const extra = await runPostToolHooks(name, args, result, ctx.root)
    await audit({ args, decision: auditDecision, ok: true, durationMs: Math.round(performance.now() - started), resultChars: result.length })
    return finish(extra ? `${result}\n\n${extra}` : result)
  } catch (err) {
    const message = `Hata: ${describeToolError(name, tool, args, err)}`
    await audit({ args, decision: auditDecision, ok: false, durationMs: Math.round(performance.now() - started), reason: message })
    return finish(message)
  }
}

/**
 * Araç hatasını modele eyleme dönük dille çevirir. Ham zod mesajı ("Expected string, received number") modeli
 * kilitleyebiliyordu; burada hangi alan, hangi tür, ne yapmalı bilgisi verilir. Ayrıca aynı araç+aynı argüman
 * tekrar denendiğinde "değiştir" uyarısı eklenir (kısır döngü).
 */
function describeToolError(name: string, tool: Tool, args: unknown, err: unknown): string {
  const last = lastToolAttempt.get(name)
  const fingerprint = JSON.stringify(args)
  const repeated = last === fingerprint
  lastToolAttempt.set(name, fingerprint)

  const zodIssues = (err as { issues?: Array<{ path?: Array<string | number>; expected?: string; code?: string; message?: string }> })?.issues
  if (Array.isArray(zodIssues) && zodIssues.length > 0) {
    const lines = zodIssues.slice(0, 5).map((issue) => {
      const field = (issue.path ?? []).join('.') || '(kök)'
      const expected = issue.expected ?? issue.code ?? 'geçerli değer'
      return `- "${field}": ${expected} bekleniyor${issue.message ? ` (${issue.message})` : ''}`
    })
    const shape = JSON.stringify(tool.parameters.properties ? Object.keys(tool.parameters.properties as object) : [])
    return [
      `"${name}" argümanları şemaya uymuyor.`,
      ...lines,
      `Beklenen alanlar: ${shape}`,
      ...(repeated ? ['Aynı argümanla tekrar denendi: en az bir alanı değiştir.'] : []),
    ].join('\n')
  }
  const base = err instanceof Error ? err.message : String(err)
  return repeated ? `${base} (aynı argümanla tekrar denendi; bir alanı değiştir)` : base
}

/** Okuma araçları ve alt ajanlar paralel, diğerleri (yazma, komut) sırayla çalışır. */
export function batchToolCalls<T extends { type: string; function?: { name: string } }>(
  toolKinds: ReadonlyMap<string, Tool['kind']>,
  calls: readonly T[],
): T[][] {
  const batches: { parallel: boolean; calls: T[] }[] = []
  for (const call of calls) {
    const parallel = call.type === 'function' && call.function !== undefined && toolKinds.get(call.function.name) === 'read'
    const last = batches.at(-1)
    if (parallel && last?.parallel) last.calls.push(call)
    else batches.push({ parallel, calls: [call] })
  }
  return batches.map((b) => b.calls)
}

/**
 * Kullanıcı mesajını (veya userText null ise yalnızca bekleyen bildirimleri) işler;
 * model araç çağırmayı bırakana kadar döngü sürer.
 */
const ENDED_NOTE = 'Bu sohbet kalıcı olarak kapatıldı. Yeni bir sohbet açabilirsin.'

/** Araç başına son denenen argüman parmak izi: aynı hata tekrarında modele "değiştir" denir. */
const lastToolAttempt = new Map<string, string>()

function refuseEnded(session: Session, userText: string | null, io: AgentIO): void {
  // Kapalı sohbette bildirim işlenmez; kutuda kalırsa main.ts turu sonsuza kadar yeniden başlatır.
  session.inbox.length = 0
  if (userText !== null) session.messages.push({ role: 'user', content: userText })
  session.messages.push({ role: 'assistant', content: ENDED_NOTE })
  io.write(`${ENDED_NOTE}\n`)
}

/** Bir turda yarım yanıt için en fazla bu kadar otomatik devam: model gerçekten bitirdiyse sonsuz döngü olmasın. */
export const MAX_UNFINISHED_NUDGES = 2
export const UNFINISHED_NOTE =
  '<system-reminder>Yanıtın bir sonraki adımı duyurup araç çağırmadan (ya da boş) bitti. Bu kullanıcı mesajı değil. İş bitmediyse şimdi ilgili aracı çağırarak kaldığın yerden devam et; gerçekten bittiyse yapılanları kısaca özetle.</system-reminder>'

/** Araçsız bitmiş yanıt yarım mı: boş, ya da "şimdi şunu yapıyorum:" gibi iki nokta/üç noktayla biten duyuru. */
export function looksUnfinished(content: string): boolean {
  const text = content.trim()
  return text === '' || /(:|\.\.\.|…)$/.test(text)
}

export async function runTurn(
  session: Session,
  userText: string | null,
  io: AgentIO,
  ctx: ToolContext,
  /** Bağlam taşması sonrası kaç kez yeniden denendi (413 kurtarma) */
  overflowRetry = 0,
): Promise<void> {
  // [g23] Bu süreçte kapanmış sohbet. Disk okuması tur başını bekletmez (g2 kuyruk yarışı).
  if (sessionEndedSync(session.id, ctx.assistantHome)) {
    refuseEnded(session, userText, io)
    return
  }
  ctx.sessionForkSource = () => ({
    model: session.model,
    root: ctx.root,
    permissionMode: session.permissions.mode,
    messages: session.messages,
    id: session.id,
  })
  ctx.unlockTools = (names) => {
    if (session.toolsLocked) return // tarif ve kısıtlı alt ajan: izin listesi dışına çıkılamaz
    const have = new Set(session.tools.map((t) => t.name))
    const extra = names.flatMap((name) => {
      const tool = deferredByName(name)
      return tool && !have.has(tool.name) ? [tool] : []
    })
    if (extra.length > 0) session.tools = [...session.tools, ...extra]
  }
  const checkpoint = session.messages.length
  const drained: { notices: string[]; message: ChatCompletionMessageParam }[] = []
  const consumed: ConsumedQueueItem[] = []
  let toolsRan = false
  if (userText !== null) session.messages.push({ role: 'user', content: userText })

  const drainInbox = () => {
    if (session.inbox.length === 0) return
    const notices = session.inbox.splice(0)
    const message: ChatCompletionMessageParam = { role: 'user', content: notices.join('\n\n') }
    drained.push({ notices, message })
    for (const n of notices) io.notice?.(n)
    session.messages.push(message)
  }

  try {
    drainInbox()
    if (session.messages.length === checkpoint) return // İşlenecek bir şey yok.

    let unfinishedNudges = 0
    let overflowRecovering = false
    // maxTurns <= 0 => sınırsız (kullanıcı adım sınırını kaldırabilir). Döngü güvenliği: kullanıcı her an durdurabilir.
    for (let step = 0; session.maxTurns <= 0 || step < session.maxTurns; step++) {
      if (step > 0) drainInbox()

      // [g1] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g2] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      consumed.push(...g2BeforeModelCall(session, io, step))
      if (step === 0) applyPlanAgent(session) // [g26] plan modu ayrı plan ajanı sözleşmesi

      // [g23] İlk turda depodan kapanış okunur; kuyruk birleştirildikten sonra, modelden önce.
      if (step === 0 && (await isSessionEnded(session.id, ctx.assistantHome))) {
        session.messages.push({ role: 'assistant', content: ENDED_NOTE })
        io.write(`${ENDED_NOTE}\n`)
        return
      }

      // [g3] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      noteModelChange(session, io.modelChanged?.bind(io))

      // [g4] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g5] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      // Döngü koruması, uzunluk sınırında otomatik devam, autoVerify kancası; true: döngü nedeniyle tur durur.
      if (g5BeforeModelCall(session, step, io, ctx, notify)) return

      // [g6] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      await autoCompact(session, io, ctx.signal, step)

      // [g9] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      // Düşük maliyet modu, canlı maliyet izleme; true: görev maliyet tavanı aşıldı, tur durur.
      if (g9BeforeModelCall(session, step, io)) return

      // [g10] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      await g10BeforeModelCall(session, io, ctx, step)

      // [g11] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      await g11BeforeModelCall(session, userText, step, io, ctx) // ajan zekâsı bölümü, hafıza katmanları, sapma/çelişki denetimi

      // [g12] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      g12BeforeModelCall(session, io)

      // [g13] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      await g13BeforeModelCall(session, ctx.root) // proje ve oturum sistem istemi katmanları, SÜRÜMLER, doküman kaynakları

      // [g14] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      // Tur ve model çağrısı izini açar; araç ve yanıt olayları io üzerinden gözlenir (telemetry-g14.ts).
      g14BeforeModelCall(session, io, step, ctx)

      // [g15] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      g15BeforeModelCall(session) // kayıt/tekrar oynatmada oturum kimliği seed'den

      // [g16] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g18] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g19] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g20] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g21] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      // [g22] model çağrısı öncesi (sıkıştırma, kuyruk vb.)
      // Araç modu seçilir; dönen io metin modunda ham <tool_call> metnini akıştan süzer.
      io = await g22BeforeModelCall(session, io, step)

      // [g7] model çağrısı öncesi (sıkıştırma, kuyruk vb.)

      const body = {
        model: session.model,
        // [perf] Eski turların uzun araç çıktıları isteğe kırpılmış gider (kayıt ve arayüz tam kalır).
        messages: pruneOldToolOutputs(session.messages),
        tools: toolSpecsFor(session),
        // Venice'ın kendi sistem istemini eklemesini kapatır; davranışı bizim istemimiz belirler.
        venice_parameters: { include_venice_system_prompt: false },
        stream_options: { include_usage: true },

        // [g1] istek gövdesi alanları

        // [g2] istek gövdesi alanları
        ...g2RequestFields(session),

        // [g3] istek gövdesi alanları

        // [g4] istek gövdesi alanları

        // [g5] istek gövdesi alanları

        // [g6] istek gövdesi alanları
        // Venice önbelleği otomatiktir; aynı anahtar istekleri önbelleği sıcak sunucuya yönlendirir (docs.venice.ai, Prompt Caching).
        prompt_cache_key: session.id,

        // [g9] istek gövdesi alanları
        // Model ön ayarı (temperature/top_p/maks token). Sağlayıcı öneki ve Venice'e özgü alanlar taşıma katmanında
        // (provider-pool-g9.ts) düzeltilir: yönlendirme tam kimliğe bakar, tel üzerinde ham kimlik gider.
        ...g9RequestFields(session),
        // Ollama'da num_ctx'e çevrilir; diğer sağlayıcılar taşıma katmanında düşürür.
        __contextLimit: contextLimitOf(session.model),

        // [g10] istek gövdesi alanları

        // [g11] istek gövdesi alanları

        // [g12] istek gövdesi alanları

        // [g13] istek gövdesi alanları

        // [g14] istek gövdesi alanları

        // [g15] istek gövdesi alanları

        // [g16] istek gövdesi alanları

        // [g18] istek gövdesi alanları

        // [g19] istek gövdesi alanları

        // [g20] istek gövdesi alanları

        // [g21] istek gövdesi alanları

        // [g22] istek gövdesi alanları
        ...g22RequestFields(session, toolSpecsFor(session)),

        // [g7] istek gövdesi alanları
      }
      // [g22] Metin protokolü dizisi de oturuma bağlı kalsın: akış kancaları ve g5/g9 kimlikle oturumu bulur.
      aliasMessages(body.messages, session.messages)
      // Idempotency-Key: SDK'nın 429/5xx yeniden denemeleri aynı mantıksal isteği aynı anahtarla gönderir (retry'de
      // headers nesnesi yeniden kullanılır); destekleyen sağlayıcı tekrarı ikinci kez işlemez. 413 kurtarması gibi
      // yeni bir mantıksal istek başlatan yollar ise bilerek yeni anahtar üretir.
      const stream = session.client.chat.completions.stream(body, { signal: ctx.signal, headers: { 'Idempotency-Key': randomUUID() } })
      // Satır içi <think> bloğu (reasoning alanı olmayan sağlayıcılar) yanıta değil düşünce akışına gider.
      const think = createThinkFilter((text) => io.write(text), (text) => io.reasoning?.(text))
      stream.on('content.delta', ({ delta }) => think.push(delta))
      // Parçaları kendimiz de biriktiririz: bazı sağlayıcılar son parçada finish_reason göndermez ve SDK'nın
      // finalChatCompletion'ı "missing finish_reason" fırlatır; o durumda yanıtı bunlardan tamamlarız.
      let accContent = ''
      let accFinish: string | null = null
      const accTools = new Map<number, { id?: string; name?: string; args: string }>()
      stream.on('chunk', (chunk) => {
        const c = chunk.choices?.[0]
        if (typeof c?.delta?.content === 'string') accContent += c.delta.content
        if (c?.finish_reason) accFinish = c.finish_reason
        for (const tc of c?.delta?.tool_calls ?? []) {
          const e = accTools.get(tc.index) ?? { args: '' }
          if (tc.id) e.id = tc.id
          if (tc.function?.name) e.name = tc.function.name
          if (tc.function?.arguments) e.args += tc.function.arguments
          accTools.set(tc.index, e)
        }
        // [g3] akıl yürütme (reasoning_content) akışı
        const thinking = reasoningDelta(chunk)
        if (thinking) io.reasoning?.(thinking)
      })
      let completion: Awaited<ReturnType<typeof stream.finalChatCompletion>>
      try {
        completion = await stream.finalChatCompletion()
      } catch (err) {
        // İlk adımdan sonra tur baştan çalıştırılamaz (araçlar iki kez uygulanır): sıkıştırıp yalnızca bu model çağrısını yinele.
        if (step > 0 && !overflowRecovering && !ctx.signal.aborted && isContextOverflow(err)) {
          overflowRecovering = true
          io.compactionFailed?.('Bağlam taştı; sıkıştırıp yeniden deneniyor.')
          const ok = await compactSession(session, { io, signal: ctx.signal }).then((r) => r.compacted, () => false)
          if (!ok) throw err
          step--
          continue
        }
        if (!/finish_reason/i.test(err instanceof Error ? err.message : String(err))) throw err
        const toolCalls = [...accTools.entries()]
          .sort((a, b) => a[0] - b[0])
          .map(([, t]) => ({ id: t.id ?? '', type: 'function' as const, function: { name: t.name ?? '', arguments: t.args } }))
        completion = {
          id: '',
          object: 'chat.completion',
          created: 0,
          model: session.model,
          choices: [
            {
              index: 0,
              logprobs: null,
              finish_reason: accFinish ?? (toolCalls.length > 0 ? 'tool_calls' : 'stop'),
              message: { role: 'assistant', content: accContent || null, refusal: null, ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}) },
            },
          ],
        } as unknown as typeof completion
      }
      overflowRecovering = false
      think.flush()

      if (completion.usage) {
        const prev = session.usage.get(session.model) ?? { promptTokens: 0, completionTokens: 0 }
        session.usage.set(session.model, {
          promptTokens: prev.promptTokens + completion.usage.prompt_tokens,
          completionTokens: prev.completionTokens + completion.usage.completion_tokens,
        })
      }
      // [g6] model çağrısı sonrası: gerçek bağlam boyutu ve önbellek isabeti
      afterModelCall(session, completion.usage, io)

      const message = completion.choices[0]?.message
      if (!message) throw new Error('Modelden boş yanıt geldi')
      // Düşünce bloğu geçmişe yazılmaz; içindeki <tool_call> metni de çalıştırılmaz.
      if (typeof message.content === 'string') message.content = stripThink(message.content)
      // [g22] model yanıtı sonrası: metin protokolündeki <tool_call> blokları tool_calls'a çevrilir
      g22AfterCompletion(session, message)

      const toolCalls: ChatCompletionMessageToolCall[] = message.tool_calls ?? []
      session.messages.push({
        role: 'assistant',
        content: message.content ?? '',
        ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}),
      })
      if (message.content) io.write('\n')
      if (toolCalls.length === 0) {
        // Model bitirirken bir bildirim geldiyse aynı turda ele alınsın.
        if (session.inbox.length > 0) continue
        // Model "şimdi X'i yapıyorum:" deyip araç çağırmadan ya da boş yanıtla durduysa iş yarım kalmasın.
        if (unfinishedNudges < MAX_UNFINISHED_NUDGES && looksUnfinished(message.content ?? '')) {
          unfinishedNudges++
          session.messages.push({ role: 'user', content: UNFINISHED_NOTE })
          continue
        }
        return
      }

      const run = (call: ChatCompletionMessageToolCall) => {
        // Durdur'dan sonra kalan çağrılar çalışmaz, onay da sormaz; yine de her tool_call'a sonuç yazılır.
        if (ctx.signal.aborted) return Promise.resolve('İptal edildi: kullanıcı turu durdurdu; bu araç çalıştırılmadı.')
        return call.type === 'function'
          ? executeToolCall(session, call.function.name, call.function.arguments, io, ctx)
          : Promise.resolve('Hata: yalnızca function araçları destekleniyor')
      }

      toolsRan = true
      const toolKinds = new Map(session.tools.map((t) => [t.name, t.kind]))
      for (const batch of batchToolCalls(toolKinds, toolCalls)) {
        const results = await Promise.all(batch.map(run))
        batch.forEach((call, i) => session.messages.push({ role: 'tool', tool_call_id: call.id, content: results[i] ?? '' }))
      }
      ctx.signal.throwIfAborted()
      // [g23] sohbet bu turda kapatıldıysa model devam etmez
      if (await isSessionEnded(session.id, ctx.assistantHome)) return
      // [g24] alt dizin talimatı, bir sonraki model çağrısından önce
      for (const call of toolCalls) {
        if (call.type === 'function') noteSubdirToolArgs(session, call.function.arguments, ctx.root)
      }
      const subdirHints = await takeSubdirHints(session, ctx.root)
      if (subdirHints) session.messages.push({ role: 'user', content: subdirHints })
    }
    io.write(`\n[${session.maxTurns} adım sınırına ulaşıldı; durduruldu. Devam etmek için "devam et" yazabilirsin ya da Ayarlar'dan adım sınırını artırabilirsin.]\n`)
  } catch (err) {
    // [g22] API araçları reddetti: model bu oturumda metin protokolüne alınır, ilk çağrıdaysa tur bir kez yeniden denenir
    const g22Retry = g22RetryAsText(session, err, checkpoint, userText !== null)
    // Yarım kalan tool_calls sonraki isteği bozar; hatada tur tamamen geri alınır. Durdur'da ise dosyalar değişmiş kalır:
    // kullanıcı mesajı ve tamamlanmış araç grupları korunur ki "devam et" denince model yapılanı bilsin.
    const base = rollbackLength(session, checkpoint) // [g6] tur içi sıkıştırma kaymasını hesaba katar
    const keep = ctx.signal.aborted ? base + completeGroups(session.messages.slice(base)).length : base
    const removed = new Set(session.messages.slice(keep))
    session.messages.length = keep
    // Yalnızca geri alınan bildirim ve kuyruk mesajları döner; özete girenler veya korunanlar yinelenmesin.
    session.inbox.unshift(...drained.filter((d) => removed.has(d.message)).flatMap((d) => d.notices))
    g2RestoreQueue(session, consumed.filter((c) => removed.has(c.message)).map((c) => c.item))
    if (ctx.signal.aborted) throw err
    if (g22Retry) return g22Retry(() => runTurn(session, userText, io, ctx))
    // 413 (bağlam taştı): bir kez zorla sıkıştır, turu yeniden dene. Sağlayıcı 400 ile "context length" derse de aynısı.
    // Araç çalıştıysa tur tekrarlanmaz: komutlar ve düzenlemeler iki kez uygulanırdı.
    if (overflowRetry < 1 && !toolsRan && isContextOverflow(err)) {
      io.compactionFailed?.('Bağlam taştı; sıkıştırıp yeniden deneniyor.')
      try {
        const result = await compactSession(session, { io, signal: ctx.signal })
        if (result.compacted) return runTurn(session, userText, io, ctx, overflowRetry + 1)
      } catch {
        /* sıkıştırma da başarısız: aşağıdaki hata yolu kullanılır */
      }
    }
    throw err
  }
}

/**
 * Sağlayıcı "bağlam taştı" mı dedi? HTTP 413 ya da 400 + bilinen metinler. Sağlayıcı SDK'sı hatayı farklı
 * sarmalayabildiği için hem durum kodu hem mesaj hem de gövde taranır.
 */
export function isContextOverflow(err: unknown): boolean {
  const status = (err as { status?: unknown })?.status
  if (status === 413) return true
  const parts: string[] = []
  if (err instanceof Error) parts.push(err.message)
  const body = (err as { error?: unknown })?.error
  if (body) parts.push(typeof body === 'string' ? body : JSON.stringify(body))
  const text = parts.join(' ')
  if (status !== undefined && status !== 400) return /context[\s_-]?length|too many tokens|maximum context|token limit/i.test(text)
  return /context[\s_-]?length|too many tokens|maximum context|prompt is too long|token limit|reduce the length/i.test(text)
}
