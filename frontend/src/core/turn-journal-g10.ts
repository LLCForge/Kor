// [g10] Tur günlüğü (çökme ve kısmi başarısızlık kurtarma) ve ölü mektup kuyruğu (başarısız arka plan görevleri).
// Günlük her model adımından önce diske yazılır: tamamlanmış araç adımları uygulama çökse veya tur hata verse de kaybolmaz.
import { createHash, randomUUID } from 'node:crypto'
import { mkdir, readdir, readFile, rm } from 'node:fs/promises'
import path from 'node:path'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import { z } from 'zod'
import { atomicWriteFile } from './atomic-write-g10.ts'
import { USER_CONFIG_DIR } from './settings.ts'

const G10_DIR = () => path.join(USER_CONFIG_DIR, 'g10')
const JOURNAL_DIR = () => path.join(G10_DIR(), 'journal')
const DEAD_LETTER_FILE = () => path.join(G10_DIR(), 'dead-letters.json')
const MAX_JOURNAL_CHARS = 8_000_000
export const MAX_DEAD_LETTERS = 100

const uuid = z.string().uuid()
const messageSchema = z.object({ role: z.enum(['system', 'user', 'assistant', 'tool']) }).passthrough()

const journalSchema = z.object({
  version: z.literal(1),
  sessionId: uuid,
  root: z.string().max(4096),
  startedAt: z.number(),
  updatedAt: z.number(),
  /** Turun ilk mesajından önceki mesajın parmak izi; null = tur sistem isteminden hemen sonra başlıyor */
  prevFingerprint: z.string().nullable(),
  /** Turun başı sıkıştırmada özetlendiyse günlük sistem mesajından sonrasının tamamını tutar */
  fromStart: z.boolean(),
  messages: z.array(messageSchema).min(1),
  /** Son model adımından sonra tamamlanan ama konuşmaya henüz yazılamamış yan etkili araçlar */
  applied: z.array(z.string().max(300)).max(200),
})

export type TurnJournal = Omit<z.infer<typeof journalSchema>, 'messages'> & { messages: ChatCompletionMessageParam[] }

export function fingerprint(message: ChatCompletionMessageParam | undefined): string | null {
  return message ? createHash('sha256').update(JSON.stringify(message)).digest('hex').slice(0, 32) : null
}

const journalFile = (sessionId: string) => path.join(JOURNAL_DIR(), `${uuid.parse(sessionId)}.json`)

/** Serileştirmeden kaba boyut: sınırı aşan tur her adımda boşuna JSON'a çevrilmesin. */
function approxChars(messages: readonly ChatCompletionMessageParam[]): number {
  let n = 0
  for (const m of messages) {
    if (typeof m.content === 'string') n += m.content.length
    else if (Array.isArray(m.content)) {
      for (const p of m.content as ReadonlyArray<{ text?: string; image_url?: { url?: string } }>) n += (p.text?.length ?? 0) + (p.image_url?.url?.length ?? 0)
    }
    if (m.role === 'assistant') for (const c of m.tool_calls ?? []) if (c.type === 'function') n += c.function.arguments.length
  }
  return n
}

export async function writeJournal(j: TurnJournal): Promise<void> {
  // Son başarılı günlük diskte kalır: kurtarma son sığan adıma döner.
  const text = approxChars(j.messages) > MAX_JOURNAL_CHARS ? null : JSON.stringify(j)
  if (text === null || text.length > MAX_JOURNAL_CHARS) {
    console.warn('Tur günlüğü çok büyük; bu adım kaydedilmedi.')
    return
  }
  await mkdir(JOURNAL_DIR(), { recursive: true })
  await atomicWriteFile(journalFile(j.sessionId), text)
}

