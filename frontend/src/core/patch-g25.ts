// [g25] Çok dosyalı yama. Codex / OpenCode "*** Begin Patch" zarfının davranışı.
// Temiz oda: satır satır çeviri değil. Ekle, sil, güncelle, taşı. Eşleşme yoksa disk değişmez.

export type PatchChunk = {
  oldLines: string[]
  newLines: string[]
  changeContext?: string
}

export type PatchHunk =
  | { type: 'add'; path: string; contents: string }
  | { type: 'delete'; path: string }
  | { type: 'update'; path: string; movePath?: string; chunks: PatchChunk[] }

export function parsePatch(input: string): PatchHunk[] {
  const text = stripFence(stripHeredoc(input.trim()))
  const lines = text.split('\n').map((line) => (line.endsWith('\r') ? line.slice(0, -1) : line))
  if (lines[0]?.trim() !== '*** Begin Patch') throw new Error('Yama "*** Begin Patch" ile başlamalı.')
  if (lines.at(-1)?.trim() !== '*** End Patch') throw new Error('Yama "*** End Patch" ile bitmeli.')
  const hunks: PatchHunk[] = []
  let i = 1
  const end = lines.length - 1
  while (i < end) {
    const header = lines[i]?.trim() ?? ''
    if (i === 1 && header.startsWith('*** Environment ID:')) {
      i++
      continue
    }
    if (header.startsWith('*** Add File: ')) {
      const filePath = header.slice('*** Add File: '.length).trim()
      if (!filePath) throw new Error('Eklenecek dosyanın yolu boş.')
      const body: string[] = []
      i++
      while (i < end && !isBoundary(lines[i] ?? '')) {
        const line = lines[i] ?? ''
        if (!line.startsWith('+')) throw new Error(`Ekleme satırı '+' ile başlamalı: ${filePath}`)
        body.push(line.slice(1))
        i++
      }
      hunks.push({ type: 'add', path: filePath, contents: body.join('\n') })
      continue
    }
    if (header.startsWith('*** Delete File: ')) {
      const filePath = header.slice('*** Delete File: '.length).trim()
      if (!filePath) throw new Error('Silinecek dosyanın yolu boş.')
      const next = lines[i + 1]?.trim()
      if (next !== undefined && i + 1 < end && !isBoundary(next)) throw new Error(`Silme gövdesi olamaz: ${filePath}`)
      hunks.push({ type: 'delete', path: filePath })
      i++
      continue
    }
    if (header.startsWith('*** Update File: ')) {
      const filePath = header.slice('*** Update File: '.length).trim()
      if (!filePath) throw new Error('Güncellenecek dosyanın yolu boş.')
      i++
      let movePath: string | undefined
      const move = lines[i]?.trimEnd()
      if (move === '*** Move to:' || move?.startsWith('*** Move to: ')) {
        movePath = move.slice('*** Move to: '.length).trim()
        if (!movePath) throw new Error(`Taşıma hedefi boş: ${filePath}`)
        i++
      }
      const chunks: PatchChunk[] = []
      while (i < end && !isBoundary(lines[i]?.trim() ?? '')) {
        const line = lines[i] ?? ''
        const trimmed = line.trimEnd()
        if (trimmed === '@@' || trimmed.startsWith('@@ ')) {
          chunks.push({ oldLines: [], newLines: [], changeContext: trimmed === '@@' ? undefined : trimmed.slice(3) })
          i++
          continue
        }
        if (chunks.length === 0) chunks.push({ oldLines: [], newLines: [] })
        const chunk = chunks[chunks.length - 1]
        if (!chunk) break
        if (line.startsWith(' ')) {
          chunk.oldLines.push(line.slice(1))
          chunk.newLines.push(line.slice(1))
        } else if (line.startsWith('-')) {
          chunk.oldLines.push(line.slice(1))
        } else if (line.startsWith('+')) {
          chunk.newLines.push(line.slice(1))
        } else if (line === '') {
          chunk.oldLines.push('')
          chunk.newLines.push('')
        } else {
          throw new Error(`Yama satırı boşluk, + veya - ile başlamalı (${filePath}): ${line}`)
        }
        i++
      }
      if (chunks.length === 0 || chunks.every((c) => c.oldLines.length === 0 && c.newLines.length === 0)) {
        throw new Error(`Güncelleme boş: ${filePath}`)
      }
      hunks.push({ type: 'update', path: filePath, movePath, chunks })
      continue
    }
    throw new Error(`Tanınmayan yama başlığı: ${header}`)
  }
  if (hunks.length === 0) throw new Error('Yama boş.')
  return hunks
}

