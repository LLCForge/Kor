// Git yolu: uygulama kendi taşınabilir git'ini taşır (dugite'ın indirdiği portable Git for Windows / git), ripgrep gibi.
// Kullanıcının sisteminde git kurulu olması gerekmez. Sıra:
//   1. Paketlenmiş masaüstü: resources/git (scripts/dist-config.ts extraResources; node_modules pakete girmez).
//   2. Geliştirme / CLI: node_modules/dugite/git.
//   3. PATH'teki `git`. O da yoksa git tabanlı özellikler (kontrol noktası, worktree, eklenti kurulumu) devre dışı kalır.
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'

// Taşınabilir git'in kendi kurulumunu konumuna göre bulan sarmalayıcı giriş noktası.
const REL = process.platform === 'win32' ? ['cmd', 'git.exe'] : ['bin', 'git']
let cached: string | undefined

function packaged(): string | null {
  const resources = (process as { resourcesPath?: string }).resourcesPath
  if (!resources) return null
  const candidate = path.join(resources, 'git', ...REL)
  return existsSync(candidate) ? candidate : null
}

function fromNodeModules(): string | null {
  try {
    // CJS paketinde require vardır; ESM'de (CLI, testler) giriş betiğine göre çözülür (ripgrep.ts ile aynı desen).
    const req = typeof require === 'function' ? require : createRequire(path.resolve(process.argv[1] ?? 'index.js'))
    const dugitePkg = req.resolve('dugite/package.json')
    const candidate = path.join(path.dirname(dugitePkg), 'git', ...REL)
    return existsSync(candidate) ? candidate : null
  } catch {
    return null // dugite yok (ör. yalnız CLI kurulumu): sistemdeki git denenir
  }
}

/** Çalıştırılacak git komutu (gömülü mutlak yol ya da PATH'te aranacak `git`). */
export function gitExePath(): string {
  cached ??= packaged() ?? fromNodeModules() ?? 'git'
  return cached
}
