// `agent.status {verb, elapsed, tokens}` (spec §16.2 kural 2, §16.3, ADR 0053): fiili çekirdek seçer, süre
// çekirdekteki Instant'tan, token sağlayıcı yanıtından gelir. Masaüstü yalnızca çizer; arayüzde fiil döndüren
// zamanlayıcı yoktur. Rust `kor:agent/status` olayını (chat.rs) oturum kimliğiyle yayar.
import { useCallback, useSyncExternalStore } from 'react'
import { api } from './api.ts'

export interface AgentStatus {
  sessionId: string
  verb: string
  verbIndex: number
  elapsedMs: number
  tokens: number
}

/** Rust'tan gelen yükü doğrular (güven sınırı: yanlış biçimli olay çizilmez). */
export function parseStatus(payload: unknown): AgentStatus | null {
  if (typeof payload !== 'object' || payload === null) return null
  const p = payload as Record<string, unknown>
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : null)
  const elapsedMs = num(p.elapsedMs)
  const tokens = num(p.tokens)
  const verbIndex = num(p.verbIndex)
  if (typeof p.sessionId !== 'string' || typeof p.verb !== 'string' || p.verb.length === 0 || p.verb.length > 64) return null
  if (elapsedMs === null || tokens === null || verbIndex === null) return null
  return { sessionId: p.sessionId, verb: p.verb, verbIndex, elapsedMs, tokens }
}

/** "1,2k token" — ondalık ayırıcı Türkçe. */
export function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1).replace('.', ',')}k token` : `${n} token`
}

type StatusApi = { agent: { onStatus: (cb: (payload: unknown) => void) => () => void } }

const latest = new Map<string, AgentStatus>()
const listeners = new Set<() => void>()
let unlisten: (() => void) | null = null

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  if (!unlisten) {
    unlisten = (api as unknown as StatusApi).agent.onStatus((payload) => {
      const s = parseStatus(payload)
      if (!s) return
      latest.set(s.sessionId, s)
      for (const l of listeners) l()
    })
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && unlisten) {
      unlisten()
      unlisten = null
      latest.clear()
    }
  }
}

/** Oturumun son `agent.status` olayı; tur başlamadan önceki (bayat) durum `since` ile elenir. */
export function useAgentStatus(sessionId: string | undefined, since: number | null): AgentStatus | null {
  const get = useCallback(() => (sessionId ? (latest.get(sessionId) ?? null) : null), [sessionId])
  const s = useSyncExternalStore(subscribe, get)
  if (!s || since === null) return s
  // Önceki turun son durumu: çekirdek yeni turun ilk durumunu hemen gönderir, o gelene kadar çizilmez.
  return Date.now() - since + 1500 < s.elapsedMs ? null : s
}
