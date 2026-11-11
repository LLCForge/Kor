// [g21] Belge önizleme tembel paketleri: pdf.js (+ gömülü çalışan kaynağı), mammoth (DOCX), three.js (GLB).
//      pdf.js çalışanı ayrı dosya olarak DEĞİL, metin olarak pdf paketine gömülür; renderer onu blob: URL'den
//      başlatır. Neden: blob çalışanı belgenin CSP'sini devralır (connect-src 'none'), file:// çalışanı ise
//      başlıksız yüklendiği için CSP'siz kalırdı. CMap/standart yazı tipi dosyaları paketlenmez ve hiçbir yerden
//      çekilmez (kapalı): gömülü yazı tipi olmayan bazı CJK belgelerde glifler eksik çizilebilir.
import { build, type BuildOptions, type Plugin } from 'esbuild'

const silenceImportMeta = { 'empty-import-meta': 'silent' } as const

function pdfWorkerSource(production: boolean): Plugin {
  const define = { 'process.env.NODE_ENV': JSON.stringify(production ? 'production' : 'development') }
  return {
    name: 'g21-pdf-worker-source',
    setup(b) {
      b.onResolve({ filter: /^g21-pdf-worker-source$/ }, (args) => ({ path: args.path, namespace: 'g21-pdf-worker' }))
      b.onLoad({ filter: /.*/, namespace: 'g21-pdf-worker' }, async () => {
        const result = await build({
          entryPoints: ['src/desktop/renderer/g21/pdf-worker-entry.ts'],
          bundle: true,
          write: false,
          minify: true,
          format: 'iife',
          platform: 'browser',
          target: 'chrome130',
          define,
          logLevel: 'warning',
          logOverride: silenceImportMeta,
        })
        const text = result.outputFiles[0]?.text
        if (!text) throw new Error('pdf.js çalışanı derlenemedi')
        return { contents: text, loader: 'text' }
      })
    },
  }
}

export function g21LazyBundles(production: boolean): BuildOptions[] {
  return [
    { entryPoints: ['src/desktop/renderer/g21/pdf-entry.ts'], outfile: 'dist/renderer/g21-pdf.js', plugins: [pdfWorkerSource(production)], logOverride: silenceImportMeta },
    { entryPoints: ['src/desktop/renderer/g21/docx-entry.ts'], outfile: 'dist/renderer/g21-docx.js' },
    { entryPoints: ['src/desktop/renderer/g21/three-entry.ts'], outfile: 'dist/renderer/g21-three.js' },
  ]
}
