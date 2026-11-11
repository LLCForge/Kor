// Klasör güveni: depoyla gelen talimatlar (AGENTS.md, .kor/) istem enjeksiyonu taşıyabilir.
// Güvenilmeyen klasörde bunlar ajana verilmez ve izin modu Manuel'e sabitlenir.
import { createHash } from 'node:crypto'
import { mkdir, readFile, rename, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import type { PermissionConfig, PermissionMode } from './permissions.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { adoptProject, LEGACY_PROJECT_DOT, PROJECT_DOT } from './layout-g28.ts' // [g28]

export const TRUST_FILE = path.join(USER_CONFIG_DIR, 'trusted.json')

/**
 * Varlığı güven sorusunu tetikleyen proje dosyaları (köke göre): talimat veya ayar taşıyanlar.
 * Yalnızca .kor/ klasörünün varlığı yetmez; o klasörü uygulamanın kendisi de (proje haritası) oluşturur.
 */
export const PROJECT_TRUST_FILES = [
  'AGENTS.md',
  path.join('.kor', 'agents'),
  path.join('.kor', 'PROJECT.md'),
  path.join('.kor', 'settings.json'),
  path.join('.kor', 'settings.local.json'),
  path.join('.kor', 'system-prompt.md'), // [g13] proje sistem istemi katmanı (prompt-g13.ts PROJECT_PROMPT_FILE)
  path.join('.kor', 'knowledge.json'), // [g11] proje hafızası / bilgi tabanı
  'CLAUDE.md', // [g24] kökte de güven sorusu
  '.goosehints', // [g24]
  // Çalıştırılabilir proje yapılandırması: bunlar güvenilen klasörde yüklenir, repo sorusuz güven kazanmasın.
  path.join('.kor', 'hooks.json'),
  path.join('.kor', 'tools'),
  path.join('.kor', 'plugins'),
  path.join('.kor', 'skills'),
  path.join('.kor', 'commands'),
  path.join('.kor', 'teams.json'),
  path.join('.claude', 'skills'),
  path.join('.claude', 'commands'),
  path.join('.agents', 'skills'),
  path.join('.cursor', 'commands'),
] as const

/** Eski .venice karşılıkları: .kor kopyası da varsa listelenmez (soru zaten onunla tetiklenir). */
const LEGACY_TRUST_FILES = PROJECT_TRUST_FILES.flatMap((rel) =>
  rel.startsWith(`${PROJECT_DOT}${path.sep}`) ? [{ rel: path.join(LEGACY_PROJECT_DOT, rel.slice(PROJECT_DOT.length + 1)), twin: rel }] : [],
)

/**
 * Uygulamanın kendi ürettiği dosyalar (proje haritası, /init ile AGENTS.md) içerik özetiyle kullanıcı klasöründe kaydedilir.
 * İçeriği kayıtla birebir aynı kalan dosya güven sorusunu tetiklemez; depo bu kaydı taklit edemez,
 * dosya elle veya depoyla değişirse özet tutmaz ve soru yeniden sorulur.
 */
export const GENERATED_FILE = path.join(USER_CONFIG_DIR, 'generated.json')
const MAX_HASHED_BYTES = 2_000_000

const generatedSchema = z.object({
  version: z.literal(1),
  files: z.record(z.string().max(4096), z.string().regex(/^[a-f0-9]{64}$/)),
})
type GeneratedStore = z.infer<typeof generatedSchema>

const sha256 = (text: string) => createHash('sha256').update(text, 'utf8').digest('hex')

async function loadGenerated(file: string): Promise<GeneratedStore> {
  try {
    const parsed = generatedSchema.safeParse(JSON.parse(await readFile(file, 'utf8')))
    // Bozuk kayıt: hiçbir dosya muaf sayılmaz (kapalı başarısızlık).
    return parsed.success ? parsed.data : { version: 1, files: {} }
  } catch {
    return { version: 1, files: {} }
  }
}

let generatedQueue: Promise<unknown> = Promise.resolve()

/** Uygulama bir proje dosyasını yazdıktan sonra çağırır. */
export async function recordGeneratedFile(absPath: string, content: string, file: string = GENERATED_FILE): Promise<void> {
  const run = async () => {
    const store = await loadGenerated(file)
    store.files[trustKey(absPath)] = sha256(content)
    await mkdir(path.dirname(file), { recursive: true })
    const tmp = `${file}.${process.pid}.tmp`
    await writeFile(tmp, `${JSON.stringify(store, null, 2)}
`, { encoding: 'utf8', mode: 0o600 })
    await rename(tmp, file)
  }
  const result = generatedQueue.then(run, run)
  generatedQueue = result.catch(() => undefined)
  return result
}

async function isUnchangedGenerated(absPath: string, store: GeneratedStore): Promise<boolean> {
  const expected = store.files[trustKey(absPath)]
  if (!expected) return false
  try {
    const info = await stat(absPath)
    if (!info.isFile() || info.size > MAX_HASHED_BYTES) return false
    return sha256(await readFile(absPath, 'utf8')) === expected
  } catch {
    return false
  }
}

const storeSchema = z.object({
  version: z.literal(1),
  folders: z.record(z.string().max(4096), z.object({ trusted: z.boolean(), decidedAt: z.number() })),
})
type TrustStore = z.infer<typeof storeSchema>

/** Windows'ta yol büyük/küçük harf duyarsızdır; aynı klasör tek anahtara iner. */
export function trustKey(root: string): string {
  const abs = path.resolve(root)
  return process.platform === 'win32' ? abs.toLowerCase() : abs
}

async function loadStore(file: string): Promise<TrustStore> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return { version: 1, folders: {} }
    throw err
  }
  let json: unknown = null
  try {
    json = JSON.parse(text)
  } catch {
    // Geçersiz JSON aşağıda şema hatası olarak ele alınır.
  }
  const parsed = storeSchema.safeParse(json)
  // Bozuk dosya güvensiz varsayılır (kapalı başarısızlık); bir sonraki kararda yeniden yazılır.
  if (!parsed.success) {
    console.warn(`${file}: güven dosyası okunamadı, tüm klasörler kararsız sayılıyor`)
    return { version: 1, folders: {} }
  }
  return parsed.data
}

