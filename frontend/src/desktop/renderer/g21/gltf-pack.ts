// [g21] 3D model paketleme (#182): .gltf/.glb ve yerel dış kaynaklarını (ayrı .bin ve doku dosyaları) tek bir
//      GLB'ye çevirir. Neden: CSP `connect-src 'none'` yükleyicinin fetch'ini (blob:/data: dahil) engeller.
//      Tüm tamponlar tek BIN bölümünde birleştirilir, dokular data: URI'ye çevrilir (img-src data: izinli);
//      böylece GLTFLoader hiçbir ağ/dosya isteği yapmadan çalışır. Saf ve test edilebilir.
import { z } from 'zod'

export const MAX_MODEL_BYTES = 200 * 1024 * 1024
export const MAX_EXTERNAL_RESOURCES = 64

const GLB_MAGIC = 0x46546c67
const CHUNK_JSON = 0x4e4f534a
const CHUNK_BIN = 0x004e4942
const IMAGE_MIME = /^image\/(png|jpeg|webp)$/

const GltfSchema = z.looseObject({
  asset: z.looseObject({ version: z.string() }),
  buffers: z.array(z.looseObject({ uri: z.string().optional(), byteLength: z.number().int().nonnegative() })).optional(),
  bufferViews: z
    .array(z.looseObject({ buffer: z.number().int().nonnegative(), byteOffset: z.number().int().nonnegative().optional(), byteLength: z.number().int().nonnegative() }))
    .optional(),
  images: z.array(z.looseObject({ uri: z.string().optional(), bufferView: z.number().int().nonnegative().optional(), mimeType: z.string().optional() })).optional(),
})
export type GltfJson = z.infer<typeof GltfSchema>

export interface ParsedModel {
  json: GltfJson
  bin: Uint8Array | null
}

/** .glb ikilisini ya da .gltf JSON metnini ayrıştırır ve şemayı doğrular. */
export function parseModel(bytes: Uint8Array): ParsedModel {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let jsonText: string
  let bin: Uint8Array | null = null
  if (bytes.length >= 12 && dv.getUint32(0, true) === GLB_MAGIC) {
    if (dv.getUint32(4, true) !== 2) throw new Error('Yalnızca glTF 2.0 desteklenir')
    const total = Math.min(dv.getUint32(8, true), bytes.length)
    let p = 12
    jsonText = ''
    while (p + 8 <= total) {
      const len = dv.getUint32(p, true)
      const type = dv.getUint32(p + 4, true)
      if (p + 8 + len > total) throw new Error('GLB bölümü dosya sonunu aşıyor')
      const data = bytes.subarray(p + 8, p + 8 + len)
      if (type === CHUNK_JSON && !jsonText) jsonText = new TextDecoder().decode(data)
      else if (type === CHUNK_BIN && !bin) bin = data
      p += 8 + len
    }
    if (!jsonText) throw new Error('GLB içinde JSON bölümü yok')
  } else {
    jsonText = new TextDecoder().decode(bytes)
  }
  let raw: unknown
  try {
    raw = JSON.parse(jsonText)
  } catch {
    throw new Error('Model JSON içeriği bozuk')
  }
  const parsed = GltfSchema.safeParse(raw)
  if (!parsed.success) throw new Error('Geçerli bir glTF 2.0 modeli değil')
  if (!parsed.data.asset.version.startsWith('2')) throw new Error('Yalnızca glTF 2.0 desteklenir')
  return { json: parsed.data, bin }
}

const isDataUri = (uri: string) => /^data:/i.test(uri)

/** Modelin dosya sisteminden okunması gereken (data: olmayan) tampon ve doku referansları. */
export function externalUris(json: GltfJson): string[] {
  const uris = new Set<string>()
  for (const b of json.buffers ?? []) if (b.uri && !isDataUri(b.uri)) uris.add(b.uri)
  for (const img of json.images ?? []) if (img.uri && !isDataUri(img.uri) && img.bufferView === undefined) uris.add(img.uri)
  if (uris.size > MAX_EXTERNAL_RESOURCES) throw new Error(`Model çok fazla dış kaynak kullanıyor (sınır ${MAX_EXTERNAL_RESOURCES})`)
  return [...uris]
}

/** data: URI'yi çözer (yalnızca base64). */
export function decodeDataUri(uri: string): { mime: string; bytes: Uint8Array } {
  const m = /^data:([^;,]*)(?:;[^,]*)*;base64,(.*)$/is.exec(uri)
  if (!m) throw new Error('Yalnızca base64 kodlu data: URI desteklenir')
  const bin = atob((m[2] ?? '').replace(/\s/g, ''))
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return { mime: m[1] ?? '', bytes }
}

