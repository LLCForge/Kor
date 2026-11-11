// [g11] Yerel proje dizini: kod tabanını bildirim sınırlarından parçalara böler ve sorguya en ilgili parçaları BM25 ile sıralar.
// Bilinçli olarak SÖZCÜKSEL (anlamsal gömme değil): yerel gömme modeli büyük indirme veya native bağımlılık ister.
// Ağ yok; dizin bellekte tutulur ve her aramada yalnızca değişen dosyalar yeniden okunur.
import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { isIgnored, layersFor, withLayer, type IgnoreLayer } from './gitignore.ts'
import { fold } from './intel-g11.ts'
import { isSensitivePath } from './permissions.ts'

export const INDEX_LIMITS = { maxFiles: 5000, maxFileBytes: 256 * 1024, maxTotalBytes: 40 * 1024 * 1024 }
const MAX_CACHED_ROOTS = 4
const SNIPPET_LINES = 14
const MAX_LINE_CHARS = 200
const K1 = 1.2
const B = 0.75
/** Dosya yolundaki terimler parçadaki her terimden ağır basar: "auth" araması auth/ altını öne çıkarsın. */
const PATH_WEIGHT = 2

const SKIPPED_DIRS = new Set(['node_modules', 'bower_components', 'vendor', 'dist', 'build', 'out', 'target', 'coverage', '__pycache__', 'venv'])
const SKIPPED_FILES = new Set(['package-lock.json', 'yarn.lock', 'pnpm-lock.yaml', 'bun.lockb', 'cargo.lock', 'poetry.lock', 'composer.lock', 'gemfile.lock'])
const BINARY_EXTS = new Set([
  '.png', '.jpg', '.jpeg', '.gif', '.webp', '.ico', '.bmp', '.tiff', '.psd', '.pdf', '.zip', '.gz', '.tgz', '.7z', '.rar', '.tar',
  '.exe', '.dll', '.so', '.dylib', '.bin', '.obj', '.o', '.a', '.lib', '.class', '.jar', '.wasm', '.pyc', '.woff', '.woff2', '.ttf',
  '.otf', '.eot', '.mp3', '.mp4', '.wav', '.ogg', '.webm', '.mov', '.avi', '.sqlite', '.db', '.map',
])
// Satır başında (girintisiz) bildirim: parça sınırı burada açılır.
const DECLARATION =
  /^(?:export\s+)?(?:default\s+)?(?:async\s+)?(?:function|class|interface|type|enum|const|let|var|def|fn|func|impl|struct|trait|module|namespace|public|private|protected|internal|static)\b|^#{1,3}\s/

interface Chunk {
  start: number
  end: number
  tf: Map<string, number>
  length: number
}

interface FileEntry {
  mtimeMs: number
  size: number
  chunks: Chunk[]
}

interface RootIndex {
  files: Map<string, FileEntry>
}

const indexes = new Map<string, RootIndex>()

/** Tanımlayıcıları bütün ve parçalarıyla verir: "createSession" → createsession, create, session. */
export function indexTerms(text: string): string[] {
  const out: string[] = []
  for (const [word] of text.matchAll(/[\p{L}\p{N}_$]{2,64}/gu)) {
    const whole = fold(word.replace(/[_$]/g, ''))
    if (whole.length >= 2) out.push(whole)
    const parts = word.split(/[_$]+|(?<=[\p{Ll}\p{N}])(?=\p{Lu})|(?<=\p{Lu})(?=\p{Lu}\p{Ll})/u).filter((p) => p.length >= 2)
    if (parts.length > 1) for (const p of parts) out.push(fold(p))
  }
  return out
}

/** [başlangıç, bitiş) satır aralıkları: en az min satırdan sonra ilk girintisiz bildirimde, en geç max satırda bölünür. */
export function chunkRanges(lines: readonly string[], min = 12, max = 60): Array<[number, number]> {
  const ranges: Array<[number, number]> = []
  let start = 0
  for (let i = 1; i < lines.length; i++) {
    const size = i - start
    if ((size >= min && DECLARATION.test(lines[i] ?? '')) || size >= max) {
      ranges.push([start, i])
      start = i
    }
  }
  if (start < lines.length) ranges.push([start, lines.length])
  return ranges
}

