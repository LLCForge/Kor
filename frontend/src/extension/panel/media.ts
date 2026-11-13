// Galeri medyası (§20.3): dosya köprüden 512 KB'lık base64 parçalarla gelir (yerel mesajlaşma ana bilgisayardan
// tarayıcıya en çok 1 MB taşır) ve burada birleşir. Saf işlevler: node --test ile sınanır.

export interface GalleryItem {
  id: string
  kind: 'image' | 'video' | 'audio' | 'other'
  mime: string
  model: string
  prompt: string
  created: number
}

interface Chunk {
  mime: string
  size: number
  offset: number
  data: string
}

export type Request = (op: string, body: Record<string, unknown>) => Promise<unknown>

/** Öğenin tüm dosyasını parça parça alır. */
export async function fetchMedia(request: Request, id: string): Promise<Blob> {
  const parts: Uint8Array<ArrayBuffer>[] = []
  let offset = 0
  let size = Number.POSITIVE_INFINITY
  let mime = 'application/octet-stream'
  while (offset < size) {
    const c = (await request('galleryChunk', { id, offset })) as Chunk
    const bytes = Uint8Array.from(atob(c.data), (ch) => ch.charCodeAt(0))
    // Dosya okunurken kısaldıysa sonsuz döngü yerine hata.
    if (bytes.length === 0 && offset < c.size) throw new Error('short read')
    parts.push(bytes)
    size = c.size
    mime = c.mime
    offset += bytes.length
  }
  return new Blob(parts, { type: mime })
}

/** İndirilen dosyanın adı: galeri kimliği zaten dosya adıdır; yol ayırıcıları atılır. */
export function downloadName(id: string): string {
  return id.replace(/[\\/:*?"<>|]/g, '_') || 'kor-media'
}
