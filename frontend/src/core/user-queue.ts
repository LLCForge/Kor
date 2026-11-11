import { randomUUID } from 'node:crypto'
import type { ChatCompletionContentPart, ChatCompletionContentPartImage, ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import type { AgentIO, Session } from './agent.ts'
import { READ_IMAGE_TAG, type UserContent } from './message-text.ts'
import type { ReasoningEffortLevel } from './reasoning.ts'

export interface QueuedUserMessage {
  id: string
  content: UserContent
  /** Arayüzde kullanıcı mesajı olarak gösterilecek metin ve ekler */
  text: string
  attachments: string[]
  /** Geri alınan turdan döndü: arayüzde zaten kullanıcı balonu olarak görünüyor */
  shown?: boolean
}

export type QueuedMessageInfo = Pick<QueuedUserMessage, 'id' | 'text' | 'attachments'>

export interface ConsumedQueueItem {
  item: QueuedUserMessage
  message: ChatCompletionMessageParam
}

/** Grup 2'nin oturum durumu: Session'a tek alan olarak eklenir. */
export interface G2SessionState {
  /** Ajan çalışırken gönderilen, bir sonraki model çağrısından önce eklenecek mesajlar */
  queue: QueuedUserMessage[]
  /** Turun ilk model çağrısından önce son kullanıcı mesajına eklenecek görseller */
  pendingImages: ChatCompletionContentPartImage[]
  /** Model destekliyorsa istekte gönderilecek reasoning_effort */
  reasoningEffort: ReasoningEffortLevel | null
  /** false: model görsel girişini desteklemiyor; null: bilinmiyor */
  vision: boolean | null
  /** Araya girme sürerken başka yoldan tur başlatılmasın */
  interrupting: boolean
}

export function g2State(session: Session): G2SessionState {
  session.g2 ??= { queue: [], pendingImages: [], reasoningEffort: null, vision: null, interrupting: false }
  return session.g2
}

export function createQueuedMessage(content: UserContent, text: string, attachments: string[]): QueuedUserMessage {
  return { id: randomUUID(), content, text, attachments }
}

export const queuedInfo = ({ id, text, attachments }: QueuedUserMessage): QueuedMessageInfo => ({ id, text, attachments })

// read_image görselleri araç sonucuna bu belirteçle bağlanır: araç sonucu metin olmak zorunda,
// görsel bir sonraki model çağrısından önce kullanıcı mesajı olarak eklenir.
const IMAGE_REF = /\[görsel-eki:([0-9a-f-]{36})\]/g
const MAX_PENDING_TOOL_IMAGES = 32
const toolImages = new Map<string, { path: string; url: string }>()

/** Araç sonucuna yazılacak belirteci döner. */
export function registerToolImage(relPath: string, dataUrl: string): string {
  const id = randomUUID()
  toolImages.set(id, { path: relPath, url: dataUrl })
  // Tüketilmeyen (iptal edilen tur) görseller belleği şişirmesin: en eskiler atılır.
  for (const key of toolImages.keys()) {
    if (toolImages.size <= MAX_PENDING_TOOL_IMAGES) break
    toolImages.delete(key)
  }
  return `[görsel-eki:${id}]`
}

const toParts = (content: UserContent): ChatCompletionContentPart[] =>
  typeof content === 'string' ? (content ? [{ type: 'text', text: content }] : []) : [...content]

/** Son model çağrısından sonra eklenen araç sonuçlarındaki görsel belirteçlerini kullanıcı mesajına çevirir. */
function attachToolImages(session: Session): void {
  const found: { path: string; url: string }[] = []
  for (let i = session.messages.length - 1; i >= 0; i--) {
    const m = session.messages[i]
    if (m?.role === 'assistant') break
    // Araç sonuçlarından sonra not (kullanıcı rolü) gelmiş olabilir; sınır son model yanıtıdır.
    if (m?.role !== 'tool' || typeof m.content !== 'string') continue
    for (const match of m.content.matchAll(IMAGE_REF)) {
      const id = match[1] ?? ''
      const image = toolImages.get(id)
      if (!image) continue
      toolImages.delete(id)
      found.unshift(image)
    }
  }
  if (found.length === 0) return
  const paths = found.map((f) => f.path).join(', ')
  if (session.g2?.vision === false) {
    session.messages.push({
      role: 'user',
      content: `${READ_IMAGE_TAG} path="${paths}">Görsel eklenemedi: seçili model görsel girişini desteklemiyor.</read-image>`,
    })
    return
  }
  session.messages.push({
    role: 'user',
    content: [
      { type: 'text', text: `${READ_IMAGE_TAG} path="${paths}">read_image ile istenen görsel(ler):</read-image>` },
      ...found.map((f): ChatCompletionContentPartImage => ({ type: 'image_url', image_url: { url: f.url } })),
    ],
  })
}

/** runTurn'da her model çağrısından önce: bekleyen görseller, read_image çıktıları ve kuyruktaki kullanıcı mesajları. Tüketilenler döner (geri alma için). */
export function g2BeforeModelCall(session: Session, io: AgentIO, step: number): ConsumedQueueItem[] {
  const state = session.g2
  if (state && state.pendingImages.length > 0) {
    const images = state.pendingImages.splice(0)
    const last = session.messages.at(-1)
    // Yalnızca turun ilk çağrısında anlamlıdır: görseller turu başlatan mesajın parçasıdır.
    if (step === 0 && last?.role === 'user') last.content = [...toParts(last.content), ...images]
  }
  attachToolImages(session)
  if (state && state.queue.length > 0) {
    const items = state.queue.splice(0)
    const consumed = items.map((item): ConsumedQueueItem => ({ item, message: { role: 'user', content: item.content } }))
    for (const c of consumed) session.messages.push(c.message)
    const fresh = items.filter((item) => !item.shown)
    if (fresh.length > 0) io.g2QueueConsumed?.(fresh.map(queuedInfo))
    return consumed
  }
  return []
}

/** Tur geri alınınca tüketilen mesajlar kuyruğun başına döner; sonraki turda modele gider. */
export function g2RestoreQueue(session: Session, items: readonly QueuedUserMessage[]): void {
  if (items.length > 0) g2State(session).queue.unshift(...items.map((item) => ({ ...item, shown: true })))
}

/** İstek gövdesine eklenecek alanlar (Venice: reasoning_effort, docs.venice.ai chat/completions). */
export function g2RequestFields(session: Session): { reasoning_effort?: ReasoningEffortLevel } {
  const effort = session.g2?.reasoningEffort
  return effort ? { reasoning_effort: effort } : {}
}
