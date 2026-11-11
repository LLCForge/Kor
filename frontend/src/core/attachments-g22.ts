// [g22] Modele özgü ekler: model destekliyorsa yerel parça (ses, PDF), desteklemiyorsa metne dönüştürme (PDF, DOCX, XLSX).
// ZIP ve XML ayrıştırma bağımlılıksızdır (npm'deki xlsx@0.18.5 açıklı); sınırlar zip bombasına ve dev belgelere karşı.
import { randomUUID } from 'node:crypto'
import { readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { inflateRawSync } from 'node:zlib'
import type { ChatCompletionContentPart, ChatCompletionMessageParam } from 'openai/resources/chat/completions'
import type { ModelCapabilities } from './model-capabilities.ts'
import { extractPdfText, MAX_PDF_BYTES } from './pdf.ts'

export type AttachmentKind = 'image' | 'pdf' | 'docx' | 'xlsx' | 'audio' | 'opk' | 'other'
/** native: modele dosya/ses parçası olarak; text: metne dönüştürülerek (other: olduğu gibi metin); unsupported: gönderilemez */
export type AttachmentPlan = 'native' | 'text' | 'unsupported'

export const MAX_CONVERTED_CHARS = 200_000
export const MAX_AUDIO_BYTES = 20 * 1024 * 1024
const MAX_ZIP_ENTRIES = 5_000
const MAX_ZIP_ENTRY_BYTES = 50 * 1024 * 1024
const MAX_SHEETS = 10
const MAX_SHEET_ROWS = 2_000
const MAX_SHEET_COLS = 200
const MAX_PENDING_PARTS = 16

// Venice input_audio biçimleri (docs.venice.ai, chat/completions).
const AUDIO_FORMATS: Readonly<Record<string, string>> = { wav: 'wav', mp3: 'mp3', ogg: 'ogg', flac: 'flac', m4a: 'm4a', aac: 'aac', aiff: 'aiff', aif: 'aiff' }

export function attachmentKind(relPath: string): AttachmentKind {
  const ext = path.extname(relPath).slice(1).toLowerCase()
  if (/^(png|jpe?g|gif|webp)$/.test(ext)) return 'image'
  if (ext === 'pdf' || ext === 'docx' || ext === 'xlsx') return ext
  if (ext === 'opk') return 'opk' // OpenBullet 2 config paketi (ZIP: LoliCode + metadata)
  return ext in AUDIO_FORMATS ? 'audio' : 'other'
}

export function attachmentPlan(kind: AttachmentKind, caps: ModelCapabilities): AttachmentPlan {
  switch (kind) {
    case 'image':
      return caps.vision ? 'native' : 'unsupported'
    case 'audio':
      return caps.audio ? 'native' : 'unsupported'
    case 'pdf':
      return caps.pdf ? 'native' : 'text'
    default:
      return 'text'
  }
}

/** g2'nin ek akışında bu modülün ele aldığı türler (görsel ve düz metin g2'de kalır). */
export const handlesAttachment = (relPath: string): boolean => ['pdf', 'docx', 'xlsx', 'audio', 'opk'].includes(attachmentKind(relPath))

// ---------- Yerel parçalar: mesaj metnine belirteç, model çağrısından önce parçaya çevrilir ----------

const PART_TOKEN = /\[g22-ek:([0-9a-f-]{36})\]/g
const pendingParts = new Map<string, ChatCompletionContentPart>()

function registerPart(part: ChatCompletionContentPart): string {
  const id = randomUUID()
  pendingParts.set(id, part)
  // Gönderilmeyen (hata veren) mesajların parçaları belleği şişirmesin.
  for (const key of pendingParts.keys()) {
    if (pendingParts.size <= MAX_PENDING_PARTS) break
    pendingParts.delete(key)
  }
  return `[g22-ek:${id}]`
}

/** Kullanıcı mesajlarındaki belirteçleri ses/dosya parçasına çevirir (kuyruk ve araya girme yolları da kapsanır). */
export function attachNativeParts(messages: ChatCompletionMessageParam[]): void {
  if (pendingParts.size === 0) return
  for (const m of messages) {
    if (m.role !== 'user') continue
    const texts = typeof m.content === 'string' ? [m.content] : m.content.flatMap((p) => (p.type === 'text' ? [p.text] : []))
    if (!texts.some((t) => t.includes('[g22-ek:'))) continue
    const parts: ChatCompletionContentPart[] = []
    const strip = (text: string) =>
      text.replace(PART_TOKEN, (_match, id: string) => {
        const part = pendingParts.get(id)
        if (!part) return '(ek bellekte bulunamadı; dosyayı yeniden ekle)'
        pendingParts.delete(id)
        parts.push(part)
        return ''
      })
    m.content =
      typeof m.content === 'string'
        ? [{ type: 'text', text: strip(m.content) }, ...parts]
        : [...m.content.map((p) => (p.type === 'text' ? { ...p, text: strip(p.text) } : p)), ...parts]
  }
}

const startsWith = (b: Buffer, sig: string, at = 0) => b.subarray(at, at + sig.length).toString('latin1') === sig

function sniffAudio(b: Buffer, format: string): boolean {
  switch (format) {
    case 'wav':
      return startsWith(b, 'RIFF') && startsWith(b, 'WAVE', 8)
    case 'mp3':
      return startsWith(b, 'ID3') || (b[0] === 0xff && ((b[1] ?? 0) & 0xe0) === 0xe0)
    case 'ogg':
      return startsWith(b, 'OggS')
    case 'flac':
      return startsWith(b, 'fLaC')
    case 'm4a':
      return startsWith(b, 'ftyp', 4)
    case 'aac':
      return startsWith(b, 'ADIF') || (b[0] === 0xff && ((b[1] ?? 0) & 0xf6) === 0xf0)
    case 'aiff':
      return startsWith(b, 'FORM') && (startsWith(b, 'AIFF', 8) || startsWith(b, 'AIFC', 8))
    default:
      return false
  }
}

async function readLimited(abs: string, rel: string, max: number): Promise<Buffer> {
  const info = await stat(abs)
  if (!info.isFile()) throw new Error(`${rel}: dosya değil`)
  if (info.size > max) throw new Error(`${rel}: dosya çok büyük (sınır ${Math.round(max / 1024 / 1024)} MB)`)
  return readFile(abs)
}

async function audioPart(abs: string, rel: string): Promise<ChatCompletionContentPart> {
  const bytes = await readLimited(abs, rel, MAX_AUDIO_BYTES)
  const format = AUDIO_FORMATS[path.extname(rel).slice(1).toLowerCase()] ?? ''
  if (!sniffAudio(bytes, format)) throw new Error(`${rel}: içerik ${format || 'ses'} dosyasına benzemiyor`)
  // SDK tipi yalnız wav|mp3 der; Venice diğer biçimleri de kabul eder (docs.venice.ai). Biçim yukarıdaki listeyle sınırlı.
  return { type: 'input_audio', input_audio: { data: bytes.toString('base64'), format } } as ChatCompletionContentPart
}

async function pdfPart(abs: string, rel: string): Promise<ChatCompletionContentPart> {
  const bytes = await readLimited(abs, rel, MAX_PDF_BYTES)
  if (!bytes.subarray(0, 1024).includes('%PDF-')) throw new Error(`${rel}: içerik PDF değil`)
  return { type: 'file', file: { filename: path.basename(rel), file_data: `data:application/pdf;base64,${bytes.toString('base64')}` } }
}

// ---------- Metne dönüştürme ----------

interface ZipEntry {
  method: number
  size: number
  compSize: number
  offset: number
}

function zipEntries(buf: Buffer): Map<string, ZipEntry> {
  let eocd = -1
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 65_557); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) {
      eocd = i
      break
    }
  }
  if (eocd < 0) throw new Error('ZIP dizini bulunamadı (bozuk belge)')
  const count = buf.readUInt16LE(eocd + 10)
  if (count > MAX_ZIP_ENTRIES) throw new Error('Belgede çok fazla parça var')
  let p = buf.readUInt32LE(eocd + 16)
  const entries = new Map<string, ZipEntry>()
  for (let n = 0; n < count; n++) {
    if (p + 46 > buf.length || buf.readUInt32LE(p) !== 0x02014b50) throw new Error('ZIP dizini bozuk')
    const nameLen = buf.readUInt16LE(p + 28)
    entries.set(buf.toString('utf8', p + 46, p + 46 + nameLen), {
      method: buf.readUInt16LE(p + 10),
      compSize: buf.readUInt32LE(p + 20),
      size: buf.readUInt32LE(p + 24),
      offset: buf.readUInt32LE(p + 42),
    })
    p += 46 + nameLen + buf.readUInt16LE(p + 30) + buf.readUInt16LE(p + 32)
  }
  return entries
}

