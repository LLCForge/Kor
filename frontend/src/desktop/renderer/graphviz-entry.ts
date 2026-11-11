// [g3] Ayrı paket (dist/renderer/graphviz.js): ```dot / ```graphviz bloğu görününce tembel yüklenir.
//      @viz-js/viz WASM'ı JS içine gömülüdür; render tamamen yereldir, ağ isteği yoktur (offline doğrulandı).
import { instance } from '@viz-js/viz'

declare global {
  interface Window {
    __g3Graphviz?: { render(code: string): Promise<string> }
  }
}

let vizPromise: ReturnType<typeof instance> | null = null

window.__g3Graphviz = {
  async render(code) {
    vizPromise ??= instance()
    const viz = await vizPromise
    const result = viz.render(code, { format: 'svg' })
    if (result.status !== 'success') {
      const msg = result.errors.map((e) => e.message).filter(Boolean).join('\n') || 'Graphviz çizimi başarısız.'
      throw new Error(msg)
    }
    // XML bildirimi/doctype ön ekini at; temizleyiciye doğrudan <svg> gitsin.
    const svgStart = result.output.indexOf('<svg')
    return svgStart >= 0 ? result.output.slice(svgStart) : result.output
  },
}
