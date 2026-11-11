// Model seçicinin favorileri ve son kullanılan 3 model (§23.2, ADR 0053). Arayüz tercihi: localStorage (persisted.ts
// ile aynı gelenek); veri değil. Gruplama saf ve testlidir (model-prefs.test.ts).
import { useCallback, useSyncExternalStore } from 'react'

export const RECENT_MAX = 3
const FAV_KEY = 'kor.modelFavorites'
const RECENT_KEY = 'kor.modelRecent'

export function pushRecent(list: readonly string[], id: string, max = RECENT_MAX): string[] {
  return [id, ...list.filter((x) => x !== id)].slice(0, max)
}

export function toggleFavorite(list: readonly string[], id: string): string[] {
  return list.includes(id) ? list.filter((x) => x !== id) : [...list, id]
}

export interface ModelSection<T> {
  key: string
  label: string
  models: T[]
}

/**
 * Favoriler en üstte, sonra son kullanılan 3 model (hızlı erişim), sonra sağlayıcıya göre gruplar (sağlayıcı sırası
 * listedeki ilk görünüşe göre, böylece seçili sıralama korunur). Bir model yalnızca bir bölümde görünür.
 */
export function sectionModels<T extends { id: string }>(
  visible: readonly T[],
  favorites: readonly string[],
  recent: readonly string[],
  providerOf: (m: T) => string,
  providerLabel: (p: string) => string,
): ModelSection<T>[] {
  const byId = new Map(visible.map((m) => [m.id, m]))
  const used = new Set<string>()
  const pick = (ids: readonly string[]) =>
    ids.flatMap((id) => {
      const m = byId.get(id)
      if (!m || used.has(id)) return []
      used.add(id)
      return [m]
    })
  const out: ModelSection<T>[] = []
  const fav = pick(favorites)
  if (fav.length) out.push({ key: 'favorites', label: 'Favoriler', models: fav })
  const rec = pick(recent.slice(0, RECENT_MAX))
  if (rec.length) out.push({ key: 'recent', label: 'Son kullanılanlar', models: rec })
  const groups = new Map<string, T[]>()
  for (const m of visible) {
    if (used.has(m.id)) continue
    const p = providerOf(m)
    const g = groups.get(p)
    if (g) g.push(m)
    else groups.set(p, [m])
  }
  for (const [p, models] of groups) out.push({ key: `provider:${p}`, label: providerLabel(p), models })
  return out
}

function readList(key: string): string[] {
  try {
    const v: unknown = JSON.parse(window.localStorage.getItem(key) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, 200) : []
  } catch {
    return []
  }
}

const listeners = new Set<() => void>()
const cache = new Map<string, string[]>()

function get(key: string): string[] {
  let v = cache.get(key)
  if (!v) {
    v = readList(key)
    cache.set(key, v)
  }
  return v
}

function set(key: string, value: string[]): void {
  cache.set(key, value)
  try {
    window.localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // Depolama kapalıysa tercih bu oturumla sınırlı kalır.
  }
  for (const l of listeners) l()
}

function subscribe(l: () => void): () => void {
  listeners.add(l)
  return () => {
    listeners.delete(l)
  }
}

export function useModelPrefs(): { favorites: string[]; recent: string[]; toggleFav: (id: string) => void; markUsed: (id: string) => void } {
  const favorites = useSyncExternalStore(subscribe, () => get(FAV_KEY))
  const recent = useSyncExternalStore(subscribe, () => get(RECENT_KEY))
  const toggleFav = useCallback((id: string) => set(FAV_KEY, toggleFavorite(get(FAV_KEY), id)), [])
  const markUsed = useCallback((id: string) => set(RECENT_KEY, pushRecent(get(RECENT_KEY), id)), [])
  return { favorites, recent, toggleFav, markUsed }
}
