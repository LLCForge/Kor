// cdesktop satır içi inceleme yorumları: diff'te bir satıra yorum eklenir, mesaj kutusunda "N inceleme yorumu
// eklenecek" görünür ve bir sonraki mesajın başına biçimli olarak eklenip ajana gönderilir. Modül düzeyi depo
// (useSyncExternalStore): diff, panel ve mesaj kutusu aynı listeyi görür. Saf kısımlar test edilebilir.
import { useSyncExternalStore } from 'react'

export interface ReviewComment {
  id: string
  path: string
  /** Yorumun bağlandığı satır (yeni ya da eski numara) */
  line: number
  side: 'old' | 'new'
  /** Satırın kod metni (bağlam) */
  code: string
  text: string
}

let comments: readonly ReviewComment[] = []
const listeners = new Set<() => void>()
let seq = 0
const emit = () => { for (const l of listeners) l() }

export function addReviewComment(input: Omit<ReviewComment, 'id'>): ReviewComment {
  const c: ReviewComment = { ...input, id: `rc-${++seq}`, text: input.text.trim() }
  comments = [...comments, c]
  emit()
  return c
}
export function removeReviewComment(id: string): void {
  comments = comments.filter((c) => c.id !== id)
  emit()
}
export function clearReviewComments(): void {
  if (comments.length === 0) return
  comments = []
  emit()
}
export function getReviewComments(): readonly ReviewComment[] {
  return comments
}
export function useReviewComments(): readonly ReviewComment[] {
  return useSyncExternalStore(
    (l) => { listeners.add(l); return () => listeners.delete(l) },
    () => comments,
  )
}

/** Mesajın başına eklenecek biçim (cdesktop "Review Comments (N)" kartıyla aynı bilgi). */
export function formatReviewComments(list: readonly ReviewComment[]): string {
  if (list.length === 0) return ''
  const lines = list.map((c) => `- ${c.path}:${c.line}${c.side === 'old' ? ' (eski)' : ''}\n  \`${c.code.trim().slice(0, 160)}\`\n  ${c.text}`)
  return `İnceleme yorumları (${list.length}):\n${lines.join('\n')}`
}

/** Mesajla yorumları birleştirir; metin boşsa yalnız yorumlar, yorum yoksa yalnız metin. */
export function mergeReviewIntoMessage(text: string, list: readonly ReviewComment[]): string {
  const block = formatReviewComments(list)
  if (!block) return text
  return text.trim() ? `${block}\n\n${text}` : block
}

/** Test/geliştirme için: depoyu sıfırla. */
export function _resetReviewComments(): void {
  comments = []
  seq = 0
  emit()
}
