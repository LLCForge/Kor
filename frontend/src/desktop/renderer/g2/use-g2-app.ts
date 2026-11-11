// App düzeyindeki grup 2 durumu (kuyruk, bekleyen görseller) ve mesaj kutusu / mesaj eylemleri için köprü.
// App.tsx Composer ve Thread'e yeni prop geçirmez (çakışmayı azaltmak için): bileşenler köprüyü
// g2Bridge.current üzerinden okur. App her çizimde köprüyü yeniler; çocuklar App'ten sonra çizildiği için güncel kalır.
import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { isHumanUserText } from '../../../core/message-text.ts'
import type { G2SendPayload } from '../../ipc-g2.ts'
import { api, errorMessage } from '../api.ts'
import type { ChatAction, ChatItem } from '../chat-state.ts'
import type { PendingImage } from './images.ts'
import { getG2Prefs } from './prefs.ts'

export interface QueuedView {
  id: string
  text: string
  /** Kullanıcı mesajında gösterilecek ek adları (görseller dahil) */
  attachments: string[]
  /** Geri alınınca taslağa, "Şimdi gönder"de yeniden gönderime kullanılır */
  payload: { text: string; attachments: string[]; images: PendingImage[] }
}

interface AppActions {
  newChat(): void
  openMemory(): void
  openSystemPrompt(): void
  pickFiles(): void
}

export interface G2Bridge extends AppActions {
  /** Etkin sohbet (yeni sohbet taslağında da dolu) */
  activeId: string
  queued: readonly QueuedView[]
  images: readonly PendingImage[]
  setImages(update: (prev: readonly PendingImage[]) => PendingImage[]): void
  setAttachments(update: (prev: string[]) => string[]): void
  /** Boştaysa gönderir, çalışıyorsa kuyruğa alır; interrupt: turu kesip hemen gönderir. Hata olursa taslak geri konur. */
  send(input: { text: string; interrupt: boolean }): Promise<void>
  sendQueuedNow(id: string): Promise<void>
  unqueue(id: string): Promise<void>
  /** Kuyruktakileri taslağa geri koyar ve turu durdurur */
  abort(): Promise<void>
  editMessage(itemId: string, items: readonly ChatItem[], newText: string): Promise<void>
  /** Hata sohbete hata kutusu olarak düşer */
  regenerate(items: readonly ChatItem[]): Promise<void>
}

export const g2Bridge: { current: G2Bridge | null } = { current: null }

interface UseG2AppArgs {
  activeId: string
  dispatchStore: Dispatch<{ sessionId: string; action: ChatAction }>
  draft: string
  setDraft: Dispatch<SetStateAction<string>>
  attachments: string[]
  setAttachments: Dispatch<SetStateAction<string[]>>
}

type QueueMap = Readonly<Record<string, readonly QueuedView[]>>
type UserItem = Extract<ChatItem, { kind: 'user' }>

const isHumanItem = (i: ChatItem): i is UserItem => i.kind === 'user' && isHumanUserText(i.text)
const toPayloadImages = (images: readonly PendingImage[]) => images.map(({ name, dataUrl }) => ({ name, dataUrl }))
// §10 (ADR 0048): görsel, ana süreçte yerel OCR ile metne çevrilip mesaja eklenir; etiket bunu söyler.
const imageNames = (images: readonly PendingImage[]) => images.map((i) => `görsel → OCR metni: ${i.name}`)