/** Dosyanın baskın satır sonu CRLF mi. */
export function usesCrlf(content: string): boolean {
  const crlf = content.split('\r\n').length - 1
  return crlf > 0 && crlf >= content.split('\n').length - 1 - crlf
}

export function applyChunks(original: string, chunks: readonly PatchChunk[], filePath: string): string {
  const raw = original.split('\n')
  if (raw.at(-1) === '') raw.pop()
  // Karşılaştırma CR'siz yapılır; satırlar kendi sonlarıyla kalır, yalnızca yeni satırlar baskın satır sonunu alır.
  const lines = raw.map((line) => (line.endsWith('\r') ? line.slice(0, -1) : line))
  const cr = usesCrlf(original) ? '\r' : ''
  let cursor = 0
  const replacements: Array<[number, number, string[]]> = []
  for (const chunk of chunks) {
    if (chunk.changeContext) {
      const at = seek(lines, [chunk.changeContext], cursor)
      if (at < 0) throw new Error(`Bağlam bulunamadı (${filePath}): ${chunk.changeContext}`)
      cursor = at + 1
    }
    if (chunk.oldLines.length === 0) {
      replacements.push([lines.length, 0, chunk.newLines])
      continue
    }
    const found = seek(lines, chunk.oldLines, cursor)
    if (found < 0) throw new Error(`Beklenen satırlar yok (${filePath}):\n${chunk.oldLines.join('\n')}`)
    replacements.push([found, chunk.oldLines.length, chunk.newLines])
    cursor = found + chunk.oldLines.length
  }
  // reverse: aynı konuma ekleyen parçalar yamadaki sırayla kalsın.
  for (const [start, remove, insert] of replacements.reverse().sort((a, b) => b[0] - a[0])) {
    // Değişmeyen bağlam satırları dosyadaki baytlarıyla kalır (karışık satır sonlu dosyada başka satıra dokunulmaz).
    let prefix = 0
    while (prefix < remove && prefix < insert.length && lines[start + prefix] === insert[prefix]) prefix++
    let suffix = 0
    while (suffix < remove - prefix && suffix < insert.length - prefix && lines[start + remove - 1 - suffix] === insert[insert.length - 1 - suffix]) suffix++
    raw.splice(start + prefix, remove - prefix - suffix, ...insert.slice(prefix, insert.length - suffix).map((line) => line + cr))
  }
  const last = raw.at(-1)
  if (last !== undefined && last !== '' && cr && !last.endsWith('\r')) raw[raw.length - 1] = last + cr
  if (raw.at(-1) !== '') raw.push('')
  return raw.join('\n')
}

function seek(lines: readonly string[], pattern: readonly string[], start: number): number {
  if (pattern.length === 0) return -1
  const compares = [
    (a: string, b: string) => a === b,
    (a: string, b: string) => a.trimEnd() === b.trimEnd(),
    (a: string, b: string) => a.trim() === b.trim(),
  ]
  for (const compare of compares) {
    for (let offset = start; offset <= lines.length - pattern.length; offset++) {
      if (pattern.every((line, index) => compare(lines[offset + index] ?? '', line))) return offset
    }
  }
  return -1
}

function isBoundary(line: string): boolean {
  return (
    line === '*** End Patch' ||
    line.startsWith('*** Add File: ') ||
    line.startsWith('*** Delete File: ') ||
    line.startsWith('*** Update File: ')
  )
}

function stripFence(input: string): string {
  const match = /^```[a-z]*\n([\s\S]*?)\n```$/.exec(input)
  return match?.[1] ?? input
}

function stripHeredoc(input: string): string {
  return input.match(/^(?:cat\s+)?<<(['"]?)(\w+)\1\s*\n([\s\S]*?)\n\2\s*$/)?.[3] ?? input
}
