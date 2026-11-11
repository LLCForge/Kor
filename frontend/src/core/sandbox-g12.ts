// [g12] Çekirdek düzeyi sandbox (madde 97): kabuk komutlarını ağsız ve yalnızca çalışma dizinine yazabilen bir izolasyonda çalıştırır.
// Ayar yalnızca kullanıcı dosyasından okunur (depoyla gelen proje dosyası sandbox'ı kapatamaz/açamaz) ve varsayılanı kapalıdır.
// Açıkken izolasyon kurulamıyorsa komut çalıştırılmaz (kapalı başarısızlık): sahte bir koruma sunulmaz.
import { spawnSync } from 'node:child_process'
import { accessSync, constants, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { z } from 'zod'
import { settingsFiles } from './settings.ts'
import { getAdditionalDirectories } from './workspace.ts'

export interface SandboxStatus {
  /** Kullanıcı ayar dosyasında "sandbox": true */
  enabled: boolean
  /** Bu platformda gerçek bir çekirdek izolasyonu kurulabiliyor */
  available: boolean
  mechanism: 'bwrap' | null
  /** Kullanıcıya gösterilecek açıklama */
  message: string
}

interface Detection {
  available: boolean
  bwrap: string | null
  message: string
}

// PATH aranmaz: çalışma dizinindeki veya ajanın değiştirdiği PATH'teki sahte bir "bwrap" izolasyonu taklit edebilirdi.
const BWRAP_CANDIDATES = ['/usr/bin/bwrap', '/bin/bwrap', '/usr/local/bin/bwrap']

const settingSchema = z.object({ sandbox: z.boolean() }).partial()

/** Bozuk ayar dosyasında kullanıcının sandbox isteyip istemediği bilinemez: açık sayılır (kapalı başarısızlık). */
export function readSandboxSetting(file: string = settingsFiles('.').user): boolean {
  let text: string
  try {
    text = readFileSync(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return false
    console.warn(`${file} okunamadı; sandbox açık kabul edildi:`, err instanceof Error ? err.message : err)
    return true
  }
  try {
    const parsed = settingSchema.safeParse(JSON.parse(text))
    if (parsed.success) return parsed.data.sandbox === true
  } catch {
    // Aşağıda uyarılır.
  }
  console.warn(`${file} geçersiz; sandbox açık kabul edildi.`)
  return true
}

/** Saf: yalnızca kök ve ek dizinler yazılabilir, ağ ve diğer ad alanları ayrılır. Sıra önemli: /tmp tmpfs'i bağlamalardan önce gelir. */
export function bwrapArgs(o: { cwd: string; writable: readonly string[] }): string[] {
  const args = [
    '--unshare-all', // ağ dahil tüm ad alanları; --share-net verilmez
    '--die-with-parent',
    '--new-session', // TIOCSTI ile terminale komut enjeksiyonunu önler (CVE-2017-5226)
    '--ro-bind', '/', '/',
    '--dev', '/dev',
    '--proc', '/proc',
    '--tmpfs', '/tmp',
  ]
  for (const p of o.writable) args.push('--bind', p, p)
  args.push('--chdir', o.cwd)
  return args
}

function findBwrap(): string | null {
  for (const candidate of BWRAP_CANDIDATES) {
    try {
      accessSync(candidate, constants.X_OK)
      return candidate
    } catch {
      // Sıradaki aday.
    }
  }
  return null
}

const WINDOWS_MESSAGE =
  "Windows'ta kabuk komutları için gerçek bir çekirdek düzeyi izolasyon yok (Windows Sandbox ayrı bir sanal makinedir; AppContainer yerel kod ister). " +
  'Sahte bir koruma sunulmaz: sandbox açıkken komutlar çalıştırılmaz.'
const MAC_MESSAGE =
  "macOS'ta sandbox-exec Apple tarafından kullanımdan kaldırılmış olarak işaretli ve profil dili belgelenmemiş; bu sürümde desteklenmiyor. " +
  'Sandbox açıkken komutlar çalıştırılmaz.'

export function detectSandbox(platform: NodeJS.Platform = process.platform): Detection {
  if (platform === 'win32') return { available: false, bwrap: null, message: WINDOWS_MESSAGE }
  if (platform === 'darwin') return { available: false, bwrap: null, message: MAC_MESSAGE }
  if (platform !== 'linux') return { available: false, bwrap: null, message: `Bu platformda (${platform}) çekirdek düzeyi izolasyon desteklenmiyor.` }
  const bwrap = findBwrap()
  if (!bwrap) return { available: false, bwrap: null, message: 'bubblewrap (bwrap) bulunamadı; paket yöneticisiyle "bubblewrap" paketini kur.' }
  // Kullanıcı ad alanları kapalı olabilir (ör. bazı dağıtımların AppArmor kısıtı): gerçek bayraklarla bir kez denenir.
  const probe = spawnSync(bwrap, [...bwrapArgs({ cwd: '/', writable: [] }), '--', '/bin/sh', '-c', 'exit 0'], { timeout: 5000, encoding: 'utf8' })
  if (probe.status !== 0) {
    const reason = (probe.stderr || probe.error?.message || `çıkış kodu ${probe.status ?? '?'}`).trim().split('\n')[0]
    return { available: false, bwrap: null, message: `bwrap çalıştırılamadı (kullanıcı ad alanları kapalı olabilir): ${reason}` }
  }
  return {
    available: true,
    bwrap,
    message: 'bubblewrap ile: ağ kapalı, dosya sistemi salt okunur; yalnızca çalışma dizini ve ek dizinler yazılabilir, /tmp geçicidir.',
  }
}

let detection: Detection | undefined

export function sandboxStatus(): SandboxStatus {
  detection ??= detectSandbox()
  return { enabled: readSandboxSetting(), available: detection.available, mechanism: detection.bwrap ? 'bwrap' : null, message: detection.message }
}

const real = (p: string) => {
  try {
    return realpathSync.native(p)
  } catch {
    return p
  }
}

/**
 * Sandbox kapalıysa çağrıyı aynen döner; açıksa bwrap ile sarar, kurulamıyorsa hata fırlatır.
 * stateFiles: kabuğun durum dökümleri (shell-session.ts); /tmp sandbox'ta geçici olduğundan önceden oluşturulup yazılabilir bağlanır.
 */
export function sandboxInvocation<T extends { file: string; args: string[] }>(inv: T, target: { root: string; cwd: string; stateFiles?: readonly string[] }): T {
  if (!readSandboxSetting()) return inv
  detection ??= detectSandbox()
  if (!detection.available || !detection.bwrap) throw new Error(`Sandbox açık ama kurulamıyor: ${detection.message}`)
  const stateFiles = target.stateFiles ?? []
  for (const file of stateFiles) writeFileSync(file, '', { mode: 0o600 })
  const writable = [...new Set([target.root, ...getAdditionalDirectories()].map(real))]
  return { ...inv, file: detection.bwrap, args: [...bwrapArgs({ cwd: target.cwd, writable: [...writable, ...stateFiles] }), '--', inv.file, ...inv.args] }
}
