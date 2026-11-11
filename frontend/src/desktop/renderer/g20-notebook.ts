// [g20] .ipynb içeriğini güvenli bir görünüm modeline çevirir (saf; DOM yok). Dosya/model çıktısı güvenilmezdir:
//      yalnızca tanınan alanlar okunur, metinler ve görseller boyutla sınırlanır, base64 karakter kümesi doğrulanır.
//      HTML/SVG/görsel çıktılar arayüzde SandboxedPreview ile izole gösterilir (bkz. NotebookBlock.tsx).

export type NbOutput =
  | { kind: 'stream'; stream: 'out' | 'err'; text: string }
  | { kind: 'error'; text: string }
  | { kind: 'html'; html: string }
  | { kind: 'image'; mime: string; base64: string }
  | { kind: 'svg'; svg: string }
  | { kind: 'markdown'; text: string }
  | { kind: 'text'; text: string }

export interface NbCell {
  type: 'code' | 'markdown' | 'raw'
  source: string
  count: number | null
  outputs: NbOutput[]
}

export interface NbView {
  language: string
  cells: NbCell[]
}

export const MAX_NB_CELLS = 500
const MAX_TEXT = 100_000
const MAX_IMAGE_B64 = 6_000_000
const RASTER = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] as const

const isRecord = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v)

/** Jupyter metni dize ya da satır dizisi olabilir. */
function joinText(value: unknown): string {
  if (typeof value === 'string') return value
  if (Array.isArray(value)) return value.filter((v): v is string => typeof v === 'string').join('')
  return ''
}

const clip = (text: string) => (text.length > MAX_TEXT ? `${text.slice(0, MAX_TEXT)}\n[çıktı kesildi]` : text)

/** Bir display_data/execute_result için Jupyter'e benzer öncelikle tek gösterim seçer. */
function pickData(data: Record<string, unknown>): NbOutput | null {
  const html = joinText(data['text/html'])
  if (html) return { kind: 'html', html: clip(html) }
  const svg = joinText(data['image/svg+xml'])
  if (svg) return { kind: 'svg', svg: clip(svg) }
  for (const mime of RASTER) {
    const raw = joinText(data[mime]).replace(/\s+/g, '')
    if (raw && raw.length <= MAX_IMAGE_B64 && /^[A-Za-z0-9+/]+={0,2}$/.test(raw)) return { kind: 'image', mime, base64: raw }
  }
  const markdown = joinText(data['text/markdown'])
  if (markdown) return { kind: 'markdown', text: clip(markdown) }
  const latex = joinText(data['text/latex'])
  if (latex) return { kind: 'text', text: clip(latex) }
  if (data['application/json'] !== undefined) {
    try {
      return { kind: 'text', text: clip(JSON.stringify(data['application/json'], null, 2)) }
    } catch {
      return { kind: 'text', text: '[JSON çıktısı gösterilemedi]' }
    }
  }
  const plain = joinText(data['text/plain'])
  return plain ? { kind: 'text', text: clip(plain) } : null
}

function normalizeOutput(raw: unknown): NbOutput | null {
  if (!isRecord(raw)) return null
  const type = raw.output_type
  if (type === 'stream') return { kind: 'stream', stream: raw.name === 'stderr' ? 'err' : 'out', text: clip(joinText(raw.text)) }
  if (type === 'error') {
    const trace = Array.isArray(raw.traceback) ? raw.traceback.filter((t): t is string => typeof t === 'string').join('\n') : ''
    const head = `${typeof raw.ename === 'string' ? raw.ename : 'Hata'}: ${typeof raw.evalue === 'string' ? raw.evalue : ''}`
    return { kind: 'error', text: clip(trace || head) }
  }
  if ((type === 'display_data' || type === 'execute_result') && isRecord(raw.data)) return pickData(raw.data)
  return null
}

/** @throws hücre listesi olmayan girdide */
export function normalizeNotebook(json: unknown): NbView {
  if (!isRecord(json) || !Array.isArray(json.cells)) throw new Error('Not defteri biçimi tanınmadı (cells dizisi yok)')
  const meta = isRecord(json.metadata) ? json.metadata : {}
  const kernel = isRecord(meta.kernelspec) ? meta.kernelspec.language : undefined
  const info = isRecord(meta.language_info) ? meta.language_info.name : undefined
  const language = typeof kernel === 'string' ? kernel : typeof info === 'string' ? info : 'python'
  const cells = json.cells.slice(0, MAX_NB_CELLS).flatMap((raw): NbCell[] => {
    if (!isRecord(raw)) return []
    const type = raw.cell_type === 'markdown' || raw.cell_type === 'raw' ? raw.cell_type : raw.cell_type === 'code' ? 'code' : null
    if (!type) return []
    const outputs = Array.isArray(raw.outputs) ? raw.outputs.map(normalizeOutput).filter((o): o is NbOutput => o !== null) : []
    const count = typeof raw.execution_count === 'number' ? raw.execution_count : null
    return [{ type, source: clip(joinText(raw.source)), count, outputs }]
  })
  return { language: language.toLowerCase(), cells }
}

/** Blok gövdesi: satır içi JSON mu yoksa çalışma dizinindeki .ipynb yolu mu? */
export function notebookSource(code: string): { kind: 'json'; text: string } | { kind: 'path'; path: string } | { kind: 'pending' } {
  const text = code.trim()
  if (text.startsWith('{')) return { kind: 'json', text }
  const path = text.split('\n', 1)[0]?.trim() ?? ''
  // Akış sırasında yarım yol için dosya istenmez.
  return /\.ipynb$/i.test(path) ? { kind: 'path', path } : { kind: 'pending' }
}
