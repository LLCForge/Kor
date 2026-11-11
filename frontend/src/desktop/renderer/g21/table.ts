// [g21] Tablo önizleme mantığı (#175-177): ayraç algılayan CSV/TSV ayrıştırma, sütun tipi algılama
//      (sayı/tarih/metin), tipe göre sıralama ve tablodan ```chart bloğu (CSV gövdeli) üretme. Saf; DOM yok.

export const MAX_TABLE_ROWS = 50_000
export const MAX_CHART_ROWS = 500
export const MAX_CHART_SERIES = 6

export type ColumnType = 'number' | 'date' | 'text' | 'empty'
export type Delimiter = ',' | ';' | '\t' | '|'

/** İlk satırlarda (tırnak dışında) en tutarlı görünen ayracı seçer. */
export function detectDelimiter(text: string): Delimiter {
  const sample = text.slice(0, 20_000).split(/\r?\n/).filter((l) => l.trim() !== '').slice(0, 20)
  let best: Delimiter = ','
  let bestScore = 0
  for (const d of [',', ';', '\t', '|'] as const) {
    const counts = sample.map((line) => line.replace(/"[^"]*"/g, '').split(d).length - 1)
    const first = counts[0] ?? 0
    if (first === 0) continue
    // Satırların çoğunda aynı sayıda ayraç olması, ayracın gerçek olduğunu gösterir.
    const consistent = counts.filter((c) => c === first).length / counts.length
    const score = first * consistent
    if (score > bestScore) {
      best = d
      bestScore = score
    }
  }
  return best
}

/** RFC 4180 benzeri ayrıştırma (tırnaklı alan, "" kaçışı, alan içi ayraç/yeni satır). */
export function parseDelimited(text: string, delimiter: Delimiter, maxRows = MAX_TABLE_ROWS + 1): { rows: string[][]; truncated: boolean } {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let inQuotes = false
  const src = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text
  for (let i = 0; i < src.length; i++) {
    const ch = src[i]
    if (inQuotes) {
      if (ch === '"') {
        if (src[i + 1] === '"') {
          field += '"'
          i++
        } else inQuotes = false
      } else field += ch
      continue
    }
    if (ch === '"' && field === '') inQuotes = true
    else if (ch === delimiter) {
      row.push(field)
      field = ''
    } else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && src[i + 1] === '\n') i++
      row.push(field)
      if (row.length > 1 || (row[0] ?? '').trim() !== '') rows.push(row)
      row = []
      field = ''
      if (rows.length >= maxRows) return { rows, truncated: i < src.length - 1 }
    } else field += ch
  }
  if (field !== '' || row.length > 0) {
    row.push(field)
    if (row.length > 1 || (row[0] ?? '').trim() !== '') rows.push(row)
  }
  return { rows, truncated: false }
}

/** Satırları başlık + eşit genişlikli gövdeye çevirir. */
export function normalizeTable(matrix: string[][]): { headers: string[]; rows: string[][] } {
  const width = matrix.reduce((w, r) => Math.max(w, r.length), 0)
  const pad = (r: string[]) => (r.length < width ? [...r, ...Array<string>(width - r.length).fill('')] : r)
  const headerRow = pad(matrix[0] ?? [])
  const headers = headerRow.map((h, i) => h.trim() || `Sütun ${i + 1}`)
  return { headers, rows: matrix.slice(1).map(pad) }
}

/**
 * Hücreyi sayıya çevirir. "1,234.5", "1.234,5", "%12", "12 %", "₺1.200", "-3e2" desteklenir. Tek ayraçlı
 * belirsiz durumda (ör. "1,5") ayraç ondalık kabul edilir; "1,234,567" gibi düzenli gruplar binlik sayılır.
 */
export function parseNumberCell(raw: string): number | null {
  let s = raw.trim().replace(/[\s ]/g, '')
  if (!s || s.length > 40) return null
  s = s.replace(/^[$€£₺¥]|[$€£₺¥]$/g, '').replace(/^%|%$/g, '')
  if (!/^[-+]?(\d|[.,]\d)[\d.,]*(e[-+]?\d+)?$/i.test(s)) return null
  const lastDot = s.lastIndexOf('.')
  const lastComma = s.lastIndexOf(',')
  if (lastDot >= 0 && lastComma >= 0) {
    s = lastComma > lastDot ? s.replace(/\./g, '').replace(',', '.') : s.replace(/,/g, '')
  } else if (lastComma >= 0) {
    s = /^[-+]?\d{1,3}(,\d{3})+$/.test(s) ? s.replace(/,/g, '') : s.split(',').length === 2 ? s.replace(',', '.') : ''
  } else if (lastDot >= 0 && /^[-+]?\d{1,3}(\.\d{3}){2,}$/.test(s)) {
    s = s.replace(/\./g, '')
  }
  if (!s) return null
  const n = Number(s)
  return Number.isFinite(n) ? n : null
}

