// [perf] Ayrı paket (dist/renderer/hljs.js): sözdizimi vurgusu yalnızca ilk diff/kod görünümünde yüklenir.
// Ana renderer paketinden ~1 MB çıkar; yüklenene kadar satırlar düz metin çizilir.
import hljs from 'highlight.js/lib/common'

;(window as { __korHljs?: typeof hljs }).__korHljs = hljs
