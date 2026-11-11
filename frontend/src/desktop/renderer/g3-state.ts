// [g3] Sohbet durumuna eklenen akış öğeleri: düşünme bloğu, model notu, tur sonu değişiklik özeti, zaman damgaları.
import { decodeInteractive, type TodoItem } from '../../core/interactive.ts'
import { isTopLevel, lineStats, type ChatAction, type ChatItem, type ChatState } from './chat-state.ts'

export interface TurnFile {
  path: string
  added: number
  removed: number
  /** Dosya bu turda oluşturuldu */
  created: boolean
}

export type G3ChatItem =
  | { kind: 'thinking'; id: string; text: string; startedAt: number; endedAt: number }
  | { kind: 'modelNote'; id: string; from: string; to: string }
  | { kind: 'changeSummary'; id: string; files: TurnFile[] }

export interface G3State {
  /** Öğe kimliği → oluşturulma anı (epoch ms); üzerine gelince gösterilen zaman damgası */
  times: Readonly<Record<string, number>>
  /** Çalışan turda değişen dosyalar; tur bitince özet kartına dönüşür */
  turnFiles: readonly TurnFile[]
}

const EMPTY: G3State = { times: {}, turnFiles: [] }

let counter = 0
const nextId = () => `g3-${++counter}`

function appendReasoning(state: ChatState, delta: string, at: number): ChatState {
  const index = state.items.findLastIndex(isTopLevel)
  const last = state.items[index]
  if (last?.kind === 'thinking') {
    return { ...state, items: state.items.with(index, { ...last, text: last.text + delta, endedAt: at }) }
  }
  // Yanıt akarken araya giren düşünme parçası yanıtı bölmesin ("Go" | "ogle"): boşsa atlanır, doluysa
  // yanıtın önündeki düşünme bloğuna eklenir (yoksa yanıtın önüne yeni blok konur).
  if (last?.kind === 'assistant') {
    if (!delta.trim()) return state
    const before = state.items.findLastIndex((item, i) => i < index && isTopLevel(item))
    const prev = state.items[before]
    if (prev?.kind === 'thinking') {
      return { ...state, items: state.items.with(before, { ...prev, text: prev.text + delta, endedAt: at }) }
    }
    const thinking = { kind: 'thinking' as const, id: nextId(), text: delta, startedAt: at, endedAt: at }
    return { ...state, items: [...state.items.slice(0, index), thinking, ...state.items.slice(index)] }
  }
  return { ...state, items: [...state.items, { kind: 'thinking', id: nextId(), text: delta, startedAt: at, endedAt: at }] }
}

/** Sona eklenen yeni öğelere zaman damgası basar; yalnızca kuyruk taranır (metin akışında O(1)). */
function stampTimes(times: G3State['times'], items: readonly ChatItem[], at: number): G3State['times'] {
  let next: Record<string, number> | null = null
  for (let i = items.length - 1; i >= 0; i--) {
    const id = items[i]?.id
    if (id === undefined || id in times) break
    next ??= { ...times }
    next[id] = at
  }
  return next ?? times
}

function addTurnFile(files: readonly TurnFile[], path: string, stats: { added: number; removed: number }, created: boolean): TurnFile[] {
  const existing = files.find((f) => f.path === path)
  if (!existing) return [...files, { path, added: stats.added, removed: stats.removed, created }]
  return files.map((f) => (f.path === path ? { ...f, added: f.added + stats.added, removed: f.removed + stats.removed } : f))
}

/** Değişiklik özeti, turun sonundaki "sonraki adım" kutusunun ve tur sonu satırının üstüne yerleşir. */
function summaryIndex(items: readonly ChatItem[]): number {
  let i = items.length
  while (i > 0) {
    const item = items[i - 1]
    const trailing = item?.kind === 'turnEnd' || (item?.kind === 'tool' && (item.name === 'suggest_next' || HIDDEN_TOOLS.has(item.name)))
    if (!trailing) break
    i--
  }
  return i
}

/**
 * Temel reducer'ın etrafındaki katman: g3 olaylarını kendisi işler, diğerlerini temel reducer'a
 * bırakıp sonucu zenginleştirir (zaman damgası, değişiklik özeti).
 */
