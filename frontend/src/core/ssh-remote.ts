// SSH uzak oturum: kullanıcının tanımladığı sunucuda komut çalıştırma, SFTP ile dosya listeleme/okuma/yazma/aktarma ve
// veritabanı tüneli. Bağlantılar sunucu başına havuzlanır (her komutta yeniden el sıkışılmaz), 10 dk boşta kalınca kapanır.
// Güvenlik:
//   - Sunucu anahtarı sabitlenir (TOFU): kullanıcı Ayarlar'da parmak izini onaylamadan hiçbir bağlantı kurulmaz; anahtar
//     değişirse (olası ortadaki adam) bağlantı reddedilir.
//   - Parola yalnızca bağlantı anında kasadan çözülür; sudo için stdin'e yazılır, komut satırına/çıktıya/günlüğe girmez.
//   - Çıktı, okuma ve aktarım boyutları sınırlıdır; zaman aşımı ve iptal uzak süreci de öldürür.
import { createHash } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import type { Duplex } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import ssh2, { type Client, type ConnectConfig, type SFTPWrapper } from 'ssh2'
import { revealSecret, type SshHost } from './connections.ts'

// ssh2 CommonJS'tir; Node ESM'de adlandırılmış dışa aktarımları görünmez, varsayılan dışa aktarım üzerinden alınır.
const { Client: SshClient } = ssh2

const READY_TIMEOUT_MS = 15_000
const IDLE_CLOSE_MS = 10 * 60_000
export const EXEC_TIMEOUT_MS = 120_000
export const MAX_EXEC_OUTPUT = 1_000_000
export const MAX_READ_BYTES = 2_000_000
export const MAX_TRANSFER_BYTES = 500_000_000
const MAX_LIST_ENTRIES = 2000

/** Sunucu anahtarı henüz onaylanmamış ya da onaylanandan farklı. */
export class HostKeyUntrustedError extends Error {
  override name = 'HostKeyUntrustedError'
  readonly fingerprint: string
  readonly mismatch: boolean
  constructor(fingerprint: string, mismatch: boolean) {
    super(
      mismatch
        ? `GÜVENLİK: sunucu anahtarı değişmiş (görülen ${fingerprint}). Ortadaki adam saldırısı olabilir; bağlantı reddedildi. Sunucu gerçekten yeniden kurulduysa kullanıcı Ayarlar > Uzak bağlantılar'dan yeni anahtarı onaylamalı.`
        : `Sunucu anahtarı henüz onaylanmadı (${fingerprint}). Kullanıcı Ayarlar > Uzak bağlantılar'da "Bağlantıyı test et" ile parmak izini doğrulayıp onaylamalı.`,
    )
    this.fingerprint = fingerprint
    this.mismatch = mismatch
  }
}

/** OpenSSH biçiminde parmak izi: "SHA256:" + base64 (dolgusuz). */
export function fingerprintOf(key: Buffer): string {
  return `SHA256:${createHash('sha256').update(key).digest('base64').replace(/=+$/, '')}`
}

/** POSIX kabuğu için tek tırnaklı güvenli argüman. */
export function shQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`
}

const expandHome = (p: string) => (p === '~' || p.startsWith('~/') || p.startsWith('~\\') ? path.join(os.homedir(), p.slice(1)) : p)

const DEFAULT_AGENT = process.platform === 'win32' ? '\\\\.\\pipe\\openssh-ssh-agent' : process.env.SSH_AUTH_SOCK

export async function connectConfig(host: SshHost): Promise<ConnectConfig> {
  const base: ConnectConfig = { host: host.host, port: host.port, username: host.username, readyTimeout: READY_TIMEOUT_MS, keepaliveInterval: 15_000 }
  switch (host.auth) {
    case 'password': {
      const password = revealSecret(host)
      if (!password) throw new Error(`${host.id}: kayıtlı parola yok. Ayarlar > Uzak bağlantılar'dan parolayı gir.`)
      return { ...base, password }
    }
    case 'key': {
      if (!host.keyPath) throw new Error(`${host.id}: özel anahtar dosyası tanımlı değil.`)
      const passphrase = revealSecret(host)
      return { ...base, privateKey: await readFile(expandHome(host.keyPath)), ...(passphrase ? { passphrase } : {}) }
    }
    case 'agent':
      if (!DEFAULT_AGENT) throw new Error('ssh-agent bulunamadı (SSH_AUTH_SOCK yok).')
      return { ...base, agent: DEFAULT_AGENT }
  }
}

