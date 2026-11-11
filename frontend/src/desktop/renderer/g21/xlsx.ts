// [g21] Bağımlılıksız, salt okunur XLSX okuyucu (#175). npm'deki xlsx@0.18.5 bilinen açıklar taşıdığı
//      (CVE-2023-30533 prototype pollution, CVE-2024-22363 ReDoS) ve exceljs 2023'ten beri yayımlanmadığı/ağır
//      Node bağımlılıkları çektiği için kullanılmadı. Formül çalıştırılmaz (önbellekteki değer gösterilir),
//      makrolar okunmaz. Ayrıştırma doğrusal XML tarayıcısıyla yapılır; hücreler düz metin olarak döner.
import { localName, scanXml, collectElements } from './xml.ts'
import { findEntry, listZip, readZipText, resolveRelative, type ZipEntryInfo } from './zip.ts'

export const MAX_SHEET_ROWS = 50_000
export const MAX_SHEET_COLS = 512

export interface SheetRef {
  name: string
  path: string
  hidden: boolean
}

export interface Workbook {
  bytes: Uint8Array
  entries: ZipEntryInfo[]
  sheets: SheetRef[]
  shared: string[]
  dateStyles: ReadonlySet<number>
  date1904: boolean
}

export interface SheetData {
  rows: string[][]
  truncated: boolean
}

// Yerleşik tarih/saat sayı biçimi kimlikleri (ECMA-376 18.8.30).
const BUILTIN_DATE_FORMATS = new Set([14, 15, 16, 17, 18, 19, 20, 21, 22, 27, 30, 36, 45, 46, 47, 50, 57])

/** Özel biçim kodunun tarih/saat olup olmadığı (tırnaklı metin, kaçışlar ve [renk] blokları yok sayılır). */
export function isDateFormat(code: string): boolean {
  const stripped = code.replace(/"[^"]*"/g, '').replace(/\\./g, '').replace(/\[[^\]]*\]/g, '')
  return /[dmyhs]/i.test(stripped) && !/^general$/i.test(stripped.trim())
}

/** Sütun harflerini 0 tabanlı dizine çevirir: A -> 0, AA -> 26. */
export function columnIndex(letters: string): number {
  let n = 0
  for (const ch of letters.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64)
  return n - 1
}

/** Excel seri tarihini ISO metnine çevirir (saat varsa "YYYY-MM-DD HH:MM[:SS]"). */
export function serialToIso(serial: number, date1904: boolean): string {
  const epoch = date1904 ? Date.UTC(1904, 0, 1) : Date.UTC(1899, 11, 30)
  const ms = Math.round(serial * 86_400_000)
  const d = new Date(epoch + ms)
  if (Number.isNaN(d.getTime())) return String(serial)
  const iso = d.toISOString()
  const time = iso.slice(11, 19)
  if (time === '00:00:00') return iso.slice(0, 10)
  return `${iso.slice(0, 10)} ${time.endsWith(':00') ? time.slice(0, 5) : time}`
}

function sharedStrings(xml: string): string[] {
  const out: string[] = []
  let inSi = false
  let inT = false
  let phonetic = 0
  let current = ''
  scanXml(xml, {
    open: (tag, _a, selfClosing) => {
      const name = localName(tag)
      if (name === 'si') {
        inSi = true
        current = ''
      } else if (name === 'rPh' && !selfClosing) phonetic++
      else if (name === 't' && inSi && !selfClosing) inT = true
    },
    close: (tag) => {
      const name = localName(tag)
      if (name === 'si') {
        out.push(current)
        inSi = false
      } else if (name === 'rPh') phonetic = Math.max(0, phonetic - 1)
      else if (name === 't') inT = false
    },
    text: (t) => {
      if (inSi && inT && phonetic === 0) current += t
    },
  })
  return out
}

function dateStyles(xml: string): Set<number> {
  const custom = new Map<number, string>()
  for (const fmt of collectElements(xml, 'numFmt')) custom.set(Number(fmt.numFmtId), fmt.formatCode ?? '')
  const result = new Set<number>()
  let inCellXfs = false
  let index = 0
  scanXml(xml, {
    open: (tag, attrs) => {
      const name = localName(tag)
      if (name === 'cellXfs') {
        inCellXfs = true
        index = 0
      } else if (name === 'xf' && inCellXfs) {
        const id = Number(attrs.numFmtId ?? 0)
        const code = custom.get(id)
        if (BUILTIN_DATE_FORMATS.has(id) || (code !== undefined && isDateFormat(code))) result.add(index)
        index++
      }
    },
    close: (tag) => {
      if (localName(tag) === 'cellXfs') inCellXfs = false
    },
  })
  return result
}

async function optionalText(bytes: Uint8Array, entries: readonly ZipEntryInfo[], name: string): Promise<string | null> {
  const entry = findEntry(entries, name)
  return entry ? readZipText(bytes, entry) : null
}

