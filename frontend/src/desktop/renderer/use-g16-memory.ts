// [g16] Bellek baskısı yönetimi (243). Arka plandaki boştaki ağır sohbetlerin arayüz durumu boşaltılır ('reset');
// sohbet yeniden görüntülenince ana süreçteki kayıttan (loadSessionView) geri yüklenir. Çalışan, onay bekleyen,
// görünen ve değişiklik listesi olan sohbetlere dokunulmaz.
import { useEffect, useRef } from 'react'
import { api, errorMessage } from './api.ts'
import type { ChatAction, ChatStore } from './chat-state.ts'
import { pickEvictions } from './g16-memory.ts'

const CHECK_INTERVAL_MS = 30_000
/** Arka planda tutulan toplam metin (~8 MB UTF-16); aşılınca en uzun süredir bakılmayanlar boşaltılır. */
const BACKGROUND_BUDGET_CHARS = 4_000_000
/** JS yığını sınırına bu oranda yaklaşılınca boşaltılabilen tüm arka plan sohbetleri boşaltılır. */
const HEAP_PRESSURE_RATIO = 0.7

interface Options {
  store: ChatStore
  activeId: string
  splitId: string | null
  runningIds: readonly string[] | undefined
  dispatchStore: (action: { sessionId: string; action: ChatAction }) => void
}

/** Chromium'a özgü performance.memory; yoksa yalnızca bütçe uygulanır. */
function underHeapPressure(): boolean {
  const memory = (performance as { memory?: { usedJSHeapSize: number; jsHeapSizeLimit: number } }).memory
  return memory !== undefined && memory.jsHeapSizeLimit > 0 && memory.usedJSHeapSize / memory.jsHeapSizeLimit > HEAP_PRESSURE_RATIO
}

export function useG16Memory({ store, activeId, splitId, runningIds, dispatchStore }: Options): void {
  const latest = useRef({ store, activeId, splitId, runningIds })
  latest.current = { store, activeId, splitId, runningIds }
  const lastUsed = useRef(new Map<string, number>())
  const evicted = useRef(new Set<string>())

  useEffect(() => {
    const now = Date.now()
    lastUsed.current.set(activeId, now)
    if (splitId) lastUsed.current.set(splitId, now)
  }, [activeId, splitId])

  useEffect(() => {
    const timer = setInterval(() => {
      const { store: current, activeId: active, splitId: split, runningIds: running } = latest.current
      const protect = new Set([active, ...(split ? [split] : []), ...(running ?? [])])
      const budget = underHeapPressure() ? 0 : BACKGROUND_BUDGET_CHARS
      for (const id of pickEvictions(current, { protect, lastUsed: lastUsed.current, budget })) {
        evicted.current.add(id)
        dispatchStore({ sessionId: id, action: { type: 'reset' } })
      }
    }, CHECK_INTERVAL_MS)
    return () => clearInterval(timer)
  }, [dispatchStore])

  // Boşaltılmış sohbet görünür olunca ve çalışmıyorken geri yüklenir; boşaltmadan sonra gelen dosya değişiklikleri korunur.
  useEffect(() => {
    for (const id of [activeId, splitId]) {
      if (!id || !evicted.current.has(id)) continue
      const chat = store[id]
      if (chat?.running || runningIds?.includes(id)) continue
      evicted.current.delete(id)
      const laterChanges = chat?.changes ?? []
      api
        .loadSessionView(id)
        .then((transcript) => {
          dispatchStore({ sessionId: id, action: { type: 'load', transcript } })
          if (laterChanges.length > 0) dispatchStore({ sessionId: id, action: { type: 'rewound', changes: laterChanges, cutFromItemId: null } })
        })
        .catch((err: unknown) => console.error('Sohbet geri yüklenemedi:', errorMessage(err)))
    }
  }, [activeId, splitId, store, runningIds, dispatchStore])
}