function zipText(buf: Buffer, entries: ReadonlyMap<string, ZipEntry>, name: string): string | null {
  const e = entries.get(name)
  if (!e) return null
  if (e.size > MAX_ZIP_ENTRY_BYTES) throw new Error(`${name} çok büyük`)
  if (e.offset + 30 > buf.length || buf.readUInt32LE(e.offset) !== 0x04034b50) throw new Error('ZIP girdisi bozuk')
  const start = e.offset + 30 + buf.readUInt16LE(e.offset + 26) + buf.readUInt16LE(e.offset + 28)
  const data = buf.subarray(start, start + e.compSize)
  if (e.method === 0) return data.toString('utf8')
  // maxOutputLength: başlıktaki boyut yalan söylese de açılan veri sınırı aşamaz (zip bombası).
  if (e.method === 8) {
    try {
      return inflateRawSync(data, { maxOutputLength: MAX_ZIP_ENTRY_BYTES }).toString('utf8')
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === 'ERR_BUFFER_TOO_LARGE') throw new Error(`${name} açılınca çok büyük`)
      throw new Error(`${name} açılamadı (bozuk belge)`)
    }
  }
  throw new Error('Desteklenmeyen ZIP sıkıştırması')
}

const ENTITIES: Readonly<Record<string, string>> = { lt: '<', gt: '>', amp: '&', quot: '"', apos: "'" }