interface Pooled {
  ready: Promise<Client>
  timer: NodeJS.Timeout | null
  /** Onaylı anahtarla kuruldu mu: anahtar ayarı değişirse havuzdaki bağlantı kullanılmaz */
  hostKey: string | undefined
}

const pool = new Map<string, Pooled>()

/**
 * Bağlantı testi: el sıkışma + kimlik doğrulama; havuza koymaz. Anahtar onaysızsa HostKeyUntrustedError (görülen parmak
 * iziyle) fırlatır — arayüz bunu kullanıcıya gösterip onay ister.
 */
export async function probeHost(host: SshHost, override?: Partial<ConnectConfig>): Promise<void> {
  const client = await openClient(host, override)
  client.end()
}

function openClient(host: SshHost, override?: Partial<ConnectConfig>): Promise<Client> {
  return connectConfig(host).then(
    (config) =>
      new Promise<Client>((resolve, reject) => {
        const client = new SshClient()
        let observed: string | null = null
        client.once('ready', () => resolve(client))
        client.once('error', (err) => {
          client.end()
          if (observed && observed !== host.hostKey) reject(new HostKeyUntrustedError(observed, host.hostKey !== undefined))
          else reject(err)
        })
        client.connect({
          ...config,
          ...override,
          hostVerifier: (key: Buffer) => {
            observed = fingerprintOf(key)
            return host.hostKey !== undefined && observed === host.hostKey
          },
        })
      }),
  )
}

function release(id: string): void {
  const entry = pool.get(id)
  if (!entry) return
  if (entry.timer) clearTimeout(entry.timer)
  entry.timer = setTimeout(() => closeHost(id), IDLE_CLOSE_MS)
  entry.timer.unref()
}

async function acquire(host: SshHost, override?: Partial<ConnectConfig>): Promise<Client> {
  const existing = pool.get(host.id)
  if (existing && existing.hostKey === host.hostKey) {
    if (existing.timer) clearTimeout(existing.timer)
    existing.timer = null
    return existing.ready
  }
  if (existing) closeHost(host.id)
  const ready = openClient(host, override)
  const entry: Pooled = { ready, timer: null, hostKey: host.hostKey }
  pool.set(host.id, entry)
  try {
    const client = await ready
    client.once('close', () => {
      if (pool.get(host.id) === entry) pool.delete(host.id)
    })
    return client
  } catch (err) {
    if (pool.get(host.id) === entry) pool.delete(host.id)
    throw err
  }
}

export function closeHost(id: string): void {
  const entry = pool.get(id)
  if (!entry) return
  pool.delete(id)
  if (entry.timer) clearTimeout(entry.timer)
  entry.ready.then((c) => c.end()).catch(() => undefined)
}

export function closeAllHosts(): void {
  for (const id of [...pool.keys()]) closeHost(id)
}

async function withClient<T>(host: SshHost, fn: (client: Client) => Promise<T>, override?: Partial<ConnectConfig>): Promise<T> {
  const client = await acquire(host, override)
  try {
    return await fn(client)
  } finally {
    release(host.id)
  }
}

export interface ExecResult {
  code: number | null
  signal: string | null
  stdout: string
  stderr: string
  truncated: boolean
  timedOut: boolean
}

export interface ExecOptions {
  timeoutMs?: number
  signal?: AbortSignal
  /** true: komut sudo ile çalışır; parola (varsa) stdin'den verilir */
  sudo?: boolean
  /** Bağlantı ayarı (testlerde yerel sunucu için) */
  override?: Partial<ConnectConfig>
}

/** sudo'lu komut: parola istemi gizli (-p ''), parola stdin'den (-S); komut tek argüman olarak sh -c'ye gider. */
export function sudoCommand(command: string): string {
  return `sudo -S -p '' -- sh -c ${shQuote(command)}`
}

