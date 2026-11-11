// [g3] Sohbet içi arama (Ctrl+F) yardımcıları (saf; test edilebilir).

/**
 * Büyük/küçük harf ve Türkçe I/ı/İ/i farkını yok sayar. Uzunluğu korur: eşleşme konumları
 * özgün metindeki konumlarla aynı kalır (Range oluşturmak için gerekli).
 */
export function foldForSearch(text: string): string {
  let out = ''
  for (const ch of text) {
    const lower = ch.toLocaleLowerCase('tr')
    const folded = lower === 'ı' ? 'i' : lower
    out += folded.length === ch.length ? folded : ch
  }
  return out
}

/** Çakışmayan eşleşmelerin başlangıç konumları; `limit` aşılmaz. */
export function findMatches(text: string, query: string, limit = 1000): number[] {
  const needle = foldForSearch(query)
  if (!needle) return []
  const haystack = foldForSearch(text)
  const found: number[] = []
  for (let i = haystack.indexOf(needle); i !== -1 && found.length < limit; i = haystack.indexOf(needle, i + needle.length)) {
    found.push(i)
  }
  return found
}
