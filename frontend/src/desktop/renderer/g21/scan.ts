// [g21] Önizleme öncesi güvenlik taraması (#190). Bulgular kullanıcıya rozetle bildirilir; asıl koruma ayrı
//      katmanlardadır: PDF'te betik motoru ve bağlantı katmanı hiç yüklenmez, HTML/SVG DOMPurify'dan geçer,
//      ağ CSP ile kapalıdır. Bu tarama "ne etkisiz bırakıldı" sorusunu yanıtlar. Saf; DOM yok.

export interface ScanFinding {
  label: string
  count: number
  /** warn: etkin içerik (betik, makro, otomatik eylem); info: pasif dış referans */
  severity: 'info' | 'warn'
}

function tally(text: string, rules: ReadonlyArray<readonly [string, RegExp, ScanFinding['severity']]>): ScanFinding[] {
  const out: ScanFinding[] = []
  for (const [label, re, severity] of rules) {
    const count = text.match(re)?.length ?? 0
    if (count > 0) out.push({ label, count, severity })
  }
  return out
}

const PDF_RULES = [
  ['JavaScript eylemi', /\/(JavaScript|JS)\b/g, 'warn'],
  ['Açılışta otomatik eylem', /\/OpenAction\b/g, 'warn'],
  ['Ek eylem tetikleyicisi', /\/AA\b/g, 'warn'],
  ['Program başlatma', /\/Launch\b/g, 'warn'],
  ['Gömülü dosya', /\/EmbeddedFiles?\b/g, 'warn'],
  ['Form gönderimi', /\/SubmitForm\b/g, 'warn'],
  ['XFA formu', /\/XFA\b/g, 'warn'],
  ['Zengin medya', /\/RichMedia\b/g, 'warn'],
  ['Dış belge bağlantısı', /\/GoToR\b/g, 'info'],
  ['Web bağlantısı', /\/URI\b/g, 'info'],
] as const

/**
 * PDF ham baytlarında etkin içerik işaretlerini sayar. En iyi çaba: sıkıştırılmış nesne akışlarındaki
 * işaretler görülmeyebilir; bu yüzden görüntüleyici bunları zaten hiç çalıştırmaz.
 */
export function scanPdf(bytes: Uint8Array): ScanFinding[] {
  const limit = Math.min(bytes.length, 64 * 1024 * 1024)
  let text = ''
  const decoder = new TextDecoder('latin1')
  for (let i = 0; i < limit; i += 1 << 20) text += decoder.decode(bytes.subarray(i, Math.min(limit, i + (1 << 20))))
  return tally(text, PDF_RULES)
}

const MARKUP_RULES = [
  ['Betik (script)', /<script\b/gi, 'warn'],
  ['Olay işleyicisi (on...)', /\son[a-z]+\s*=/gi, 'warn'],
  ['javascript: bağlantısı', /javascript\s*:/gi, 'warn'],
  ['Gömülü çerçeve/nesne', /<(iframe|frame|object|embed|applet)\b/gi, 'warn'],
  ['Form', /<form\b/gi, 'warn'],
  ['Meta yönlendirme', /<meta[^>]+http-equiv\s*=\s*["']?refresh/gi, 'warn'],
  ['Dış kaynak referansı', /\b(src|href|xlink:href|poster|background|action)\s*=\s*["']?\s*(https?:)?\/\//gi, 'info'],
  ['Dış stil (url/@import)', /(url\(\s*["']?\s*(https?:)?\/\/|@import\b)/gi, 'info'],
  ['Stil sayfası bağlantısı', /<link\b/gi, 'info'],
] as const

/** HTML/XHTML/SVG/Markdown metnindeki etkin içerik ve dış referansları sayar. */
export function scanMarkup(text: string): ScanFinding[] {
  return tally(text.slice(0, 16 * 1024 * 1024), MARKUP_RULES)
}

/**
 * Office Open XML (DOCX/XLSX) paket taraması: makrolar, ActiveX, OLE nesneleri ve dış ilişkiler.
 * @param names arşivdeki giriş adları
 * @param relsXml tüm .rels dosyalarının metni
 */
export function scanOfficePackage(names: readonly string[], relsXml: readonly string[]): ScanFinding[] {
  const out: ScanFinding[] = []
  const add = (label: string, count: number, severity: ScanFinding['severity']) => {
    if (count > 0) out.push({ label, count, severity })
  }
  add('VBA makro projesi', names.filter((n) => /vbaProject\.bin$/i.test(n)).length, 'warn')
  add('ActiveX denetimi', names.filter((n) => /\/activeX\//i.test(n)).length, 'warn')
  add('Gömülü OLE nesnesi', names.filter((n) => /\/embeddings\//i.test(n)).length, 'warn')
  const external = relsXml.reduce((s, x) => s + (x.match(/TargetMode\s*=\s*["']External["']/g)?.length ?? 0), 0)
  add('Dış ilişki (bağlantı/şablon/görsel)', external, 'info')
  return out
}

/** Bulgu listesinin tek satırlık özeti. */
export function summarizeFindings(findings: readonly ScanFinding[]): string {
  if (findings.length === 0) return 'Etkin içerik veya dış referans bulunmadı'
  return findings.map((f) => `${f.label}: ${f.count}`).join(' · ')
}
