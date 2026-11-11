// §32.3 (ADR 0062): IPC gecikmesi ve olay hızı sayaçları. kor-bridge.ts her çağrıyı ve olayı buraya
// bildirir; bu modül köprüyü içe aktarmaz (döngü yok). Yavaş çağrılar `setSlowSink` ile verilen
// alıcıya gider (perf.log açıksa boot.ts bağlar). Hiçbir zamanlayıcı yok: sayılar yalnızca okunduğunda hesaplanır.

/** Bu süreyi aşan IPC çağrısı ya da kare perf.log'a yazılır (§32.3). */
export const SLOW_MS = 50

const RING = 256
const ipc = new Float64Array(RING)
let ipcCount = 0
let lastIpc: { method: string; ms: number } | null = null
let events = 0
let eventsAt = 0
let eventsSeen = 0

export interface SlowEntry {
  kind: 'ipc' | 'frame'
  what: string
  ms: number
  detail?: string
}

let sink: ((e: SlowEntry) => void) | null = null

export function setSlowSink(fn: ((e: SlowEntry) => void) | null): void {
  sink = fn
}

export function reportSlow(e: SlowEntry): void {
  sink?.(e)
}

export function recordIpc(method: string, ms: number): void {
  ipc[ipcCount % RING] = ms
  ipcCount++
  lastIpc = { method, ms }
  // perfmon.log kendi gecikmesini tekrar bildirmesin.
  if (ms > SLOW_MS && !method.startsWith('perfmon.')) reportSlow({ kind: 'ipc', what: method, ms })
}

export function recordEvent(): void {
  events++
}

/** Yakın-sıra yüzdelik (kor_runtime::budgets::Stat ile aynı tanım). */
export function percentile(values: readonly number[], q: number): number {
  if (values.length === 0) return 0
  const v = [...values].sort((a, b) => a - b)
  const i = Math.min(v.length - 1, Math.max(0, Math.ceil(q * v.length) - 1))
  return v[i] ?? 0
}

export interface IpcSnapshot {
  calls: number
  p50: number
  p95: number
  last: { method: string; ms: number } | null
  /** Son okumadan beri saniyedeki olay sayısı. */
  eventsPerSec: number
}

export function snapshot(now: number = performance.now()): IpcSnapshot {
  const n = Math.min(ipcCount, RING)
  const values = Array.from(ipc.subarray(0, n))
  const dt = eventsAt === 0 ? 0 : (now - eventsAt) / 1000
  const rate = dt > 0 ? (events - eventsSeen) / dt : 0
  eventsAt = now
  eventsSeen = events
  return { calls: ipcCount, p50: percentile(values, 0.5), p95: percentile(values, 0.95), last: lastIpc, eventsPerSec: rate }
}

/** Testler için sıfırla. */
export function resetStats(): void {
  ipc.fill(0)
  ipcCount = 0
  lastIpc = null
  events = 0
  eventsAt = 0
  eventsSeen = 0
}