/** Hücreyi tarihe (UTC ms) çevirir: ISO (YYYY-MM-DD[ T]HH:MM[:SS]) ve GG.AA.YYYY / GG/AA/YYYY. */
export function parseDateCell(raw: string): number | null {
  const s = raw.trim()
  let y: number, mo: number, d: number
  let h = 0, mi = 0, se = 0
  let m = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?$/.exec(s)
  if (m) {
    ;[y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])]
    if (m[4]) [h, mi, se] = [Number(m[4]), Number(m[5]), Number(m[6] ?? 0)]
  } else {
    m = /^(\d{1,2})[./](\d{1,2})[./](\d{4})(?: (\d{1,2}):(\d{2})(?::(\d{2}))?)?$/.exec(s)
    if (!m) return null
    ;[d, mo, y] = [Number(m[1]), Number(m[2]), Number(m[3])]
    if (m[4]) [h, mi, se] = [Number(m[4]), Number(m[5]), Number(m[6] ?? 0)]
  }
  if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59 || se > 59) return null
  const t = Date.UTC(y, mo - 1, d, h, mi, se)
  // 31.02 gibi taşan günler reddedilir.
  return new Date(t).getUTCDate() === d ? t : null
}

/** Örneklenen satırlarda boş olmayan hücrelerin en az %90'ı aynı tipe uyuyorsa sütun o tiptir. */
export function inferColumnTypes(headers: readonly string[], rows: readonly string[][], sample = 1000): ColumnType[] {
  const body = rows.slice(0, sample)
  return headers.map((_, col) => {
    const values = body.map((r) => (r[col] ?? '').trim()).filter((v) => v !== '')
    if (values.length === 0) return 'empty'
    const need = Math.ceil(values.length * 0.9)
    if (values.filter((v) => parseNumberCell(v) !== null).length >= need) return 'number'
    if (values.filter((v) => parseDateCell(v) !== null).length >= need) return 'date'
    return 'text'
  })
}

/** Tipe göre karşılaştırma; ayrıştırılamayan/boş hücreler her zaman sona. */
export function compareTyped(a: string, b: string, type: ColumnType): number {
  if (type === 'number' || type === 'date') {
    const parse = type === 'number' ? parseNumberCell : parseDateCell
    const x = parse(a)
    const y = parse(b)
    if (x === null || y === null) return x === null ? (y === null ? 0 : 1) : -1
    return x - y
  }
  return a.localeCompare(b, 'tr', { numeric: true })
}

function csvField(value: string): string {
  return /[",\n\r]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value
}

/**
 * Tablodan ```chart bloğu gövdesi üretir (CSV: ilk sütun etiket, ardından sayısal seriler; ondalık nokta).
 * Etiket sütunu ilk metin/tarih sütunudur; yoksa satır numarası kullanılır. Sayısal sütun yoksa null.
 */
export function toChartCsv(headers: readonly string[], rows: readonly string[][], types: readonly ColumnType[]): string | null {
  const numeric = types.flatMap((t, i) => (t === 'number' ? [i] : [])).slice(0, MAX_CHART_SERIES)
  if (numeric.length === 0) return null
  const labelCol = types.findIndex((t) => t === 'text' || t === 'date')
  const lines = [[labelCol >= 0 ? (headers[labelCol] ?? 'Etiket') : '#', ...numeric.map((c) => headers[c] ?? '')].map(csvField).join(',')]
  rows.slice(0, MAX_CHART_ROWS).forEach((row, i) => {
    const label = labelCol >= 0 ? (row[labelCol] ?? '') : String(i + 1)
    const values = numeric.map((c) => {
      const n = parseNumberCell(row[c] ?? '')
      return n === null ? '' : String(n)
    })
    lines.push([label, ...values].map(csvField).join(','))
  })
  return lines.join('\n')
}
