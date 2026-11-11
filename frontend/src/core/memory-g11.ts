// [g11] Katmanlı hafıza: oturum (bellekte, bu sohbet), proje (.kor/knowledge.json, güven kapılı), global (memory.md).
// Proje katmanı aynı zamanda yapılandırılmış bilgi tabanıdır: karar, kısıt, terim ve not kayıtları (zod şemalı).
import { createHash, randomBytes } from 'node:crypto'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { conflictKey, findConflicts, fold, type Conflict } from './intel-g11.ts'
import { loadMemory, MEMORY_FILE, saveMemory } from './memory.ts'
import { ensureMapDir } from './project-map.ts'
import { maskSecrets } from './secrets.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { isFolderTrusted, recordGeneratedFile } from './trust.ts'
import { confineToWorkspace } from './workspace.ts'
import { adoptProject } from './layout-g28.ts' // [g28]

/** Köke göre; trust.ts PROJECT_TRUST_FILES listesinde de var (depoyla gelen kopya güven sorusunu tetikler). */
export const KNOWLEDGE_FILE = path.join('.kor', 'knowledge.json')
export const KNOWLEDGE_KINDS = ['decision', 'constraint', 'term', 'note'] as const
export type KnowledgeKind = (typeof KNOWLEDGE_KINDS)[number]
export type MemoryLayer = 'session' | 'project' | 'global'

export const MAX_KNOWLEDGE_RECORDS = 300
export const MAX_SESSION_NOTES = 30
export const RECORD_ID = /^[sgk][0-9a-f]{8}$/

export interface MemoryRecord {
  id: string
  layer: MemoryLayer
  text: string
  kind?: KnowledgeKind
  subject?: string
}

const knowledgeRecordSchema = z.object({
  id: z.string().regex(/^k[0-9a-f]{8}$/),
  kind: z.enum(KNOWLEDGE_KINDS),
  subject: z.string().trim().min(1).max(120),
  statement: z.string().trim().min(1).max(1000),
  createdAt: z.number().int().nonnegative(),
})
type KnowledgeRecord = z.infer<typeof knowledgeRecordSchema>

const knowledgeFileSchema = z.object({
  version: z.literal(1),
  records: z.array(knowledgeRecordSchema).max(MAX_KNOWLEDGE_RECORDS),
})

export const knowledgeInput = z.object({
  kind: z.enum(KNOWLEDGE_KINDS).describe('decision: verilmiş karar, constraint: uyulması gereken kısıt, term: alan terimi tanımı, note: diğer kalıcı proje bilgisi'),
  subject: z.string().trim().min(1).max(120).describe('Kısa konu/terim adı; aynı konu için hep aynı başlığı kullan (ör. "Paket yöneticisi")'),
  statement: z.string().trim().min(1).max(1000).describe('Tek cümlelik karar, kısıt veya tanım'),
})
export type KnowledgeInput = z.infer<typeof knowledgeInput>

const newId = (prefix: 's' | 'k') => `${prefix}${randomBytes(4).toString('hex')}`
const globalId = (line: string) => `g${createHash('sha256').update(line, 'utf8').digest('hex').slice(0, 8)}`

/** Hafızaya API anahtarı/token yazılmasın: maskeleme bir şey değiştiriyorsa metin gizli bilgi içeriyordur. */
function assertNoSecrets(text: string): void {
  if (maskSecrets(text) !== text) throw new Error('Metin gizli bilgi (anahtar/token) içeriyor; hafızaya kaydedilmedi.')
}

/** İzin onayında kaydın kendisini gösterebilmek için (araç summarize'ı eşzamanlıdır). */
const previews = new Map<string, string>()
export const recordPreview = (id: string) => previews.get(id)
function remember(records: readonly MemoryRecord[]): void {
  if (previews.size > 2000) previews.clear()
  for (const r of records) previews.set(r.id, r.subject ? `${r.subject}: ${r.text}` : r.text)
}

// ---------- Oturum katmanı ----------

export interface SessionMemory {
  notes: MemoryRecord[]
  /** Kullanıcıya bu sohbette zaten bildirilen çakışmalar (conflictKey) */
  surfaced: Set<string>
}

const sessionMemories = new WeakMap<object, SessionMemory>()

/** Anahtar: oturumun ShellRegistry'si; alt ajanlar üst oturumla aynı kaydı paylaşır, araçlar ctx.shells ile erişir. */
export function sessionMemoryOf(owner: object): SessionMemory {
  let memory = sessionMemories.get(owner)
  if (!memory) {
    memory = { notes: [], surfaced: new Set() }
    sessionMemories.set(owner, memory)
  }
  return memory
}