export function sshExec(host: SshHost, command: string, o: ExecOptions = {}): Promise<ExecResult> {
  const timeoutMs = o.timeoutMs ?? EXEC_TIMEOUT_MS
  const password = o.sudo && host.auth === 'password' ? revealSecret(host) : null
  return withClient(
    host,
    (client) =>
      new Promise<ExecResult>((resolve, reject) => {
        client.exec(o.sudo ? sudoCommand(command) : command, (err, stream) => {
          if (err) return reject(err)
          let stdout = ''
          let stderr = ''
          let truncated = false
          let timedOut = false
          const take = (current: string, chunk: Buffer): string => {
            if (current.length >= MAX_EXEC_OUTPUT) {
              truncated = true
              return current
            }
            const next = current + chunk.toString('utf8')
            if (next.length > MAX_EXEC_OUTPUT) truncated = true
            return next.slice(0, MAX_EXEC_OUTPUT)
          }
          const kill = () => {
            try {
              stream.signal('KILL')
            } catch {
              // sunucu sinyal desteklemiyor olabilir; kanal kapatılır
            }
            stream.close()
          }
          const timer = setTimeout(() => {
            timedOut = true
            kill()
          }, timeoutMs)
          const onAbort = () => kill()
          o.signal?.addEventListener('abort', onAbort, { once: true })
          stream.on('data', (chunk: Buffer) => (stdout = take(stdout, chunk)))
          stream.stderr.on('data', (chunk: Buffer) => (stderr = take(stderr, chunk)))
          stream.on('close', (code: number | null, signal: string | null) => {
            clearTimeout(timer)
            o.signal?.removeEventListener('abort', onAbort)
            resolve({ code: code ?? null, signal: signal ?? null, stdout, stderr, truncated, timedOut })
          })
          if (o.sudo && password) stream.write(`${password}\n`)
          stream.end()
        })
      }),
    o.override,
  )
}

function sftpOf(client: Client): Promise<SFTPWrapper> {
  return new Promise((resolve, reject) => client.sftp((err, sftp) => (err ? reject(err) : resolve(sftp))))
}

async function withSftp<T>(host: SshHost, fn: (sftp: SFTPWrapper) => Promise<T>, override?: Partial<ConnectConfig>): Promise<T> {
  return withClient(
    host,
    async (client) => {
      const sftp = await sftpOf(client)
      try {
        return await fn(sftp)
      } finally {
        sftp.end()
      }
    },
    override,
  )
}

const promisify =
  <A extends unknown[], R>(call: (...args: [...A, (err: Error | null | undefined, result: R) => void]) => void) =>
  (...args: A): Promise<R> =>
    new Promise((resolve, reject) => call(...args, (err, result) => (err ? reject(err) : resolve(result))))

export interface RemoteEntry {
  name: string
  type: 'dir' | 'file' | 'link' | 'other'
  size: number
  modified: string
}

export function sftpList(host: SshHost, dir: string, override?: Partial<ConnectConfig>): Promise<{ entries: RemoteEntry[]; truncated: boolean }> {
  return withSftp(
    host,
    async (sftp) => {
      const list = await promisify<[string], Array<{ filename: string; attrs: { mode: number; size: number; mtime: number } }>>((p, cb) => sftp.readdir(p, cb))(dir)
      const entries = list
        .map(({ filename, attrs }) => {
          const kind = attrs.mode & 0o170000
          const type: RemoteEntry['type'] = kind === 0o040000 ? 'dir' : kind === 0o100000 ? 'file' : kind === 0o120000 ? 'link' : 'other'
          return { name: filename, type, size: attrs.size, modified: new Date(attrs.mtime * 1000).toISOString().slice(0, 16).replace('T', ' ') }
        })
        .sort((a, b) => Number(b.type === 'dir') - Number(a.type === 'dir') || a.name.localeCompare(b.name))
      return { entries: entries.slice(0, MAX_LIST_ENTRIES), truncated: entries.length > MAX_LIST_ENTRIES }
    },
    override,
  )
}

