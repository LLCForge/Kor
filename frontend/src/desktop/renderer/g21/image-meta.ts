// [g21] Görsel meta verisi (#181): JPEG (SOF + APP1 EXIF), PNG (IHDR, tEXt/iTXt, eXIf), GIF ve WebP boyutları.
//      Bağımlılıksız TIFF/IFD okuyucu; her okuma sınır denetimli, IFD döngüleri ve giriş sayısı sınırlı.
//      Konum (GPS) girişleri `sensitive` işaretlidir: arayüz bunları varsayılan olarak GİZLER.

export interface MetaEntry {
  group: 'Genel' | 'Kamera' | 'Çekim' | 'Konum' | 'Metin'
  label: string
  value: string
  sensitive?: boolean
}

export interface ImageMeta {
  format: string
  width: number | null
  height: number | null
  entries: MetaEntry[]
  hasLocation: boolean
}

const MAX_IFD_ENTRIES = 512
const MAX_TEXT_CHUNKS = 64
const latin1 = new TextDecoder('latin1')
const utf8 = new TextDecoder('utf-8')

const IFD0_TAGS: Record<number, [MetaEntry['group'], string]> = {
  0x010e: ['Metin', 'Açıklama'],
  0x010f: ['Kamera', 'Üretici'],
  0x0110: ['Kamera', 'Model'],
  0x0112: ['Genel', 'Yönlendirme'],
  0x0131: ['Genel', 'Yazılım'],
  0x0132: ['Genel', 'Değiştirilme tarihi'],
  0x013b: ['Metin', 'Sanatçı'],
  0x8298: ['Metin', 'Telif'],
}
const EXIF_TAGS: Record<number, [MetaEntry['group'], string]> = {
  0x829a: ['Çekim', 'Pozlama süresi'],
  0x829d: ['Çekim', 'Diyafram (f)'],
  0x8827: ['Çekim', 'ISO'],
  0x9003: ['Çekim', 'Çekim tarihi'],
  0x920a: ['Çekim', 'Odak uzaklığı (mm)'],
  0xa434: ['Kamera', 'Objektif'],
}

type Value = string | number | number[]

