// [g3] Markdown işleme yardımcıları (saf; test edilebilir): dosya yolu bağlantıları, satırlara bölme, tablo/kod kopyalama.
import type { Element, ElementContent, Root, RootContent, Text } from 'hast'

export interface FileRef {
  start: number
  end: number
  path: string
  line: number | undefined
}

// "src/a.ts", "./src/a.ts:12", "src\\a.ts:3:7", "README.md:5". Tek başına "package.json" eşleşmez (alan adlarıyla karışır);
// URL içi (önünde "/" veya ":" olan), mutlak ve "../" ile başlayan yollar dışarıda kalır.
const SEGMENT = String.raw`[\w@-][\w.@-]*`
const EXT = String.raw`\.[A-Za-z][\w]{0,9}`
const FILE_REF_SOURCE = String.raw`(?<![\w./\\@:-])((?:\.\/)?(?:${SEGMENT}[\\/])+${SEGMENT}${EXT}|${SEGMENT}${EXT}(?=:\d))(?::(\d{1,6})(?::\d{1,6})?)?(?![\w\\/])`

export function findFileRefs(text: string): FileRef[] {
  const refs: FileRef[] = []
  for (const m of text.matchAll(new RegExp(FILE_REF_SOURCE, 'g'))) {
    const path = m[1]
    if (!path || m.index === undefined) continue
    const line = m[2] ? Number(m[2]) : undefined
    refs.push({ start: m.index, end: m.index + m[0].length, path: path.replaceAll('\\', '/').replace(/^\.\//, ''), line: line && line > 0 ? line : undefined })
  }
  return refs
}

/** Satır içi kodun tamamı bir dosya yoluysa döner. */
export function parseFileRef(text: string): FileRef | null {
  const trimmed = text.trim()
  const ref = findFileRefs(trimmed)[0]
  return ref && ref.start === 0 && ref.end === trimmed.length ? ref : null
}

const SKIP_TAGS = new Set(['code', 'pre', 'a', 'button', 'script', 'style'])

function hasClass(el: Element, name: string): boolean {
  const cls = el.properties.className
  return Array.isArray(cls) && cls.includes(name)
}

function splitText(node: Text): ElementContent[] {
  const refs = findFileRefs(node.value)
  if (refs.length === 0) return [node]
  const out: ElementContent[] = []
  let pos = 0
  for (const ref of refs) {
    if (ref.start > pos) out.push({ type: 'text', value: node.value.slice(pos, ref.start) })
    out.push({
      type: 'element',
      tagName: 'span',
      properties: { className: ['g3-file-ref'], dataPath: ref.path, ...(ref.line ? { dataLine: ref.line } : {}) },
      children: [{ type: 'text', value: node.value.slice(ref.start, ref.end) }],
    })
    pos = ref.end
  }
  if (pos < node.value.length) out.push({ type: 'text', value: node.value.slice(pos) })
  return out
}

function linkify(parent: Root | Element): void {
  const children: Array<RootContent | ElementContent> = []
  for (const child of parent.children) {
    if (child.type === 'text') {
      children.push(...splitText(child))
      continue
    }
    if (child.type === 'element' && !SKIP_TAGS.has(child.tagName) && !hasClass(child, 'katex')) linkify(child)
    children.push(child)
  }
  // Metin düğümü yerine metin + span konur; ikisi de hem kök hem öğe çocuğu olarak geçerlidir.
  parent.children = children as typeof parent.children
}

/** Düz metindeki dosya yollarını tıklanabilir span.g3-file-ref öğelerine çevirir (kod ve bağlantılar hariç). */
export function rehypeFileLinks() {
  return (tree: Root) => linkify(tree)
}

/** Bir hast düğümünün düz metni. */
export function hastText(node: Root | RootContent | ElementContent): string {
  if (node.type === 'text') return node.value
  if (node.type === 'element' || node.type === 'root') return node.children.map(hastText).join('')
  return ''
}

function nodeLines(node: ElementContent): ElementContent[][] {
  if (node.type === 'text') return node.value.split('\n').map((value): ElementContent[] => (value ? [{ type: 'text', value }] : []))
  if (node.type === 'element') return splitHastLines(node.children).map((parts): ElementContent[] => (parts.length > 0 ? [{ ...node, children: parts }] : []))
  return [[]]
}

/** Vurgulanmış kodu satırlara böler; birden çok satıra yayılan öğe (ör. çok satırlı yorum) her satırda kopyalanır. */
export function splitHastLines(children: readonly ElementContent[]): ElementContent[][] {
  const lines: ElementContent[][] = [[]]
  for (const child of children) {
    const parts = nodeLines(child)
    lines[lines.length - 1]?.push(...(parts[0] ?? []))
    for (const part of parts.slice(1)) lines.push([...part])
  }
  return lines
}

/** İçerikte ``` varsa daha uzun çit kullanılır; kopyalanan blok bozulmaz. */
export function codeFence(code: string, language: string): string {
  const longest = Math.max(2, ...[...code.matchAll(/`+/g)].map((m) => m[0].length))
  const fence = '`'.repeat(longest + 1)
  return `${fence}${language}\n${code.replace(/\n$/, '')}\n${fence}`
}

const cell = (el: Element) => hastText(el).replace(/\s+/g, ' ').trim().replaceAll('|', '\\|')

/** GFM tablosu: ilk satır başlık kabul edilir. */
export function tableToMarkdown(table: Element): string {
  const rows: Element[] = []
  const collect = (el: Element) => {
    for (const child of el.children) {
      if (child.type !== 'element') continue
      if (child.tagName === 'tr') rows.push(child)
      else collect(child)
    }
  }
  collect(table)
  const matrix = rows.map((tr) => tr.children.filter((c): c is Element => c.type === 'element' && (c.tagName === 'th' || c.tagName === 'td')))
  const width = Math.max(0, ...matrix.map((r) => r.length))
  if (width === 0) return ''
  const line = (cells: string[]) => `| ${Array.from({ length: width }, (_, i) => cells[i] ?? '').join(' | ')} |`
  const header = matrix[0] ?? []
  const align = Array.from({ length: width }, (_, i) => {
    const value = header[i]?.properties.align
    return value === 'center' ? ':---:' : value === 'right' ? '---:' : value === 'left' ? ':---' : '---'
  })
  return [line(header.map(cell)), line(align), ...matrix.slice(1).map((r) => line(r.map(cell)))].join('\n')
}

// Kapanmamış çit (akış sürerken) de kod sayılır; içindeki metin dönüştürülmez.
const CODE_SPANS = /(```[\s\S]*?(?:```|$)|~~~[\s\S]*?(?:~~~|$)|`[^`\n]*`)/g

function outsideCode(text: string, fn: (part: string) => string): string {
  return text
    .split(CODE_SPANS)
    .map((part, i) => (i % 2 === 1 ? part : fn(part)))
    .join('')
}

/** Modellerin sık kullandığı \( \) ve \[ \] LaTeX ayraçlarını remark-math'in anladığı $$ biçimine çevirir. */
export function normalizeMathDelimiters(text: string): string {
  if (!text.includes('\\(') && !text.includes('\\[')) return text
  return outsideCode(text, (part) =>
    part
      .replace(/\\\[([\s\S]+?)\\\]/g, (_, inner: string) => `\n$$\n${inner.trim()}\n$$\n`)
      .replace(/\\\(([^\n]+?)\\\)/g, (_, inner: string) => `$$${inner.trim()}$$`),
  )
}

/** Yanıttaki benzersiz http(s) bağlantıları (kod blokları hariç), en fazla `max` tane. */
export function extractLinks(text: string, max = 4): string[] {
  const found = new Set<string>()
  outsideCode(text, (part) => {
    for (const m of part.matchAll(/https?:\/\/[^\s<>()[\]"'`]+/g)) {
      const url = m[0].replace(/[.,;:!?*_]+$/, '')
      try {
        const parsed = new URL(url)
        if (parsed.protocol === 'http:' || parsed.protocol === 'https:') found.add(parsed.toString())
      } catch {
        // Geçersiz adres: önizleme kartı oluşturulmaz.
      }
    }
    return part
  })
  return [...found].slice(0, max)
}
