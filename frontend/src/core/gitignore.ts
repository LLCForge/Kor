import { readFile } from 'node:fs/promises'
import path from 'node:path'
import ignore, { type Ignore } from 'ignore'

/** Bir .gitignore dosyasının kuralları; base: dosyanın bulunduğu klasör ('/' ayraçlı, kök için ''). */
export interface IgnoreLayer {
  base: string
  ig: Ignore
}

export function createLayer(base: string, content: string): IgnoreLayer {
  return { base, ig: ignore().add(content) }
}

/**
 * Git'in iç içe .gitignore anlamı: kökten derine doğru her katman kendi göreli yoluyla sınanır,
 * derindeki katmanın kararı (yok sayma veya ! ile geri alma) üsttekini ezer.
 */
export function isIgnored(layers: readonly IgnoreLayer[], relPath: string, isDir: boolean): boolean {
  let ignored = false
  for (const layer of layers) {
    if (layer.base && !relPath.startsWith(`${layer.base}/`)) continue
    const rel = layer.base ? relPath.slice(layer.base.length + 1) : relPath
    if (!rel) continue
    const result = layer.ig.test(isDir ? `${rel}/` : rel)
    if (result.ignored) ignored = true
    else if (result.unignored) ignored = false
  }
  return ignored
}

async function readGitignore(root: string, dir: string): Promise<string | null> {
  try {
    return await readFile(path.join(root, dir, '.gitignore'), 'utf8')
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code
    if (code === 'ENOENT' || code === 'ENOTDIR' || code === 'EISDIR') return null
    throw err
  }
}

/** parent katmanlarına dir/.gitignore (varsa) eklenmiş liste. */
export async function withLayer(root: string, dir: string, parent: readonly IgnoreLayer[]): Promise<IgnoreLayer[]> {
  const content = await readGitignore(root, dir)
  return content === null ? [...parent] : [...parent, createLayer(dir, content)]
}

/** Kökten dir'e kadar (dahil) tüm .gitignore katmanları. */
export async function layersFor(root: string, dir: string): Promise<IgnoreLayer[]> {
  const parts = dir ? dir.split('/') : []
  let layers = await withLayer(root, '', [])
  for (let i = 1; i <= parts.length; i++) layers = await withLayer(root, parts.slice(0, i).join('/'), layers)
  return layers
}