export function addSessionNote(memory: SessionMemory, note: string): MemoryRecord {
  const text = note.replace(/\s+/g, ' ').trim()
  assertNoSecrets(text)
  const existing = memory.notes.find((n) => fold(n.text) === fold(text))
  if (existing) return existing
  if (memory.notes.length >= MAX_SESSION_NOTES) throw new Error(`Oturum hafızası dolu (${MAX_SESSION_NOTES} not); gereksiz notları memory_forget ile sil.`)
  const record: MemoryRecord = { id: newId('s'), layer: 'session', text }
  memory.notes.push(record)
  remember([record])
  return record
}

// ---------- Proje katmanı (bilgi tabanı) ----------

async function requireTrusted(root: string): Promise<void> {
  if (!(await isFolderTrusted(root))) {
    throw new Error('Bu klasör güvenilir olarak işaretlenmedi; proje hafızası (bilgi tabanı) okunmaz ve yazılmaz.')
  }
}

const toMemory = (r: KnowledgeRecord): MemoryRecord => ({ id: r.id, layer: 'project', text: r.statement, kind: r.kind, subject: r.subject })

/** Depoyla gelebilen dosya: kök dışına bağlantı reddedilir, şemaya uymayan içerik hata verir (üzerine yazılmaz). */
async function readKnowledge(root: string): Promise<KnowledgeRecord[]> {
  adoptProject(root)
  let text: string
  try {
    text = await readFile(confineToWorkspace(root, KNOWLEDGE_FILE), 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw err
  }
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`${KNOWLEDGE_FILE.split(path.sep).join('/')} geçerli JSON değil; düzelt veya sil.`)
  }
  const parsed = knowledgeFileSchema.safeParse(json)
  if (!parsed.success) throw new Error(`${KNOWLEDGE_FILE.split(path.sep).join('/')} beklenen biçimde değil:\n${z.prettifyError(parsed.error)}`)
  return parsed.data.records
}

let writeQueue: Promise<unknown> = Promise.resolve()

/** Okuma-değiştirme-yazma tek sırada: eşzamanlı iki ekleme birbirini silmesin. Yazılan içerik "uygulama üretti" diye kaydedilir. */
function mutateKnowledge<T>(root: string, change: (records: KnowledgeRecord[]) => { records: KnowledgeRecord[]; result: T }): Promise<T> {
  const run = async () => {
    await requireTrusted(root)
    const current = await readKnowledge(root)
    const { records, result } = change(current)
    if (records === current) return result // Değişiklik yok: dosya yazılmaz (yoksa da oluşturulmaz).
    await ensureMapDir(root)
    const abs = confineToWorkspace(root, KNOWLEDGE_FILE)
    const content = `${JSON.stringify({ version: 1, records }, null, 2)}\n`
    const tmp = `${abs}.${process.pid}.tmp`
    await writeFile(tmp, content, 'utf8')
    await rename(tmp, abs)
    await recordGeneratedFile(abs, content)
    return result
  }
  const result = writeQueue.then(run, run)
  writeQueue = result.catch(() => undefined)
  return result
}

export async function loadProjectRecords(root: string): Promise<MemoryRecord[]> {
  await requireTrusted(root)
  const records = (await readKnowledge(root)).map(toMemory)
  remember(records)
  return records
}

export async function addKnowledge(root: string, input: KnowledgeInput): Promise<{ record: MemoryRecord; duplicate: boolean }> {
  assertNoSecrets(`${input.subject} ${input.statement}`)
  return mutateKnowledge<{ record: MemoryRecord; duplicate: boolean }>(root, (records) => {
    const same = records.find(
      (r) => r.kind === input.kind && fold(r.subject) === fold(input.subject) && fold(r.statement) === fold(input.statement),
    )
    if (same) return { records, result: { record: toMemory(same), duplicate: true } }
    if (records.length >= MAX_KNOWLEDGE_RECORDS) throw new Error(`Bilgi tabanı dolu (${MAX_KNOWLEDGE_RECORDS} kayıt); eski kayıtları sil.`)
    const created: KnowledgeRecord = { id: newId('k'), ...input, createdAt: Date.now() }
    remember([toMemory(created)])
    return { records: [...records, created], result: { record: toMemory(created), duplicate: false } }
  })
}

// ---------- Global katman ----------

/** memory.md satırları; kimlik satır içeriğinin özetidir (dosya elle düzenlense de kararlı). */
export async function loadGlobalRecords(file: string = MEMORY_FILE): Promise<MemoryRecord[]> {
  const text = (await loadMemory(file)) ?? ''
  const records = text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'))
    .map((line): MemoryRecord => ({ id: globalId(line), layer: 'global', text: line.replace(/^[-*]\s+/, '').replace(/^\[\d{4}-\d{2}-\d{2}\]\s*/, '') }))
  remember(records)
  return records
}

// ---------- Katmanlar arası ----------

export interface LayeredRecords {
  session: MemoryRecord[]
  /** null: klasör güvenilmez (yüklenmedi) */
  project: MemoryRecord[] | null
  global: MemoryRecord[]
  /** Proje dosyası okunamadıysa nedeni */
  projectError?: string
  /** Kullanıcının "ikisi de geçerli" dediği çiftler (conflictKey) */
  keptPairs: Set<string>
}