export function useG2App(o: UseG2AppArgs): { bind(actions: AppActions): void } {
  const [queued, setQueued] = useState<QueueMap>({})
  // Olay dinleyicisi ve ardışık IPC yanıtları en güncel kuyruğu görsün diye ref eşzamanlı güncellenir.
  const queuedRef = useRef<QueueMap>(queued)
  const [images, setImages] = useState<readonly PendingImage[]>([])
  const imagesRef = useRef(images)
  imagesRef.current = images
  const latest = useRef(o)
  latest.current = o
  const actionsRef = useRef<AppActions | null>(null)

  const updateQueued = useCallback((sessionId: string, fn: (list: readonly QueuedView[]) => readonly QueuedView[]) => {
    const next = { ...queuedRef.current, [sessionId]: fn(queuedRef.current[sessionId] ?? []) }
    queuedRef.current = next
    setQueued(next)
  }, [])

  // Sohbet değişince bekleyen görseller temizlenir (App de ekleri ve taslağı sıfırlar).
  useEffect(() => setImages([]), [o.activeId])

  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.type === 'g2:queueConsumed') {
          const ids = new Set(event.items.map((i) => i.id))
          updateQueued(event.sessionId, (list) => list.filter((q) => !ids.has(q.id)))
        } else if ((event.type === 'done' || event.type === 'error') && (queuedRef.current[event.sessionId]?.length ?? 0) > 0) {
          // Tur kuyruktaki mesajı tüketmeden bittiyse kalan mesajlarla yeni tur başlar.
          api.g2QueueFlush(event.sessionId).catch((err: unknown) => {
            latest.current.dispatchStore({ sessionId: event.sessionId, action: { type: 'failed', message: errorMessage(err) } })
          })
        }
      }),
    [updateQueued],
  )

  /** Geri alınan veya gönderilemeyen mesajları taslağa, eklere ve görsellere geri koyar. */
  const restore = (items: readonly QueuedView[]) => {
    if (items.length === 0) return
    const { draft, setDraft, setAttachments } = latest.current
    setDraft([...items.map((q) => q.payload.text), draft].filter((t) => t.trim()).join('\n\n'))
    setAttachments((prev) => [...new Set([...prev, ...items.flatMap((q) => q.payload.attachments)])])
    setImages((prev) => [...prev, ...items.flatMap((q) => q.payload.images)])
  }

  const deliver = async (sessionId: string, view: Omit<QueuedView, 'id'>, interrupt: boolean) => {
    const payload: G2SendPayload = {
      text: view.payload.text,
      attachments: view.payload.attachments,
      images: toPayloadImages(view.payload.images),
      interrupt,
      reasoningEffort: getG2Prefs().effort,
    }
    try {
      await api.setActiveSession(sessionId)
      const result = await api.g2Send(sessionId, payload)
      if (result.status === 'queued') updateQueued(sessionId, (list) => [...list, { ...view, id: result.queueId }])
      else latest.current.dispatchStore({ sessionId, action: { type: 'userSent', text: view.text, attachments: view.attachments, at: Date.now() } })
    } catch (err) {
      restore([{ ...view, id: '' }])
      throw err
    }
  }

  const userItems = (items: readonly ChatItem[]) => items.filter(isHumanItem)

  g2Bridge.current = {
    activeId: o.activeId,
    queued: queued[o.activeId] ?? [],
    images,
    setImages: (update) => setImages((prev) => update(prev)),
    setAttachments: (update) => o.setAttachments((prev) => update(prev)),
    send: async ({ text, interrupt }) => {
      const { activeId, attachments, setDraft, setAttachments } = latest.current
      const imgs = imagesRef.current
      setDraft('')
      setAttachments([])
      setImages([])
      await deliver(activeId, { text, attachments: [...attachments, ...imageNames(imgs)], payload: { text, attachments, images: [...imgs] } }, interrupt)
    },
    sendQueuedNow: async (id) => {
      const sessionId = latest.current.activeId
      const item = queuedRef.current[sessionId]?.find((q) => q.id === id)
      // false: mesaj bu arada konuşmaya eklendi; arayüz olayla güncellenir.
      if (!item || !(await api.g2QueueRemove(sessionId, id))) return
      updateQueued(sessionId, (list) => list.filter((q) => q.id !== id))
      await deliver(sessionId, item, true)
    },
    unqueue: async (id) => {
      const sessionId = latest.current.activeId
      const item = queuedRef.current[sessionId]?.find((q) => q.id === id)
      if (!item || !(await api.g2QueueRemove(sessionId, id))) return
      updateQueued(sessionId, (list) => list.filter((q) => q.id !== id))
      restore([item])
    },
    abort: async () => {
      const sessionId = latest.current.activeId
      const items = queuedRef.current[sessionId] ?? []
      if (items.length > 0) {
        const removed = new Set(await api.g2QueueClear(sessionId))
        updateQueued(sessionId, () => [])
        restore(items.filter((q) => removed.has(q.id)))
      }
      await api.abort(sessionId)
    },
    editMessage: async (itemId, items, newText) => {
      const sessionId = latest.current.activeId
      const humans = userItems(items)
      const ordinal = humans.findIndex((i) => i.id === itemId)
      const item = humans[ordinal]
      if (!item) throw new Error('Mesaj bulunamadı')
      const text = newText.trim()
      await api.g2EditMessage(sessionId, { ordinal, originalText: item.text, newText: text, reasoningEffort: getG2Prefs().effort })
      latest.current.dispatchStore({ sessionId, action: { type: 'g2:rewind', itemId, text, at: Date.now() } })
    },
    regenerate: async (items) => {
      const sessionId = latest.current.activeId
      const last = userItems(items).at(-1)
      if (!last) return
      try {
        await api.g2Regenerate(sessionId, { text: last.text, reasoningEffort: getG2Prefs().effort })
        latest.current.dispatchStore({ sessionId, action: { type: 'g2:rewind', itemId: last.id, at: Date.now() } })
      } catch (err) {
        latest.current.dispatchStore({ sessionId, action: { type: 'failed', message: errorMessage(err) } })
      }
    },
    newChat: () => actionsRef.current?.newChat(),
    openMemory: () => actionsRef.current?.openMemory(),
    openSystemPrompt: () => actionsRef.current?.openSystemPrompt(),
    pickFiles: () => actionsRef.current?.pickFiles(),
  }

  return {
    // App, erken dönüşlerden sonra tanımlanan işleyicilerini buradan bağlar.
    bind: (actions) => {
      actionsRef.current = actions
    },
  }
}