export function toBase64(bytes: Uint8Array): string {
  let bin = ''
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return btoa(bin)
}

/** İçerik imzasından doku MIME türü (uzantıya güvenilmez). */
export function sniffImageMime(bytes: Uint8Array): string | null {
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return 'image/png'
  if (bytes[0] === 0xff && bytes[1] === 0xd8) return 'image/jpeg'
  if (String.fromCharCode(...bytes.subarray(0, 4)) === 'RIFF' && String.fromCharCode(...bytes.subarray(8, 12)) === 'WEBP') return 'image/webp'
  return null
}

const align4 = (n: number) => (n + 3) & ~3

/**
 * Modeli tek BIN bölümlü bir GLB'ye paketler. `external` dış referansların (uri -> bayt) okunmuş halidir.
 * Desteklenmeyen doku türleri (ör. KTX2) data: URI'ye çevrilmez; yükleyici bunları hata olarak bildirir.
 */
export function packGlb(model: ParsedModel, external: ReadonlyMap<string, Uint8Array>): Uint8Array {
  const json = structuredClone(model.json)
  const buffers = json.buffers ?? []
  const parts: Uint8Array[] = buffers.map((b, i) => {
    let data: Uint8Array | undefined
    if (b.uri === undefined) data = i === 0 && model.bin ? model.bin : undefined
    else if (isDataUri(b.uri)) data = decodeDataUri(b.uri).bytes
    else data = external.get(b.uri)
    if (!data) throw new Error(`Model tamponu bulunamadı: ${b.uri ?? `#${i}`}`)
    if (data.length < b.byteLength) throw new Error(`Model tamponu eksik: ${b.uri ?? `#${i}`}`)
    return data.subarray(0, b.byteLength)
  })
  const offsets: number[] = []
  let total = 0
  for (const part of parts) {
    offsets.push(total)
    total = align4(total + part.length)
  }
  if (total > MAX_MODEL_BYTES) throw new Error('Model önizleme için çok büyük')
  const merged = new Uint8Array(total)
  parts.forEach((part, i) => merged.set(part, offsets[i] ?? 0))

  for (const view of json.bufferViews ?? []) {
    const base = offsets[view.buffer]
    const part = parts[view.buffer]
    if (base === undefined || !part) throw new Error('bufferView geçersiz tampona başvuruyor')
    const start = view.byteOffset ?? 0
    if (start + view.byteLength > part.length) throw new Error('bufferView tampon sınırını aşıyor')
    view.buffer = 0
    view.byteOffset = base + start
  }

  for (const image of json.images ?? []) {
    let bytes: Uint8Array | undefined
    let mime = image.mimeType ?? ''
    if (image.bufferView !== undefined) {
      const view = json.bufferViews?.[image.bufferView]
      if (!view) throw new Error('Doku geçersiz bufferView kullanıyor')
      bytes = merged.subarray(view.byteOffset ?? 0, (view.byteOffset ?? 0) + view.byteLength)
    } else if (image.uri && !isDataUri(image.uri)) bytes = external.get(image.uri)
    else if (image.uri) continue // zaten data: URI
    if (!bytes) throw new Error(`Doku bulunamadı: ${image.uri ?? '(gömülü)'}`)
    mime = sniffImageMime(bytes) ?? mime
    if (!IMAGE_MIME.test(mime)) continue
    image.uri = `data:${mime};base64,${toBase64(bytes)}`
    delete image.bufferView
    image.mimeType = mime
  }

  json.buffers = total > 0 ? [{ byteLength: total }] : []
  const jsonBytes = new TextEncoder().encode(JSON.stringify(json))
  const jsonLen = align4(jsonBytes.length)
  const glbLen = 12 + 8 + jsonLen + (total > 0 ? 8 + total : 0)
  const out = new Uint8Array(glbLen)
  const dv = new DataView(out.buffer)
  dv.setUint32(0, GLB_MAGIC, true)
  dv.setUint32(4, 2, true)
  dv.setUint32(8, glbLen, true)
  dv.setUint32(12, jsonLen, true)
  dv.setUint32(16, CHUNK_JSON, true)
  out.set(jsonBytes, 20)
  out.fill(0x20, 20 + jsonBytes.length, 20 + jsonLen)
  if (total > 0) {
    const binAt = 20 + jsonLen
    dv.setUint32(binAt, total, true)
    dv.setUint32(binAt + 4, CHUNK_BIN, true)
    out.set(merged, binAt + 8)
  }
  return out
}
