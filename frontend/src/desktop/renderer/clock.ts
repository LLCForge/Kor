// [perf] Paylaşımlı saat: süre sayaçları (araç kartı, alt ajan kartı, etkinlik grubu, model testi) kendi
// setInterval'ını kurmak yerine aralık başına tek bir zamanlayıcıya abone olur. 50 kart = 1 zamanlayıcı.
import { useCallback, useSyncExternalStore } from 'react'

interface Ticker { now: number; listeners: Set<() => void>; timer: ReturnType<typeof setInterval> | null }
const tickers = new Map<number, Ticker>()

function tickerFor(ms: number): Ticker {
  let t = tickers.get(ms)
  if (!t) {
    t = { now: Date.now(), listeners: new Set(), timer: null }
    tickers.set(ms, t)
  }
  return t
}

function subscribe(ms: number, listener: () => void): () => void {
  const t = tickerFor(ms)
  t.listeners.add(listener)
  if (!t.timer) {
    t.now = Date.now() // zamanlayıcı dururken değer bayatladı
    t.timer = setInterval(() => {
      t!.now = Date.now()
      for (const l of t!.listeners) l()
    }, ms)
  }
  return () => {
    t.listeners.delete(listener)
    if (t.listeners.size === 0 && t.timer) {
      clearInterval(t.timer)
      t.timer = null
    }
  }
}

const frozen = () => 0

/** active=false iken abone olmaz ve sabit değer döner (yeniden çizim yok). */
export function useClock(active: boolean, ms = 1000): number {
  // subscribe kararlı olmalı: her çizimde yeni fonksiyon aboneliği (ve zamanlayıcıyı) sıfırlar, sayaç hiç ilerlemez.
  const sub = useCallback((l: () => void) => (active ? subscribe(ms, l) : () => undefined), [active, ms])
  return useSyncExternalStore(sub, () => (active ? tickerFor(ms).now : frozen()))
}
