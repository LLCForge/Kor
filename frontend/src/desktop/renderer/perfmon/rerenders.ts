// §32.2 (ADR 0062): gereksiz yeniden çizim sayacı, why-did-you-render'ın bağımlılıksız karşılığı. Yalnız
// geliştirici build'inde (NODE_ENV !== 'production') sayar; release'de `tracked.tsx` düz `memo` olur.
// Gereksiz çizim: bileşen, öncekiyle DEĞERCE aynı ama BAŞKA bir props nesnesiyle yeniden çizildi (üst bileşen
// her seferinde yeni dizi/nesne verdi ya da memo'suz bileşene aynı props'u yeniden geçti). Bileşenin kendi
// durumu değiştiğinde React aynı props nesnesini verir: o çizim sayılmaz. Fonksiyonlar kimlikle karşılaştırılır
// (yeni kapanış = farklı), böylece yanlış alarm yerine kaçırma tarafında kalınır.

export const RERENDER_TRACKING = process.env.NODE_ENV !== 'production'

const counts = new Map<string, number>()

/** İç içe değer eşitliği; derinlik ve öğe sınırıyla (büyük listelerde çizim yolunu yavaşlatmaz). */
export function sameValue(a: unknown, b: unknown, depth = 4, budget = { n: 2000 }): boolean {
  if (Object.is(a, b)) return true
  if (depth === 0 || --budget.n < 0) return false
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  if (Array.isArray(a) !== Array.isArray(b)) return false
  if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false
  const proto = Object.getPrototypeOf(a)
  // Yalnız düz nesne ve dizi: Date, Map, DOM düğümü… kimlikle.
  if (proto !== Object.prototype && proto !== Array.prototype && proto !== null) return false
  const ka = Object.keys(a)
  const kb = Object.keys(b)
  if (ka.length !== kb.length) return false
  const rb = b as Record<string, unknown>
  const ra = a as Record<string, unknown>
  return ka.every((k) => Object.hasOwn(rb, k) && sameValue(ra[k], rb[k], depth - 1, budget))
}

/** Bir çizimi kaydeder; gereksizse sayacı artırıp `true` döner. */
export function noteRender(name: string, prev: object | null, next: object): boolean {
  if (prev === null || prev === next || !sameValue(prev, next)) return false
  counts.set(name, (counts.get(name) ?? 0) + 1)
  return true
}

export function rerenderCounts(): Record<string, number> {
  return Object.fromEntries(counts)
}

export function rerenderTotal(): number {
  let n = 0
  for (const v of counts.values()) n += v
  return n
}

export function resetRerenders(): void {
  counts.clear()
}

// E2E ve bindirme için: `window.__korRerenders()` → { total, byComponent }.
if (RERENDER_TRACKING && typeof window !== 'undefined') {
  ;(window as unknown as { __korRerenders: () => { total: number; byComponent: Record<string, number> } }).__korRerenders = () => ({
    total: rerenderTotal(),
    byComponent: rerenderCounts(),
  })
}
