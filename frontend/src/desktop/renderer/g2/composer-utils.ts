import { uiLocale } from '../ui-locale.ts'
// Mesaj kutusunun saf yardımcıları (test edilebilir; React/DOM yok).

export type Trigger =
  /** Mesaj başında "/kom" */
  | { kind: 'command'; query: string; start: number; end: number }
  /** "/model ara" — komut argümanı */
  | { kind: 'commandArg'; command: string; query: string; start: number; end: number }
  /** "@dosya" */
  | { kind: 'mention'; query: string; start: number; end: number }

/** İmlecin önündeki metne göre açılır menü tetikleyicisi. */
export function detectTrigger(text: string, caret: number): Trigger | null {
  const before = text.slice(0, caret)
  const command = /^\/([^\s/]*)$/.exec(before)
  if (command) return { kind: 'command', query: command[1] ?? '', start: 0, end: caret }
  const arg = /^\/([^\s/]+) ([^\n]*)$/.exec(before)
  if (arg?.[1] !== undefined) {
    const query = arg[2] ?? ''
    return { kind: 'commandArg', command: arg[1], query, start: caret - query.length, end: caret }
  }
  const mention = /(?:^|\s)@([^\s@]*)$/.exec(before)
  if (mention) {
    const query = mention[1] ?? ''
    return { kind: 'mention', query, start: caret - query.length - 1, end: caret }
  }
  return null
}

/** [start, end) aralığını değiştirir; yeni imleç konumunu da döner. */
export function replaceRange(text: string, start: number, end: number, insert: string): { text: string; cursor: number } {
  return { text: text.slice(0, start) + insert + text.slice(end), cursor: start + insert.length }
}

/** Yaklaşık token: ~4 karakter = 1 token. */
export const estimateTokens = (chars: number) => Math.ceil(chars / 4)

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024).toLocaleString(uiLocale())} KB`
  return `${(bytes / (1024 * 1024)).toLocaleString(uiLocale(), { maximumFractionDigits: 1 })} MB`
}

/** Seçili modelin "-fast" karşılığı: açıkken temel modele, kapalıyken hızlı varyanta geçilir. target null: geçilemez. */
export function fastVariant(modelId: string, ids: readonly string[]): { on: boolean; target: string | null } {
  if (modelId.endsWith('-fast')) {
    const base = modelId.slice(0, -'-fast'.length)
    return { on: true, target: ids.includes(base) ? base : null }
  }
  const fast = `${modelId}-fast`
  return { on: false, target: ids.includes(fast) ? fast : null }
}

/** Diziyi değiştirmeden bir elemanı başka konuma taşır (ek sıralama). */
export function moveItem<T>(items: readonly T[], from: number, to: number): T[] {
  if (from === to || from < 0 || from >= items.length || to < 0 || to >= items.length) return [...items]
  const next = [...items]
  const [moved] = next.splice(from, 1)
  if (moved !== undefined) next.splice(to, 0, moved)
  return next
}
