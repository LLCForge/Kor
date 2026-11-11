// Saf bulanık eşleştirme: dosya araması (ana süreç) ve slash komutları (arayüz) ortak kullanır.

const BOUNDARY = '/\\._- '

/** Sorgu hedefin alt dizisi değilse null; büyük puan daha iyi eşleşme. */
export function fuzzyScore(query: string, target: string): number | null {
  const q = query.toLowerCase()
  const t = target.toLowerCase()
  if (q.length === 0) return -t.length
  let score = 0
  let from = 0
  let prev = -2
  for (const ch of q) {
    const at = t.indexOf(ch, from)
    if (at === -1) return null
    score += 1
    if (at === prev + 1) score += 5
    if (at === 0 || BOUNDARY.includes(t[at - 1] ?? '')) score += 8
    prev = at
    from = at + 1
  }
  const base = t.slice(t.lastIndexOf('/') + 1)
  if (t.includes(q)) score += 10
  if (base.includes(q)) score += 20
  if (base.startsWith(q)) score += 10
  // Kısa yollar ve adlar öne çıksın.
  return score - t.length * 0.05
}

/** En iyi eşleşenler önce; eşit puanda özgün sıra korunur. */
export function fuzzyFilter<T>(items: readonly T[], query: string, keys: (item: T) => readonly string[], limit = Infinity): T[] {
  const scored: { item: T; score: number; index: number }[] = []
  items.forEach((item, index) => {
    let best: number | null = null
    for (const key of keys(item)) {
      const s = fuzzyScore(query, key)
      if (s !== null && (best === null || s > best)) best = s
    }
    if (best !== null) scored.push({ item, score: best, index })
  })
  scored.sort((a, b) => b.score - a.score || a.index - b.index)
  return scored.slice(0, limit).map((s) => s.item)
}
