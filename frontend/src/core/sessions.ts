import { mkdir, readdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import { z } from 'zod'
import { PERMISSION_MODES, type PermissionMode } from './permissions.ts'
import { USER_CONFIG_DIR } from './settings.ts'
import { tools } from './tools.ts'
import { isSummaryMessage, summaryBody, summaryTitle } from './context.ts' // [g6]
import { isHumanUserText, userContentText } from './message-text.ts' // [g2] görselli (çok parçalı) kullanıcı mesajları

export const SESSIONS_DIR = path.join(USER_CONFIG_DIR, 'sessions')
const MAX_TITLE_CHARS = 80

const idSchema = z.string().uuid()

const storedSchema = z.object({
  version: z.literal(1),
  id: idSchema,
  title: z.string().max(200),
  root: z.string().max(4096),
  model: z.string().max(200),
  createdAt: z.number(),
  updatedAt: z.number(),
  pinned: z.boolean(),

  // [g1] kayıt alanları (zod; geriye dönük uyum için .optional())
  customTitle: z.string().max(200).optional(),
  autoTitle: z.string().max(200).optional(),
  archived: z.boolean().optional(),
  sectionId: idSchema.optional(),
  permissionMode: z.enum(PERMISSION_MODES).optional(),
  usage: z
    .record(z.string().max(200), z.object({ promptTokens: z.number().int().nonnegative(), completionTokens: z.number().int().nonnegative() }))
    .optional(),
  recap: z.string().max(400).optional(),

  // [g2] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g3] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g4] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g5] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g6] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g9] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g10] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g11] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g12] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g13] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g14] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g15] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g16] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g18] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g19] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g20] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g21] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g22] kayıt alanları (zod; geriye dönük uyum için .optional())

  // [g26] oturum ağacı
  parentId: idSchema.optional(),

  // [cdx] Kenar çubuğu dosya istatistiği (cdesktop "📄 N +X −Y"): oturum başı → son hâl toplamı
  stats: z.object({ files: z.number().int().nonnegative(), added: z.number().int().nonnegative(), removed: z.number().int().nonnegative() }).optional(),

  // [g7] kayıt alanları (zod; geriye dönük uyum için .optional())
  // Mesajlar bu uygulamanın kendi yazdığı dosyadan gelir; rol alanı doğrulanır, gövde OpenAI şemasındadır.
  messages: z.array(z.object({ role: z.enum(['system', 'user', 'assistant', 'tool']) }).passthrough()),
})

export interface StoredSession {
  version: 1
  id: string
  title: string
  root: string
  model: string
  createdAt: number
  updatedAt: number
  pinned: boolean

  // [g1] StoredSession alanları (opsiyonel)
  /** Kullanıcının verdiği ad; otomatik başlık bunu ezmez */
  customTitle?: string
  /** Modelin ürettiği kısa başlık */
  autoTitle?: string
  archived?: boolean
  /** Kullanıcı tanımlı bölüm (sections.json); bölüm silinmişse yok sayılır */
  sectionId?: string
  permissionMode?: PermissionMode
  /** Model kimliğine göre toplam token; maliyet arayüzde güncel fiyatlarla hesaplanır */
  usage?: Record<string, { promptTokens: number; completionTokens: number }>
  /** Son asistan yanıtının kısa özeti (kenar çubuğunda ikinci satır) */
  recap?: string

  // [g2] StoredSession alanları (opsiyonel)

  // [g3] StoredSession alanları (opsiyonel)

  // [g4] StoredSession alanları (opsiyonel)

  // [g5] StoredSession alanları (opsiyonel)

  // [g6] StoredSession alanları (opsiyonel)

  // [g9] StoredSession alanları (opsiyonel)

  // [g10] StoredSession alanları (opsiyonel)

  // [g11] StoredSession alanları (opsiyonel)

  // [g12] StoredSession alanları (opsiyonel)

  // [g13] StoredSession alanları (opsiyonel)

  // [g14] StoredSession alanları (opsiyonel)

  // [g15] StoredSession alanları (opsiyonel)

  // [g16] StoredSession alanları (opsiyonel)

  // [g18] StoredSession alanları (opsiyonel)

  // [g19] StoredSession alanları (opsiyonel)

  // [g20] StoredSession alanları (opsiyonel)

  // [g21] StoredSession alanları (opsiyonel)

  // [g22] StoredSession alanları (opsiyonel)

  /** [g26] Bu kayıt hangi sohbetten kopyalandı. */
  parentId?: string
  /** [cdx] Değişen dosya sayısı ve toplam +/- satır (kenar çubuğu satırı) */
  stats?: { files: number; added: number; removed: number }

  // [g7] StoredSession alanları (opsiyonel)

  messages: ChatCompletionMessageParam[]
}

export type SessionMeta = Omit<StoredSession, 'messages' | 'version'>

/** id UUID olarak doğrulanır; dosya yolu dışarı kaçamaz (CWE-22). */
function sessionFile(id: string, dir: string): string {
  return path.join(dir, `${idSchema.parse(id)}.json`)
}

export async function saveSession(session: StoredSession, dir: string = SESSIONS_DIR): Promise<void> {
  // [g1] Aynı sohbete yazmalar sıraya alınır: ortak .tmp dosyası çakışmaz, eski anlık görüntü yenisinin üstüne yazılmaz.
  return serializedWrite(sessionFile(session.id, dir), () => writeSessionFile(session, dir))
}

/**
 * [perf] Görseller sohbet JSON'una base64 gömülmez: data URL'ler `images/<sessionId>/<sha256>.txt` dosyasına
 * alınır, mesajda `kor-img:<hash>` referansı kalır. Yüklerken geri çözülür; model ve arayüz hep data URL görür.
 * Böylece 5 görselli sohbet 20 MB'lık, her turda yeniden yazılan bir dosya olmaz.
 */
const IMG_REF = 'kor-img:'
const IMG_MIN_CHARS = 2_048
type Part = { type?: string; text?: string; image_url?: { url?: string } }

function imagesDir(sessionId: string, dir: string): string {
  return path.join(dir, 'images', idSchema.parse(sessionId))
}

export async function externalizeImages(session: StoredSession, dir: string): Promise<StoredSession> {
  let out: StoredSession | null = null
  for (let i = 0; i < session.messages.length; i++) {
    const m = session.messages[i]!
    if (!Array.isArray(m.content)) continue
    let parts: Part[] | null = null
    for (let j = 0; j < m.content.length; j++) {
      const p = m.content[j] as Part
      const url = p.type === 'image_url' ? p.image_url?.url : undefined
      if (!url || !url.startsWith('data:image/') || url.length < IMG_MIN_CHARS) continue
      const hash = createHash('sha256').update(url).digest('hex')
      const folder = imagesDir(session.id, dir)
      await mkdir(folder, { recursive: true })
      const file = path.join(folder, `${hash}.txt`)
      await writeFile(file, url, { encoding: 'utf8', mode: 0o600, flag: 'wx' }).catch((err: NodeJS.ErrnoException) => {
        if (err.code !== 'EEXIST') throw err
      })
      parts ??= [...(m.content as Part[])]
      parts[j] = { ...p, image_url: { ...p.image_url, url: `${IMG_REF}${hash}` } }
    }
    if (parts) {
      out ??= { ...session, messages: [...session.messages] }
      out.messages[i] = { ...m, content: parts } as ChatCompletionMessageParam
    }
  }
  return out ?? session
}

export async function hydrateImages(session: StoredSession, dir: string): Promise<StoredSession> {
  let out: StoredSession | null = null
  for (let i = 0; i < session.messages.length; i++) {
    const m = session.messages[i]!
    if (!Array.isArray(m.content)) continue
    let parts: Part[] | null = null
    for (let j = 0; j < m.content.length; j++) {
      const p = m.content[j] as Part
      const url = p.type === 'image_url' ? p.image_url?.url : undefined
      if (!url || !url.startsWith(IMG_REF)) continue
      const hash = url.slice(IMG_REF.length)
      if (!/^[0-9a-f]{64}$/.test(hash)) continue
      const data = await readFile(path.join(imagesDir(session.id, dir), `${hash}.txt`), 'utf8').catch(() => null)
      parts ??= [...(m.content as Part[])]
      // Dosya kayıpsa görsel yerine metin: model/arayüz bozuk bir data URL görmesin.
      parts[j] = data ? { ...p, image_url: { ...p.image_url, url: data } } : { type: 'text', text: '[görsel kayıp]' }
    }
    if (parts) {
      out ??= { ...session, messages: [...session.messages] }
      out.messages[i] = { ...m, content: parts } as ChatCompletionMessageParam
    }
  }
  return out ?? session
}

async function writeSessionFile(session: StoredSession, dir: string): Promise<void> {
  await mkdir(dir, { recursive: true })
  const file = sessionFile(session.id, dir)
  const slim = await externalizeImages(session, dir)
  // Önce geçici dosyaya yaz, sonra taşı: yazma sırasında çökme dosyayı bozmaz.
  const tmp = `${file}.${process.pid}.tmp`
  await writeFile(tmp, JSON.stringify(slim), { encoding: 'utf8', mode: 0o600 })
  try {
    await renameWithRetry(tmp, file)
  } catch (err) {
    await rm(tmp, { force: true }).catch(() => undefined)
    throw err
  }
}

const LOCK_CODES = new Set(['EPERM', 'EACCES', 'EBUSY'])

/** Windows'ta tarayıcı/dizinleyici hedefi kısa süre kilitleyebilir (atomic-write-g10 ile aynı neden); 0600 için kendi geçici dosyası kullanılır. */
export async function renameWithRetry(from: string, to: string, move: typeof rename = rename, attempts = 6): Promise<void> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await move(from, to)
    } catch (err) {
      if (attempt >= attempts || !LOCK_CODES.has((err as NodeJS.ErrnoException).code ?? '')) throw err
      await sleep(50 * 2 ** (attempt - 1))
    }
  }
}

