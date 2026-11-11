// Yan panel ↔ arka plan bağlantısı: istek/yanıt (rid) ve itilen olaylar (status, event, tabChanged, pending, step).
import { ext } from '../ext.ts'
import type { Status } from '../transport.ts'

type Listener = (m: Record<string, unknown>) => void

const port = ext().runtime.connect({ name: 'kor-panel' })
const waiting = new Map<number, { ok: (v: unknown) => void; fail: (e: Error) => void }>()
const listeners = new Set<Listener>()
let next = 0

port.onMessage.addListener((raw) => {
  const m = (raw ?? {}) as Record<string, unknown>
  if (m.kind === 'res') {
    const w = waiting.get(Number(m.rid))
    if (!w) return
    waiting.delete(Number(m.rid))
    if (m.ok === true) w.ok(m.result)
    else w.fail(new Error(typeof m.error === 'string' ? m.error : 'hata'))
    return
  }
  for (const l of listeners) l(m)
})

/** Arka plana bir istek (`op`) gönderir. */
export function call<T = unknown>(op: string, args: Record<string, unknown> = {}): Promise<T> {
  const rid = ++next
  return new Promise<T>((ok, fail) => {
    waiting.set(rid, { ok: (v) => ok(v as T), fail })
    port.postMessage({ ...args, op, rid })
  })
}

/** Kor çekirdeğine yan panel isteği (`{ type: "client", op }`); yanıtta `error` varsa hata. */
export async function client<T = unknown>(op: string, body: Record<string, unknown> = {}, timeoutMs?: number): Promise<T> {
  const r = await call<Record<string, unknown> | null>('client', { body: { ...body, op }, timeoutMs })
  if (r && typeof r === 'object' && typeof r.error === 'string') throw new Error(r.error)
  return r as T
}

export function on(l: Listener): () => void {
  listeners.add(l)
  return () => listeners.delete(l)
}

export type { Status }
