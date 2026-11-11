// [g21] Tembel pdf.js paketi (#171-173, #186, #189). window.__g21Pdf üzerinden sunulur.
//      Güvenlik: pdf.js 6.x (CVE-2024-4367 4.2.67'de düzeltildi; 6.x'te `isEvalSupported` seçeneği ve eval/
//      new Function yolu tamamen kaldırıldı; test/g21-pdf-bundle.test.ts bunu kurulu pakette doğrular).
//      Ayrıştırma ayrı bir çalışanda yapılır; çalışan blob: URL'den başlatılır ve sahte (ana iş parçacığı)
//      çalışana geri düşüş yoktur: workerSrc tanımsız olduğundan pdf.js başaramazsa hata verir (fail-closed).
import { getDocument, PDFWorker, TextLayer, version } from 'pdfjs-dist'
import workerSource from 'g21-pdf-worker-source'

let workerUrl: string | null = null

/**
 * Her belge için ayrı, izole bir pdf.js çalışanı. pdf.js dışarıdan verilen portu kendisi sonlandırmaz;
 * `terminate` çalışanı ve iş parçacığını birlikte kapatır.
 */
function createWorker(): { pdfWorker: PDFWorker; terminate: () => void } {
  workerUrl ??= URL.createObjectURL(new Blob([workerSource], { type: 'text/javascript' }))
  const port = new Worker(workerUrl, { name: 'g21-pdf' })
  const pdfWorker = PDFWorker.create({ name: 'g21-pdf', port })
  return {
    pdfWorker,
    terminate: () => {
      pdfWorker.destroy()
      port.terminate()
    },
  }
}

const api = { getDocument, TextLayer, createWorker, version }
export type G21PdfApi = typeof api
;(window as unknown as { __g21Pdf: G21PdfApi }).__g21Pdf = api
