// [g17] Sohbet görünümü durumu: mesaj sabitleme, dışa aktarma seçimi, içerik türü filtresi, yan yana karşılaştırma.
//      Tek etkin sohbet için küçük harici depo (useSyncExternalStore); Thread her sohbette yeniden bağlanınca sıfırlanır.
import { useSyncExternalStore } from 'react'

export type ContentFilter = 'all' | 'code' | 'diagram' | 'image'

export interface G17View {
  pinned: ReadonlySet<string>
  selected: ReadonlySet<string>
  filter: ContentFilter
  compare: readonly string[]
}

const EMPTY: G17View = { pinned: new Set(), selected: new Set(), filter: 'all', compare: [] }
let state: G17View = EMPTY
const listeners = new Set<() => void>()

function set(next: G17View): void {
  state = next
  for (const l of listeners) l()
}

const toggleIn = (set: ReadonlySet<string>, id: string): Set<string> => {
  const next = new Set(set)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  return next
}

export const g17View = {
  reset: () => set(EMPTY),
  togglePinned: (id: string) => set({ ...state, pinned: toggleIn(state.pinned, id) }),
  toggleSelected: (id: string) => set({ ...state, selected: toggleIn(state.selected, id) }),
  clearSelected: () => set({ ...state, selected: new Set() }),
  setFilter: (filter: ContentFilter) => set({ ...state, filter }),
  toggleCompare: (id: string) =>
    set({
      ...state,
      // En çok iki yanıt karşılaştırılır; üçüncü seçim en eskiyi düşürür.
      compare: state.compare.includes(id) ? state.compare.filter((x) => x !== id) : [...state.compare, id].slice(-2),
    }),
  clearCompare: () => set({ ...state, compare: [] }),
}

export function useG17View(): G17View {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => state,
  )
}
