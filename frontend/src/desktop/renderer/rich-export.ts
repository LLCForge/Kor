// [temel] Zengin içerik dışa aktarma yardımcıları (yalnızca bellek + tarayıcı indirmesi; diske yazma grupların işi).
//         Ana belge CSP'si img-src 'self' data: olduğundan görseller data: URL ile oluşturulur (blob: yasak).

/** Varsayılan PNG ölçek çarpanı (keskin çıktı için). */
export const PNG_SCALE = 2

/** data: URL render'ı için SVG'nin xmlns'i olmalı; yoksa eklenir. */
function withXmlns(svg: string): string {
  return /\bxmlns=/.test(svg) ? svg : svg.replace(/<svg\b/, '<svg xmlns="http://www.w3.org/2000/svg"')
}

/** Temizlenmiş bir SVG'yi PNG blob'una çevirir. SVG dış referans içermemelidir (canvas kirlenmesini önler). */
export function svgToPng(svg: string, scale = PNG_SCALE): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => {
      const width = Math.max(1, Math.round((img.naturalWidth || img.width || 1) * scale))
      const height = Math.max(1, Math.round((img.naturalHeight || img.height || 1) * scale))
      const canvas = document.createElement('canvas')
      canvas.width = width
      canvas.height = height
      const ctx = canvas.getContext('2d')
      if (!ctx) {
        reject(new Error('Canvas bağlamı alınamadı'))
        return
      }
      ctx.drawImage(img, 0, 0, width, height)
      canvasToPng(canvas).then(resolve, reject)
    }
    img.onerror = () => reject(new Error('SVG görsel olarak yüklenemedi'))
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(withXmlns(svg))}`
  })
}

/** Bir <canvas> içeriğini PNG blob'una çevirir. */
export function canvasToPng(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('PNG oluşturulamadı'))), 'image/png'))
}

// shortcut: yalnızca <canvas> ya da <svg> olan/içeren elemanlar desteklenir. Genel HTML rasterleştirme
//           (foreignObject tekniği) güvenilmez ve CSP ile çakışır; bir grup gerçekten gerektiğinde bu yolu genişletir.
/** Bir elemanı PNG blob'una çevirir: doğrudan canvas, içindeki canvas, ya da içindeki/olan SVG. */
export function elementToPng(el: Element, scale = PNG_SCALE): Promise<Blob> {
  if (el instanceof HTMLCanvasElement) return canvasToPng(el)
  const canvas = el.querySelector('canvas')
  if (canvas) return canvasToPng(canvas)
  const svg = el instanceof SVGSVGElement ? el : el.querySelector('svg')
  if (svg) return svgToPng(new XMLSerializer().serializeToString(svg), scale)
  return Promise.reject(new Error('Bu eleman PNG olarak dışa aktarılamıyor (SVG veya canvas gerekir)'))
}

/** Metni (CSV/JSON vb.) belirtilen MIME ile indirme olarak kaydeder (ağ yok; bellek içi blob). */
export function downloadText(text: string, filename: string, mime = 'text/plain'): void {
  downloadBlob(new Blob([text], { type: `${mime};charset=utf-8` }), filename)
}

/** Bir blob'u kullanıcının indirmesi olarak kaydeder (ağ yok; yerel nesne URL'i kullanılır). */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.rel = 'noopener'
  a.click()
  // İndirme başladıktan sonra nesne URL'i serbest bırakılır.
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}
