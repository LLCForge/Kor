/**
 * Sade HTML -> okunabilir Markdown dönüştürücü (web_fetch için). Tam bir HTML ayrıştırıcı değildir:
 * gezinme/betik/stil gibi gürültüyü atar, başlık, paragraf, liste, bağlantı, kod ve tabloyu korur.
 * shortcut: iç içe aynı adlı atılan etiketler (svg içinde svg) ve bozuk HTML yaklaşık işlenir; daha iyisi gerekirse bir ayrıştırıcı kütüphanesine geçilir.
 */

const NAMED_ENTITIES: Readonly<Record<string, string>> = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', copy: '©', reg: '®', trade: '™',
  hellip: '…', mdash: '—', ndash: '–', lsquo: '‘', rsquo: '’', ldquo: '“', rdquo: '”', laquo: '«', raquo: '»',
  bull: '•', middot: '·', euro: '€', pound: '£', yen: '¥', times: '×', divide: '÷', deg: '°', para: '¶', sect: '§',
}

export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z][a-z0-9]*);/gi, (whole, body: string) => {
    if (body[0] === '#') {
      const code = body[1] === 'x' || body[1] === 'X' ? Number.parseInt(body.slice(2), 16) : Number.parseInt(body.slice(1), 10)
      return Number.isFinite(code) && code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : whole
    }
    return NAMED_ENTITIES[body.toLowerCase()] ?? whole
  })
}

/** Etiketleri atıp düz metne çevirir (arama özetleri gibi kısa parçalar için). */
export function stripTags(html: string): string {
  return decodeEntities(html.replace(/<[^>]*>/g, '')).replace(/\s+/g, ' ').trim()
}

const DROPPED = ['script', 'style', 'noscript', 'svg', 'template', 'iframe', 'head', 'nav', 'footer', 'aside', 'form', 'button', 'select', 'canvas', 'object']
const BLOCK = new Set(['p', 'div', 'section', 'article', 'main', 'header', 'figure', 'figcaption', 'table', 'thead', 'tbody', 'dl', 'dt', 'dd', 'address', 'details', 'summary'])

function attr(attrs: string, name: string): string | null {
  const m = new RegExp(`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i').exec(attrs)
  return m ? decodeEntities(m[1] ?? m[2] ?? m[3] ?? '') : null
}

function absoluteUrl(href: string, base: string | undefined): string | null {
  try {
    const url = new URL(href, base)
    // javascript:, data: vb. bağlantılar çıktıya taşınmaz.
    return url.protocol === 'http:' || url.protocol === 'https:' || url.protocol === 'mailto:' ? url.href : null
  } catch {
    return null
  }
}

export interface MarkdownResult {
  title: string | null
  markdown: string
}

export function htmlToMarkdown(html: string, baseUrl?: string): MarkdownResult {
  const titleMatch = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(html)
  const title = titleMatch?.[1] ? stripTags(titleMatch[1]) || null : null

  let body = html.replace(/<!--[\s\S]*?-->/g, '')
  // <pre> blokları gürültü temizliğinden ÖNCE ayrılır (twoslash gibi araçlar tanımlayıcıları <button> içine koyar)
  // ve boşluklarıyla korunur; yer tutucuyla ayrılır.
  const blocks: string[] = []
  body = body.replace(/<pre\b([^>]*)>([\s\S]*?)<\/pre\s*>/gi, (_m, attrs: string, inner: string) => {
    const lang = /class\s*=\s*["'][^"']*language-([\w+-]+)/i.exec(`${attrs} ${inner}`)?.[1] ?? ''
    const code = decodeEntities(inner.replace(/<[^>]*>/g, '')).replace(/^\n+|\s+$/g, '')
    blocks.push(`\n\n\`\`\`${lang}\n${code}\n\`\`\`\n\n`)
    return `\u0000${blocks.length - 1}\u0000`
  })
  for (const tag of DROPPED) body = body.replace(new RegExp(`<${tag}\\b[\\s\\S]*?</${tag}\\s*>`, 'gi'), ' ')
  // Okunabilirlik: varsa ana içerik bölgesi alınır.
  const main = /<(main|article)\b[^>]*>([\s\S]*)<\/\1\s*>/i.exec(body)
  if (main?.[2]) body = main[2]
  else body = /<body\b[^>]*>([\s\S]*)<\/body\s*>/i.exec(body)?.[1] ?? body

  let out = ''
  const lists: Array<{ ordered: boolean; count: number }> = []
  const links: Array<string | null> = []
  const tokens = /<(\/?)([a-zA-Z][a-zA-Z0-9-]*)([^>]*)>|([^<]+)|</g
  for (let m = tokens.exec(body); m; m = tokens.exec(body)) {
    const text = m[4] ?? (m[2] === undefined ? '<' : undefined)
    if (text !== undefined) {
      out += decodeEntities(text.replace(/\s+/g, ' '))
      continue
    }
    const closing = m[1] === '/'
    const tag = (m[2] ?? '').toLowerCase()
    const attrs = m[3] ?? ''
    if (/^h[1-6]$/.test(tag)) out += closing ? '\n\n' : `\n\n${'#'.repeat(Number(tag[1]))} `
    else if (BLOCK.has(tag)) out += '\n\n'
    else if (tag === 'br') out += '\n'
    else if (tag === 'hr') out += '\n\n---\n\n'
    else if (tag === 'ul' || tag === 'ol') {
      // Yalnızca en dıştaki liste paragraf gibi ayrılır; iç içe liste üst maddeye yapışık kalır.
      if (closing) lists.pop()
      if (lists.length === 0) out += '\n\n'
      if (!closing) lists.push({ ordered: tag === 'ol', count: 0 })
    } else if (tag === 'li' && !closing) {
      const list = lists.at(-1)
      // \u0001: satır kırpmasından korunan girinti; sonda boşluğa çevrilir.
      const indent = '\u0001\u0001'.repeat(Math.max(0, lists.length - 1))
      if (list) list.count++
      out += `\n${indent}${list?.ordered ? `${list.count}.` : '-'} `
    } else if (tag === 'a') {
      if (closing) {
        const href = links.pop()
        if (href) out += `](${href})`
      } else {
        const href = absoluteUrl(attr(attrs, 'href') ?? '', baseUrl)
        links.push(href)
        if (href) out += '['
      }
    } else if (tag === 'img' && !closing) {
      const alt = attr(attrs, 'alt')?.trim()
      const src = absoluteUrl(attr(attrs, 'src') ?? '', baseUrl)
      if (alt && src) out += `![${alt}](${src})`
    } else if (tag === 'strong' || tag === 'b') out += '**'
    else if (tag === 'em' || tag === 'i') out += '*'
    else if (tag === 'code') out += '`'
    else if (tag === 'blockquote') out += closing ? '\n\n' : '\n\n> '
    else if ((tag === 'td' || tag === 'th') && !closing) out += ' | '
    else if (tag === 'tr' && closing) out += ' |\n'
  }

  const markdown = out
    .split('\n')
    .map((line) => line.replace(/[ \t]+/g, ' ').trim())
    .join('\n')
    .replace(/\[\s*\]\([^)]*\)/g, '')
    .replaceAll('\u0001', ' ')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/\u0000(\d+)\u0000/g, (_m, i: string) => blocks[Number(i)] ?? '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
  return { title, markdown }
}
