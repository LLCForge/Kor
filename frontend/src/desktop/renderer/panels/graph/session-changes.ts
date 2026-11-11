// "Bu oturumda değişenler" katmanı: ajanın `fileChanged` olaylarını sohbet başına toplar. Panel kayıt defterinden
// uygulama açılışında içe aktarıldığı için abonelik ilk turdan önce kurulur; panel kapalıyken de birikir.
// shortcut: geçmişten yeniden açılan bir sohbetin eski değişiklikleri sayılmaz (olay yeniden oynatılmaz).
import { api } from '../../api.ts'

const changed = new Map<string, Set<string>>()
const listeners = new Set<() => void>()

api.onEvent((e) => {
  if (e.type !== 'fileChanged') return
  const prev = changed.get(e.sessionId)
  if (prev?.has(e.path)) return
  // Yeni küme: useSyncExternalStore kimlik değişiminden anlar.
  changed.set(e.sessionId, new Set(prev).add(e.path))
  for (const l of listeners) l()
})

const EMPTY: ReadonlySet<string> = new Set()

export function changedFiles(sessionId: string | null): ReadonlySet<string> {
  return (sessionId !== null && changed.get(sessionId)) || EMPTY
}

export function onChangedFiles(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
