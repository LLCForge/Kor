// [g17] Bağımlılıksız "store" (sıkıştırmasız) ZIP oluşturucu — toplu diyagram/görsel dışa aktarımı için.
//      PNG'ler zaten sıkıştırılmıştır; store yeterli ve ayrı bir sıkıştırma kütüphanesi gerektirmez.

export interface ZipEntry {
  name: string
  data: Uint8Array
}

const CRC_TABLE = (() => {
  const table = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = (c & 1) !== 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    table[n] = c >>> 0
  }
  return table
})()

/** Standart (IEEE) CRC-32; ZIP başlıkları için. */
export function crc32(data: Uint8Array): number {
  let c = 0xffffffff
  for (let i = 0; i < data.length; i++) c = (CRC_TABLE[(c ^ (data[i] ?? 0)) & 0xff] ?? 0) ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

const encoder = new TextEncoder()
const LOCAL_HEADER = 30
const CENTRAL_HEADER = 46
const EOCD = 22
// UTF-8 ad bayrağı (bit 11); 1980-01-01 sabit tarih (üretim kararlı olsun).
const FLAG_UTF8 = 0x0800
const DOS_DATE = 0x0021

/** Girişleri tek bir store-ZIP bayt dizisine paketler (saf; test edilebilir). */
export function zipBytes(entries: readonly ZipEntry[]): Uint8Array<ArrayBuffer> {
  const files = entries.map((e) => {
    const name = encoder.encode(e.name)
    return { name, data: e.data, crc: crc32(e.data) }
  })
  let total = EOCD
  for (const f of files) total += LOCAL_HEADER + f.name.length + f.data.length + CENTRAL_HEADER + f.name.length
  const buf = new Uint8Array(total)
  const view = new DataView(buf.buffer)
  const offsets: number[] = []
  let off = 0

  const u16 = (v: number) => {
    view.setUint16(off, v, true)
    off += 2
  }
  const u32 = (v: number) => {
    view.setUint32(off, v >>> 0, true)
    off += 4
  }

  for (const f of files) {
    offsets.push(off)
    u32(0x04034b50)
    u16(20) // sürüm
    u16(FLAG_UTF8)
    u16(0) // yöntem: store
    u16(0) // saat
    u16(DOS_DATE)
    u32(f.crc)
    u32(f.data.length) // sıkıştırılmış boyut
    u32(f.data.length) // gerçek boyut
    u16(f.name.length)
    u16(0) // ekstra
    buf.set(f.name, off)
    off += f.name.length
    buf.set(f.data, off)
    off += f.data.length
  }

  const cdStart = off
  files.forEach((f, i) => {
    u32(0x02014b50)
    u16(20) // üreten sürüm
    u16(20) // gereken sürüm
    u16(FLAG_UTF8)
    u16(0) // yöntem
    u16(0) // saat
    u16(DOS_DATE)
    u32(f.crc)
    u32(f.data.length)
    u32(f.data.length)
    u16(f.name.length)
    u16(0) // ekstra
    u16(0) // yorum
    u16(0) // disk
    u16(0) // iç öznitelik
    u32(0) // dış öznitelik
    u32(offsets[i] ?? 0)
    buf.set(f.name, off)
    off += f.name.length
  })

  const cdSize = off - cdStart
  u32(0x06054b50)
  u16(0) // disk
  u16(0) // merkezi dizinin başladığı disk
  u16(files.length)
  u16(files.length)
  u32(cdSize)
  u32(cdStart)
  u16(0) // yorum uzunluğu
  return buf
}

/** store-ZIP'i indirilebilir bir Blob olarak döndürür. */
export function makeZip(entries: readonly ZipEntry[]): Blob {
  const bytes = zipBytes(entries)
  return new Blob([bytes], { type: 'application/zip' })
}
