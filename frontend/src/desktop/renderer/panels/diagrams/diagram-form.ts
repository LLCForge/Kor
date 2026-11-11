// §27.14 diyagram paneli yardımcıları.
import type { DiagramStatus } from '../../../ipc-ek14.ts'

/** `scope=src depth=2` → `{ scope: 'src', depth: '2' }` (anahtar yalnızca harf/rakam/_). */
export function parseParams(text: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const part of text.split(/\s+/)) {
    const i = part.indexOf('=')
    if (i <= 0) continue
    const k = part.slice(0, i).toLowerCase()
    const v = part.slice(i + 1)
    if (/^[a-z0-9_]+$/.test(k) && v) out[k] = v
  }
  return out
}

export function statusLabel(s: DiagramStatus): string {
  switch (s) {
    case 'up_to_date':
      return 'güncel'
    case 'stale':
      return 'eskimiş'
    case 'edited':
      return 'elle düzenlenmiş'
    case 'invalid':
      return 'geçersiz'
  }
}