export function decodeXml(text: string): string {
  return text.replace(/&(#x[0-9a-f]{1,6}|#\d{1,7}|lt|gt|amp|quot|apos);/gi, (match, e: string) => {
    if (e[0] !== '#') return ENTITIES[e.toLowerCase()] ?? match
    const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : Number(e.slice(1))
    return code <= 0x10ffff ? String.fromCodePoint(code) : match
  })
}

// Bozuk/kötü niyetli belgede (kapanmayan etiketler) düzenli ifade geri izlemesi karesel büyümesin: etiketler indexOf ile
// ileriye doğru tek geçişte bulunur; öznitelik ayrıştırma yalnızca makul uzunluktaki etiketlerde yapılır.
const MAX_TAG_CHARS = 2_000

interface XmlTag {
  start: number
  end: number
  /** "/w:p" için "/w:p", "<c r=..."> için "c" */
  name: string
  attrs: string
  selfClosing: boolean
}

function* xmlTags(xml: string): Generator<XmlTag> {
  for (let i = xml.indexOf('<'); i >= 0; ) {
    const close = xml.indexOf('>', i)
    if (close < 0) return
    const body = xml.slice(i + 1, close)
    const name = /^\/?[\w:.-]+/.exec(body)?.[0] ?? ''
    const rest = body.slice(name.length)
    yield { start: i, end: close + 1, name, attrs: rest.length <= MAX_TAG_CHARS ? rest : '', selfClosing: body.endsWith('/') }
    i = xml.indexOf('<', close + 1)
  }
}

function* elements(xml: string, name: string): Generator<{ attrs: string; inner: string }> {
  let open: { attrs: string; from: number } | null = null
  for (const t of xmlTags(xml)) {
    if (open === null && t.name === name) {
      if (t.selfClosing) yield { attrs: t.attrs, inner: '' }
      else open = { attrs: t.attrs, from: t.end }
    } else if (open !== null && t.name === `/${name}`) {
      yield { attrs: open.attrs, inner: xml.slice(open.from, t.start) }
      open = null
    }
  }
}

