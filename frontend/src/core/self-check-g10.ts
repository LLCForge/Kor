// [g10] Başlangıç öz-denetimi: API anahtarı, ağ, disk ve yardımcı programlar. Sorunlar listelenir; hiçbiri uygulamayı durdurmaz.
// Anahtarın kendisi asla sonuca, günlüğe veya arayüze yazılmaz; yalnızca Venice'a Authorization başlığında gider.
import { execFile } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { mkdir, rm, statfs, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { gitExePath } from './git-path.ts'
import { ripgrepPath } from './ripgrep.ts'
import { USER_CONFIG_DIR, VENICE_BASE_URL } from './settings.ts'

export type CheckStatus = 'ok' | 'warn' | 'fail'

export interface CheckItem {
  id: 'apiKey' | 'network' | 'disk' | 'git' | 'ripgrep' | 'node' | 'config'
  label: string
  status: CheckStatus
  message: string
}

export interface SelfCheckProbes {
  hasApiKey(): boolean
  /** Anahtar doğrulama isteği: HTTP durum kodu veya ağ hatasında null */
  keyStatus(): Promise<number | null>
  /** Anahtarsız erişilebilirlik: HTTP durum kodu veya ağ hatasında null */
  reachStatus(): Promise<number | null>
  freeBytes(dir: string): Promise<number | null>
  canWrite(dir: string): Promise<boolean>
  version(command: string): Promise<string | null>
  nodeVersion: string
  /** Geçersiz ayar dosyaları ("dosya: sorun"); verilmezse denetlenmez */
  configProblems?(): Promise<string[]>
}

const WARN_FREE_BYTES = 1_000_000_000
const FAIL_FREE_BYTES = 100_000_000
const MIN_NODE = [23, 6]

function nodeAtLeast(version: string, [major, minor]: number[]): boolean {
  const [a = 0, b = 0] = version.split('.').map(Number)
  return a > (major ?? 0) || (a === major && b >= (minor ?? 0))
}

const gb = (n: number) => `${(n / 1e9).toFixed(1)} GB`

export async function runSelfCheck(p: SelfCheckProbes, dataDir: string = USER_CONFIG_DIR): Promise<CheckItem[]> {
  const items: CheckItem[] = []
  const [reach, key, free, writable, git, rg, config] = await Promise.all([
    p.reachStatus(),
    p.hasApiKey() ? p.keyStatus() : Promise.resolve(null),
    p.freeBytes(dataDir),
    p.canWrite(dataDir),
    p.version(gitExePath()),
    p.version(ripgrepPath()),
    p.configProblems ? p.configProblems().catch((err: unknown) => [String(err)]) : Promise.resolve(null),
  ])

  if (!p.hasApiKey()) items.push({ id: 'apiKey', label: 'API anahtarı', status: 'fail', message: 'Kayıtlı Venice API anahtarı yok.' })
  else if (key === 401 || key === 403) items.push({ id: 'apiKey', label: 'API anahtarı', status: 'fail', message: 'Venice anahtarı reddetti (geçersiz, iptal edilmiş veya süresi dolmuş).' })
  else if (key !== null && key >= 200 && key < 300) items.push({ id: 'apiKey', label: 'API anahtarı', status: 'ok', message: 'Anahtar geçerli.' })
  else items.push({ id: 'apiKey', label: 'API anahtarı', status: 'warn', message: key === null ? 'Anahtar ağ sorunu nedeniyle doğrulanamadı.' : `Anahtar doğrulanamadı (HTTP ${key}).` })

  if (reach === null) items.push({ id: 'network', label: 'Ağ', status: 'fail', message: "Venice API'sine ulaşılamıyor (çevrimdışı, DNS veya güvenlik duvarı)." })
  else if (reach >= 500) items.push({ id: 'network', label: 'Ağ', status: 'warn', message: `Venice API'si hata veriyor (HTTP ${reach}); geçici olabilir.` })
  else items.push({ id: 'network', label: 'Ağ', status: 'ok', message: "Venice API'sine erişiliyor." })

  if (!writable) items.push({ id: 'disk', label: 'Disk', status: 'fail', message: `Veri klasörüne yazılamıyor: ${dataDir}. Sohbetler ve kurtarma günlüğü kaydedilemez.` })
  else if (free !== null && free < FAIL_FREE_BYTES) items.push({ id: 'disk', label: 'Disk', status: 'fail', message: `Diskte çok az yer kaldı (${gb(free)}).` })
  else if (free !== null && free < WARN_FREE_BYTES) items.push({ id: 'disk', label: 'Disk', status: 'warn', message: `Diskte az yer kaldı (${gb(free)}).` })
  else items.push({ id: 'disk', label: 'Disk', status: 'ok', message: free === null ? 'Yazılabilir.' : `Yazılabilir, ${gb(free)} boş.` })

  items.push(
    git
      ? { id: 'git', label: 'git', status: 'ok', message: git }
      : { id: 'git', label: 'git', status: 'warn', message: 'git bulunamadı: git işlemleri ve yıkıcı git komutları öncesi kontrol noktası çalışmaz.' },
  )
  items.push(
    rg
      ? { id: 'ripgrep', label: 'ripgrep', status: 'ok', message: rg }
      : { id: 'ripgrep', label: 'ripgrep', status: 'warn', message: 'ripgrep (rg) bulunamadı: arama daha yavaş Node yedeğiyle yapılır.' },
  )
  items.push(
    nodeAtLeast(p.nodeVersion, MIN_NODE)
      ? { id: 'node', label: 'Node', status: 'ok', message: `Node ${p.nodeVersion}` }
      : { id: 'node', label: 'Node', status: 'fail', message: `Node ${p.nodeVersion} eski; en az ${MIN_NODE.join('.')} gerekir.` },
  )
  if (config !== null) {
    items.push(
      config.length === 0
        ? { id: 'config', label: 'Yapılandırma', status: 'ok', message: 'Ayar dosyaları geçerli.' }
        : { id: 'config', label: 'Yapılandırma', status: 'fail', message: `Hatalı ayar dosyası (düzeltilene kadar ilgili özellik çalışmaz):\n${config.join('\n')}` },
    )
  }
  return items
}

async function httpStatus(url: string, headers: Record<string, string>): Promise<number | null> {
  try {
    const res = await fetch(url, { headers, signal: AbortSignal.timeout(8_000) })
    await res.body?.cancel()
    return res.status
  } catch {
    return null
  }
}

export function defaultProbes(apiKey: () => string | null, root: () => string | null = () => null): SelfCheckProbes {
  return {
    configProblems: async () => (await import('./config-check.ts')).validateConfigFiles(root()),
    hasApiKey: () => apiKey() !== null,
    keyStatus: () => {
      const key = apiKey()
      return key ? httpStatus(`${VENICE_BASE_URL}/api_keys/rate_limits`, { Authorization: `Bearer ${key}` }) : Promise.resolve(null)
    },
    reachStatus: () => httpStatus(`${VENICE_BASE_URL}/models?type=text`, {}),
    freeBytes: async (dir) => {
      try {
        await mkdir(dir, { recursive: true })
        const s = await statfs(dir)
        return s.bavail * s.bsize
      } catch {
        return null
      }
    },
    canWrite: async (dir) => {
      const probe = path.join(dir, `.g10-write-${randomBytes(4).toString('hex')}`)
      try {
        await mkdir(dir, { recursive: true })
        await writeFile(probe, 'ok', { flag: 'wx' })
        await rm(probe, { force: true })
        return true
      } catch {
        return false
      }
    },
    version: (command) =>
      new Promise((resolve) => {
        execFile(command, ['--version'], { timeout: 5_000, windowsHide: true }, (err, stdout) => {
          resolve(err ? null : (stdout.split(/\r?\n/)[0]?.trim() ?? null))
        })
      }),
    nodeVersion: process.versions.node,
  }
}
