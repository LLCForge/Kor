// [g3] Ayrı paket (dist/renderer/wavedrom.js): ```wavedrom bloğu görününce tembel yüklenir. DOM'suz yol:
//      renderAny + onml.stringify ile SVG metni üretilir (document/window'a dokunmaz). Ağ yoktur.
//      Kaynak JSON5 olabilir (tırnaksız anahtarlar); önce JSON5 ile ayrıştırılır.
import { parse as parseJson5 } from 'json5'
import { onml, renderAny, waveSkin } from 'wavedrom'

declare global {
  interface Window {
    __g3Wavedrom?: { render(code: string): string }
  }
}

window.__g3Wavedrom = {
  render(code) {
    const source = parseJson5(code) as Record<string, unknown>
    const tree = renderAny(0, source, waveSkin)
    if (!Array.isArray(tree) || tree[0] !== 'svg') {
      throw new Error('WaveDrom kaynağı bir signal/assign/reg alanı içermelidir.')
    }
    return onml.stringify(tree)
  },
}
