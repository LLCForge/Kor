import { randomUUID } from 'node:crypto'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { USER_CONFIG_DIR } from './settings.ts'

/** Kenar çubuğundaki kullanıcı tanımlı bölümler (~/.kor/sections.json). */
export const SECTIONS_FILE = path.join(USER_CONFIG_DIR, 'sections.json')
export const MAX_SECTIONS = 50
export const MAX_SECTION_NAME = 60

export const sectionNameSchema = z.string().trim().min(1, 'Bölüm adı boş olamaz').max(MAX_SECTION_NAME, `Bölüm adı en fazla ${MAX_SECTION_NAME} karakter olabilir`)

const sectionSchema = z.object({ id: z.string().uuid(), name: sectionNameSchema, createdAt: z.number() }).strict()
const fileSchema = z.object({ version: z.literal(1), sections: z.array(sectionSchema).max(MAX_SECTIONS) }).strict()

export interface Section {
  id: string
  name: string
  createdAt: number
}

/** Dosya yoksa boş liste; bozuksa hata (üzerine yazıp kullanıcının bölümlerini silmemek için). */
export async function loadSections(file: string = SECTIONS_FILE): Promise<Section[]> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw err
  }
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`${file}: geçersiz JSON`)
  }
  const parsed = fileSchema.safeParse(json)
  if (!parsed.success) throw new Error(`${file}:\n${z.prettifyError(parsed.error)}`)
  return parsed.data.sections
}

async function saveSections(sections: Section[], file: string): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true })
  const tmp = `${file}.${process.pid}.tmp`
  await writeFile(tmp, `${JSON.stringify({ version: 1, sections }, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 })
  await rename(tmp, file)
}

const queues = new Map<string, Promise<unknown>>()

/** Oku-değiştir-yaz adımları dosya başına sırayla çalışır; aynı anda iki pencereden ekleme kaybolmaz. */
function mutate<T>(file: string, change: (sections: Section[]) => { sections: Section[]; result: T }): Promise<T> {
  const run = (queues.get(file) ?? Promise.resolve()).then(async () => {
    const { sections, result } = change(await loadSections(file))
    await saveSections(sections, file)
    return result
  })
  // Hata `run` ile çağırana döner; kuyruk zinciri yalnızca sırayı korur.
  queues.set(file, run.then(() => undefined, () => undefined))
  return run
}

function findIndex(sections: Section[], id: string): number {
  const index = sections.findIndex((s) => s.id === id)
  if (index === -1) throw new Error('Bölüm bulunamadı')
  return index
}

function assertUniqueName(sections: Section[], name: string, exceptId?: string): void {
  const key = name.toLocaleLowerCase('tr')
  if (sections.some((s) => s.id !== exceptId && s.name.toLocaleLowerCase('tr') === key)) throw new Error(`"${name}" adında bir bölüm zaten var`)
}

/** Ad kullanıcıdan gelir; zod hata metni yerine okunur bir mesaj döner. */
function parseName(raw: string): string {
  const parsed = sectionNameSchema.safeParse(raw)
  if (!parsed.success) throw new Error(parsed.error.issues[0]?.message ?? 'Geçersiz bölüm adı')
  return parsed.data
}

export async function createSection(rawName: string, file: string = SECTIONS_FILE): Promise<Section> {
  const name = parseName(rawName)
  return mutate(file, (sections) => {
    if (sections.length >= MAX_SECTIONS) throw new Error(`En fazla ${MAX_SECTIONS} bölüm oluşturulabilir`)
    assertUniqueName(sections, name)
    const section: Section = { id: randomUUID(), name, createdAt: Date.now() }
    return { sections: [...sections, section], result: section }
  })
}

export async function renameSection(id: string, rawName: string, file: string = SECTIONS_FILE): Promise<void> {
  const name = parseName(rawName)
  return mutate(file, (sections) => {
    const index = findIndex(sections, id)
    assertUniqueName(sections, name, id)
    return { sections: sections.map((s, i) => (i === index ? { ...s, name } : s)), result: undefined }
  })
}

/** Sohbet dosyalarına dokunulmaz: silinmiş bölüme ait sohbetler listede proje gruplarına döner. */
export async function deleteSection(id: string, file: string = SECTIONS_FILE): Promise<void> {
  return mutate(file, (sections) => {
    findIndex(sections, id)
    return { sections: sections.filter((s) => s.id !== id), result: undefined }
  })
}