export async function readJournal(sessionId: string): Promise<TurnJournal | null> {
  let text: string
  try {
    text = await readFile(journalFile(sessionId), 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
  const parsed = journalSchema.safeParse(JSON.parse(text))
  if (!parsed.success) {
    console.warn('Bozuk tur günlüğü yok sayıldı:', sessionId)
    return null
  }
  return { ...parsed.data, messages: parsed.data.messages as ChatCompletionMessageParam[] }
}

export async function deleteJournal(sessionId: string): Promise<void> {
  await rm(journalFile(sessionId), { force: true })
}

export async function listJournalIds(): Promise<string[]> {
  try {
    const names = await readdir(JOURNAL_DIR())
    return names.map((n) => n.replace(/\.json$/, '')).filter((id) => uuid.safeParse(id).success)
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw err
  }
}

/**
 * Sonuçları henüz eklenmemiş araç çağrısı grubunu (adım sürerken) keser: API, her tool_calls mesajının ardından
 * tüm sonuçları bekler; yarım grup geri yüklenirse sonraki istek reddedilir.
 */
export function completeGroups(messages: readonly ChatCompletionMessageParam[]): ChatCompletionMessageParam[] {
  for (let i = 0; i < messages.length; i++) {
    const m = messages[i]
    if (m?.role !== 'assistant' || !m.tool_calls?.length) continue
    const answered = new Set<string>()
    for (let k = i + 1; k < messages.length && messages[k]?.role === 'tool'; k++) {
      const t = messages[k]
      if (t?.role === 'tool') answered.add(t.tool_call_id)
    }
    if (!m.tool_calls.every((c) => answered.has(c.id))) return messages.slice(0, i)
  }
  return [...messages]
}

const same = (a: ChatCompletionMessageParam | undefined, b: ChatCompletionMessageParam | undefined) =>
  a !== undefined && b !== undefined && JSON.stringify(a) === JSON.stringify(b)

/** Tur başlangıcı mevcut konuşmada hâlâ duruyor mu (tur geri alınmadıysa true). */
export function turnStillPresent(current: readonly ChatCompletionMessageParam[], j: TurnJournal): boolean {
  if (j.fromStart) return same(current[1], j.messages[0]) && current.length >= j.messages.length + 1
  const first = j.messages[0]
  return current.some((m) => same(m, first))
}

/**
 * Günlükteki tamamlanmış adımları konuşmaya geri ekler. Konuşma tur başladıktan sonra başka yönde değiştiyse null
 * (güvenli değil). Günlük her zaman araç çağrısı/sonuç çiftleri tamamken yazıldığından sonuç API'ye geçerli gider.
 */
export function reconstruct(current: readonly ChatCompletionMessageParam[], j: TurnJournal): ChatCompletionMessageParam[] | null {
  const system = current[0]
  if (!system) return null
  // Mevcut konuşmanın tur kısmı günlüğün öneki olmalı: günlükten ileride olan bir konuşmadan mesaj asla atılmaz.
  const isPrefix = (from: number) => current.length - from <= j.messages.length && current.slice(from).every((m, k) => same(m, j.messages[k]))
  if (j.fromStart) return current.length === 1 || isPrefix(1) ? [system, ...j.messages] : null
  const first = j.messages[0]
  for (let i = current.length - 1; i >= 1; i--) if (same(current[i], first)) return isPrefix(i) ? [...current.slice(0, i), ...j.messages] : null
  // Tur geri alınmış (hata): konuşma tam tur öncesinde bitiyorsa günlük sona eklenir.
  // prevFingerprint null: tur ilk turdu (öncesinde yalnızca her açılışta yenilenen sistem istemi var).
  const atTurnStart = j.prevFingerprint === null ? current.length === 1 : fingerprint(current.at(-1)) === j.prevFingerprint
  return atTurnStart ? [...current, ...j.messages] : null
}

export interface JournalStep {
  tool: string
  ok: boolean
}

function argsPath(raw: string): string | null {
  try {
    const value: unknown = JSON.parse(raw)
    const p = typeof value === 'object' && value !== null ? (value as Record<string, unknown>).path : undefined
    return typeof p === 'string' ? p : null
  } catch {
    return null
  }
}

const FILE_TOOLS = new Set(['write_file', 'edit_file', 'multi_edit', 'edit_notebook'])

/** Günlükteki tamamlanmış araç adımları ve değiştirilen dosyalar (arayüz özeti için). */
export function journalSummary(j: TurnJournal): { steps: JournalStep[]; files: string[] } {
  const results = new Map<string, string>()
  for (const m of j.messages) if (m.role === 'tool' && typeof m.content === 'string') results.set(m.tool_call_id, m.content)
  const steps: JournalStep[] = []
  const files = new Set<string>()
  for (const m of j.messages) {
    if (m.role !== 'assistant') continue
    for (const call of m.tool_calls ?? []) {
      if (call.type !== 'function') continue
      const result = results.get(call.id)
      if (result === undefined) continue
      const ok = !/^(Hata|Engellendi|Kullanıcı bu işlemi reddetti|Döngü)/.test(result)
      steps.push({ tool: call.function.name, ok })
      const p = ok && FILE_TOOLS.has(call.function.name) ? argsPath(call.function.arguments) : null
      if (p) files.add(p)
    }
  }
  return { steps, files: [...files] }
}

export function continuationPrompt(j: TurnJournal): string {
  const extra = j.applied.length > 0 ? `\nAyrıca kesintiden hemen önce şu işlemler tamamlanmıştı (sonuçları konuşmada yok): ${j.applied.join('; ')}.` : ''
  return (
    '[Kurtarma] Önceki tur uygulama kapandığı veya bir hata oluştuğu için yarıda kaldı. Yukarıdaki tamamlanmış adımlar uygulandı.' +
    `${extra}\nDosyaların güncel halini varsay; değiştirmeden önce ilgili dosyaları yeniden oku. Tamamlanan adımları tekrarlama ve kalan adımlardan devam et.`
  )
}

// ---------- Ölü mektup kuyruğu ----------

const deadLetterSchema = z.object({
  id: uuid,
  sessionId: uuid,
  kind: z.enum(['subagent', 'shell']),
  title: z.string().max(500),
  error: z.string().max(4000),
  at: z.number(),
})

export type DeadLetter = z.infer<typeof deadLetterSchema>

let dlqQueue: Promise<unknown> = Promise.resolve()

export async function listDeadLetters(): Promise<DeadLetter[]> {
  try {
    const parsed = z.array(deadLetterSchema).safeParse(JSON.parse(await readFile(DEAD_LETTER_FILE(), 'utf8')))
    if (parsed.success) return parsed.data
    console.warn('Ölü mektup kuyruğu dosyası bozuk; boş sayıldı.')
    return []
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw err
  }
}

function mutateDeadLetters(fn: (list: DeadLetter[]) => DeadLetter[]): Promise<DeadLetter[]> {
  const run = async () => {
    const next = fn(await listDeadLetters()).slice(-MAX_DEAD_LETTERS)
    await mkdir(G10_DIR(), { recursive: true })
    await atomicWriteFile(DEAD_LETTER_FILE(), JSON.stringify(next))
    return next
  }
  const result = dlqQueue.then(run, run)
  dlqQueue = result.catch(() => undefined)
  return result
}

export function addDeadLetter(entry: Omit<DeadLetter, 'id' | 'at'>): Promise<DeadLetter[]> {
  const item: DeadLetter = {
    ...entry,
    id: randomUUID(),
    at: Date.now(),
    title: entry.title.slice(0, 500),
    error: entry.error.slice(0, 4000),
  }
  return mutateDeadLetters((list) => [...list, item])
}

export function removeDeadLetter(id: string): Promise<DeadLetter[]> {
  return mutateDeadLetters((list) => list.filter((d) => d.id !== id))
}

export function retryPrompt(d: DeadLetter): string {
  const what = d.kind === 'subagent' ? `arka plan alt ajanı (${d.title})` : `arka plan komutu: ${d.title}`
  return `Daha önce başarısız olan şu görevi yeniden dene: ${what}.\nHata: ${d.error}\nÖnce hatanın nedenini kısaca değerlendir; geçici bir sorunsa aynı görevi yeniden başlat, değilse düzeltip öyle başlat.`
}
