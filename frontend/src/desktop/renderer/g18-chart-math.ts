import { uiLocale } from './ui-locale.ts'
// [g18] Grafik hesapları (saf, DOM'suz): istatistik özeti, kutu grafiği çeyrekleri, histogram kovaları,
//       "güzel" eksen tıkları, doğrusal ölçek, yığma, şelale adımları, yakınlaştırma penceresi ve sayı biçimi.

export interface Stats {
  count: number
  min: number
  max: number
  mean: number
  median: number
  sum: number
}

/** null ve sonlu olmayan değerleri atar. */
export function finite(values: readonly (number | null)[]): number[] {
  return values.filter((v): v is number => v !== null && Number.isFinite(v))
}

/** Sıralı dizide doğrusal enterpolasyonlu yüzdelik (R tip 7; Excel PERCENTILE.INC ile aynı). */
export function quantile(sorted: readonly number[], q: number): number {
  if (sorted.length === 0) return Number.NaN
  const pos = (sorted.length - 1) * q
  const lo = Math.floor(pos)
  const hi = Math.ceil(pos)
  const a = sorted[lo]!
  return a + (sorted[hi]! - a) * (pos - lo)
}

/** min/maks/ortalama/medyan/toplam; değer yoksa null. */
export function summarize(values: readonly (number | null)[]): Stats | null {
  const xs = finite(values)
  if (xs.length === 0) return null
  const sorted = [...xs].sort((a, b) => a - b)
  const sum = xs.reduce((s, v) => s + v, 0)
  return { count: xs.length, min: sorted[0]!, max: sorted.at(-1)!, mean: sum / xs.length, median: quantile(sorted, 0.5), sum }
}

export interface BoxStats {
  min: number
  q1: number
  median: number
  q3: number
  max: number
  /** Tukey bıyıkları: [q1 - 1.5·IQR, q3 + 1.5·IQR] içindeki en uç veri noktaları */
  lo: number
  hi: number
  outliers: number[]
}

export function boxStats(values: readonly (number | null)[]): BoxStats | null {
  const sorted = finite(values).sort((a, b) => a - b)
  if (sorted.length === 0) return null
  const q1 = quantile(sorted, 0.25)
  const q3 = quantile(sorted, 0.75)
  const fence = 1.5 * (q3 - q1)
  const inside = sorted.filter((v) => v >= q1 - fence && v <= q3 + fence)
  return {
    min: sorted[0]!,
    q1,
    median: quantile(sorted, 0.5),
    q3,
    max: sorted.at(-1)!,
    lo: inside[0] ?? q1,
    hi: inside.at(-1) ?? q3,
    outliers: sorted.filter((v) => v < q1 - fence || v > q3 + fence),
  }
}

export interface Bin {
  x0: number
  x1: number
  count: number
}

/** Eşit genişlikli kovalar; kova sayısı verilmezse Sturges kuralı (1..50 arası). */
export function histogram(values: readonly (number | null)[], bins?: number): Bin[] {
  const xs = finite(values)
  if (xs.length === 0) return []
  const min = Math.min(...xs)
  const max = Math.max(...xs)
  const k = Math.max(1, Math.min(50, Math.round(bins ?? Math.ceil(Math.log2(xs.length)) + 1)))
  const width = max === min ? 1 : (max - min) / k
  const out: Bin[] = Array.from({ length: k }, (_, i) => ({ x0: min + i * width, x1: min + (i + 1) * width, count: 0 }))
  // Son kova kapalı aralıktır: maks değer son kovaya düşer.
  for (const v of xs) out[Math.min(k - 1, Math.floor((v - min) / width))]!.count++
  return out
}

export interface Ticks {
  min: number
  max: number
  step: number
  ticks: number[]
}

const tidy = (v: number) => Number(v.toPrecision(12))

/** [min, max] aralığını 1-2-2.5-5 adımlı "güzel" tıklara yuvarlar. */
export function niceTicks(min: number, max: number, count = 5): Ticks {
  let lo = Number.isFinite(min) ? min : 0
  let hi = Number.isFinite(max) ? max : 1
  if (lo > hi) [lo, hi] = [hi, lo]
  if (lo === hi) {
    const pad = Math.abs(lo) / 2 || 1
    lo -= pad
    hi += pad
  }
  const raw = (hi - lo) / Math.max(1, count)
  const mag = 10 ** Math.floor(Math.log10(raw))
  const norm = raw / mag
  const step = (norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 2.5 ? 2.5 : norm <= 5 ? 5 : 10) * mag
  const start = tidy(Math.floor(lo / step + 1e-9) * step)
  const end = tidy(Math.ceil(hi / step - 1e-9) * step)
  const ticks: number[] = []
  for (let v = start; v <= end + step / 2; v += step) ticks.push(tidy(v))
  return { min: start, max: end, step, ticks }
}

/** [d0, d1] alanını [r0, r1] aralığına eşler (sıfır genişlikli alanda orta nokta). */
export function scaleLinear(d0: number, d1: number, r0: number, r1: number): (v: number) => number {
  const span = d1 - d0
  return span === 0 ? () => (r0 + r1) / 2 : (v) => r0 + ((v - d0) / span) * (r1 - r0)
}

export interface Segment {
  y0: number
  y1: number
}

