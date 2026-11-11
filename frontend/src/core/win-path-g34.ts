// [g34] Windows'ta kabuk komutları için PATH'i kayıt defterinden tazeler. Sorun: uygulama açıkken bir araç kurulursa
// (ör. winget ile PHP), winget PATH'i kayıt defterinde günceller ama çalışan süreçler (ve onları başlatan explorer)
// güncel değeri yeniden başlamadan görmez; o yüzden "php bulunamadı" hataları çıkar. Çözüm: komut çalıştırırken makine +
// kullanıcı PATH'ini kayıt defterinden okuyup birleştir, böylece yeni kurulan araçlar yeniden başlatmadan bulunur.
import { execFileSync } from 'node:child_process'

let cache: { value: string; at: number } | null = null
const TTL_MS = 15_000

function queryRegistryPath(hive: string): string {
  try {
    const out = execFileSync('reg', ['query', hive, '/v', 'Path'], { encoding: 'utf8', timeout: 4000, windowsHide: true })
    // Satır: "    Path    REG_EXPAND_SZ    C:\...;C:\..."
    const match = /\r?\n\s*Path\s+REG(?:_EXPAND)?_SZ\s+([^\r\n]*)/i.exec(out)
    return match?.[1]?.trim() ?? ''
  } catch {
    return '' // reg yoksa/okunamazsa boş: çağıran process PATH'ine düşer
  }
}

/** REG_EXPAND_SZ içindeki %VAR% parçalarını env'den genişletir; bilinmeyen değişken olduğu gibi bırakılır. */
export function expandEnvVars(text: string, env: NodeJS.ProcessEnv = process.env): string {
  return text.replace(/%([^%]+)%/g, (whole, name: string) => env[name] ?? env[name.toUpperCase()] ?? whole)
}

/** Windows'ta kayıt defterindeki güncel PATH (makine + kullanıcı, genişletilmiş). Null = Windows değil veya okunamadı. 15 sn önbellekli. */
export function freshSystemPath(env: NodeJS.ProcessEnv = process.env): string | null {
  if (process.platform !== 'win32') return null
  const now = Date.now()
  if (cache && now - cache.at < TTL_MS) return cache.value
  const machine = expandEnvVars(queryRegistryPath('HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment'), env)
  const user = expandEnvVars(queryRegistryPath('HKCU\\Environment'), env)
  const value = [machine, user].filter(Boolean).join(';') || null
  if (value) cache = { value, at: now }
  return value
}
