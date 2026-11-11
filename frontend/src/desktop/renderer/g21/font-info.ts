// [g21] Yazı tipi bilgisi (#183): sfnt (TTF/OTF/TTC) ve WOFF tablolarından ad (name) ve glif sayısı (maxp).
//      WOFF2 (brotli + dönüşümler) yalnızca biçim olarak tanınır. Görsel önizlemeyi tarayıcının FontFace API'si
//      yapar; Chromium yazı tipini OTS ile doğrular (bozuk/kötü niyetli dosya reddedilir).

export interface FontInfo {
  format: string
  family?: string
  subfamily?: string
  fullName?: string
  version?: string
  designer?: string
  license?: string
  glyphs?: number
}

interface TableRef {
  offset: number
  length: number
  /** WOFF: sıkıştırılmış uzunluk (length'ten küçükse zlib ile sıkıştırılmış) */
  compLength: number
}

const tag = (bytes: Uint8Array, at: number) => String.fromCharCode(...bytes.subarray(at, at + 4))

async function inflate(data: Uint8Array): Promise<Uint8Array> {
  const stream = new ReadableStream<BufferSource>({
    start(c) {
      c.enqueue(data as Uint8Array<ArrayBuffer>) // dosya baytları SharedArrayBuffer değildir
      c.close()
    },
  }).pipeThrough(new DecompressionStream('deflate'))
  return new Uint8Array(await new Response(stream).arrayBuffer())
}

function utf16be(bytes: Uint8Array): string {
  let out = ''
  for (let i = 0; i + 1 < bytes.length; i += 2) out += String.fromCharCode(((bytes[i] ?? 0) << 8) | (bytes[i + 1] ?? 0))
  return out
}

/** name tablosundan istenen kimlikleri okur (Windows Unicode kayıtları öncelikli). */
export function parseNameTable(table: Uint8Array): Map<number, string> {
  const dv = new DataView(table.buffer, table.byteOffset, table.byteLength)
  const names = new Map<number, string>()
  const count = Math.min(dv.getUint16(2), 1024)
  const storage = dv.getUint16(4)
  const rank = new Map<number, number>()
  for (let i = 0; i < count; i++) {
    const r = 6 + i * 12
    if (r + 12 > table.length) break
    const platform = dv.getUint16(r)
    const encoding = dv.getUint16(r + 2)
    const language = dv.getUint16(r + 4)
    const id = dv.getUint16(r + 6)
    const len = dv.getUint16(r + 8)
    const off = storage + dv.getUint16(r + 10)
    if (off + len > table.length || ![1, 2, 4, 5, 9, 13, 16, 17].includes(id)) continue
    const raw = table.subarray(off, off + len)
    let value: string | null = null
    let score = 0
    if (platform === 3 && (encoding === 1 || encoding === 10)) {
      value = utf16be(raw)
      score = language === 0x0409 ? 3 : 2
    } else if (platform === 0) {
      value = utf16be(raw)
      score = 2
    } else if (platform === 1 && encoding === 0) {
      value = String.fromCharCode(...raw)
      score = 1
    }
    if (value && score > (rank.get(id) ?? 0)) {
      names.set(id, value.replace(/\0/g, '').trim())
      rank.set(id, score)
    }
  }
  return names
}

/** Yazı tipi dosyasının biçimini ve ad bilgilerini döndürür. */
export async function readFontInfo(bytes: Uint8Array): Promise<FontInfo> {
  if (bytes.length < 12) throw new Error('Yazı tipi dosyası çok kısa')
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const sig = tag(bytes, 0)
  if (sig === 'wOF2') return { format: 'WOFF2' }
  const tables = new Map<string, TableRef>()
  let format: string
  if (sig === 'wOFF') {
    const flavor = tag(bytes, 4)
    format = flavor === 'OTTO' ? 'WOFF (OpenType/CFF)' : 'WOFF (TrueType)'
    const num = Math.min(dv.getUint16(12), 512)
    for (let i = 0; i < num; i++) {
      const r = 44 + i * 20
      if (r + 20 > bytes.length) break
      tables.set(tag(bytes, r), { offset: dv.getUint32(r + 4), compLength: dv.getUint32(r + 8), length: dv.getUint32(r + 12) })
    }
  } else {
    let base = 0
    if (sig === 'ttcf') {
      format = 'TrueType Collection'
      base = dv.getUint32(12)
    } else if (sig === 'OTTO') format = 'OpenType (CFF)'
    else if (sig === 'true' || dv.getUint32(0) === 0x00010000) format = 'TrueType'
    else throw new Error('Tanınmayan yazı tipi biçimi')
    const num = Math.min(dv.getUint16(base + 4), 512)
    for (let i = 0; i < num; i++) {
      const r = base + 12 + i * 16
      if (r + 16 > bytes.length) break
      const length = dv.getUint32(r + 12)
      tables.set(tag(bytes, r), { offset: dv.getUint32(r + 8), length, compLength: length })
    }
  }
  const read = async (name: string): Promise<Uint8Array | null> => {
    const t = tables.get(name)
    if (!t || t.offset + t.compLength > bytes.length || t.length > 4 * 1024 * 1024) return null
    const raw = bytes.subarray(t.offset, t.offset + t.compLength)
    return t.compLength < t.length ? inflate(raw) : raw
  }
  const info: FontInfo = { format }
  try {
    const name = await read('name')
    if (name) {
      const names = parseNameTable(name)
      const pick = (...ids: number[]) => ids.map((id) => names.get(id)).find(Boolean)
      info.family = pick(16, 1)
      info.subfamily = pick(17, 2)
      info.fullName = pick(4)
      info.version = pick(5)
      info.designer = pick(9)
      info.license = pick(13)?.slice(0, 400)
    }
    const maxp = await read('maxp')
    if (maxp && maxp.length >= 6) info.glyphs = new DataView(maxp.buffer, maxp.byteOffset, maxp.byteLength).getUint16(4)
  } catch (err) {
    console.warn('Yazı tipi tabloları okunamadı:', err)
  }
  return info
}
