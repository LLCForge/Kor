// [g8] Zengin blok grubunun saf (DOM'suz) veri yardımcıları: tablo ayrıştırma/sıralama/süzme/dışa aktarma,
//      JSON'dan şema çıkarımı ve bağımsız HTML belgesi üretimi. React/DOM kullanmaz; birim test edilir.

/** Tablo satır sayısı üst sınırı (DoS: devasa tablo belleğe alınmaz). */
export const MAX_TABLE_ROWS = 50_000

export interface TableData {
  headers: string[]
  rows: string[][]
}

/** Bir CSV satırını alanlara böler (çift tırnaklı alan, "" kaçışı ve alan içi virgül/yeni satır desteklenir). */
function parseCsv(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let inQuotes = false
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]!
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"'
          i++
        } else inQuotes = false
      } else field += ch
      continue
    }
    if (ch === '"') inQuotes = true
    else if (ch === ',') {
      row.push(field)
      field = ''
    } else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && text[i + 1] === '\n') i++
      row.push(field)
      rows.push(row)
      row = []
      field = ''
    } else field += ch
  }
  if (field !== '' || row.length) {
    row.push(field)
    rows.push(row)
  }
  return rows.filter((r) => r.length > 1 || (r[0] ?? '').trim() !== '')
}

/** Markdown boru tablosunu satırlara böler (ayraç satırı --- atlanır, baş/son boru kırpılır). */
function parsePipe(text: string): string[][] {
  const lines = text.split(/\r?\n/).filter((l) => l.trim() !== '')
  const out: string[][] = []
  for (const line of lines) {
    const trimmed = line.trim().replace(/^\|/, '').replace(/\|$/, '')
    const cells = trimmed.split('|').map((c) => c.trim())
    if (cells.every((c) => /^:?-{2,}:?$/.test(c) || c === '')) continue // ayraç satırı
    out.push(cells)
  }
  return out
}

function normalizeMatrix(matrix: string[][]): TableData {
  if (matrix.length === 0) return { headers: [], rows: [] }
  const headers = (matrix[0] ?? []).map((h) => String(h))
  const width = headers.length
  const rows = matrix.slice(1, MAX_TABLE_ROWS + 1).map((r) => {
    const cells = r.map((c) => String(c))
    while (cells.length < width) cells.push('')
    return cells.slice(0, width)
  })
  return { headers, rows }
}

/**
 * Tablo metnini ayrıştırır. Biçim otomatik algılanır: JSON (nesne/dizi dizisi), Markdown boru tablosu ya da CSV.
 * @throws çok büyük ya da ayrıştırılamayan girdide
 */
export function parseTable(text: string): TableData {
  const trimmed = text.trim()
  if (!trimmed) return { headers: [], rows: [] }
  if (trimmed.startsWith('[') || trimmed.startsWith('{')) {
    const data: unknown = JSON.parse(trimmed)
    const arr = Array.isArray(data) ? data : [data]
    if (arr.length === 0) return { headers: [], rows: [] }
    if (Array.isArray(arr[0])) return normalizeMatrix(arr.map((r) => (r as unknown[]).map((c) => cellToText(c))))
    // Nesne dizisi: başlıklar anahtarların birleşimi
    const headers: string[] = []
    for (const item of arr) {
      if (item && typeof item === 'object') for (const key of Object.keys(item)) if (!headers.includes(key)) headers.push(key)
    }
    if (arr.length > MAX_TABLE_ROWS) throw new Error('Tablo görüntülenemeyecek kadar büyük')
    const rows = arr.map((item) => headers.map((h) => cellToText((item as Record<string, unknown>)?.[h])))
    return { headers, rows }
  }
  const firstLine = trimmed.split(/\r?\n/, 1)[0] ?? ''
  const matrix = firstLine.includes('|') ? parsePipe(trimmed) : parseCsv(trimmed)
  if (matrix.length > MAX_TABLE_ROWS + 1) throw new Error('Tablo görüntülenemeyecek kadar büyük')
  return normalizeMatrix(matrix)
}

function cellToText(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'object') return JSON.stringify(value)
  return String(value)
}

/** Sayıya çevrilebiliyorsa sonlu sayı döndürür (sıralama/süzmede sayısal karşılaştırma için). */
function asNumber(value: string): number | null {
  const trimmed = value.trim()
  if (trimmed === '') return null
  const n = Number(trimmed.replace(/,/g, ''))
  return Number.isFinite(n) ? n : null
}

export type SortDir = 'asc' | 'desc'

/** İki hücreyi karşılaştırır: ikisi de sayıysa sayısal, değilse Türkçe yerel sıralama. */
export function compareCells(a: string, b: string): number {
  const na = asNumber(a)
  const nb = asNumber(b)
  if (na !== null && nb !== null) return na - nb
  return a.localeCompare(b, 'tr')
}

/** Satırları belirtilen sütuna göre kararlı biçimde sıralar (yeni dizi döndürür). */
export function sortRows(rows: string[][], col: number, dir: SortDir): string[][] {
  const factor = dir === 'asc' ? 1 : -1
  return rows
    .map((row, i) => ({ row, i }))
    .sort((x, y) => {
      const cmp = compareCells(x.row[col] ?? '', y.row[col] ?? '')
      return cmp !== 0 ? cmp * factor : x.i - y.i
    })
    .map((x) => x.row)
}

/** Herhangi bir hücrede arama metnini (Türkçe küçük harf) içeren satırları süzer. */
export function filterRows(rows: string[][], query: string): string[][] {
  const q = query.trim().toLocaleLowerCase('tr')
  if (!q) return rows
  return rows.filter((row) => row.some((cell) => cell.toLocaleLowerCase('tr').includes(q)))
}

