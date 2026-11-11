// Kor arayüz derlemesi: tarayıcı paketleri dist/renderer/ altına yazılır, Tauri webview bu klasörü sunar
// (src-tauri/tauri.conf.json -> build.frontendDist). Ana süreç yoktur; arka uç Rust'tır (src-tauri).
import { copyFile, mkdir, rm } from 'node:fs/promises'
import { build, context, type BuildOptions } from 'esbuild'
import { g21LazyBundles } from './g21-bundles.ts' // [g21]
import { writeGenerated as writeParityCommands } from './parity-commands.ts' // ADR 0031
import { writeGenerated as writeTheme } from './theme-gen.ts' // ADR 0053
import { writeGenerated as writeProviderLogos } from './provider-logos.ts' // ADR 0053
import { buildExtension, KOR_ASSETS } from './build-extension.ts' // ADR 0058: tarayıcı eklentisi (WebView kilidi dahil, §21.2)
import { writeGenerated as writeIpcTypes } from './ipc-gen.ts' // ADR 0061

writeParityCommands() // parity.toml → renderer/parity-commands.gen.ts (değişmediyse dokunmaz)
writeProviderLogos() // assets/providers/*.svg → renderer/provider-logos.gen.ts (§23.2)
writeIpcTypes() // kor-core/schema/ipc.json → src/desktop/ipc.gen.ts (§16.3)
writeTheme() // assets/theme.json → theme.gen.css, kor-core theme.rs, kor-theme.ts, Tauri arka planı (ADR 0053)

const production = process.argv.includes('--production')
const watch = process.argv.includes('--watch')
const common = { bundle: true, sourcemap: !production, minify: production, logLevel: 'warning' } as const
const browserDefine = { 'process.env.NODE_ENV': JSON.stringify(production ? 'production' : 'development') }
const browser = { ...common, platform: 'browser', format: 'iife', target: 'chrome130', define: browserDefine } as const

// Ağır/isteğe bağlı kütüphaneler ayrı IIFE paketidir; sohbette ilk kez gerektiğinde tembel yüklenir
// (bkz. rich-lazy.ts -> loadRichBundle).
const lazyBundles: BuildOptions[] = [
  { entryPoints: ['src/desktop/renderer/mermaid-entry.ts'], outfile: 'dist/renderer/mermaid.js' }, // ```mermaid
  // Mermaid dışı diyagram dilleri; her biri yalnızca ilgili blok görününce yüklenir (gömülü WASM/saf JS, ağ yok).
  { entryPoints: ['src/desktop/renderer/graphviz-entry.ts'], outfile: 'dist/renderer/graphviz.js' }, // ```dot / ```graphviz
  // nomnoml yalnızca renderSvg için kullanılır; Node-only compileFile'ın fs/path require'ları ölü koddur, dışlanır.
  { entryPoints: ['src/desktop/renderer/nomnoml-entry.ts'], outfile: 'dist/renderer/nomnoml.js', external: ['fs', 'path'] }, // ```nomnoml
  { entryPoints: ['src/desktop/renderer/wavedrom-entry.ts'], outfile: 'dist/renderer/wavedrom.js' }, // ```wavedrom
  { entryPoints: ['src/desktop/renderer/g19-map-entry.ts'], outfile: 'dist/renderer/g19-map.js' }, // [g19] ```map ```geojson
  { entryPoints: ['src/desktop/renderer/katex-entry.ts'], outfile: 'dist/renderer/katex.js' }, // [g16] matematik (MathView)
  { entryPoints: ['src/desktop/renderer/hljs-entry.ts'], outfile: 'dist/renderer/hljs.js' }, // [perf] diff sözdizimi vurgusu (DiffView)
  { entryPoints: ['src/desktop/renderer/codemirror-entry.ts'], outfile: 'dist/renderer/codemirror.js' }, // [g16] editör (CodeEditor)
  ...g21LazyBundles(production), // [g21] belge önizleme: pdf.js (+ gömülü çalışan), mammoth, three.js
]

const bundles: BuildOptions[] = [
  {
    ...browser,
    entryPoints: ['src/desktop/renderer/main.tsx'],
    outfile: 'dist/renderer/renderer.js',
    jsx: 'automatic',
    ...KOR_ASSETS, // assets/KorThinking.tsx (§16.1)
    // KaTeX yazı tipleri ayrı dosya olarak kopyalanır (CSP: font-src 'self')
    loader: { '.woff2': 'file', '.woff': 'file', '.ttf': 'file', '.svg': 'text', '.png': 'dataurl', '.wav': 'file' },
  },
  ...lazyBundles.map((bundle) => ({ ...browser, ...bundle })),
]

await rm('dist', { recursive: true, force: true })
await mkdir('dist/renderer', { recursive: true })
await copyFile('src/desktop/renderer/index.html', 'dist/renderer/index.html')

if (watch) {
  // `tauri dev`: değişince yeniden paketlenir; webview'i yenilemek (Ctrl+R) yeterli.
  const contexts = await Promise.all(bundles.map((b) => context(b)))
  await Promise.all(contexts.map((c) => c.watch()))
  console.log('Kor arayüzü izleniyor: dist/renderer/')
} else {
  await Promise.all([...bundles.map((b) => build(b)), buildExtension(production)])
  console.log('Kor arayüzü hazır: dist/renderer/ ve extension/dist/')
}
