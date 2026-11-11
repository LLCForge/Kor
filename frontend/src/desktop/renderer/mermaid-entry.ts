// [g3] Ayrı paket (dist/renderer/mermaid.js): yalnızca sohbette ```mermaid bloğu görününce yüklenir.
//
// CSP (style-src 'self') satır içi <style> öğelerini ve style="" özniteliklerini engeller. 'unsafe-inline'
// eklemek yerine: mermaid'in SVG'si DOMPurify ile temizlenir, <style> içeriği gölge köke (shadow root)
// CSSOM ile (adoptedStyleSheets) bağlanır, style öznitelikleri de element.style.cssText ile uygulanır.
// CSSOM, CSP'nin satır içi stil denetimine tabi değildir; stiller gölge köke hapsolur ve sayfanın geri kalanına sızamaz.
import DOMPurify from 'dompurify'
import mermaid from 'mermaid'

declare global {
  interface Window {
    // opts.theme: açık/koyu moda bağlı hazır tema geçişi (feature 24). Verilmezse sistem moduna göre seçilir.
    __g3Mermaid?: { render(code: string, host: HTMLElement, opts?: { theme?: string }): Promise<void> }
  }
}

const FONT = "'Segoe UI', system-ui, -apple-system, sans-serif"
let theme: string | null = null
let counter = 0
let queue: Promise<unknown> = Promise.resolve()

function configure(override?: string): void {
  const next = override || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'neutral')
  if (next === theme) return
  theme = next
  mermaid.initialize({
    startOnLoad: false,
    // strict: etiketler temizlenir, click/href etkileşimleri kapanır.
    securityLevel: 'strict',
    // next yalnızca geçerli hazır tema adlarından biridir (THEMES / sistem modu); union'a daraltılır.
    theme: next as 'base' | 'dark' | 'default' | 'forest' | 'neutral',
    fontFamily: FONT,
    themeVariables: { fontFamily: FONT, fontSize: '14px' },
    // HTML etiketleri foreignObject gerektirir; temizlikte kaldırılacağı için SVG metni kullanılır.
    htmlLabels: false,
    flowchart: { htmlLabels: false },
    suppressErrorRendering: true,
  })
}

interface Sanitized {
  fragment: DocumentFragment
  css: string
  styles: string[]
}

function sanitize(svg: string): Sanitized {
  const purifier = DOMPurify(window)
  const css: string[] = []
  const styles: string[] = []
  const pendingStyles = new Map<Element, string>()
  purifier.addHook('uponSanitizeElement', (node, data) => {
    if (data.tagName === 'style') css.push(node.textContent ?? '')
  })
  purifier.addHook('uponSanitizeAttribute', (node, data) => {
    if (data.attrName !== 'style') return
    pendingStyles.set(node, data.attrValue)
    data.keepAttr = false
  })
  purifier.addHook('afterSanitizeAttributes', (node) => {
    const style = pendingStyles.get(node)
    if (style) node.setAttribute('data-g3s', String(styles.push(style) - 1))
  })
  const fragment = purifier.sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true },
    FORBID_TAGS: ['style', 'foreignObject', 'script'],
    RETURN_DOM_FRAGMENT: true,
  })
  return { fragment, css: css.join('\n'), styles }
}

async function renderNow(code: string, host: HTMLElement, themeOverride?: string): Promise<void> {
  configure(themeOverride)
  // Ölçüm için görünmez kap: metin boyutları son görünümle aynı yazı tipiyle hesaplanır.
  const measure = document.createElement('div')
  measure.className = 'g3-mermaid-measure'
  document.body.append(measure)
  try {
    const { svg } = await mermaid.render(`g3-mermaid-${++counter}`, code, measure)
    const { fragment, css, styles } = sanitize(svg)
    const root = host.shadowRoot ?? host.attachShadow({ mode: 'open' })
    const sheet = new CSSStyleSheet()
    // Doğal boyut + taşma görünür: diyagram ölçeklenip kırpılmaz; büyükse host (overflow:auto) içinde kaydırılır/pan yapılır.
    sheet.replaceSync(`svg { display: block; overflow: visible; }\n${css}`)
    root.adoptedStyleSheets = [sheet]
    root.replaceChildren(fragment)
    for (const el of root.querySelectorAll<SVGElement | HTMLElement>('[data-g3s]')) {
      el.style.cssText = styles[Number(el.getAttribute('data-g3s'))] ?? ''
      el.removeAttribute('data-g3s')
    }
    // SVG'yi viewBox'ın doğal boyutuna sabitle (mermaid'in responsive max-width'ini ezer): düğümler küçülüp kırpılmaz.
    const svgEl = root.querySelector('svg')
    const vb = svgEl
      ?.getAttribute('viewBox')
      ?.split(/[\s,]+/)
      .map(Number)
    if (svgEl && vb && vb.length === 4 && vb[2] > 0 && vb[3] > 0) {
      svgEl.style.maxWidth = 'none'
      svgEl.style.width = `${Math.ceil(vb[2])}px`
      svgEl.style.height = `${Math.ceil(vb[3])}px`
    }
  } finally {
    measure.remove()
  }
}

window.__g3Mermaid = {
  // mermaid genel durum kullanır; çizimler sıraya alınır.
  render: (code, host, opts) => {
    const run = queue.then(() => renderNow(code, host, opts?.theme))
    queue = run.catch(() => undefined)
    return run
  },
}
