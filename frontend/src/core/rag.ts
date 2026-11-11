// RAG belge araması: çalışma dizinindeki metin dosyalarını parçalara bölüp gömme vektörleriyle indeksler; sorgu
// vektörüne kosinüs benzerliğiyle en yakın parçaları döner (anlamsal arama). İndeks .kor/rag-index.json'da
// önbelleklenir; dosya listesi/boyut/mtime değişince yeniden kurulur. Gömme, oturumun sağladığı embed yeteneğiyle
// yapılır (anahtar araçlara sızmaz). İş alanı içeriği Venice'e gömme için gönderilir (sohbetle aynı sağlayıcı).
import { createHash } from 'node:crypto'
import { glob, mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { projectDotDir } from './layout-g28.ts' // [g28]

const CHUNK_CHARS = 1500
const OVERLAP = 200
const MAX_FILE_BYTES = 400_000
const MAX_FILES = 800
const MAX_CHUNKS = 1200
const IGNORED_DIRS = ['node_modules', '.git', '.kor', '.venice', 'dist', 'paketler', 'build', 'out', '.next', 'coverage']
const TEXT_EXT = new Set([
  'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'json', 'md', 'markdown', 'txt', 'py', 'go', 'rs', 'java', 'kt', 'c', 'cc', 'cpp', 'h', 'hpp', 'cs',
  'rb', 'php', 'swift', 'scala', 'sh', 'bash', 'ps1', 'css', 'scss', 'less', 'html', 'xml', 'yml', 'yaml', 'toml', 'ini', 'sql', 'vue', 'svelte',
])

export interface RagChunk {
  file: string
  startLine: number
  text: string
}
export interface RagIndex {
  model: string
  signature: string
  chunks: RagChunk[]
  vectors: number[][]
}
export type EmbedFn = (texts: readonly string[]) => Promise<number[][]>

function toPosix(p: string): string {
  return p.split(path.sep).join('/')
}

async function collectTextFiles(root: string): Promise<string[]> {
  const out: string[] = []
  try {
    for await (const entry of glob('**/*', { cwd: root, exclude: (p) => IGNORED_DIRS.some((d) => p.split(/[\\/]/).includes(d)) })) {
      if (out.length >= MAX_FILES) break
      const ext = entry.slice(entry.lastIndexOf('.') + 1).toLowerCase()
      if (!TEXT_EXT.has(ext)) continue
      try {
        if ((await stat(path.join(root, entry))).size <= MAX_FILE_BYTES) out.push(toPosix(entry))
      } catch {
        // okunamıyorsa atla
      }
    }
  } catch {
    // glob desteklenmiyorsa boş
  }
  return out.sort()
}

async function signatureOf(root: string, files: readonly string[]): Promise<string> {
  const parts: string[] = []
  for (const f of files) {
    try {
      const s = await stat(path.join(root, f))
      parts.push(`${f}:${s.size}:${Math.round(s.mtimeMs)}`)
    } catch {
      // atla
    }
  }
  return createHash('sha256').update(parts.join('\n')).digest('hex')
}

export function chunkText(file: string, text: string): RagChunk[] {
  const chunks: RagChunk[] = []
  let pos = 0
  while (pos < text.length) {
    const end = Math.min(text.length, pos + CHUNK_CHARS)
    const startLine = pos === 0 ? 1 : text.slice(0, pos).split('\n').length
    const slice = text.slice(pos, end).trim()
    if (slice) chunks.push({ file, startLine, text: slice })
    if (end >= text.length) break
    pos = end - OVERLAP
  }
  return chunks
}

async function buildRagIndex(root: string, files: readonly string[], signature: string, embed: EmbedFn, model: string): Promise<RagIndex> {
  const chunks: RagChunk[] = []
  for (const f of files) {
    if (chunks.length >= MAX_CHUNKS) break
    let text: string
    try {
      text = await readFile(path.join(root, f), 'utf8')
    } catch {
      continue
    }
    if (text.includes('\0')) continue // ikili dosya
    for (const c of chunkText(f, text)) {
      if (chunks.length >= MAX_CHUNKS) break
      chunks.push(c)
    }
  }
  const vectors = chunks.length > 0 ? await embed(chunks.map((c) => c.text)) : []
  const index: RagIndex = { model, signature, chunks: chunks.slice(0, vectors.length), vectors }
  const dot = projectDotDir(root)
  await mkdir(dot, { recursive: true })
  await writeFile(path.join(dot, 'rag-index.json'), JSON.stringify(index))
  return index
}

export async function loadRagIndex(root: string): Promise<RagIndex | null> {
  try {
    const parsed = JSON.parse(await readFile(path.join(projectDotDir(root), 'rag-index.json'), 'utf8')) as Partial<RagIndex>
    if (typeof parsed.model === 'string' && typeof parsed.signature === 'string' && Array.isArray(parsed.chunks) && Array.isArray(parsed.vectors) && parsed.chunks.length === parsed.vectors.length) {
      return parsed as RagIndex
    }
  } catch {
    // yok ya da bozuk
  }
  return null
}

/** İndeksi yükler; yoksa ya da bayatsa (dosya listesi/boyut/mtime değişmiş) yeniden kurar. */
export async function ensureRagIndex(root: string, embed: EmbedFn, model = 'text-embedding-bge-m3'): Promise<RagIndex> {
  const files = await collectTextFiles(root)
  const signature = await signatureOf(root, files)
  const existing = await loadRagIndex(root)
  if (existing && existing.signature === signature && existing.model === model) return existing
  return buildRagIndex(root, files, signature, embed, model)
}

function cosine(a: readonly number[], b: readonly number[]): number {
  let dot = 0
  let na = 0
  let nb = 0
  const n = Math.min(a.length, b.length)
  for (let i = 0; i < n; i++) {
    dot += a[i]! * b[i]!
    na += a[i]! * a[i]!
    nb += b[i]! * b[i]!
  }
  return na === 0 || nb === 0 ? 0 : dot / (Math.sqrt(na) * Math.sqrt(nb))
}

export function searchRag(index: RagIndex, queryVector: readonly number[], k: number): Array<RagChunk & { score: number }> {
  return index.vectors
    .map((v, i) => ({ ...index.chunks[i]!, score: cosine(queryVector, v) }))
    .sort((a, b) => b.score - a.score)
    .slice(0, k)
}