export function sftpReadText(host: SshHost, file: string, override?: Partial<ConnectConfig>): Promise<string> {
  return withSftp(
    host,
    async (sftp) => {
      const attrs = await promisify<[string], { size: number }>((p, cb) => sftp.stat(p, cb))(file)
      if (attrs.size > MAX_READ_BYTES) throw new Error(`Dosya çok büyük (${attrs.size} bayt; sınır ${MAX_READ_BYTES}). ssh_exec ile head/tail/grep kullan veya ssh_download ile indir.`)
      const data = await promisify<[string], Buffer>((p, cb) => sftp.readFile(p, cb))(file)
      if (data.subarray(0, 8000).includes(0)) throw new Error('İkili dosya; metin olarak gösterilemez. ssh_download ile indir.')
      return data.toString('utf8')
    },
    override,
  )
}

/** Yarım yazılmış dosya kalmasın: geçici dosyaya yazılır, sonra yerine taşınır. */
export function sftpWriteText(host: SshHost, file: string, content: string, override?: Partial<ConnectConfig>): Promise<void> {
  return withSftp(
    host,
    async (sftp) => {
      const tmp = `${file}.kor-${process.pid}-${Date.now()}.tmp`
      await promisify<[string, string], void>((p, d, cb) => sftp.writeFile(p, d, cb))(tmp, content)
      try {
        // posix-rename hedefin üzerine yazar (OpenSSH); desteklenmiyorsa sil-taşı
        await promisify<[string, string], void>((a, b, cb) => sftp.ext_openssh_rename(a, b, cb))(tmp, file)
      } catch {
        await promisify<[string], void>((p, cb) => sftp.unlink(p, cb))(file).catch(() => undefined)
        await promisify<[string, string], void>((a, b, cb) => sftp.rename(a, b, cb))(tmp, file)
      }
    },
    override,
  )
}

export function sftpUpload(host: SshHost, localAbs: string, remote: string, override?: Partial<ConnectConfig>): Promise<number> {
  return withSftp(
    host,
    async (sftp) => {
      const { size } = await stat(localAbs)
      if (size > MAX_TRANSFER_BYTES) throw new Error(`Dosya çok büyük (${size} bayt; sınır ${MAX_TRANSFER_BYTES}).`)
      await pipeline(createReadStream(localAbs), sftp.createWriteStream(remote))
      return size
    },
    override,
  )
}

export function sftpDownload(host: SshHost, remote: string, localAbs: string, override?: Partial<ConnectConfig>): Promise<number> {
  return withSftp(
    host,
    async (sftp) => {
      const attrs = await promisify<[string], { size: number }>((p, cb) => sftp.stat(p, cb))(remote)
      if (attrs.size > MAX_TRANSFER_BYTES) throw new Error(`Dosya çok büyük (${attrs.size} bayt; sınır ${MAX_TRANSFER_BYTES}).`)
      await pipeline(sftp.createReadStream(remote), createWriteStream(localAbs))
      return attrs.size
    },
    override,
  )
}

/** Sunucudan erişilen bir adrese (ör. 127.0.0.1:5432) SSH tüneli; veritabanı sürücüsü bu akışı soket gibi kullanır. */
export async function openTunnel(host: SshHost, dstHost: string, dstPort: number, override?: Partial<ConnectConfig>): Promise<Duplex> {
  const client = await acquire(host, override)
  try {
    return await new Promise<Duplex>((resolve, reject) =>
      client.forwardOut('127.0.0.1', 0, dstHost, dstPort, (err, stream) => (err ? reject(err) : resolve(stream))),
    )
  } finally {
    release(host.id)
  }
}

/** Modele dönen komut özeti: çıkış kodu + stdout/stderr (boşsa yazılmaz). */
export function formatExec(r: ExecResult): string {
  const head = r.timedOut ? 'Zaman aşımı: komut durduruldu.' : `Çıkış kodu: ${r.code ?? '?'}${r.signal ? ` (sinyal ${r.signal})` : ''}`
  const parts = [head]
  if (r.stdout.trim()) parts.push(`--- stdout ---\n${r.stdout.trimEnd()}`)
  if (r.stderr.trim()) parts.push(`--- stderr ---\n${r.stderr.trimEnd()}`)
  if (r.truncated) parts.push(`[çıktı ${MAX_EXEC_OUTPUT} karakterde kesildi]`)
  return parts.join('\n')
}