/** TIFF yapısını (EXIF gövdesi) okur; GPS dahil girişleri döndürür. */
export function parseTiff(bytes: Uint8Array): MetaEntry[] {
  if (bytes.length < 8) return []
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const order = latin1.decode(bytes.subarray(0, 2))
  if (order !== 'II' && order !== 'MM') return []
  const le = order === 'II'
  const u16 = (o: number) => dv.getUint16(o, le)
  const u32 = (o: number) => dv.getUint32(o, le)
  if (u16(2) !== 42) return []
  const SIZES: Record<number, number> = { 1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 9: 4, 10: 8 }
  const visited = new Set<number>()

  const readValue = (type: number, count: number, at: number): Value | null => {
    const size = (SIZES[type] ?? 0) * count
    if (size === 0 || count > 4096) return null
    const off = size <= 4 ? at : u32(at)
    if (off + size > bytes.length) return null
    switch (type) {
      case 2:
        return latin1.decode(bytes.subarray(off, off + count)).replace(/\0[\s\S]*$/, '').trim()
      case 3:
        return count === 1 ? u16(off) : Array.from({ length: count }, (_, i) => u16(off + i * 2))
      case 4:
        return count === 1 ? u32(off) : Array.from({ length: count }, (_, i) => u32(off + i * 4))
      case 5:
      case 10: {
        const vals = Array.from({ length: count }, (_, i) => {
          const num = type === 5 ? u32(off + i * 8) : dv.getInt32(off + i * 8, le)
          const den = type === 5 ? u32(off + i * 8 + 4) : dv.getInt32(off + i * 8 + 4, le)
          return den === 0 ? 0 : num / den
        })
        return count === 1 ? (vals[0] ?? 0) : vals
      }
      case 7:
        return latin1.decode(bytes.subarray(off, off + Math.min(count, 256))).replace(/\0+$/, '').trim()
      default:
        return null
    }
  }

  const readIfd = (offset: number): Map<number, Value> => {
    const tags = new Map<number, Value>()
    if (offset <= 0 || offset + 2 > bytes.length || visited.has(offset)) return tags
    visited.add(offset)
    const n = Math.min(u16(offset), MAX_IFD_ENTRIES)
    for (let i = 0; i < n; i++) {
      const at = offset + 2 + i * 12
      if (at + 12 > bytes.length) break
      const value = readValue(u16(at + 2), u32(at + 4), at + 8)
      if (value !== null) tags.set(u16(at), value)
    }
    return tags
  }

  const entries: MetaEntry[] = []
  const push = (group: MetaEntry['group'], label: string, value: Value, sensitive = false) => {
    const text = Array.isArray(value) ? value.map((v) => +v.toFixed(4)).join(', ') : typeof value === 'number' ? String(+value.toFixed(4)) : value
    if (text) entries.push({ group, label, value: text, ...(sensitive ? { sensitive } : {}) })
  }
  try {
    const ifd0 = readIfd(u32(4))
    for (const [tag, [group, label]] of Object.entries(IFD0_TAGS)) {
      const v = ifd0.get(Number(tag))
      if (v !== undefined) push(group, label, v)
    }
    const exifPtr = ifd0.get(0x8769)
    if (typeof exifPtr === 'number') {
      const exif = readIfd(exifPtr)
      for (const [tag, [group, label]] of Object.entries(EXIF_TAGS)) {
        const v = exif.get(Number(tag))
        if (v === undefined) continue
        if (Number(tag) === 0x829a && typeof v === 'number' && v > 0 && v < 1) push(group, label, `1/${Math.round(1 / v)} sn`)
        else push(group, label, v)
      }
    }
    const gpsPtr = ifd0.get(0x8825)
    if (typeof gpsPtr === 'number') {
      const gps = readIfd(gpsPtr)
      const coord = (val: Value | undefined, ref: Value | undefined): number | null => {
        if (!Array.isArray(val) || val.length < 3) return null
        const deg = (val[0] ?? 0) + (val[1] ?? 0) / 60 + (val[2] ?? 0) / 3600
        return ref === 'S' || ref === 'W' ? -deg : deg
      }
      const lat = coord(gps.get(2), gps.get(1))
      const lon = coord(gps.get(4), gps.get(3))
      if (lat !== null && lon !== null) push('Konum', 'Koordinat', `${lat.toFixed(6)}, ${lon.toFixed(6)}`, true)
      const alt = gps.get(6)
      if (typeof alt === 'number') push('Konum', 'Yükseklik (m)', alt, true)
      const date = gps.get(0x1d)
      if (typeof date === 'string') push('Konum', 'GPS tarihi', date, true)
      if (gps.size > 0 && !entries.some((e) => e.group === 'Konum')) push('Konum', 'GPS verisi', `${gps.size} alan`, true)
    }
  } catch (err) {
    // Bozuk EXIF: o ana kadar okunanlar gösterilir.
    if (!(err instanceof RangeError)) throw err
  }
  return entries
}

function jpeg(bytes: Uint8Array, dv: DataView): ImageMeta {
  const meta: ImageMeta = { format: 'JPEG', width: null, height: null, entries: [], hasLocation: false }
  let p = 2
  while (p + 4 <= bytes.length) {
    if (bytes[p] !== 0xff) break
    const marker = bytes[p + 1] ?? 0
    if (marker === 0xd8 || (marker >= 0xd0 && marker <= 0xd7) || marker === 0x01) {
      p += 2
      continue
    }
    if (marker === 0xda || marker === 0xd9) break
    const len = dv.getUint16(p + 2)
    if (len < 2 || p + 2 + len > bytes.length) break
    const seg = bytes.subarray(p + 4, p + 2 + len)
    if (marker === 0xe1 && latin1.decode(seg.subarray(0, 6)) === 'Exif\0\0') meta.entries.push(...parseTiff(seg.subarray(6)))
    if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc && seg.length >= 5) {
      meta.height = dv.getUint16(p + 5)
      meta.width = dv.getUint16(p + 7)
    }
    p += 2 + len
  }
  return meta
}

