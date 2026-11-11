import type { Dirent } from 'node:fs'
import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { isSensitivePath } from './permissions.ts'

export const MAX_IMAGE_BYTES = 5 * 1024 * 1024
export const IMAGE_MIME_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'] as const
export type ImageMime = (typeof IMAGE_MIME_TYPES)[number]

export const TREE_MAX_DEPTH = 3
export const TREE_MAX_ENTRIES = 300
const IGNORED_DIRS = new Set(['node_modules', '.git'])

const startsWith = (bytes: Uint8Array, signature: readonly number[], offset = 0) =>
  signature.every((b, i) => bytes[offset + i] === b)
const ascii = (text: string) => [...text].map((c) => c.charCodeAt(0))

/** Dosya içeriğinin ilk baytlarından görsel türünü belirler; uzantıya veya beyan edilen türe güvenilmez. */
export function sniffImageMime(bytes: Uint8Array): ImageMime | null {
  if (startsWith(bytes, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return 'image/png'
  if (startsWith(bytes, [0xff, 0xd8, 0xff])) return 'image/jpeg'
  if (startsWith(bytes, ascii('GIF87a')) || startsWith(bytes, ascii('GIF89a'))) return 'image/gif'
  if (startsWith(bytes, ascii('RIFF')) && startsWith(bytes, ascii('WEBP'), 8)) return 'image/webp'
  return null
}

export interface ValidatedImage {
  mime: ImageMime
  /** Sihirli baytlardan bulunan türle yeniden kurulan data URL */
  dataUrl: string
  size: number
}

function validateImageBytes(bytes: Buffer, label: string, declared?: string): ValidatedImage {
  if (bytes.length === 0) throw new Error(`${label}: görsel boş`)
  if (bytes.length > MAX_IMAGE_BYTES) throw new Error(`${label}: görsel çok büyük (sınır 5 MB)`)
  const mime = sniffImageMime(bytes)
  if (!mime) throw new Error(`${label}: desteklenmeyen görsel biçimi (yalnızca PNG, JPEG, WebP, GIF)`)
  const normalized = declared === 'image/jpg' ? 'image/jpeg' : declared
  if (normalized !== undefined && normalized !== mime) throw new Error(`${label}: beyan edilen tür (${declared}) içerikle uyuşmuyor`)
  return { mime, dataUrl: `data:${mime};base64,${bytes.toString('base64')}`, size: bytes.length }
}

const DATA_URL = /^data:([a-z]+\/[a-z0-9.+-]+);base64,([A-Za-z0-9+/]+={0,2})$/

/** Arayüzden gelen (güvenilmez) data URL'yi çözer; boyut ve sihirli bayt doğrulaması yapar. */
export function parseImageDataUrl(dataUrl: string, label = 'görsel'): ValidatedImage {
  // Kodlanmış boyut kabaca 4/3 katıdır; çözmeden önce kaba sınır bellek tüketimini önler.
  if (dataUrl.length > Math.ceil((MAX_IMAGE_BYTES * 4) / 3) + 100) throw new Error(`${label}: görsel çok büyük (sınır 5 MB)`)
  const match = DATA_URL.exec(dataUrl)
  if (!match?.[1] || !match[2]) throw new Error(`${label}: geçersiz görsel verisi`)
  return validateImageBytes(Buffer.from(match[2], 'base64'), label, match[1])
}

/** Mutlak yoldaki görsel dosyasını okur ve doğrular (yol çağıran tarafından çalışma dizinine hapsedilmiş olmalı). */
export async function readImageFile(abs: string, label = path.basename(abs)): Promise<ValidatedImage> {
  const info = await stat(abs)
  if (!info.isFile()) throw new Error(`${label}: dosya değil`)
  if (info.size > MAX_IMAGE_BYTES) throw new Error(`${label}: görsel çok büyük (sınır 5 MB)`)
  return validateImageBytes(await readFile(abs), label)
}

export const looksLikeImagePath = (p: string) => /\.(png|jpe?g|gif|webp)$/i.test(p)

const toPosix = (p: string) => p.split(path.sep).join('/')

async function sortedEntries(dir: string): Promise<Dirent[]> {
  const entries = await readdir(dir, { withFileTypes: true })
  return entries.sort((a, b) => Number(b.isDirectory()) - Number(a.isDirectory()) || a.name.localeCompare(b.name, 'tr'))
}

/**
 * Klasörü <tree path="..."> bloğu olarak döner: derinlik ve giriş sayısı sınırlı,
 * node_modules/.git ve hassas dosyalar hariç. Sembolik bağlantıların içine girilmez (çalışma dizininden kaçmasın).
 */
export async function buildDirectoryTree(absDir: string, relDir: string): Promise<string> {
  const label = toPosix(relDir).replace(/\/+$/, '') || '.'
  const lines: string[] = [`${label}/`]
  let count = 0
  let truncated = false

  const walk = async (dir: string, depth: number): Promise<void> => {
    for (const entry of await sortedEntries(dir)) {
      if (count >= TREE_MAX_ENTRIES) {
        truncated = true
        return
      }
      const isDir = entry.isDirectory()
      if (isDir && IGNORED_DIRS.has(entry.name)) continue
      if (!isDir && isSensitivePath(entry.name)) continue
      count++
      lines.push(`${'  '.repeat(depth)}${entry.name}${isDir ? '/' : ''}`)
      if (isDir && depth < TREE_MAX_DEPTH) await walk(path.join(dir, entry.name), depth + 1)
    }
  }
  await walk(absDir, 1)
  if (truncated) lines.push(`[${TREE_MAX_ENTRIES} giriş sınırına ulaşıldı; liste kesildi]`)
  return `<tree path="${label}">\n${lines.join('\n')}\n</tree>`
}

export interface WorkspaceEntry {
  /** Çalışma dizinine göre, '/' ayraçlı */
  path: string
  isDir: boolean
}

/** @-tamamlama için dizin taraması; node_modules, .git, sembolik bağlantılar ve hassas dosyalar hariç. */
export async function walkWorkspace(root: string, maxEntries = 20_000, maxDepth = 12): Promise<WorkspaceEntry[]> {
  const out: WorkspaceEntry[] = []
  const queue: { abs: string; rel: string; depth: number }[] = [{ abs: root, rel: '', depth: 0 }]
  // Genişlik öncelikli: sınıra ulaşılırsa sığ (daha olası) yollar listede kalır.
  for (let i = 0; i < queue.length && out.length < maxEntries; i++) {
    const { abs, rel, depth } = queue[i] as { abs: string; rel: string; depth: number }
    let entries: Dirent[]
    try {
      entries = await readdir(abs, { withFileTypes: true })
    } catch (err) {
      // Okunamayan alt klasör (izin vb.) taramayı durdurmaz.
      console.warn('Klasör taranamadı:', rel || '.', err instanceof Error ? err.message : err)
      continue
    }
    for (const entry of entries) {
      if (entry.isSymbolicLink()) continue
      const childRel = rel ? `${rel}/${entry.name}` : entry.name
      if (entry.isDirectory()) {
        if (IGNORED_DIRS.has(entry.name)) continue
        out.push({ path: childRel, isDir: true })
        if (depth + 1 < maxDepth) queue.push({ abs: path.join(abs, entry.name), rel: childRel, depth: depth + 1 })
      } else if (entry.isFile() && !isSensitivePath(entry.name)) {
        out.push({ path: childRel, isDir: false })
      }
      if (out.length >= maxEntries) break
    }
  }
  return out
}