function buildChunks(rel: string, content: string): Chunk[] {
  const lines = content.replace(/\r?\n$/, '').split(/\r?\n/)
  const pathTerms = indexTerms(rel.replace(/\.[^./]+$/, ''))
  return chunkRanges(lines).map(([start, end]) => {
    const tf = new Map<string, number>()
    let length = 0
    for (const term of indexTerms(lines.slice(start, end).join('\n'))) {
      tf.set(term, (tf.get(term) ?? 0) + 1)
      length++
    }
    for (const term of pathTerms) tf.set(term, (tf.get(term) ?? 0) + PATH_WEIGHT)
    return { start, end, tf, length: length + pathTerms.length * PATH_WEIGHT }
  })
}

interface WalkedFile {
  rel: string
  size: number
  mtimeMs: number
}

interface WalkResult {
  files: WalkedFile[]
  skipped: number
  truncated: boolean
}

function isBinaryName(name: string): boolean {
  const lower = name.toLowerCase()
  return BINARY_EXTS.has(path.extname(lower)) || lower.endsWith('.min.js')
}

/** .gitignore'a uyar; sembolik bağlar izlenmez (Dirent onları dosya/klasör saymaz), hassas dosyalar listeye girmez. */
async function walk(root: string): Promise<WalkResult> {
  const out: WalkResult = { files: [], skipped: 0, truncated: false }
  let totalBytes = 0
  const queue: Array<{ dir: string; layers: IgnoreLayer[] }> = [{ dir: '', layers: await layersFor(root, '') }]
  while (queue.length > 0 && !out.truncated) {
    const next = queue.shift()
    if (!next) break
    let entries
    try {
      entries = await readdir(path.join(root, next.dir), { withFileTypes: true })
    } catch (err) {
      if (next.dir === '') throw err
      console.warn('g11 dizin: klasör okunamadı:', next.dir, err instanceof Error ? err.message : err)
      continue
    }
    entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    const candidates: string[] = []
    for (const e of entries) {
      const rel = next.dir ? `${next.dir}/${e.name}` : e.name
      if (e.isDirectory()) {
        if (e.name.startsWith('.') || SKIPPED_DIRS.has(e.name) || isIgnored(next.layers, rel, true)) continue
        queue.push({ dir: rel, layers: await withLayer(root, rel, next.layers) })
      } else if (e.isFile()) {
        if (isIgnored(next.layers, rel, false) || isSensitivePath(rel) || SKIPPED_FILES.has(e.name.toLowerCase()) || isBinaryName(e.name)) continue
        candidates.push(rel)
      }
    }
    const stats = await Promise.all(
      candidates.map(async (rel) => {
        try {
          const info = await stat(path.join(root, rel))
          return { rel, size: info.size, mtimeMs: info.mtimeMs }
        } catch {
          return null // Tarama ile stat arasında silinmiş.
        }
      }),
    )
    for (const file of stats) {
      if (!file) continue
      if (file.size > INDEX_LIMITS.maxFileBytes) {
        out.skipped++
        continue
      }
      if (out.files.length >= INDEX_LIMITS.maxFiles || totalBytes + file.size > INDEX_LIMITS.maxTotalBytes) {
        out.truncated = true
        break
      }
      totalBytes += file.size
      out.files.push(file)
    }
  }
  return out
}

const rootKey = (root: string) => {
  const abs = path.resolve(root)
  return process.platform === 'win32' ? abs.toLowerCase() : abs
}

/** Dizini günceller: boyutu ve değişiklik zamanı aynı kalan dosyalar yeniden okunmaz. */
async function refresh(root: string): Promise<{ index: RootIndex; walked: WalkResult }> {
  const key = rootKey(root)
  const previous = indexes.get(key)
  const walked = await walk(root)
  const files = new Map<string, FileEntry>()
  for (const f of walked.files) {
    const old = previous?.files.get(f.rel)
    if (old && old.mtimeMs === f.mtimeMs && old.size === f.size) {
      files.set(f.rel, old)
      continue
    }
    let content: string
    try {
      content = await readFile(path.join(root, f.rel), 'utf8')
    } catch {
      continue
    }
    if (content.includes('\u0000')) continue // İkili dosya
    files.set(f.rel, { mtimeMs: f.mtimeMs, size: f.size, chunks: buildChunks(f.rel, content) })
  }
  const index = { files }
  indexes.delete(key)
  indexes.set(key, index)
  if (indexes.size > MAX_CACHED_ROOTS) indexes.delete(indexes.keys().next().value ?? '')
  return { index, walked }
}

