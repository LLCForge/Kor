import { appendFile, mkdir, readdir, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import { USER_CONFIG_DIR } from './settings.ts'

export const AUDIT_DIR = path.join(USER_CONFIG_DIR, 'audit')
const MAX_FIELD_CHARS = 500

export interface AuditEntry {
  sessionId: string
  root: string
  model: string
  tool: string
  args: unknown
  decision: 'allow' | 'deny' | 'approved' | 'rejected' | 'invalid'
  ok?: boolean
  durationMs?: number
  resultChars?: number
  reason?: string
}

/** Uzun alanları (dosya içerikleri gibi) kısaltır; tam içerik günlüğe yazılmaz. */
function compact(value: unknown): unknown {
  if (typeof value === 'string') {
    return value.length > MAX_FIELD_CHARS ? `${value.slice(0, MAX_FIELD_CHARS)}… [${value.length} karakter]` : value
  }
  if (Array.isArray(value)) return value.slice(0, 50).map(compact)
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, compact(v)]))
  }
  return value
}

let warned = false

/**
 * Günlük, çalışma dizininin DIŞINDA tutulur; ajan kendi kaydını değiştiremez.
 * Günlüğe yazılamaması ajanı durdurmaz, bir kez uyarı verir.
 */
export async function writeAudit(entry: AuditEntry): Promise<void> {
  const now = new Date()
  const file = path.join(AUDIT_DIR, `${now.toISOString().slice(0, 10)}.jsonl`)
  const line = JSON.stringify({ ts: now.toISOString(), ...entry, args: compact(entry.args) })
  try {
    await mkdir(AUDIT_DIR, { recursive: true })
    await appendFile(file, `${line}\n`, { encoding: 'utf8', mode: 0o600 })
  } catch (err) {
    if (!warned) {
      warned = true
      console.error(`Audit log yazılamadı: ${err instanceof Error ? err.message : String(err)}`)
    }
  }
}

export const AUDIT_DEFAULT_KEEP_DAYS = 30
export const AUDIT_MAX_BYTES = 200 * 1024 * 1024

/**
 * Günlük budama: keepDays'ten eski gün dosyaları silinir; toplam boyut maxBytes'ı aşıyorsa en eskiden başlayarak
 * sınırın altına inilir (bugünün dosyası korunur). Uygulama açılışında ve günde bir çağrılır; hata ajanı durdurmaz.
 */
export async function pruneAudit(opts: { keepDays?: number; maxBytes?: number; now?: number; dir?: string } = {}): Promise<{ removed: string[] }> {
  const dir = opts.dir ?? AUDIT_DIR
  const keepDays = opts.keepDays ?? AUDIT_DEFAULT_KEEP_DAYS
  const maxBytes = opts.maxBytes ?? AUDIT_MAX_BYTES
  const now = opts.now ?? Date.now()
  const removed: string[] = []
  let names: string[]
  try {
    names = (await readdir(dir)).filter((n) => /^\d{4}-\d{2}-\d{2}\.jsonl$/.test(n)).sort()
  } catch {
    return { removed }
  }
  const today = new Date(now).toISOString().slice(0, 10)
  const cutoff = new Date(now - keepDays * 86_400_000).toISOString().slice(0, 10)
  const sized: Array<{ name: string; size: number }> = []
  for (const name of names) {
    const day = name.slice(0, 10)
    if (day < cutoff && day !== today) {
      await rm(path.join(dir, name), { force: true }).catch(() => undefined)
      removed.push(name)
      continue
    }
    const size = await stat(path.join(dir, name)).then((s) => s.size).catch(() => 0)
    sized.push({ name, size })
  }
  let total = sized.reduce((a, b) => a + b.size, 0)
  for (const f of sized) {
    if (total <= maxBytes || f.name.slice(0, 10) === today) continue
    await rm(path.join(dir, f.name), { force: true }).catch(() => undefined)
    removed.push(f.name)
    total -= f.size
  }
  return { removed }
}