/** Çalışma kitabını açar: sayfa listesi, paylaşılan metinler ve tarih stilleri. Sayfalar ayrıca okunur. */
export async function openWorkbook(bytes: Uint8Array): Promise<Workbook> {
  const entries = listZip(bytes)
  const workbookXml = await optionalText(bytes, entries, 'xl/workbook.xml')
  if (workbookXml === null) throw new Error('Geçerli bir XLSX dosyası değil (xl/workbook.xml yok)')
  const relsXml = (await optionalText(bytes, entries, 'xl/_rels/workbook.xml.rels')) ?? ''
  const targets = new Map<string, string>()
  for (const rel of collectElements(relsXml, 'Relationship')) {
    if (rel.Id && rel.Target && rel.TargetMode !== 'External') targets.set(rel.Id, rel.Target)
  }
  const sheets: SheetRef[] = []
  for (const sheet of collectElements(workbookXml, 'sheet')) {
    const rid = sheet['r:id'] ?? Object.entries(sheet).find(([k]) => localName(k) === 'id' && k !== 'sheetId')?.[1]
    const target = rid ? targets.get(rid) : undefined
    if (!target) continue
    try {
      const path = target.startsWith('/') ? target.slice(1) : resolveRelative('xl/workbook.xml', target)
      sheets.push({ name: sheet.name ?? `Sayfa ${sheets.length + 1}`, path, hidden: sheet.state === 'hidden' || sheet.state === 'veryHidden' })
    } catch (err) {
      console.warn('XLSX sayfa yolu atlandı:', err)
    }
  }
  if (sheets.length === 0) throw new Error('Çalışma kitabında okunabilir sayfa bulunamadı')
  const pr = collectElements(workbookXml, 'workbookPr')[0]
  const shared = sharedStrings((await optionalText(bytes, entries, 'xl/sharedStrings.xml')) ?? '')
  const styles = dateStyles((await optionalText(bytes, entries, 'xl/styles.xml')) ?? '')
  return { bytes, entries, sheets, shared, dateStyles: styles, date1904: pr?.date1904 === '1' || pr?.date1904 === 'true' }
}

/** Sayfa XML'ini satır matrisine çevirir (saf; boş hücreler '' olur, sağdaki boş sütunlar kırpılır). */
export function parseSheetXml(xml: string, ctx: Pick<Workbook, 'shared' | 'dateStyles' | 'date1904'>, maxRows = MAX_SHEET_ROWS): SheetData {
  const rows: string[][] = []
  let truncated = false
  let rowIndex = -1
  let cell: { col: number; type: string; style: number } | null = null
  let capture: 'v' | 't' | null = null
  let value = ''
  let inline = ''
  let nextCol = 0

  scanXml(xml, {
    open: (tag, attrs, selfClosing) => {
      if (truncated) return
      const name = localName(tag)
      if (name === 'row') {
        const r = Number(attrs.r)
        rowIndex = Number.isInteger(r) && r > 0 ? r - 1 : rowIndex + 1
        nextCol = 0
        if (rowIndex >= maxRows) truncated = true
      } else if (name === 'c') {
        const ref = /^([A-Za-z]{1,3})(\d*)$/.exec(attrs.r ?? '')
        const col = ref ? columnIndex(ref[1] ?? 'A') : nextCol
        nextCol = col + 1
        cell = selfClosing ? null : { col, type: attrs.t ?? 'n', style: Number(attrs.s ?? 0) }
        value = ''
        inline = ''
      } else if (cell && !selfClosing && (name === 'v' || name === 't')) capture = name
    },
    close: (tag) => {
      const name = localName(tag)
      if (name === 'v' || name === 't') capture = null
      else if (name === 'c' && cell && !truncated) {
        const { col, type, style } = cell
        cell = null
        if (col >= MAX_SHEET_COLS || rowIndex < 0) return
        let text: string
        if (type === 's') text = ctx.shared[Number(value)] ?? ''
        else if (type === 'inlineStr') text = inline
        else if (type === 'b') text = value === '1' ? 'TRUE' : 'FALSE'
        else if (type === 'str' || type === 'e' || type === 'd') text = value
        else {
          const num = Number(value)
          text = value !== '' && Number.isFinite(num) && ctx.dateStyles.has(style) ? serialToIso(num, ctx.date1904) : value
        }
        if (text === '') return
        while (rows.length <= rowIndex) rows.push([])
        const row = rows[rowIndex] ?? []
        while (row.length < col) row.push('')
        row[col] = text
        rows[rowIndex] = row
      }
    },
    text: (t) => {
      if (capture === 'v') value += t
      else if (capture === 't') inline += t
    },
  })
  const width = rows.reduce((w, r) => Math.max(w, r.length), 0)
  return { rows: rows.map((r) => (r.length < width ? [...r, ...Array<string>(width - r.length).fill('')] : r)), truncated }
}

/** Bir sayfayı okur. */
export async function readSheet(wb: Workbook, index: number): Promise<SheetData> {
  const ref = wb.sheets[index]
  if (!ref) throw new Error('Sayfa bulunamadı')
  const entry = findEntry(wb.entries, ref.path)
  if (!entry) throw new Error(`Sayfa dosyası arşivde yok: ${ref.path}`)
  return parseSheetXml(await readZipText(wb.bytes, entry), wb)
}
