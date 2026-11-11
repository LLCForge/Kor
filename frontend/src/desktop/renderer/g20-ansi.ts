// [g20] Çalıştırma çıktısı metin yardımcıları (saf; DOM yok): ANSI SGR renk kodlarını güvenli parçalara ayırma ve uzun
//      çıktıyı katlama. Metin React ile metin düğümü olarak çizilir, renkler yalnızca sınıf adıdır (g20-fg-N ...):
//      enjeksiyon yüzeyi yok. Renk dışı kaçış dizileri atılır.

export interface AnsiSegment {
  text: string
  /** Boşlukla ayrılmış sınıf adları; düz metinde boş dize */
  classes: string
}

interface Style {
  fg: number | null
  bg: number | null
  bold: boolean
  dim: boolean
  italic: boolean
  underline: boolean
}

const PLAIN: Style = { fg: null, bg: null, bold: false, dim: false, italic: false, underline: false }

// CSI (renk ve imleç komutları), OSC (başlık/bağlantı) ve tek karakterli kaçışlar.
const ESCAPE = /\u001b\[([0-9;:]*)([@-~])|\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)?|\u001b[@-Z\\-_]/g

function applySgr(style: Style, params: string): Style {
  const codes = params === '' ? [0] : params.split(/[;:]/).map((p) => (p === '' ? 0 : Number(p)))
  let next = { ...style }
  for (let i = 0; i < codes.length; i++) {
    const code = codes[i] ?? 0
    if (code === 0) next = { ...PLAIN }
    else if (code === 1) next.bold = true
    else if (code === 2) next.dim = true
    else if (code === 3) next.italic = true
    else if (code === 4) next.underline = true
    else if (code === 22) next = { ...next, bold: false, dim: false }
    else if (code === 23) next.italic = false
    else if (code === 24) next.underline = false
    else if (code >= 30 && code <= 37) next.fg = code - 30
    else if (code === 39) next.fg = null
    else if (code >= 40 && code <= 47) next.bg = code - 40
    else if (code === 49) next.bg = null
    else if (code >= 90 && code <= 97) next.fg = code - 90 + 8
    else if (code >= 100 && code <= 107) next.bg = code - 100 + 8
    else if (code === 38 || code === 48) {
      // 256 renkte yalnızca ilk 16 renk temaya eşlenir; 24 bit renk yok sayılır (parametreleri atlanır).
      const mode = codes[i + 1]
      if (mode === 5) {
        const n = codes[i + 2] ?? 0
        if (n < 16) next[code === 38 ? 'fg' : 'bg'] = n
        i += 2
      } else if (mode === 2) i += 4
    }
  }
  return next
}

function classesOf(style: Style): string {
  const out: string[] = []
  if (style.fg !== null) out.push(`g20-fg-${style.fg}`)
  if (style.bg !== null) out.push(`g20-bg-${style.bg}`)
  if (style.bold) out.push('g20-b')
  if (style.dim) out.push('g20-dim')
  if (style.italic) out.push('g20-i')
  if (style.underline) out.push('g20-u')
  return out.join(' ')
}

export function parseAnsi(input: string): AnsiSegment[] {
  const segments: AnsiSegment[] = []
  let style = PLAIN
  let last = 0
  const push = (text: string) => {
    if (!text) return
    const classes = classesOf(style)
    const prev = segments.at(-1)
    if (prev && prev.classes === classes) prev.text += text
    else segments.push({ text, classes })
  }
  for (const match of input.matchAll(ESCAPE)) {
    push(input.slice(last, match.index))
    last = match.index + match[0].length
    if (match[2] === 'm') style = applySgr(style, match[1] ?? '')
  }
  push(input.slice(last))
  return segments
}

export interface Chunk {
  stream: 'out' | 'err'
  text: string
}

/** Uzun çıktıyı katlar: ilk `limit` satır görünür; parçalar ve akış türleri (stdout/stderr) korunur. */
export function foldChunks(chunks: readonly Chunk[], limit: number): { visible: Chunk[]; hiddenLines: number } {
  const visible: Chunk[] = []
  let lines = 0
  let hidden = 0
  for (const chunk of chunks) {
    const parts = chunk.text.split('\n')
    const count = chunk.text.endsWith('\n') ? parts.length - 1 : parts.length
    if (lines >= limit) {
      hidden += count
      continue
    }
    const room = limit - lines
    if (count <= room) {
      visible.push(chunk)
      lines += count
    } else {
      visible.push({ stream: chunk.stream, text: `${parts.slice(0, room).join('\n')}\n` })
      hidden += count - room
      lines = limit
    }
  }
  return { visible, hiddenLines: hidden }
}

/** Kaçış dizileri olmadan düz metin (kopyalama için). */
export function stripAnsi(input: string): string {
  return input.replace(ESCAPE, '')
}
