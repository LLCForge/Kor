// [g3] Ayrı paket (dist/renderer/nomnoml.js): ```nomnoml bloğu görününce tembel yüklenir. renderSvg saf bir
//      dönüşümdür (yalnızca metin ölçümü için belge kullanır); ağ yoktur. Hatada istisna fırlatır.
import * as nomnoml from 'nomnoml'

declare global {
  interface Window {
    __g3Nomnoml?: { render(code: string): string }
  }
}

window.__g3Nomnoml = {
  render: (code) => nomnoml.renderSvg(code),
}
