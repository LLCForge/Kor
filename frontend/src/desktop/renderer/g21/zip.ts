// [g21] Bağımlılıksız ZIP okuyucu: merkezi dizini ayrıştırır (çıkarmadan listeleme, #184) ve tek bir girişi
//      yalnızca bellekte açar (DOCX/XLSX/EPUB). Diske hiçbir şey yazılmaz.
//      Güvenlik: giriş sayısı ve giriş başına açılmış boyut sınırlıdır; açma akış halinde sayılır, bildirilen
//      boyut yalan söylese bile sınır aşılınca durur (zip bombası). ZIP64 ve şifreli girişler açıkça reddedilir.

export const MAX_ZIP_ENTRIES = 20_000
export const MAX_ENTRY_BYTES = 64 * 1024 * 1024

export interface ZipEntryInfo {
  name: string
  /** Açılmış boyut (merkezi dizinde bildirilen) */
  size: number
  compressedSize: number
  /** 0 = store, 8 = deflate */
  method: number
  encrypted: boolean
  dir: boolean
  localOffset: number
}

const SIG_EOCD = 0x06054b50
const SIG_CENTRAL = 0x02014b50
const SIG_LOCAL = 0x04034b50
const utf8 = new TextDecoder('utf-8')

function view(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
}

function findEocd(bytes: Uint8Array, dv: DataView): number {
  // EOCD kaydı sondadır; en fazla 64 KiB yorum alanı olabilir.
  const min = Math.max(0, bytes.length - 22 - 0xffff)
  for (let i = bytes.length - 22; i >= min; i--) {
    if (dv.getUint32(i, true) === SIG_EOCD) return i
  }
  throw new Error('Geçerli bir ZIP arşivi değil (dizin sonu bulunamadı)')
}

/** Merkezi dizini okur. Bozuk/ZIP64/aşırı büyük arşivde açıklayıcı hata fırlatır. */
export function listZip(bytes: Uint8Array): ZipEntryInfo[] {
  if (bytes.length < 22) throw new Error('Geçerli bir ZIP arşivi değil')
  const dv = view(bytes)
  const eocd = findEocd(bytes, dv)
  const count = dv.getUint16(eocd + 10, true)
  const cdOffset = dv.getUint32(eocd + 16, true)
  if (count === 0xffff || cdOffset === 0xffffffff) throw new Error('ZIP64 arşivleri desteklenmiyor')
  if (count > MAX_ZIP_ENTRIES) throw new Error(`Arşivde çok fazla giriş var (${count}, sınır ${MAX_ZIP_ENTRIES})`)
  const entries: ZipEntryInfo[] = []
  let p = cdOffset
  try {
    for (let k = 0; k < count; k++) {
      if (dv.getUint32(p, true) !== SIG_CENTRAL) throw new Error('bozuk merkezi dizin')
      const flags = dv.getUint16(p + 8, true)
      const method = dv.getUint16(p + 10, true)
      const compressedSize = dv.getUint32(p + 20, true)
      const size = dv.getUint32(p + 24, true)
      const nameLen = dv.getUint16(p + 28, true)
      const extraLen = dv.getUint16(p + 30, true)
      const commentLen = dv.getUint16(p + 32, true)
      const localOffset = dv.getUint32(p + 42, true)
      if (p + 46 + nameLen > bytes.length) throw new Error('bozuk giriş adı')
      const name = utf8.decode(bytes.subarray(p + 46, p + 46 + nameLen))
      entries.push({ name, size, compressedSize, method, encrypted: (flags & 1) !== 0, dir: name.endsWith('/'), localOffset })
      p += 46 + nameLen + extraLen + commentLen
    }
  } catch (err) {
    throw new Error(`ZIP arşivi okunamadı: ${err instanceof RangeError ? 'beklenmeyen dosya sonu' : err instanceof Error ? err.message : String(err)}`)
  }
  return entries
}

async function inflateRaw(data: Uint8Array, limit: number): Promise<Uint8Array> {
  const source = new ReadableStream<BufferSource>({
    start(controller) {
      // IPC/dosya baytları hiçbir zaman SharedArrayBuffer değildir.
      controller.enqueue(data as Uint8Array<ArrayBuffer>)
      controller.close()
    },
  })
  const reader = source.pipeThrough(new DecompressionStream('deflate-raw')).getReader()
  const chunks: Uint8Array[] = []
  let total = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    total += value.length
    if (total > limit) {
      await reader.cancel().catch(() => undefined)
      throw new Error('Giriş açılınca boyut sınırını aşıyor (olası zip bombası)')
    }
    chunks.push(value)
  }
  const out = new Uint8Array(total)
  let o = 0
  for (const c of chunks) {
    out.set(c, o)
    o += c.length
  }
  return out
}

/** Tek bir girişi belleğe açar (store veya deflate). */
export async function readZipEntry(bytes: Uint8Array, entry: ZipEntryInfo, limit = MAX_ENTRY_BYTES): Promise<Uint8Array> {
  if (entry.encrypted) throw new Error(`Şifreli giriş açılamaz: ${entry.name}`)
  if (entry.dir) return new Uint8Array(0)
  if (entry.size > limit) throw new Error(`Giriş çok büyük: ${entry.name}`)
  const dv = view(bytes)
  const p = entry.localOffset
  if (p + 30 > bytes.length || dv.getUint32(p, true) !== SIG_LOCAL) throw new Error(`Bozuk yerel başlık: ${entry.name}`)
  const start = p + 30 + dv.getUint16(p + 26, true) + dv.getUint16(p + 28, true)
  const end = start + entry.compressedSize
  if (end > bytes.length) throw new Error(`Giriş verisi eksik: ${entry.name}`)
  const data = bytes.subarray(start, end)
  if (entry.method === 0) return data
  if (entry.method === 8) return inflateRaw(data, limit)
  throw new Error(`Desteklenmeyen sıkıştırma yöntemi (${entry.method}): ${entry.name}`)
}

