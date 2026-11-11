import { useSyncExternalStore } from 'react'
import { z } from 'zod'
import { REASONING_EFFORTS } from '../../../core/reasoning.ts'

// Mesaj kutusu tercihleri (vim, akıl yürütme düzeyi): yalnızca bu cihazdaki görünüm kolaylığı,
// kaybolursa varsayılana döner. Mesaj kutusu yeniden bağlansa (boş ekran -> sohbet) da korunur.
const KEY = 'kor.g2.prefs'

const prefsSchema = z.object({
  vim: z.boolean(),
  effort: z.enum(REASONING_EFFORTS).nullable(),
  /** [cdx] Mesaj gönderme kısayolu: Enter (Shift+Enter yeni satır) ya da Ctrl/⌘+Enter (Enter yeni satır) */
  sendKey: z.enum(['enter', 'ctrlEnter']).default('enter'),
})

export type G2Prefs = z.infer<typeof prefsSchema>

const DEFAULTS: G2Prefs = { vim: false, effort: null, sendKey: 'enter' }

function load(): G2Prefs {
  try {
    const raw = localStorage.getItem(KEY)
    const parsed = raw ? prefsSchema.safeParse(JSON.parse(raw)) : null
    return parsed?.success ? parsed.data : DEFAULTS
  } catch (err) {
    console.warn('Mesaj kutusu tercihleri okunamadı:', err)
    return DEFAULTS
  }
}

let prefs = load()
const listeners = new Set<() => void>()

export const getG2Prefs = (): G2Prefs => prefs

export function setG2Prefs(patch: Partial<G2Prefs>): void {
  prefs = { ...prefs, ...patch }
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs))
  } catch (err) {
    console.warn('Mesaj kutusu tercihleri kaydedilemedi:', err)
  }
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useG2Prefs(): G2Prefs {
  return useSyncExternalStore(subscribe, getG2Prefs)
}