export interface SearchHit {
  path: string
  /** 1 tabanlı, dahil */
  startLine: number
  endLine: number
  score: number
}

export interface SearchOutcome {
  hits: SearchHit[]
  files: number
  chunks: number
  skipped: number
  truncated: boolean
}

/** pathPrefix: köke göre '/' ayraçlı klasör; yalnızca onun altındaki parçalar sıralanır. */
export async function searchIndex(root: string, query: string, limit = 8, pathPrefix = ''): Promise<SearchOutcome> {
  const { index, walked } = await refresh(root)
  const terms = [...new Set(indexTerms(query))]
  const prefix = pathPrefix.replace(/\\/g, '/').replace(/^\.\/?/, '').replace(/\/+$/, '')
  const candidates: Array<{ path: string; chunk: Chunk }> = []
  let totalLength = 0
  for (const [rel, entry] of index.files) {
    if (prefix && rel !== prefix && !rel.startsWith(`${prefix}/`)) continue
    for (const chunk of entry.chunks) {
      candidates.push({ path: rel, chunk })
      totalLength += chunk.length
    }
  }
  const outcome = { files: index.files.size, chunks: candidates.length, skipped: walked.skipped, truncated: walked.truncated }
  if (terms.length === 0 || candidates.length === 0) return { hits: [], ...outcome }

  const n = candidates.length
  const avg = totalLength / n || 1
  const idf = new Map<string, number>()
  for (const term of terms) {
    let df = 0
    for (const c of candidates) if (c.chunk.tf.has(term)) df++
    idf.set(term, Math.log(1 + (n - df + 0.5) / (df + 0.5)))
  }
  const scored: SearchHit[] = []
  for (const { path: rel, chunk } of candidates) {
    let score = 0
    for (const term of terms) {
      const tf = chunk.tf.get(term)
      if (!tf) continue
      score += ((idf.get(term) ?? 0) * tf * (K1 + 1)) / (tf + K1 * (1 - B + (B * chunk.length) / avg))
    }
    if (score > 0) scored.push({ path: rel, startLine: chunk.start + 1, endLine: chunk.end, score })
  }
  scored.sort((a, b) => b.score - a.score)
  return { hits: scored.slice(0, limit), ...outcome }
}

/** Modelin göreceği metin: her sonuç için yol:satırlar, puan ve kısa bir kesit. */
export async function formatSearch(root: string, query: string, outcome: SearchOutcome): Promise<string> {
  const footer = `[yerel sözcüksel (BM25) dizin: ${outcome.files} dosya, ${outcome.chunks} parça${outcome.skipped ? `, ${outcome.skipped} büyük dosya atlandı` : ''}${outcome.truncated ? ', dosya sınırına ulaşıldı' : ''}]`
  if (outcome.hits.length === 0) return `"${query}" için eşleşme yok. Kod terimleriyle (İngilizce tanımlayıcılar, eş anlamlılar) yeniden dene.\n${footer}`
  const blocks: string[] = []
  for (const [i, hit] of outcome.hits.entries()) {
    let snippet = ''
    try {
      const lines = (await readFile(path.join(root, hit.path), 'utf8')).split(/\r?\n/)
      const last = Math.min(hit.endLine, hit.startLine + SNIPPET_LINES - 1)
      snippet = lines
        .slice(hit.startLine - 1, last)
        .map((l, j) => `${hit.startLine + j}: ${l.length > MAX_LINE_CHARS ? `${l.slice(0, MAX_LINE_CHARS)}…` : l}`)
        .join('\n')
      if (last < hit.endLine) snippet += '\n…'
    } catch {
      snippet = '(dosya okunamadı)'
    }
    blocks.push(`${i + 1}. ${hit.path}:${hit.startLine}-${hit.endLine} (puan ${hit.score.toFixed(2)})\n${snippet}`)
  }
  return `${blocks.join('\n\n')}\n\n${footer}`
}
