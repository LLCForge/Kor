// [g21] Belge HTML'i (DOCX/EPUB çıktısı) için katı DOMPurify profili (#174, #180, #190). İzin listesi (fail-closed):
//      yalnızca metin yapısı etiketleri ve birkaç zararsız öznitelik. style/class/id YOK (uygulama arayüzünü
//      taklit eden ya da kaplayan CSS ve DOM çakışması önlenir), bağlantılar href'siz düz metne döner, görseller
//      yalnızca base64 data: URI. Sonuç bir DocumentFragment'tir; ana DOM'a yalnızca bu temizlenmiş parça girer.
import DOMPurify from 'dompurify'

export const MAX_DOC_HTML = 24 * 1024 * 1024
export const MAX_DOC_NODES = 150_000
export const DOC_IMAGE_URI = /^data:image\/(?:png|jpe?g|gif|webp|bmp|svg\+xml);base64,[a-z0-9+/=\s]*$/i

const ALLOWED_TAGS = [
  'p', 'br', 'hr', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'strong', 'b', 'em', 'i', 'u', 's', 'strike', 'del', 'ins', 'sub',
  'sup', 'small', 'mark', 'span', 'div', 'section', 'article', 'header', 'footer', 'aside', 'main', 'blockquote', 'q',
  'cite', 'code', 'pre', 'kbd', 'samp', 'var', 'abbr', 'dfn', 'time', 'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'table',
  'thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'caption', 'colgroup', 'col', 'figure', 'figcaption', 'img', 'a', 'ruby',
  'rt', 'rp', 'bdi', 'bdo', 'wbr',
]
const ALLOWED_ATTR = ['src', 'alt', 'title', 'colspan', 'rowspan', 'lang', 'dir', 'start', 'reversed', 'scope', 'headers', 'abbr', 'datetime']

export interface CleanDoc {
  fragment: DocumentFragment
  /** DOMPurify'ın kaldırdığı öğe ve öznitelik sayısı */
  removed: number
}

let purifier: ReturnType<typeof DOMPurify> | null = null

function instance(): ReturnType<typeof DOMPurify> {
  if (purifier) return purifier
  purifier = DOMPurify(window)
  purifier.addHook('uponSanitizeAttribute', (_node, data) => {
    if (data.attrName === 'src' && !DOC_IMAGE_URI.test(data.attrValue)) data.keepAttr = false
  })
  return purifier
}

/** Güvenilmez belge HTML'ini temizler. Çok büyük ya da çok karmaşık içerikte hata fırlatır (DoS). */
export function sanitizeDocHtml(html: string): CleanDoc {
  if (html.length > MAX_DOC_HTML) throw new Error('Belge görüntülenemeyecek kadar büyük')
  const p = instance()
  const fragment = p.sanitize(html, { ALLOWED_TAGS, ALLOWED_ATTR, ALLOW_DATA_ATTR: false, ALLOW_ARIA_ATTR: false, RETURN_DOM_FRAGMENT: true })
  if (fragment.querySelectorAll('*').length > MAX_DOC_NODES) throw new Error('Belge görüntülenemeyecek kadar karmaşık')
  return { fragment, removed: p.removed.length }
}
