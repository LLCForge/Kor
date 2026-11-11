// Uzak bağlantı deposu: SSH sunucuları ve veritabanları (~/.kor/connections.json, 0600).
// Sırlar (parola, anahtar parolası) dosyaya DÜZ METİN yazılmaz: ana süreçteki işletim sistemi kasasıyla (Electron safeStorage:
// Windows DPAPI, macOS Keychain, Linux libsecret) şifrelenir. Kasa yoksa (CLI) sır kaydedilemez; anahtar/agent kimlik
// doğrulaması ya da KOR_SECRET_<ID> ortam değişkeni kullanılır. Sırlar modele, günlüğe ve arayüze hiçbir zaman dönmez.
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { atomicWriteFile } from './atomic-write-g10.ts'
import { USER_CONFIG_DIR } from './settings.ts'

export const CONNECTIONS_FILE = path.join(USER_CONFIG_DIR, 'connections.json')

export const connectionId = z.string().regex(/^[a-z0-9][a-z0-9_-]{0,39}$/, 'Kimlik: küçük harf, rakam, - ve _ (en fazla 40)')
// Ana makine adı, IPv4 veya IPv6; boşluk ve kabuk karakteri yok (bağlantı dizesine/komuta enjekte edilemesin).
const hostName = z.string().min(1).max(253).regex(/^[A-Za-z0-9.:_-]+$/, 'Geçersiz sunucu adresi')
const label = z.string().trim().min(1).max(80)

export const sshHostSchema = z.object({
  id: connectionId,
  name: label,
  host: hostName,
  port: z.number().int().min(1).max(65535),
  username: z.string().min(1).max(64).regex(/^[A-Za-z0-9._@\\-]+$/, 'Geçersiz kullanıcı adı'),
  auth: z.enum(['password', 'key', 'agent']),
  /** Özel anahtar dosyası (~ ile başlayabilir); auth=key */
  keyPath: z.string().min(1).max(1024).optional(),
  /** Kasa ile şifrelenmiş parola (auth=password) veya anahtar parolası (auth=key) */
  secret: z.string().max(20_000).optional(),
  /** Kullanıcının onayladığı sunucu anahtarı parmak izi (OpenSSH biçimi). Yoksa bağlantı kurulmaz. */
  hostKey: z.string().regex(/^SHA256:[A-Za-z0-9+/]{20,64}$/).optional(),
})

export const dbConnectionSchema = z.object({
  id: connectionId,
  name: label,
  kind: z.enum(['postgres', 'mysql', 'sqlite']),
  host: hostName.optional(),
  port: z.number().int().min(1).max(65535).optional(),
  database: z.string().min(1).max(128).optional(),
  user: z.string().min(1).max(64).optional(),
  /** sqlite: çalışma dizinine göre dosya yolu */
  file: z.string().min(1).max(1024).optional(),
  ssl: z.boolean().optional(),
  /** Veritabanına bu SSH sunucusu üzerinden tünelle bağlanılır (yalnızca sunucuda açık veritabanları için). */
  sshHost: connectionId.optional(),
  secret: z.string().max(20_000).optional(),
})

export type SshHost = z.infer<typeof sshHostSchema>
export type DbConnection = z.infer<typeof dbConnectionSchema>

const fileSchema = z.object({
  version: z.literal(1),
  ssh: z.array(sshHostSchema).max(100),
  databases: z.array(dbConnectionSchema).max(100),
})
export type Connections = z.infer<typeof fileSchema>

export interface SecretCodec {
  encrypt(plain: string): string
  decrypt(cipher: string): string
}

let codec: SecretCodec | null = null

/** Masaüstü ana süreci uygulama hazır olunca safeStorage ile çağırır; CLI'da kasa yoktur. */
export function configureSecretCodec(next: SecretCodec | null): void {
  codec = next
}

export const canStoreSecrets = (): boolean => codec !== null

export function sealSecret(plain: string): string {
  if (!codec) throw new Error('Bu ortamda güvenli sır deposu yok; parola kaydedilemez (anahtar veya ssh-agent kullan ya da KOR_SECRET_<ID> ortam değişkenini ayarla).')
  return codec.encrypt(plain)
}

/** Sır: önce KOR_SECRET_<ID> ortam değişkeni (CLI/CI), sonra kasadaki şifreli değer. Bulunamazsa null. */
export function revealSecret(entry: { id: string; secret?: string | undefined }): string | null {
  const fromEnv = process.env[`KOR_SECRET_${entry.id.toUpperCase().replace(/-/g, '_')}`]
  if (fromEnv) return fromEnv
  if (!entry.secret || !codec) return null
  try {
    return codec.decrypt(entry.secret)
  } catch {
    return null // başka kullanıcı/makinede şifrelenmiş: yeniden girilmeli
  }
}

