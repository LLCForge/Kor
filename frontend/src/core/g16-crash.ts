// [g16] Çökme raporlayıcı (200): yerel olay günlüğü, döküm listesi ve maskeli rapor. Electron'dan bağımsız (test edilebilir).
import { createHash } from 'node:crypto'
import { appendFile, readdir, readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { maskSecrets } from './secrets.ts'

const MAX_EVENTS = 200
const MAX_DUMPS = 50

export const crashEventSchema = z.object({
  at: z.number(),
  process: z.string().max(100),
  reason: z.string().max(100),
  exitCode: z.number().nullable(),
  detail: z.string().max(500).optional(),
})
export type CrashEvent = z.infer<typeof crashEventSchema>

export interface DumpInfo {
  name: string
  size: number
  mtimeMs: number
}

/** Bozuk satırlar atlanır; günlük yalnızca tanılama içindir, okuma hatası uygulamayı durdurmaz. */
export async function readCrashEvents(file: string): Promise<CrashEvent[]> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw err
  }
  return text.split('\n').flatMap((line) => {
    if (!line.trim()) return []
    try {
      const parsed = crashEventSchema.safeParse(JSON.parse(line))
      return parsed.success ? [parsed.data] : []
    } catch {
      return []
    }
  })
}

/** Günlük son MAX_EVENTS kayıtla sınırlı kalır (sınırsız büyüme yok). */
export async function appendCrashEvent(file: string, event: CrashEvent): Promise<void> {
  const events = await readCrashEvents(file)
  if (events.length >= MAX_EVENTS) {
    const kept = [...events.slice(-(MAX_EVENTS - 1)), event]
    await writeFile(file, `${kept.map((e) => JSON.stringify(e)).join('\n')}\n`, { encoding: 'utf8', mode: 0o600 })
    return
  }
  await appendFile(file, `${JSON.stringify(event)}\n`, { encoding: 'utf8', mode: 0o600 })
}

/** Crashpad dökümleri (*.dmp) alt klasörlerde durur (reports, completed, pending); derinlik ve sayı sınırlı. */
export async function listDumps(dir: string, depth = 3): Promise<Array<DumpInfo & { file: string }>> {
  const found: Array<DumpInfo & { file: string }> = []
  const walk = async (current: string, level: number): Promise<void> => {
    let entries
    try {
      entries = await readdir(current, { withFileTypes: true })
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') return
      throw err
    }
    for (const entry of entries) {
      const full = path.join(current, entry.name)
      if (entry.isDirectory() && level < depth) await walk(full, level + 1)
      else if (entry.isFile() && entry.name.toLowerCase().endsWith('.dmp')) {
        const info = await stat(full)
        found.push({ file: full, name: entry.name, size: info.size, mtimeMs: info.mtimeMs })
      }
    }
  }
  await walk(dir, 0)
  return found.sort((a, b) => b.mtimeMs - a.mtimeMs).slice(0, MAX_DUMPS)
}

const escapeRegExp = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** Gizli değerler, ev klasörü ve kullanıcı adı maskelenir (Windows yolları büyük/küçük harf duyarsız). */
export function maskPersonal(text: string, who: { home: string; user: string }): string {
  let out = maskSecrets(text)
  for (const home of new Set([who.home, who.home.replaceAll('\\', '/'), JSON.stringify(who.home).slice(1, -1)])) {
    if (home.length >= 3) out = out.replace(new RegExp(escapeRegExp(home), 'gi'), '~')
  }
  if (who.user.length >= 3) out = out.replace(new RegExp(`\\b${escapeRegExp(who.user)}\\b`, 'gi'), '<kullanıcı>')
  return out
}

export interface CrashReportMeta {
  appVersion: string
  electron: string
  platform: string
  arch: string
  distribution: string
}

/**
 * Dışa aktarılan/gönderilen rapor: olay günlüğü + döküm adları, boyutları ve SHA-256 özetleri. Dökümlerin kendisi
 * eklenmez: işlem belleğinden parça içerir ve ikili içerik güvenilir biçimde maskelenemez.
 */
export async function buildCrashReport(
  meta: CrashReportMeta,
  events: readonly CrashEvent[],
  dumps: ReadonlyArray<DumpInfo & { file: string }>,
  who: { home: string; user: string },
  now: Date = new Date(),
): Promise<string> {
  const dumpSummaries = await Promise.all(
    dumps.map(async (d) => ({
      name: d.name,
      size: d.size,
      at: new Date(d.mtimeMs).toISOString(),
      sha256: createHash('sha256').update(await readFile(d.file)).digest('hex'),
    })),
  )
  const report = {
    kind: 'kor-crash-report',
    version: 1,
    createdAt: now.toISOString(),
    ...meta,
    events: events.map((e) => ({ ...e, at: new Date(e.at).toISOString() })),
    dumps: dumpSummaries,
  }
  return maskPersonal(JSON.stringify(report, null, 2), who)
}
