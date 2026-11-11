// [g16] Dağıtım ve tanılama penceresini (G16Panel) her yerden açmak için küçük ortak depo. CSS/bileşen içermez:
// slash-commands.ts testlerde Node altında yüklenir.
import { useSyncExternalStore } from 'react'

let open = false
const listeners = new Set<() => void>()

function set(next: boolean): void {
  open = next
  for (const listener of listeners) listener()
}

export const openG16Panel = () => set(true)
export const closeG16Panel = () => set(false)

export function useG16PanelOpen(): boolean {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => open,
  )
}
