// [g21] Tembel mammoth paketi (#174): DOCX -> HTML dönüşümü tamamen renderer'da, bellekte. Dış dosya erişimi
//      kapalı (externalFileAccess: false), görseller data: URI. Çıktı GÜVENİLMEZDİR: çağıran DOMPurify'dan geçirir.
import mammoth from 'mammoth'

const api = {
  async toHtml(data: ArrayBuffer): Promise<{ html: string; warnings: string[] }> {
    const result = await mammoth.convertToHtml({ arrayBuffer: data }, { externalFileAccess: false })
    return { html: result.value, warnings: result.messages.map((m) => m.message) }
  },
}
export type G21DocxApi = typeof api
;(window as unknown as { __g21Docx: G21DocxApi }).__g21Docx = api
