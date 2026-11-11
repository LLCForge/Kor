// [cdx] Satır içi mesaj düzenleme durumu: hangi kullanıcı mesajı düzenleniyor (cdesktop balonun yerine turuncu kutu).
import { useSyncExternalStore } from 'react'

let editingId: string | null = null
const listeners = new Set<() => void>()
const emit = () => { for (const l of listeners) l() }

export function startEditing(id: string): void {
  editingId = id
  emit()
}
export function stopEditing(): void {
  editingId = null
  emit()
}
export function useEditingId(): string | null {
  return useSyncExternalStore(
    (l) => { listeners.add(l); return () => listeners.delete(l) },
    () => editingId,
  )
}