const attrsOf = (attrs: string): Record<string, string> =>
  Object.fromEntries([...attrs.matchAll(/([\w:]+)="([^"]*)"/g)].map((m) => [m[1] ?? '', decodeXml(m[2] ?? '')]))

/** word/document.xml: paragraf sonu, sekme ve satır sonları korunur; biçim atılır. */
export function docxText(xml: string): string {
  let out = ''
  let textFrom = -1
  for (const t of xmlTags(xml)) {
    if (textFrom >= 0) out += decodeXml(xml.slice(textFrom, t.start))
    textFrom = t.name === 'w:t' && !t.selfClosing ? t.end : -1
    // Sekme durağı tanımları (<w:tab w:val=.../>) metin değildir; yalnızca özniteliksiz çalışma sekmesi sayılır.
    if (t.name === 'w:tab' && t.attrs.trim() === '/') out += '\t'
    else if (t.name === 'w:br' || t.name === 'w:cr' || t.name === '/w:p') out += '\n'
  }
  return out.replace(/\n{3,}/g, '\n\n').trim()
}

const csvField = (v: string) => (/[",\r\n]/.test(v) ? `"${v.replaceAll('"', '""')}"` : v)

function columnIndex(ref: string | undefined, fallback: number): number {
  const letters = /^([A-Z]{1,3})\d/.exec(ref ?? '')?.[1]
  if (!letters) return fallback
  return [...letters].reduce((n, c) => n * 26 + c.charCodeAt(0) - 64, 0) - 1
}

const runText = (xml: string) => [...elements(xml, 't')].map((t) => decodeXml(t.inner)).join('')

/** Çalışma kitabının sayfalarını CSV'ye çevirir (formüller değil hesaplanmış değerler). */
export function xlsxToCsv(buf: Buffer): { text: string; sheets: number; truncated: boolean } {
  const entries = zipEntries(buf)
  const read = (name: string) => zipText(buf, entries, name)
  const shared = [...elements(read('xl/sharedStrings.xml') ?? '', 'si')].map((si) => runText(si.inner))
  const rels = new Map(
    [...elements(read('xl/_rels/workbook.xml.rels') ?? '', 'Relationship')].map((r) => {
      const a = attrsOf(r.attrs)
      return [a['Id'] ?? '', a['Target'] ?? ''] as const
    }),
  )
  const sheets = [...elements(read('xl/workbook.xml') ?? '', 'sheet')].map((s) => attrsOf(s.attrs))
  let truncated = sheets.length > MAX_SHEETS
  const parts: string[] = []
  for (const sheet of sheets.slice(0, MAX_SHEETS)) {
    const target = rels.get(sheet['r:id'] ?? '') ?? ''
    const xml = target ? read(target.startsWith('/') ? target.slice(1) : `xl/${target}`) : null
    if (xml === null) continue
    const lines: string[] = []
    for (const row of elements(xml, 'row')) {
      if (lines.length >= MAX_SHEET_ROWS) {
        truncated = true
        break
      }
      const cells: string[] = []
      for (const c of elements(row.inner, 'c')) {
        const a = attrsOf(c.attrs)
        const col = columnIndex(a['r'], cells.length)
        if (col >= MAX_SHEET_COLS) {
          truncated = true
          continue
        }
        const inner = c.inner
        const raw = decodeXml(/<v>([^<]*)<\/v>/.exec(inner)?.[1] ?? '')
        const value =
          a['t'] === 's' ? (shared[Number(raw)] ?? '') : a['t'] === 'inlineStr' ? runText(inner) : a['t'] === 'b' ? (raw === '1' ? 'TRUE' : 'FALSE') : raw
        while (cells.length < col) cells.push('')
        cells[col] = value
      }
      lines.push(cells.map(csvField).join(','))
    }
    parts.push(`# Sayfa: ${sheet['name'] ?? '?'}\n${lines.join('\n')}`)
  }
  return { text: parts.join('\n\n'), sheets: sheets.length, truncated }
}

// OpenBullet .opk (ZIP) içindeki okunur metin girdileri. İkili parçalar (görseller vb.) atlanır.
const OPK_TEXT_ENTRY = /\.(loli|les|json|md|txt|csv|xml|yml|yaml|ini|cfg|lolicode)$/i
/** Script (LoliCode) önce, sonra ayarlar/metadata, sonra kalan metinler: model en çok işe yarayanı başta görsün. */
function opkEntryRank(name: string): number {
  const base = name.toLowerCase()
  if (/\.(loli|les|lolicode)$/.test(base) || base.includes('script') || base.includes('config.les')) return 0
  if (base.includes('metadata') || base.includes('settings') || base.includes('readme')) return 1
  return 2
}

export function opkToText(buf: Buffer): { text: string; note: string } {
  const entries = zipEntries(buf)
  const names = [...entries.keys()].sort((a, b) => opkEntryRank(a) - opkEntryRank(b) || a.localeCompare(b))
  const parts: string[] = []
  let skipped = 0
  for (const name of names) {
    if (!OPK_TEXT_ENTRY.test(name)) {
      skipped++
      continue
    }
    let content: string | null
    try {
      content = zipText(buf, entries, name)
    } catch {
      skipped++
      continue
    }
    if (content === null || content.includes('\u0000')) {
      skipped++
      continue
    }
    const trimmed = content.trim()
    if (trimmed) parts.push(`=== ${name} ===\n${trimmed}`)
  }
  if (parts.length === 0) throw new Error('.opk içinde okunur metin girdisi bulunamadı')
  return {
    text: parts.join('\n\n'),
    note: `OpenBullet .opk paketi açıldı: ${parts.length} metin girdisi${skipped > 0 ? `, ${skipped} ikili/atlandı` : ''} (LoliCode + ayarlar)`,
  }
}

async function convertToText(abs: string, rel: string, kind: AttachmentKind): Promise<{ text: string; note: string }> {
  const bytes = await readLimited(abs, rel, MAX_PDF_BYTES)
  if (kind === 'opk') {
    if (!startsWith(bytes, 'PK\u0003\u0004')) throw new Error(`${rel}: içerik .opk (ZIP) paketi değil`)
    return opkToText(bytes)
  }
  if (kind === 'pdf') {
    if (!bytes.subarray(0, 1024).includes('%PDF-')) throw new Error(`${rel}: içerik PDF değil`)
    const pdf = await extractPdfText(new Uint8Array(bytes))
    const more = pdf.pages.length < pdf.totalPages ? '; devamı için read_pdf aracı kullanılabilir' : ''
    return {
      text: pdf.pages.map((p) => `--- Sayfa ${p.page} ---\n${p.text}`).join('\n\n'),
      note: `PDF metne dönüştürüldü: ${pdf.pages.length}/${pdf.totalPages} sayfa${more}`,
    }
  }
  if (kind !== 'docx' && kind !== 'xlsx') throw new Error(`${rel}: bu tür metne dönüştürülemez`)
  if (!startsWith(bytes, 'PK\u0003\u0004')) throw new Error(`${rel}: içerik ${kind.toUpperCase()} belgesi değil`)
  if (kind === 'docx') {
    const xml = zipText(bytes, zipEntries(bytes), 'word/document.xml')
    if (xml === null) throw new Error(`${rel}: belge gövdesi (word/document.xml) yok`)
    return { text: docxText(xml), note: 'DOCX metne dönüştürüldü (biçimlendirme atıldı)' }
  }
  const sheet = xlsxToCsv(bytes)
  return { text: sheet.text, note: `XLSX CSV'ye dönüştürüldü: ${Math.min(sheet.sheets, MAX_SHEETS)}/${sheet.sheets} sayfa${sheet.truncated ? ', satır/sütun sınırıyla kesildi' : ''}` }
}

/** g2 ek akışının kullandığı <file> bloğu; içerikteki </file> kaçırılır ki ek ayrıştırma (splitAttachments) bozulmasın. */
const fileBlock = (rel: string, body: string) => `<file path="${rel.split(path.sep).join('/')}">\n${body.replaceAll('</file>', '<\\/file>')}\n</file>`

/**
 * PDF/DOCX/XLSX/ses eki için mesaja girecek blok. Yol çağıran tarafından çalışma dizinine hapsedilmiş ve
 * hassas dosya kontrolünden geçmiş olmalı (g2 buildContent).
 */
export async function buildAttachmentBlock(abs: string, rel: string, caps: ModelCapabilities): Promise<string> {
  const kind = attachmentKind(rel)
  if (kind === 'audio') {
    if (!caps.audio) throw new Error(`${rel}: seçili model ses girişini desteklemiyor; ses destekleyen bir model seç.`)
    return fileBlock(rel, `[Ses eki modele ses olarak gönderildi] ${registerPart(await audioPart(abs, rel))}`)
  }
  if (kind === 'pdf' && caps.pdf) return fileBlock(rel, `[PDF modele dosya olarak gönderildi] ${registerPart(await pdfPart(abs, rel))}`)
  const { text, note } = await convertToText(abs, rel, kind)
  const clipped = text.length > MAX_CONVERTED_CHARS ? `${text.slice(0, MAX_CONVERTED_CHARS)}\n[${MAX_CONVERTED_CHARS} karakter sınırında kesildi]` : text
  return fileBlock(rel, `[${note}]\n${clipped || '(metin bulunamadı)'}`)
}
