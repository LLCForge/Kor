import { diffLines } from 'diff'
import type { AgentEventBody, LiveSession, SubagentEventInfo, TranscriptItem } from '../ipc.ts'
import { g3Reduce, type G3ChatItem, type G3State } from './g3-state.ts' // [g3]
import { g2QueueConsumed, g2Rewind, type G2ChatAction } from './g2/chat-g2.ts' // [g2]
import type { ConfirmReviewFields, ReviewFile } from '../ipc-review.ts' // [review]
import type { RuleFinding } from '../ipc.gen.ts' // §28.3

export interface SubagentState extends SubagentEventInfo {
  status: 'running' | 'done' | 'failed' | 'stopped'
  report: string | null
  /** Alt ajanın başlama anı (epoch ms): kart yeniden yüklense de sayaç sıfırlanmaz */
  startedAt?: number
}

export interface ToolItem {
  kind: 'tool'
  id: string
  name: string
  summary: string
  result: string | null
  awaitingApproval: boolean
  warning?: string
  /** Bu araç çağrısının yaptığı satır değişiklikleri */
  added: number
  removed: number
  /** Bu çağrının yaptığı düzenlemeler (önce→sonra): kart açılınca cdesktop/Claude Code gibi gerçek diff gösterilir.
   *  multi_edit birden çok dosyaya dokunabilir; chat.changes ise oturum-başı→son hâl toplamıdır (yan panel). */
  edits?: FileEdit[]
  /** Onay beklerken: yapılacak değişikliğin önizlemesi (cdesktop onay kartı diff'i) */
  preview?: FileEdit
  /** [review] Onay beklerken yazılacak dosyalar (dosya bazında kabul/düzenle/reddet, §12.3) */
  files?: ReviewFile[]
  /** [review] Yıkıcı işlemin neden anahtarı (kırmızı işaret) */
  destructive?: string
  /** §28.3 düzenlemenin kural uyumu (ihlal kırmızı, ilgili sarı, uyulan yeşil) */
  rules?: RuleFinding[]
  /** Bu araç bir alt ajana aitse, alt ajanı başlatan agent çağrısının kimliği */
  parentId?: string
  /** agent aracı için alt ajan durumu */
  subagent?: SubagentState
  /** §23.4 tek satır: ikon · fiil · hedef · süre (epoch ms; kayıttan açılan araçlarda yok) */
  startedAt?: number
  endedAt?: number
  /** Çalışan komutun canlı görünümü: önce `$ komut`, ardından çıktı satırları (Claude Code Bash kartı) */
  live?: string[]
}

export type ChatItem =
  | { kind: 'user'; id: string; text: string; attachments: string[] }
  | { kind: 'assistant'; id: string; text: string }
  | ToolItem
  /** alert: tura bağlı olmayan bir işlem başarısız oldu (kırmızı, ekran okuyucuya duyurulur) */
  | { kind: 'notice'; id: string; text: string; alert?: true }
  | { kind: 'error'; id: string; text: string }
  // [g11] isteğe göre otomatik devreye giren skiller
  | { kind: 'turnEnd'; id: string; durationMs: number }
  // [g6] bağlam sıkıştırma ayracı; token sayıları yalnızca canlı olayda bilinir (kayıttan açılınca yoktur)
  | {
      kind: 'compaction'
      id: string
      summary: string
      beforeTokens?: number
      afterTokens?: number
      /** Sıkıştırma anı (epoch ms): ayraçta saat olarak gösterilir */
      at?: number
      /** Sıkıştırmayı yapan model */
      model?: string
      /** Kullanıcı düzenleyebilsin diye özet gövdesi; yoksa summary kullanılır */
      text?: string
      /** Özetleme sürüyor (ayraçta "özetleniyor…") */
      running?: boolean
      /** Düzenlenen özeti kaydeder; false dönerse düzenleme açık kalır */
      onSave?: (body: string) => Promise<boolean> | boolean
    }
  | G3ChatItem // [g3] düşünme, model notu, değişiklik özeti