/** persist imzası: her mesajın uzunluğu sırayla; özet düzenlemesi aynı uzunlukta olabileceğinden özetin içeriği de. Sistem istemi hariç. */
export function messagesFingerprint(messages: readonly ChatCompletionMessageParam[]): string {
  let hash = 0
  const mix = (n: number) => {
    hash = (Math.imul(hash, 31) + n) | 0
  }
  for (const m of messages.slice(1)) {
    const { content } = m
    if (typeof content === 'string') {
      mix(content.length)
      if (isSummaryMessage(m)) for (let i = 0; i < content.length; i++) mix(content.charCodeAt(i))
    } else if (Array.isArray(content)) {
      for (const part of content) mix(part.type === 'text' ? part.text.length : 1)
    }
    if (m.role === 'assistant') mix(m.tool_calls?.length ?? 0)
  }
  return `${messages.length}:${hash}`
}

const writeQueues = new Map<string, Promise<unknown>>()

/** Aynı dosyaya yapılan işlemleri çağrı sırasıyla çalıştırır; hata çağırana döner, kuyruk sürer. */
function serializedWrite<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const run = (writeQueues.get(key) ?? Promise.resolve()).then(fn, fn)
  const tail = run.then(
    () => undefined,
    () => undefined, // Hata zaten `run` üzerinden çağırana iletildi; kuyruk yalnızca sırayı tutar.
  )
  writeQueues.set(key, tail)
  void tail.then(() => {
    if (writeQueues.get(key) === tail) writeQueues.delete(key)
  })
  return run
}

