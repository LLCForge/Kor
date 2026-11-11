// [g11] "Hafıza katmanları" penceresini her yerden (kenar çubuğu, slash komutu) açmak için küçük depo.
// Saf modül (React/DOM yok): slash-commands.ts testlerde doğrudan içe aktarılabilsin.
let open = false
const listeners = new Set<() => void>()

function set(value: boolean): void {
  open = value
  for (const l of listeners) l()
}

export const openG11Panel = () => set(true)
export const closeG11Panel = () => set(false)
export const isG11PanelOpen = () => open

export function subscribeG11Panel(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
