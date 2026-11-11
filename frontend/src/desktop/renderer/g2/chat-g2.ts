// chat-state.ts'e bağlanan grup 2 reducer parçaları (saf; test edilebilir).
import type { AgentEventBody } from '../../ipc.ts'
import type { ChatItem, ChatState } from '../chat-state.ts'

/**
 * Düzenle/yeniden üret: verilen kullanıcı mesajından sonrasını kaldırır (text verilirse mesaj metnini değiştirir)
 * ve yeni turu başlamış sayar.
 */
export type G2ChatAction = { type: 'g2:rewind'; itemId: string; text?: string; at: number }

export function g2Rewind(state: ChatState, action: G2ChatAction): ChatState {
  const index = state.items.findIndex((i) => i.id === action.itemId)
  const item = state.items[index]
  if (!item) return state
  const updated: ChatItem = item.kind === 'user' && action.text !== undefined ? { ...item, text: action.text } : item
  return { ...state, items: [...state.items.slice(0, index), updated], running: true, turnStartedAt: action.at }
}

export function g2QueueConsumed(state: ChatState, event: Extract<AgentEventBody, { type: 'g2:queueConsumed' }>): ChatState {
  const added = event.items.map((q): ChatItem => ({ kind: 'user', id: `g2-q-${q.id}`, text: q.text, attachments: q.attachments }))
  return { ...state, items: [...state.items, ...added] }
}
