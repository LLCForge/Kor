// Arama için ripgrep yolu. Uygulama kendi ikilisini taşır (@vscode/ripgrep-<platform>-<arch>, VS Code'un da
// kullandığı paket); kullanıcının sisteminde rg kurulu olması gerekmez. Sıra:
//   1. Paketlenmiş masaüstü: resources/ripgrep (scripts/dist-config.ts extraResources; node_modules pakete girmez).
//   2. Geliştirme / CLI: node_modules'taki platform paketi.
//   3. PATH'teki `rg`. O da yoksa grep aracı Node yedeğine düşer.
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'

const BINARY = process.platform === 'win32' ? 'rg.exe' : 'rg'
let cached: string | undefined

function packaged(): string | null {
  const resources = (process as { resourcesPath?: string }).resourcesPath
  if (!resources) return null
  const candidate = path.join(resources, 'ripgrep', BINARY)
  return existsSync(candidate) ? candidate : null
}

function fromNodeModules(): string | null {
  try {
    // CJS paketinde (dist/main.cjs) require vardır; ESM'de (CLI, testler) giriş betiğine göre çözülür (code-intel.ts ile aynı).
    const req = typeof require === 'function' ? require : createRequire(path.resolve(process.argv[1] ?? 'index.js'))
    const resolved = req.resolve(`@vscode/ripgrep-${process.platform}-${process.arch}/bin/${BINARY}`)
    return existsSync(resolved) ? resolved : null
  } catch {
    return null // desteklenmeyen platform/mimari: sistemdeki rg denenir
  }
}

/** Çalıştırılacak ripgrep komutu (mutlak yol ya da PATH'te aranacak `rg`). */
export function ripgrepPath(): string {
  cached ??= packaged() ?? fromNodeModules() ?? 'rg'
  return cached
}