/**
 * Yalnızca meta alanlarını değiştirir: dosya kuyruk içinde yeniden okunur, böylece aynı anda süren
 * tur kayıtlarının mesajları geri alınmaz. updatedAt bilerek değişmez (yeniden adlandırma listeyi karıştırmasın).
 */
export async function updateSessionMeta(
  id: string,
  patch: (current: StoredSession) => Partial<Omit<StoredSession, 'id' | 'version' | 'messages'>>,
  dir: string = SESSIONS_DIR,
): Promise<StoredSession> {
  return serializedWrite(sessionFile(id, dir), async () => {
    const current = await loadSession(id, dir)
    const next: StoredSession = { ...current, ...patch(current), id: current.id, version: 1, messages: current.messages }
    await writeSessionFile(next, dir)
    return next
  })
}

/** Görünen başlık: kullanıcı adı > otomatik başlık > ilk mesaj. */
export function displayTitle(s: Pick<StoredSession, 'customTitle' | 'autoTitle' | 'messages'>): string {
  return s.customTitle || s.autoTitle || titleFrom(s.messages)
}

/**
 * Kayıtlı 'bypass' modu geri yüklenmez: uygulama açılışında (son sohbetin geri yüklenmesi dahil)
 * kullanıcı farkında olmadan onaysız moda girmesin. Diğer modlar aynen döner.
 */
export function restorablePermissionMode(mode: PermissionMode | undefined): PermissionMode | undefined {
  return mode === 'bypass' ? undefined : mode
}

export const RECAP_CHARS = 120

