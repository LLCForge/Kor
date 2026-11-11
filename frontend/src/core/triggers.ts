// Zamanlanmış görevler (tetikleyiciler): belirli aralıklarla bir istemle ajanı çalıştırır. Depo kullanıcı düzeyindedir
// (~/.kor/triggers.json). Tetikleme yalnızca masaüstü uygulaması AÇIKKEN olur (kalıcı arka plan servisi yok)
// ve GÖZETİMSİZDİR: onay isteyen araçlar reddedilir, yani görev yalnızca önceden izin verilen (ve salt okunur)
// araçları kullanabilir. Aralık cron yerine dakikadır (sağlam, ayrıştırıcı gerektirmez).
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import path from 'node:path'
import { z } from 'zod'
import { USER_CONFIG_DIR } from './settings.ts'

const FILE = 'triggers.json'
const MIN_INTERVAL = 1
const MAX_INTERVAL = 7 * 24 * 60 // bir hafta

export interface Trigger {
  id: string
  name: string
  prompt: string
  intervalMinutes: number
  root: string
  enabled: boolean
  createdAt: number
  lastRun: number | null
  nextRun: number
}

const triggerSchema = z.object({
  id: z.string().uuid(),
  name: z.string().max(120),
  prompt: z.string().min(1).max(10_000),
  intervalMinutes: z.number().int().min(MIN_INTERVAL).max(MAX_INTERVAL),
  root: z.string().max(4096),
  enabled: z.boolean(),
  createdAt: z.number(),
  lastRun: z.number().nullable(),
  nextRun: z.number(),
})
const fileSchema = z.object({ triggers: z.array(triggerSchema).max(100) })

export async function loadTriggers(dir: string = USER_CONFIG_DIR): Promise<Trigger[]> {
  try {
    const parsed = fileSchema.safeParse(JSON.parse(await readFile(path.join(dir, FILE), 'utf8')))
    return parsed.success ? parsed.data.triggers : []
  } catch {
    return []
  }
}

async function writeTriggers(triggers: Trigger[], dir: string): Promise<void> {
  await mkdir(dir, { recursive: true })
  await writeFile(path.join(dir, FILE), JSON.stringify({ triggers }, null, 2), { mode: 0o600 })
}

export async function addTrigger(input: { name?: string; prompt: string; intervalMinutes: number; root: string }, dir: string = USER_CONFIG_DIR, now: number = Date.now()): Promise<Trigger> {
  const interval = Math.max(MIN_INTERVAL, Math.min(MAX_INTERVAL, Math.round(input.intervalMinutes)))
  const trigger: Trigger = {
    id: randomUUID(),
    name: (input.name ?? input.prompt).slice(0, 120),
    prompt: input.prompt,
    intervalMinutes: interval,
    root: input.root,
    enabled: true,
    createdAt: now,
    lastRun: null,
    nextRun: now + interval * 60_000,
  }
  const triggers = await loadTriggers(dir)
  triggers.push(trigger)
  await writeTriggers(triggers, dir)
  return trigger
}

export async function setTriggerEnabled(id: string, enabled: boolean, dir: string = USER_CONFIG_DIR): Promise<boolean> {
  const triggers = await loadTriggers(dir)
  const t = triggers.find((x) => x.id === id)
  if (!t) return false
  t.enabled = enabled
  // Yeniden etkinleştirince bir sonraki çalışma zamanı şimdiden ileriye alınır.
  if (enabled) t.nextRun = Date.now() + t.intervalMinutes * 60_000
  await writeTriggers(triggers, dir)
  return true
}

export async function removeTrigger(id: string, dir: string = USER_CONFIG_DIR): Promise<boolean> {
  const triggers = await loadTriggers(dir)
  const next = triggers.filter((t) => t.id !== id)
  if (next.length === triggers.length) return false
  await writeTriggers(next, dir)
  return true
}

/** Çalışma zamanı gelmiş etkin tetikleyiciler. */
export function dueTriggers(triggers: readonly Trigger[], now: number = Date.now()): Trigger[] {
  return triggers.filter((t) => t.enabled && t.nextRun <= now)
}

/** Tetikleme sonrası lastRun/nextRun güncellenir (sürüklenme birikmesin diye now'dan ileri). */
export async function markFired(id: string, dir: string = USER_CONFIG_DIR, now: number = Date.now()): Promise<void> {
  const triggers = await loadTriggers(dir)
  const t = triggers.find((x) => x.id === id)
  if (!t) return
  t.lastRun = now
  t.nextRun = now + t.intervalMinutes * 60_000
  await writeTriggers(triggers, dir)
}
