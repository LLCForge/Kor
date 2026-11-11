// Bellek geri çağırma ve bağlam bütçesi: bellek bağlama sığıyorsa tamamı (önbellek dostu, sabit) girer; sığmıyorsa
// her istekte yalnızca o istekle İLGİLİ kayıtlar, ilgililik eşiğini geçenler ve bütçe kadar eklenir. Alakasız
// bellek bağlama sokulmaz: tek ortak sıradan sözcük (ör. "proje") yetmez.
import { scoreBm25, tokenize } from './bm25-g26.ts'

/** memory.md bu boyutu aşarsa sistem istemine tamamı değil, isteğe göre ilgili kısmı girer. */
export const MEMORY_INLINE_CHARS = 6000
/** Mutlak alt eşik: yaygın tek terimli eşleşmeler (düşük idf) elenir. */
const MIN_SCORE = 1
/** Göreli eşik: en iyi eşleşmenin bu oranının altındakiler gürültü sayılır. */
const RELATIVE_SCORE = 0.35

/** Bellek için bağlam payı: pencerenin ~%5'i (≈4 karakter/token), 3k-12k karakter arası (büyük pencerede her çağrının maliyeti şişmesin). */
export function memoryBudgetChars(limitTokens: number | null): number {
  if (!limitTokens) return 8000
  return Math.min(12_000, Math.max(3000, Math.floor(limitTokens * 0.05 * 4)))
}

export function selectRelevant<T>(items: readonly T[], query: string, textOf: (item: T) => string, o: { maxChars: number; limit: number }): T[] {
  const terms = [...new Set(tokenize(query))]
  if (terms.length === 0 || items.length === 0) return []
  const scored = scoreBm25(items, terms, textOf).filter((row) => row.score >= MIN_SCORE)
  const top = scored[0]?.score ?? 0
  const out: T[] = []
  let used = 0
  for (const row of scored) {
    if (row.score < top * RELATIVE_SCORE || out.length >= o.limit) break
    const size = textOf(row.item).length + 4
    if (used + size > o.maxChars) continue
    out.push(row.item)
    used += size
  }
  return out
}

/** Bağlayıcı kayıtlar (kısıt, karar) önce: bütçe dolarsa dışarıda kalan, yeniden üretilebilir notlar olsun. */
const KIND_PRIORITY: Readonly<Record<string, number>> = { constraint: 0, decision: 1, term: 2, note: 3 }

export function prioritizeRecords<T extends { kind?: string | undefined }>(records: readonly T[]): T[] {
  return records
    .map((r, i) => ({ r, i }))
    .sort((a, b) => (KIND_PRIORITY[a.r.kind ?? 'note'] ?? 3) - (KIND_PRIORITY[b.r.kind ?? 'note'] ?? 3) || a.i - b.i)
    .map(({ r }) => r)
}