export interface FileChange {
  path: string
  before: string | null
  after: string
  /** [perf] İçerik henüz alınmadı (büyük dosya); SidePanel getFileChange ile doldurur */
  lazy?: { added: number; removed: number; bytes: number }
}

/** Tek bir araç çağrısının tek bir dosyada yaptığı düzenleme: o adımın öncesi → sonrası. */
export interface FileEdit {
  path: string
  previous: string | null
  after: string
}

export interface ChatState {
  items: ChatItem[]
  changes: FileChange[]
  running: boolean
  /** Çalışan turun başlangıcı (epoch ms); durum satırındaki sayaç için */
  turnStartedAt: number | null
  /** [g3] zaman damgaları ve tur içi dosya değişiklikleri */
  g3?: G3State
  /** Kayıt store'da yokken gelen olaylarla oluştu: geçmiş henüz yüklenmedi, açılınca dökümle birleşir. */
  partial?: true
}

export type ChatAction =
  | { type: 'evict' }
  | { type: 'hydrateChange'; path: string; before: string | null; after: string }
  | { type: 'event'; event: AgentEventBody; at: number }
  | { type: 'userSent'; text: string; attachments: string[]; at: number }
  | { type: 'approved'; id: string }
  | { type: 'reverted'; path: string }
  | { type: 'reset' }
  /** runningSince: ana süreçte tur hâlâ sürüyorsa açılış anı (epoch ms) */
  | { type: 'load'; transcript: TranscriptItem[]; runningSince?: number }
  /** Gönderim başarısız: tur hiç başlamadı, çalışıyor durumu kapanır */
  | { type: 'failed'; message: string }
  /** Tura bağlı olmayan hata (sabitleme, model değiştirme...): tur etkilenmez */
  | { type: 'notify'; message: string }
  // [g4] geri sarma: değişiklik listesi ana süreçten gelen listeyle değişir; istenirse konuşma o mesajdan kesilir
  | { type: 'rewound'; changes: FileChange[]; cutFromItemId: string | null }
  | G2ChatAction // [g2]

export const initialChatState: ChatState = { items: [], changes: [], running: false, turnStartedAt: null }

let counter = 0
const nextId = () => `local-${++counter}`

export function lineStats(before: string | null, after: string): { added: number; removed: number } {
  let added = 0
  let removed = 0
  for (const part of diffLines(before ?? '', after)) {
    if (part.added) added += part.count ?? 0
    if (part.removed) removed += part.count ?? 0
  }
  return { added, removed }
}

function updateTool(items: ChatItem[], id: string, patch: (item: ToolItem) => Partial<ToolItem>): ChatItem[] {
  return items.map((item) => (item.kind === 'tool' && item.id === id ? { ...item, ...patch(item) } : item))
}

/** Araç sonuçlanmadan tur bitti (ör. PreToolUse kancası hata verdi). */
export const TOOL_STOPPED = 'Durduruldu: tur bu araç sonuçlanmadan bitti.'
export const TOOL_FAILURE = /^(Hata:|Engellendi:|Kullanıcı bu işlemi reddetti|Durduruldu: tur)/

/** Hâlâ sürmekte olan veya kullanıcıdan karar bekleyen araç: katlanmaz, bellekten düşürülmez. */
export const isLiveItem = (i: ChatItem): boolean =>
  i.kind === 'tool' && (i.awaitingApproval || i.result === null || i.subagent?.status === 'running')