/** "İkisi de geçerli" cevabı kalıcıdır: aynı soru her yeni sohbette yeniden sorulmasın. Kimlikler (g: içerik özeti, k: kayıtlı) kararlıdır. */
export const KEPT_PAIRS_FILE = path.join(USER_CONFIG_DIR, 'memory-conflicts.json')
const MAX_KEPT_PAIRS = 500
const keptSchema = z.object({ version: z.literal(1), keepBoth: z.array(z.string().max(40)).max(MAX_KEPT_PAIRS) })

async function loadKeptPairs(file: string): Promise<string[]> {
  try {
    const parsed = keptSchema.safeParse(JSON.parse(await readFile(file, 'utf8')))
    return parsed.success ? parsed.data.keepBoth : []
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') console.warn('g11: çakışma kararları okunamadı:', err instanceof Error ? err.message : err)
    return []
  }
}

let keptQueue: Promise<unknown> = Promise.resolve()

export function keepBoth(a: string, b: string, file: string = KEPT_PAIRS_FILE): Promise<void> {
  if (!RECORD_ID.test(a) || !RECORD_ID.test(b) || a === b) return Promise.reject(new Error('Geçersiz kayıt kimliği'))
  const run = async () => {
    const key = conflictKey(a, b)
    const pairs = (await loadKeptPairs(file)).filter((p) => p !== key)
    pairs.push(key)
    await mkdir(path.dirname(file), { recursive: true })
    const tmp = `${file}.${process.pid}.tmp`
    await writeFile(tmp, `${JSON.stringify({ version: 1, keepBoth: pairs.slice(-MAX_KEPT_PAIRS) })}\n`, { encoding: 'utf8', mode: 0o600 })
    await rename(tmp, file)
  }
  const result = keptQueue.then(run, run)
  keptQueue = result.catch(() => undefined)
  return result
}

/** Bir katmanın hatası diğerlerini düşürmez; bozuk bilgi tabanı sohbeti durdurmamalı. */
export async function loadLayers(root: string | null, session: SessionMemory | null, globalFile?: string, keptFile: string = KEPT_PAIRS_FILE): Promise<LayeredRecords> {
  const out: LayeredRecords = {
    session: session ? [...session.notes] : [],
    project: null,
    global: await loadGlobalRecords(globalFile),
    keptPairs: new Set(await loadKeptPairs(keptFile)),
  }
  if (root && (await isFolderTrusted(root))) {
    try {
      out.project = await loadProjectRecords(root)
    } catch (err) {
      out.project = []
      out.projectError = err instanceof Error ? err.message : String(err)
      console.warn('g11: proje bilgi tabanı okunamadı:', out.projectError)
    }
  }
  return out
}

export const allRecords = (layers: LayeredRecords): MemoryRecord[] => [...layers.session, ...(layers.project ?? []), ...layers.global]

export function layerConflicts(layers: LayeredRecords): Conflict[] {
  return findConflicts(allRecords(layers), 100)
    .filter((c) => !layers.keptPairs.has(conflictKey(c.a, c.b)))
    .slice(0, 20)
}

/** Kaydı katmanından siler; silinen metni (yoksa null) döner. */
export async function forgetRecord(id: string, o: { root: string | null; session: SessionMemory | null; globalFile?: string }): Promise<string | null> {
  if (!RECORD_ID.test(id)) throw new Error('Geçersiz kayıt kimliği')
  if (id.startsWith('s')) {
    const index = o.session?.notes.findIndex((n) => n.id === id) ?? -1
    if (!o.session || index < 0) return null
    const [removed] = o.session.notes.splice(index, 1)
    return removed?.text ?? null
  }
  if (id.startsWith('k')) {
    if (!o.root) throw new Error('Açık bir proje klasörü yok')
    return mutateKnowledge(o.root, (records) => {
      const found = records.find((r) => r.id === id)
      return found ? { records: records.filter((r) => r.id !== id), result: `${found.subject}: ${found.statement}` } : { records, result: null }
    })
  }
  const file = o.globalFile ?? MEMORY_FILE
  const lines = ((await loadMemory(file)) ?? '').split('\n')
  const kept = lines.filter((l) => !l.trim() || l.trim().startsWith('#') || globalId(l.trim()) !== id)
  if (kept.length === lines.length) return null
  await saveMemory(kept.join('\n'), file)
  return lines.find((l) => l.trim() && globalId(l.trim()) === id)?.trim() ?? null
}

/** Arayüz özeti */
export interface MemoryOverview {
  session: MemoryRecord[]
  project: { root: string; trusted: boolean; records: MemoryRecord[]; error?: string } | null
  global: MemoryRecord[]
  conflicts: Conflict[]
  confidenceScore: boolean
}
