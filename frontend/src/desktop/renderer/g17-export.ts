// [g17] Dışa aktarma/paylaşım yardımcıları. Saf kısımlar (TSV, SVG arka planı, Markdown, sınıflandırma,
//      HTML belge sarma, tema zorlama) test edilebilir; DOM/canvas kısımları yalnızca arayüzde çalışır.
//      Güvenlik: HTML anlık görüntüsü zaten render edilmiş (react-markdown ile güvenli) DOM'dan klonlanır;
//      ham model çıktısı enjekte edilmez. Dışa aktarılan belge ağ erişmez (CSP: default-src 'none').
import type { Element } from 'hast'
import type { ChatItem } from './chat-state.ts'
import { hastText } from './markdown-utils.ts'
import { uiLocale } from './ui-locale.ts'

export type ExportTheme = 'auto' | 'light' | 'dark'

export interface ImageExportOptions {
  scale: number
  transparent: boolean
  theme: ExportTheme
  title?: string
  timestamp?: boolean
}

// ---------- Saf yardımcılar ----------

const tsvCell = (el: Element) =>
  hastText(el)
    .replace(/[\t\r\n]+/g, ' ')
    .trim()

/** GFM tablosunu, elektronik tabloya yapıştırmaya uygun TSV'ye çevirir (satırlar \n, hücreler \t). */
export function tableToTsv(table: Element): string {
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
  return matrix.map((r) => Array.from({ length: width }, (_, i) => (r[i] ? tsvCell(r[i] as Element) : '')).join('\t')).join('\n')
}

export interface ContentFlags {
  code: boolean
  diagram: boolean
  image: boolean
}

// Zengin blok dilleri (diyagram sayılır); tek arama noktası registerRichBlock olsa da filtre için metinden tahmin yeter.
const DIAGRAM_FENCE = /```[ \t]*(?:mermaid|dot|graphviz|chart|plantuml|nomnoml|sequence|gantt)\b/i

/** Bir mesajın içerik türlerini metinden sınıflandırır (içerik türü filtresi için). */
export function classifyMessage(text: string): ContentFlags {
  return {
    code: /```/.test(text) || /(^|\n) {4,}\S/.test(text),
    diagram: DIAGRAM_FENCE.test(text),
    image: /!\[[^\]]*\]\([^)]+\)/.test(text) || /<img\b/i.test(text),
  }
}

export interface ChatSummary {
  userMessages: number
  assistantMessages: number
  toolCalls: number
  codeBlocks: number
  diagrams: number
  words: number
}