/** Tur sonu: açık kalan ön plan araçları durdurulmuş sayılır. Arka plan alt ajanları ve araçları çalışmaya devam eder. */
function closeOpenTools(items: readonly ChatItem[]): ChatItem[] {
  const background = new Set<string>()
  for (const i of items) if (i.kind === 'tool' && i.subagent?.background && i.subagent.status === 'running') background.add(i.id)
  return items.map((i) => {
    if (i.kind !== 'tool' || i.subagent?.background || (i.parentId && background.has(i.parentId))) return i
    const sub = i.subagent?.status === 'running' ? i.subagent : undefined
    if (i.result !== null && !sub) return i
    return { ...i, result: i.result ?? TOOL_STOPPED, awaitingApproval: false, ...(sub ? { subagent: { ...sub, status: 'stopped' as const } } : {}) }
  })
}

function endTurn(state: ChatState, at: number, extra: ChatItem[] = []): ChatState {
  const durationMs = state.turnStartedAt === null ? 0 : at - state.turnStartedAt
  return {
    ...state,
    running: false,
    turnStartedAt: null,
    items: [...closeOpenTools(state.items), ...extra, { kind: 'turnEnd', id: nextId(), durationMs }],
  }
}

/** Rapor başlığı model içindir; arayüzde tekrar göstermeye gerek yok. */
const REPORT_HEADER = /^\[[^\]\n]+ alt ajanı raporu\]\n/
const stripReportHeader = (text: string) => text.replace(REPORT_HEADER, '')

/** "<task-notification>\nArka plan alt ajanı tamamlandı (...)\n..." → ilk anlamlı satır */
function noticeTitle(text: string): string {
  const line = text.replace(/<\/?task-notification>/g, '').trim().split('\n')[0] ?? ''
  return line.length > 160 ? `${line.slice(0, 159)}…` : line
}

/** Alt ajana ait olmayan öğe: akışın sırası bunlarla belirlenir. */
export const isTopLevel = (i: ChatItem): boolean => !(i.kind === 'tool' && i.parentId)

