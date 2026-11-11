// [g16] Bellek baskısı yönetimi (243): arka plandaki ağır sohbet durumlarından hangilerinin boşaltılacağını seçer (saf; test edilebilir).
import type { ChatState, ChatStore } from './chat-state.ts'

/** Yaklaşık ağırlık: öğelerdeki metinlerin UTF-16 karakter sayısı (iki düzey derinliğe kadar; tam ölçüm gerekmez). */
export function chatWeight(chat: ChatState): number {
  let total = 0
  const add = (value: unknown, depth: number): void => {
    if (typeof value === 'string') total += value.length
    else if (depth > 0 && typeof value === 'object' && value !== null) for (const v of Object.values(value)) add(v, depth - 1)
  }
  for (const item of chat.items) add(item, 2)
  return total
}

/**
 * Boşaltılabilir: tur çalışmıyor, onay beklemiyor, dosya değişikliği listesi boş (kayıttan geri yüklenemez; boşaltılırsa
 * değişiklik paneli ve geri alma kaybolurdu) ve boşaltılacak bir şey var.
 */
export function isEvictable(chat: ChatState): boolean {
  return !chat.running && chat.changes.length === 0 && chat.items.length > 0 && !chat.items.some((i) => i.kind === 'tool' && i.awaitingApproval)
}

export interface EvictionOptions {
  /** Görünen (etkin, yan bölme) ve ana süreçte çalışan sohbetler: asla boşaltılmaz */
  protect: ReadonlySet<string>
  /** Son görüntülenme zamanı; en eskisi önce boşaltılır */
  lastUsed: ReadonlyMap<string, number>
  /** Arka planda tutulabilecek toplam ağırlık (karakter); 0 = boşaltılabilen her şey */
  budget: number
}

export function pickEvictions(store: ChatStore, { protect, lastUsed, budget }: EvictionOptions): string[] {
  const candidates = Object.entries(store)
    .filter(([id, chat]) => !protect.has(id) && isEvictable(chat))
    .map(([id, chat]) => ({ id, weight: chatWeight(chat), used: lastUsed.get(id) ?? 0 }))
    .sort((a, b) => a.used - b.used)
  let total = candidates.reduce((sum, c) => sum + c.weight, 0)
  const evict: string[] = []
  for (const c of candidates) {
    if (total <= budget) break
    evict.push(c.id)
    total -= c.weight
  }
  return evict
}
