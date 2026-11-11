// [g14] Gözlem panosunu her yerden (slash komutu, kenar çubuğu) açmak için saf veriyolu.
// React/DOM içermez: slash-commands.ts testlerde doğrudan içe aktarılır.

export type G14Tab = 'overview' | 'traces' | 'tools' | 'cost' | 'events' | 'processes' | 'raw' | 'settings'

const listeners = new Set<(tab: G14Tab) => void>()

export function openObservability(tab: G14Tab = 'overview'): void {
  for (const l of listeners) l(tab)
}

export function onOpenObservability(listener: (tab: G14Tab) => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
