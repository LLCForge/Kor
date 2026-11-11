// [g21] Kelime bazlı zengin metin farkı (#179). `diff` paketi zaten bağımlılık; uzun/çok farklı metinde
//      O(ND) maliyeti sınırlamak için girdi boyutu, düzenleme uzunluğu ve süre üst sınırlıdır (DoS).
import { diffWordsWithSpace } from 'diff'

export const MAX_DIFF_CHARS = 300_000
export const MAX_EDIT_LENGTH = 5_000
/** Hesap ana iş parçacığında yapılır; arayüz en fazla bu kadar bekler. */
export const DIFF_TIMEOUT_MS = 1_500

export interface WordPart {
  value: string
  kind: 'same' | 'added' | 'removed'
}

export interface WordDiffResult {
  parts: WordPart[]
  addedWords: number
  removedWords: number
  truncated: boolean
}

const wordCount = (s: string) => s.match(/\S+/g)?.length ?? 0

/** İki metnin kelime farkı; fark aşırı büyükse null (kullanıcıya "çok farklı" denir). */
export function wordDiff(before: string, after: string): WordDiffResult | null {
  const truncated = before.length > MAX_DIFF_CHARS || after.length > MAX_DIFF_CHARS
  const changes = diffWordsWithSpace(before.slice(0, MAX_DIFF_CHARS), after.slice(0, MAX_DIFF_CHARS), { maxEditLength: MAX_EDIT_LENGTH, timeout: DIFF_TIMEOUT_MS })
  if (!changes) return null
  let addedWords = 0
  let removedWords = 0
  const parts = changes.map((c): WordPart => {
    if (c.added) addedWords += wordCount(c.value)
    if (c.removed) removedWords += wordCount(c.value)
    return { value: c.value, kind: c.added ? 'added' : c.removed ? 'removed' : 'same' }
  })
  return { parts, addedWords, removedWords, truncated }
}
