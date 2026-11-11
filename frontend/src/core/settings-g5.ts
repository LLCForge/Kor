import { readFileSync } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { settingsFiles } from './settings.ts'

export const SHELL_KINDS = ['cmd', 'powershell', 'pwsh', 'git-bash'] as const
export type ShellPreference = (typeof SHELL_KINDS)[number]

// settings.ts'teki [g5] alanlarıyla aynı kısıtlar; diğer alanlar o dosyada doğrulanır.
const g5Schema = z
  .object({
    shell: z.enum(SHELL_KINDS),
    additionalDirectories: z.array(z.string().min(1).max(4096)).max(50),
    autoVerify: z.boolean(),
  })
  .partial()

export interface G5Settings {
  shell: ShellPreference | null
  additionalDirectories: string[]
  autoVerify: boolean
}

const DEFAULTS: G5Settings = { shell: null, additionalDirectories: [], autoVerify: false }

/**
 * Güvenlik: bu ayarlar komut çalıştırmayı ve dosya erişimini genişletir; depoyla gelen proje dosyaları
 * bunları açamamalı. Bu yüzden yalnızca kullanıcı dosyasından (~/.kor/settings.json) okunur.
 */
function parse(text: string | null): G5Settings {
  if (text === null) return DEFAULTS
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch {
    return DEFAULTS // Hata ayrıntısını loadSettings dosya adıyla bildirir.
  }
  const parsed = g5Schema.safeParse(json)
  if (!parsed.success) return DEFAULTS
  return { ...DEFAULTS, ...parsed.data, shell: parsed.data.shell ?? null }
}

/** Modül yüklenirken (kabuk adı) çağrılır: okuma hatası uygulamayı düşürmez, varsayılana döner. */
export function loadG5SettingsSync(file: string = settingsFiles('.').user): G5Settings {
  try {
    return parse(readFileSync(file, 'utf8'))
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') console.warn(`${file} okunamadı:`, err instanceof Error ? err.message : err)
    return DEFAULTS
  }
}

export async function loadG5Settings(file: string = settingsFiles('.').user): Promise<G5Settings> {
  let text: string | null
  try {
    text = await readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err
    text = null
  }
  return parse(text)
}

/** Ek dizinleri mutlak yola çevirir; olmayan veya dizin olmayanlar uyarıyla atlanır. */
export async function validateDirectories(dirs: readonly string[], base: string, warn: (msg: string) => void): Promise<string[]> {
  const valid: string[] = []
  for (const dir of dirs) {
    const abs = path.resolve(base, dir)
    try {
      if ((await stat(abs)).isDirectory()) {
        valid.push(abs)
        continue
      }
    } catch {
      // Aşağıda uyarılır.
    }
    warn(`Ek dizin bulunamadı veya dizin değil, atlandı: ${abs}`)
  }
  return valid
}