export function g3Reduce(state: ChatState, action: ChatAction, base: (s: ChatState, a: ChatAction) => ChatState): ChatState {
  let next: ChatState
  if (action.type === 'event' && action.event.type === 'g3:reasoning') next = appendReasoning(state, action.event.delta, action.at)
  else if (action.type === 'event' && action.event.type === 'g3:model') {
    next = { ...state, items: [...state.items, { kind: 'modelNote', id: nextId(), from: action.event.from, to: action.event.to }] }
  } else next = base(state, action)

  if (action.type === 'reset' || action.type === 'load') return next
  let g3 = next.g3 ?? EMPTY

  if (action.type === 'event') {
    const event = action.event
    // Büyük dosyada içerik gelmez (previous null, after boş): istatistik ana süreçten, "yeni dosya" sayılmaz.
    if (event.type === 'fileChanged') {
      const stats = event.large ?? lineStats(event.previous, event.after)
      g3 = { ...g3, turnFiles: addTurnFile(g3.turnFiles, event.path, stats, !event.large && event.previous === null) }
    }
    if ((event.type === 'done' || event.type === 'error') && g3.turnFiles.length > 0) {
      const summary: ChatItem = { kind: 'changeSummary', id: nextId(), files: [...g3.turnFiles] }
      const at = summaryIndex(next.items)
      next = { ...next, items: [...next.items.slice(0, at), summary, ...next.items.slice(at)] }
      g3 = { ...g3, turnFiles: [] }
    }
  }
  if (action.type === 'event' || action.type === 'userSent') g3 = { ...g3, times: stampTimes(g3.times, next.items, action.at) }
  return g3 === next.g3 ? next : { ...next, g3 }
}

/** Akışta kart olarak çizilen araçlar; todo_write akışta görünmez, alttaki sabit listede gösterilir. */
export const CARD_TOOLS: ReadonlySet<string> = new Set(['mark_chapter', 'suggest_next', 'ask_user', 'exit_plan_mode'])
export const HIDDEN_TOOLS: ReadonlySet<string> = new Set(['todo_write'])

/**
 * Son turun üst düzey (alt ajana ait olmayan) son geçerli todo_write listesi; önceki turların listesi geri dönmez.
 * Tur sürüyorsa önceki tur sonu sınırdır; bittiyse son tur sonu o turun kendisine aittir, liste görünür kalır.
 */
export function latestTodos(items: readonly ChatItem[], running = false): TodoItem[] | null {
  let turnEnds = running ? 1 : 0
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i]
    // Kuyruktan tüketilen mesaj (g2-q-) aynı turun içindedir.
    if (item?.kind === 'user' && !item.id.startsWith('g2-q-')) return null
    if (item?.kind === 'turnEnd' && ++turnEnds > 1) return null
    if (item?.kind !== 'tool' || item.name !== 'todo_write' || item.parentId) continue
    if (item.result?.startsWith('Hata:')) continue
    const payload = decodeInteractive(item.name, item.summary)
    if (payload?.tool === 'todo_write') return payload.args.items
  }
  return null
}

export interface Chapter {
  id: string
  title: string
  summary: string | undefined
}

export const chapterAnchor = (toolId: string) => `g3-chapter-${toolId}`

export function chaptersOf(items: readonly ChatItem[]): Chapter[] {
  return items.flatMap((item) => {
    if (item.kind !== 'tool' || item.name !== 'mark_chapter' || item.parentId) return []
    const payload = decodeInteractive(item.name, item.summary)
    return payload?.tool === 'mark_chapter' ? [{ id: item.id, title: payload.args.title, summary: payload.args.summary }] : []
  })
}

/** Kartın düğmeleri yalnızca son kullanıcı mesajından sonra gelen (son turdaki) kartta etkindir. */
export function isInLastTurn(items: readonly ChatItem[], id: string): boolean {
  const index = items.findIndex((i) => i.id === id)
  return index !== -1 && !items.slice(index + 1).some((i) => i.kind === 'user')
}

export function lastUserMessage(items: readonly ChatItem[]): Extract<ChatItem, { kind: 'user' }> | undefined {
  return items.findLast((i): i is Extract<ChatItem, { kind: 'user' }> => i.kind === 'user')
}
