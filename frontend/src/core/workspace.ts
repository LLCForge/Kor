import { lstatSync, realpathSync } from 'node:fs'
import path from 'node:path'
import { scratchDir } from './scratch-g33.ts'

let additionalRoots: readonly string[] = []

/** Ek çalışma dizinleri (--add-dir, additionalDirectories): dosya araçları bunları da kök kabul eder. */
export function setAdditionalDirectories(dirs: readonly string[]): void {
  additionalRoots = [...new Set(dirs.map((d) => path.resolve(d)))]
}

export function getAdditionalDirectories(): readonly string[] {
  return additionalRoots
}

export function isWithin(base: string, target: string): boolean {
  const rel = path.relative(base, target)
  return rel === '' || (rel !== '..' && !rel.startsWith(`..${path.sep}`) && !path.isAbsolute(rel))
}

/**
 * Var olan en yakın üst dizinin gerçek yolunu (symlink/junction çözülmüş) bulur ve henüz olmayan
 * parçaları ekler; böylece oluşturulacak dosyalar da denetlenir.
 */
export function realpathOfNearest(abs: string): string {
  const missing: string[] = []
  let current = abs
  for (;;) {
    try {
      return path.join(realpathSync.native(current), ...missing.reverse())
    } catch (err) {
      const code = (err as NodeJS.ErrnoException).code
      if (code !== 'ENOENT' && code !== 'ENOTDIR') throw err
      // Kırık bağlantı: hedefi dışarıda olabilir ve yazma onu takip eder (CWE-59); çözülemiyorsa reddedilir.
      if (lstatSync(current, { throwIfNoEntry: false })?.isSymbolicLink()) {
        throw new Error(`Çözülemeyen sembolik bağlantı reddedildi: ${current}`)
      }
      const parent = path.dirname(current)
      if (parent === current) return abs
      missing.push(path.basename(current))
      current = parent
    }
  }
}

/**
 * Yolu çalışma dizinine (ve ek dizinlere) hapseder (CWE-22). Önce sözdizimsel kontrol,
 * sonra gerçek yol kontrolü: dizin içindeki bir symlink/junction dışarıyı gösteriyorsa reddedilir (CWE-59).
 * shortcut: denetim ile kullanım arasında bağlantı değiştirilirse (TOCTOU) yakalanmaz; O_NOFOLLOW gerekirse ele alınır.
 */
export function confineToWorkspace(root: string, target: string): string {
  const abs = path.resolve(root, target)
  // [g33] İstem deneme dosyalarını scratch dizinine yazdırıyor; dosya araçları da orayı görmeli, yoksa yazım reddedilir.
  const roots = [path.resolve(root), ...additionalRoots, scratchDir(root)]
  if (!roots.some((r) => isWithin(r, abs)))
    throw new Error(
      `Çalışma dizini dışına erişim reddedildi: ${target}. Dosya araçları yalnızca çalışma dizinini (${path.resolve(root)}), eklenen dizinleri ve scratch dizinini (${scratchDir(root)}) görür; bunun dışındaki sistem temp (%TEMP%, /tmp) dahil hiçbir yeri göremez. İndirme/klonlama/çıkarma işlemini çalışma dizinine (göreli yol ya da alt klasör) yap, sonra dosya araçlarıyla oku.`,
    )
  const real = realpathOfNearest(abs)
  if (!roots.some((r) => isWithin(realpathOfNearest(r), real))) {
    throw new Error(`Sembolik bağlantı çalışma dizini dışını gösteriyor, erişim reddedildi: ${target}`)
  }
  return abs
}
