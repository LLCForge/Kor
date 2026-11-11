// [g26] BM25 sıralama. tool_search anahtar kelime saymaz; seyrek terim daha ağırdır.

const K1 = 1.2
const B = 0.75

export function tokenize(text: string): string[] {
  return text.toLocaleLowerCase('tr').split(/[^\p{L}\p{N}_]+/u).filter((term) => term.length > 1)
}

function covers(queryTerm: string, docTerm: string): boolean {
  if (queryTerm === docTerm) return true
  // Türkçe çekim: "takvim" "takvimde" ile, "etkinlik" "etkinlikleri" ile eşleşir.
  if (queryTerm.length < 4 || docTerm.length < 4) return false
  return docTerm.startsWith(queryTerm) || queryTerm.startsWith(docTerm)
}

/** Sorgu terimleri belgede geçmeyen kayıt elenir. Eşit puanda önceki kayıt kalır. */
export function rankByBm25<T>(items: readonly T[], queryTokens: readonly string[], textOf: (item: T) => string, limit: number): T[] {
  return scoreBm25(items, queryTokens, textOf)
    .slice(0, limit)
    .map((row) => row.item)
}

/** Puanlı sıralama (yüksekten düşüğe, yalnız puanı > 0): ilgililik eşiği uygulamak isteyenler için. */
export function scoreBm25<T>(items: readonly T[], queryTokens: readonly string[], textOf: (item: T) => string): Array<{ item: T; score: number }> {
  const docs = items.map((item) => tokenize(textOf(item)))
  const total = docs.length || 1
  const avg = docs.reduce((sum, doc) => sum + doc.length, 0) / total || 1
  const df = new Map<string, number>()
  for (const term of queryTokens) {
    let docsWith = 0
    for (const doc of docs) if (doc.some((token) => covers(term, token))) docsWith += 1
    df.set(term, docsWith)
  }
  const scored = items.map((item, index) => {
    const doc = docs[index] ?? []
    let score = 0
    for (const term of queryTokens) {
      const freq = doc.filter((token) => covers(term, token)).length
      if (freq === 0) continue
      const docsWith = df.get(term) ?? 0
      const idf = Math.log(1 + (total - docsWith + 0.5) / (docsWith + 0.5))
      const norm = freq + K1 * (1 - B + (B * doc.length) / avg)
      score += (idf * (freq * (K1 + 1))) / norm
    }
    return { item, score, index }
  })
  return scored
    .filter((row) => row.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ item, score }) => ({ item, score }))
}