function applyEvent(state: ChatState, event: AgentEventBody, at: number): ChatState {
  switch (event.type) {
    case 'turnStart':
      // Kullanıcı gönderdiğinde userSent zaten başlattı; bildirimle açılan turlarda burada başlar.
      return state.running ? state : { ...state, running: true, turnStartedAt: at }
    case 'text': {
      // Arka plan alt ajanının aracı araya girse de yanıt bölünmez: alt ajan öğeleri atlanır.
      const index = state.items.findLastIndex(isTopLevel)
      const last = state.items[index]
      if (last?.kind === 'assistant') {
        return { ...state, items: state.items.with(index, { ...last, text: last.text + event.delta }) }
      }
      // Araçlar arasındaki yalnız satır sonları yeni bir balon açmasın.
      if (!event.delta.trim()) return state
      return { ...state, items: [...state.items, { kind: 'assistant', id: nextId(), text: event.delta }] }
    }
    case 'tool':
      // Yeniden yüklemede canlı kayıt aynı aracı tekrar gönderebilir.
      if (state.items.some((i) => i.kind === 'tool' && i.id === event.id)) return state
      return {
        ...state,
        items: [
          ...state.items,
          {
            kind: 'tool',
            id: event.id,
            name: event.name,
            summary: event.summary,
            result: null,
            awaitingApproval: false,
            added: 0,
            removed: 0,
            startedAt: at,
            ...(event.parentId ? { parentId: event.parentId } : {}),
          },
        ],
      }
    case 'confirm': {
      const review = event as typeof event & ConfirmReviewFields // [review]
      return {
        ...state,
        items: updateTool(state.items, event.id, () => ({
          awaitingApproval: true,
          ...(event.warning ? { warning: event.warning } : {}),
          ...(event.preview ? { preview: { path: event.preview.path, previous: event.preview.before, after: event.preview.after } } : {}),
          ...(review.files?.length ? { files: review.files } : {}),
          ...(review.destructive ? { destructive: review.destructive } : {}),
          ...(review.rules?.length ? { rules: review.rules } : {}),
        })),
      }
    }
    case 'toolProgress': {
      // Çok uzun çıktıda yalnızca son satırlar canlı tutulur; tam çıktı sonuçta gelir.
      const MAX_LIVE = 400
      return {
        ...state,
        items: updateTool(state.items, event.id, (t) => {
          const live = [...(t.live ?? []), event.line]
          return { live: live.length > MAX_LIVE ? [live[0]!, ...live.slice(-(MAX_LIVE - 1))] : live }
        }),
      }
    }
    case 'toolResult':
      return {
        ...state,
        items: updateTool(state.items, event.id, (t) => ({
          result: event.result,
          awaitingApproval: false,
          endedAt: at,
          // Ön plan alt ajanının sonucu raporudur; arka planda olanın sonucu yalnızca "başlatıldı" bilgisidir.
          ...(t.subagent && !t.subagent.background
            ? { subagent: { ...t.subagent, status: TOOL_FAILURE.test(event.result) ? 'failed' : 'done', report: stripReportHeader(event.result) } }
            : {}),
        })),
      }
    case 'subagent':
      return {
        ...state,
        items: updateTool(state.items, event.id, () => ({
          subagent: {
            agentType: event.agentType,
            description: event.description,
            background: event.background,
            model: event.model,
            status: 'running',
            report: null,
            startedAt: at,
          },
        })),
      }
    case 'subagentDone':
      return {
        ...state,
        // Arka plan alt ajanında tur sonu yoktur: bitince açık kalan araçları burada kapanır.
        items: updateTool(state.items, event.id, (t) =>
          t.subagent ? { subagent: { ...t.subagent, status: event.ok ? 'done' : 'failed', report: event.result } } : {},
        ).map((i) => (i.kind === 'tool' && i.parentId === event.id && i.result === null ? { ...i, result: TOOL_STOPPED, awaitingApproval: false } : i)),
      }
    case 'notice':
      return { ...state, items: [...state.items, { kind: 'notice', id: nextId(), text: noticeTitle(event.text) }] }
    case 'fileChanged': {
      const change: FileChange = event.large
        ? { path: event.path, before: null, after: '', lazy: event.large }
        : { path: event.path, before: event.before, after: event.after }
      const others = state.changes.filter((c) => c.path !== event.path)
      // Değişikliği yapan araca yaz: grup satırındaki +/- toplamı buradan gelir.
      const target =
        (event.toolId ? state.items.find((i): i is ToolItem => i.kind === 'tool' && i.id === event.toolId) : undefined) ??
        state.items.findLast((i): i is ToolItem => i.kind === 'tool' && i.result === null)
      const stats = event.large ?? lineStats(event.previous, event.after)
      const items = target
        ? updateTool(state.items, target.id, (t) => ({
            added: t.added + stats.added,
            removed: t.removed + stats.removed,
            // Bu adımın önce→sonra çifti kartta diff olarak gösterilir (toplam değil, bu düzenleme); büyük dosyada atlanır.
            ...(event.large ? {} : { edits: [...(t.edits ?? []), { path: event.path, previous: event.previous, after: event.after }] }),
          }))
        : state.items
      return { ...state, items, changes: [...others, change] }
    }

    case 'done':
      return endTurn(state, at)
    case 'error':
      return endTurn(state, at, [{ kind: 'error', id: nextId(), text: event.message }])
    case 'g2:queueConsumed': // [g2]
      return g2QueueConsumed(state, event)
    // [g6] bağlam göstergesi kendi aboneliğiyle günceller; akışa yalnızca sıkıştırma ayracı girer
    case 'g6:context':
      return state
    case 'g6:compacted':
      // Model adı ve zaman ayraçta gösterilir; gövde düzenlenebilir olsun diye text de taşınır.
      return {
        ...state,
        items: [
          ...state.items.filter((i) => !(i.kind === 'compaction' && i.running)),
          { kind: 'compaction', id: nextId(), summary: event.summary, text: event.summary, beforeTokens: event.beforeTokens, afterTokens: event.afterTokens, at, model: event.model },
        ],
      }
    // [g6] özetleme sürüyor: geçici bir ayraç; bitince g6:compacted onun yerini alır
    case 'g6:compactionState':
      return event.running
        ? { ...state, items: [...state.items, { kind: 'compaction', id: `g6-running-${at}`, summary: '', running: true }] }
        : { ...state, items: state.items.map((i) => (i.kind === 'compaction' && i.running ? { ...i, running: false } : i)) }
    // [g6] sıkıştırma başarısız: görünür uyarı (sessiz console.warn yerine)
    case 'g6:compactionFailed':
      return {
        ...state,
        items: [
          ...state.items.filter((i) => !(i.kind === 'compaction' && i.running)),
          { kind: 'error', id: nextId(), text: `Bağlam sıkıştırılamadı: ${event.message}` },
        ],
      }
    // [g11] otomatik devreye giren skiller: aynı isim kümesi art arda gelirse tek satırda kalır
    // [g3] diğer grupların olayları kendi katmanlarında işlenir
    default:
      return state
  }
}

