// node --test scripts/assets-check.test.ts
// §16.4 "Varlık" (ADR 0061): every file in assets/ is used by at least one surface, and every
// `assets/<path>` a surface names exists. A dead asset or a dangling reference fails the build.
import { createHash } from 'node:crypto'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const REPO = resolve(fileURLToPath(import.meta.url), '../../..')
const ASSETS = join(REPO, 'assets')
/** Where surfaces live: the Rust workspace, the desktop app, the extension, CI and packaging. */
const SOURCES = ['backend/apps', 'backend/crates', 'backend/README.md', 'frontend/src', 'frontend/scripts', 'frontend/src-tauri/src', 'frontend/src-tauri/build.rs', 'frontend/src-tauri/tauri.conf.json', 'frontend/src-tauri/tauri.release.conf.json', 'frontend/src-tauri/Cargo.toml', 'frontend/extension', 'frontend/package.json', '.github', 'scripts', 'README.md', 'docs']
const SKIP_DIRS = new Set(['target', 'node_modules', 'dist', 'gen', '.git'])
const TEXT = /\.(rs|ts|tsx|js|mjs|json|toml|md|yml|yaml|css|html|py|sh|ps1|ftl)$/
/** Files that describe a folder rather than being used (licences, folder READMEs). */
const NOT_ASSETS = /(^|\/)(README\.md|[^/]*-OFL\.txt)$/

function walk(dir: string, out: string[] = []): string[] {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name)
    if (e.isDirectory()) {
      if (!SKIP_DIRS.has(e.name)) walk(p, out)
    } else out.push(p)
  }
  return out
}

function sources(): Array<{ path: string; text: string }> {
  const files: string[] = []
  for (const s of SOURCES) {
    const p = join(REPO, s)
    if (!existsSync(p)) continue
    if (statSync(p).isDirectory()) files.push(...walk(p).filter((f) => TEXT.test(f)))
    else files.push(p)
  }
  return files.filter((f) => !f.endsWith('assets-check.test.ts')).map((path) => ({ path, text: readFileSync(path, 'utf8') }))
}

const posix = (p: string) => p.split('\\').join('/')

test('every asset is referenced by a surface', () => {
  const all = sources().map((s) => s.text).join('\n')
  const assets = walk(ASSETS).map((f) => posix(relative(ASSETS, f))).filter((a) => !NOT_ASSETS.test(a))
  // A folder read as a whole (`assets/panels/`, `include_dir`-style loops) covers its files.
  const unused = assets.filter((a) => {
    const name = a.split('/').pop()!
    const dir = a.includes('/') ? a.slice(0, a.lastIndexOf('/')) : null
    return !all.includes(name) && !all.includes(`assets/${a}`) && !(dir !== null && new RegExp(`assets/${dir}/?['"\`]`).test(all))
  })
  assert.deepEqual(unused, [], 'remove the file or use it')
})

test('every assets/<path> reference exists', () => {
  const missing: string[] = []
  for (const s of sources()) {
    for (const m of s.text.matchAll(/(?<![\w-])assets\/([\w.@/-]+\.[a-z0-9]{2,5})\b/g)) {
      const ref = m[1]!
      // Paths of other projects' asset folders (renderer bundles, extension) are not repo assets.
      const at = s.text.slice(Math.max(0, m.index! - 40), m.index!)
      if (/(dist|renderer|extension|public|src)\/$/.test(at) || /[\w.-]\/$/.test(at) && !/\.\.\/$/.test(at)) continue
      if (!existsSync(join(ASSETS, ref))) missing.push(`${posix(relative(REPO, s.path))}: assets/${ref}`)
    }
  }
  assert.deepEqual([...new Set(missing)], [])
})

test('uygulama simgeleri bugünkü logodan üretilmiş (SOURCE.sha256, icon-gen.ts)', () => {
  const root = resolve(fileURLToPath(new URL('.', import.meta.url)), '../..')
  const svg = readFileSync(join(root, 'assets/kor-mark-color.svg'))
  const want = createHash('sha256').update(svg).digest('hex')
  const rec = readFileSync(join(root, 'frontend/src-tauri/icons/SOURCE.sha256'), 'utf8').split(/\s+/)[0]
  assert.equal(rec, want, 'logo değişti ama simgeler yeniden üretilmedi: `node scripts/icon-gen.ts` (frontend)')
})