/** Son asistan yanıtının ilk ~120 karakteri; Markdown işaretleri ve boşluklar sadeleştirilir. */
export function recapFrom(messages: readonly ChatCompletionMessageParam[]): string | undefined {
  const last = messages.findLast((m) => m.role === 'assistant' && typeof m.content === 'string' && m.content.trim())
  if (!last || typeof last.content !== 'string') return undefined
  const plain = last.content
    .replace(/<think>[\s\S]*?<\/think>/g, ' ')
    .replace(/```[\s\S]*?(```|$)/g, ' ')
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    // İşaretler yalnızca biçimlendirme oldukları yerde kalkar: `a * b` ve snake_case bozulmaz.
    .replace(/`([^`\n]*)`/g, '$1')
    .replace(/(\*{1,3}|~~)(?=\S)(.+?)(?<=\S)\1/g, '$2')
    .replace(/(?<!\w)(_{1,3})(?=\S)(.+?)(?<=\S)\1(?!\w)/g, '$2')
    .replace(/^\s{0,3}(#{1,6}|>|[-*+])\s+/gm, '')
    .replace(/\|/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
  if (!plain) return undefined
  return plain.length > RECAP_CHARS ? `${plain.slice(0, RECAP_CHARS - 1).trimEnd()}…` : plain
}

export async function loadSession(id: string, dir: string = SESSIONS_DIR): Promise<StoredSession> {
  const parsed = storedSchema.parse(JSON.parse(await readFile(sessionFile(id, dir), 'utf8')))
  return hydrateImages({ ...parsed, messages: parsed.messages as ChatCompletionMessageParam[] }, dir)
}

/**
 * Oturum listesi meta önbelleği: her dosya için (mtime, boyut) → meta. Yalnızca değişen/yeni dosyalar okunup
 * parse edilir; diğerleri stat ile doğrulanır. Önbellek bellekte tutulur ve `sessions.index` dosyasına yazılır,
 * böylece uygulama yeniden açılınca da ilk liste hızlıdır. Dosya yoksa/bozuksa sessizce baştan kurulur.
 */
const INDEX_FILE = 'sessions.index'
interface IndexEntry { mtimeMs: number; size: number; meta: SessionMeta }
const indexCache = new Map<string, Map<string, IndexEntry>>()
const indexSchema = z.record(z.string(), z.object({ mtimeMs: z.number(), size: z.number(), meta: z.record(z.string(), z.unknown()) }))

async function loadIndex(dir: string): Promise<Map<string, IndexEntry>> {
  const cached = indexCache.get(dir)
  if (cached) return cached
  const map = new Map<string, IndexEntry>()
  try {
    const parsed = indexSchema.safeParse(JSON.parse(await readFile(path.join(dir, INDEX_FILE), 'utf8')))
    if (parsed.success) for (const [name, e] of Object.entries(parsed.data)) map.set(name, { mtimeMs: e.mtimeMs, size: e.size, meta: e.meta as unknown as SessionMeta })
  } catch {
    /* indeks yok ya da bozuk: baştan kurulur */
  }
  indexCache.set(dir, map)
  return map
}

async function readMeta(file: string): Promise<SessionMeta | null> {
  try {
    const { messages: _m, version: _v, ...meta } = storedSchema.parse(JSON.parse(await readFile(file, 'utf8')))
    return meta
  } catch {
    return null // Bozuk dosya listeyi düşürmez.
  }
}

/** Testler/araçlar için: bellek önbelleğini boşaltır (indeks dosyası bir sonraki listede yeniden okunur). */
export function resetSessionIndex(): void {
  indexCache.clear()
}

export async function listSessions(dir: string = SESSIONS_DIR): Promise<SessionMeta[]> {
  let names: string[]
  try {
    names = await readdir(dir)
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw err
  }
  const index = await loadIndex(dir)
  let dirty = false
  const seen = new Set<string>()
  const metas = await Promise.all(
    names
      .filter((n) => n.endsWith('.json') && !n.endsWith('.tmp'))
      .map(async (n): Promise<SessionMeta | null> => {
        seen.add(n)
        const file = path.join(dir, n)
        let st
        try {
          st = await stat(file)
        } catch {
          return null
        }
        const hit = index.get(n)
        if (hit && hit.mtimeMs === st.mtimeMs && hit.size === st.size) return hit.meta
        const meta = await readMeta(file)
        dirty = true
        if (meta) index.set(n, { mtimeMs: st.mtimeMs, size: st.size, meta })
        else index.delete(n)
        return meta
      }),
  )
  for (const n of [...index.keys()]) if (!seen.has(n)) { index.delete(n); dirty = true }
  if (dirty) {
    // İndeks yazımı en iyi çaba: başarısızlık listeyi etkilemez (bir sonraki listede yeniden denenir).
    const obj: Record<string, IndexEntry> = {}
    for (const [k, v] of index) obj[k] = v
    void serializedWrite(path.join(dir, INDEX_FILE), async () => {
      const tmp = path.join(dir, `${INDEX_FILE}.${process.pid}.tmp`)
      await writeFile(tmp, JSON.stringify(obj), { encoding: 'utf8', mode: 0o600 })
      await rename(tmp, path.join(dir, INDEX_FILE))
    }).catch(() => undefined)
  }
  return metas.filter((m): m is SessionMeta => m !== null).sort((a, b) => b.updatedAt - a.updatedAt)
}

export async function deleteSession(id: string, dir: string = SESSIONS_DIR): Promise<void> {
  // Yazma kuyruğunun arkasından: süren bir kayıt, silinen dosyayı geri getirmesin.
  await serializedWrite(sessionFile(id, dir), async () => {
    await rm(sessionFile(id, dir), { force: true })
    await rm(imagesDir(id, dir), { recursive: true, force: true })
  })
}

export async function setSessionPinned(id: string, pinned: boolean, dir: string = SESSIONS_DIR): Promise<void> {
  await updateSessionMeta(id, () => ({ pinned }), dir)
}

const FILE_BLOCKS = /^(?:<(file|tree) path="([^"]*)">[\s\S]*?<\/\1>\n\n)+/

/** Kullanıcı mesajının başına eklenen <file> ve <tree> (klasör eki) bloklarını ayırır. */
export function splitAttachments(content: string): { text: string; attachments: string[] } {
  const match = FILE_BLOCKS.exec(content)
  if (!match) return { text: content, attachments: [] }
  const attachments = [...match[0].matchAll(/<(?:file|tree) path="([^"]*)">/g)].map((m) => m[1] ?? '')
  return { text: content.slice(match[0].length), attachments }
}

export function titleFrom(messages: readonly ChatCompletionMessageParam[]): string {
  const first = messages.find((m) => m.role === 'user')
  // [g6] Sıkıştırılmış sohbetin ilk kullanıcı mesajı özettir; başlık özetin içinde saklanır.
  const kept = isSummaryMessage(first) && typeof first?.content === 'string' ? summaryTitle(first.content) : null
  if (kept) return kept
  // [g2] görselli (çok parçalı) mesajların metin kısmı
  const raw = first ? splitAttachments(userContentText(first.content) ?? '').text : ''
  const line = raw.replace(/\s+/g, ' ').trim()
  return line.length > MAX_TITLE_CHARS ? `${line.slice(0, MAX_TITLE_CHARS - 1)}…` : line || 'Yeni sohbet'
}

export type TranscriptItem =
  | { kind: 'user'; text: string; attachments: string[] }
  | { kind: 'assistant'; text: string }
  /** Arka plan bildirimi (<task-notification>): kullanıcı yazmadı; arayüz başlığını satır olarak gösterir */
  | { kind: 'notice'; text: string }
  | { kind: 'tool'; id: string; name: string; summary: string; result: string }
  // [g6] sıkıştırılmış önceki kısım
  | { kind: 'compaction'; summary: string }

/** Kaydedilmiş API mesajlarından arayüzde gösterilecek akışı yeniden kurar. */
export function transcriptFromMessages(messages: readonly ChatCompletionMessageParam[]): TranscriptItem[] {
  const results = new Map<string, string>()
  for (const m of messages) {
    if (m.role === 'tool' && typeof m.content === 'string') results.set(m.tool_call_id, m.content)
  }
  const items: TranscriptItem[] = []
  for (const m of messages) {
    if (m.role === 'user' && typeof m.content === 'string' && isSummaryMessage(m)) {
      items.push({ kind: 'compaction', summary: summaryBody(m.content) }) // [g6]
    } else if (m.role === 'user' && typeof m.content === 'string' && m.content.startsWith('<system-reminder>')) {
      continue // uygulamanın modele verdiği iç not; kullanıcı yazmadı
    } else if (m.role === 'user' && userContentText(m.content) !== null) {
      const text = userContentText(m.content) ?? ''
      // [g2] görselli (çok parçalı) kullanıcı mesajları da dökümde görünür
      items.push(isHumanUserText(text) ? { kind: 'user', ...splitAttachments(text) } : { kind: 'notice', text })
    } else if (m.role === 'assistant') {
      if (typeof m.content === 'string' && m.content.trim()) items.push({ kind: 'assistant', text: m.content })
      for (const call of m.tool_calls ?? []) {
        if (call.type !== 'function') continue
        let args: unknown = {}
        try {
          args = JSON.parse(call.function.arguments || '{}')
        } catch {
          // Bozuk argüman: özet ham metin olur.
        }
        const tool = tools.find((t) => t.name === call.function.name)
        items.push({
          kind: 'tool',
          id: call.id,
          name: call.function.name,
          summary: tool ? tool.summarize(args) : call.function.arguments.slice(0, 120),
          result: results.get(call.id) ?? '',
        })
      }
    }
  }
  return items
}