export function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case 'event':
      return applyEvent(state, action.event, action.at)
    case 'userSent':
      return {
        ...state,
        running: true,
        turnStartedAt: action.at,
        items: [...state.items, { kind: 'user', id: nextId(), text: action.text, attachments: action.attachments }],
      }
    case 'approved':
      return { ...state, items: updateTool(state.items, action.id, () => ({ awaitingApproval: false })) }
    case 'reverted':
      return { ...state, changes: state.changes.filter((c) => c.path !== action.path) }
    case 'reset':
      return initialChatState
    case 'evict':
      return initialChatState // bellekten düşürüldü; storeReducer anahtarı siler
    case 'hydrateChange':
      return { ...state, changes: state.changes.map((c) => (c.path === action.path ? { path: c.path, before: action.before, after: action.after } : c)) }
    case 'load': {
      const loaded = action.transcript.map((t): ChatItem => {
        if (t.kind === 'tool') {
          return {
            ...t,
            awaitingApproval: false,
            added: 0,
            removed: 0,
            ...(t.name === 'agent' ? { subagent: restoredSubagent(t.summary, t.result) } : {}),
          }
        }
        return t.kind === 'notice' ? { kind: 'notice', id: nextId(), text: noticeTitle(t.text) } : { ...t, id: nextId() }
      })
      // Kısmi kayıttaki canlı durum korunur; tam kayıt dökümle değişir.
      const { partial: _partial, ...live } = state.partial ? state : initialChatState
      const resumed = action.runningSince !== undefined && !live.running && !live.items.some((i) => i.kind === 'turnEnd')
      return {
        ...live,
        items: mergeLiveItems(loaded, live.items),
        running: live.running || resumed,
        turnStartedAt: live.turnStartedAt ?? (resumed ? (action.runningSince ?? null) : null),
      }
    }
    case 'failed':
      return {
        ...state,
        running: false,
        turnStartedAt: null,
        items: [...closeOpenTools(state.items), { kind: 'error', id: nextId(), text: action.message }],
      }
    case 'notify':
      return { ...state, items: [...state.items, { kind: 'notice', id: nextId(), text: action.message, alert: true }] }
    // [g4] geri sarma
    case 'rewound': {
      const cut = action.cutFromItemId === null ? -1 : state.items.findIndex((i) => i.id === action.cutFromItemId)
      return { ...state, changes: action.changes, items: cut >= 0 ? state.items.slice(0, cut) : state.items }
    }
    case 'g2:rewind': // [g2]
      return g2Rewind(state, action)
  }
}