/** Seri değerlerini dizin bazında yığar; pozitifler yukarı, negatifler aşağı ayrı birikir (null = 0). */
export function stackValues(series: readonly (readonly (number | null)[])[]): Segment[][] {
  const n = Math.max(0, ...series.map((s) => s.length))
  const pos = new Array<number>(n).fill(0)
  const neg = new Array<number>(n).fill(0)
  return series.map((s) =>
    Array.from({ length: n }, (_, i) => {
      const v = s[i] ?? 0
      const base = v >= 0 ? pos : neg
      const y0 = base[i]!
      base[i] = y0 + v
      return { y0, y1: y0 + v }
    }),
  )
}

export interface WaterfallStep {
  label: string
  start: number
  end: number
  delta: number
  total: boolean
}

const TOTAL_LABEL = /^(toplam|total|net|sonuç|bakiye|kapanış)$/i

/** Artış/azalış adımları; "Toplam" etiketli giriş ara toplam olur, hiç yoksa sona bir toplam eklenir. */
export function waterfall(labels: readonly string[], values: readonly (number | null)[]): WaterfallStep[] {
  const steps: WaterfallStep[] = []
  let run = 0
  values.forEach((v, i) => {
    const label = labels[i] ?? String(i + 1)
    if (TOTAL_LABEL.test(label.trim())) {
      steps.push({ label, start: 0, end: run, delta: run, total: true })
      return
    }
    const delta = v ?? 0
    steps.push({ label, start: run, end: run + delta, delta, total: false })
    run += delta
  })
  if (!steps.some((s) => s.total)) steps.push({ label: 'Toplam', start: 0, end: run, delta: run, total: true })
  return steps
}

/** Görünür x aralığı, toplam alanın kesri olarak [0, 1]. */
export interface ZoomWindow {
  start: number
  end: number
}

export const FULL_WINDOW: ZoomWindow = { start: 0, end: 1 }
const MIN_SPAN = 0.02

const clampWindow = (start: number, span: number): ZoomWindow => {
  const s = Math.min(Math.max(0, start), 1 - span)
  return { start: s, end: s + span }
}

/** factor < 1 yakınlaştırır; anchor (0..1) pencerede sabit kalan nokta. */
export function zoomWindow(w: ZoomWindow, factor: number, anchor = 0.5): ZoomWindow {
  const span = Math.min(1, Math.max(MIN_SPAN, (w.end - w.start) * factor))
  if (span >= 1) return FULL_WINDOW
  const pivot = w.start + anchor * (w.end - w.start)
  return clampWindow(pivot - anchor * span, span)
}

/** Pencereyi toplam alanın kesri kadar kaydırır (kenarlarda durur). */
export function panWindow(w: ZoomWindow, delta: number): ZoomWindow {
  return clampWindow(w.start + delta, w.end - w.start)
}

/** n kategorinin pencerede görünen [başlangıç, bitiş) dizinleri; en az 2 (ya da n) kategori. */
export function windowIndices(n: number, w: ZoomWindow): [number, number] {
  if (n <= 2) return [0, n]
  let i0 = Math.floor(w.start * n + 1e-9)
  let i1 = Math.ceil(w.end * n - 1e-9)
  if (i1 - i0 < 2) i1 = i0 + 2
  if (i1 > n) [i0, i1] = [n - 2, n]
  return [Math.max(0, i0), i1]
}

const plainFmt = new Intl.NumberFormat(uiLocale(), { maximumFractionDigits: 2 })
const compactFmt = new Intl.NumberFormat(uiLocale(), { notation: 'compact', maximumFractionDigits: 1 })

/** Türkçe sayı biçimi; compact=true eksenlerde 12,5 B / 3,4 Mn gibi kısaltır. */
export function formatNumber(v: number | null | undefined, compact = false): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '–'
  return compact && Math.abs(v) >= 10_000 ? compactFmt.format(v) : plainFmt.format(v)
}

// Sıralı (büyüklük) rampası: tek ton mavi, açık temada açıktan koyuya; koyu temada yüzeye yakın uç koyu.
const SEQ_RAMP = ['#cde2fb', '#9ec5f4', '#6da7ec', '#3987e5', '#256abf', '#184f95', '#0d366b']

const hex = (c: string) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16)) as [number, number, number]

/** t ∈ [0, 1] için ısı haritası rengi (doğrusal RGB enterpolasyonu). */
export function seqColor(t: number, dark: boolean): string {
  const ramp = dark ? [...SEQ_RAMP].reverse() : SEQ_RAMP
  const x = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 0)) * (ramp.length - 1)
  const i = Math.min(ramp.length - 2, Math.floor(x))
  const a = hex(ramp[i]!)
  const b = hex(ramp[i + 1]!)
  const f = x - i
  return `#${a.map((v, k) => Math.round(v + (b[k]! - v) * f).toString(16).padStart(2, '0')).join('')}`
}

/** Hücre üstündeki metin için açık/koyu mürekkep seçimi (WCAG göreli parlaklık). */
export function inkOn(color: string): 'dark' | 'light' {
  const [r, g, b] = hex(color).map((v) => {
    const c = v / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  }) as [number, number, number]
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.4 ? 'dark' : 'light'
}
