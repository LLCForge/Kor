// [g16] Artımlı markdown (241): metni bağımsız ayrıştırılabilen üst düzey bloklara böler. Akış sırasında yalnızca son
// blok değişir; tamamlanmış bloklar aynı dizgi kaldığı için memo ile yeniden ayrıştırılmaz.
//
// Bölme yalnızca birleşik ayrıştırmayla aynı sonucu verdiği yerde yapılır: boş satırdan sonra sütun 0'da başlayan ve
// önceki bloğa bağlanamayan bir satır. Çit (``` ~~~) ve $$ matematik blokları içinde bölünmez; girintili satır, liste
// maddesi, alıntı ve tablo satırı önceki bloğa eklenir (gevşek liste, liste devamı, girintili kod bozulmasın).
// Belgenin her yerinden başvurulabilen yapılar (bağlantı/dipnot tanımları) veya boş satırla bitmeyen HTML blokları
// varsa metin bölünmez.

const FENCE_OPEN = /^\s*(`{3,}|~{3,})/
const MATH_OPEN = /^\s*\${2,}[^$]*$/
const MATH_CLOSE = /^\s*\${2,}\s*$/
const JOINS_PREVIOUS = /^(?:\s|[-*+](?:\s|$)|\d{1,9}[.)](?:\s|$)|>|\|)/
const WHOLE_DOCUMENT = /^ {0,3}\[[^\]\n]+\]:|\[\^[^\]\n]+\]|^ {0,3}<(?:!--|pre\b|script\b|style\b|textarea\b)/im

const FENCE_LANG = /^\s*(?:`{3,}|~{3,})\s*([^\s`{]+)/
const FENCE_CLOSE = /^\s*(`{3,}|~{3,})\s*$/

/**
 * Önceki blok yalnızca "birlikte tutulan" dilde çitlerden oluşuyorsa ve yeni satır da öyle bir çit açıyorsa bölünmez.
 * Art arda diyagramlar (#29) ancak aynı ayrıştırmada kardeş kalırsa rehypeDiagramTabs onları sekmeli karta toplar.
 */
function joinsFenceRun(previous: readonly string[], line: string, keepTogether: (lang: string) => boolean): boolean {
  const next = FENCE_LANG.exec(line)?.[1]
  if (!next || !keepTogether(next.toLowerCase())) return false
  const body = previous.filter((l) => l.trim() !== '')
  const first = FENCE_LANG.exec(body[0] ?? '')?.[1]
  return first !== undefined && keepTogether(first.toLowerCase()) && FENCE_CLOSE.test(body.at(-1) ?? '')
}

export function splitMarkdownBlocks(text: string, keepTogether?: (lang: string) => boolean): string[] {
  if (WHOLE_DOCUMENT.test(text)) return [text]
  const lines = text.split('\n')
  const blocks: string[] = []
  let start = 0
  let fence: { char: string; length: number } | null = null
  let math = false
  let previousBlank = false

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? ''
    if (fence) {
      const close = /^\s*(`{3,}|~{3,})\s*$/.exec(line)?.[1]
      if (close && close[0] === fence.char && close.length >= fence.length) fence = null
      previousBlank = false
      continue
    }
    if (math) {
      if (MATH_CLOSE.test(line)) math = false
      previousBlank = false
      continue
    }
    const blank = line.trim() === ''
    const joinsRun = keepTogether !== undefined && joinsFenceRun(lines.slice(start, i), line, keepTogether)
    if (previousBlank && !blank && i > start && !JOINS_PREVIOUS.test(line) && !joinsRun) {
      blocks.push(lines.slice(start, i).join('\n'))
      start = i
    }
    const open = FENCE_OPEN.exec(line)?.[1]
    if (open) fence = { char: open[0] ?? '`', length: open.length }
    else if (MATH_OPEN.test(line)) math = true
    previousBlank = blank
  }
  blocks.push(lines.slice(start).join('\n'))
  return blocks
}