/** Kayıttan açılan sohbette agent çağrısı: özet "Tip: açıklama" biçimindedir. */
function restoredSubagent(summary: string, result: string): SubagentState {
  const [agentType = 'agent', ...rest] = summary.split(': ')
  const background = result.startsWith('Arka plan alt ajanı başlatıldı')
  return {
    agentType,
    description: rest.join(': ').replace(/ \(arka plan\)$/, ''),
    background,
    model: '',
    status: TOOL_FAILURE.test(result) ? 'failed' : 'done',
    report: background ? null : stripReportHeader(result),
  }
}

/**
 * Dökümün üstüne yükleme öncesi gelen canlı öğeler: dökümde zaten olanlar tekrar eklenmez.
 * Canlı araç kimliği kayıttakinden farklı olduğu için araçlar ad ve özetle eşlenir.
 */
function mergeLiveItems(loaded: ChatItem[], live: readonly ChatItem[]): ChatItem[] {
  const merged = [...loaded]
  const liveIds = new Set(live.map((i) => i.id))
  for (const item of live) {
    if (item.kind === 'compaction' && !item.running) continue
    if ((item.kind === 'assistant' || item.kind === 'user' || item.kind === 'notice') && merged.some((o) => o.kind === item.kind && o.text.includes(item.text))) continue
    if (item.kind === 'tool') {
      // Kor arka ucu çağrı kimliğini dökümde de korur: aynı kimlikli kayıt bu canlı aracın kendisidir.
      const same = merged.findIndex((o) => o.kind === 'tool' && o.id === item.id)
      const known = merged[same]
      if (known?.kind === 'tool') {
        merged[same] = { ...item, result: item.result ?? (known.result || null) }
        continue
      }
    }
    if (item.kind === 'tool' && !item.parentId) {
      // Süren araç, kayıttaki sonuçsuz tool_call'dır (özet MCP gibi araçlarda farklı olabilir; aynı adlılar başlama sırasıyla eşlenir).
      const open = item.result === null ? merged.findIndex((o) => o.kind === 'tool' && !liveIds.has(o.id) && o.name === item.name && o.result === '') : -1
      const at = open >= 0 ? open : merged.findIndex((o) => o.kind === 'tool' && !liveIds.has(o.id) && o.name === item.name && o.summary === item.summary)
      const known = merged[at]
      if (known?.kind === 'tool') {
        merged[at] = { ...item, result: item.result ?? (known.result || null), ...(known.subagent && !item.subagent ? { subagent: known.subagent } : {}) }
        continue
      }
    }
    merged.push(item)
  }
  return merged
}

/** Ana süreçteki süren turun olayları; 'load' bunları kayıttaki tool_call'larla eşler. */
export function liveActions(live: LiveSession, now = Date.now()): ChatAction[] {
  const at = live.since ?? now
  return [...(live.since !== null ? [{ type: 'turnStart' } as const] : []), ...live.events].map((event) => ({ type: 'event', event, at }))
}

/** Dökümü yüklenmeli mi: kayıt yok veya yalnızca canlı olaylardan oluşmuş. */
export const needsTranscript = (chat: ChatState | undefined): boolean => !chat || chat.partial === true

/** Sohbet kimliğine göre tüm sohbetlerin durumu: arka planda çalışanlar da güncel kalır. */
export type ChatStore = Readonly<Record<string, ChatState>>

export function storeReducer(store: ChatStore, action: { sessionId: string; action: ChatAction }): ChatStore {
  // [perf] Boşta kalan sohbetler store'dan tamamen çıkar (tekrar açılınca diskten yüklenir).
  if (action.action.type === 'evict') {
    if (!(action.sessionId in store)) return store
    const { [action.sessionId]: _gone, ...rest } = store
    return rest
  }
  const current = store[action.sessionId]
  const next = g3Reduce(current ?? initialChatState, action.action, chatReducer) // [g3] katman
  // Bellekten düşmüş sohbete olay geldi: bu kayıt geçmişi içermez, açılışta yükleme atlanmamalı.
  return { ...store, [action.sessionId]: !current && action.action.type === 'event' ? { ...next, partial: true } : next }
}
