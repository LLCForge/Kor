import { realpath } from 'node:fs/promises'
import path from 'node:path'
import { resolveInWorkspace } from './tools.ts'

export function isInside(root: string, target: string): boolean {
  const rel = path.relative(root, target)
  return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel))
}

/**
 * resolveInWorkspace + symlink denetimi: hedefin (yoksa en yakın var olan atasının) gerçek yolu da
 * çalışma dizininde olmalı. Dizin içindeki bir bağlantı üzerinden dışarı okuma/yazma engellenir (CWE-59).
 */
export async function resolveReal(root: string, rel: string): Promise<string> {
  const abs = resolveInWorkspace(root, rel)
  const realRoot = await realpath(root)
  let probe = abs
  for (;;) {
    let real: string | null = null
    try {
      real = await realpath(probe)
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err
    }
    if (real !== null) {
      if (!isInside(realRoot, real)) throw new Error(`Çalışma dizini dışına erişim reddedildi: ${rel}`)
      return abs
    }
    const parent = path.dirname(probe)
    if (parent === probe) throw new Error(`Yol çözülemedi: ${rel}`)
    probe = parent
  }
}