/** Sohbetin görsel özet kartı için sayısal özet. */
export function chatSummary(items: readonly ChatItem[]): ChatSummary {
  const summary: ChatSummary = { userMessages: 0, assistantMessages: 0, toolCalls: 0, codeBlocks: 0, diagrams: 0, words: 0 }
  for (const it of items) {
    if (it.kind === 'user') summary.userMessages++
    else if (it.kind === 'tool') summary.toolCalls++
    else if (it.kind === 'assistant') {
      summary.assistantMessages++
      summary.codeBlocks += Math.floor((it.text.match(/```/g)?.length ?? 0) / 2)
      summary.diagrams += it.text.match(new RegExp(DIAGRAM_FENCE, 'gi'))?.length ?? 0
      summary.words += it.text.trim() ? it.text.trim().split(/\s+/).length : 0
    }
  }
  return summary
}

/** Seçili (veya tüm) kullanıcı/asistan mesajlarını Markdown belgesine çevirir. */
export function messagesToMarkdown(items: readonly ChatItem[], selected?: ReadonlySet<string>): string {
  const parts: string[] = []
  for (const it of items) {
    if (it.kind !== 'user' && it.kind !== 'assistant') continue
    if (selected && !selected.has(it.id)) continue
    const role = it.kind === 'user' ? 'Kullanıcı' : 'Asistan'
    parts.push(`## ${role}\n\n${it.text.trim()}`)
  }
  return parts.join('\n\n---\n\n')
}

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : c === '"' ? '&quot;' : '&#39;'))
}

/** Toplanan CSS'te koyu tema medya sorgusunu zorlar: light=asla, dark=her zaman, auto=değişmez. */
export function forceThemeCss(css: string, theme: ExportTheme): string {
  if (theme === 'auto') return css
  const condition = theme === 'dark' ? 'all' : 'not all'
  return css.replace(/@media\s*\(\s*prefers-color-scheme\s*:\s*dark\s*\)/gi, `@media ${condition}`)
}

/** Kendine yeten (ağsız, gömülü) bir HTML belgesi üretir. */
export function wrapHtmlDoc(bodyHtml: string, css: string, opts: { title: string; theme: ExportTheme }): string {
  const csp = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:"
  return [
    '<!doctype html>',
    '<html lang="tr">',
    '<head>',
    '<meta charset="utf-8">',
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    `<meta http-equiv="Content-Security-Policy" content="${csp}">`,
    `<title>${escapeHtml(opts.title)}</title>`,
    `<style>\n${forceThemeCss(css, opts.theme)}\n</style>`,
    '</head>',
    '<body class="g17-export-doc">',
    bodyHtml,
    '</body>',
    '</html>',
    '',
  ].join('\n')
}

/** Diyagram SVG dizgesine, saydam değilse arka plan dikdörtgeni ekler (SVG dosyası dışa aktarımı). */
export function decorateSvgBackground(svg: string, transparent: boolean, theme: Exclude<ExportTheme, 'auto'>): string {
  if (transparent) return svg
  const match = svg.match(/<svg\b[^>]*>/i)
  if (!match) return svg
  const bg = theme === 'dark' ? '#1f1e1b' : '#ffffff'
  return svg.replace(match[0], `${match[0]}<rect x="0" y="0" width="100%" height="100%" fill="${bg}"/>`)
}

// ---------- Arayüz (DOM/canvas) yardımcıları ----------

interface ThemeColors {
  bg: string
  text: string
  muted: string
}

function themeColors(theme: ExportTheme): ThemeColors {
  if (theme === 'light') return { bg: '#ffffff', text: '#1a1a18', muted: '#6b675e' }
  if (theme === 'dark') return { bg: '#1f1e1b', text: '#eeeeee', muted: '#9a958a' }
  const cs = getComputedStyle(document.documentElement)
  const read = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback
  return { bg: read('--surface', '#ffffff'), text: read('--text', '#1a1a18'), muted: read('--muted', '#6b675e') }
}

/** İçinde bulunulan temayı çözer (SVG arka planı 'auto' bağlamında). */
export function resolveTheme(theme: ExportTheme): Exclude<ExportTheme, 'auto'> {
  if (theme !== 'auto') return theme
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

function withXmlns(svg: string): string {
  return /\bxmlns=/.test(svg) ? svg : svg.replace(/<svg\b/, '<svg xmlns="http://www.w3.org/2000/svg"')
}

/** Diyagram/grafik SVG'sini, ölçek + saydamlık + tema + başlık/zaman damgası ile PNG blob'una çevirir. */
export function diagramToPng(svg: string, opts: ImageExportOptions): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => {
      const w = Math.max(1, Math.round(img.naturalWidth || img.width || 1))
      const h = Math.max(1, Math.round(img.naturalHeight || img.height || 1))
      const pad = opts.transparent ? 0 : 16
      const headH = opts.title ? 30 : 0
      const footH = opts.timestamp ? 22 : 0
      const cssW = w + pad * 2
      const cssH = h + pad * 2 + headH + footH
      const canvas = document.createElement('canvas')
      canvas.width = Math.round(cssW * opts.scale)
      canvas.height = Math.round(cssH * opts.scale)
      const ctx = canvas.getContext('2d')
      if (!ctx) {
        reject(new Error('Canvas bağlamı alınamadı'))
        return
      }
      ctx.scale(opts.scale, opts.scale)
      const colors = themeColors(opts.theme)
      if (!opts.transparent) {
        ctx.fillStyle = colors.bg
        ctx.fillRect(0, 0, cssW, cssH)
      }
      ctx.textBaseline = 'middle'
      if (opts.title) {
        ctx.fillStyle = colors.text
        ctx.font = '600 16px system-ui, sans-serif'
        ctx.fillText(opts.title.slice(0, 200), pad, pad + headH / 2)
      }
      ctx.drawImage(img, pad, pad + headH, w, h)
      if (opts.timestamp) {
        ctx.fillStyle = colors.muted
        ctx.font = '12px system-ui, sans-serif'
        ctx.fillText(new Date().toLocaleString(uiLocale()), pad, pad + headH + h + footH / 2)
      }
      canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('PNG oluşturulamadı'))), 'image/png')
    }
    img.onerror = () => reject(new Error('SVG görsel olarak yüklenemedi'))
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(withXmlns(svg))}`
  })
}

/** Bir blob'u panoya görsel olarak kopyalar (yalnızca clipboard-sanitized-write izni verilmiştir). */
export async function copyImageToClipboard(blob: Blob): Promise<void> {
  if (typeof ClipboardItem === 'undefined') throw new Error('Pano görsel kopyalamayı desteklemiyor')
  await navigator.clipboard.write([new ClipboardItem({ [blob.type || 'image/png']: blob })])
}

/** Aynı origin'li tüm stil sayfalarının kurallarını tek dizgede toplar (dışa aktarılan belgeye gömmek için). */
export function collectStyles(): string {
  const parts: string[] = []
  for (const sheet of Array.from(document.styleSheets)) {
    try {
      for (const rule of Array.from(sheet.cssRules)) parts.push(rule.cssText)
    } catch {
      // Çapraz origin sayfa okunamaz; atlanır (bizim sayfalar aynı origin'dedir).
    }
  }
  return parts.join('\n')
}

// Dışa aktarılan belgede anlamsız olan etkileşim öğeleri (düğmeler, menüler).
const OMIT_SELECTOR = '.action-bar, .g17-bar, .g17-menu, .g3-code-tools, .g3-table-tools, button, [data-g17-omit]'

/** Verilen mesaj düğümlerini (zaten render edilmiş, güvenli DOM) tek bir HTML belgesine dönüştürür. */
export function snapshotMessagesHtml(nodes: readonly Element[] | readonly HTMLElement[], opts: { title: string; theme: ExportTheme }): string {
  const clones = (nodes as readonly HTMLElement[]).map((node) => {
    const clone = node.cloneNode(true) as HTMLElement
    for (const el of Array.from(clone.querySelectorAll(OMIT_SELECTOR))) el.remove()
    return clone.outerHTML
  })
  const body = `<div class="app"><main class="main"><div class="g3-thread"><div class="viewport"><div class="messages">${clones.join('')}</div></div></div></main></div>`
  return wrapHtmlDoc(body, collectStyles(), opts)
}
