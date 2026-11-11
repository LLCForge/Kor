// Grup 7 pencerelerini (ayarlar, model karşılaştırma, model testi) her yerden açmak için küçük ortak depo.
// Tek seferde tek pencere açık olur; G7Layer bunu dinler.
import { useSyncExternalStore } from 'react'
import useSWR from 'swr'
import { api } from './api.ts'

export type SettingsSection = 'general' | 'models' | 'permissions' | 'appearance' | 'privacy' | 'about' | 'providers' | 'costs' | 'usage' | 'capabilities' | 'memory' | 'design' | 'system' | 'extensions' | 'mcp' | 'plugins' | 'agents' | 'chrome' | 'connections' | 'routines' | 'core' | 'shell' | 'osint' | 'templates' | 'readability' | 'keymap' | 'automations' | 'security' | 'compat' | 'offline' | 'team' | 'hooks' | 'tools' | 'mcpdev' | 'prereqs'

export type G7Dialog = { kind: 'settings'; section?: SettingsSection } | { kind: 'compare' } | { kind: 'probe'; models?: string[] }

let current: G7Dialog | null = null
const listeners = new Set<() => void>()

function emit(): void {
  for (const l of listeners) l()
}

export function openG7Dialog(dialog: G7Dialog): void {
  current = dialog
  emit()
}

export function closeG7Dialog(): void {
  current = null
  emit()
}

export function useG7Dialog(): G7Dialog | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => current,
  )
}

/** Açık klasörün güven bilgisi; kök değişince yeniden alınır. */
export const trustKey = (root: string) => ['g7:trust', root] as const

/** Katalog SWR anahtarı: takma ad veya AA anahtarı değişince yenilemek için dışa açık. */
export const CATALOG_KEY = 'g7:catalog'

export function useCatalog(enabled = true) {
  return useSWR(enabled ? CATALOG_KEY : null, () => api.g7GetCatalog(false), { revalidateOnFocus: false, dedupingInterval: 60_000 })
}
