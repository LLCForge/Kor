import { getDocumentProxy } from 'unpdf'

export const MAX_PDF_BYTES = 30 * 1024 * 1024
export const MAX_PDF_PAGES_PER_CALL = 20

/** "3", "1-5", "1,3,7-9" biçimindeki sayfa aralığını 1 tabanlı sıralı sayfa listesine çevirir. */
export function parsePageRange(spec: string, totalPages: number): number[] {
  const pages = new Set<number>()
  for (const part of spec.split(',').map((p) => p.trim()).filter(Boolean)) {
    const m = /^(\d+)(?:\s*-\s*(\d+))?$/.exec(part)
    if (!m) throw new Error(`Geçersiz sayfa aralığı: "${part}" (ör. "3", "1-5", "1,3,7-9")`)
    const start = Number(m[1])
    const end = m[2] === undefined ? start : Number(m[2])
    if (start < 1 || end < start) throw new Error(`Geçersiz sayfa aralığı: "${part}"`)
    if (start > totalPages) throw new Error(`Sayfa ${start} yok; belge ${totalPages} sayfa`)
    for (let p = start; p <= Math.min(end, totalPages); p++) {
      pages.add(p)
      if (pages.size > MAX_PDF_PAGES_PER_CALL) throw new Error(`Tek çağrıda en fazla ${MAX_PDF_PAGES_PER_CALL} sayfa okunabilir`)
    }
  }
  if (pages.size === 0) throw new Error('Sayfa aralığı boş')
  return [...pages].sort((a, b) => a - b)
}

export interface PdfText {
  totalPages: number
  pages: Array<{ page: number; text: string }>
}

/** Seçilen sayfaların metnini çıkarır; pages verilmezse ilk 20 sayfa okunur. */
export async function extractPdfText(data: Uint8Array, pageSpec?: string): Promise<PdfText> {
  let pdf
  try {
    pdf = await getDocumentProxy(data)
  } catch (err) {
    throw new Error(`PDF açılamadı (bozuk veya şifreli olabilir): ${err instanceof Error ? err.message : String(err)}`)
  }
  try {
    const totalPages = pdf.numPages
    const wanted = pageSpec
      ? parsePageRange(pageSpec, totalPages)
      : Array.from({ length: Math.min(totalPages, MAX_PDF_PAGES_PER_CALL) }, (_, i) => i + 1)
    const pages: PdfText['pages'] = []
    for (const n of wanted) {
      const content = await (await pdf.getPage(n)).getTextContent()
      let text = ''
      for (const item of content.items) {
        if ('str' in item) text += item.str + (item.hasEOL ? '\n' : '')
      }
      pages.push({ page: n, text: text.replace(/[ \t]+\n/g, '\n').trim() })
    }
    return { totalPages, pages }
  } finally {
    await pdf.loadingTask.destroy()
  }
}