/** Girişi UTF-8 metin olarak okur (BOM atılır). */
export async function readZipText(bytes: Uint8Array, entry: ZipEntryInfo, limit = MAX_ENTRY_BYTES): Promise<string> {
  const text = utf8.decode(await readZipEntry(bytes, entry, limit))
  return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text
}

/** Arşivdeki bir girişi tam adıyla bulur; bulunamazsa null. */
export function findEntry(entries: readonly ZipEntryInfo[], name: string): ZipEntryInfo | null {
  return entries.find((e) => e.name === name) ?? null
}

/** Girişin dikkat gerektiren özellikleri (zip slip, şifre, şüpheli sıkıştırma oranı, çalıştırılabilir). */
export function entryWarnings(entry: ZipEntryInfo): string[] {
  const warnings: string[] = []
  const segments = entry.name.split(/[\\/]/)
  if (/^([\\/]|[a-zA-Z]:)/.test(entry.name) || segments.includes('..')) warnings.push('çalışma dizini dışına yol (zip slip)')
  if (entry.encrypted) warnings.push('şifreli')
  if (entry.compressedSize > 0 && entry.size > 1_000_000 && entry.size / entry.compressedSize > 100) warnings.push('aşırı sıkıştırma oranı')
  if (/\.(exe|dll|bat|cmd|ps1|vbs|scr|msi|com|jar|sh|app|lnk|hta)$/i.test(entry.name)) warnings.push('çalıştırılabilir')
  return warnings
}

export interface ZipTreeNode {
  name: string
  path: string
  dir: boolean
  size: number
  compressedSize: number
  warnings: string[]
  children: ZipTreeNode[]
}

/** Düz giriş listesini klasör ağacına çevirir (klasörler önce, ada göre sıralı; boyutlar klasöre toplanır). */
export function zipTree(entries: readonly ZipEntryInfo[]): ZipTreeNode {
  const root: ZipTreeNode = { name: '', path: '', dir: true, size: 0, compressedSize: 0, warnings: [], children: [] }
  const dirs = new Map<string, ZipTreeNode>([['', root]])
  const ensureDir = (path: string): ZipTreeNode => {
    const existing = dirs.get(path)
    if (existing) return existing
    const cut = path.lastIndexOf('/')
    const parent = ensureDir(cut < 0 ? '' : path.slice(0, cut))
    const node: ZipTreeNode = { name: path.slice(cut + 1), path, dir: true, size: 0, compressedSize: 0, warnings: [], children: [] }
    parent.children.push(node)
    dirs.set(path, node)
    return node
  }
  for (const entry of entries) {
    const clean = entry.name.replace(/\\/g, '/').replace(/\/+$/, '')
    if (!clean) continue
    if (entry.dir) {
      ensureDir(clean)
      continue
    }
    const cut = clean.lastIndexOf('/')
    const parent = ensureDir(cut < 0 ? '' : clean.slice(0, cut))
    parent.children.push({
      name: clean.slice(cut + 1),
      path: clean,
      dir: false,
      size: entry.size,
      compressedSize: entry.compressedSize,
      warnings: entryWarnings(entry),
      children: [],
    })
  }
  const finish = (node: ZipTreeNode): void => {
    for (const child of node.children) {
      if (child.dir) finish(child)
      node.size += child.size
      node.compressedSize += child.compressedSize
    }
    // Kapalı klasörde de görünsün: riskli giriş içeren klasör işaretlenir.
    if (node.children.some((c) => c.warnings.length > 0)) node.warnings.push('riskli giriş içerir')
    node.children.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name, 'tr') : a.dir ? -1 : 1))
  }
  finish(root)
  return root
}

/**
 * Göreli bir referansı (ör. "../images/a.png") taban dosyanın klasörüne göre çözer; "/" ayraçlı, normalize yol
 * döndürür. Şemalı (http:, data:, file: ...) ya da kökten (/) başlayan referanslar ve kök dışına çıkışlar reddedilir.
 */
export function resolveRelative(baseFile: string, ref: string): string {
  let decoded: string
  try {
    decoded = decodeURIComponent(ref.split('#')[0] ?? '')
  } catch {
    throw new Error(`Geçersiz yol: ${ref}`)
  }
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(decoded) || /^[\\/]/.test(decoded)) throw new Error(`Yalnızca göreli yerel yollara izin verilir: ${ref}`)
  const parts = baseFile.replace(/\\/g, '/').split('/').slice(0, -1)
  for (const seg of decoded.replace(/\\/g, '/').split('/')) {
    if (seg === '' || seg === '.') continue
    if (seg === '..') {
      if (parts.length === 0) throw new Error(`Kök dışına çıkan yol: ${ref}`)
      parts.pop()
    } else parts.push(seg)
  }
  if (parts.length === 0) throw new Error(`Geçersiz yol: ${ref}`)
  return parts.join('/')
}