/** Tek bir CSV alanını gerekiyorsa tırnaklar (virgül, tırnak ya da satır sonu içeriyorsa). */
function csvField(value: string): string {
  return /[",\n\r]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value
}

/** Tablo verisini CSV metnine çevirir (RFC 4180 benzeri kaçış). */
export function toCsv({ headers, rows }: TableData): string {
  const lines = [headers.map(csvField).join(',')]
  for (const row of rows) lines.push(row.map(csvField).join(','))
  return lines.join('\r\n')
}

/** Tablo verisini nesne dizisi JSON metnine çevirir. */
export function toJsonRows({ headers, rows }: TableData): string {
  const objects = rows.map((row) => Object.fromEntries(headers.map((h, i) => [h, row[i] ?? ''])))
  return JSON.stringify(objects, null, 2)
}

// --- JSON şema çıkarımı (#112) -------------------------------------------------

/** Bir JSON değerinden TypeScript benzeri tip adı üretir; iç içe nesne/dizileri özyineler. */
function schemaType(value: unknown, depth: number): string {
  if (depth > 20) return 'unknown'
  if (value === null) return 'null'
  if (Array.isArray(value)) {
    if (value.length === 0) return 'unknown[]'
    const inner = [...new Set(value.map((v) => schemaType(v, depth + 1)))]
    return inner.length === 1 ? `${inner[0]}[]` : `(${inner.join(' | ')})[]`
  }
  const t = typeof value
  if (t === 'object') {
    const entries = Object.entries(value as Record<string, unknown>).map(([k, v]) => `${formatKey(k)}: ${schemaType(v, depth + 1)}`)
    return entries.length ? `{ ${entries.join('; ')} }` : '{}'
  }
  if (t === 'number' || t === 'boolean' || t === 'string') return t
  return 'unknown'
}

/** Nesne anahtarını gerekiyorsa tırnaklar (tanımlayıcı değilse). */
function formatKey(key: string): string {
  return /^[A-Za-z_$][\w$]*$/.test(key) ? key : JSON.stringify(key)
}

/** Örnek JSON değerinden okunabilir, girintili bir tip şeması metni üretir. */
export function inferSchema(value: unknown, rootName = 'Root'): string {
  const body = schemaType(value, 0)
  if (body.startsWith('{')) return `type ${rootName} = ${expandObject(body)}`
  return `type ${rootName} = ${body}`
}

/** Tek satırlık nesne tipini okunabilirlik için girintili çok satıra açar (yalnızca en dış düzey). */
function expandObject(oneLine: string): string {
  // shortcut: yalnızca en dış nesne girintilenir; derin iç içe tipler tek satır kalır (okunabilirlik yeterli).
  const inner = oneLine.slice(1, -1).trim()
  if (!inner) return '{}'
  const parts = splitTopLevel(inner)
  return `{\n${parts.map((p) => `  ${p.trim()}`).join('\n')}\n}`
}

/** Noktalı virgülle ayrılmış alanları, iç içe { } ve ( ) derinliğini sayarak en üst düzeyde böler. */
function splitTopLevel(text: string): string[] {
  const parts: string[] = []
  let depth = 0
  let current = ''
  for (const ch of text) {
    if (ch === '{' || ch === '(' || ch === '[') depth++
    else if (ch === '}' || ch === ')' || ch === ']') depth--
    if (ch === ';' && depth === 0) {
      if (current.trim()) parts.push(current)
      current = ''
    } else current += ch
  }
  if (current.trim()) parts.push(current)
  return parts
}

// --- Bağımsız HTML dışa aktarma (#120) ----------------------------------------

/** Metni HTML bağlamına güvenle gömmek için kaçırır (XSS önleme; React dışı statik çıktı). */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] ?? c)
}

/**
 * Gövde işaretlemesini kendi kendine yeten bir HTML belgesine sarar (artifact'i dosyaya aktarma #120).
 * Başlık kaçırılır; gövde zaten kaçırılmış/güvenli HTML olmalıdır. Dış kaynak ya da betik gömülmez.
 */
export function buildStandaloneHtml(title: string, bodyHtml: string): string {
  const style =
    'body{font:15px/1.6 system-ui,sans-serif;margin:2rem auto;max-width:960px;padding:0 1rem;color:#1a1a18;background:#fff}' +
    'table{border-collapse:collapse;width:100%}th,td{border:1px solid #ddd;padding:6px 10px;text-align:left}' +
    'th{background:#f3f0e8}h1{font-size:1.4rem}pre{background:#f6f6f4;padding:1rem;border-radius:8px;overflow:auto}'
  return (
    '<!doctype html><html lang="tr"><head><meta charset="utf-8">' +
    '<meta name="viewport" content="width=device-width, initial-scale=1">' +
    `<title>${escapeHtml(title)}</title><style>${style}</style></head>` +
    `<body><h1>${escapeHtml(title)}</h1>${bodyHtml}</body></html>`
  )
}

/** Tablo verisini bağımsız HTML belgesinin gövdesi olan statik bir <table> işaretlemesine çevirir. */
export function tableToHtml({ headers, rows }: TableData): string {
  const head = headers.map((h) => `<th>${escapeHtml(h)}</th>`).join('')
  const body = rows.map((row) => `<tr>${row.map((c) => `<td>${escapeHtml(c)}</td>`).join('')}</tr>`).join('')
  return `<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`
}