const EMPTY: Connections = { version: 1, ssh: [], databases: [] }

export async function loadConnections(file: string = CONNECTIONS_FILE): Promise<Connections> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return structuredClone(EMPTY)
    throw err
  }
  const parsed = fileSchema.safeParse(JSON.parse(text))
  if (!parsed.success) throw new Error(`${file}:\n${z.prettifyError(parsed.error)}`)
  return parsed.data
}

let writeQueue: Promise<unknown> = Promise.resolve()

/** Oku-değiştir-yaz tek kuyrukta: eşzamanlı iki kayıt birbirinin değişikliğini silmesin. */
export function mutateConnections(change: (c: Connections) => Connections, file: string = CONNECTIONS_FILE): Promise<Connections> {
  const run = async () => {
    const next = fileSchema.parse(change(await loadConnections(file)))
    const ids = [...next.ssh.map((h) => h.id), ...next.databases.map((d) => d.id)]
    if (new Set(ids).size !== ids.length) throw new Error('Aynı kimlik iki bağlantıda kullanılamaz.')
    for (const db of next.databases) {
      if (db.sshHost && !next.ssh.some((h) => h.id === db.sshHost)) throw new Error(`${db.id}: tünel için SSH sunucusu bulunamadı (${db.sshHost}).`)
    }
    await atomicWriteFile(file, `${JSON.stringify(next, null, 2)}\n`)
    return next
  }
  const result = writeQueue.then(run, run)
  writeQueue = result.catch(() => undefined)
  return result
}

/** Arayüze ve modele dönen biçim: şifreli sır yerine yalnızca "kayıtlı mı". */
export type PublicSshHost = Omit<SshHost, 'secret'> & { hasSecret: boolean }
export type PublicDbConnection = Omit<DbConnection, 'secret'> & { hasSecret: boolean }

export function publicSshHost({ secret, ...rest }: SshHost): PublicSshHost {
  return { ...rest, hasSecret: Boolean(secret) || revealSecret({ id: rest.id }) !== null }
}

export function publicDbConnection({ secret, ...rest }: DbConnection): PublicDbConnection {
  return { ...rest, hasSecret: Boolean(secret) || revealSecret({ id: rest.id }) !== null }
}

/**
 * Sistem istemi için kısa liste: yalnızca kimlik, ad ve tür (adres/kullanıcı adı sağlayıcıya gönderilmez; gerekirse
 * remote_connections aracıyla alınır). Bağlantı yoksa ya da dosya okunamazsa null.
 */
export async function connectionsPromptSection(file?: string): Promise<string | null> {
  let c: Connections
  try {
    c = await loadConnections(file)
  } catch {
    return null
  }
  if (c.ssh.length === 0 && c.databases.length === 0) return null
  const ssh = c.ssh.map((h) => `${h.id} (${h.name}${h.hostKey ? '' : ', key not yet trusted'})`)
  const dbs = c.databases.map((d) => `${d.id} (${d.name}, ${d.kind}${d.sshHost ? ` via ${d.sshHost}` : ''})`)
  return [
    '# REMOTE CONNECTIONS',
    'The user configured these. Refer to them by id; credentials are handled by the app and never shown to you.',
    ...(ssh.length > 0 ? [`SSH servers: ${ssh.join(', ')} — use ssh_exec / ssh_list / ssh_read_file / ssh_write_file / ssh_upload / ssh_download. Use sudo: true instead of typing sudo; pass non-interactive flags (-y).`] : []),
    ...(dbs.length > 0 ? [`Databases: ${dbs.join(', ')} — call db_schema before db_query (read-only).`] : []),
  ].join('\n')
}

export async function findSshHost(id: string, file?: string): Promise<SshHost> {
  const host = (await loadConnections(file)).ssh.find((h) => h.id === id)
  if (!host) throw new Error(`SSH sunucusu bulunamadı: ${id}. remote_connections ile tanımlı sunucuları listele; yenisini kullanıcı Ayarlar > Uzak bağlantılar'dan ekler.`)
  return host
}

export async function findDbConnection(id: string, file?: string): Promise<{ db: DbConnection; tunnel: SshHost | null }> {
  const all = await loadConnections(file)
  const db = all.databases.find((d) => d.id === id)
  if (!db) throw new Error(`Veritabanı bağlantısı bulunamadı: ${id}. remote_connections ile listele.`)
  return { db, tunnel: db.sshHost ? (all.ssh.find((h) => h.id === db.sshHost) ?? null) : null }
}
