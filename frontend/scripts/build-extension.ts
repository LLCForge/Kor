// Kor tarayıcı eklentisi derlemesi (§18, ADR 0058): TypeScript + React kaynağı (src/extension) esbuild ile
// extension/dist/chrome (Chrome, Edge, Brave, Chromium) ve extension/dist/firefox altına paketlenir. Tema
// (extension/theme.css, ADR 0053) ve WebView kilidi (webview-lock-extension.ts, §21.2) aynen kullanılır; sayfa
// görüntüsü betiği headless tarayıcıyla aynı snapshot.js'ten üretilir. `node scripts/build-extension.ts [--production]`.
import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build, type BuildOptions } from 'esbuild'
import { manifest, messages, LOCALES, type Target } from '../src/extension/manifest.ts'

const FRONTEND = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const ROOT = resolve(FRONTEND, '..')
const SRC = resolve(FRONTEND, 'src/extension')
export const DIST = resolve(FRONTEND, 'extension/dist')

/**
 * Marka bileşenleri (KorThinking, KorSpinner, KorDots) tek kaynaktan, `assets/KorThinking.tsx`'ten paketlenir (§16.2).
 * Dosya frontend paketinin dışında olduğundan tsc onu CommonJS sayar; TS `kor-assets/KorThinking` bildirimini
 * (renderer/kor-assets.d.ts) görür, esbuild takma adla asıl dosyayı ve frontend/node_modules'taki React'i bağlar.
 */
export const KOR_ASSETS: Pick<BuildOptions, 'alias' | 'nodePaths'> = {
  alias: { 'kor-assets/KorThinking': resolve(ROOT, 'assets/KorThinking.tsx'), 'kor-assets/sounds': resolve(ROOT, 'assets/sounds') },
  nodePaths: [resolve(FRONTEND, 'node_modules')],
}

/** Headless tarayıcının erişilebilirlik görüntüsü (bir ifade) → eklentinin sayfa betiğinin işlevi. */
export async function writeSnapshot(): Promise<void> {
  const js = await readFile(resolve(ROOT, 'backend/crates/kor-tools/src/tools/browser/snapshot.js'), 'utf8')
  const body = `// ÜRETİLDİ: scripts/build-extension.ts, kaynak backend/crates/kor-tools/src/tools/browser/snapshot.js. Elle düzenlemeyin.\nexport function snapshot() {\n  return ${js.trim().replace(/;$/, '')}\n}\n`
  await writeFile(resolve(SRC, 'page/snapshot.gen.js'), body)
}

async function version(): Promise<string> {
  const pkg = JSON.parse(await readFile(resolve(FRONTEND, 'package.json'), 'utf8')) as { version: string }
  return pkg.version
}

export async function buildExtension(production: boolean): Promise<void> {
  await writeSnapshot()
  const v = await version()
  const common: BuildOptions = {
    bundle: true,
    minify: production,
    sourcemap: false,
    logLevel: 'warning',
    platform: 'browser',
    format: 'iife',
    define: { 'process.env.NODE_ENV': JSON.stringify(production ? 'production' : 'development') },
  }
  for (const target of ['chrome', 'firefox'] as Target[]) {
    const out = resolve(DIST, target)
    await rm(out, { recursive: true, force: true })
    await mkdir(resolve(out, 'icons'), { recursive: true })
    const browserTarget = target === 'chrome' ? 'chrome116' : 'firefox128'
    await Promise.all([
      build({ ...common, target: browserTarget, entryPoints: [resolve(SRC, 'background.ts')], outfile: resolve(out, 'background.js') }),
      build({ ...common, target: browserTarget, entryPoints: [resolve(SRC, 'page/index.ts')], outfile: resolve(out, 'page.js') }),
      build({ ...common, target: browserTarget, entryPoints: [resolve(SRC, 'page/console.ts')], outfile: resolve(out, 'console.js') }),
      build({ ...common, ...KOR_ASSETS, target: browserTarget, jsx: 'automatic', entryPoints: [resolve(SRC, 'panel/main.tsx')], outfile: resolve(out, 'sidepanel.js') }),
      build({ ...common, target: browserTarget, entryPoints: [resolve(FRONTEND, 'src/desktop/renderer/webview-lock-extension.ts')], outfile: resolve(out, 'webview-lock.js') }),
    ])
    await copyFile(resolve(SRC, 'static/sidepanel.html'), resolve(out, 'sidepanel.html'))
    await copyFile(resolve(FRONTEND, 'extension/theme.css'), resolve(out, 'theme.css'))
    await copyFile(resolve(ROOT, 'assets/ic_launcher-48.png'), resolve(out, 'icons/48.png'))
    await copyFile(resolve(ROOT, 'assets/favicon-96x96.png'), resolve(out, 'icons/96.png'))
    await copyFile(resolve(ROOT, 'assets/favicon-128x128.png'), resolve(out, 'icons/128.png'))
    for (const lang of Object.keys(LOCALES)) {
      await mkdir(resolve(out, '_locales', lang), { recursive: true })
      await writeFile(resolve(out, '_locales', lang, 'messages.json'), JSON.stringify(messages(lang), null, 2))
    }
    await writeFile(resolve(out, 'manifest.json'), JSON.stringify(manifest(target, v), null, 2))
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await buildExtension(process.argv.includes('--production'))
  console.log(`Kor eklentisi hazır: ${DIST}`)
}
