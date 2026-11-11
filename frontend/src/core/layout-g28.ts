// [g28] Proje noktası .kor, kullanıcı klasörü ~/.kor.
// Eski proje .venice ve ev klasörü .kor silinmez. Kopya doğrulanırsa yeni yol esastır; bozuksa eski durur.
import { cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

export const PROJECT_DOT = '.kor'
export const LEGACY_PROJECT_DOT = '.venice'
export const USER_DIR_NAME = '.kor'
export const LEGACY_USER_DIR = '.kor'

/**
 * Kalıcı veri kökü: ev dizininde SABİT ad. Kurulum klasörü, sürüm, Electron userData yolu ve paket türü
 * (kurulum/taşınabilir/MSI) değişse de hafıza aynı yerde kalır. Üstüne KOR_HOME yalnızca açıkça verilirse yön değişir.
 */
export function persistentDataRoot(): string {
  return path.join(os.homedir(), USER_DIR_NAME)
}

const adopted = new Set<string>()

function fileCount(abs: string): number {
  let n = 0
  const walk = (dir: string) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, ent.name)
      if (ent.isDirectory()) walk(full)
      else if (ent.isFile()) n += 1
    }
  }
  walk(abs)
  return n
}

function populated(abs: string): boolean {
  if (!existsSync(abs)) return false
  const st = statSync(abs)
  if (st.isFile()) return true
  return readdirSync(abs).length > 0
}

/**
 * Eski ağacı yeni yola kopyalar. Yeni doluysa dokunmaz.
 * Kopya dosya sayısı tutmazsa yeni parçayı kaldırır (eskiye dokunmaz) ve false döner.
 */
export function copyLegacyTree(oldAbs: string, nextAbs: string): boolean {
  if (!existsSync(oldAbs)) return false
  if (populated(nextAbs)) return true
  const stage = `${nextAbs}.migrate`
  try {
    rmSync(stage, { recursive: true, force: true })
    mkdirSync(path.dirname(nextAbs), { recursive: true })
    cpSync(oldAbs, stage, { recursive: true, force: false })
    const oldCount = statSync(oldAbs).isDirectory() ? fileCount(oldAbs) : 1
    const nextCount = statSync(stage).isDirectory() ? fileCount(stage) : 1
    if (oldCount !== nextCount) {
      rmSync(stage, { recursive: true, force: true })
      return false
    }
    if (existsSync(nextAbs) && !populated(nextAbs)) rmSync(nextAbs, { recursive: true, force: true })
    if (existsSync(nextAbs)) {
      rmSync(stage, { recursive: true, force: true })
      return true
    }
    renameSync(stage, nextAbs)
    return existsSync(nextAbs)
  } catch {
    try {
      rmSync(stage, { recursive: true, force: true })
    } catch {
      // yarım kopya kalırsa eski klasör durur
    }
    return false
  }
}

/** Proje .venice ağacını .kor altına alır. Süreçte kök başına bir kez. */
export function adoptProject(root: string): void {
  const key = path.resolve(root)
  if (adopted.has(key)) return
  adopted.add(key)
  const old = path.join(root, LEGACY_PROJECT_DOT)
  const next = path.join(root, PROJECT_DOT)
  if (!existsSync(old) || populated(next)) return
  const ok = copyLegacyTree(old, next)
  if (!ok) adopted.delete(key)
}

/** Yazma ve okuma kökü. Kopya duruyorsa .kor; bozuksa ve yeni yoksa eski. */
export function projectDotDir(root: string): string {
  adoptProject(root)
  const next = path.join(root, PROJECT_DOT)
  const old = path.join(root, LEGACY_PROJECT_DOT)
  if (populated(next)) return next
  if (existsSync(old)) return old
  return next
}

/**
 * Kullanıcı veri klasörü.
 * Taşınabilir paket ortam değişkeniyle kendi klasörünü dayatıyordu; o yüzden taşınabilir ↔ kurulum geçişinde hafıza
 * "kayboluyordu". Artık sıra: açık KOR_HOME → kalıcı sabit kök. Eski kökler (kurulumun kendi klasörü, .kor)
 * varsa içerik kalıcı köke BİR KEZ taşınır ve hiçbir şey silinmez.
 */
export function userConfigDir(): string {
  const override = process.env.KOR_HOME
  if (override) return override
  const next = persistentDataRoot()
  const old = path.join(os.homedir(), LEGACY_USER_DIR)
  // Eski ortam değişkeni (VENICE_AGENT_HOME) yalnızca kalıcı kök hiç yokken kullanılır.
  const legacyEnv = process.env.VENICE_AGENT_HOME
  if (populated(next)) {
    // Eski ev klasörü varsa ve yeni doluysa yalnızca eksik dosyalar tamamlanır.
    if (existsSync(old)) mergeMissing(old, next)
    return next
  }
  if (existsSync(old) && copyLegacyTree(old, next) && populated(next)) {
    backupPath()
    return next
  }
  if (legacyEnv && !existsSync(next) && populated(legacyEnv)) {
    if (copyLegacyTree(legacyEnv, next) && populated(next)) return next
  }
  return next
}

const BACKUP_DIR = 'yedekler'
const KEEP_BACKUPS = 10

/**
 * Açılış anında hafıza dosyalarının anlık kopyası. Kullanıcı yanlışlıkla silse bile son hâl geri getirilebilir.
 * Yalnızca hafıza/ayar dosyaları (küçük); sohbetler zaten kendi klasöründe.
 */
export function backupPath(): string | null {
  const root = userConfigDirSafe()
  const files = ['memory.md', 'settings.json', 'organization-memory.md']
  const present = files.map((f) => path.join(root, f)).filter((f) => existsSync(f))
  if (present.length === 0) return null
  try {
    const dir = path.join(root, BACKUP_DIR)
    mkdirSync(dir, { recursive: true })
    const stamp = new Date().toISOString().replace(/[:.]/g, '-')
    for (const file of present) cpSync(file, path.join(dir, `${path.basename(file)}.${stamp}`), { force: true })
    pruneBackups(dir)
    return dir
  } catch {
    return null
  }
}

function pruneBackups(dir: string): void {
  try {
    const entries = readdirSync(dir)
      .map((name) => ({ name, at: statSync(path.join(dir, name)).mtimeMs }))
      .sort((a, b) => b.at - a.at)
    for (const old of entries.slice(KEEP_BACKUPS)) rmSync(path.join(dir, old.name), { force: true })
  } catch {
    // yedek budanamazsa eskiler kalır
  }
}

/** userConfigDir kendi içinde userConfigDir() çağırmasın diye ayrık sarmalayıcı (sonsuz döngü koruması). */
let resolving = false
function userConfigDirSafe(): string {
  if (resolving) return persistentDataRoot()
  resolving = true
  try {
    return userConfigDir()
  } finally {
    resolving = false
  }
}

/** Eski ağaçtaki eksik dosyaları hedefe kopyalar (var olanı ezmez). */
function mergeMissing(from: string, to: string): void {
  try {
    for (const ent of readdirSync(from, { withFileTypes: true })) {
      const src = path.join(from, ent.name)
      const dst = path.join(to, ent.name)
      if (ent.isDirectory()) {
        if (!existsSync(dst)) cpSync(src, dst, { recursive: true })
      } else if (!existsSync(dst)) {
        cpSync(src, dst)
      }
    }
  } catch {
    // birleştirme başarısız olursa mevcut veri durur
  }
}
