// §27.16 tahmin yardımcıları (ADR 0063): composer taslağından tahmin isteğine, çalışan görevin
// ilerleme satırına. Rakamlar Rust'tan gelir; burada yalnızca biçim ve sapma.

/** Tahmin istenecek kadar anlamlı taslak mı (kısa selamlaşma ve slash komutları hariç). */
export function estimable(draft: string): boolean {
  const t = draft.trim()
  return t.length >= 12 && !t.startsWith('/')
}

/** `45 sn`, `4 dk`, `1 dk 12 sn`, `1 sa 5 dk`. */
export function formatDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs))
  if (s < 60) return `${s} sn`
  if (s < 3600) return s % 60 === 0 ? `${s / 60} dk` : `${Math.floor(s / 60)} dk ${s % 60} sn`
  return `${Math.floor(s / 3600)} sa ${Math.floor((s % 3600) / 60)} dk`
}

/** Çalışan görev: `1 dk 12 sn / ≈4 dk`; tahminin iki katı aşıldıysa `drift`. */
export function progressLine(elapsedSecs: number, estimateSecs: number): { text: string; drift: boolean } {
  return {
    text: `${formatDuration(elapsedSecs)} / ≈${formatDuration(estimateSecs)}`,
    drift: elapsedSecs > estimateSecs * 2,
  }
}