function png(bytes: Uint8Array, dv: DataView): ImageMeta {
  const meta: ImageMeta = { format: 'PNG', width: dv.getUint32(16), height: dv.getUint32(20), entries: [], hasLocation: false }
  let p = 8
  let texts = 0
  while (p + 12 <= bytes.length) {
    const len = dv.getUint32(p)
    const type = latin1.decode(bytes.subarray(p + 4, p + 8))
    if (p + 12 + len > bytes.length || type === 'IDAT' || type === 'IEND') break
    const data = bytes.subarray(p + 8, p + 8 + len)
    if ((type === 'tEXt' || type === 'iTXt') && texts++ < MAX_TEXT_CHUNKS) {
      const zero = data.indexOf(0)
      if (zero > 0) {
        const key = latin1.decode(data.subarray(0, zero))
        let value: string | null = null
        if (type === 'tEXt') value = latin1.decode(data.subarray(zero + 1))
        else if (data[zero + 1] === 0) {
          // iTXt: sıkıştırma bayrağı, yöntem, dil\0, çevrilmiş anahtar\0, metin (yalnızca sıkıştırmasız okunur).
          const langEnd = data.indexOf(0, zero + 3)
          const transEnd = langEnd < 0 ? -1 : data.indexOf(0, langEnd + 1)
          if (transEnd >= 0) value = utf8.decode(data.subarray(transEnd + 1))
        }
        if (value) meta.entries.push({ group: 'Metin', label: key, value: value.slice(0, 2000) })
      }
    } else if (type === 'eXIf') meta.entries.push(...parseTiff(data))
    p += 12 + len
  }
  return meta
}

/** Görsel baytlarından biçim, boyut ve meta girişlerini çıkarır. Tanınmayan biçimde format 'bilinmiyor'. */
export function readImageMeta(bytes: Uint8Array): ImageMeta {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const head = latin1.decode(bytes.subarray(0, 16))
  let meta: ImageMeta = { format: 'bilinmiyor', width: null, height: null, entries: [], hasLocation: false }
  try {
    if (bytes[0] === 0xff && bytes[1] === 0xd8) meta = jpeg(bytes, dv)
    // 0x89 windows-1252 çözümünde başka karaktere dönüşür; bayt olarak denetlenir.
    else if (bytes[0] === 0x89 && head.slice(1, 8) === 'PNG\r\n\x1a\n' && bytes.length >= 24) meta = png(bytes, dv)
    else if (head.startsWith('GIF8') && bytes.length >= 10) meta = { ...meta, format: 'GIF', width: dv.getUint16(6, true), height: dv.getUint16(8, true) }
    else if (head.startsWith('RIFF') && head.slice(8, 12) === 'WEBP' && bytes.length >= 30) {
      const chunk = head.slice(12, 16)
      if (chunk === 'VP8X') meta = { ...meta, format: 'WebP', width: 1 + (dv.getUint32(24, true) & 0xffffff), height: 1 + (dv.getUint32(27, true) & 0xffffff) }
      else if (chunk === 'VP8 ') meta = { ...meta, format: 'WebP', width: dv.getUint16(26, true) & 0x3fff, height: dv.getUint16(28, true) & 0x3fff }
      else if (chunk === 'VP8L') {
        const bits = dv.getUint32(21, true)
        meta = { ...meta, format: 'WebP', width: 1 + (bits & 0x3fff), height: 1 + ((bits >> 14) & 0x3fff) }
      } else meta = { ...meta, format: 'WebP' }
    }
  } catch (err) {
    if (!(err instanceof RangeError)) throw err
  }
  meta.hasLocation = meta.entries.some((e) => e.sensitive)
  return meta
}