/** null: kullanıcı bu klasör için henüz karar vermedi. */
export async function getFolderTrust(root: string, file: string = TRUST_FILE): Promise<boolean | null> {
  const store = await loadStore(file)
  const key = trustKey(root)
  return Object.hasOwn(store.folders, key) ? (store.folders[key]?.trusted ?? null) : null
}

let writeQueue: Promise<unknown> = Promise.resolve()

export async function setFolderTrust(root: string, trusted: boolean, file: string = TRUST_FILE): Promise<void> {
  const run = async () => {
    const store = await loadStore(file)
    store.folders[trustKey(root)] = { trusted, decidedAt: Date.now() }
    await mkdir(path.dirname(file), { recursive: true })
    const tmp = `${file}.${process.pid}.tmp`
    await writeFile(tmp, `${JSON.stringify(store, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 })
    await rename(tmp, file)
  }
  // Eşzamanlı iki karar birbirinin yazdığını silmesin.
  const result = writeQueue.then(run, run)
  writeQueue = result.catch(() => undefined)
  return result
}

/** Kökte bulunan proje dosyaları (görüntülemek için '/' ayraçlı). */
export async function projectTrustFiles(root: string, generatedFile: string = GENERATED_FILE): Promise<string[]> {
  adoptProject(root)
  const generated = await loadGenerated(generatedFile)
  const found: string[] = []
  for (const rel of PROJECT_TRUST_FILES) {
    const abs = path.join(root, rel)
    try {
      await stat(abs)
    } catch {
      continue // Dosya yok: güven sorusunu tetiklemez.
    }
    if (await isUnchangedGenerated(abs, generated)) continue // Bizim ürettiğimiz ve değişmemiş.
    found.push(rel.split(path.sep).join('/'))
  }
  for (const { rel, twin } of LEGACY_TRUST_FILES) {
    if (found.includes(twin.split(path.sep).join('/'))) continue
    const exists = await stat(path.join(root, rel)).then(() => true, () => false)
    if (exists) found.push(rel.split(path.sep).join('/'))
  }
  return found
}

/** Proje dosyası olmayan klasörde soru sorulmaz ve güvenilir sayılır; açık "güvenme" kararı her zaman geçerlidir. */
export async function isFolderTrusted(root: string, file: string = TRUST_FILE, generatedFile: string = GENERATED_FILE): Promise<boolean> {
  const decision = await getFolderTrust(root, file)
  if (decision !== null) return decision
  return (await projectTrustFiles(root, generatedFile)).length === 0
}

/**
 * İzin yapılandırmasını sarar: kurallar ve plan modu ortak nesneden canlı okunur,
 * ama Otomatik/Düzenlemeleri kabul et/İzinleri atla modları Manuel'e iner.
 */
export function manualPermissions(base: PermissionConfig): PermissionConfig {
  return {
    get mode(): PermissionMode {
      return base.mode === 'plan' ? 'plan' : 'default'
    },
    set mode(mode: PermissionMode) {
      base.mode = mode
    },
    get allow() {
      return base.allow
    },
    set allow(rules: string[]) {
      base.allow = rules
    },
    get deny() {
      return base.deny
    },
    set deny(rules: string[]) {
      base.deny = rules
    },
  }
}
